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

use crate::mcp;
use anyhow::{Context, Result};
use asf_kernel::broker::{Broker, Decision};
use asf_kernel::capability::{reach_rank, reversibility_rank};
use asf_kernel::kernel::Fabric;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use asf_kernel::{tools, trace};
use rusqlite::Connection;
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
pub const TOOL_REF: &str = "tool:vault@1.2";

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
fn default_session_caveats() -> (Vec<Value>, Vec<&'static str>) {
    (
        vec![
            json!({"dim":"action.allow","tools":[TOOL_REF],
                   "actions":["note.list","note.read","note.write","note.edit","note.move"]}),
            json!({"dim":"reversibility.max","max":"compensable"}),
            json!({"dim":"paths.write","globs":["**"]}),
            json!({"dim":"budget.count","action_class":"write","max":20,"window":"run"}),
            json!({"dim":"approval.min_auth","min":"local_session"}),
        ],
        vec!["budget.count:write"],
    )
}

pub struct Session {
    pub broker: Arc<Mutex<Broker>>,
    pub cap: String,
    pub channel: String,
    pub manifest: String,
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
pub fn bootstrap(home: &Path, vault: &Path) -> Result<Session> {
    std::fs::create_dir_all(home)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700))?;
    }
    let memory_db = home.join("memory.db");
    if !memory_db.exists() {
        Connection::open(&memory_db)?
            .execute_batch("CREATE TABLE memories (id INTEGER PRIMARY KEY, fact TEXT);")?;
    }
    let stores = vec![
        StoreSpec { store: "fs:vault".into(), tier: 1, kind: StoreKind::Fs, path: vault.to_path_buf() },
        StoreSpec { store: "db:memory".into(), tier: 1, kind: StoreKind::Sqlite, path: memory_db },
    ];
    let fabric = Fabric::open(home.join("fabric"), stores)?;
    let mut broker = Broker::new(fabric)?;

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
                "vault maintenance via brokered MCP session",
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
    // pointed at the branch; the live vault is trunk, mutated only by the
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
    if trace::meta_get(&broker.fabric.conn, "tool_registered")?.as_deref() != Some(TOOL_REF) {
        broker.register_tool(TOOL_REF, vault_tool_actions())?;
        trace::meta_set(&broker.fabric.conn, "tool_registered", TOOL_REF)?;
    }
    let (caveats, escalatable) = default_session_caveats();
    let expires = (time::OffsetDateTime::now_utc() + time::Duration::hours(2))
        .format(&time::format_description::well_known::Rfc3339)?;
    let cap = broker.mint(&step.manifest, &agent, caveats, escalatable, &expires)?;
    eprintln!("asfd: session manifest {} cap {cap}", step.manifest);
    eprintln!(
        "asfd: branch at {}",
        branch.get("fs:vault").map(|p| p.display().to_string()).unwrap_or_default()
    );

    Ok(Session {
        broker: Arc::new(Mutex::new(broker)),
        cap,
        channel,
        manifest: step.manifest,
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
fn advertisable(cap: &Value, broker: &Broker, action_name: &str) -> bool {
    let Ok(reg) = tools::lookup_action(
        &broker.fabric.conn,
        &broker.fabric.fabric_vk(),
        TOOL_REF,
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
        let listed = allow["tools"].as_array().is_some_and(|t| t.iter().any(|x| x == TOOL_REF))
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
fn filter_tools_result(broker: &Broker, cap_id: &str, mut resp: Value) -> Value {
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
                    .is_some_and(|n| advertisable(&cap, broker, n))
            })
            .cloned()
            .collect();
        resp["result"]["tools"] = Value::Array(kept);
    }
    resp
}

pub fn run(home: PathBuf, vault: PathBuf, downstream: Vec<String>) -> Result<()> {
    let session = bootstrap(&home, &vault)?;
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

    // Point the downstream tool server at the session branch: any argument
    // naming the live vault path is rewritten to the branch path. The
    // downstream never learns where trunk lives.
    let branch_vault = session.branch.get("fs:vault").cloned();
    let downstream: Vec<String> = downstream
        .into_iter()
        .map(|arg| {
            match (&branch_vault, arg == vault.to_string_lossy()) {
                (Some(b), true) => b.to_string_lossy().into_owned(),
                _ => arg,
            }
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
                filter_tools_result(&b, &session.cap, resp)
            };
            let mut o = out.lock().expect("stdout lock");
            mcp::write_msg(&mut *o, &filtered)?;
        } else if mcp::is_request(&msg) && msg["method"] == "tools/call" {
            let id = msg["id"].clone();
            let name = msg["params"]["name"].as_str().unwrap_or("").to_string();
            let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));

            let decision = {
                let mut b = session.broker.lock().expect("broker lock");
                b.propose_call(&session.cap, TOOL_REF, &name, &args)
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
pub fn recover_cli(home: &Path, vault: &Path, manifests: &[String]) -> Result<()> {
    let memory_db = home.join("memory.db");
    let stores = vec![
        StoreSpec { store: "fs:vault".into(), tier: 1, kind: StoreKind::Fs, path: vault.to_path_buf() },
        StoreSpec { store: "db:memory".into(), tier: 1, kind: StoreKind::Sqlite, path: memory_db },
    ];
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
