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

pub const TOOL_REF: &str = "tool:vault@1.0";

fn vault_tool_actions() -> Value {
    json!([
        { "name": "note.read",  "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "files.vault",
          "class": "read", "store": "fs:vault", "path_args": ["path"] },
        { "name": "note.write", "side_effect": "local", "surface": "fixed",
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
                   "actions":["note.read","note.write","note.move"]}),
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
}

/// Open (bootstrapping on first run) the fabric home and start a session:
/// step-boundary manifest + fresh session capability (M6: cheap, frequent).
pub fn bootstrap(home: &Path, vault: &Path) -> Result<Session> {
    std::fs::create_dir_all(home)?;
    let memory_db = home.join("memory.db");
    if !memory_db.exists() {
        Connection::open(&memory_db)?
            .execute_batch("CREATE TABLE memories (id INTEGER PRIMARY KEY, fact TEXT);")?;
    }
    let stores = vec![
        StoreSpec { store: "fs:vault".into(), tier: 1, kind: StoreKind::Fs, path: vault.to_path_buf() },
        StoreSpec { store: "db:memory".into(), tier: 1, kind: StoreKind::Sqlite, path: memory_db },
    ];
    let mut fabric = Fabric::open(home.join("fabric"), stores)?;

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

    let mut broker = Broker::new(fabric)?;
    if trace::meta_get(&broker.fabric.conn, "tool_registered")?.is_none() {
        broker.register_tool(TOOL_REF, vault_tool_actions())?;
        trace::meta_set(&broker.fabric.conn, "tool_registered", "1")?;
    }
    let (caveats, escalatable) = default_session_caveats();
    let expires = (time::OffsetDateTime::now_utc() + time::Duration::hours(2))
        .format(&time::format_description::well_known::Rfc3339)?;
    let cap = broker.mint(&step.manifest, &agent, caveats, escalatable, &expires)?;
    eprintln!("asfd: session manifest {} cap {cap}", step.manifest);

    Ok(Session {
        broker: Arc::new(Mutex::new(broker)),
        cap,
        channel,
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
            let reply = match req["cmd"].as_str() {
                Some("list") => match b.list_escalations("pending") {
                    Ok(v) => json!({ "ok": true, "escalations": v }),
                    Err(e) => json!({ "ok": false, "error": e.to_string() }),
                },
                Some("approve") => {
                    let id = req["id"].as_i64().unwrap_or(-1);
                    let uses = req["uses"].as_i64().unwrap_or(1);
                    match b.approve_escalation(id, uses, &channel, "local_session") {
                        Ok(()) => json!({ "ok": true }),
                        Err(e) => json!({ "ok": false, "error": e.to_string() }),
                    }
                }
                Some("deny") => {
                    let id = req["id"].as_i64().unwrap_or(-1);
                    match b.deny_escalation(id, &channel, "local_session") {
                        Ok(()) => json!({ "ok": true }),
                        Err(e) => json!({ "ok": false, "error": e.to_string() }),
                    }
                }
                _ => json!({ "ok": false, "error": "unknown cmd" }),
            };
            drop(b);
            let _ = writeln!(stream, "{reply}");
        }
    });
    Ok(())
}

/// Should `action_name` be advertised to the agent at all?
///
/// Advertised = registered AND every *statically checkable* dimension either
/// passes or is escalatable. Flat-out unreachable actions are noise (and an
/// injection target); escalatable ones must stay visible — attempting them
/// is exactly how JIT elicitation starts (brief §5.3). Dynamic dimensions
/// (paths, budgets, time) never filter: they depend on the call.
fn advertisable(cap: &Value, conn: &rusqlite::Connection, action_name: &str) -> bool {
    let Ok(reg) = tools::lookup_action(conn, TOOL_REF, action_name) else {
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
                    .is_some_and(|n| advertisable(&cap, &broker.fabric.conn, n))
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

    let _ = child.kill();
    Ok(())
}

/// `asf approve` — the human side of the C2 surface.
pub fn approve_cli(home: &Path, cmd: &str, id: Option<i64>, uses: i64) -> Result<()> {
    use std::os::unix::net::UnixStream;
    let sock = home.join("approvals.sock");
    let mut stream = UnixStream::connect(&sock)
        .with_context(|| format!("no daemon at {} — is asf proxy running?", sock.display()))?;
    let req = match cmd {
        "list" => json!({ "cmd": "list" }),
        "approve" => json!({ "cmd": "approve", "id": id, "uses": uses }),
        "deny" => json!({ "cmd": "deny", "id": id }),
        other => anyhow::bail!("unknown approve subcommand {other}"),
    };
    writeln!(stream, "{req}")?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    println!("{}", line.trim());
    Ok(())
}
