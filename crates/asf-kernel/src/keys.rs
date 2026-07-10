//! Key hierarchy and KEK/DEK wrapping. Spec §8.1, §8.2.
//!
//! Stage 1 keeps device-held keys as files in a keystore directory (0600):
//! `user_root` signs Principals and Intents; `fabric` (the coordinator — the
//! broker key of §8.1 once the daemon exists, SI-3) signs events, manifests,
//! channels; agent instance keys sign runtime attestations (M4, milestone 2).
//!
//! Payload encryption: per-payload 256-bit DEKs, AES-256-GCM, wrapped to an
//! owner KEK (SI-9). Crypto-shredding destroys the wrapped DEK row.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng as AeadOsRng},
    AeadCore, Aes256Gcm, Key, Nonce,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed key file {0}")]
    Malformed(PathBuf),
    #[error("aead failure (wrong KEK or corrupted ciphertext)")]
    Aead,
}

/// Well-known signing roles in the Stage 1 keystore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    UserRoot,
    Fabric,
    AgentInstance,
}

impl Role {
    fn file_name(self) -> &'static str {
        match self {
            Role::UserRoot => "user_root.ed25519",
            Role::Fabric => "fabric.ed25519",
            Role::AgentInstance => "agent_instance.ed25519",
        }
    }
}

/// File-backed keystore. Signing keys and the KEK live here; nothing in this
/// directory is ever placed in an agent's context (brief principle 4).
pub struct Keystore {
    dir: PathBuf,
}

impl Keystore {
    /// Open (creating if absent) a keystore at `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, KeyError> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        set_private_dir(&dir)?;
        Ok(Self { dir })
    }

    fn load_or_create_raw(&self, name: &str, len: usize) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        let path = self.dir.join(name);
        match fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() != len {
                    return Err(KeyError::Malformed(path));
                }
                return Ok(Zeroizing::new(bytes));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(KeyError::Io(e)),
        }
        let mut bytes = Zeroizing::new(vec![0u8; len]);
        OsRng.fill_bytes(&mut bytes);
        match publish_private_new(&path, &bytes) {
            Ok(()) => Ok(bytes),
            // Another broker may have won first-start initialization. Never
            // overwrite its key: load the winner and discard our candidate.
            Err(KeyError::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = fs::read(&path)?;
                if existing.len() != len {
                    return Err(KeyError::Malformed(path));
                }
                Ok(Zeroizing::new(existing))
            }
            Err(e) => Err(e),
        }
    }

    /// Load (or generate on first use) the signing key for a role.
    pub fn signing_key(&self, role: Role) -> Result<SigningKey, KeyError> {
        let bytes = self.load_or_create_raw(role.file_name(), 32)?;
        let mut arr = Zeroizing::new([0u8; 32]);
        arr.copy_from_slice(&bytes);
        Ok(SigningKey::from_bytes(&arr))
    }

    pub fn verifying_key(&self, role: Role) -> Result<VerifyingKey, KeyError> {
        Ok(self.signing_key(role)?.verifying_key())
    }

    /// Store a real credential in the vault (brief §5.3: secrets live here
    /// and are injected at call time; the agent never sees them).
    pub fn secret_set(&self, name: &str, value: &str) -> Result<(), KeyError> {
        let path = self.dir.join("secrets.json");
        let mut map: serde_json::Map<String, serde_json::Value> = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?).unwrap_or_default()
        } else {
            Default::default()
        };
        map.insert(name.into(), serde_json::Value::String(value.into()));
        let encoded = Zeroizing::new(serde_json::to_vec(&map).expect("serialize"));
        write_private_atomic(&path, &encoded)
    }

    pub fn secret_get(&self, name: &str) -> Result<Option<String>, KeyError> {
        let path = self.dir.join("secrets.json");
        if !path.exists() {
            return Ok(None);
        }
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice(&fs::read(&path)?).unwrap_or_default();
        Ok(map.get(name).and_then(|v| v.as_str()).map(str::to_string))
    }

    /// The owner KEK (Stage 1: single-human, one KEK). §8.2's multi-actor
    /// key-distribution is mechanism-reserved, policy-deferred.
    pub fn kek(&self) -> Result<Kek, KeyError> {
        let bytes = self.load_or_create_raw("owner.kek", 32)?;
        let mut arr = Zeroizing::new([0u8; 32]);
        arr.copy_from_slice(&bytes);
        Ok(Kek {
            kek_id: format!("kek:{}", hex::encode(&sha2::Sha256::digest(*arr)[..8])),
            key: arr,
        })
    }
}

fn set_private_dir(path: &Path) -> Result<(), KeyError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn create_private_new(path: &Path, bytes: &[u8]) -> Result<(), KeyError> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Publish a fully written private file without ever replacing an existing
/// winner. Linking the complete temporary inode makes target visibility atomic;
/// unlike direct O_EXCL creation, readers cannot observe a zero-length window.
fn publish_private_new(path: &Path, bytes: &[u8]) -> Result<(), KeyError> {
    let mut random = [0u8; 8];
    OsRng.fill_bytes(&mut random);
    let tmp = path.with_extension(format!("new-{}", hex::encode(random)));
    create_private_new(&tmp, bytes)?;
    let published = fs::hard_link(&tmp, path);
    let cleanup = fs::remove_file(&tmp);
    if let Err(e) = published {
        return Err(KeyError::Io(e));
    }
    cleanup?;
    Ok(())
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), KeyError> {
    let mut random = [0u8; 8];
    OsRng.fill_bytes(&mut random);
    let tmp = path.with_extension(format!("tmp-{}", hex::encode(random)));
    create_private_new(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

use sha2::Digest;

/// A key-encryption key that wraps per-payload DEKs.
pub struct Kek {
    key: Zeroizing<[u8; 32]>,
    pub kek_id: String,
}

/// A freshly generated data-encryption key, plus its wrapped form for storage.
pub struct WrappedDek {
    pub dek_id: String,
    pub wrap_nonce: Vec<u8>,
    pub wrapped: Vec<u8>,
    /// Plaintext DEK — use immediately, never persist.
    pub dek: Zeroizing<[u8; 32]>,
}

impl Kek {
    /// Generate a fresh DEK and wrap it (AES-256-GCM, SI-9).
    pub fn new_dek(&self) -> Result<WrappedDek, KeyError> {
        let mut dek = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut dek[..]);
        let dek_id = format!("dek:{}", hex::encode(&sha2::Sha256::digest(*dek)[..12]));
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key[..]));
        let nonce = Aes256Gcm::generate_nonce(&mut AeadOsRng);
        let wrapped = cipher
            .encrypt(&nonce, dek.as_slice())
            .map_err(|_| KeyError::Aead)?;
        Ok(WrappedDek {
            dek_id,
            wrap_nonce: nonce.to_vec(),
            wrapped,
            dek,
        })
    }

    /// Unwrap a stored DEK.
    pub fn unwrap_dek(
        &self,
        wrap_nonce: &[u8],
        wrapped: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, KeyError> {
        if wrap_nonce.len() != 12 {
            return Err(KeyError::Aead);
        }
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key[..]));
        let plain = Zeroizing::new(cipher
            .decrypt(Nonce::from_slice(wrap_nonce), wrapped)
            .map_err(|_| KeyError::Aead)?);
        let mut dek = Zeroizing::new([0u8; 32]);
        if plain.len() != dek.len() {
            return Err(KeyError::Aead);
        }
        dek.copy_from_slice(&plain);
        Ok(dek)
    }
}

/// AES-256-GCM encrypt with a DEK. Returns (nonce, ciphertext).
pub fn dek_encrypt(dek: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), KeyError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek));
    let nonce = Aes256Gcm::generate_nonce(&mut AeadOsRng);
    let ct = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| KeyError::Aead)?;
    Ok((nonce.to_vec(), ct))
}

pub fn dek_decrypt(dek: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, KeyError> {
    if nonce.len() != 12 {
        return Err(KeyError::Aead);
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek));
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| KeyError::Aead)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_persist_across_opens() {
        let dir = tempfile::tempdir().unwrap();
        let ks1 = Keystore::open(dir.path()).unwrap();
        let vk1 = ks1.verifying_key(Role::UserRoot).unwrap();
        let ks2 = Keystore::open(dir.path()).unwrap();
        assert_eq!(vk1, ks2.verifying_key(Role::UserRoot).unwrap());
        assert_ne!(vk1, ks2.verifying_key(Role::Fabric).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn keystore_and_new_key_permissions_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        let ks = Keystore::open(&keys).unwrap();
        ks.signing_key(Role::Fabric).unwrap();
        assert_eq!(fs::metadata(&keys).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(
            fs::metadata(keys.join("fabric.ed25519"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn concurrent_first_open_converges_on_one_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let ks = Keystore::open(path).unwrap();
                    barrier.wait();
                    ks.verifying_key(Role::Fabric).unwrap()
                })
            })
            .collect();
        let keys: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(keys[0], keys[1]);
    }

    #[test]
    fn dek_wrap_roundtrip_and_shred_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let ks = Keystore::open(dir.path()).unwrap();
        let kek = ks.kek().unwrap();
        let wd = kek.new_dek().unwrap();
        let (nonce, ct) = dek_encrypt(&wd.dek, b"secret payload").unwrap();

        // Unwrap from storage form and decrypt.
        let dek = kek.unwrap_dek(&wd.wrap_nonce, &wd.wrapped).unwrap();
        assert_eq!(dek_decrypt(&dek, &nonce, &ct).unwrap(), b"secret payload");

        // A different DEK cannot decrypt the ciphertext. The storage layer's
        // separate forensic-erasure problem is deliberately outside this
        // primitive test.
        let other = kek.new_dek().unwrap();
        assert!(dek_decrypt(&other.dek, &nonce, &ct).is_err());
    }
}
