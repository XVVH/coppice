//! Snapshot coordinator. Brief §5.2 (Tier 1 — owned local state), spec §3
//! `state.roots` and §5.3 coherent revert.
//!
//! Content-addressed captures of filesystem trees and sqlite files into a
//! CAS directory. Because delegations happen at step boundaries where
//! nothing is in flight (brief §4), "atomic multi-root capture" degenerates
//! to: collect each store's current root, record the tuple in one manifest.
//! Revert is prepare-all-then-swap-all: every root is materialized to a
//! staging path first (any failure aborts with stores untouched), then all
//! swaps are simple renames. A crash mid-swap leaves recoverable staging
//! directories rather than a half-written store.
//!
//! sqlite roots hash the main db file bytes after a WAL TRUNCATE checkpoint
//! (SI-6). Fs trees are git-like: files only (empty dirs not tracked),
//! executable bit tracked, symlinks rejected — a vault should not contain
//! them, and following one silently would capture state outside the root.

use crate::canon::sha256_hex;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, thiserror::Error)]
pub enum SnapError {
    #[error("io at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("sqlite: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("symlink {0} unsupported in snapshot roots")]
    Symlink(PathBuf),
    #[error("blob {0} missing from CAS")]
    MissingBlob(String),
    #[error("malformed tree object {0}")]
    MalformedTree(String),
    #[error("store root {0} does not exist")]
    MissingRoot(PathBuf),
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> SnapError + '_ {
    move |source| SnapError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// What kind of store a root is. Spec §3 also names `branch` (Tier 2) and
/// `mirror` (Tier 3); those arrive with later milestones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    Fs,
    Sqlite,
}

impl StoreKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StoreKind::Fs => "fs",
            StoreKind::Sqlite => "sqlite",
        }
    }
}

/// A store registered with the coordinator.
#[derive(Debug, Clone)]
pub struct StoreSpec {
    /// Manifest store name, e.g. "fs:vault", "db:memory".
    pub store: String,
    pub tier: u8,
    pub kind: StoreKind,
    pub path: PathBuf,
}

/// Content-addressed blob store on disk.
pub struct Cas {
    dir: PathBuf,
}

impl Cas {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, SnapError> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir).map_err(io_err(&dir))?;
        Ok(Self { dir })
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        let hex = hash.strip_prefix("sha256:").unwrap_or(hash);
        self.dir.join(&hex[..2]).join(hex)
    }

    pub fn put(&self, content: &[u8]) -> Result<String, SnapError> {
        let hash = sha256_hex(content);
        let path = self.blob_path(&hash);
        if !path.exists() {
            let parent = path.parent().expect("blob path has parent");
            fs::create_dir_all(parent).map_err(io_err(parent))?;
            let tmp = path.with_extension("tmp");
            fs::write(&tmp, content).map_err(io_err(&tmp))?;
            fs::rename(&tmp, &path).map_err(io_err(&path))?;
        }
        Ok(hash)
    }

    pub fn get(&self, hash: &str) -> Result<Vec<u8>, SnapError> {
        let path = self.blob_path(hash);
        if !path.exists() {
            return Err(SnapError::MissingBlob(hash.into()));
        }
        fs::read(&path).map_err(io_err(&path))
    }
}

/// Capture a store's current state into the CAS; returns its root hash.
pub fn capture(cas: &Cas, spec: &StoreSpec) -> Result<String, SnapError> {
    match spec.kind {
        StoreKind::Fs => capture_fs(cas, &spec.path),
        StoreKind::Sqlite => capture_sqlite(cas, &spec.path),
    }
}

fn capture_fs(cas: &Cas, root: &Path) -> Result<String, SnapError> {
    if !root.is_dir() {
        return Err(SnapError::MissingRoot(root.to_path_buf()));
    }
    let mut entries: Vec<Value> = Vec::new();
    for entry in WalkDir::new(root).sort_by_file_name() {
        let entry = entry.map_err(|e| SnapError::Io {
            path: root.to_path_buf(),
            source: e.into(),
        })?;
        if entry.path_is_symlink() {
            return Err(SnapError::Symlink(entry.path().to_path_buf()));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let content = fs::read(entry.path()).map_err(io_err(entry.path()))?;
        let hash = cas.put(&content)?;
        let rel = entry
            .path()
            .strip_prefix(root)
            .expect("walkdir yields children of root")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let mode = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if entry.metadata().map_err(|e| SnapError::Io {
                    path: entry.path().to_path_buf(),
                    source: e.into(),
                })?.permissions().mode() & 0o111 != 0
                {
                    "755"
                } else {
                    "644"
                }
            }
            #[cfg(not(unix))]
            {
                "644"
            }
        };
        entries.push(json!({
            "path": rel,
            "mode": mode,
            "size": content.len(),
            "hash": hash,
        }));
    }
    // Deterministic order independent of walk order.
    entries.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let tree = json!({ "kind": "fs_tree", "entries": entries });
    let bytes = serde_json_canonicalizer::to_vec(&tree).expect("tree serializes");
    cas.put(&bytes)
}

fn capture_sqlite(cas: &Cas, db_path: &Path) -> Result<String, SnapError> {
    if !db_path.is_file() {
        return Err(SnapError::MissingRoot(db_path.to_path_buf()));
    }
    // Fold any WAL into the main file so the byte image is complete (SI-6).
    {
        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
    }
    let bytes = fs::read(db_path).map_err(io_err(db_path))?;
    cas.put(&bytes)
}

/// Load and parse a stored fs_tree object.
pub fn load_tree_object(cas: &Cas, root: &str) -> Result<Value, SnapError> {
    let bytes = cas.get(root)?;
    serde_json::from_slice(&bytes).map_err(|_| SnapError::MalformedTree(root.into()))
}

/// Materialize an fs tree root into `dest` (created; must not be live —
/// callers stage or branch, never write a live store directly).
pub fn materialize_fs(cas: &Cas, root: &str, dest: &Path) -> Result<(), SnapError> {
    let tree = load_tree_object(cas, root)?;
    let entries = tree["entries"]
        .as_array()
        .ok_or_else(|| SnapError::MalformedTree(root.into()))?;
    fs::create_dir_all(dest).map_err(io_err(dest))?;
    for e in entries {
        let (rel, hash) = (
            e["path"].as_str().ok_or_else(|| SnapError::MalformedTree(root.into()))?,
            e["hash"].as_str().ok_or_else(|| SnapError::MalformedTree(root.into()))?,
        );
        let target = dest.join(rel);
        if let Some(p) = target.parent() {
            fs::create_dir_all(p).map_err(io_err(p))?;
        }
        let content = cas.get(hash)?;
        fs::write(&target, &content).map_err(io_err(&target))?;
        #[cfg(unix)]
        if e["mode"].as_str() == Some("755") {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))
                .map_err(io_err(&target))?;
        }
    }
    Ok(())
}

/// Materialize a sqlite byte-image root as a file at `dest`.
pub fn materialize_sqlite(cas: &Cas, root: &str, dest: &Path) -> Result<(), SnapError> {
    let bytes = cas.get(root)?;
    if let Some(p) = dest.parent() {
        fs::create_dir_all(p).map_err(io_err(p))?;
    }
    fs::write(dest, &bytes).map_err(io_err(dest))?;
    Ok(())
}

/// Build (and store) an fs_tree object for a merged `path → hash` map,
/// pulling per-entry metadata (mode, size) from the source tree objects
/// in priority order. Every (path, hash) pair in a merge came from one of
/// the sources, so lookup cannot miss on well-formed input.
pub fn compose_tree(
    cas: &Cas,
    merged: &std::collections::BTreeMap<String, String>,
    sources: &[&Value],
) -> Result<String, SnapError> {
    let mut entries: Vec<Value> = Vec::new();
    'outer: for (path, hash) in merged {
        // Exact (path, hash) match first (covers moves/renames via the
        // side that has the new path), then any entry with this hash.
        for pass in 0..2 {
            for src in sources {
                for e in src["entries"].as_array().into_iter().flatten() {
                    let hash_match = e["hash"].as_str() == Some(hash.as_str());
                    let path_match = e["path"].as_str() == Some(path.as_str());
                    if hash_match && (path_match || pass == 1) {
                        let mut entry = e.clone();
                        entry["path"] = json!(path);
                        entries.push(entry);
                        continue 'outer;
                    }
                }
            }
        }
        return Err(SnapError::MalformedTree(format!(
            "merged entry {path}={hash} not found in any source tree"
        )));
    }
    entries.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let tree = json!({ "kind": "fs_tree", "entries": entries });
    cas.put(&serde_json_canonicalizer::to_vec(&tree).expect("tree serializes"))
}

/// A restore staged but not yet applied. Dropping it without `commit`
/// leaves the live store untouched.
pub struct PreparedRestore {
    spec: StoreSpec,
    staging: PathBuf,
}

/// Stage a store's restore to `root` next to the live path (same
/// filesystem, so commit is a rename).
pub fn prepare_restore(
    cas: &Cas,
    spec: &StoreSpec,
    root: &str,
) -> Result<PreparedRestore, SnapError> {
    let parent = spec
        .path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let staging = parent.join(format!(
        ".asf-staging-{}-{}",
        spec.path.file_name().unwrap_or_default().to_string_lossy(),
        &root.strip_prefix("sha256:").unwrap_or(root)[..12],
    ));
    if staging.exists() {
        if staging.is_dir() {
            fs::remove_dir_all(&staging).map_err(io_err(&staging))?;
        } else {
            fs::remove_file(&staging).map_err(io_err(&staging))?;
        }
    }
    match spec.kind {
        StoreKind::Fs => materialize_fs(cas, root, &staging)?,
        StoreKind::Sqlite => materialize_sqlite(cas, root, &staging)?,
    }
    Ok(PreparedRestore {
        spec: spec.clone(),
        staging,
    })
}

/// Swap a staged restore into place. Callers must have closed any open
/// connections to sqlite stores first.
pub fn commit_restore(prepared: PreparedRestore) -> Result<(), SnapError> {
    let live = &prepared.spec.path;
    match prepared.spec.kind {
        StoreKind::Fs => {
            let old = live.with_extension("asf-old");
            if old.exists() {
                fs::remove_dir_all(&old).map_err(io_err(&old))?;
            }
            if live.exists() {
                fs::rename(live, &old).map_err(io_err(live))?;
            }
            fs::rename(&prepared.staging, live).map_err(io_err(live))?;
            if old.exists() {
                fs::remove_dir_all(&old).map_err(io_err(&old))?;
            }
        }
        StoreKind::Sqlite => {
            fs::rename(&prepared.staging, live).map_err(io_err(live))?;
            // A restored image must not be polluted by a stale WAL/SHM.
            for ext in ["-wal", "-shm"] {
                let side = PathBuf::from(format!("{}{}", live.display(), ext));
                if side.exists() {
                    fs::remove_file(&side).map_err(io_err(&side))?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, content: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn fs_spec(path: &Path) -> StoreSpec {
        StoreSpec {
            store: "fs:vault".into(),
            tier: 1,
            kind: StoreKind::Fs,
            path: path.to_path_buf(),
        }
    }

    #[test]
    fn fs_capture_is_content_addressed_and_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let vault = tmp.path().join("vault");
        write(&vault.join("a.md"), "alpha");
        write(&vault.join("sub/b.md"), "beta");

        let r1 = capture(&cas, &fs_spec(&vault)).unwrap();
        let r2 = capture(&cas, &fs_spec(&vault)).unwrap();
        assert_eq!(r1, r2, "same content, same root");

        write(&vault.join("a.md"), "alpha2");
        let r3 = capture(&cas, &fs_spec(&vault)).unwrap();
        assert_ne!(r1, r3, "content change, root change");
    }

    #[test]
    fn fs_restore_roundtrip_including_deletions_and_additions() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let vault = tmp.path().join("vault");
        write(&vault.join("keep.md"), "original");
        write(&vault.join("doomed.md"), "will be deleted by agent");
        let spec = fs_spec(&vault);
        let root = capture(&cas, &spec).unwrap();

        // Agent run: modify, delete, add.
        write(&vault.join("keep.md"), "mutated");
        fs::remove_file(vault.join("doomed.md")).unwrap();
        write(&vault.join("new/added.md"), "agent addition");

        let prepared = prepare_restore(&cas, &spec, &root).unwrap();
        commit_restore(prepared).unwrap();

        assert_eq!(fs::read_to_string(vault.join("keep.md")).unwrap(), "original");
        assert_eq!(
            fs::read_to_string(vault.join("doomed.md")).unwrap(),
            "will be deleted by agent"
        );
        assert!(!vault.join("new").exists(), "additions rolled back");
        assert_eq!(capture(&cas, &spec).unwrap(), root, "root round-trips");
    }

    #[test]
    fn sqlite_capture_restore_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let db = tmp.path().join("memory.db");
        let spec = StoreSpec {
            store: "db:memory".into(),
            tier: 1,
            kind: StoreKind::Sqlite,
            path: db.clone(),
        };
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);
                 INSERT INTO notes (body) VALUES ('remember the milk');",
            )
            .unwrap();
        }
        let root = capture(&cas, &spec).unwrap();
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute("INSERT INTO notes (body) VALUES ('agent scribble')", [])
                .unwrap();
        }
        assert_ne!(capture(&cas, &spec).unwrap(), root);

        let prepared = prepare_restore(&cas, &spec, &root).unwrap();
        commit_restore(prepared).unwrap();

        let conn = Connection::open(&db).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "agent write rolled back");
        assert_eq!(capture(&cas, &spec).unwrap(), root);
    }

    #[test]
    fn prepare_failure_leaves_live_store_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let vault = tmp.path().join("vault");
        write(&vault.join("a.md"), "live");
        let spec = fs_spec(&vault);
        // Bogus root: prepare fails, live content untouched.
        assert!(prepare_restore(&cas, &spec, "sha256:deadbeefdeadbeef").is_err());
        assert_eq!(fs::read_to_string(vault.join("a.md")).unwrap(), "live");
    }

    #[test]
    fn symlinks_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let vault = tmp.path().join("vault");
        write(&vault.join("a.md"), "x");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hosts", vault.join("link")).unwrap();
            assert!(matches!(
                capture(&cas, &fs_spec(&vault)),
                Err(SnapError::Symlink(_))
            ));
        }
    }
}
