//! Trace substrate. Spec §6, brief §5.1.
//!
//! Append-only sqlite event log: hash-chained per span (`prev` + `seq`),
//! signed by the emitting component (Stage 1: the fabric key, SI-3). The
//! global rowid is the substrate offset used by `captured_before` (§3.1) and
//! `trace.substrate_offset` (§3). Append-only is enforced in-database with
//! triggers; verification recomputes ids and signatures from raw bytes, so
//! any post-hoc edit is detectable even if the triggers are bypassed.
//!
//! The substrate also stores fabric objects (principals, channels, intents,
//! manifests) — brief §5.1 counts these among the records the fabric itself
//! emits and fully owns.

use crate::canon::{self, CanonError};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{Map, Value};

#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("sqlite: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("canon: {0}")]
    Canon(#[from] CanonError),
    #[error("unknown event kind {0}")]
    UnknownKind(String),
    #[error("span {span} chain broken at seq {seq}: {detail}")]
    ChainBroken {
        span: String,
        seq: i64,
        detail: String,
    },
    #[error("object {0} not found")]
    ObjectNotFound(String),
}

/// Spec §6 event kinds, plus Stage-1 extensions flagged as SI-11:
/// `register` (principal/channel/tool object registration) and
/// `intent` (intent capture — the substrate countersign of SI-10).
pub const EVENT_KINDS: &[&str] = &[
    "tool_call",
    "verdict",
    "escalation",
    "ratification",
    "promotion",
    "revert",
    "compensation",
    "grant",
    "expiry",
    "snapshot",
    "drift",
    "shred",
    "amendment",
    "remanifest",
    // SI-11 extensions (Option A ratified 2026-07-08):
    "register",
    "intent",
    // SI-13 extension: escalation resolution (C1-stamped human approval/denial).
    "approval",
];

pub fn init(conn: &Connection) -> Result<(), TraceError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS events (
            offset   INTEGER PRIMARY KEY AUTOINCREMENT,
            id       TEXT NOT NULL UNIQUE,
            span     TEXT NOT NULL,
            seq      INTEGER NOT NULL,
            prev     TEXT,
            manifest TEXT,
            at       TEXT NOT NULL,
            kind     TEXT NOT NULL,
            raw      TEXT NOT NULL,
            UNIQUE (span, seq)
        );
        CREATE INDEX IF NOT EXISTS events_span ON events (span, seq);
        CREATE TRIGGER IF NOT EXISTS events_append_only_u
            BEFORE UPDATE ON events
            BEGIN SELECT RAISE(ABORT, 'trace substrate is append-only'); END;
        CREATE TRIGGER IF NOT EXISTS events_append_only_d
            BEFORE DELETE ON events
            BEGIN SELECT RAISE(ABORT, 'trace substrate is append-only'); END;
        CREATE TABLE IF NOT EXISTS objects (
            id         TEXT PRIMARY KEY,
            kind       TEXT NOT NULL,
            created_at TEXT NOT NULL,
            raw        TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// Mint a fresh span id.
pub fn new_span() -> String {
    let mut b = [0u8; 16];
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(&mut b);
    format!("span:{}", hex::encode(b))
}

#[derive(Debug, Clone)]
pub struct Appended {
    pub id: String,
    pub offset: i64,
    pub span: String,
    pub seq: i64,
}

/// Current head of the global substrate (max offset; 0 if empty).
pub fn head_offset(conn: &Connection) -> Result<i64, TraceError> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(offset), 0) FROM events",
        [],
        |r| r.get(0),
    )?)
}

/// Append a signed event to a span's chain. `seq` starts at 0 with
/// `prev: null` (SI-4); `manifest` is nullable for substrate-level events
/// (SI-5). The whole operation is one transaction: chain-tip read and insert
/// cannot interleave.
pub fn append(
    conn: &mut Connection,
    sk: &SigningKey,
    span: &str,
    manifest: Option<&str>,
    kind: &str,
    body: Value,
    at: &str,
) -> Result<Appended, TraceError> {
    if !EVENT_KINDS.contains(&kind) {
        return Err(TraceError::UnknownKind(kind.into()));
    }
    let tx = conn.transaction()?;
    let tip: Option<(String, i64)> = tx
        .query_row(
            "SELECT id, seq FROM events WHERE span = ?1 ORDER BY seq DESC LIMIT 1",
            [span],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (prev, seq) = match tip {
        Some((prev_id, prev_seq)) => (Value::String(prev_id), prev_seq + 1),
        None => (Value::Null, 0),
    };

    let mut obj = Map::new();
    obj.insert("span".into(), Value::String(span.into()));
    obj.insert("seq".into(), Value::from(seq));
    obj.insert("prev".into(), prev.clone());
    obj.insert(
        "manifest".into(),
        manifest.map(|m| Value::String(m.into())).unwrap_or(Value::Null),
    );
    obj.insert("at".into(), Value::String(at.into()));
    obj.insert("kind".into(), Value::String(kind.into()));
    obj.insert("body".into(), body);
    let sealed = canon::seal("evt", obj, sk)?;
    let id = sealed["id"].as_str().expect("sealed").to_string();
    let raw = serde_json::to_string(&Value::Object(sealed)).expect("serialize");

    tx.execute(
        "INSERT INTO events (id, span, seq, prev, manifest, at, kind, raw)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            id,
            span,
            seq,
            prev.as_str(),
            manifest,
            at,
            kind,
            raw
        ],
    )?;
    let offset = tx.last_insert_rowid();
    tx.commit()?;
    Ok(Appended {
        id,
        offset,
        span: span.into(),
        seq,
    })
}

/// A decoded event row.
#[derive(Debug, Clone)]
pub struct EventRow {
    pub offset: i64,
    pub id: String,
    pub span: String,
    pub seq: i64,
    pub prev: Option<String>,
    pub manifest: Option<String>,
    pub at: String,
    pub kind: String,
    pub raw: Value,
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        offset: row.get(0)?,
        id: row.get(1)?,
        span: row.get(2)?,
        seq: row.get(3)?,
        prev: row.get(4)?,
        manifest: row.get(5)?,
        at: row.get(6)?,
        kind: row.get(7)?,
        raw: serde_json::from_str(&row.get::<_, String>(8)?).unwrap_or(Value::Null),
    })
}

const EVENT_COLS: &str = "offset, id, span, seq, prev, manifest, at, kind, raw";

pub fn events_in_span(conn: &Connection, span: &str) -> Result<Vec<EventRow>, TraceError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EVENT_COLS} FROM events WHERE span = ?1 ORDER BY seq"
    ))?;
    let rows = stmt.query_map([span], row_to_event)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn all_events(conn: &Connection) -> Result<Vec<EventRow>, TraceError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EVENT_COLS} FROM events ORDER BY offset"
    ))?;
    let rows = stmt.query_map([], row_to_event)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Verify a span's chain end-to-end: seq contiguity from 0, prev-linkage,
/// id recomputation from raw bytes, and signature by the emitting component.
/// Returns the number of verified events.
pub fn verify_span(
    conn: &Connection,
    vk: &VerifyingKey,
    span: &str,
) -> Result<usize, TraceError> {
    let events = events_in_span(conn, span)?;
    let mut expected_prev: Option<String> = None;
    for (i, ev) in events.iter().enumerate() {
        let fail = |detail: String| TraceError::ChainBroken {
            span: span.into(),
            seq: ev.seq,
            detail,
        };
        if ev.seq != i as i64 {
            return Err(fail(format!("expected seq {i}, found {}", ev.seq)));
        }
        // prev-link: the raw object's prev must equal the previous event id.
        let raw_prev = ev.raw.get("prev").cloned().unwrap_or(Value::Null);
        let want = expected_prev
            .as_ref()
            .map(|p| Value::String(p.clone()))
            .unwrap_or(Value::Null);
        if raw_prev != want {
            return Err(fail(format!("prev {raw_prev} != expected {want}")));
        }
        // Row columns must agree with the signed raw object (a tampered
        // index column can't misrepresent the chain).
        if ev.raw.get("span").and_then(Value::as_str) != Some(span)
            || ev.raw.get("seq").and_then(Value::as_i64) != Some(ev.seq)
            || ev.raw.get("kind").and_then(Value::as_str) != Some(ev.kind.as_str())
            || ev.raw.get("id").and_then(Value::as_str) != Some(ev.id.as_str())
        {
            return Err(fail("index columns disagree with signed object".into()));
        }
        canon::verify(&ev.raw, vk).map_err(|e| fail(e.to_string()))?;
        expected_prev = Some(ev.id.clone());
    }
    Ok(events.len())
}

// ---- objects ----------------------------------------------------------

/// Store a sealed fabric object (principal, channel, intent, manifest…).
pub fn put_object(
    conn: &Connection,
    kind: &str,
    sealed: &Map<String, Value>,
    now: &str,
) -> Result<String, TraceError> {
    let id = sealed
        .get("id")
        .and_then(Value::as_str)
        .expect("sealed object has id")
        .to_string();
    let raw = serde_json::to_string(&Value::Object(sealed.clone())).expect("serialize");
    conn.execute(
        "INSERT OR IGNORE INTO objects (id, kind, created_at, raw) VALUES (?1,?2,?3,?4)",
        params![id, kind, now, raw],
    )?;
    Ok(id)
}

pub fn get_object(conn: &Connection, id: &str) -> Result<Value, TraceError> {
    conn.query_row("SELECT raw FROM objects WHERE id = ?1", [id], |r| {
        r.get::<_, String>(0)
    })
    .optional()?
    .map(|raw| serde_json::from_str(&raw).expect("stored objects are valid JSON"))
    .ok_or_else(|| TraceError::ObjectNotFound(id.into()))
}

pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>, TraceError> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), TraceError> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn setup() -> (Connection, SigningKey) {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        (conn, SigningKey::generate(&mut OsRng))
    }

    #[test]
    fn chain_links_and_offsets_are_global() {
        let (mut conn, sk) = setup();
        let s1 = new_span();
        let s2 = new_span();
        let a = append(&mut conn, &sk, &s1, None, "snapshot", serde_json::json!({"n":1}), "t1").unwrap();
        let b = append(&mut conn, &sk, &s2, None, "snapshot", serde_json::json!({"n":2}), "t2").unwrap();
        let c = append(&mut conn, &sk, &s1, Some("man:x"), "tool_call", serde_json::json!({"n":3}), "t3").unwrap();
        // Per-span seq; global offset.
        assert_eq!((a.seq, b.seq, c.seq), (0, 0, 1));
        assert_eq!((a.offset, b.offset, c.offset), (1, 2, 3));
        assert_eq!(head_offset(&conn).unwrap(), 3);
        // Head has prev null; second links to first (SI-4).
        let evs = events_in_span(&conn, &s1).unwrap();
        assert_eq!(evs[0].raw["prev"], Value::Null);
        assert_eq!(evs[1].raw["prev"], Value::String(a.id.clone()));
        assert_eq!(verify_span(&conn, &sk.verifying_key(), &s1).unwrap(), 2);
        assert_eq!(verify_span(&conn, &sk.verifying_key(), &s2).unwrap(), 1);
    }

    #[test]
    fn unknown_kind_rejected() {
        let (mut conn, sk) = setup();
        let s = new_span();
        assert!(matches!(
            append(&mut conn, &sk, &s, None, "made_up", serde_json::json!({}), "t"),
            Err(TraceError::UnknownKind(_))
        ));
    }

    #[test]
    fn append_only_enforced_by_triggers() {
        let (mut conn, sk) = setup();
        let s = new_span();
        append(&mut conn, &sk, &s, None, "snapshot", serde_json::json!({}), "t").unwrap();
        assert!(conn
            .execute("UPDATE events SET kind = 'revert' WHERE seq = 0", [])
            .is_err());
        assert!(conn.execute("DELETE FROM events", []).is_err());
    }

    #[test]
    fn tamper_after_bypassing_triggers_is_detected() {
        let (mut conn, sk) = setup();
        let s = new_span();
        append(&mut conn, &sk, &s, None, "snapshot", serde_json::json!({"v":1}), "t1").unwrap();
        append(&mut conn, &sk, &s, None, "tool_call", serde_json::json!({"v":2}), "t2").unwrap();
        // An attacker with raw db access drops the triggers and edits history.
        conn.execute_batch(
            "DROP TRIGGER events_append_only_u;
             UPDATE events SET raw = replace(raw, '\"v\":2', '\"v\":999') WHERE seq = 1;",
        )
        .unwrap();
        let err = verify_span(&conn, &sk.verifying_key(), &s).unwrap_err();
        assert!(matches!(err, TraceError::ChainBroken { seq: 1, .. }));
    }

    #[test]
    fn deleting_a_middle_event_breaks_the_chain() {
        let (mut conn, sk) = setup();
        let s = new_span();
        for i in 0..3 {
            append(&mut conn, &sk, &s, None, "snapshot", serde_json::json!({"i":i}), "t").unwrap();
        }
        conn.execute_batch("DROP TRIGGER events_append_only_d; DELETE FROM events WHERE seq = 1;")
            .unwrap();
        assert!(verify_span(&conn, &sk.verifying_key(), &s).is_err());
    }
}
