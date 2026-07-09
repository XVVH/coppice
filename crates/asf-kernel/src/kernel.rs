//! The kernel loop: step-boundary manifests, drift detection, coherent
//! revert. Spec §3 (DelegationManifest), §6 (events), A12 (drift
//! attribution); brief §4.
//!
//! Stage 1 scope: Tier-1 stores only, no broker, no capabilities —
//! `authority` is omitted from manifests (SI-7) and `checks` on tool_call
//! events are empty until milestone 2 wires capability evaluation in.

use crate::canon;
use crate::keys::{Kek, Keystore, Role};
use crate::payload::{self, PayloadRef};
use crate::snapshot::{self, Cas, StoreSpec};
use crate::trace;
use crate::now_rfc3339;
use ed25519_dalek::SigningKey;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::path::Path;

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
    #[error("no active manifest — call step_boundary first")]
    NoActiveManifest,
    #[error("unknown store {0}")]
    UnknownStore(String),
    #[error("manifest {0} malformed: {1}")]
    MalformedManifest(String, String),
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

impl Fabric {
    /// Open (or create) a fabric home at `dir`, coordinating `stores`.
    /// Layout: `dir/fabric.db`, `dir/cas/`, `dir/keys/`.
    pub fn open(dir: impl AsRef<Path>, stores: Vec<StoreSpec>) -> Result<Self, KernelError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).map_err(|source| snapshot::SnapError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
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
        let keystore = Keystore::open(dir.join("keys"))?;
        let fabric_sk = keystore.signing_key(Role::Fabric)?;
        let user_sk = keystore.signing_key(Role::UserRoot)?;
        let kek = keystore.kek()?;
        let cas = Cas::open(dir.join("cas"))?;
        let substrate_span = match trace::meta_get(&conn, "substrate_span")? {
            Some(s) => s,
            None => {
                let s = trace::new_span();
                trace::meta_set(&conn, "substrate_span", &s)?;
                s
            }
        };
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
        let dir = dir.as_ref();
        let conn = Connection::open(dir.join("fabric.db"))?;
        let raw = trace::meta_get(&conn, "stores")?
            .ok_or_else(|| KernelError::UnknownStore("no stores recorded in this home".into()))?;
        drop(conn);
        let specs: Vec<Value> = serde_json::from_str(&raw).expect("stored specs are valid JSON");
        let stores = specs
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
            .collect();
        Self::open(dir, stores)
    }

    /// Fork a manifest's state into a working branch (brief §3 principle 2:
    /// working state lives on branches; promotion is the only mutation).
    /// Materializes every root under `<home>/branches/<manifest>/` and
    /// switches tool-call capture to the branch. Returns store → path.
    pub fn create_branch(
        &mut self,
        manifest_id: &str,
    ) -> Result<std::collections::BTreeMap<String, std::path::PathBuf>, KernelError> {
        let man = trace::get_object(&self.conn, manifest_id)?;
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

    /// Compare each store's live root against the last attested root and
    /// emit `drift` events for divergences.
    ///
    /// Attribution, Stage 1 (single-human zero-authorship default, A12):
    /// every expected-root update happens at a step boundary or tool_call,
    /// so a divergence found here occurred outside any recorded agent
    /// action; with exactly one human on the machine it attributes quietly
    /// to `human_local`. `tool_known` and `unattributed` become reachable in
    /// milestone 2 when the broker's write log and external tools exist.
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
                    let ev = self.substrate_event(
                        None,
                        "drift",
                        json!({
                            "store": spec.store,
                            "expected_root": expected_root,
                            "observed_root": observed,
                            "between": [since, head],
                            "attribution": attribution,
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

    pub(crate) fn set_expected_root(
        &self,
        store: &str,
        root: &str,
        offset: i64,
    ) -> Result<(), KernelError> {
        self.set_expected(store, root, offset)
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
    pub fn step_boundary(
        &mut self,
        delegator: &str,
        delegate: &str,
        intent: &str,
        behavior: Value,
    ) -> Result<StepOutcome, KernelError> {
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

        let parent = trace::meta_get(&self.conn, "current_manifest")?;
        let behavior_changed = match &parent {
            Some(p) => {
                let pm = trace::get_object(&self.conn, p)?;
                pm.get("behavior") != Some(&behavior)
            }
            None => false,
        };

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
        // `authority` omitted in Stage 1 — SI-7.
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
        let man = trace::get_object(&self.conn, manifest_id)?;
        let roots = man["state"]["roots"]
            .as_array()
            .ok_or_else(|| {
                KernelError::MalformedManifest(manifest_id.into(), "state.roots missing".into())
            })?
            .clone();

        // Stage every root first; abort wholesale on any failure.
        let mut prepared = Vec::new();
        for r in &roots {
            let store_name = r["store"].as_str().ok_or_else(|| {
                KernelError::MalformedManifest(manifest_id.into(), "root without store".into())
            })?;
            let root = r["root"].as_str().ok_or_else(|| {
                KernelError::MalformedManifest(manifest_id.into(), "root without hash".into())
            })?;
            let spec = self.store(store_name)?.clone();
            prepared.push((
                store_name.to_string(),
                root.to_string(),
                snapshot::prepare_restore(&self.cas, &spec, root)?,
            ));
        }
        // Commit all restores.
        let mut restored = Vec::new();
        for (store, root, prep) in prepared {
            snapshot::commit_restore(&self.cas, prep)?;
            restored.push(json!({ "store": store, "root": root }));
        }

        let ev = self.substrate_event(
            Some(manifest_id),
            "revert",
            json!({ "manifest": manifest_id, "roots_restored": restored }),
        )?;
        for r in &roots {
            self.set_expected(
                r["store"].as_str().unwrap(),
                r["root"].as_str().unwrap(),
                ev.offset,
            )?;
        }
        trace::meta_set(&self.conn, "current_manifest", manifest_id)?;
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

    /// The diff-attribution ledger: replay every event and require that
    /// each store's root only ever changes with a recorded cause. Returns
    /// one line per event plus the final accounting; errors if any root
    /// transition lacks an explaining event (that would mean the substrate
    /// itself failed, not just out-of-band drift — drift IS a cause).
    pub fn explain(&self) -> Result<Explanation, KernelError> {
        let events = trace::all_events(&self.conn)?;
        let mut lines = Vec::new();
        let mut roots: std::collections::BTreeMap<String, String> = Default::default();
        for ev in &events {
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
                    format!(
                        "DRIFT in {store}: {} -> {} between offsets {}..{}, attributed {}",
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

fn short(hash: &str) -> String {
    let h = hash.strip_prefix("sha256:").unwrap_or(hash);
    h.chars().take(12).collect()
}

#[derive(Debug)]
pub struct LedgerLine {
    pub offset: i64,
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
