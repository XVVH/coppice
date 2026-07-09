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
    let mut fabric = Fabric::open(tmp.path().join("fabric"), stores).unwrap();
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
