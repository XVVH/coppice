//! Milestone 2 invariant suite — broker, capabilities, approvals.
//!
//! Coverage map (spec = source of truth):
//! - M1 (roots cover capability reach): `m1_mint_rejects_uncovered_store`
//! - M2 (cap bound to manifest): `m2_capability_dies_with_its_manifest`
//! - §5 F1 broker-minted: `forged_capability_is_dead`
//! - §5.1 fail-closed unknown dims: `unknown_dimension_denies_at_broker`
//! - §5.2 attenuation: unit tests in capability.rs +
//!   `attenuation_through_broker`
//! - §4 conservative defaults: unit tests in tools.rs +
//!   `undeclared_action_uncallable`
//! - A9 batching + approvals + C1 stamping + approval.min_auth:
//!   `escalation_batch_approval_cycle`
//! - Credential custody (brief §5.3): `credentials_injected_never_traced`
//! - C2 (daemon-owned approvals):       structural — approvals exist only as
//!   broker API + daemon socket (see asf-cli); the MCP path has no approval
//!   method, verified in the proxy smoke test.
//! - C5 (sender binding), C3/C4: need real channels/Telegram — milestone 3.

use asf_kernel::broker::{Broker, BrokerError, Decision};
use asf_kernel::kernel::Fabric;
use asf_kernel::payload::PayloadRef;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use asf_kernel::trace;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

struct World {
    _tmp: tempfile::TempDir,
    vault: PathBuf,
    broker: Broker,
    human: String,
    agent: String,
    intent: String,
    manifest: String,
}

fn vault_actions() -> Value {
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

fn default_caveats() -> Vec<Value> {
    vec![
        json!({"dim":"action.allow","tools":["tool:vault@1.0"],
               "actions":["note.read","note.write","note.move"]}),
        json!({"dim":"reversibility.max","max":"compensable"}),
        json!({"dim":"paths.write","globs":["inbox/**","MOCs/**"]}),
        json!({"dim":"budget.count","action_class":"write","max":2,"window":"run"}),
        json!({"dim":"approval.min_auth","min":"local_session"}),
    ]
}

fn far_expiry() -> String {
    "2027-01-01T00:00:00Z".to_string()
}

fn setup() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    fs::create_dir_all(vault.join("inbox")).unwrap();
    fs::write(vault.join("inbox/todo.md"), "- tidy\n").unwrap();
    let memory_db = tmp.path().join("memory.db");
    Connection::open(&memory_db)
        .unwrap()
        .execute_batch("CREATE TABLE memories (id INTEGER PRIMARY KEY, fact TEXT);")
        .unwrap();

    let stores = vec![
        StoreSpec { store: "fs:vault".into(), tier: 1, kind: StoreKind::Fs, path: vault.clone() },
        StoreSpec { store: "db:memory".into(), tier: 1, kind: StoreKind::Sqlite, path: memory_db },
    ];
    let mut fabric = Fabric::initialize(tmp.path().join("fabric"), stores).unwrap();
    let human = fabric.register_principal("human", "josh", "01", None).unwrap();
    let agent = fabric.register_principal("agent", "hermes", "02", Some(&human)).unwrap();
    let chan = fabric
        .register_channel(&human, "local_session", b"tty", "local_session")
        .unwrap();
    let intent = fabric
        .capture_intent(&human, &chan, "local_session", "vault maintenance", json!({}), None)
        .unwrap();
    let behavior = json!({"bundle": "sha256:1111", "skills": []});
    let step = fabric.step_boundary(&human, &agent, &intent, behavior).unwrap();

    let mut broker = Broker::new(fabric).unwrap();
    broker.register_tool("tool:vault@1.0", vault_actions()).unwrap();
    World {
        _tmp: tmp,
        vault,
        broker,
        human,
        agent,
        intent,
        manifest: step.manifest,
    }
}

fn mint_default(w: &mut World) -> String {
    w.broker
        .mint(
            &w.manifest,
            &w.agent,
            default_caveats(),
            vec!["budget.count:write"],
            &far_expiry(),
        )
        .unwrap()
}

fn write_call(w: &mut World, cap: &str, path: &str) -> Decision {
    w.broker
        .propose_call(cap, "tool:vault@1.0", "note.write", &json!({"path": path, "content": "x"}))
        .unwrap()
}

#[test]
fn allowed_call_full_cycle_with_checks_in_ledger() {
    let mut w = setup();
    let cap = mint_default(&mut w);
    match write_call(&mut w, &cap, "inbox/a.md") {
        Decision::Allowed { ticket, forwarded_args } => {
            assert_eq!(forwarded_args["path"], "inbox/a.md");
            fs::write(w.vault.join("inbox/a.md"), "x").unwrap(); // execute
            let ev = w.broker.record_result(ticket, br#"{"ok":true}"#).unwrap();
            let rows = trace::events_in_span(&w.broker.fabric.conn, &ev.span).unwrap();
            let tc = rows.iter().find(|e| e.kind == "tool_call").unwrap();
            let checks = tc.raw["body"]["checks"].as_array().unwrap();
            assert_eq!(checks.len(), 5, "every caveat produced a check record");
            assert!(checks.iter().all(|c| c["ok"] == true));
        }
        other => panic!("expected Allowed, got {other:?}"),
    }
}

#[test]
fn m1_mint_rejects_uncovered_store() {
    let mut w = setup();
    // A tool operating on a store the manifest does not snapshot.
    w.broker
        .register_tool(
            "tool:books@1.0",
            json!([{ "name": "ledger.post", "side_effect": "local", "surface": "fixed",
                     "domain": "finance.books", "class": "write",
                     "store": "db:books", "path_args": [] }]),
        )
        .unwrap();
    let caveats = vec![json!({"dim":"action.allow","tools":["tool:books@1.0"],"actions":["ledger.post"]})];
    match w.broker.mint(&w.manifest, &w.agent, caveats, vec![], &far_expiry()) {
        Err(BrokerError::M1 { store, .. }) => assert_eq!(store, "db:books"),
        other => panic!("expected M1 violation, got {other:?}"),
    }
}

#[test]
fn m2_capability_dies_with_its_manifest() {
    let mut w = setup();
    let cap = mint_default(&mut w);
    // New step boundary → new manifest → the old capability is orphaned.
    let behavior = json!({"bundle": "sha256:1111", "skills": []});
    let step2 = {
        let f = &mut w.broker.fabric;
        f.step_boundary(&w.human, &w.agent, &w.intent, behavior).unwrap()
    };
    match write_call(&mut w, &cap, "inbox/a.md") {
        Decision::Denied { structural: Some(s), .. } => assert!(s.contains("M2"), "{s}"),
        other => panic!("expected structural M2 denial, got {other:?}"),
    }
    // Attenuation-identity onto the new manifest revives authority (M5 flow).
    let cap2 = w
        .broker
        .attenuate(&cap, &w.agent, &step2.manifest, default_caveats(),
                   vec!["budget.count:write"], &far_expiry())
        .unwrap();
    assert!(matches!(write_call(&mut w, &cap2, "inbox/a.md"), Decision::Allowed { .. }));
}

#[test]
fn forged_capability_is_dead() {
    let mut w = setup();
    let cap = mint_default(&mut w);
    // Tamper with the stored capability object: widen the budget.
    let mut obj = trace::get_object(&w.broker.fabric.conn, &cap).unwrap();
    obj["caveats"][3]["max"] = json!(10_000);
    w.broker
        .fabric
        .conn
        .execute(
            "UPDATE objects SET raw = ?2 WHERE id = ?1",
            rusqlite::params![cap, serde_json::to_string(&obj).unwrap()],
        )
        .unwrap();
    match write_call(&mut w, &cap, "inbox/a.md") {
        Decision::Denied { structural: Some(s), .. } => assert!(s.contains("F1"), "{s}"),
        other => panic!("expected F1 denial, got {other:?}"),
    }
}

#[test]
fn w14_mistyped_capability_cannot_dispatch_or_escalate() {
    let mut w = setup();
    let cap = mint_default(&mut w);
    w.broker
        .fabric
        .conn
        .execute(
            "UPDATE objects SET kind = 'manifest' WHERE id = ?1",
            [&cap],
        )
        .unwrap();

    let protected = w.vault.join("inbox/must-not-dispatch.md");
    let decision = write_call(&mut w, &cap, "inbox/must-not-dispatch.md");
    if let Decision::Allowed { .. } = decision {
        fs::write(&protected, "dispatched").unwrap();
    }
    match decision {
        Decision::Denied {
            structural: Some(reason),
            ..
        } => {
            assert!(reason.contains("RF-19"), "{reason}");
            assert!(reason.contains("stored kind"), "{reason}");
        }
        other => panic!("mistyped capability must be structurally denied: {other:?}"),
    }
    assert!(
        !protected.exists(),
        "a mistyped capability crossed the protected dispatch boundary"
    );
    assert!(w.broker.list_escalations("pending").unwrap().is_empty());
}

#[test]
fn w14_mistyped_capability_cannot_receive_approval_authority() {
    let mut w = setup();
    let cap = mint_default(&mut w);
    for path in ["inbox/a.md", "inbox/b.md"] {
        match write_call(&mut w, &cap, path) {
            Decision::Allowed { ticket, .. } => {
                w.broker.record_result(ticket, b"{}").unwrap();
            }
            other => panic!("expected budget-burning call to pass: {other:?}"),
        }
    }
    let escalation = match write_call(&mut w, &cap, "inbox/c.md") {
        Decision::Escalated { escalations } => escalations[0],
        other => panic!("expected escalation, got {other:?}"),
    };
    w.broker
        .fabric
        .conn
        .execute(
            "UPDATE objects SET kind = 'manifest' WHERE id = ?1",
            [&cap],
        )
        .unwrap();

    assert!(
        w.broker
            .approve_escalation(escalation, 1, "chan:tty", "local_session")
            .is_err(),
        "mistyped capability received approval authority"
    );
    assert_eq!(w.broker.list_escalations("pending").unwrap().len(), 1);
    let exemptions: i64 = w
        .broker
        .fabric
        .conn
        .query_row("SELECT COUNT(*) FROM exemptions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(exemptions, 0);
    let events = trace::all_events(&w.broker.fabric.conn).unwrap();
    assert_eq!(
        events.iter().filter(|event| event.kind == "approval").count(),
        0,
        "failed approval still appended an authority event"
    );
}

#[test]
fn undeclared_action_uncallable() {
    let mut w = setup();
    let cap = mint_default(&mut w);
    let d = w
        .broker
        .propose_call(&cap, "tool:vault@1.0", "note.nuke", &json!({"path": "inbox/a.md"}))
        .unwrap();
    assert!(matches!(d, Decision::Denied { structural: Some(ref s), .. } if s.contains("undeclared")));
}

#[test]
fn unknown_dimension_denies_at_broker() {
    let mut w = setup();
    let mut caveats = default_caveats();
    caveats.push(json!({"dim":"taint.egress","urls_and_bodies_derived_only_from":["intent"]}));
    // taint.egress is a real §5.1 dimension the Stage 2 evaluator does not
    // implement — precisely the fail-closed case.
    let cap = w
        .broker
        .mint(&w.manifest, &w.agent, caveats, vec![], &far_expiry())
        .unwrap();
    let d = write_call(&mut w, &cap, "inbox/a.md");
    match d {
        Decision::Denied { reasons, .. } => assert!(reasons.contains(&"taint.egress".to_string())),
        other => panic!("expected fail-closed denial, got {other:?}"),
    }
}

#[test]
fn out_of_scope_paths_and_reversibility_denied() {
    let mut w = setup();
    let cap = mint_default(&mut w);
    // Path outside globs.
    assert!(matches!(
        write_call(&mut w, &cap, "secrets/key.md"),
        Decision::Denied { ref reasons, .. } if reasons.contains(&"paths.write".to_string())
    ));
    // note.delete: blocked by action.allow AND would exceed reversibility.max.
    let d = w
        .broker
        .propose_call(&cap, "tool:vault@1.0", "note.delete", &json!({"path": "inbox/a.md"}))
        .unwrap();
    match d {
        Decision::Denied { reasons, .. } => {
            assert!(reasons.contains(&"action.allow".to_string()));
            assert!(reasons.contains(&"reversibility.max".to_string()));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn exemption_survives_a_denied_call() {
    // RF-2 regression: an approved exemption must NOT be consumed by a call
    // that is ultimately denied for an unrelated (non-escalatable) reason.
    let mut w = setup();
    let cap = mint_default(&mut w);

    // Burn the write budget (max 2), then escalate + approve 1 exemption.
    for p in ["inbox/a.md", "inbox/b.md"] {
        match write_call(&mut w, &cap, p) {
            Decision::Allowed { ticket, .. } => { w.broker.record_result(ticket, b"{}").unwrap(); }
            other => panic!("{other:?}"),
        }
    }
    let esc = match write_call(&mut w, &cap, "inbox/c.md") {
        Decision::Escalated { escalations } => escalations[0],
        other => panic!("expected escalation, got {other:?}"),
    };
    w.broker.approve_escalation(esc, 1, "chan:tty", "local_session").unwrap();

    // A call that is BOTH over budget (escalatable, exemption applies) AND
    // out-of-scope path (hard, non-escalatable deny). It must be denied —
    // and must NOT spend the exemption.
    match write_call(&mut w, &cap, "secrets/leak.md") {
        Decision::Denied { reasons, .. } => {
            assert!(reasons.contains(&"paths.write".to_string()), "{reasons:?}");
        }
        other => panic!("expected denial, got {other:?}"),
    }

    // The exemption is intact: a legitimate over-budget write now passes.
    // (Before the RF-2 fix, the denied call above burned it and this parks.)
    match write_call(&mut w, &cap, "inbox/d.md") {
        Decision::Allowed { ticket, .. } => { w.broker.record_result(ticket, b"{}").unwrap(); }
        other => panic!("exemption was wrongly consumed by the denied call: {other:?}"),
    }

    // And it was a bounded grant of 1: the next over-budget write parks again.
    assert!(matches!(
        write_call(&mut w, &cap, "inbox/e.md"),
        Decision::Escalated { .. }
    ));
}

#[test]
fn escalation_batch_approval_cycle() {
    let mut w = setup();
    let cap = mint_default(&mut w);
    // Burn the write budget (max 2).
    for p in ["inbox/a.md", "inbox/b.md"] {
        match write_call(&mut w, &cap, p) {
            Decision::Allowed { ticket, .. } => {
                w.broker.record_result(ticket, b"{}").unwrap();
            }
            other => panic!("{other:?}"),
        }
    }
    // Next three writes: all park in ONE batch (A9).
    let mut esc_id = None;
    for p in ["inbox/c.md", "inbox/d.md", "inbox/e.md"] {
        match write_call(&mut w, &cap, p) {
            Decision::Escalated { escalations } => {
                assert_eq!(escalations.len(), 1);
                if let Some(prev) = esc_id {
                    assert_eq!(prev, escalations[0], "batched, not re-pinged");
                }
                esc_id = Some(escalations[0]);
            }
            other => panic!("{other:?}"),
        }
    }
    let esc = esc_id.unwrap();
    let pending = w.broker.list_escalations("pending").unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["count"], 3, "one legible batch of three");

    // approval.min_auth = local_session: a weaker channel may not approve (C1).
    match w.broker.approve_escalation(esc, 2, "chan:tg", "platform_oauth") {
        Err(BrokerError::ChannelTooWeak { .. }) => {}
        other => panic!("expected ChannelTooWeak, got {other:?}"),
    }

    // Strong channel approves 2 uses; the approval event is C1-stamped.
    w.broker.approve_escalation(esc, 2, "chan:tty", "local_session").unwrap();
    let events = trace::all_events(&w.broker.fabric.conn).unwrap();
    let appr = events.iter().find(|e| e.kind == "approval").unwrap();
    assert_eq!(appr.raw["body"]["channel"], "chan:tty");
    assert_eq!(appr.raw["body"]["auth_strength"], "local_session");
    assert_eq!(appr.raw["body"]["escalation"], esc);
    assert_eq!(appr.raw["body"]["capability"], cap);
    assert_eq!(appr.raw["body"]["caveat"], "budget.count:write");

    // Two exempted writes pass (checks show the exemption)…
    for p in ["inbox/c.md", "inbox/d.md"] {
        match write_call(&mut w, &cap, p) {
            Decision::Allowed { ticket, .. } => {
                w.broker.record_result(ticket, b"{}").unwrap();
            }
            other => panic!("{other:?}"),
        }
    }
    // …and the third parks again: approvals are bounded, not blank cheques.
    assert!(matches!(write_call(&mut w, &cap, "inbox/e.md"), Decision::Escalated { .. }));
}

#[test]
fn credentials_injected_never_traced() {
    let mut w = setup();
    w.broker
        .register_tool(
            "tool:web@1.0",
            json!([{ "name": "web.fetch", "side_effect": "external", "surface": "open",
                     "egress": true, "reversibility": "reversible", "domain": "web.research",
                     "class": "read", "store": "fs:vault", "path_args": [],
                     "credentials": { "arg": "api_key", "secret": "web_api_key" } }]),
        )
        .unwrap();
    w.broker.fabric.keystore().secret_set("web_api_key", "s3cr3t-t0ken").unwrap();

    let caveats = vec![
        json!({"dim":"action.allow","tools":["tool:web@1.0"],"actions":["web.fetch"]}),
        json!({"dim":"external_reach","mode":"live"}),
    ];
    let cap = w.broker.mint(&w.manifest, &w.agent, caveats, vec![], &far_expiry()).unwrap();

    let agent_args = json!({"url": "https://example.com", "api_key": "$injected"});
    match w.broker.propose_call(&cap, "tool:web@1.0", "web.fetch", &agent_args).unwrap() {
        Decision::Allowed { ticket, forwarded_args } => {
            // The downstream copy holds the real secret…
            assert_eq!(forwarded_args["api_key"], "s3cr3t-t0ken");
            let ev = w.broker.record_result(ticket, b"{\"status\":200}").unwrap();
            // …the ledger copy never does.
            let rows = trace::events_in_span(&w.broker.fabric.conn, &ev.span).unwrap();
            let tc = rows.iter().find(|e| e.kind == "tool_call").unwrap();
            let args_ref: PayloadRef =
                serde_json::from_value(tc.raw["body"]["args"].clone()).unwrap();
            let stored = w.broker.fabric.get_payload(&args_ref).unwrap();
            let stored_str = String::from_utf8(stored).unwrap();
            assert!(!stored_str.contains("s3cr3t-t0ken"), "secret leaked into ledger");
            assert!(stored_str.contains("$injected"), "agent-supplied args preserved");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn attenuation_through_broker() {
    let mut w = setup();
    let cap = mint_default(&mut w);

    // Narrower child: inbox only, 1 write, read/write only.
    let child_caveats = vec![
        json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.read","note.write"]}),
        json!({"dim":"reversibility.max","max":"reversible"}),
        json!({"dim":"paths.write","globs":["inbox/**"]}),
        json!({"dim":"budget.count","action_class":"write","max":1,"window":"run"}),
        json!({"dim":"approval.min_auth","min":"local_session"}),
    ];
    let child = w
        .broker
        .attenuate(&cap, &w.agent, &w.manifest, child_caveats, vec![], &far_expiry())
        .unwrap();
    assert!(matches!(write_call(&mut w, &child, "inbox/x.md"), Decision::Allowed { .. }));
    assert!(matches!(
        write_call(&mut w, &child, "MOCs/x.md"),
        Decision::Denied { .. }
    ), "parent allowed MOCs, child must not");

    // Widening attempt: fatal at attenuation time, no capability minted.
    let mut wide = default_caveats();
    wide[2] = json!({"dim":"paths.write","globs":["**"]});
    assert!(matches!(
        w.broker.attenuate(&cap, &w.agent, &w.manifest, wide, vec![], &far_expiry()),
        Err(BrokerError::Cap(_))
    ));
}

#[test]
fn attenuation_rechecks_m1_for_the_child_manifest() {
    let mut w = setup();
    w.broker
        .register_tool(
            "tool:memory@1.0",
            json!([{
                "name": "memory.write",
                "side_effect": "local",
                "surface": "fixed",
                "reversibility": "reversible",
                "domain": "memory.local",
                "class": "write",
                "store": "db:memory",
                "path_args": []
            }]),
        )
        .unwrap();
    let caveats = vec![json!({
        "dim": "action.allow",
        "tools": ["tool:memory@1.0"],
        "actions": ["memory.write"]
    })];
    let parent = w
        .broker
        .mint(
            &w.manifest,
            &w.agent,
            caveats.clone(),
            vec![],
            &far_expiry(),
        )
        .unwrap();

    // Mint a valid fabric-signed child manifest that deliberately omits the
    // memory store. A child capability retaining memory.write must be rejected
    // by M1 even though its parent covered that store.
    let home = w._tmp.path().join("fabric");
    let vault_only = w
        .broker
        .fabric
        .stores()
        .iter()
        .find(|s| s.store == "fs:vault")
        .unwrap()
        .clone();
    let mut subset = Fabric::open_existing_with_stores(&home, vec![vault_only]).unwrap();
    let child_manifest = subset
        .step_boundary(
            &w.human,
            &w.agent,
            &w.intent,
            json!({"bundle":"sha256:child","skills":[]}),
        )
        .unwrap()
        .manifest;
    drop(subset);

    assert!(matches!(
        w.broker.attenuate(
            &parent,
            &w.agent,
            &child_manifest,
            caveats,
            vec![],
            &far_expiry(),
        ),
        Err(BrokerError::M1 { ref store, .. }) if store == "db:memory"
    ));
}

#[test]
fn expired_capability_denies_structurally() {
    let mut w = setup();
    let cap = w
        .broker
        .mint(&w.manifest, &w.agent, default_caveats(), vec![], "2026-01-01T00:00:00Z")
        .unwrap();
    match write_call(&mut w, &cap, "inbox/a.md") {
        Decision::Denied { structural: Some(s), .. } => assert!(s.contains("expired")),
        other => panic!("{other:?}"),
    }
}

// ---- A22/§5.4 capability closure: decision-time surface ------------------
//
// The gate-side matrix lives in tests/gate.rs; here: closure denies
// structurally before any caveat, escalation, or exemption is consulted,
// mint/attenuation refuse closed authority, and the revocation API keeps
// its own provenance rules.

fn count_events(w: &World, kind: &str) -> usize {
    trace::all_events(&w.broker.fabric.conn)
        .unwrap()
        .iter()
        .filter(|e| e.kind == kind)
        .count()
}

#[test]
fn a22_revoked_capability_denies_structurally_and_never_escalates() {
    let mut w = setup();
    let cap = mint_default(&mut w);
    // Live before closure (also the positive half of the pair).
    match write_call(&mut w, &cap, "inbox/before.md") {
        Decision::Allowed { ticket, .. } => {
            fs::write(w.vault.join("inbox/before.md"), "x").unwrap();
            w.broker.record_result(ticket, b"{}").unwrap();
        }
        other => panic!("expected Allowed before revoke: {other:?}"),
    }

    let ev = w
        .broker
        .revoke_capability(&cap, "compromise", Some(("chan:test", "local_session")))
        .unwrap();
    assert!(ev.starts_with("evt:"), "revocation is a signed event");

    // Closed: structural denial, never escalation — an escalatable closure
    // would be an un-revoke lever inside the agent's loop (§5.4).
    match write_call(&mut w, &cap, "inbox/after.md") {
        Decision::Denied { structural: Some(s), .. } => {
            assert!(s.contains("revoked"), "{s}");
        }
        other => panic!("closed capability must deny structurally: {other:?}"),
    }
    assert_eq!(
        w.broker.list_escalations("pending").unwrap().len(),
        0,
        "closure denial must not enqueue an escalation"
    );
    // The revoke event carries C1 provenance.
    let events = trace::all_events(&w.broker.fabric.conn).unwrap();
    let rv = events.iter().find(|e| e.kind == "revoke").unwrap();
    assert_eq!(rv.raw["body"]["channel"], "chan:test");
    assert_eq!(rv.raw["body"]["auth_strength"], "local_session");
    assert_eq!(rv.raw["body"]["reason"], "compromise");
}

#[test]
fn a22_signed_revoke_cannot_be_concealed_by_unsigned_indexes() {
    let mut w = setup();
    let cap = mint_default(&mut w);
    let revoke = w
        .broker
        .revoke_capability(&cap, "compromise", None)
        .unwrap();

    // Reproduce RF-16 exactly: keep the fabric-signed raw revoke intact but
    // make its materialized selectors call it a grant on another span. A
    // database writer has no signing key. The verified-event boundary must
    // fail closed before the broker returns a dispatch ticket.
    w.broker
        .fabric
        .conn
        .execute_batch("DROP TRIGGER events_append_only_u")
        .unwrap();
    w.broker
        .fabric
        .conn
        .execute(
            "UPDATE events SET kind = 'grant', span = 'span:concealed' WHERE id = ?1",
            [&revoke],
        )
        .unwrap();

    let protected = w.vault.join("inbox/must-not-run.md");
    let decision = write_call(&mut w, &cap, "inbox/must-not-run.md");
    if matches!(&decision, Decision::Allowed { .. }) {
        // Model the downstream dispatch edge: an Allowed verdict is exactly
        // what would make the protected mutation occur.
        fs::write(&protected, "dispatched").unwrap();
    }
    assert!(
        !protected.exists(),
        "the protected effect must not occur when signed/index state disagrees"
    );
    match decision {
        Decision::Denied { structural: Some(reason), .. } => assert!(
            reason.contains("index column") && reason.contains("fail closed"),
            "{reason}"
        ),
        other => panic!("concealed signed revoke must deny structurally: {other:?}"),
    }
    assert!(
        w.broker.list_escalations("pending").unwrap().is_empty(),
        "structural substrate failure must not become an escalation path"
    );
}

#[test]
fn w11_each_signed_event_selector_mismatch_prevents_dispatch() {
    let cases = [
        ("id", "id = 'evt:index-tamper'"),
        ("span", "span = 'span:index-tamper'"),
        ("seq", "seq = 99"),
        ("prev", "prev = 'evt:index-tamper'"),
        ("manifest", "manifest = 'man:index-tamper'"),
        ("at", "at = 't-index-tamper'"),
        ("kind", "kind = 'grant'"),
    ];

    for (field, assignment) in cases {
        let mut w = setup();
        let cap = mint_default(&mut w);
        let revoke = w
            .broker
            .revoke_capability(&cap, "compromise", None)
            .unwrap();
        w.broker
            .fabric
            .conn
            .execute_batch("DROP TRIGGER events_append_only_u")
            .unwrap();
        w.broker
            .fabric
            .conn
            .execute(
                &format!("UPDATE events SET {assignment} WHERE id = ?1"),
                [&revoke],
            )
            .unwrap();

        let protected = w.vault.join(format!("inbox/{field}-must-not-run.md"));
        let decision = write_call(
            &mut w,
            &cap,
            &format!("inbox/{field}-must-not-run.md"),
        );
        if matches!(&decision, Decision::Allowed { .. }) {
            fs::write(&protected, "dispatched").unwrap();
        }
        assert!(
            !protected.exists(),
            "{field} mismatch permitted the protected dispatch"
        );
        match decision {
            Decision::Denied { structural: Some(reason), .. } => {
                assert!(
                    reason.contains("index column") && reason.contains(field),
                    "{field}: {reason}"
                );
            }
            other => panic!("{field} mismatch must deny structurally: {other:?}"),
        }
        assert!(
            w.broker.list_escalations("pending").unwrap().is_empty(),
            "{field} mismatch must not create an escalation path"
        );
    }
}

#[test]
fn a22_child_revoke_preserves_parent_and_siblings() {
    let mut w = setup();
    let parent = mint_default(&mut w);
    let narrower = vec![
        json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.write"]}),
        json!({"dim":"reversibility.max","max":"reversible"}),
        json!({"dim":"paths.write","globs":["inbox/**"]}),
        json!({"dim":"budget.count","action_class":"write","max":2,"window":"run"}),
        json!({"dim":"approval.min_auth","min":"local_session"}),
    ];
    let child1 = w
        .broker
        .attenuate(&parent, &w.agent, &w.manifest, narrower.clone(), vec![], &far_expiry())
        .unwrap();
    let child2 = w
        .broker
        .attenuate(&parent, &w.agent, &w.manifest, narrower, vec![], &far_expiry())
        .unwrap();
    assert_ne!(child1, child2, "distinct issued_at ⇒ distinct ids");

    w.broker
        .revoke_capability(&child1, "operator_request", Some(("chan:test", "local_session")))
        .unwrap();

    // Revoking a child leaves its parent and siblings live (§5.4).
    match write_call(&mut w, &child1, "inbox/c1.md") {
        Decision::Denied { structural: Some(s), .. } => assert!(s.contains("revoked"), "{s}"),
        other => panic!("revoked child must deny: {other:?}"),
    }
    assert!(matches!(
        write_call(&mut w, &parent, "inbox/p.md"),
        Decision::Allowed { .. }
    ));
    assert!(matches!(
        write_call(&mut w, &child2, "inbox/c2.md"),
        Decision::Allowed { .. }
    ));
}

#[test]
fn a22_exemptions_cannot_resurrect_closed_capability() {
    let mut w = setup();
    let cap = w
        .broker
        .mint(
            &w.manifest,
            &w.agent,
            vec![
                json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.write"]}),
                json!({"dim":"paths.write","globs":["inbox/**"]}),
                json!({"dim":"budget.count","action_class":"write","max":1,"window":"run"}),
                json!({"dim":"approval.min_auth","min":"local_session"}),
            ],
            vec!["budget.count:write"],
            &far_expiry(),
        )
        .unwrap();
    // Exhaust the budget, escalate, and approve headroom — all pre-revoke.
    match write_call(&mut w, &cap, "inbox/one.md") {
        Decision::Allowed { ticket, .. } => {
            fs::write(w.vault.join("inbox/one.md"), "x").unwrap();
            w.broker.record_result(ticket, b"{}").unwrap();
        }
        other => panic!("{other:?}"),
    }
    let esc = match write_call(&mut w, &cap, "inbox/two.md") {
        Decision::Escalated { escalations } => escalations[0],
        other => panic!("expected escalation: {other:?}"),
    };
    w.broker
        .approve_escalation(esc, 3, "chan:test", "local_session")
        .unwrap();

    w.broker
        .revoke_capability(&cap, "behavior_change", None)
        .unwrap();

    // The signed approval headroom is now visible-but-inert: closure is
    // checked before exemptions are consulted, and the unused exemption
    // is not consumed by the denial (nothing to destructively delete).
    match write_call(&mut w, &cap, "inbox/three.md") {
        Decision::Denied { structural: Some(s), .. } => assert!(s.contains("revoked"), "{s}"),
        other => panic!("exemption must not resurrect closed authority: {other:?}"),
    }
    let remaining: i64 = w
        .broker
        .fabric
        .conn
        .query_row(
            "SELECT remaining FROM exemptions WHERE cap = ?1",
            [cap.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 3, "denial consumed no exemption headroom");
}

#[test]
fn a22_post_revoke_attenuation_is_refused() {
    let mut w = setup();
    let parent = mint_default(&mut w);
    w.broker
        .revoke_capability(&parent, "compromise", Some(("chan:test", "local_session")))
        .unwrap();
    let grants_before = count_events(&w, "grant");

    let out = w.broker.attenuate(
        &parent,
        &w.agent,
        &w.manifest,
        default_caveats(),
        vec![],
        &far_expiry(),
    );
    match out {
        Err(BrokerError::CapabilityClosed(msg)) => assert!(msg.contains("revoked"), "{msg}"),
        other => panic!("closed parent must not produce a child: {other:?}"),
    }
    assert_eq!(
        count_events(&w, "grant"),
        grants_before,
        "no grant event was emitted for a child of a closed parent"
    );
}

#[test]
fn a22_broker_refuses_to_grant_a_closed_id() {
    let mut w = setup();
    // The timestamp-collision seam: identical body at an identical instant
    // reproduces the same content id. Restoration must be a NEW mint, and
    // a colliding re-mint must fail loudly, never silently issue a dead
    // token (§5.4).
    let issued = "2026-07-12T00:00:00Z";
    let cap = w
        .broker
        .mint_at(&w.manifest, &w.agent, default_caveats(), vec![], &far_expiry(), issued)
        .unwrap();
    w.broker
        .revoke_capability(&cap, "compromise", Some(("chan:test", "local_session")))
        .unwrap();
    let grants_before = count_events(&w, "grant");

    match w.broker.mint_at(
        &w.manifest,
        &w.agent,
        default_caveats(),
        vec![],
        &far_expiry(),
        issued,
    ) {
        Err(BrokerError::CapabilityClosed(msg)) => {
            assert!(msg.contains("permanent"), "{msg}");
        }
        other => panic!("granting a closed id must be refused: {other:?}"),
    }
    assert_eq!(count_events(&w, "grant"), grants_before, "no re-grant landed");

    // A genuinely new mint (different issued_at ⇒ different id) restores.
    let restored = w
        .broker
        .mint_at(
            &w.manifest,
            &w.agent,
            default_caveats(),
            vec![],
            &far_expiry(),
            "2026-07-12T00:00:01Z",
        )
        .unwrap();
    assert_ne!(restored, cap);
    assert!(matches!(
        write_call(&mut w, &restored, "inbox/again.md"),
        Decision::Allowed { .. }
    ));
}

#[test]
fn a22_revocation_api_refuses_bad_targets_and_bad_provenance() {
    let mut w = setup();
    let cap = mint_default(&mut w);

    // Unknown target: a typo'd kill fails loudly instead of poisoning an id.
    assert!(matches!(
        w.broker
            .revoke_capability("cap:doesnotexist", "compromise", Some(("chan:test", "local_session"))),
        Err(BrokerError::InvalidRevocation(_))
    ));
    // Broker-mechanical closure must name a mechanical cause (§5.4).
    assert!(matches!(
        w.broker.revoke_capability(&cap, "operator_request", None),
        Err(BrokerError::InvalidRevocation(_))
    ));
    // Neither refusal emitted a closure event, and the target stays live.
    assert_eq!(count_events(&w, "revoke"), 0, "refusals close nothing");
    assert!(matches!(
        write_call(&mut w, &cap, "inbox/live.md"),
        Decision::Allowed { .. }
    ));
}
