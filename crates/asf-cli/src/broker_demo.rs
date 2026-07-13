//! `asf broker-demo` — narrated milestone 2 round-trip, in-process.
//! The process-level equivalent (real MCP proxy + socket approvals) is
//! `asf proxy` / `asf approve`; this demo makes the decision pipeline
//! legible in one terminal.

use anyhow::{bail, Result};
use asf_kernel::broker::{Broker, Decision};
use asf_kernel::kernel::{AuthorityMode, Fabric};
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use asf_kernel::trace;
use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::path::Path;

fn banner(s: &str) {
    println!("\n━━━ {s} ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
}

pub fn run(dir: &Path) -> Result<()> {
    banner("setup: vault, memory, fabric, principals, intent, manifest");
    let vault = dir.join("vault");
    fs::create_dir_all(vault.join("inbox"))?;
    fs::write(vault.join("inbox/todo.md"), "- tidy\n")?;
    let memory_db = dir.join("memory.db");
    Connection::open(&memory_db)?
        .execute_batch("CREATE TABLE IF NOT EXISTS memories (id INTEGER PRIMARY KEY, fact TEXT);")?;
    let stores = vec![
        StoreSpec { store: "fs:vault".into(), tier: 1, kind: StoreKind::Fs, path: vault.clone() },
        StoreSpec { store: "db:memory".into(), tier: 1, kind: StoreKind::Sqlite, path: memory_db },
    ];
    let mut fabric = Fabric::initialize(dir.join("fabric"), stores)?;
    let human = fabric.register_principal("human", "josh", "01", None)?;
    let agent = fabric.register_principal("agent", "hermes", "02", Some(&human))?;
    let chan = fabric.register_channel(&human, "local_session", b"tty", "local_session")?;
    let intent = fabric.capture_intent(&human, &chan, "local_session",
        "tidy the vault; touch nothing outside inbox and MOCs", json!({}), None)?;
    let step = fabric.step_boundary_with_mode(
        &human,
        &agent,
        &intent,
        json!({"bundle":"sha256:demo2","skills":[]}),
        AuthorityMode::Brokered,
    )?;
    println!("manifest {} (authority mode: brokered)", step.manifest);

    let mut broker = Broker::new(fabric)?;
    broker.register_tool("tool:vault@1.0", json!([
        { "name": "note.read",  "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "files.vault",
          "class": "read", "store": "fs:vault", "path_args": ["path"] },
        { "name": "note.write", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "files.vault",
          "class": "write", "store": "fs:vault", "path_args": ["path"] },
        { "name": "note.delete","side_effect": "local", "surface": "fixed",
          "reversibility": "irreversible", "domain": "files.vault",
          "class": "delete", "store": "fs:vault", "path_args": ["path"] },
    ]))?;

    banner("broker mints the session capability (F1) — reads like a virtual card");
    println!("  read/write only · inbox/** + MOCs/** · 2 writes this run · nothing worse");
    println!("  than compensable · approvals need local_session · expires in 2h");
    let expires = (time::OffsetDateTime::now_utc() + time::Duration::hours(2))
        .format(&time::format_description::well_known::Rfc3339)?;
    let cap = broker.mint(&step.manifest, &agent, vec![
        json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.read","note.write"]}),
        json!({"dim":"reversibility.max","max":"compensable"}),
        json!({"dim":"paths.write","globs":["inbox/**","MOCs/**"]}),
        json!({"dim":"budget.count","action_class":"write","max":2,"window":"run"}),
        json!({"dim":"approval.min_auth","min":"local_session"}),
    ], vec!["budget.count:write"], &expires)?;
    println!("cap {cap}");

    let write = |b: &mut Broker, path: &str| -> Result<Decision> {
        Ok(b.propose_call(&cap, "tool:vault@1.0", "note.write",
            &json!({"path": path, "content": "x"}))?)
    };

    banner("in-scope writes pass, every caveat checked and traced");
    for p in ["inbox/a.md", "MOCs/b.md"] {
        match write(&mut broker, p)? {
            Decision::Allowed { ticket, .. } => {
                fs::create_dir_all(vault.join(Path::new(p).parent().unwrap()))?;
                fs::write(vault.join(p), "x")?;
                broker.record_result(ticket, b"{\"ok\":true}")?;
                println!("ALLOW  note.write {p}");
            }
            other => bail!("expected allow for {p}: {other:?}"),
        }
    }

    banner("out of bounds: wrong path / undeclared danger — denied, ledger-visible");
    match write(&mut broker, "secrets/key.md")? {
        Decision::Denied { reasons, event, .. } =>
            println!("DENY   note.write secrets/key.md — {} ({event})", reasons.join(", ")),
        other => bail!("{other:?}"),
    }
    match broker.propose_call(&cap, "tool:vault@1.0", "note.delete", &json!({"path":"inbox/a.md"}))? {
        Decision::Denied { reasons, .. } =>
            println!("DENY   note.delete inbox/a.md — {}", reasons.join(", ")),
        other => bail!("{other:?}"),
    }

    banner("budget exhausted: three violations, ONE batch escalation (A9)");
    let mut esc = -1;
    for p in ["inbox/c.md", "inbox/d.md", "inbox/e.md"] {
        match write(&mut broker, p)? {
            Decision::Escalated { escalations } => {
                esc = escalations[0];
                println!("PARK   note.write {p} → escalation #{esc}");
            }
            other => bail!("{other:?}"),
        }
    }
    let pending = broker.list_escalations("pending")?;
    println!("pending batches: {} (count {})", pending.len(), pending[0]["count"]);

    banner("approval on the broker's own surface (C2), channel-stamped (C1)");
    match broker.approve_escalation(esc, 1, "chan:weak", "platform_oauth") {
        Err(e) => println!("weak channel rejected: {e}"),
        Ok(_) => bail!("weak channel must not approve"),
    }
    broker.approve_escalation(esc, 2, &chan, "local_session")?;
    println!("approved 2 uses via local_session");
    for p in ["inbox/c.md", "inbox/d.md"] {
        match write(&mut broker, p)? {
            Decision::Allowed { ticket, .. } => {
                fs::write(vault.join(p), "x")?;
                broker.record_result(ticket, b"{}")?;
                println!("ALLOW  note.write {p} (approved exemption)");
            }
            other => bail!("{other:?}"),
        }
    }
    match write(&mut broker, "inbox/e.md")? {
        Decision::Escalated { .. } => println!("PARK   note.write inbox/e.md — approvals bound, not blank"),
        other => bail!("{other:?}"),
    }

    banner("attenuation: sub-agent gets a strict subset or nothing (§5.2)");
    let child = broker.attenuate(&cap, &agent, &step.manifest, vec![
        json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.read"]}),
        json!({"dim":"reversibility.max","max":"reversible"}),
        json!({"dim":"paths.write","globs":["inbox/**"]}),
        json!({"dim":"budget.count","action_class":"write","max":0,"window":"run"}),
        json!({"dim":"approval.min_auth","min":"local_session"}),
    ], vec![], &expires)?;
    println!("child cap {child} (read-only, inbox-only)");
    let widen = broker.attenuate(&cap, &agent, &step.manifest, vec![
        json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.read","note.write","note.delete"]}),
        json!({"dim":"reversibility.max","max":"irreversible"}),
        json!({"dim":"paths.write","globs":["**"]}),
        json!({"dim":"budget.count","action_class":"write","max":999,"window":"run"}),
        json!({"dim":"approval.min_auth","min":"local_session"}),
    ], vec![], &expires);
    println!("widening attempt: {}", widen.err().map(|e| e.to_string()).unwrap_or("ACCEPTED?!".into()));

    banner("fail closed: a caveat dimension this broker doesn't know");
    let alien = broker.mint(&step.manifest, &agent, vec![
        json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.write"]}),
        json!({"dim":"taint.egress","urls_and_bodies_derived_only_from":["intent"]}),
    ], vec![], &expires)?;
    match broker.propose_call(&alien, "tool:vault@1.0", "note.write", &json!({"path":"inbox/z.md"}))? {
        Decision::Denied { reasons, .. } =>
            println!("DENY   under unknown dim: {}", reasons.join(", ")),
        other => bail!("{other:?}"),
    }

    banner("verify + ledger");
    for (span, n) in broker.fabric.verify_all_spans()? {
        println!("span {}… — {} event(s) OK", &span[5..17], n);
    }
    let explanation = broker.fabric.explain()?;
    println!();
    for l in &explanation.lines {
        println!("  [{:>3}] {:<11} {}", l.offset, l.kind, l.line);
    }
    if !explanation.unexplained.is_empty() {
        bail!("UNEXPLAINED: {:?}", explanation.unexplained);
    }
    let events = trace::all_events(&broker.fabric.conn)?;
    for kind in ["grant", "tool_call", "verdict", "escalation", "approval"] {
        if !events.iter().any(|e| e.kind == kind) {
            bail!("ledger missing {kind} events");
        }
    }
    println!("\nbroker round-trip complete: grants, checks, denials, batches, approvals — all in the ledger.");
    Ok(())
}
