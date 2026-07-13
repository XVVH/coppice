//! Snapshot coordinator. Brief §5.2 (Tier 1 — owned local state), spec §3
//! `state.roots` and §5.3 coherent revert.
//!
//! Content-addressed captures of filesystem trees and sqlite files into a
//! CAS directory. Because delegations happen at step boundaries where
//! nothing is in flight (brief §4), "atomic multi-root capture" degenerates
//! to: collect each store's current root, record the tuple in one manifest.
//! Restore is prepare-all-then-commit-all: every store's plan is validated
//! first (tree parse + CAS integrity, with immutable verified bytes staged;
//! any failure aborts with stores untouched), then committed. Sqlite commits
//! are staging renames; fs
//! commits apply IN PLACE (RF-10) — only differing files are written, so
//! unchanged files keep their mtimes/inodes and a promotion touches
//! exactly what it changed.
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
    #[error("invalid sha256 CAS address {0}")]
    InvalidHash(String),
    #[error("CAS integrity failure: requested {expected}, read {observed}")]
    HashMismatch { expected: String, observed: String },
    #[error("malformed tree object {0}")]
    MalformedTree(String),
    #[error("unsafe path {path:?} in tree object {root}")]
    UnsafePath { root: String, path: String },
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(io_err(&dir))?;
        }
        Ok(Self { dir })
    }

    /// Open an already-published CAS without creating or repairing it.
    /// Existing-home reopen must not silently complete a partial fabric.
    pub fn open_existing(dir: impl AsRef<Path>) -> Result<Self, SnapError> {
        let dir = dir.as_ref().to_path_buf();
        w13_validate_existing_cas(&dir)?;
        Ok(Self { dir })
    }

    fn blob_path(&self, hash: &str) -> Result<PathBuf, SnapError> {
        let Some(raw) = hash.strip_prefix("sha256:") else {
            return Err(SnapError::InvalidHash(hash.into()));
        };
        let decoded = hex::decode(raw).map_err(|_| SnapError::InvalidHash(hash.into()))?;
        if decoded.len() != 32 {
            return Err(SnapError::InvalidHash(hash.into()));
        }
        let canonical = hex::encode(decoded);
        Ok(self.dir.join(&canonical[..2]).join(canonical))
    }

    pub fn put(&self, content: &[u8]) -> Result<String, SnapError> {
        let hash = sha256_hex(content);
        let path = self.blob_path(&hash)?;
        if !path.exists() {
            let parent = path.parent().expect("blob path has parent");
            fs::create_dir_all(parent).map_err(io_err(parent))?;
            let mut nonce = [0u8; 8];
            use rand::RngCore;
            rand::rngs::OsRng.fill_bytes(&mut nonce);
            let tmp = path.with_extension(format!("tmp-{}", hex::encode(nonce)));
            fs::write(&tmp, content).map_err(io_err(&tmp))?;
            fs::rename(&tmp, &path).map_err(io_err(&path))?;
        } else {
            // An existing address is not proof that its bytes are intact.
            // Refuse to reuse a corrupted or substituted CAS object.
            self.get(&hash)?;
        }
        Ok(hash)
    }

    pub fn has(&self, hash: &str) -> bool {
        self.blob_path(hash).is_ok_and(|p| p.exists())
    }

    pub fn get(&self, hash: &str) -> Result<Vec<u8>, SnapError> {
        let path = self.blob_path(hash)?;
        if !path.exists() {
            return Err(SnapError::MissingBlob(hash.into()));
        }
        let bytes = fs::read(&path).map_err(io_err(&path))?;
        let observed = sha256_hex(&bytes);
        if observed != hash.to_ascii_lowercase() {
            return Err(SnapError::HashMismatch {
                expected: hash.into(),
                observed,
            });
        }
        Ok(bytes)
    }
}

fn w13_validate_existing_cas(dir: &Path) -> Result<(), SnapError> {
    let metadata = fs::symlink_metadata(dir)
        .map_err(|_| SnapError::MissingRoot(dir.to_path_buf()))?;
    if metadata.file_type().is_symlink() {
        return Err(SnapError::Symlink(dir.to_path_buf()));
    }
    if !metadata.is_dir() {
        return Err(SnapError::Io {
            path: dir.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "existing CAS path is not a directory",
            ),
        });
    }
    Ok(())
}

/// Capture a store's current state into the CAS; returns its root hash.
pub fn capture(cas: &Cas, spec: &StoreSpec) -> Result<String, SnapError> {
    match spec.kind {
        StoreKind::Fs => capture_fs(cas, &spec.path),
        StoreKind::Sqlite => capture_sqlite(cas, &spec.path),
    }
}

/// Directories that live inside a store root but outside the fabric's
/// byte-boundary (RF-11). `.git` is the operator's out-of-band backstop
/// (dogfooding.md): capturing it would entangle the backstop with the very
/// system it exists to recover from, churn state roots on every git
/// command, and let promotions rewrite git's internals. Excluded from
/// capture, invisible on branches, and never deleted by restore.
pub const EXCLUDED_DIRS: &[&str] = &[".git"];

fn is_excluded_dir(e: &walkdir::DirEntry) -> bool {
    e.file_type().is_dir() && EXCLUDED_DIRS.iter().any(|d| e.file_name() == *d)
}

fn capture_fs(cas: &Cas, root: &Path) -> Result<String, SnapError> {
    if !root.is_dir() {
        return Err(SnapError::MissingRoot(root.to_path_buf()));
    }
    let mut entries: Vec<Value> = Vec::new();
    for entry in WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !is_excluded_dir(e))
    {
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
    let tree: Value =
        serde_json::from_slice(&bytes).map_err(|_| SnapError::MalformedTree(root.into()))?;
    validate_tree_object(&tree, root)?;
    Ok(tree)
}

fn validate_tree_path(root: &str, rel: &str) -> Result<(), SnapError> {
    use std::path::Component;
    let path = Path::new(rel);
    if rel.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(SnapError::UnsafePath {
            root: root.into(),
            path: rel.into(),
        });
    }
    Ok(())
}

fn validate_tree_object(tree: &Value, root: &str) -> Result<(), SnapError> {
    if tree["kind"] != "fs_tree" {
        return Err(SnapError::MalformedTree(root.into()));
    }
    let entries = tree["entries"]
        .as_array()
        .ok_or_else(|| SnapError::MalformedTree(root.into()))?;
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let rel = entry["path"]
            .as_str()
            .ok_or_else(|| SnapError::MalformedTree(root.into()))?;
        validate_tree_path(root, rel)?;
        if !seen.insert(rel) {
            return Err(SnapError::MalformedTree(format!(
                "{root}: duplicate path {rel}"
            )));
        }
        if entry["hash"].as_str().is_none()
            || !matches!(entry["mode"].as_str(), Some("644" | "755"))
            || entry["size"].as_u64().is_none()
        {
            return Err(SnapError::MalformedTree(root.into()));
        }
    }
    Ok(())
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
        validate_tree_path("composed tree", path)?;
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
    plan: RestorePlan,
}

enum RestorePlan {
    /// Whole-file swap via a staging path (sqlite): materialize, rename.
    Swap { staging: PathBuf },
    /// In-place fs apply (RF-10): write only files whose content differs,
    /// delete files absent from the tree, prune empty dirs. Unchanged
    /// files keep their mtimes and inodes — a promotion touches exactly
    /// what it changed, so the vault does not read as wholly rewritten to
    /// humans, sync clients, or backup tools.
    FsInPlace { entries: Vec<FsEntry> },
}

struct FsEntry {
    rel: String,
    hash: String,
    mode: String,
    content: Vec<u8>,
}

fn w14_stage_verified_blob(cas: &Cas, hash: &str) -> Result<Vec<u8>, SnapError> {
    cas.get(hash)
}

fn w14_prepare_verified_fs_entries(cas: &Cas, root: &str) -> Result<Vec<FsEntry>, SnapError> {
    let tree = load_tree_object(cas, root)?;
    let raw = tree["entries"]
        .as_array()
        .ok_or_else(|| SnapError::MalformedTree(root.into()))?;
    let mut entries = Vec::with_capacity(raw.len());
    for e in raw {
        let hash = e["hash"]
            .as_str()
            .ok_or_else(|| SnapError::MalformedTree(root.into()))?
            .to_string();
        entries.push(FsEntry {
            rel: e["path"]
                .as_str()
                .ok_or_else(|| SnapError::MalformedTree(root.into()))?
                .to_string(),
            content: w14_stage_verified_blob(cas, &hash)?,
            hash,
            mode: e["mode"].as_str().unwrap_or("644").to_string(),
        });
    }
    Ok(entries)
}

/// Prepare a store's restore to `root`. All tree parsing and every referenced
/// CAS object's hash verification happen here. Filesystem bytes are retained
/// in the prepared plan, so neither corruption discovered late nor a
/// prepare/commit substitution can cause partial live mutation (RF-20/W-14).
pub fn prepare_restore(
    cas: &Cas,
    spec: &StoreSpec,
    root: &str,
) -> Result<PreparedRestore, SnapError> {
    match spec.kind {
        StoreKind::Fs => {
            let entries = w14_prepare_verified_fs_entries(cas, root)?;
            Ok(PreparedRestore {
                spec: spec.clone(),
                plan: RestorePlan::FsInPlace { entries },
            })
        }
        StoreKind::Sqlite => {
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
            materialize_sqlite(cas, root, &staging)?;
            Ok(PreparedRestore {
                spec: spec.clone(),
                plan: RestorePlan::Swap { staging },
            })
        }
    }
}

/// Apply a prepared restore. Callers must have closed any open
/// connections to sqlite stores first.
pub fn commit_restore(cas: &Cas, prepared: PreparedRestore) -> Result<(), SnapError> {
    let live = &prepared.spec.path;
    match prepared.plan {
        RestorePlan::FsInPlace { entries } => apply_fs_in_place(cas, live, &entries)?,
        RestorePlan::Swap { staging } => {
            fs::rename(&staging, live).map_err(io_err(live))?;
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

/// The in-place fs apply. Per-file writes are tmp+rename (atomic per
/// file); a crash mid-apply leaves a mixed-but-valid tree that a rerun of
/// the same restore completes (the apply is idempotent), and drift
/// detection attributes anything left over. Excluded dirs (`.git`) are
/// never written to and never deleted.
fn apply_fs_in_place(cas: &Cas, root: &Path, entries: &[FsEntry]) -> Result<(), SnapError> {
    apply_fs_in_place_with_hook(cas, root, entries, &mut |_| Ok(()))
}

/// Test seam for deterministic interruption at content-mutation boundaries.
/// Production supplies a no-op hook; unit tests can fail after mutation N and
/// verify that replay converges without encoding sleeps or permission tricks.
fn apply_fs_in_place_with_hook(
    _cas: &Cas,
    root: &Path,
    entries: &[FsEntry],
    before_mutation: &mut dyn FnMut(&Path) -> Result<(), SnapError>,
) -> Result<(), SnapError> {
    fs::create_dir_all(root).map_err(io_err(root))?;
    let desired: std::collections::BTreeMap<&str, &FsEntry> =
        entries.iter().map(|e| (e.rel.as_str(), e)).collect();

    // Pass 1: delete live files the tree does not contain.
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !is_excluded_dir(e))
    {
        let entry = entry.map_err(|e| SnapError::Io {
            path: root.to_path_buf(),
            source: e.into(),
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .expect("walkdir yields children of root")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        if !desired.contains_key(rel.as_str()) {
            before_mutation(entry.path())?;
            fs::remove_file(entry.path()).map_err(io_err(entry.path()))?;
        }
    }

    // Pass 2: write only files whose content differs.
    for e in entries {
        let target = root.join(&e.rel);
        if let Ok(existing) = fs::read(&target) {
            if sha256_hex(&existing) == e.hash {
                continue; // identical — leave mtime and inode alone
            }
        }
        if let Some(p) = target.parent() {
            fs::create_dir_all(p).map_err(io_err(p))?;
        }
        before_mutation(&target)?;
        let tmp = target.with_file_name(format!(
            ".asf-tmp-{}",
            target.file_name().unwrap_or_default().to_string_lossy()
        ));
        fs::write(&tmp, &e.content).map_err(io_err(&tmp))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if e.mode == "755" { 0o755 } else { 0o644 };
            fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)).map_err(io_err(&tmp))?;
        }
        fs::rename(&tmp, &target).map_err(io_err(&target))?;
    }

    // Pass 3: prune dirs the deletions emptied (never the root, never
    // excluded dirs). remove_dir refuses non-empty dirs; that is the test.
    for entry in WalkDir::new(root)
        .contents_first(true)
        .into_iter()
        .filter_entry(|e| !is_excluded_dir(e))
        .flatten()
    {
        if entry.file_type().is_dir() && entry.path() != root {
            let _ = fs::remove_dir(entry.path());
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
        commit_restore(&cas, prepared).unwrap();

        assert_eq!(fs::read_to_string(vault.join("keep.md")).unwrap(), "original");
        assert_eq!(
            fs::read_to_string(vault.join("doomed.md")).unwrap(),
            "will be deleted by agent"
        );
        assert!(!vault.join("new").exists(), "additions rolled back");
        assert_eq!(capture(&cas, &spec).unwrap(), root, "root round-trips");
    }

    /// RF-10: restore is in-place — files whose content already matches
    /// the target tree are not rewritten, so their mtimes (and inodes)
    /// survive. A promotion must touch exactly what it changed.
    #[test]
    fn fs_restore_leaves_unchanged_files_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let vault = tmp.path().join("vault");
        write(&vault.join("untouched.md"), "stable");
        write(&vault.join("mutated.md"), "original");
        let spec = fs_spec(&vault);
        let root = capture(&cas, &spec).unwrap();

        let mtime_before = fs::metadata(vault.join("untouched.md")).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&vault.join("mutated.md"), "changed");

        let prepared = prepare_restore(&cas, &spec, &root).unwrap();
        commit_restore(&cas, prepared).unwrap();

        assert_eq!(fs::read_to_string(vault.join("mutated.md")).unwrap(), "original");
        assert_eq!(
            fs::metadata(vault.join("untouched.md")).unwrap().modified().unwrap(),
            mtime_before,
            "unchanged file was rewritten — RF-10 regression"
        );
    }

    /// RF-11: `.git` is the operator's out-of-band backstop — outside the
    /// store's byte-boundary. Not captured, and never deleted by restore.
    #[test]
    fn git_dir_is_outside_the_store_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let vault = tmp.path().join("vault");
        write(&vault.join("note.md"), "content");
        write(&vault.join(".git/config"), "[core]");
        write(&vault.join(".git/objects/ab/cdef"), "blob");
        let spec = fs_spec(&vault);

        let root = capture(&cas, &spec).unwrap();
        // Git activity must not move the state root.
        write(&vault.join(".git/index"), "changed by git commit");
        assert_eq!(capture(&cas, &spec).unwrap(), root, "git churn read as drift");

        // Restore must neither delete nor rewrite the backstop.
        write(&vault.join("note.md"), "mutated");
        let prepared = prepare_restore(&cas, &spec, &root).unwrap();
        commit_restore(&cas, prepared).unwrap();
        assert_eq!(fs::read_to_string(vault.join("note.md")).unwrap(), "content");
        assert_eq!(fs::read_to_string(vault.join(".git/config")).unwrap(), "[core]");
        assert_eq!(
            fs::read_to_string(vault.join(".git/index")).unwrap(),
            "changed by git commit"
        );
    }

    /// Restores prune directories their deletions emptied (a reverted
    /// move must not leave a husk of empty folders).
    #[test]
    fn fs_restore_prunes_emptied_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let vault = tmp.path().join("vault");
        write(&vault.join("a.md"), "root note");
        let spec = fs_spec(&vault);
        let root = capture(&cas, &spec).unwrap();

        write(&vault.join("deep/nested/b.md"), "agent addition");
        let prepared = prepare_restore(&cas, &spec, &root).unwrap();
        commit_restore(&cas, prepared).unwrap();
        assert!(!vault.join("deep").exists(), "emptied dirs must be pruned");
        assert!(vault.exists(), "the root itself is never pruned");
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
        commit_restore(&cas, prepared).unwrap();

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
    fn w14_prepared_fs_restore_commits_only_staged_verified_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let desired = tmp.path().join("desired");
        let live = tmp.path().join("live");
        write(&desired.join("kept.md"), "verified desired bytes");
        write(&live.join("kept.md"), "old live bytes");
        write(&live.join("obsolete.md"), "delete only after prepare");

        let root = capture_fs(&cas, &desired).unwrap();
        let tree = load_tree_object(&cas, &root).unwrap();
        let blob = tree["entries"][0]["hash"].as_str().unwrap();
        let prepared = prepare_restore(&cas, &fs_spec(&live), &root).unwrap();

        // After prepare, commit must consume the immutable verified bytes in
        // the plan rather than re-reading attacker-controlled CAS storage.
        fs::write(cas.blob_path(blob).unwrap(), b"substituted after prepare").unwrap();
        commit_restore(&cas, prepared).unwrap();

        assert_eq!(
            fs::read_to_string(live.join("kept.md")).unwrap(),
            "verified desired bytes"
        );
        assert!(!live.join("obsolete.md").exists());
    }

    #[test]
    fn interrupted_in_place_apply_is_idempotently_recoverable() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let live = tmp.path().join("live");
        let desired = tmp.path().join("desired");
        fs::create_dir_all(&live).unwrap();
        fs::create_dir_all(&desired).unwrap();
        write(&live.join("a.md"), "old a");
        write(&live.join("b.md"), "old b");
        write(&live.join("obsolete.md"), "remove me");
        write(&desired.join("a.md"), "new a");
        write(&desired.join("b.md"), "new b");
        let desired_root = capture_fs(&cas, &desired).unwrap();
        let prepared = prepare_restore(&cas, &fs_spec(&live), &desired_root).unwrap();
        let RestorePlan::FsInPlace { entries } = prepared.plan else {
            panic!("fs restore must use in-place plan");
        };

        let mut mutations = 0;
        let result = apply_fs_in_place_with_hook(
            &cas,
            &live,
            &entries,
            &mut |_| {
                mutations += 1;
                if mutations == 2 {
                    return Err(SnapError::Io {
                        path: live.clone(),
                        source: std::io::Error::other("injected interruption"),
                    });
                }
                Ok(())
            },
        );
        assert!(result.is_err(), "the deterministic failpoint must fire");
        assert_ne!(
            capture_fs(&cas, &live).unwrap(),
            desired_root,
            "the interruption should leave a mixed, not falsely complete, tree"
        );

        apply_fs_in_place(&cas, &live, &entries).unwrap();
        assert_eq!(
            capture_fs(&cas, &live).unwrap(),
            desired_root,
            "replaying the same restore must converge exactly"
        );
    }

    /// Child-process half of `hard_exit_mid_apply_reopens_and_converges`.
    /// It is also discovered by the normal test harness, where it is a no-op;
    /// the parent re-invokes this exact test with an explicit crash scenario.
    #[test]
    fn crash_during_fs_apply_helper() {
        let Ok(cas_dir) = std::env::var("ASF_CRASH_CAS") else {
            return;
        };
        let live = PathBuf::from(std::env::var("ASF_CRASH_LIVE").unwrap());
        let desired_root = std::env::var("ASF_CRASH_ROOT").unwrap();
        let marker = PathBuf::from(std::env::var("ASF_CRASH_MARKER").unwrap());
        let cas = Cas::open(cas_dir).unwrap();
        let prepared = prepare_restore(&cas, &fs_spec(&live), &desired_root).unwrap();
        let RestorePlan::FsInPlace { entries } = prepared.plan else {
            panic!("fs restore must use in-place plan");
        };
        let mut mutations = 0;
        let result = apply_fs_in_place_with_hook(&cas, &live, &entries, &mut |_| {
            mutations += 1;
            if mutations == 2 {
                fs::write(&marker, b"crash boundary reached").unwrap();
                // Hard process exit: no Rust unwinding or destructor cleanup.
                std::process::exit(86);
            }
            Ok(())
        });
        panic!("crash helper returned instead of exiting: {result:?}");
    }

    #[test]
    fn hard_exit_mid_apply_reopens_and_converges() {
        let tmp = tempfile::tempdir().unwrap();
        let cas_dir = tmp.path().join("cas");
        let cas = Cas::open(&cas_dir).unwrap();
        let live = tmp.path().join("live");
        let desired = tmp.path().join("desired");
        let marker = tmp.path().join("crash-reached");
        fs::create_dir_all(&live).unwrap();
        fs::create_dir_all(&desired).unwrap();
        write(&live.join("a.md"), "old a");
        write(&live.join("b.md"), "old b");
        write(&live.join("obsolete.md"), "remove me");
        write(&desired.join("a.md"), "new a");
        write(&desired.join("b.md"), "new b");
        let desired_root = capture_fs(&cas, &desired).unwrap();

        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "snapshot::tests::crash_during_fs_apply_helper",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("ASF_CRASH_CAS", &cas_dir)
            .env("ASF_CRASH_LIVE", &live)
            .env("ASF_CRASH_ROOT", &desired_root)
            .env("ASF_CRASH_MARKER", &marker)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(86),
            "helper did not exit at failpoint: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(marker.exists());
        assert_ne!(
            capture_fs(&cas, &live).unwrap(),
            desired_root,
            "hard exit should expose a detectable mixed tree"
        );

        // Simulate restart: reopen the CAS, rebuild the prepared plan, replay,
        // and require exact convergence to the intended content root.
        drop(cas);
        let reopened = Cas::open(&cas_dir).unwrap();
        let prepared = prepare_restore(&reopened, &fs_spec(&live), &desired_root).unwrap();
        commit_restore(&reopened, prepared).unwrap();
        assert_eq!(capture_fs(&reopened, &live).unwrap(), desired_root);
    }

    #[test]
    fn cas_get_rehashes_content_before_returning_it() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let root = cas.put(b"authentic").unwrap();
        let path = cas.blob_path(&root).unwrap();
        fs::write(&path, b"substituted").unwrap();

        assert!(matches!(
            cas.get(&root),
            Err(SnapError::HashMismatch { expected, .. }) if expected == root
        ));
        assert!(matches!(
            cas.put(b"authentic"),
            Err(SnapError::HashMismatch { .. })
        ));
    }

    #[test]
    fn materialization_rejects_paths_outside_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let blob = cas.put(b"escape").unwrap();
        let tree = json!({
            "kind": "fs_tree",
            "entries": [{
                "path": "../outside.md",
                "mode": "644",
                "size": 6,
                "hash": blob,
            }],
        });
        let root = cas
            .put(&serde_json_canonicalizer::to_vec(&tree).unwrap())
            .unwrap();
        let dest = tmp.path().join("dest");

        assert!(matches!(
            materialize_fs(&cas, &root, &dest),
            Err(SnapError::UnsafePath { .. })
        ));
        assert!(!tmp.path().join("outside.md").exists());
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
