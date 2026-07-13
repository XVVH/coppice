//! The kernel loop: step-boundary manifests, drift detection, coherent
//! revert. Spec §3 (DelegationManifest), §6 (events), A12 (drift
//! attribution); brief §4.
//!
//! Manifests declare their enforcement mode, never a capability id (A21):
//! `authority: {"mode":"brokered"}` for broker-fronted runs, omitted for
//! observed runs. The capability binds backward via `bound_manifest` (M2)
//! and forward via the broker's signed `grant` event (§6, M7); the gate
//! enforces the brokered fail-closed rule in `broker.rs`.

use crate::canon;
use crate::keys::{Kek, Keystore, Role};
use crate::payload::{self, PayloadRef};
use crate::snapshot::{self, Cas, StoreSpec};
use crate::trace;
use crate::now_rfc3339;
use ed25519_dalek::SigningKey;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Map, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error(transparent)]
    Trace(#[from] trace::TraceError),
    #[error(transparent)]
    Payload(#[from] payload::PayloadError),
    #[error(transparent)]
    Snapshot(#[from] snapshot::SnapError),
    #[error(transparent)]
    Keys(#[from] crate::keys::KeyError),
    #[error(transparent)]
    Canon(#[from] canon::CanonError),
    #[error("sqlite: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("fabric is not initialized at {home}: expected database {database}")]
    NotInitialized { home: PathBuf, database: PathBuf },
    #[error("refusing to initialize over existing or partial fabric home {home}")]
    AlreadyInitialized { home: PathBuf },
    #[error(
        "sqlite runtime {observed} is below conservative safe floor {minimum}; its version does not prove inclusion of the WAL-reset corruption fix (RF-21)"
    )]
    UnsafeSqliteVersion { observed: i32, minimum: i32 },
    #[error("no active manifest — call step_boundary first")]
    NoActiveManifest,
    #[error("unknown store {0}")]
    UnknownStore(String),
    #[error("manifest {0} malformed: {1}")]
    MalformedManifest(String, String),
    #[error("recoverable multi-root state change failed: {0}")]
    StateChange(String),
}

const STATE_CHANGE_JOURNAL: &str = "state-change.pending.json";

#[derive(Clone)]
pub(crate) struct StateRootTransition {
    pub store: String,
    pub before: String,
    pub after: String,
}

pub(crate) struct StateChangePlan {
    pub manifest: String,
    pub event_kind: String,
    pub event_body: Value,
    pub transitions: Vec<StateRootTransition>,
    pub set_current_manifest: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StateChangePoint {
    AfterJournal,
    BeforeStoreSync(String),
    AfterStore(String),
    BeforeDatabaseCommit,
    AfterDatabaseCommit,
}

fn state_io(path: &Path, source: std::io::Error) -> KernelError {
    snapshot::SnapError::Io {
        path: path.to_path_buf(),
        source,
    }
    .into()
}

fn sync_state_directory(path: &Path) -> Result<(), KernelError> {
    std::fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| state_io(path, source))
}

fn state_change_journal_value(
    sk: &SigningKey,
    event: &trace::Appended,
    event_kind: &str,
    manifest: &str,
    transitions: &[StateRootTransition],
) -> Result<Value, KernelError> {
    let before = transitions
        .iter()
        .map(|root| json!({"store": root.store, "root": root.before}))
        .collect::<Vec<_>>();
    let after = transitions
        .iter()
        .map(|root| json!({"store": root.store, "root": root.after}))
        .collect::<Vec<_>>();
    let mut body = Map::new();
    body.insert("kind".into(), json!("state_change_recovery"));
    body.insert("version".into(), json!(1));
    body.insert("event".into(), json!(event.id));
    body.insert("event_kind".into(), json!(event_kind));
    body.insert("manifest".into(), json!(manifest));
    body.insert("before".into(), Value::Array(before));
    body.insert("after".into(), Value::Array(after));
    Ok(Value::Object(canon::seal("txn", body, sk)?))
}

fn publish_state_change_journal(home: &Path, value: &Value) -> Result<(), KernelError> {
    let path = home.join(STATE_CHANGE_JOURNAL);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&path).map_err(|source| state_io(&path, source))?;
    let bytes = serde_json::to_vec(value).expect("signed recovery journal serializes");
    file.write_all(&bytes)
        .map_err(|source| state_io(&path, source))?;
    file.sync_all().map_err(|source| state_io(&path, source))?;
    drop(file);
    sync_state_directory(home)
}

fn remove_state_change_journal(home: &Path) -> Result<(), KernelError> {
    let path = home.join(STATE_CHANGE_JOURNAL);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|source| state_io(&path, source))?;
        sync_state_directory(home)?;
    }
    Ok(())
}

fn recovery_roots(value: &Value, field: &'static str) -> Result<Vec<StateRootTransition>, KernelError> {
    let before = value["before"].as_array().ok_or_else(|| {
        KernelError::StateChange("recovery journal has no before-root tuple".into())
    })?;
    let after = value["after"].as_array().ok_or_else(|| {
        KernelError::StateChange("recovery journal has no after-root tuple".into())
    })?;
    if before.len() != after.len() {
        return Err(KernelError::StateChange(
            "recovery root tuples have different lengths".into(),
        ));
    }
    let mut before_map = std::collections::BTreeMap::new();
    for root in before {
        let (Some(store), Some(hash)) = (root["store"].as_str(), root["root"].as_str()) else {
            return Err(KernelError::StateChange(format!(
                "recovery journal has malformed {field} roots"
            )));
        };
        if before_map.insert(store.to_string(), hash.to_string()).is_some() {
            return Err(KernelError::StateChange(format!(
                "recovery journal repeats store {store}"
            )));
        }
    }
    let mut transitions = Vec::new();
    for root in after {
        let (Some(store), Some(hash)) = (root["store"].as_str(), root["root"].as_str()) else {
            return Err(KernelError::StateChange(format!(
                "recovery journal has malformed {field} roots"
            )));
        };
        let old = before_map.remove(store).ok_or_else(|| {
            KernelError::StateChange(format!("recovery journal adds store {store}"))
        })?;
        transitions.push(StateRootTransition {
            store: store.to_string(),
            before: old,
            after: hash.to_string(),
        });
    }
    if !before_map.is_empty() {
        return Err(KernelError::StateChange(
            "recovery journal drops a store from its after tuple".into(),
        ));
    }
    transitions.sort_by(|left, right| left.store.cmp(&right.store));
    Ok(transitions)
}

fn state_change_event_roots(
    event: &trace::VerifiedEvent,
) -> Result<std::collections::BTreeMap<String, String>, KernelError> {
    let roots = match event.kind.as_str() {
        "revert" => event.raw["body"]["roots_restored"].as_array(),
        "promotion" => event.raw["body"]["stores"].as_array(),
        _ => None,
    }
    .ok_or_else(|| {
        KernelError::StateChange(format!(
            "signed {} event {} has no root tuple",
            event.kind, event.id
        ))
    })?;
    let mut mapped = std::collections::BTreeMap::new();
    for root in roots {
        let store = root["store"].as_str().ok_or_else(|| {
            KernelError::StateChange(format!("signed event {} has a root without store", event.id))
        })?;
        let hash = if event.kind == "promotion" {
            root["merged"].as_str()
        } else {
            root["root"].as_str()
        }
        .ok_or_else(|| {
            KernelError::StateChange(format!("signed event {} has a root without hash", event.id))
        })?;
        if mapped.insert(store.to_string(), hash.to_string()).is_some() {
            return Err(KernelError::StateChange(format!(
                "signed event {} repeats store {store}",
                event.id
            )));
        }
    }
    Ok(mapped)
}

/// First mainline SQLite patch release containing the WAL-reset fix.
///
/// Keep this runtime guard even with `rusqlite/bundled`: it turns a future
/// feature/dependency regression into a loud refusal before a fabric home is
/// created or opened. Upstream also published older fixed backports, but a
/// version-only check cannot attest their patch provenance; the conservative
/// floor deliberately rejects them. SQLite encodes 3.51.3 as 3_051_003.
const MIN_SAFE_SQLITE_VERSION: i32 = 3_051_003;

fn require_safe_sqlite_version(observed: i32) -> Result<(), KernelError> {
    if observed < MIN_SAFE_SQLITE_VERSION {
        return Err(KernelError::UnsafeSqliteVersion {
            observed,
            minimum: MIN_SAFE_SQLITE_VERSION,
        });
    }
    Ok(())
}

fn w13_prepare_new_home(path: &Path) -> Result<(), KernelError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path.file_name().ok_or_else(|| snapshot::SnapError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "fabric home has no final path component",
        ),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| snapshot::SnapError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut existing = None;
    for entry in std::fs::read_dir(parent).map_err(|source| snapshot::SnapError::Io {
        path: parent.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| snapshot::SnapError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        if entry.file_name() == name {
            existing = Some(entry);
            break;
        }
    }
    if let Some(entry) = existing {
        if !entry.file_type().map_err(|source| snapshot::SnapError::Io {
            path: entry.path(),
            source,
        })?.is_dir()
            || std::fs::read_dir(entry.path())
                .map_err(|source| snapshot::SnapError::Io {
                    path: path.to_path_buf(),
                    source,
                })?
                .next()
                .is_some()
        {
            return Err(KernelError::AlreadyInitialized {
                home: path.to_path_buf(),
            });
        }
    } else {
        std::fs::create_dir(path).map_err(|source| snapshot::SnapError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|source| snapshot::SnapError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |source| snapshot::SnapError::Io {
                path: path.to_path_buf(),
                source,
            },
        )?;
    }
    Ok(())
}

fn w13_validate_existing_fabric_home(
    home: &Path,
    database: &Path,
) -> Result<(), KernelError> {
    let home_metadata = std::fs::symlink_metadata(home).map_err(|_| {
        KernelError::NotInitialized {
            home: home.to_path_buf(),
            database: database.to_path_buf(),
        }
    })?;
    if !home_metadata.is_dir() {
        return Err(KernelError::NotInitialized {
            home: home.to_path_buf(),
            database: database.to_path_buf(),
        });
    }

    let database_metadata = std::fs::symlink_metadata(database).map_err(|_| {
        KernelError::NotInitialized {
            home: home.to_path_buf(),
            database: database.to_path_buf(),
        }
    })?;
    if !database_metadata.is_file() {
        return Err(KernelError::NotInitialized {
            home: home.to_path_buf(),
            database: database.to_path_buf(),
        });
    }
    Ok(())
}

/// The Stage 1 fabric: one sqlite substrate (events, objects, payloads),
/// a CAS for snapshots, a keystore, and the registered Tier-1 stores.
pub struct Fabric {
    pub conn: Connection,
    pub cas: Cas,
    pub kek: Kek,
    stores: Vec<StoreSpec>,
    keystore: Keystore,
    fabric_sk: SigningKey,
    user_sk: SigningKey,
    substrate_span: String,
    home: std::path::PathBuf,
    /// When set, the agent's writes land here instead of the live stores
    /// (store name → branch path); tool_call events capture branch roots.
    branch: Option<std::collections::BTreeMap<String, std::path::PathBuf>>,
}

/// Holds the per-fabric-home gate lock (M8); dropping it releases the
/// flock. See [`Fabric::gate_lock`].
pub struct GateGuard {
    _file: std::fs::File,
}

#[derive(Debug, Clone)]
pub struct DriftReport {
    pub store: String,
    pub expected_root: String,
    pub observed_root: String,
    pub between: (i64, i64),
    pub attribution: String,
    pub event_id: String,
}

#[derive(Debug, Clone)]
pub struct StepOutcome {
    pub manifest: String,
    pub span: String,
    pub drift: Vec<DriftReport>,
    pub remanifest: bool,
}

/// M7 (A21): the enforcement mode a manifest declares. `Observed` omits
/// `authority` — the conservative default; `Brokered` seals
/// `authority: {"mode":"brokered"}`, which the gate enforces fail-closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityMode {
    Observed,
    Brokered,
}

impl Fabric {
    /// Explicitly initialize a new fabric home at `dir`, coordinating
    /// `stores`. Refuses any existing or partially initialized home.
    /// Layout: `dir/fabric.db`, `dir/cas/`, `dir/keys/`.
    pub fn initialize(dir: impl AsRef<Path>, stores: Vec<StoreSpec>) -> Result<Self, KernelError> {
        Self::initialize_with_sqlite_version(dir, stores, rusqlite::version_number())
    }

    /// Test seam for the fail-before-effects SQLite version gate. Production
    /// always supplies `rusqlite::version_number()` through [`Self::initialize`].
    fn initialize_with_sqlite_version(
        dir: impl AsRef<Path>,
        stores: Vec<StoreSpec>,
        sqlite_version: i32,
    ) -> Result<Self, KernelError> {
        require_safe_sqlite_version(sqlite_version)?;
        let dir = dir.as_ref();
        w13_prepare_new_home(dir)?;
        let keystore = Keystore::initialize(dir.join("keys"))?;
        let fabric_sk = keystore.signing_key(Role::Fabric)?;
        let user_sk = keystore.signing_key(Role::UserRoot)?;
        let kek = keystore.kek()?;
        let conn = Connection::open(dir.join("fabric.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        trace::init(&conn)?;
        payload::init(&conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS expected_roots (
                store        TEXT PRIMARY KEY,
                root         TEXT NOT NULL,
                since_offset INTEGER NOT NULL
            );",
        )?;
        let cas = Cas::open(dir.join("cas"))?;
        let substrate_span = trace::new_span();
        trace::meta_set(&conn, "substrate_span", &substrate_span)?;
        // Persist store specs so operator tools can reopen this home
        // without re-supplying the topology (asf revert/ledger/stats).
        let stores_json = serde_json::to_string(
            &stores
                .iter()
                .map(|s| {
                    json!({ "store": s.store, "tier": s.tier,
                            "kind": s.kind.as_str(), "path": s.path })
                })
                .collect::<Vec<_>>(),
        )
        .expect("stores serialize");
        trace::meta_set(&conn, "stores", &stores_json)?;

        Ok(Self {
            conn,
            cas,
            kek,
            stores,
            keystore,
            fabric_sk,
            user_sk,
            substrate_span,
            home: dir.to_path_buf(),
            branch: None,
        })
    }

    /// Reopen an existing fabric home using the store topology persisted
    /// at first open. Errors if the home was never initialized.
    pub fn open_existing(dir: impl AsRef<Path>) -> Result<Self, KernelError> {
        Self::open_existing_impl(dir.as_ref(), None)
    }

    /// Reopen an existing home with an explicitly supplied runtime store
    /// view. This never rewrites the persisted topology and exists for
    /// callers constructing a deliberately narrower manifest view.
    pub fn open_existing_with_stores(
        dir: impl AsRef<Path>,
        stores: Vec<StoreSpec>,
    ) -> Result<Self, KernelError> {
        Self::open_existing_impl(dir.as_ref(), Some(stores))
    }

    fn open_existing_impl(
        dir: &Path,
        stores_override: Option<Vec<StoreSpec>>,
    ) -> Result<Self, KernelError> {
        require_safe_sqlite_version(rusqlite::version_number())?;
        let database = dir.join("fabric.db");
        w13_validate_existing_fabric_home(dir, &database)?;
        // Validate all home-lifetime identity material before SQLite is
        // opened. Missing or malformed authority therefore cannot create
        // WAL sidecars, replacement keys, or any other protected effect.
        let keystore = Keystore::open_existing(dir.join("keys"))?;
        let fabric_sk = keystore.signing_key(Role::Fabric)?;
        let user_sk = keystore.signing_key(Role::UserRoot)?;
        let kek = keystore.kek()?;
        let conn = Connection::open(database)?;
        let stores = match stores_override {
            Some(stores) => stores,
            None => {
                let raw = trace::meta_get(&conn, "stores")?.ok_or_else(|| {
                    KernelError::UnknownStore("no stores recorded in this home".into())
                })?;
                let specs: Vec<Value> = serde_json::from_str(&raw)
                    .map_err(|error| KernelError::MalformedManifest("stores".into(), error.to_string()))?;
                specs
                    .iter()
                    .map(|s| StoreSpec {
                        store: s["store"].as_str().unwrap_or_default().to_string(),
                        tier: s["tier"].as_u64().unwrap_or(1) as u8,
                        kind: if s["kind"] == "sqlite" {
                            crate::snapshot::StoreKind::Sqlite
                        } else {
                            crate::snapshot::StoreKind::Fs
                        },
                        path: std::path::PathBuf::from(s["path"].as_str().unwrap_or_default()),
                    })
                    .collect()
            }
        };
        let substrate_span = trace::meta_get(&conn, "substrate_span")?.ok_or_else(|| {
            KernelError::MalformedManifest("fabric-home".into(), "missing substrate span".into())
        })?;
        let cas = Cas::open_existing(dir.join("cas"))?;
        let mut fabric = Self {
            conn,
            cas,
            kek,
            stores,
            keystore,
            fabric_sk,
            user_sk,
            substrate_span,
            home: dir.to_path_buf(),
            branch: None,
        };
        let journal = fabric.home.join(STATE_CHANGE_JOURNAL);
        match std::fs::symlink_metadata(&journal) {
            Ok(_) => {
                let _gate = fabric.gate_lock()?;
                fabric.recover_pending_state_change()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(state_io(&journal, error)),
        }
        Ok(fabric)
    }

    /// Fork a manifest's state into a working branch (brief §3 principle 2:
    /// working state lives on branches; promotion is the only mutation).
    /// Materializes every root under `<home>/branches/<manifest>/` and
    /// switches tool-call capture to the branch. Returns store → path.
    pub fn create_branch(
        &mut self,
        manifest_id: &str,
    ) -> Result<std::collections::BTreeMap<String, std::path::PathBuf>, KernelError> {
        let man = self.load_manifest(manifest_id)?;
        let short = manifest_id.strip_prefix("man:").unwrap_or(manifest_id);
        let base = self.home.join("branches").join(&short[..12.min(short.len())]);
        let mut map = std::collections::BTreeMap::new();
        for r in man["state"]["roots"].as_array().into_iter().flatten() {
            let store = r["store"].as_str().unwrap_or_default().to_string();
            let root = r["root"].as_str().unwrap_or_default();
            let spec = self.store(&store)?.clone();
            let dest = base.join(store.replace([':', '/'], "_"));
            if dest.exists() {
                if dest.is_dir() {
                    std::fs::remove_dir_all(&dest).ok();
                } else {
                    std::fs::remove_file(&dest).ok();
                }
            }
            match spec.kind {
                crate::snapshot::StoreKind::Fs => {
                    snapshot::materialize_fs(&self.cas, root, &dest)?
                }
                crate::snapshot::StoreKind::Sqlite => {
                    snapshot::materialize_sqlite(&self.cas, root, &dest)?
                }
            }
            map.insert(store, dest);
        }
        self.branch = Some(map.clone());
        Ok(map)
    }

    pub fn branch_paths(&self) -> Option<&std::collections::BTreeMap<String, std::path::PathBuf>> {
        self.branch.as_ref()
    }

    pub(crate) fn fabric_sk(&self) -> &SigningKey {
        &self.fabric_sk
    }

    pub(crate) fn substrate_span(&self) -> &str {
        &self.substrate_span
    }

    pub fn keystore(&self) -> &Keystore {
        &self.keystore
    }

    pub fn stores(&self) -> &[StoreSpec] {
        &self.stores
    }

    pub fn store(&self, name: &str) -> Result<&StoreSpec, KernelError> {
        self.stores
            .iter()
            .find(|s| s.store == name)
            .ok_or_else(|| KernelError::UnknownStore(name.into()))
    }

    pub fn fabric_vk(&self) -> ed25519_dalek::VerifyingKey {
        self.fabric_sk.verifying_key()
    }

    /// RF-19/W-14: manifests are authority-bearing inputs to branch,
    /// promotion, parent-lineage, and revert operations. All such consumers
    /// share this exact signed-id/prefix/stored-kind boundary.
    pub(crate) fn load_manifest(&self, id: &str) -> Result<Value, KernelError> {
        Ok(trace::load_verified_object(
            &self.conn,
            id,
            "man",
            "manifest",
            &self.fabric_vk(),
        )?)
    }

    fn substrate_event(
        &mut self,
        manifest: Option<&str>,
        kind: &str,
        body: Value,
    ) -> Result<trace::Appended, KernelError> {
        let span = self.substrate_span.clone();
        Ok(trace::append(
            &mut self.conn,
            &self.fabric_sk,
            &span,
            manifest,
            kind,
            body,
            &now_rfc3339(),
        )?)
    }

    /// Commit one promotion/revert as a recoverable transaction spanning the
    /// signed ledger and every live Tier-1 store. The callback may add cache
    /// updates or companion signed events to the same SQLite transaction;
    /// every database failure occurs before the first live-store mutation.
    pub(crate) fn commit_state_change<F>(
        &mut self,
        change: StateChangePlan,
        finalize: F,
    ) -> Result<trace::Appended, KernelError>
    where
        F: FnOnce(&Transaction<'_>, &trace::Appended) -> Result<(), KernelError>,
    {
        self.commit_state_change_with_hook(
            change,
            finalize,
            &mut |_| Ok(()),
        )
    }

    fn commit_state_change_with_hook<F>(
        &mut self,
        change: StateChangePlan,
        finalize: F,
        hook: &mut dyn FnMut(StateChangePoint) -> Result<(), KernelError>,
    ) -> Result<trace::Appended, KernelError>
    where
        F: FnOnce(&Transaction<'_>, &trace::Appended) -> Result<(), KernelError>,
    {
        let StateChangePlan {
            manifest,
            event_kind,
            event_body,
            mut transitions,
            set_current_manifest,
        } = change;
        transitions.sort_by(|left, right| left.store.cmp(&right.store));
        if transitions
            .windows(2)
            .any(|pair| pair[0].store == pair[1].store)
        {
            return Err(KernelError::StateChange(
                "state-change tuple repeats a store".into(),
            ));
        }
        let mut target = Vec::new();
        let mut rollback = Vec::new();
        for root in &transitions {
            let spec = self.store(&root.store)?.clone();
            let observed = snapshot::capture(&self.cas, &spec)?;
            if observed != root.before {
                return Err(KernelError::StateChange(format!(
                    "{} changed while the gate was preparing: expected {}, observed {}",
                    root.store, root.before, observed
                )));
            }
            if root.before != root.after {
                target.push((
                    spec.clone(),
                    snapshot::prepare_restore(&self.cas, &spec, &root.after)?,
                ));
                rollback.push((
                    spec.clone(),
                    snapshot::prepare_restore(&self.cas, &spec, &root.before)?,
                ));
            }
        }

        let sk = self.fabric_sk.clone();
        let span = self.substrate_span.clone();
        let home = self.home.clone();
        let cas = &self.cas;
        let now = now_rfc3339();
        let tx = self.conn.transaction()?;
        let event = trace::append_in_tx(
            &tx,
            &sk,
            &span,
            Some(&manifest),
            &event_kind,
            event_body,
            &now,
        )?;
        for root in &transitions {
            tx.execute(
                "INSERT INTO expected_roots (store, root, since_offset) VALUES (?1,?2,?3)
                 ON CONFLICT(store) DO UPDATE SET root = excluded.root,
                                                  since_offset = excluded.since_offset",
                params![root.store, root.after, event.offset],
            )?;
        }
        if set_current_manifest {
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('current_manifest', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [manifest.as_str()],
            )?;
        }
        finalize(&tx, &event)?;
        let journal = state_change_journal_value(
            &sk,
            &event,
            &event_kind,
            &manifest,
            &transitions,
        )?;
        publish_state_change_journal(&home, &journal)?;

        let applied = (|| -> Result<(), KernelError> {
            hook(StateChangePoint::AfterJournal)?;
            for (spec, prepared) in target {
                snapshot::commit_restore(cas, prepared)?;
                hook(StateChangePoint::BeforeStoreSync(spec.store.clone()))?;
                snapshot::sync_store(&spec)?;
                hook(StateChangePoint::AfterStore(spec.store.clone()))?;
            }
            hook(StateChangePoint::BeforeDatabaseCommit)?;
            Ok(())
        })();
        if let Err(apply_error) = applied {
            let mut rollback_error = None;
            for (spec, prepared) in rollback {
                if let Err(error) = snapshot::commit_restore(cas, prepared)
                    .and_then(|_| snapshot::sync_store(&spec))
                {
                    rollback_error = Some(error);
                    break;
                }
            }
            let database_rollback = tx.rollback();
            if let Some(error) = rollback_error {
                return Err(KernelError::StateChange(format!(
                    "{apply_error}; rollback also failed: {error}; recovery journal retained"
                )));
            }
            if let Err(error) = database_rollback {
                return Err(KernelError::StateChange(format!(
                    "{apply_error}; ledger rollback also failed: {error}; recovery journal retained"
                )));
            }
            remove_state_change_journal(&home)?;
            return Err(apply_error);
        }

        if let Err(commit_error) = tx.commit() {
            let recovery = self.recover_pending_state_change();
            return match recovery {
                Ok(()) => Err(commit_error.into()),
                Err(recovery_error) => Err(KernelError::StateChange(format!(
                    "ledger commit failed: {commit_error}; immediate recovery also failed: {recovery_error}; recovery journal retained"
                ))),
            };
        }
        hook(StateChangePoint::AfterDatabaseCommit)?;
        remove_state_change_journal(&home)?;
        Ok(event)
    }

    /// Resolve a journal left by an interrupted state change. The primary
    /// event's verified presence is the commit bit: absent rolls every store
    /// back; present and exactly linked rolls every store forward.
    fn recover_pending_state_change(&mut self) -> Result<(), KernelError> {
        let path = self.home.join(STATE_CHANGE_JOURNAL);
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| state_io(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(KernelError::StateChange(format!(
                "unsafe recovery-journal path {}",
                path.display()
            )));
        }
        if !metadata.is_file() {
            return Err(KernelError::StateChange(format!(
                "unsafe recovery-journal path {}",
                path.display()
            )));
        }
        let raw = std::fs::read_to_string(&path).map_err(|source| state_io(&path, source))?;
        let journal = canon::parse_fabric_json(&raw)?;
        canon::verify(&journal, &self.fabric_vk())?;
        if journal["id"]
            .as_str()
            .is_none_or(|id| !id.starts_with("txn:"))
            || journal["kind"] != "state_change_recovery"
            || journal["version"] != 1
        {
            return Err(KernelError::StateChange(
                "recovery journal has the wrong signed type or version".into(),
            ));
        }
        let event_id = journal["event"].as_str().ok_or_else(|| {
            KernelError::StateChange("recovery journal has no event id".into())
        })?;
        let event_kind = journal["event_kind"].as_str().ok_or_else(|| {
            KernelError::StateChange("recovery journal has no event kind".into())
        })?;
        let manifest = journal["manifest"].as_str().ok_or_else(|| {
            KernelError::StateChange("recovery journal has no manifest".into())
        })?;
        if !matches!(event_kind, "promotion" | "revert") {
            return Err(KernelError::StateChange(format!(
                "recovery journal names unsupported event kind {event_kind}"
            )));
        }
        let transitions = recovery_roots(&journal, "state-change")?;
        let events = trace::verified_events(&self.conn, &self.fabric_vk())?;
        let committed = events.iter().find(|event| event.id == event_id);
        let desired = match committed {
            Some(event) => {
                if event.kind != event_kind || event.manifest.as_deref() != Some(manifest) {
                    return Err(KernelError::StateChange(
                        "recovery journal/event linkage does not match".into(),
                    ));
                }
                let event_roots = state_change_event_roots(event)?;
                let expected = transitions
                    .iter()
                    .map(|root| (root.store.clone(), root.after.clone()))
                    .collect::<std::collections::BTreeMap<_, _>>();
                if event_roots != expected {
                    return Err(KernelError::StateChange(
                        "signed event roots do not match the recovery journal".into(),
                    ));
                }
                transitions
                    .iter()
                    .map(|root| (root.store.clone(), root.after.clone()))
                    .collect::<Vec<_>>()
            }
            None => transitions
                .iter()
                .map(|root| (root.store.clone(), root.before.clone()))
                .collect::<Vec<_>>(),
        };
        let mut prepared = Vec::new();
        for (store, root) in desired {
            let spec = self.store(&store)?.clone();
            prepared.push((
                spec.clone(),
                snapshot::prepare_restore(&self.cas, &spec, &root)?,
            ));
        }
        for (spec, prepared) in prepared {
            snapshot::commit_restore(&self.cas, prepared)?;
            snapshot::sync_store(&spec)?;
        }
        remove_state_change_journal(&self.home)
    }

    // ---- registration (SI-11 event kinds) -----------------------------

    /// Register a principal. Humans are sealed by the user root key,
    /// agents/services by the fabric key (§8.1; SI-3 for non-human kinds).
    pub fn register_principal(
        &mut self,
        kind: &str,
        label: &str,
        pubkey_hex: &str,
        root: Option<&str>,
    ) -> Result<String, KernelError> {
        let mut body = Map::new();
        body.insert("kind".into(), json!(kind));
        body.insert("root".into(), root.map(Value::from).unwrap_or(Value::Null));
        body.insert("pubkey".into(), json!(format!("ed25519:{pubkey_hex}")));
        body.insert("meta".into(), json!({ "label": label }));
        let sk = if kind == "human" {
            &self.user_sk
        } else {
            &self.fabric_sk
        };
        let sealed = canon::seal("prin", body, sk)?;
        let id = trace::put_object(&self.conn, "principal", &sealed, &now_rfc3339())?;
        self.substrate_event(
            None,
            "register",
            json!({ "object": id, "object_kind": "principal" }),
        )?;
        Ok(id)
    }

    /// Register a channel — the front door (§2.1). `address` binds the
    /// sender identity and is stored as a payload (it may be PII).
    pub fn register_channel(
        &mut self,
        principal: &str,
        kind: &str,
        address: &[u8],
        auth_strength: &str,
    ) -> Result<String, KernelError> {
        let addr_ref = payload::put(&self.conn, &self.kek, address, "text/plain", &now_rfc3339())?;
        let mut body = Map::new();
        body.insert("principal".into(), json!(principal));
        body.insert("kind".into(), json!(kind));
        body.insert("address".into(), serde_json::to_value(&addr_ref).unwrap());
        body.insert("auth_strength".into(), json!(auth_strength));
        body.insert("registered_at".into(), json!(now_rfc3339()));
        let sealed = canon::seal("chan", body, &self.fabric_sk)?;
        let id = trace::put_object(&self.conn, "channel", &sealed, &now_rfc3339())?;
        self.substrate_event(
            None,
            "register",
            json!({ "object": id, "object_kind": "channel" }),
        )?;
        Ok(id)
    }

    /// Capture a signed IntentArtifact before untrusted data enters the run
    /// (§3.1). `captured_before` is the substrate head at seal time, and the
    /// `intent` event countersigns it into the substrate (SI-10/SI-11).
    pub fn capture_intent(
        &mut self,
        principal: &str,
        channel: &str,
        auth_strength: &str,
        text: &str,
        structured: Value,
        amends: Option<&str>,
    ) -> Result<String, KernelError> {
        let text_ref = payload::put(
            &self.conn,
            &self.kek,
            text.as_bytes(),
            "text/plain",
            &now_rfc3339(),
        )?;
        let captured_before = trace::head_offset(&self.conn)? + 1;
        let mut body = Map::new();
        body.insert("principal".into(), json!(principal));
        body.insert("created_at".into(), json!(now_rfc3339()));
        body.insert("text".into(), serde_json::to_value(&text_ref).unwrap());
        body.insert("structured".into(), structured);
        body.insert("captured_before".into(), json!(captured_before));
        body.insert("channel".into(), json!(channel));
        body.insert("auth_strength".into(), json!(auth_strength));
        body.insert(
            "amends".into(),
            amends.map(Value::from).unwrap_or(Value::Null),
        );
        let sealed = canon::seal("int", body, &self.user_sk)?;
        let id = trace::put_object(&self.conn, "intent", &sealed, &now_rfc3339())?;
        self.substrate_event(
            None,
            "intent",
            json!({
                "intent": id,
                "channel": channel,
                "auth_strength": auth_strength,
                "captured_before": captured_before,
            }),
        )?;
        Ok(id)
    }

    // ---- drift (A12) ---------------------------------------------------

    /// Serialize gates (promotion, approval-time re-merge, revert) per
    /// fabric home (M8, A20). Cross-PROCESS by construction — concurrent
    /// proxies on one home are an observed fact, not a hypothetical — via
    /// flock on `<home>/gate.lock`; released when the guard drops.
    /// Fabric-home grain, not per-store: gates consume and attest all
    /// roots as one coherent tuple, and per-store locks could deadlock.
    pub fn gate_lock(&self) -> Result<GateGuard, KernelError> {
        let path = self.home.join("gate.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|source| snapshot::SnapError::Io { path: path.clone(), source })?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(snapshot::SnapError::Io {
                    path,
                    source: std::io::Error::last_os_error(),
                }
                .into());
            }
        }
        Ok(GateGuard { _file: file })
    }

    /// The A13 operation-class summary of a divergence (A20): what changed
    /// between two attested roots, in the same vocabulary rules and
    /// promotion previews speak. Opaque stores degrade to a whole-store
    /// `modify` (SI-18). Paths land in the plaintext substrate — the same
    /// exposure promotion event bodies already accept.
    fn divergence_ops(&self, spec: &StoreSpec, expected: &str, observed: &str) -> Vec<Value> {
        match spec.kind {
            crate::snapshot::StoreKind::Fs => {
                let trees = (
                    snapshot::load_tree_object(&self.cas, expected),
                    snapshot::load_tree_object(&self.cas, observed),
                );
                match trees {
                    (Ok(e), Ok(o)) => {
                        // base = trunk = expected: every difference reads as
                        // observed-side ops, with rename/move detection, and
                        // conflicts are impossible by construction.
                        let e = crate::promote::tree_from_object(&e);
                        let o = crate::promote::tree_from_object(&o);
                        crate::promote::three_way(&e, &o, &e)
                            .ops
                            .iter()
                            .map(crate::promote::Op::to_json)
                            .collect()
                    }
                    _ => vec![json!({"op": "modify", "path": "(tree unavailable)"})],
                }
            }
            crate::snapshot::StoreKind::Sqlite => {
                vec![json!({"op": "modify", "path": format!("{} (whole store)", spec.store)})]
            }
        }
    }

    /// Compare each store's live root against the last attested root and
    /// emit `drift` events for divergences.
    ///
    /// Attribution, Stage 1 (single-human zero-authorship default, A12):
    /// every expected-root update happens at a step boundary or tool_call,
    /// so a divergence found here occurred outside any recorded agent
    /// action; with exactly one human on the machine it attributes quietly
    /// to `human_local`. `tool_known` and `unattributed` become reachable in
    /// milestone 2 when the broker's write log and external tools exist.
    ///
    /// M8 (A20): one drift event per divergence window — emission updates
    /// the expected root, so re-checks are silent until the next
    /// divergence. Gates call this before any merge or revert consumes
    /// live state; the body carries the A13 op summary so the narrative is
    /// equivalent no matter when the change happened.
    pub fn check_drift(&mut self) -> Result<Vec<DriftReport>, KernelError> {
        let mut reports = Vec::new();
        let head = trace::head_offset(&self.conn)?;
        for spec in self.stores.clone() {
            let observed = snapshot::capture(&self.cas, &spec)?;
            let expected: Option<(String, i64)> = self
                .conn
                .query_row(
                    "SELECT root, since_offset FROM expected_roots WHERE store = ?1",
                    [&spec.store],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((expected_root, since)) = expected {
                if expected_root != observed {
                    let attribution = "human_local";
                    let ops = self.divergence_ops(&spec, &expected_root, &observed);
                    let ev = self.substrate_event(
                        None,
                        "drift",
                        json!({
                            "store": spec.store,
                            "expected_root": expected_root,
                            "observed_root": observed,
                            "between": [since, head],
                            "attribution": attribution,
                            "ops": ops,
                        }),
                    )?;
                    self.set_expected(&spec.store, &observed, ev.offset)?;
                    reports.push(DriftReport {
                        store: spec.store.clone(),
                        expected_root,
                        observed_root: observed,
                        between: (since, head),
                        attribution: attribution.into(),
                        event_id: ev.id,
                    });
                }
            }
        }
        Ok(reports)
    }

    fn set_expected(&self, store: &str, root: &str, offset: i64) -> Result<(), KernelError> {
        self.conn.execute(
            "INSERT INTO expected_roots (store, root, since_offset) VALUES (?1,?2,?3)
             ON CONFLICT(store) DO UPDATE SET root = excluded.root,
                                              since_offset = excluded.since_offset",
            params![store, root, offset],
        )?;
        Ok(())
    }

    // ---- step boundary (§3, M5) ----------------------------------------

    /// Create a manifest at a step boundary: drift-check, capture every
    /// registered root, bind state + behavior + a fresh trace span, sign,
    /// record. If a parent manifest exists this is a re-manifest (M5 when
    /// the behavior hash changed; M6 makes frequent re-manifesting normal).
    ///
    /// This form declares no authority mode — an observed run (M7).
    pub fn step_boundary(
        &mut self,
        delegator: &str,
        delegate: &str,
        intent: &str,
        behavior: Value,
    ) -> Result<StepOutcome, KernelError> {
        self.step_boundary_with_mode(delegator, delegate, intent, behavior, AuthorityMode::Observed)
    }

    /// `step_boundary` with an explicit M7 authority-mode declaration.
    /// `Brokered` seals `authority: {"mode":"brokered"}` into the body; the
    /// gate then requires every effect to be capability-attributed with a
    /// verified grant event at a lower substrate offset (A21, fail-closed).
    pub fn step_boundary_with_mode(
        &mut self,
        delegator: &str,
        delegate: &str,
        intent: &str,
        behavior: Value,
        mode: AuthorityMode,
    ) -> Result<StepOutcome, KernelError> {
        // Authenticate the existing lineage before drift accounting or CAS
        // capture can mutate fabric state. An invalid materialized parent is
        // not a step boundary and must create no derivative evidence.
        let parent = trace::meta_get(&self.conn, "current_manifest")?;
        let behavior_changed = match &parent {
            Some(p) => {
                let pm = self.load_manifest(p)?;
                pm.get("behavior") != Some(&behavior)
            }
            None => false,
        };

        let drift = self.check_drift()?;

        let mut roots = Vec::new();
        for spec in &self.stores {
            let root = snapshot::capture(&self.cas, spec)?;
            roots.push(json!({
                "store": spec.store,
                "tier": spec.tier,
                "kind": spec.kind.as_str(),
                "root": root,
            }));
        }

        let span = trace::new_span();
        let substrate_offset = trace::head_offset(&self.conn)?;
        let mut body = Map::new();
        body.insert(
            "parent".into(),
            parent.clone().map(Value::from).unwrap_or(Value::Null),
        );
        body.insert("created_at".into(), json!(now_rfc3339()));
        body.insert("delegator".into(), json!(delegator));
        body.insert("delegate".into(), json!(delegate));
        body.insert("intent".into(), json!(intent));
        body.insert("state".into(), json!({ "roots": roots }));
        if mode == AuthorityMode::Brokered {
            body.insert("authority".into(), json!({ "mode": "brokered" }));
        }
        body.insert("behavior".into(), behavior);
        body.insert(
            "trace".into(),
            json!({ "span": span, "substrate_offset": substrate_offset }),
        );
        let sealed = canon::seal("man", body, &self.fabric_sk)?;
        let man_id = trace::put_object(&self.conn, "manifest", &sealed, &now_rfc3339())?;

        let snap_ev = {
            let span = span.clone();
            trace::append(
                &mut self.conn,
                &self.fabric_sk,
                &span,
                Some(&man_id),
                "snapshot",
                json!({ "roots": sealed["state"]["roots"] }),
                &now_rfc3339(),
            )?
        };
        if let Some(p) = &parent {
            let span = span.clone();
            trace::append(
                &mut self.conn,
                &self.fabric_sk,
                &span,
                Some(&man_id),
                "remanifest",
                json!({ "parent": p, "behavior_changed": behavior_changed }),
                &now_rfc3339(),
            )?;
        }

        for r in sealed["state"]["roots"].as_array().expect("roots array") {
            self.set_expected(
                r["store"].as_str().unwrap(),
                r["root"].as_str().unwrap(),
                snap_ev.offset,
            )?;
        }
        trace::meta_set(&self.conn, "current_manifest", &man_id)?;
        trace::meta_set(&self.conn, "current_span", &span)?;

        Ok(StepOutcome {
            manifest: man_id,
            span,
            drift,
            remanifest: parent.is_some(),
        })
    }

    pub fn current_manifest(&self) -> Result<Option<String>, KernelError> {
        Ok(trace::meta_get(&self.conn, "current_manifest")?)
    }

    // ---- tool calls ------------------------------------------------------

    /// Record a tool call in the active span (§6 tool_call body). Args and
    /// result go to the payload store; `summary` must already be the
    /// caveat-relevant extract (F4); `checks` is the broker's caveat check
    /// record (empty array pre-broker). Undeclared reversibility defaults to
    /// `irreversible` (§0). Captures `state_root_after` for every registered
    /// store and advances the expected roots.
    #[allow(clippy::too_many_arguments)]
    pub fn record_tool_call(
        &mut self,
        tool: &str,
        action: &str,
        args: &[u8],
        result: &[u8],
        summary: Value,
        checks: Value,
        reversibility: Option<&str>,
    ) -> Result<trace::Appended, KernelError> {
        let manifest = trace::meta_get(&self.conn, "current_manifest")?
            .ok_or(KernelError::NoActiveManifest)?;
        let span = trace::meta_get(&self.conn, "current_span")?
            .ok_or(KernelError::NoActiveManifest)?;
        let args_ref = payload::put(&self.conn, &self.kek, args, "application/json", &now_rfc3339())?;
        let result_ref =
            payload::put(&self.conn, &self.kek, result, "application/json", &now_rfc3339())?;

        // Branched runs capture the agent's world (the branch) under
        // "branch:" keys and leave trunk expectations alone — trunk only
        // moves at promotion. Unbranched runs capture live stores.
        let mut roots_after = Map::new();
        match &self.branch {
            Some(branch) => {
                for spec in &self.stores {
                    if let Some(bpath) = branch.get(&spec.store) {
                        let mut bspec = spec.clone();
                        bspec.path = bpath.clone();
                        roots_after.insert(
                            format!("branch:{}", spec.store),
                            Value::String(snapshot::capture(&self.cas, &bspec)?),
                        );
                    }
                }
            }
            None => {
                for spec in &self.stores {
                    roots_after.insert(
                        spec.store.clone(),
                        Value::String(snapshot::capture(&self.cas, spec)?),
                    );
                }
            }
        }

        let ev = trace::append(
            &mut self.conn,
            &self.fabric_sk,
            &span,
            Some(&manifest),
            "tool_call",
            json!({
                "tool": tool,
                "action": action,
                "args": serde_json::to_value(&args_ref).unwrap(),
                "result": serde_json::to_value(&result_ref).unwrap(),
                "summary": summary,
                "checks": checks,
                "reversibility": reversibility.unwrap_or("irreversible"),
                "compensator": Value::Null,
                "state_root_after": Value::Object(roots_after.clone()),
            }),
            &now_rfc3339(),
        )?;
        for (store, root) in &roots_after {
            if store.starts_with("branch:") {
                continue; // trunk expectations move only at promotion
            }
            self.set_expected(store, root.as_str().expect("root is string"), ev.offset)?;
        }
        Ok(ev)
    }

    // ---- revert (§5.3 coherent revert) ----------------------------------

    /// Revert ALL of a manifest's state roots atomically-in-effect:
    /// every root is staged before any store is touched, then all are
    /// swapped. Emits the `revert` event and re-baselines expectations —
    /// undo never gaslights the agent with a world its memory contradicts.
    pub fn revert_to(&mut self, manifest_id: &str) -> Result<(), KernelError> {
        self.revert_to_with_hook(manifest_id, &mut |_| Ok(()))
    }

    fn revert_to_with_hook(
        &mut self,
        manifest_id: &str,
        hook: &mut dyn FnMut(StateChangePoint) -> Result<(), KernelError>,
    ) -> Result<(), KernelError> {
        // Authenticate the requested authority object before a rejected
        // revert can emit drift evidence or capture any new CAS content.
        let man = self.load_manifest(manifest_id)?;
        // M8: revert consumes live state exactly like a merge does — any
        // out-of-band divergence must be attributed BEFORE the restore
        // erases it, and gates serialize per home.
        let _gate = self.gate_lock()?;
        self.check_drift()?;
        let roots = man["state"]["roots"]
            .as_array()
            .ok_or_else(|| {
                KernelError::MalformedManifest(manifest_id.into(), "state.roots missing".into())
            })?
            .clone();

        let mut transitions = Vec::new();
        let mut restored = Vec::new();
        for r in &roots {
            let store_name = r["store"].as_str().ok_or_else(|| {
                KernelError::MalformedManifest(manifest_id.into(), "root without store".into())
            })?;
            let root = r["root"].as_str().ok_or_else(|| {
                KernelError::MalformedManifest(manifest_id.into(), "root without hash".into())
            })?;
            let spec = self.store(store_name)?.clone();
            transitions.push(StateRootTransition {
                store: store_name.to_string(),
                before: snapshot::capture(&self.cas, &spec)?,
                after: root.to_string(),
            });
            restored.push(json!({ "store": store_name, "root": root }));
        }
        self.commit_state_change_with_hook(
            StateChangePlan {
                manifest: manifest_id.to_string(),
                event_kind: "revert".into(),
                event_body: json!({ "manifest": manifest_id, "roots_restored": restored }),
                transitions,
                set_current_manifest: true,
            },
            |_, _| Ok(()),
            hook,
        )?;
        Ok(())
    }

    // ---- shred -----------------------------------------------------------

    /// Crypto-shred a payload and record that the ledger forgot (§1, §6).
    pub fn shred_payload(&mut self, hash: &str, reason: &str) -> Result<(), KernelError> {
        payload::shred(&self.conn, hash, reason, &now_rfc3339())?;
        self.substrate_event(
            None,
            "shred",
            json!({ "payload_hash": hash, "reason": reason }),
        )?;
        Ok(())
    }

    pub fn get_payload(&self, r: &PayloadRef) -> Result<Vec<u8>, KernelError> {
        Ok(payload::get(&self.conn, &self.kek, &r.hash)?)
    }

    // ---- verification / ledger ------------------------------------------

    /// Verify every span's hash chain and signatures. Returns
    /// (span, verified_event_count) pairs.
    pub fn verify_all_spans(&self) -> Result<Vec<(String, usize)>, KernelError> {
        let spans: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT DISTINCT span FROM events ORDER BY span")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let vk = self.fabric_vk();
        let mut out = Vec::new();
        for span in spans {
            let n = trace::verify_span(&self.conn, &vk, &span)?;
            out.push((span, n));
        }
        Ok(out)
    }

    /// Read-only operator diagnostic view. Unlike the authority view, this
    /// reports every unverified retained row as an integrity finding instead
    /// of omitting it. SI-25 still bounds cross-span offset/head claims.
    pub fn ledger_event_view(&self) -> Result<trace::LedgerEventView, KernelError> {
        Ok(trace::ledger_event_view(&self.conn, &self.fabric_vk())?)
    }

    /// The diff-attribution ledger: replay every event and require that
    /// each store's root only ever changes with a recorded cause. Returns
    /// one line per event plus the final accounting; errors if any root
    /// transition lacks an explaining event (that would mean the substrate
    /// itself failed, not just out-of-band drift — drift IS a cause).
    pub fn explain(&self) -> Result<Explanation, KernelError> {
        let view = self.ledger_event_view()?;
        // RF-28: never let a partial diagnostic view reach root accounting.
        // In particular, the authority view's deliberate unsigned-row omission
        // is not an acceptable posture for an operator integrity surface.
        view.w19_require_clean()?;
        let events = &view.events;
        let mut lines = Vec::new();
        let mut roots: std::collections::BTreeMap<String, String> = Default::default();
        for ev in events {
            let body = &ev.raw["body"];
            let line = match ev.kind.as_str() {
                "register" => format!(
                    "registered {} {}",
                    body["object_kind"].as_str().unwrap_or("?"),
                    body["object"].as_str().unwrap_or("?")
                ),
                "intent" => format!(
                    "intent {} captured via {} ({}) before offset {}",
                    body["intent"].as_str().unwrap_or("?"),
                    body["channel"].as_str().unwrap_or("?"),
                    body["auth_strength"].as_str().unwrap_or("?"),
                    body["captured_before"]
                ),
                "snapshot" => {
                    let mut parts = Vec::new();
                    for r in body["roots"].as_array().into_iter().flatten() {
                        let store = r["store"].as_str().unwrap_or("?").to_string();
                        let root = r["root"].as_str().unwrap_or("?").to_string();
                        roots.insert(store.clone(), root.clone());
                        parts.push(format!("{store}={}", short(&root)));
                    }
                    format!(
                        "manifest {} snapshotted [{}]",
                        ev.manifest.as_deref().unwrap_or("?"),
                        parts.join(", ")
                    )
                }
                "remanifest" => format!(
                    "re-manifest from parent {} (behavior_changed={})",
                    body["parent"].as_str().unwrap_or("?"),
                    body["behavior_changed"]
                ),
                "tool_call" => {
                    if let Some(after) = body["state_root_after"].as_object() {
                        for (store, root) in after {
                            // "branch:" roots are the agent's fork, not
                            // trunk state; trunk moves at promotion.
                            if !store.starts_with("branch:") {
                                roots.insert(store.clone(), root.as_str().unwrap_or("?").into());
                            }
                        }
                    }
                    format!(
                        "tool_call {}.{} ({})",
                        body["tool"].as_str().unwrap_or("?"),
                        body["action"].as_str().unwrap_or("?"),
                        body["reversibility"].as_str().unwrap_or("?")
                    )
                }
                "drift" => {
                    let store = body["store"].as_str().unwrap_or("?").to_string();
                    let observed = body["observed_root"].as_str().unwrap_or("?").to_string();
                    roots.insert(store.clone(), observed.clone());
                    // A20: the op summary is the narrative — a drift that
                    // names its paths is much harder to misread than one
                    // that names two hashes.
                    let summary = summarize_ops(&body["ops"]);
                    format!(
                        "DRIFT in {store}: {}{} -> {} between offsets {}..{}, attributed {}",
                        summary,
                        short(body["expected_root"].as_str().unwrap_or("?")),
                        short(&observed),
                        body["between"][0],
                        body["between"][1],
                        body["attribution"].as_str().unwrap_or("?")
                    )
                }
                "revert" => {
                    let mut parts = Vec::new();
                    for r in body["roots_restored"].as_array().into_iter().flatten() {
                        let store = r["store"].as_str().unwrap_or("?").to_string();
                        let root = r["root"].as_str().unwrap_or("?").to_string();
                        roots.insert(store.clone(), root.clone());
                        parts.push(format!("{store}={}", short(&root)));
                    }
                    format!(
                        "REVERT to manifest {} [{}]",
                        body["manifest"].as_str().unwrap_or("?"),
                        parts.join(", ")
                    )
                }
                "shred" => format!(
                    "shredded payload {} ({})",
                    short(body["payload_hash"].as_str().unwrap_or("?")),
                    body["reason"].as_str().unwrap_or("?")
                ),
                "promotion" => {
                    let mut parts = Vec::new();
                    for s in body["stores"].as_array().into_iter().flatten() {
                        let store = s["store"].as_str().unwrap_or("?").to_string();
                        let merged = s["merged"].as_str().unwrap_or("?").to_string();
                        roots.insert(store.clone(), merged.clone());
                        parts.push(format!("{store}={}", short(&merged)));
                    }
                    format!(
                        "PROMOTED manifest {} to trunk [{}] — {} op(s), {} conflict(s), policy {}",
                        body["manifest"].as_str().unwrap_or("?"),
                        parts.join(", "),
                        body["ops"].as_array().map(Vec::len).unwrap_or(0),
                        body["conflicts"].as_array().map(Vec::len).unwrap_or(0),
                        body["policy"].as_str().unwrap_or("?")
                    )
                }
                "grant" => format!(
                    "capability {} granted{}",
                    body["capability"].as_str().unwrap_or("?"),
                    body["parent"]
                        .as_str()
                        .map(|p| format!(" (attenuated from {p})"))
                        .unwrap_or_default()
                ),
                // A22 (§5.4): the signed early-closure edge — permanent for
                // the id, descendant-closing through the ancestry view.
                "revoke" => format!(
                    "REVOKED capability {} ({}){}",
                    body["capability"].as_str().unwrap_or("?"),
                    body["reason"].as_str().unwrap_or("?"),
                    body["channel"]
                        .as_str()
                        .map(|c| format!(
                            " via {c} ({})",
                            body["auth_strength"].as_str().unwrap_or("?")
                        ))
                        .unwrap_or_else(|| " by broker (mechanical)".into())
                ),
                "verdict" => format!(
                    "DENY {}.{} — {}",
                    body["tool"].as_str().unwrap_or("?"),
                    body["action"].as_str().unwrap_or("?"),
                    body["structural"].as_str().map(str::to_string).unwrap_or_else(
                        || body["failed"]
                            .as_array()
                            .map(|f| f.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
                            .unwrap_or_default()
                    )
                ),
                // Promotion-policy escalations carry `promotion`, not
                // `escalation`, and their whole point is the ops preview —
                // render both or the line reads as "#null" (RF-12).
                "escalation" if body["caveat"] == "promotion.policy" => {
                    let sample = &body["sample"][0];
                    let ops: Vec<&str> = sample["ops"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    format!(
                        "PARKED promotion #{} — ops [{}], {} conflict(s); resolve via `asf approve --home <H> promotions`",
                        body["promotion"],
                        ops.join(", "),
                        sample["conflicts"]
                    )
                }
                "escalation" => format!(
                    "ESCALATE #{} caveat {} (batch count {})",
                    body["escalation"],
                    body["caveat"].as_str().unwrap_or("?"),
                    body["count"]
                ),
                // Promotion approvals carry `promotion` (no uses);
                // escalation approvals carry `escalation` + `uses` (RF-12).
                "approval" if body.get("promotion").is_some() => format!(
                    "APPROVAL promotion #{} {} via {} ({})",
                    body["promotion"],
                    body["resolution"].as_str().unwrap_or("?"),
                    body["channel"].as_str().unwrap_or("?"),
                    body["auth_strength"].as_str().unwrap_or("?")
                ),
                "approval" => format!(
                    "APPROVAL #{} {} ({} uses) via {} ({})",
                    body["escalation"],
                    body["resolution"].as_str().unwrap_or("?"),
                    body["uses"],
                    body["channel"].as_str().unwrap_or("?"),
                    body["auth_strength"].as_str().unwrap_or("?")
                ),
                other => format!("{other} event"),
            };
            lines.push(LedgerLine {
                offset: ev.offset,
                at: ev.at.clone(),
                span: ev.span.clone(),
                kind: ev.kind.clone(),
                line,
            });
        }
        // Final accounting: live roots must equal the ledger's last word.
        let mut unexplained = Vec::new();
        for spec in &self.stores {
            let live = snapshot::capture(&self.cas, spec)?;
            match roots.get(&spec.store) {
                Some(last) if *last == live => {}
                Some(last) => unexplained.push(format!(
                    "{}: live root {} but ledger says {}",
                    spec.store,
                    short(&live),
                    short(last)
                )),
                None => unexplained.push(format!("{}: never appears in ledger", spec.store)),
            }
        }
        Ok(Explanation { lines, unexplained })
    }
}

/// Render a drift body's A13 op summary (A20). Empty for pre-A20 events,
/// which carried only the two root hashes.
fn summarize_ops(ops: &Value) -> String {
    let Some(list) = ops.as_array().filter(|l| !l.is_empty()) else {
        return String::new();
    };
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut paths: Vec<String> = Vec::new();
    for o in list {
        *counts.entry(o["op"].as_str().unwrap_or("?")).or_insert(0) += 1;
        if paths.len() < 3 {
            paths.push(o["path"].as_str().map(str::to_string).unwrap_or_else(|| {
                format!(
                    "{}→{}",
                    o["from"].as_str().unwrap_or("?"),
                    o["to"].as_str().unwrap_or("?")
                )
            }));
        }
    }
    let counts_s = counts
        .iter()
        .map(|(k, v)| format!("{v} {k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let ell = if list.len() > 3 { ", …" } else { "" };
    format!("{counts_s} ({}{ell}) · ", paths.join(", "))
}

fn short(hash: &str) -> String {
    let h = hash.strip_prefix("sha256:").unwrap_or(hash);
    h.chars().take(12).collect()
}

#[derive(Debug)]
pub struct LedgerLine {
    pub offset: i64,
    pub at: String,
    pub span: String,
    pub kind: String,
    pub line: String,
}

#[derive(Debug)]
pub struct Explanation {
    pub lines: Vec<LedgerLine>,
    /// Store roots whose live value the ledger cannot account for.
    /// Non-empty means the substrate failed its own thesis.
    pub unexplained: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StateChangeWorld {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        vault: PathBuf,
        memory: PathBuf,
        fabric: Fabric,
        transitions: Vec<StateRootTransition>,
    }

    fn state_change_world() -> StateChangeWorld {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("fabric");
        let vault = tmp.path().join("vault");
        let memory = tmp.path().join("memory.db");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::write(vault.join("note.md"), b"target").unwrap();
        Connection::open(&memory)
            .unwrap()
            .execute_batch(
                "CREATE TABLE memories (fact TEXT); INSERT INTO memories VALUES ('target');",
            )
            .unwrap();
        let stores = vec![
            StoreSpec {
                store: "fs:vault".into(),
                tier: 1,
                kind: snapshot::StoreKind::Fs,
                path: vault.clone(),
            },
            StoreSpec {
                store: "db:memory".into(),
                tier: 1,
                kind: snapshot::StoreKind::Sqlite,
                path: memory.clone(),
            },
        ];
        let fabric = Fabric::initialize(&home, stores).unwrap();
        let target_fs = snapshot::capture(&fabric.cas, fabric.store("fs:vault").unwrap()).unwrap();
        let target_db = snapshot::capture(&fabric.cas, fabric.store("db:memory").unwrap()).unwrap();
        std::fs::write(vault.join("note.md"), b"before").unwrap();
        Connection::open(&memory)
            .unwrap()
            .execute("INSERT INTO memories VALUES ('before')", [])
            .unwrap();
        let before_fs = snapshot::capture(&fabric.cas, fabric.store("fs:vault").unwrap()).unwrap();
        let before_db = snapshot::capture(&fabric.cas, fabric.store("db:memory").unwrap()).unwrap();
        StateChangeWorld {
            _tmp: tmp,
            home,
            vault,
            memory,
            fabric,
            transitions: vec![
                StateRootTransition {
                    store: "fs:vault".into(),
                    before: before_fs,
                    after: target_fs,
                },
                StateRootTransition {
                    store: "db:memory".into(),
                    before: before_db,
                    after: target_db,
                },
            ],
        }
    }

    fn state_change_body(transitions: &[StateRootTransition]) -> Value {
        json!({
            "manifest": "man:test",
            "roots_restored": transitions.iter().map(|root| {
                json!({"store": root.store, "root": root.after})
            }).collect::<Vec<_>>(),
        })
    }

    fn assert_transition_side(fabric: &Fabric, transitions: &[StateRootTransition], after: bool) {
        for transition in transitions {
            let observed = snapshot::capture(
                &fabric.cas,
                fabric.store(&transition.store).unwrap(),
            )
            .unwrap();
            let expected = if after {
                &transition.after
            } else {
                &transition.before
            };
            assert_eq!(&observed, expected, "{} is on the wrong side", transition.store);
        }
    }

    fn reseal_recovery_journal(
        fabric: &Fabric,
        value: &Value,
        prefix: &str,
        mutate: impl FnOnce(&mut Map<String, Value>),
    ) -> Value {
        let mut body = value.as_object().unwrap().clone();
        body.remove("id");
        body.remove("sig");
        mutate(&mut body);
        Value::Object(canon::seal(prefix, body, fabric.fabric_sk()).unwrap())
    }

    fn unsigned_test_journal(world: &StateChangeWorld) -> Value {
        state_change_journal_value(
            world.fabric.fabric_sk(),
            &trace::Appended {
                id: "evt:test-recovery".into(),
                offset: 0,
                span: world.fabric.substrate_span().into(),
                seq: 0,
            },
            "revert",
            "man:test",
            &world.transitions,
        )
        .unwrap()
    }

    fn assert_world_files_are_before(world: &StateChangeWorld) {
        assert_eq!(std::fs::read(world.vault.join("note.md")).unwrap(), b"before");
        assert_eq!(
            Connection::open(&world.memory)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn w14_later_store_failure_rolls_back_every_root_and_event() {
        let mut world = state_change_world();
        let body = state_change_body(&world.transitions);
        let result = world.fabric.commit_state_change_with_hook(
            StateChangePlan {
                manifest: "man:test".into(),
                event_kind: "revert".into(),
                event_body: body,
                transitions: world.transitions.clone(),
                set_current_manifest: true,
            },
            |_, _| Ok(()),
            &mut |point| {
                if point == StateChangePoint::AfterStore("db:memory".into()) {
                    Err(KernelError::StateChange("injected later-store failure".into()))
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.is_err());
        assert_transition_side(&world.fabric, &world.transitions, false);
        assert!(!world.home.join(STATE_CHANGE_JOURNAL).exists());
        assert!(trace::verified_events(&world.fabric.conn, &world.fabric.fabric_vk())
            .unwrap()
            .iter()
            .all(|event| event.kind != "revert"));
    }

    #[test]
    fn w14_reopen_rolls_back_uncommitted_partial_state_change() {
        let mut world = state_change_world();
        let body = state_change_body(&world.transitions);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = world.fabric.commit_state_change_with_hook(
                StateChangePlan {
                    manifest: "man:test".into(),
                    event_kind: "revert".into(),
                    event_body: body,
                    transitions: world.transitions.clone(),
                    set_current_manifest: true,
                },
                |_, _| Ok(()),
                &mut |point| {
                    if point == StateChangePoint::AfterStore("db:memory".into()) {
                        panic!("simulated hard exit");
                    }
                    Ok(())
                },
            );
        }));
        assert!(result.is_err());
        assert!(world.home.join(STATE_CHANGE_JOURNAL).is_file());
        drop(world.fabric);
        let reopened = Fabric::open_existing(&world.home).unwrap();
        assert_transition_side(&reopened, &world.transitions, false);
        assert!(!world.home.join(STATE_CHANGE_JOURNAL).exists());
        assert_eq!(std::fs::read(world.vault.join("note.md")).unwrap(), b"before");
        assert_eq!(
            Connection::open(&world.memory)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn w14_reopen_rolls_forward_committed_state_change() {
        let mut world = state_change_world();
        let body = state_change_body(&world.transitions);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = world.fabric.commit_state_change_with_hook(
                StateChangePlan {
                    manifest: "man:test".into(),
                    event_kind: "revert".into(),
                    event_body: body,
                    transitions: world.transitions.clone(),
                    set_current_manifest: true,
                },
                |_, _| Ok(()),
                &mut |point| {
                    if point == StateChangePoint::AfterDatabaseCommit {
                        panic!("simulated hard exit after ledger commit");
                    }
                    Ok(())
                },
            );
        }));
        assert!(result.is_err());
        assert!(world.home.join(STATE_CHANGE_JOURNAL).is_file());
        drop(world.fabric);
        let reopened = Fabric::open_existing(&world.home).unwrap();
        assert_transition_side(&reopened, &world.transitions, true);
        assert!(!world.home.join(STATE_CHANGE_JOURNAL).exists());
        assert_eq!(std::fs::read(world.vault.join("note.md")).unwrap(), b"target");
        assert_eq!(
            Connection::open(&world.memory)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn w14_store_sync_failure_rolls_back_every_root_and_event() {
        let mut world = state_change_world();
        let body = state_change_body(&world.transitions);
        let vault = world.vault.clone();
        let result = world.fabric.commit_state_change_with_hook(
            StateChangePlan {
                manifest: "man:test".into(),
                event_kind: "revert".into(),
                event_body: body,
                transitions: world.transitions.clone(),
                set_current_manifest: true,
            },
            |_, _| Ok(()),
            &mut |point| {
                if point == StateChangePoint::BeforeStoreSync("fs:vault".into()) {
                    std::fs::remove_dir_all(&vault).unwrap();
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_transition_side(&world.fabric, &world.transitions, false);
        assert!(!world.home.join(STATE_CHANGE_JOURNAL).exists());
        assert!(trace::verified_events(&world.fabric.conn, &world.fabric.fabric_vk())
            .unwrap()
            .iter()
            .all(|event| event.kind != "revert"));
    }

    #[test]
    fn w14_reopen_rolls_forward_committed_promotion() {
        let mut world = state_change_world();
        let stores = world
            .transitions
            .iter()
            .map(|root| {
                json!({
                    "store": root.store,
                    "base": root.before,
                    "branch": root.after,
                    "trunk_before": root.before,
                    "merged": root.after,
                })
            })
            .collect::<Vec<_>>();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = world.fabric.commit_state_change_with_hook(
                StateChangePlan {
                    manifest: "man:test".into(),
                    event_kind: "promotion".into(),
                    event_body: json!({
                        "manifest": "man:test",
                        "stores": stores,
                        "ops": [],
                        "conflicts": [],
                        "trace_check": {},
                        "policy": "test",
                    }),
                    transitions: world.transitions.clone(),
                    set_current_manifest: false,
                },
                |_, _| Ok(()),
                &mut |point| {
                    if point == StateChangePoint::AfterDatabaseCommit {
                        panic!("simulated hard exit after promotion commit");
                    }
                    Ok(())
                },
            );
        }));
        assert!(result.is_err());
        drop(world.fabric);
        let reopened = Fabric::open_existing(&world.home).unwrap();
        assert_transition_side(&reopened, &world.transitions, true);
        assert!(!world.home.join(STATE_CHANGE_JOURNAL).exists());
    }

    #[test]
    fn w14_reopen_rejects_mistyped_or_unsafe_recovery_journal_before_mutation() {
        for variant in ["prefix", "kind", "version"] {
            let world = state_change_world();
            let journal = unsigned_test_journal(&world);
            let malformed = match variant {
                "prefix" => reseal_recovery_journal(&world.fabric, &journal, "cap", |_| {}),
                "kind" => reseal_recovery_journal(&world.fabric, &journal, "txn", |body| {
                    body.insert("kind".into(), json!("other_recovery"));
                }),
                "version" => reseal_recovery_journal(&world.fabric, &journal, "txn", |body| {
                    body.insert("version".into(), json!(2));
                }),
                _ => unreachable!(),
            };
            std::fs::write(
                world.home.join(STATE_CHANGE_JOURNAL),
                serde_json::to_vec(&malformed).unwrap(),
            )
            .unwrap();
            assert!(Fabric::open_existing(&world.home).is_err(), "accepted {variant} variant");
            assert_world_files_are_before(&world);
        }

        let world = state_change_world();
        std::fs::create_dir(world.home.join(STATE_CHANGE_JOURNAL)).unwrap();
        assert!(Fabric::open_existing(&world.home).is_err());
        assert_world_files_are_before(&world);

        #[cfg(unix)]
        {
            let world = state_change_world();
            let target = world.home.join("journal-target");
            std::fs::write(&target, b"not a journal").unwrap();
            std::os::unix::fs::symlink(&target, world.home.join(STATE_CHANGE_JOURNAL)).unwrap();
            assert!(Fabric::open_existing(&world.home).is_err());
            assert_world_files_are_before(&world);
        }
    }

    #[test]
    fn w14_reopen_rejects_journal_event_misbinding_without_restore() {
        for field in ["manifest", "event_kind"] {
            let mut world = state_change_world();
            let body = state_change_body(&world.transitions);
            let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = world.fabric.commit_state_change_with_hook(
                    StateChangePlan {
                        manifest: "man:test".into(),
                        event_kind: "revert".into(),
                        event_body: body,
                        transitions: world.transitions.clone(),
                        set_current_manifest: true,
                    },
                    |_, _| Ok(()),
                    &mut |point| {
                        if point == StateChangePoint::AfterDatabaseCommit {
                            panic!("simulated committed crash");
                        }
                        Ok(())
                    },
                );
            }));
            assert!(crashed.is_err());
            let path = world.home.join(STATE_CHANGE_JOURNAL);
            let journal = canon::parse_fabric_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let malformed = reseal_recovery_journal(&world.fabric, &journal, "txn", |body| {
                body.insert(
                    field.into(),
                    if field == "manifest" {
                        json!("man:other")
                    } else {
                        json!("promotion")
                    },
                );
            });
            std::fs::write(&path, serde_json::to_vec(&malformed).unwrap()).unwrap();
            std::fs::write(world.vault.join("note.md"), b"must-not-be-restored").unwrap();
            assert!(Fabric::open_existing(&world.home).is_err(), "accepted {field} mismatch");
            assert_eq!(
                std::fs::read(world.vault.join("note.md")).unwrap(),
                b"must-not-be-restored"
            );
        }
    }

    fn tree_bytes(root: &Path) -> std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> {
        fn visit(
            root: &Path,
            path: &Path,
            out: &mut std::collections::BTreeMap<PathBuf, Option<Vec<u8>>>,
        ) {
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let metadata = std::fs::symlink_metadata(path).unwrap();
            if metadata.is_dir() {
                out.insert(relative, None);
                let mut children: Vec<_> = std::fs::read_dir(path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect();
                children.sort();
                for child in children {
                    visit(root, &child, out);
                }
            } else {
                out.insert(relative, Some(std::fs::read(path).unwrap()));
            }
        }

        let mut out = std::collections::BTreeMap::new();
        if root.exists() {
            visit(root, root, &mut out);
        }
        out
    }

    #[test]
    fn initialized_fabric_reopens_with_same_home_identities() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("fabric");
        std::fs::create_dir(&home).unwrap();
        let fabric = Fabric::initialize(&home, vec![]).unwrap();
        let fabric_vk = fabric.fabric_vk();
        let user_vk = fabric
            .keystore()
            .verifying_key(Role::UserRoot)
            .unwrap();
        let kek_id = fabric.kek.kek_id.clone();
        drop(fabric);

        let reopened = Fabric::open_existing(&home).unwrap();
        assert_eq!(reopened.fabric_vk(), fabric_vk);
        assert_eq!(
            reopened
                .keystore()
                .verifying_key(Role::UserRoot)
                .unwrap(),
            user_vk
        );
        assert_eq!(reopened.kek.kek_id, kek_id);
    }

    #[test]
    fn existing_fabric_key_loss_fails_without_home_mutation() {
        let tmp = tempfile::tempdir().unwrap();
        for lost in ["fabric.ed25519", "user_root.ed25519", "owner.kek"] {
            let home = tmp.path().join(lost);
            drop(Fabric::initialize(&home, vec![]).unwrap());
            std::fs::remove_file(home.join("keys").join(lost)).unwrap();
            let before = tree_bytes(&home);

            assert!(matches!(
                Fabric::open_existing(&home),
                Err(KernelError::Keys(crate::keys::KeyError::Missing(_)))
            ));
            assert_eq!(tree_bytes(&home), before);
            assert!(!home.join("keys").join(lost).exists());
        }
    }

    #[test]
    fn partial_fabric_initialization_never_completes_itself() {
        let tmp = tempfile::tempdir().unwrap();

        let database_only = tmp.path().join("database-only");
        std::fs::create_dir(&database_only).unwrap();
        drop(Connection::open(database_only.join("fabric.db")).unwrap());

        let keys_only = tmp.path().join("keys-only");
        std::fs::create_dir(&keys_only).unwrap();
        Keystore::initialize(keys_only.join("keys")).unwrap();

        let keys_and_empty_database = tmp.path().join("keys-and-empty-database");
        std::fs::create_dir(&keys_and_empty_database).unwrap();
        Keystore::initialize(keys_and_empty_database.join("keys")).unwrap();
        drop(Connection::open(keys_and_empty_database.join("fabric.db")).unwrap());

        let mixed = tmp.path().join("mixed");
        std::fs::create_dir(&mixed).unwrap();
        drop(Connection::open(mixed.join("fabric.db")).unwrap());
        std::fs::create_dir(mixed.join("keys")).unwrap();
        std::fs::write(mixed.join("keys/fabric.ed25519"), [0x33; 32]).unwrap();

        for home in [
            &database_only,
            &keys_only,
            &keys_and_empty_database,
            &mixed,
        ] {
            let before = tree_bytes(home);
            assert!(Fabric::initialize(home, vec![]).is_err());
            assert!(Fabric::open_existing(home).is_err());
            assert_eq!(tree_bytes(home), before);
        }
    }

    #[test]
    fn existing_fabric_missing_or_substituted_cas_fails_without_repair() {
        let tmp = tempfile::tempdir().unwrap();

        let missing = tmp.path().join("missing-cas");
        drop(Fabric::initialize(&missing, vec![]).unwrap());
        std::fs::remove_dir(missing.join("cas")).unwrap();
        let before = tree_bytes(&missing);
        assert!(Fabric::open_existing(&missing).is_err());
        assert_eq!(tree_bytes(&missing), before);
        assert!(!missing.join("cas").exists());

        let file = tmp.path().join("file-cas");
        drop(Fabric::initialize(&file, vec![]).unwrap());
        std::fs::remove_dir(file.join("cas")).unwrap();
        std::fs::write(file.join("cas"), b"not a directory").unwrap();
        let before = tree_bytes(&file);
        assert!(Fabric::open_existing(&file).is_err());
        assert_eq!(tree_bytes(&file), before);

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let linked = tmp.path().join("linked-cas");
            let target = tmp.path().join("cas-target");
            std::fs::create_dir(&target).unwrap();
            std::fs::write(target.join("sentinel"), b"unchanged").unwrap();
            drop(Fabric::initialize(&linked, vec![]).unwrap());
            std::fs::remove_dir(linked.join("cas")).unwrap();
            symlink(&target, linked.join("cas")).unwrap();

            assert!(Fabric::open_existing(&linked).is_err());
            assert!(std::fs::symlink_metadata(linked.join("cas"))
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(std::fs::read(target.join("sentinel")).unwrap(), b"unchanged");
        }
    }

    #[cfg(unix)]
    #[test]
    fn fabric_home_and_database_symlinks_cannot_redirect_reopen_or_initialization() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let empty_target = tmp.path().join("empty-target");
        let init_link = tmp.path().join("init-link");
        std::fs::create_dir(&empty_target).unwrap();
        symlink(&empty_target, &init_link).unwrap();
        let before = tree_bytes(&empty_target);

        assert!(matches!(
            Fabric::initialize(&init_link, vec![]),
            Err(KernelError::AlreadyInitialized { .. })
        ));
        assert_eq!(tree_bytes(&empty_target), before);

        let initialized_target = tmp.path().join("initialized-target");
        drop(Fabric::initialize(&initialized_target, vec![]).unwrap());
        let reopen_link = tmp.path().join("reopen-link");
        symlink(&initialized_target, &reopen_link).unwrap();
        let before = tree_bytes(&initialized_target);
        assert!(Fabric::open_existing(&reopen_link).is_err());
        assert_eq!(tree_bytes(&initialized_target), before);

        let database_home = tmp.path().join("database-link");
        drop(Fabric::initialize(&database_home, vec![]).unwrap());
        let foreign_database = tmp.path().join("foreign.db");
        std::fs::write(&foreign_database, b"foreign sentinel").unwrap();
        std::fs::remove_file(database_home.join("fabric.db")).unwrap();
        symlink(&foreign_database, database_home.join("fabric.db")).unwrap();
        let foreign_before = std::fs::read(&foreign_database).unwrap();

        assert!(Fabric::open_existing(&database_home).is_err());
        assert_eq!(std::fs::read(&foreign_database).unwrap(), foreign_before);
        assert!(std::fs::symlink_metadata(database_home.join("fabric.db"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn patched_sqlite_runtime_opens_fabric() {
        assert!(
            rusqlite::version_number() >= MIN_SAFE_SQLITE_VERSION,
            "bundled SQLite {} is below RF-21 floor {}",
            rusqlite::version(),
            MIN_SAFE_SQLITE_VERSION
        );
        let tmp = tempfile::tempdir().unwrap();
        let floor_home = tmp.path().join("exact-floor");
        let floor = Fabric::initialize_with_sqlite_version(
            &floor_home,
            vec![],
            MIN_SAFE_SQLITE_VERSION,
        )
        .unwrap();
        assert!(floor_home.join("fabric.db").is_file());
        drop(floor);

        let bundled_home = tmp.path().join("bundled-runtime");
        let bundled = Fabric::initialize(&bundled_home, vec![]).unwrap();
        assert!(bundled_home.join("fabric.db").is_file());
        drop(bundled);
    }

    #[test]
    fn affected_sqlite_runtime_creates_no_fabric_state() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("must-not-exist");
        let result =
            Fabric::initialize_with_sqlite_version(&home, vec![], MIN_SAFE_SQLITE_VERSION - 1);
        assert!(matches!(
            result,
            Err(KernelError::UnsafeSqliteVersion { .. })
        ));
        assert!(
            !home.exists(),
            "an unsafe SQLite runtime must not initialize any protected fabric state"
        );
    }
}
