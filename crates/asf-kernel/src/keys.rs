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
    #[error("required key file is missing: {0}")]
    Missing(PathBuf),
    #[error("refusing to initialize over existing or partial keystore {0}")]
    AlreadyInitialized(PathBuf),
    #[error("malformed secret store {0}")]
    MalformedSecretStore(PathBuf),
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
    /// Initialize a new keystore, publishing all home-lifetime material as
    /// one directory entry. Refuses an existing or partially published
    /// keystore; recovery and rotation belong to SI-27/W-17.
    pub fn initialize(dir: impl AsRef<Path>) -> Result<Self, KeyError> {
        let dir = dir.as_ref().to_path_buf();
        match fs::symlink_metadata(&dir) {
            Ok(_) => return Err(KeyError::AlreadyInitialized(dir)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(KeyError::Io(error)),
        }
        let parent = dir.parent().ok_or_else(|| {
            KeyError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "keystore has no parent directory",
            ))
        })?;
        fs::create_dir_all(parent)?;

        let mut random = [0u8; 8];
        OsRng.fill_bytes(&mut random);
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("keys");
        let staging = parent.join(format!(".{name}.initialize-{}", hex::encode(random)));
        fs::create_dir(&staging)?;
        set_private_dir(&staging)?;

        let result: Result<(), KeyError> = (|| {
            for name in [
                Role::Fabric.file_name(),
                Role::UserRoot.file_name(),
                "owner.kek",
            ] {
                let mut bytes = Zeroizing::new([0u8; 32]);
                OsRng.fill_bytes(&mut bytes[..]);
                create_private_new(&staging.join(name), &bytes[..])?;
            }
            sync_dir(&staging)?;
            fs::rename(&staging, &dir)?;
            sync_dir(parent)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result?;
        Ok(Self { dir })
    }

    /// Open an existing keystore. Required home-lifetime material is checked
    /// eagerly so no authority-bearing operation can run on a partial view.
    pub fn open_existing(dir: impl AsRef<Path>) -> Result<Self, KeyError> {
        let dir = dir.as_ref().to_path_buf();
        let metadata = fs::symlink_metadata(&dir).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                KeyError::Missing(dir.clone())
            } else {
                KeyError::Io(error)
            }
        })?;
        if !metadata.is_dir() {
            return Err(KeyError::Malformed(dir));
        }
        let keystore = Self { dir };
        for name in [
            Role::Fabric.file_name(),
            Role::UserRoot.file_name(),
            "owner.kek",
        ] {
            w13_load_required_key(&keystore.dir.join(name), 32)?;
        }
        Ok(keystore)
    }

    fn load_required_raw(&self, name: &str, len: usize) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        let path = self.dir.join(name);
        w13_load_required_key(&path, len)
    }

    /// Load a signing key. Key creation occurs only during explicit
    /// initialization; reopen never creates replacement authority.
    pub fn signing_key(&self, role: Role) -> Result<SigningKey, KeyError> {
        let bytes = self.load_required_raw(role.file_name(), 32)?;
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
        let mut map = w13_load_secret_store(&path)?.unwrap_or_default();
        map.insert(name.into(), serde_json::Value::String(value.into()));
        let encoded = Zeroizing::new(serde_json::to_vec(&map).expect("serialize"));
        write_private_atomic(&path, &encoded)
    }

    pub fn secret_get(&self, name: &str) -> Result<Option<String>, KeyError> {
        let path = self.dir.join("secrets.json");
        Ok(w13_load_secret_store(&path)?
            .and_then(|map| map.get(name).and_then(|v| v.as_str()).map(str::to_string)))
    }

    /// The owner KEK (Stage 1: single-human, one KEK). §8.2's multi-actor
    /// key-distribution is mechanism-reserved, policy-deferred.
    pub fn kek(&self) -> Result<Kek, KeyError> {
        let bytes = self.load_required_raw("owner.kek", 32)?;
        let mut arr = Zeroizing::new([0u8; 32]);
        arr.copy_from_slice(&bytes);
        Ok(Kek {
            kek_id: format!("kek:{}", hex::encode(&sha2::Sha256::digest(*arr)[..8])),
            key: arr,
        })
    }
}

fn w13_load_required_key(path: &Path, len: usize) -> Result<Zeroizing<Vec<u8>>, KeyError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            KeyError::Missing(path.to_path_buf())
        } else {
            KeyError::Io(error)
        }
    })?;
    if !metadata.is_file() {
        return Err(KeyError::Malformed(path.to_path_buf()));
    }
    let bytes = fs::read(path)?;
    if bytes.len() != len {
        return Err(KeyError::Malformed(path.to_path_buf()));
    }
    Ok(Zeroizing::new(bytes))
}

fn w13_load_secret_store(
    path: &Path,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, KeyError> {
    let parent = path.parent().ok_or_else(|| {
        KeyError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret store has no parent directory",
        ))
    })?;
    let name = path.file_name().ok_or_else(|| {
        KeyError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret store has no file name",
        ))
    })?;
    let mut found = None;
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if entry.file_name() == name {
            found = Some(entry);
            break;
        }
    }
    let Some(entry) = found else {
        return Ok(None);
    };
    if entry.file_type()?.is_symlink() {
        return Err(KeyError::MalformedSecretStore(path.to_path_buf()));
    }
    let metadata = entry.metadata()?;
    if !metadata.is_file() {
        return Err(KeyError::MalformedSecretStore(path.to_path_buf()));
    }
    Ok(Some(w13_parse_secret_store(path, &fs::read(path)?)?))
}

fn w13_parse_secret_store(
    path: &Path,
    bytes: &[u8],
) -> Result<serde_json::Map<String, serde_json::Value>, KeyError> {
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(bytes)
        .map_err(|_| KeyError::MalformedSecretStore(path.to_path_buf()))?;
    if map.values().any(|value| !value.is_string()) {
        return Err(KeyError::MalformedSecretStore(path.to_path_buf()));
    }
    Ok(map)
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

fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), KeyError> {
    let mut random = [0u8; 8];
    OsRng.fill_bytes(&mut random);
    let tmp = path.with_extension(format!("tmp-{}", hex::encode(random)));
    create_private_new(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

fn sync_dir(path: &Path) -> Result<(), KeyError> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()?;
    }
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
    fn key_continuity_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys");
        let ks1 = Keystore::initialize(&path).unwrap();
        let vk1 = ks1.verifying_key(Role::UserRoot).unwrap();
        let fabric1 = ks1.verifying_key(Role::Fabric).unwrap();
        let kek1 = ks1.kek().unwrap().kek_id;
        let ks2 = Keystore::open_existing(&path).unwrap();
        assert_eq!(vk1, ks2.verifying_key(Role::UserRoot).unwrap());
        assert_eq!(fabric1, ks2.verifying_key(Role::Fabric).unwrap());
        assert_eq!(kek1, ks2.kek().unwrap().kek_id);
        assert_ne!(vk1, fabric1);
    }

    #[cfg(unix)]
    #[test]
    fn initialized_keystore_material_is_complete_and_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        Keystore::initialize(&keys).unwrap();
        assert_eq!(fs::metadata(&keys).unwrap().permissions().mode() & 0o777, 0o700);
        for name in ["fabric.ed25519", "user_root.ed25519", "owner.kek"] {
            let metadata = fs::metadata(keys.join(name)).unwrap();
            assert!(metadata.is_file());
            assert_eq!(metadata.len(), 32);
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn concurrent_initialization_publishes_one_complete_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    Keystore::initialize(path).is_ok()
                })
            })
            .collect();
        let successes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|success| *success)
            .count();
        assert_eq!(successes, 1);
        let ks = Keystore::open_existing(&path).unwrap();
        ks.signing_key(Role::Fabric).unwrap();
        ks.signing_key(Role::UserRoot).unwrap();
        ks.kek().unwrap();
        assert!(fs::read_dir(dir.path())
            .unwrap()
            .all(|entry| entry.unwrap().file_name() == "keys"));
    }

    #[test]
    fn missing_or_malformed_required_key_never_regenerates() {
        let dir = tempfile::tempdir().unwrap();
        for (case, malformed) in [("missing", false), ("malformed", true)] {
            for lost in ["fabric.ed25519", "user_root.ed25519", "owner.kek"] {
                let path = dir.path().join(format!("{case}-{lost}"));
                Keystore::initialize(&path).unwrap();
                let preserved: Vec<_> = ["fabric.ed25519", "user_root.ed25519", "owner.kek"]
                    .into_iter()
                    .filter(|name| *name != lost)
                    .map(|name| (name, fs::read(path.join(name)).unwrap()))
                    .collect();
                if malformed {
                    fs::write(path.join(lost), [0x5a; 31]).unwrap();
                } else {
                    fs::remove_file(path.join(lost)).unwrap();
                }

                assert!(Keystore::open_existing(&path).is_err());
                if malformed {
                    assert_eq!(fs::read(path.join(lost)).unwrap(), [0x5a; 31]);
                } else {
                    assert!(!path.join(lost).exists());
                }
                for (name, bytes) in preserved {
                    assert_eq!(fs::read(path.join(name)).unwrap(), bytes);
                }
            }
        }

        let directory_case = dir.path().join("directory-key");
        Keystore::initialize(&directory_case).unwrap();
        let owner = directory_case.join("owner.kek");
        fs::remove_file(&owner).unwrap();
        fs::create_dir(&owner).unwrap();
        assert!(matches!(
            Keystore::open_existing(&directory_case),
            Err(KeyError::Malformed(path)) if path == owner
        ));
        assert!(owner.is_dir());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let symlink_case = dir.path().join("symlink-key");
            Keystore::initialize(&symlink_case).unwrap();
            let fabric = symlink_case.join("fabric.ed25519");
            fs::remove_file(&fabric).unwrap();
            symlink(symlink_case.join("user_root.ed25519"), &fabric).unwrap();
            assert!(matches!(
                Keystore::open_existing(&symlink_case),
                Err(KeyError::Malformed(path)) if path == fabric
            ));
            assert!(fs::symlink_metadata(&fabric).unwrap().file_type().is_symlink());

            let target = dir.path().join("symlink-directory-target");
            Keystore::initialize(&target).unwrap();
            let link = dir.path().join("symlink-directory");
            symlink(&target, &link).unwrap();
            assert!(matches!(
                Keystore::open_existing(&link),
                Err(KeyError::Malformed(path)) if path == link
            ));
            assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        }
    }

    #[test]
    fn partial_keystore_initialization_never_mints_missing_material() {
        let dir = tempfile::tempdir().unwrap();
        for present in [0usize, 1, 2] {
            let path = dir.path().join(format!("partial-{present}"));
            fs::create_dir(&path).unwrap();
            let names = ["fabric.ed25519", "user_root.ed25519", "owner.kek"];
            for (index, name) in names.iter().enumerate().take(present) {
                fs::write(path.join(name), [index as u8 + 1; 32]).unwrap();
            }
            let before: Vec<_> = fs::read_dir(&path)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (entry.file_name(), fs::read(entry.path()).unwrap())
                })
                .collect();

            assert!(matches!(
                Keystore::initialize(&path),
                Err(KeyError::AlreadyInitialized(_))
            ));
            let after: Vec<_> = fs::read_dir(&path)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (entry.file_name(), fs::read(entry.path()).unwrap())
                })
                .collect();
            assert_eq!(before, after, "partial case {present} was mutated");
        }
    }

    #[test]
    fn valid_secret_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys");
        let ks = Keystore::initialize(&path).unwrap();
        ks.secret_set("first", "alpha").unwrap();
        ks.secret_set("second", "beta").unwrap();
        assert_eq!(ks.secret_get("first").unwrap().as_deref(), Some("alpha"));
        assert_eq!(ks.secret_get("second").unwrap().as_deref(), Some("beta"));
    }

    #[test]
    fn malformed_secret_store_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        for (case, malformed) in [
            ("syntax", b"{not-json".as_slice()),
            ("shape", b"[\"not\",\"an\",\"object\"]".as_slice()),
            ("value", b"{\"credential\":42}".as_slice()),
        ] {
            let path = dir.path().join(case);
            let ks = Keystore::initialize(&path).unwrap();
            let secrets = path.join("secrets.json");
            fs::write(&secrets, malformed).unwrap();

            assert!(matches!(
                ks.secret_get("credential"),
                Err(KeyError::MalformedSecretStore(_))
            ));
            assert!(matches!(
                ks.secret_set("credential", "replacement"),
                Err(KeyError::MalformedSecretStore(_))
            ));
            assert_eq!(fs::read(&secrets).unwrap(), malformed);
        }


        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let path = dir.path().join("symlink");
            let ks = Keystore::initialize(&path).unwrap();
            let secrets = path.join("secrets.json");
            symlink(path.join("missing-target"), &secrets).unwrap();
            assert!(matches!(
                ks.secret_get("credential"),
                Err(KeyError::MalformedSecretStore(_))
            ));
            assert!(matches!(
                ks.secret_set("credential", "replacement"),
                Err(KeyError::MalformedSecretStore(_))
            ));
            assert!(fs::symlink_metadata(&secrets)
                .unwrap()
                .file_type()
                .is_symlink());
        }
    }

    #[test]
    fn dek_wrap_roundtrip_and_shred_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let ks = Keystore::initialize(dir.path().join("keys")).unwrap();
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
