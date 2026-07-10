//! `asf proxy` — the broker daemon fronting a downstream MCP server.
//! Brief §5.3; spec C2. ADR 0003.
//!
//! Topology: agent (MCP client) ⇄ this proxy (stdio) ⇄ downstream server
//! (spawned child, stdio). Every message passes through byte-faithfully
//! EXCEPT `tools/call`, which runs the broker pipeline: allow (forward with
//! injected credentials, trace with checks), deny (tool-error to the
//! agent), or escalate (parked; the agent is told an approval is pending
//! but cannot carry it).
//!
//! The dogfooding baseline fails closed when this broker/proxy path is
//! unavailable. The brief's required loud, ledger-visible fail-open degraded
//! mode and per-capability `on_broker_outage` inversion are deferred until
//! live-egress integration; absence of that path is a tracked release gate,
//! not an implemented guarantee.
//!
//! C2 is topological here: the approval surface is a Unix socket owned by
//! this daemon (`<home>/approvals.sock`, driven by `asf approve`, a
//! separate terminal — visually and procedurally distinct from the agent's
//! chat loop). The MCP stream has no approval method; an approval arriving
//! "through the agent" is unrepresentable.

use crate::{mcp, workboard_server};
use anyhow::{bail, Context, Result};
use asf_kernel::broker::{Broker, Decision};
use asf_kernel::capability::{reach_rank, reversibility_rank};
use asf_kernel::kernel::Fabric;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use asf_kernel::{tools, trace};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{stdin, stdout, BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

// @1.1: adds note.list — dogfooding found workflow 3 dead on arrival
// without enumeration (can't summarize or reorganize what you can't see).
// @1.2: adds note.edit — whole-document rewrite was the only mutation
// (DF-P2 finding), which is error-prone for link fixing and bloats the
// payload store with full copies per touch.
pub const VAULT_TOOL_REF: &str = "tool:vault@1.2";
pub const WORKBOARD_TOOL_REF: &str = "tool:workboard@1.0";

fn vault_tool_actions() -> Value {
    json!([
        { "name": "note.list",  "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "files.vault",
          "class": "read", "store": "fs:vault", "path_args": ["path"] },
        { "name": "note.read",  "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "files.vault",
          "class": "read", "store": "fs:vault", "path_args": ["path"] },
        { "name": "note.write", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "files.vault",
          "class": "write", "store": "fs:vault", "path_args": ["path"] },
        { "name": "note.edit",  "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "files.vault",
          "class": "write", "store": "fs:vault", "path_args": ["path"] },
        { "name": "note.move",  "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "files.vault",
          "class": "move", "store": "fs:vault", "path_args": ["src", "dest"] },
        { "name": "note.delete","side_effect": "local", "surface": "fixed",
          "reversibility": "irreversible", "domain": "files.vault",
          "class": "delete", "store": "fs:vault", "path_args": ["path"] },
    ])
}

/// The zero-authorship session grant (§0): derived entirely from registered
/// metadata — every local fixed-surface action whose declared reversibility
/// is compensable-or-better, path-scoped to the store, write budget metered
/// and escalatable, approvals at local_session. No user authored any of it.
fn vault_session_caveats() -> (Vec<Value>, Vec<&'static str>) {
    (
        vec![
            json!({"dim":"action.allow","tools":[VAULT_TOOL_REF],
                   "actions":["note.list","note.read","note.write","note.edit","note.move"]}),
            json!({"dim":"reversibility.max","max":"compensable"}),
            json!({"dim":"paths.write","globs":["**"]}),
            json!({"dim":"budget.count","action_class":"write","max":20,"window":"run"}),
            json!({"dim":"approval.min_auth","min":"local_session"}),
        ],
        vec!["budget.count:write"],
    )
}

fn workboard_tool_actions() -> Value {
    json!([
        { "name": "work.list", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.tracking",
          "class": "read", "store": "db:workboard", "path_args": [] },
        { "name": "work.get", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.tracking",
          "class": "read", "store": "db:workboard", "path_args": [] },
        { "name": "work.create", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.tracking",
          "class": "write", "store": "db:workboard", "path_args": [] },
        { "name": "work.update", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.tracking",
          "class": "write", "store": "db:workboard", "path_args": [] },
        { "name": "work.add_dependency", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.tracking",
          "class": "write", "store": "db:workboard", "path_args": [] },
        { "name": "work.link_evidence", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.tracking",
          "class": "write", "store": "db:workboard", "path_args": [] },
        { "name": "work.close", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.tracking",
          "class": "write", "store": "db:workboard", "path_args": [] },
        { "name": "evidence.list", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.evidence",
          "class": "read", "store": "fs:evidence", "path_args": [] },
        { "name": "evidence.read", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.evidence",
          "class": "read", "store": "fs:evidence", "path_args": ["path"] },
        { "name": "evidence.create", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.evidence",
          "class": "write", "store": "fs:evidence", "path_args": ["path"] },
        { "name": "evidence.edit", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "work.evidence",
          "class": "write", "store": "fs:evidence", "path_args": ["path"] },
    ])
}

fn workboard_session_caveats() -> (Vec<Value>, Vec<&'static str>) {
    (
        vec![
            json!({"dim":"action.allow","tools":[WORKBOARD_TOOL_REF],
                   "actions":["work.list","work.get","work.create","work.update",
                              "work.add_dependency","work.link_evidence","work.close",
                              "evidence.list","evidence.read","evidence.create","evidence.edit"]}),
            json!({"dim":"reversibility.max","max":"compensable"}),
            json!({"dim":"budget.count","action_class":"write","max":20,"window":"run"}),
            json!({"dim":"approval.min_auth","min":"local_session"}),
        ],
        vec!["budget.count:write"],
    )
}

/// Trusted, compile-time dogfooding profiles. A downstream process cannot
/// provide its own tool registration, store topology, or default grant.
#[derive(Clone, Debug)]
pub enum Profile {
    Vault { vault: PathBuf },
    Workboard { db: PathBuf, evidence: PathBuf },
}

impl Profile {
    pub fn vault(vault: PathBuf) -> Self {
        Self::Vault { vault }
    }

    pub fn workboard(db: PathBuf, evidence: PathBuf) -> Self {
        Self::Workboard { db, evidence }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Vault { .. } => "vault",
            Self::Workboard { .. } => "workboard",
        }
    }

    fn tool_ref(&self) -> &'static str {
        match self {
            Self::Vault { .. } => VAULT_TOOL_REF,
            Self::Workboard { .. } => WORKBOARD_TOOL_REF,
        }
    }

    fn actions(&self) -> Value {
        match self {
            Self::Vault { .. } => vault_tool_actions(),
            Self::Workboard { .. } => workboard_tool_actions(),
        }
    }

    fn caveats(&self) -> (Vec<Value>, Vec<&'static str>) {
        match self {
            Self::Vault { .. } => vault_session_caveats(),
            Self::Workboard { .. } => workboard_session_caveats(),
        }
    }

    fn intent(&self) -> &'static str {
        match self {
            Self::Vault { .. } => "vault maintenance via brokered MCP session",
            Self::Workboard { .. } => "local project tracking via brokered workboard session",
        }
    }

    fn prepare_stores(&self, home: &Path) -> Result<Vec<StoreSpec>> {
        match self {
            Self::Vault { vault } => {
                let memory_db = home.join("memory.db");
                if !memory_db.exists() {
                    Connection::open(&memory_db)?
                        .execute_batch("CREATE TABLE memories (id INTEGER PRIMARY KEY, fact TEXT);")?;
                }
                Ok(vec![
                    StoreSpec { store: "fs:vault".into(), tier: 1, kind: StoreKind::Fs, path: vault.clone() },
                    StoreSpec { store: "db:memory".into(), tier: 1, kind: StoreKind::Sqlite, path: memory_db },
                ])
            }
            Self::Workboard { db, evidence } => {
                workboard_server::init(db, evidence)?;
                Ok(vec![
                    StoreSpec { store: "db:workboard".into(), tier: 1, kind: StoreKind::Sqlite, path: db.clone() },
                    StoreSpec { store: "fs:evidence".into(), tier: 1, kind: StoreKind::Fs, path: evidence.clone() },
                ])
            }
        }
    }

    fn assert_compatible_home(&self, home: &Path) -> Result<()> {
        let db = home.join("fabric/fabric.db");
        if !db.exists() {
            return Ok(());
        }
        let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let raw_stores = trace::meta_get(&conn, "stores")?
            .context("existing fabric home has no recorded store topology")?;
        let stores: Value = serde_json::from_str(&raw_stores)
            .context("existing fabric home has malformed store topology")?;
        let stored: std::collections::BTreeMap<String, PathBuf> = stores
            .as_array()
            .context("existing fabric store topology is not an array")?
            .iter()
            .map(|store| {
                let name = store["store"].as_str().context("stored root has no name")?;
                let path = store["path"].as_str().context("stored root has no path")?;
                Ok((name.to_string(), PathBuf::from(path)))
            })
            .collect::<Result<_>>()?;
        let requested: std::collections::BTreeMap<String, PathBuf> = match self {
            Self::Vault { vault } => [
                ("fs:vault".to_string(), vault.clone()),
                ("db:memory".to_string(), home.join("memory.db")),
            ].into_iter().collect(),
            Self::Workboard { db, evidence } => [
                ("db:workboard".to_string(), db.clone()),
                ("fs:evidence".to_string(), evidence.clone()),
            ].into_iter().collect(),
        };
        if stored != requested {
            bail!(
                "fabric home {} is pinned to a different store topology; use the original roots or a separate --home",
                home.display(),
            );
        }
        let existing = trace::meta_get(&conn, "dogfood_profile")?.or_else(|| {
            if stored.contains_key("db:workboard") || stored.contains_key("fs:evidence") {
                Some("workboard".to_string())
            } else if stored.contains_key("fs:vault") {
                Some("vault".to_string())
            } else {
                None
            }
        });
        if existing.as_deref().is_some_and(|name| name != self.name()) {
            bail!(
                "fabric home {} belongs to the {} profile, not {}; use a separate --home",
                home.display(),
                existing.unwrap(),
                self.name(),
            );
        }
        Ok(())
    }

    fn rewrites(&self, branch: &std::collections::BTreeMap<String, PathBuf>) -> Vec<(String, String)> {
        match self {
            Self::Vault { vault } => branch.get("fs:vault").map(|target| vec![(
                vault.to_string_lossy().into_owned(), target.to_string_lossy().into_owned(),
            )]).unwrap_or_default(),
            Self::Workboard { db, evidence } => [
                ("db:workboard", db),
                ("fs:evidence", evidence),
            ].into_iter().filter_map(|(store, source)| branch.get(store).map(|target| (
                source.to_string_lossy().into_owned(), target.to_string_lossy().into_owned(),
            ))).collect(),
        }
    }
}

pub struct Session {
    pub broker: Arc<Mutex<Broker>>,
    pub cap: String,
    pub channel: String,
    pub manifest: String,
    pub tool_ref: &'static str,
    /// store → branch path. The agent's whole session runs on this fork;
    /// trunk only changes at promotion (brief §3 principle 2).
    pub branch: std::collections::BTreeMap<String, std::path::PathBuf>,
}

/// Meta key marking a session whose branch has not faced the gate yet.
/// Written at bootstrap, cleared when the promotion gate runs (any
/// outcome except error). A key left behind is a stranded session (RF-9):
/// the client killed the daemon before EOF, or promotion errored — either
/// way the next bootstrap (or `asf recover`) picks it up.
fn live_key(manifest: &str) -> String {
    format!("session_live:{manifest}")
}

/// Is `pid` a live asf process? Guards recovery against reaping the branch
/// of a concurrently running proxy on the same home. The args check makes
/// PID reuse by an unrelated process read as dead, which is the direction
/// we want to fail in.
fn pid_is_live_asf(pid: i64) -> bool {
    let Ok(out) = Command::new("ps").args(["-p", &pid.to_string(), "-o", "args="]).output() else {
        return false;
    };
    out.status.success() && String::from_utf8_lossy(&out.stdout).contains("asf")
}

/// Run one stranded session through the promotion gate and report on
/// stderr. Clears the live-marker on Applied/Parked; leaves it on error so
/// the next recovery pass retries.
fn gate_stranded(broker: &mut Broker, manifest: &str, branch: &std::collections::BTreeMap<String, PathBuf>) -> Result<()> {
    match broker.promote_manifest(manifest, branch) {
        Ok(asf_kernel::broker::PromotionOutcome::Applied { event }) => {
            eprintln!("asfd: recovered stranded session {manifest} — promoted to trunk ({event})");
            trace::meta_del(&broker.fabric.conn, &live_key(manifest))?;
        }
        Ok(asf_kernel::broker::PromotionOutcome::Parked { promotion }) => {
            eprintln!("asfd: recovered stranded session {manifest} — parked as promotion #{promotion}");
            trace::meta_del(&broker.fabric.conn, &live_key(manifest))?;
        }
        Err(e) => eprintln!("asfd: stranded session {manifest} could not be gated: {e} (will retry next start)"),
    }
    Ok(())
}

/// RF-9 recovery: any session whose live-marker survived (its proxy is not
/// running anymore) is gated now, before the new session snapshots trunk.
/// Crash-safe by construction — covers SIGKILL, power loss, and clients
/// that kill the daemon instead of closing stdin.
pub fn recover_stranded_sessions(broker: &mut Broker) -> Result<()> {
    for (key, value) in trace::meta_scan(&broker.fabric.conn, "session_live:")? {
        let manifest = key.trim_start_matches("session_live:").to_string();
        let info: Value = serde_json::from_str(&value).unwrap_or(json!({}));
        if info["pid"].as_i64().is_some_and(pid_is_live_asf) {
            eprintln!("asfd: session {manifest} still live (pid {}) — not recovering", info["pid"]);
            continue;
        }
        let branch: std::collections::BTreeMap<String, PathBuf> = info["branch"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| v.as_str().map(|p| (k.clone(), PathBuf::from(p))))
            .collect();
        if branch.is_empty() || branch.values().any(|p| !p.exists()) {
            eprintln!("asfd: stranded session {manifest} has no intact branch on disk — dropping marker");
            trace::meta_del(&broker.fabric.conn, &live_key(&manifest))?;
            continue;
        }
        gate_stranded(broker, &manifest, &branch)?;
    }
    Ok(())
}

/// The single end-of-session path (EOF, signal, recovery all converge
/// here): run the gate once, clear the live-marker. The marker doubles as
/// the run-once guard — EOF and a signal racing each other is harmless.
fn finish_session(broker: &Arc<Mutex<Broker>>, manifest: &str, branch: &std::collections::BTreeMap<String, PathBuf>) {
    let mut b = broker.lock().expect("broker lock");
    match trace::meta_get(&b.fabric.conn, &live_key(manifest)) {
        Ok(Some(_)) => {}
        _ => return, // gate already ran (or marker unreadable — recovery will decide)
    }
    let _ = gate_stranded(&mut b, manifest, branch);
}

/// Open (bootstrapping on first run) the fabric home and start a session:
/// step-boundary manifest + fresh session capability (M6: cheap, frequent).
pub fn bootstrap(home: &Path, profile: &Profile) -> Result<Session> {
    std::fs::create_dir_all(home)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700))?;
    }
    profile.assert_compatible_home(home)?;
    let stores = profile.prepare_stores(home)?;
    let fabric = Fabric::open(home.join("fabric"), stores)?;
    let mut broker = Broker::new(fabric)?;

    match trace::meta_get(&broker.fabric.conn, "dogfood_profile")? {
        Some(existing) if existing != profile.name() => bail!(
            "fabric home {} belongs to the {existing} profile, not {}; use a separate --home",
            home.display(),
            profile.name(),
        ),
        Some(_) => {}
        None => trace::meta_set(&broker.fabric.conn, "dogfood_profile", profile.name())?,
    }

    // Gate any session a previous proxy left stranded BEFORE this session
    // snapshots trunk, so today's manifest starts from recovered state.
    recover_stranded_sessions(&mut broker)?;
    let fabric = &mut broker.fabric;

    let (human, agent, channel, intent) = match (
        trace::meta_get(&fabric.conn, "human")?,
        trace::meta_get(&fabric.conn, "agent")?,
        trace::meta_get(&fabric.conn, "channel")?,
        trace::meta_get(&fabric.conn, "intent")?,
    ) {
        (Some(h), Some(a), Some(c), Some(i)) => (h, a, c, i),
        _ => {
            let h = fabric.register_principal("human", "operator", "01", None)?;
            let a = fabric.register_principal("agent", "mcp-client", "02", Some(&h))?;
            let c = fabric.register_channel(&h, "local_session", b"asfd-stdio", "local_session")?;
            let i = fabric.capture_intent(
                &h, &c, "local_session",
                profile.intent(),
                json!({}), None,
            )?;
            for (k, v) in [("human", &h), ("agent", &a), ("channel", &c), ("intent", &i)] {
                trace::meta_set(&fabric.conn, k, v)?;
            }
            (h, a, c, i)
        }
    };

    let behavior = json!({ "bundle": "sha256:asfd-stage2", "skills": [] });
    let step = fabric.step_boundary(&human, &agent, &intent, behavior)?;
    for d in &step.drift {
        eprintln!("asfd: drift in {} attributed {} (ledger evt {})", d.store, d.attribution, d.event_id);
    }
    // Fork the session's working state: the downstream tool server gets
    // pointed at the branch; the live roots are trunk, mutated only by the
    // human and by promotion.
    let branch = fabric.create_branch(&step.manifest)?;

    // Mark the session live for RF-9 recovery: if this process dies before
    // the gate runs, the marker survives and the next start gates the
    // branch. The pid lets recovery skip concurrently live sessions.
    trace::meta_set(
        &fabric.conn,
        &live_key(&step.manifest),
        &serde_json::to_string(&json!({
            "pid": std::process::id(),
            "started": time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)?,
            "branch": branch.iter().map(|(k, v)| (k.clone(), v.to_string_lossy().into_owned()))
                .collect::<std::collections::BTreeMap<_, _>>(),
        }))?,
    )?;

    // Register the tool surface once per version: the meta value is the
    // registered ref, so a version bump (new/changed actions) re-registers
    // under the new ref while prior versions stay in the ledger.
    let tool_ref = profile.tool_ref();
    if trace::meta_get(&broker.fabric.conn, "tool_registered")?.as_deref() != Some(tool_ref) {
        broker.register_tool(tool_ref, profile.actions())?;
        trace::meta_set(&broker.fabric.conn, "tool_registered", tool_ref)?;
    }
    let (caveats, escalatable) = profile.caveats();
    let expires = (time::OffsetDateTime::now_utc() + time::Duration::hours(2))
        .format(&time::format_description::well_known::Rfc3339)?;
    let cap = broker.mint(&step.manifest, &agent, caveats, escalatable, &expires)?;
    eprintln!("asfd: session manifest {} cap {cap}", step.manifest);
    for (store, path) in &branch {
        eprintln!("asfd: branch {store} at {}", path.display());
    }

    Ok(Session {
        broker: Arc::new(Mutex::new(broker)),
        cap,
        channel,
        manifest: step.manifest,
        tool_ref,
        branch,
    })
}

/// The daemon-owned approval surface (C2). Line protocol over a Unix
/// socket: {"cmd":"list"} | {"cmd":"approve","id":N,"uses":N} |
/// {"cmd":"deny","id":N}. The connecting party is, by construction, a
/// local process of the operating user: the registered local_session
/// channel.
fn spawn_approval_surface(
    home: &Path,
    broker: Arc<Mutex<Broker>>,
    channel: String,
) -> Result<()> {
    let sock_path = home.join("approvals.sock");
    let _ = std::fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path)
        .with_context(|| format!("binding approval socket {}", sock_path.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600))?;
    }
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(match stream.try_clone() {
                Ok(s) => s,
                Err(_) => continue,
            });
            let mut stream = stream;
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let req: Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let mut b = broker.lock().expect("broker lock");
            let reply = dispatch_approval_cmd(&mut b, &req, &channel);
            drop(b);
            let _ = writeln!(stream, "{reply}");
        }
    });
    Ok(())
}

/// One command dispatcher for both C2 surfaces: the daemon socket and the
/// offline `asf approve` fallback. Everything here is local_session by
/// construction (same-machine, operating-user-only paths).
fn dispatch_approval_cmd(b: &mut Broker, req: &Value, channel: &str) -> Value {
    let id = req["id"].as_i64().unwrap_or(-1);
    match req["cmd"].as_str() {
        Some("list") => match b.list_escalations("pending") {
            Ok(v) => json!({ "ok": true, "escalations": v }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        },
        Some("approve") => {
            let uses = req["uses"].as_i64().unwrap_or(1);
            match b.approve_escalation(id, uses, channel, "local_session") {
                Ok(()) => json!({ "ok": true }),
                Err(e) => json!({ "ok": false, "error": e.to_string() }),
            }
        }
        Some("deny") => match b.deny_escalation(id, channel, "local_session") {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        },
        Some("promotions") => match b.list_promotions("pending") {
            Ok(v) => json!({ "ok": true, "promotions": v }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        },
        Some("promote") => match b.approve_promotion(id, channel, "local_session") {
            Ok(event) => json!({ "ok": true, "event": event }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        },
        Some("reject") => match b.reject_promotion(id, channel, "local_session") {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        },
        _ => json!({ "ok": false, "error": "unknown cmd" }),
    }
}

/// Should `action_name` be advertised to the agent at all?
///
/// Advertised = registered AND every *statically checkable* dimension either
/// passes or is escalatable. Flat-out unreachable actions are noise (and an
/// injection target); escalatable ones must stay visible — attempting them
/// is exactly how JIT elicitation starts (brief §5.3). Dynamic dimensions
/// (paths, budgets, time) never filter: they depend on the call.
fn advertisable(cap: &Value, broker: &Broker, tool_ref: &str, action_name: &str) -> bool {
    let Ok(reg) = tools::lookup_action(
        &broker.fabric.conn,
        &broker.fabric.fabric_vk(),
        tool_ref,
        action_name,
    ) else {
        return false; // undeclared actions cannot be called (§4) — or shown
    };
    let escalatable: Vec<&str> = cap["on_violation"]["escalatable"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let caveat = |dim: &str| -> Option<&Value> {
        cap["caveats"].as_array()?.iter().find(|c| c["dim"] == dim)
    };

    if let Some(allow) = caveat("action.allow") {
        let listed = allow["tools"].as_array().is_some_and(|t| t.iter().any(|x| x == tool_ref))
            && allow["actions"].as_array().is_some_and(|a| a.iter().any(|x| x == action_name));
        if !listed && !escalatable.contains(&"action.allow") {
            return false;
        }
    }
    if let Some(rev) = caveat("reversibility.max") {
        let max = rev["max"].as_str().and_then(reversibility_rank);
        let have = reg["reversibility"].as_str().and_then(reversibility_rank);
        let within = matches!((max, have), (Some(m), Some(h)) if h <= m);
        if !within && !escalatable.contains(&"reversibility.max") {
            return false;
        }
    }
    if let Some(reach) = caveat("external_reach") {
        let live_ok = reach["mode"].as_str().and_then(reach_rank) == reach_rank("live");
        if reg["side_effect"] == "external" && !live_ok && !escalatable.contains(&"external_reach") {
            return false;
        }
    }
    true
}

/// Filter a downstream tools/list result down to the session grant.
fn filter_tools_result(broker: &Broker, cap_id: &str, tool_ref: &str, mut resp: Value) -> Value {
    let Ok(cap) = trace::get_object(&broker.fabric.conn, cap_id) else {
        resp["result"]["tools"] = json!([]); // no readable grant → advertise nothing
        return resp;
    };
    if let Some(list) = resp["result"]["tools"].as_array() {
        let kept: Vec<Value> = list
            .iter()
            .filter(|t| {
                t["name"]
                    .as_str()
                    .is_some_and(|n| advertisable(&cap, broker, tool_ref, n))
            })
            .cloned()
            .collect();
        resp["result"]["tools"] = Value::Array(kept);
    }
    resp
}

pub fn run(home: PathBuf, profile: Profile, downstream: Vec<String>) -> Result<()> {
    let session = bootstrap(&home, &profile)?;
    spawn_approval_surface(&home, session.broker.clone(), session.channel.clone())?;

    // Real MCP clients rarely grant a graceful stdin EOF — Claude Code
    // kills the server on shutdown (RF-9). Catch the catchable signals and
    // run the gate before dying; SIGKILL is covered by bootstrap recovery.
    {
        let broker = session.broker.clone();
        let manifest = session.manifest.clone();
        let branch = session.branch.clone();
        let mut signals = signal_hook::iterator::Signals::new([
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGHUP,
        ])?;
        thread::spawn(move || {
            if signals.forever().next().is_some() {
                // Taking the broker lock serializes with result recording,
                // not with the downstream effect itself. If the result was
                // recorded, the gate sees its signed root. If the downstream
                // mutated first but has not returned, branch-tip verification
                // rejects the untraced root and the live marker remains for
                // explicit recovery; nothing silently reaches trunk.
                finish_session(&broker, &manifest, &branch);
                std::process::exit(0);
            }
        });
    }

    // Point the downstream tool server at the session branch: arguments
    // naming any live root are rewritten to their branch paths. The
    // downstream never learns where trunk lives.
    let rewrites = profile.rewrites(&session.branch);
    let downstream: Vec<String> = downstream
        .into_iter()
        .map(|arg| {
            rewrites.iter().find_map(|(live, branch)| (arg == *live).then(|| branch.clone()))
                .unwrap_or(arg)
        })
        .collect();

    let mut child: Child = Command::new(&downstream[0])
        .args(&downstream[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning downstream {downstream:?}"))?;
    let mut child_in = child.stdin.take().expect("child stdin");
    let child_out = child.stdout.take().expect("child stdout");

    // Responses to intercepted ids come back to the main loop; everything
    // else the downstream says goes straight to the agent.
    let intercepted: Arc<Mutex<HashMap<String, Sender<Value>>>> = Default::default();
    let out = Arc::new(Mutex::new(stdout()));
    {
        let intercepted = intercepted.clone();
        let out = out.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(child_out);
            while let Ok(Some(msg)) = mcp::read_msg(&mut reader) {
                let routed = mcp::is_response(&msg)
                    .then(|| mcp::id_key(&msg["id"]))
                    .and_then(|k| intercepted.lock().expect("map lock").remove(&k));
                match routed {
                    Some(tx) => {
                        let _ = tx.send(msg);
                    }
                    None => {
                        let mut o = out.lock().expect("stdout lock");
                        let _ = mcp::write_msg(&mut *o, &msg);
                    }
                }
            }
        });
    }

    let mut reader = BufReader::new(stdin());
    while let Some(msg) = mcp::read_msg(&mut reader)? {
        if mcp::is_request(&msg) && msg["method"] == "tools/list" {
            // Forward, but filter the response to the session grant: only
            // actions that are allowed or could escalate get advertised.
            let (tx, rx) = channel();
            intercepted
                .lock()
                .expect("map lock")
                .insert(mcp::id_key(&msg["id"]), tx);
            mcp::write_msg(&mut child_in, &msg)?;
            let resp = rx.recv().context("downstream vanished during tools/list")?;
            let filtered = {
                let b = session.broker.lock().expect("broker lock");
                filter_tools_result(&b, &session.cap, session.tool_ref, resp)
            };
            let mut o = out.lock().expect("stdout lock");
            mcp::write_msg(&mut *o, &filtered)?;
        } else if mcp::is_request(&msg) && msg["method"] == "tools/call" {
            let id = msg["id"].clone();
            let name = msg["params"]["name"].as_str().unwrap_or("").to_string();
            let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));

            let decision = {
                let mut b = session.broker.lock().expect("broker lock");
                b.propose_call(&session.cap, session.tool_ref, &name, &args)
            };
            let reply = match decision {
                Ok(Decision::Allowed { ticket, forwarded_args }) => {
                    let (tx, rx) = channel();
                    intercepted
                        .lock()
                        .expect("map lock")
                        .insert(mcp::id_key(&id), tx);
                    let mut fwd = msg.clone();
                    fwd["params"]["arguments"] = forwarded_args;
                    mcp::write_msg(&mut child_in, &fwd)?;
                    let resp = rx.recv().context("downstream vanished mid-call")?;
                    let result_bytes = serde_json::to_vec(resp.get("result").unwrap_or(&Value::Null))?;
                    session
                        .broker
                        .lock()
                        .expect("broker lock")
                        .record_result(ticket, &result_bytes)?;
                    resp
                }
                Ok(Decision::Denied { reasons, structural, event }) => mcp::tool_error(
                    &id,
                    &format!(
                        "ASF broker denied {name}: {} (ledger {event})",
                        structural.unwrap_or_else(|| reasons.join(", "))
                    ),
                ),
                Ok(Decision::Escalated { escalations }) => mcp::tool_error(
                    &id,
                    &format!(
                        "ASF broker parked {name} pending human approval (escalation {escalations:?}). \
                         Approval happens on the operator's own surface; you may mention it is pending, \
                         you cannot grant it."
                    ),
                ),
                Err(e) => mcp::tool_error(&id, &format!("ASF broker error: {e}")),
            };
            let mut o = out.lock().expect("stdout lock");
            mcp::write_msg(&mut *o, &reply)?;
        } else {
            mcp::write_msg(&mut child_in, &msg)?;
        }
    }

    // Session over: the branch faces the gate. §5.3 — trace verified
    // against capability, three-way merge, zero-authorship policy; clean
    // additive runs land, anything else parks for `asf approve`.
    finish_session(&session.broker, &session.manifest, &session.branch);

    let _ = child.kill();
    Ok(())
}

/// `asf recover` — gate sessions a dead proxy left stranded (RF-9). The
/// marker-based scan handles everything a post-fix proxy started; explicit
/// manifest ids handle legacy branches from before the markers existed
/// (the operator asserts those sessions are dead by naming them). Run this
/// with no live proxy on the home.
pub fn recover_cli(home: &Path, profile: &Profile, manifests: &[String]) -> Result<()> {
    profile.assert_compatible_home(home)?;
    let stores = profile.prepare_stores(home)?;
    let fabric = Fabric::open(home.join("fabric"), stores)?;
    let mut broker = Broker::new(fabric)?;
    recover_stranded_sessions(&mut broker)?;
    for manifest in manifests {
        // The gate is not idempotent: refuse manifests that already faced
        // it (a promotion event, or a parked/decided promotions row).
        let gated: bool = broker.fabric.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM events WHERE kind = 'promotion' AND manifest = ?1)
                 OR EXISTS (SELECT 1 FROM promotions WHERE manifest = ?1)",
            [manifest],
            |r| r.get(0),
        )?;
        if gated {
            eprintln!("asf recover: {manifest} already faced the gate — skipping");
            continue;
        }
        let short = manifest.strip_prefix("man:").unwrap_or(manifest);
        let base = home.join("fabric/branches").join(&short[..12.min(short.len())]);
        let branch: std::collections::BTreeMap<String, PathBuf> = broker
            .fabric
            .stores()
            .iter()
            .map(|s| (s.store.clone(), base.join(s.store.replace([':', '/'], "_"))))
            .filter(|(_, p)| p.exists())
            .collect();
        if branch.is_empty() {
            eprintln!("asf recover: no branch on disk for {manifest} (looked in {})", base.display());
            continue;
        }
        gate_stranded(&mut broker, manifest, &branch)?;
    }
    Ok(())
}

/// `asf approve` — the human side of the C2 surface. Talks to the daemon
/// socket when one is up; otherwise operates directly on the fabric home
/// (same machine, same operating user: still a local_session surface —
/// C2 forbids the *agent* carrying approvals, not offline operators).
pub fn approve_cli(home: &Path, cmd: &str, id: Option<i64>, uses: i64) -> Result<()> {
    use std::os::unix::net::UnixStream;
    let req = match cmd {
        "list" => json!({ "cmd": "list" }),
        "approve" => json!({ "cmd": "approve", "id": id, "uses": uses }),
        "deny" => json!({ "cmd": "deny", "id": id }),
        "promotions" => json!({ "cmd": "promotions" }),
        "promote" => json!({ "cmd": "promote", "id": id }),
        "reject" => json!({ "cmd": "reject", "id": id }),
        other => anyhow::bail!("unknown approve subcommand {other}"),
    };
    let sock = home.join("approvals.sock");
    if let Ok(mut stream) = UnixStream::connect(&sock) {
        writeln!(stream, "{req}")?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        println!("{}", line.trim());
        return Ok(());
    }
    // Offline: open the home directly.
    let fabric = Fabric::open_existing(home.join("fabric"))
        .with_context(|| format!("no daemon socket and no fabric home under {}", home.display()))?;
    let channel = trace::meta_get(&fabric.conn, "channel")?
        .unwrap_or_else(|| "chan:local".to_string());
    let mut broker = Broker::new(fabric)?;
    println!("{}", dispatch_approval_cmd(&mut broker, &req, &channel));
    Ok(())
}
