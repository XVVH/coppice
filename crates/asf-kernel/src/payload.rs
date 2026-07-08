//! Payload store. Spec §1 (R3).
//!
//! No fabric object embeds sensitive content: content lives here, addressed
//! by the sha256 of its plaintext, encrypted per-payload with its own DEK.
//! Erasure is crypto-shredding: destroy the wrapped DEK, insert a tombstone.
//! Hash and lineage persist; a destroyed payload resolves to
//! `{hash, shredded_at, reason}`.

use crate::keys::{dek_decrypt, dek_encrypt, Kek};
use crate::canon::sha256_hex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    #[error("sqlite: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("crypto: {0}")]
    Crypto(#[from] crate::keys::KeyError),
    #[error("payload {0} not found")]
    NotFound(String),
    #[error("payload {hash} shredded at {shredded_at} ({reason})")]
    Shredded {
        hash: String,
        shredded_at: String,
        reason: String,
    },
}

/// Spec §1 PayloadRef — embedded in fabric objects in place of content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayloadRef {
    pub hash: String,
    pub size: u64,
    pub media_type: String,
    pub dek_id: String,
}

pub fn init(conn: &Connection) -> Result<(), PayloadError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS payloads (
            hash       TEXT PRIMARY KEY,
            size       INTEGER NOT NULL,
            media_type TEXT NOT NULL,
            dek_id     TEXT NOT NULL,
            nonce      BLOB NOT NULL,
            ciphertext BLOB NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS deks (
            dek_id     TEXT PRIMARY KEY,
            kek_id     TEXT NOT NULL,
            alg        TEXT NOT NULL,          -- SI-9: recorded per DEK
            wrap_nonce BLOB NOT NULL,
            wrapped    BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tombstones (
            hash        TEXT PRIMARY KEY,
            shredded_at TEXT NOT NULL,
            reason      TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// Store content; returns its PayloadRef. Idempotent per plaintext hash.
pub fn put(
    conn: &Connection,
    kek: &Kek,
    content: &[u8],
    media_type: &str,
    now: &str,
) -> Result<PayloadRef, PayloadError> {
    let hash = sha256_hex(content);
    if let Some(r) = lookup_ref(conn, &hash)? {
        return Ok(r);
    }
    let wd = kek.new_dek()?;
    let (nonce, ct) = dek_encrypt(&wd.dek, content)?;
    conn.execute(
        "INSERT INTO deks (dek_id, kek_id, alg, wrap_nonce, wrapped) VALUES (?1,?2,?3,?4,?5)",
        params![wd.dek_id, kek.kek_id, "aes-256-gcm", wd.wrap_nonce, wd.wrapped],
    )?;
    conn.execute(
        "INSERT INTO payloads (hash, size, media_type, dek_id, nonce, ciphertext, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![hash, content.len() as i64, media_type, wd.dek_id, nonce, ct, now],
    )?;
    Ok(PayloadRef {
        hash,
        size: content.len() as u64,
        media_type: media_type.into(),
        dek_id: wd.dek_id,
    })
}

fn lookup_ref(conn: &Connection, hash: &str) -> Result<Option<PayloadRef>, PayloadError> {
    Ok(conn
        .query_row(
            "SELECT hash, size, media_type, dek_id FROM payloads WHERE hash = ?1",
            [hash],
            |row| {
                Ok(PayloadRef {
                    hash: row.get(0)?,
                    size: row.get::<_, i64>(1)? as u64,
                    media_type: row.get(2)?,
                    dek_id: row.get(3)?,
                })
            },
        )
        .optional()?)
}

/// Resolve a payload to plaintext. A shredded payload returns the tombstone
/// as an error — structure persists, substance does not.
pub fn get(conn: &Connection, kek: &Kek, hash: &str) -> Result<Vec<u8>, PayloadError> {
    if let Some((shredded_at, reason)) = tombstone(conn, hash)? {
        return Err(PayloadError::Shredded {
            hash: hash.into(),
            shredded_at,
            reason,
        });
    }
    let row = conn
        .query_row(
            "SELECT p.nonce, p.ciphertext, d.wrap_nonce, d.wrapped
             FROM payloads p JOIN deks d ON d.dek_id = p.dek_id
             WHERE p.hash = ?1",
            [hash],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| PayloadError::NotFound(hash.into()))?;
    let dek = kek.unwrap_dek(&row.2, &row.3)?;
    Ok(dek_decrypt(&dek, &row.0, &row.1)?)
}

pub fn tombstone(
    conn: &Connection,
    hash: &str,
) -> Result<Option<(String, String)>, PayloadError> {
    Ok(conn
        .query_row(
            "SELECT shredded_at, reason FROM tombstones WHERE hash = ?1",
            [hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

/// Crypto-shred: destroy the wrapped DEK (and, belt-and-braces, the
/// ciphertext), leave a tombstone. The caller emits the `shred` trace event —
/// the ledger records *that* it forgot, never what.
pub fn shred(
    conn: &Connection,
    hash: &str,
    reason: &str,
    now: &str,
) -> Result<(), PayloadError> {
    let dek_id: Option<String> = conn
        .query_row("SELECT dek_id FROM payloads WHERE hash = ?1", [hash], |r| {
            r.get(0)
        })
        .optional()?;
    let dek_id = dek_id.ok_or_else(|| PayloadError::NotFound(hash.into()))?;
    conn.execute("DELETE FROM deks WHERE dek_id = ?1", [&dek_id])?;
    conn.execute(
        "UPDATE payloads SET ciphertext = x'' WHERE hash = ?1",
        [hash],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO tombstones (hash, shredded_at, reason) VALUES (?1,?2,?3)",
        params![hash, now, reason],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{Keystore};

    fn setup() -> (Connection, Kek) {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let kek = Keystore::open(dir.path()).unwrap().kek().unwrap();
        (conn, kek)
    }

    #[test]
    fn roundtrip_and_idempotent_put() {
        let (conn, kek) = setup();
        let r1 = put(&conn, &kek, b"hello vault", "text/plain", "t0").unwrap();
        let r2 = put(&conn, &kek, b"hello vault", "text/plain", "t1").unwrap();
        assert_eq!(r1, r2); // same plaintext → same ref, no duplicate DEK
        assert_eq!(get(&conn, &kek, &r1.hash).unwrap(), b"hello vault");
        assert!(r1.hash.starts_with("sha256:"));
    }

    #[test]
    fn shred_destroys_substance_keeps_structure() {
        let (conn, kek) = setup();
        let a = put(&conn, &kek, b"doomed", "text/plain", "t0").unwrap();
        let b = put(&conn, &kek, b"survivor", "text/plain", "t0").unwrap();
        shred(&conn, &a.hash, "ttl_expiry", "t9").unwrap();

        match get(&conn, &kek, &a.hash) {
            Err(PayloadError::Shredded { hash, reason, .. }) => {
                assert_eq!(hash, a.hash);
                assert_eq!(reason, "ttl_expiry");
            }
            other => panic!("expected tombstone, got {other:?}"),
        }
        // Unrelated payloads are untouched (per-payload DEKs).
        assert_eq!(get(&conn, &kek, &b.hash).unwrap(), b"survivor");
        // The hash row itself persists: lineage intact.
        assert!(lookup_ref(&conn, &a.hash).unwrap().is_some());
    }
}
