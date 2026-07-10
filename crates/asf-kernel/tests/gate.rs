//! Promotion-gate invariant suite. Spec §5.3 (A11, A13).
//!
//! Coverage map:
//! - Three-way merge, trunk-wins conflicts: unit tests in promote.rs +
//!   `concurrent_human_and_agent_edits_merge` here (over real stores)
//! - Rename detection / mass-rename-not-mass-delete: unit tests in
//!   promote.rs + `reorganization_parks_but_renders_as_moves`
//! - Trace-vs-capability at the gate: `gate_blocks_run_that_exceeded_its_token`
//! - Promoted changes remain revertible: `promotion_survives_revert`
//! - Zero-authorship policy + park/approve on the C2 surface:
//!   `destructive_ops_park_then_apply_on_approval`
//! - Coherent multi-store promotion (sqlite whole-store rule, SI-18):
//!   `sqlite_branch_change_promotes_whole_store`
//! - M7 authority mode (A21): brokered fails closed on unattributed effects
//!   (`m7_brokered_manifest_rejects_unattributed_calls`), the honest
//!   brokered path gates clean with grant-before-effect ordering
//!   (`m7_brokered_end_to_end_gates_clean`), and observed mode stays
//!   lenient (`m7_observed_manifest_tolerates_unattributed_calls`).

use asf_kernel::broker::{Broker, BrokerError, Decision, PromotionOutcome};
use asf_kernel::kernel::{AuthorityMode, Fabric};
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use asf_kernel::trace;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

struct World {
    _tmp: tempfile::TempDir,
    vault: PathBuf,
    memory_db: PathBuf,
    broker: Broker,
    agent: String,
    manifest: String,
    branch: BTreeMap<String, PathBuf>,
    cap: String,
    chan: String,
}

fn vault_actions() -> Value {
    json!([
        { "name": "note.write", "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "files.vault",
          "class": "write", "store": "fs:vault", "path_args": ["path"] },
        { "name": "note.move",  "side_effect": "local", "surface": "fixed",
          "reversibility": "reversible", "domain": "files.vault",
          "class": "move", "store": "fs:vault", "path_args": ["src", "dest"] },
        { "name": "note.delete", "side_effect": "local", "surface": "fixed",
          "reversibility": "irreversible", "domain": "files.vault",
          "class": "delete", "store": "fs:vault", "path_args": ["path"] },
    ])
}

fn caveats() -> Vec<Value> {
    vec![
        json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.write","note.move","note.delete"]}),
        json!({"dim":"paths.write","globs":["**"]}),
        json!({"dim":"budget.count","action_class":"write","max":10,"window":"run"}),
        json!({"dim":"approval.min_auth","min":"local_session"}),
    ]
}

fn setup() -> World {
    setup_mode(AuthorityMode::Observed)
}

fn setup_brokered() -> World {
    setup_mode(AuthorityMode::Brokered)
}

fn setup_mode(mode: AuthorityMode) -> World {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    fs::create_dir_all(vault.join("inbox")).unwrap();
    fs::write(vault.join("inbox/a.md"), "base a\n").unwrap();
    fs::write(vault.join("inbox/b.md"), "base b\n").unwrap();
    fs::write(vault.join("shared.md"), "base shared\n").unwrap();
    let memory_db = tmp.path().join("memory.db");
    Connection::open(&memory_db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE memories (id INTEGER PRIMARY KEY, fact TEXT);
             INSERT INTO memories (fact) VALUES ('base');",
        )
        .unwrap();

    let stores = vec![
        StoreSpec { store: "fs:vault".into(), tier: 1, kind: StoreKind::Fs, path: vault.clone() },
        StoreSpec { store: "db:memory".into(), tier: 1, kind: StoreKind::Sqlite, path: memory_db.clone() },
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
    let step = fabric
        .step_boundary_with_mode(&human, &agent, &intent, json!({"bundle":"sha256:g","skills":[]}), mode)
        .unwrap();
    let branch = fabric.create_branch(&step.manifest).unwrap();

    let mut broker = Broker::new(fabric).unwrap();
    broker.register_tool("tool:vault@1.0", vault_actions()).unwrap();
    let cap = broker
        .mint(&step.manifest, &agent, caveats(), vec![], "2027-01-01T00:00:00Z")
        .unwrap();
    World {
        _tmp: tmp,
        vault,
        memory_db,
        broker,
        agent,
        manifest: step.manifest,
        branch,
        cap,
        chan,
    }
}

/// Agent writes to its branch through the broker (allowed + recorded).
fn agent_write(w: &mut World, rel: &str, content: &str) {
    let d = w
        .broker
        .propose_call(&w.cap, "tool:vault@1.0", "note.write",
                      &json!({"path": rel, "content": content}))
        .unwrap();
    match d {
        Decision::Allowed { ticket, .. } => {
            let p = w.branch["fs:vault"].join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
            w.broker.record_result(ticket, b"{}").unwrap();
        }
        other => panic!("expected allow: {other:?}"),
    }
}

fn agent_delete(w: &mut World, rel: &str) {
    let d = w
        .broker
        .propose_call(
            &w.cap,
            "tool:vault@1.0",
            "note.delete",
            &json!({"path": rel}),
        )
        .unwrap();
    match d {
        Decision::Allowed { ticket, .. } => {
            fs::remove_file(w.branch["fs:vault"].join(rel)).unwrap();
            w.broker.record_result(ticket, b"{}").unwrap();
        }
        other => panic!("expected allow: {other:?}"),
    }
}

#[test]
fn concurrent_human_and_agent_edits_merge() {
    let mut w = setup();
    // Agent (on branch): edits a.md, adds new.md.
    agent_write(&mut w, "inbox/a.md", "agent edit\n");
    agent_write(&mut w, "new.md", "agent addition\n");
    // Human (on trunk, mid-session): edits b.md AND a.md (conflict).
    fs::write(w.vault.join("inbox/b.md"), "human edit\n").unwrap();
    fs::write(w.vault.join("inbox/a.md"), "human edit of a\n").unwrap();

    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        // A conflict exists → policy must park, even though other ops are adds.
        PromotionOutcome::Parked { promotion } => {
            let pending = w.broker.list_promotions("pending").unwrap();
            assert_eq!(pending[0]["id"], promotion);
            let conflicts = pending[0]["preview"]["conflicts"].as_array().unwrap();
            assert_eq!(conflicts.len(), 1);
            assert_eq!(conflicts[0]["path"], "inbox/a.md");
            assert_eq!(conflicts[0]["resolution"], "trunk_wins");

            w.broker.approve_promotion(promotion, &w.chan, "local_session").unwrap();
        }
        other => panic!("expected park on conflict: {other:?}"),
    }

    // Merged trunk: human's conflict version won, human's b.md stands,
    // agent's non-conflicting addition landed.
    assert_eq!(fs::read_to_string(w.vault.join("inbox/a.md")).unwrap(), "human edit of a\n");
    assert_eq!(fs::read_to_string(w.vault.join("inbox/b.md")).unwrap(), "human edit\n");
    assert_eq!(fs::read_to_string(w.vault.join("new.md")).unwrap(), "agent addition\n");

    // The ledger shows the promotion and explains all live roots.
    let expl = w.broker.fabric.explain().unwrap();
    assert!(expl.unexplained.is_empty(), "{:?}", expl.unexplained);
    let events = trace::all_events(&w.broker.fabric.conn).unwrap();
    assert!(events.iter().any(|e| e.kind == "promotion"));
}

#[test]
fn clean_additive_run_auto_promotes() {
    let mut w = setup();
    agent_write(&mut w, "inbox/c.md", "new note\n");
    agent_write(&mut w, "inbox/a.md", "improved a\n");
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Applied { .. } => {}
        other => panic!("adds+modifies with no conflicts must auto-promote: {other:?}"),
    }
    assert_eq!(fs::read_to_string(w.vault.join("inbox/c.md")).unwrap(), "new note\n");
    assert_eq!(fs::read_to_string(w.vault.join("inbox/a.md")).unwrap(), "improved a\n");
    assert!(w.broker.fabric.check_drift().unwrap().is_empty(), "promotion re-baselined expectations");
}

#[test]
fn gate_rejects_branch_state_with_no_signed_tool_call_attestation() {
    let mut w = setup();
    fs::write(w.branch["fs:vault"].join("untraced.md"), "not recorded\n").unwrap();
    let branch = w.branch.clone();

    match w.broker.promote_manifest(&w.manifest, &branch) {
        Err(BrokerError::GateTraceViolation(msg)) => {
            assert!(msg.contains("untraced branch divergence"), "{msg}");
        }
        other => panic!("untraced branch state must not face policy or merge: {other:?}"),
    }
    assert!(!w.vault.join("untraced.md").exists());
}

#[test]
fn gate_uses_signed_approval_binding_not_mutable_escalation_rows() {
    let mut w = setup();
    let mut tight = caveats();
    *tight
        .iter_mut()
        .find(|c| c["dim"] == "budget.count")
        .unwrap() = json!({
        "dim": "budget.count",
        "action_class": "write",
        "max": 1,
        "window": "run"
    });
    w.cap = w
        .broker
        .mint(
            &w.manifest,
            &w.agent,
            tight,
            vec!["budget.count:write"],
            "2027-01-01T00:00:00Z",
        )
        .unwrap();

    agent_write(&mut w, "inbox/first.md", "first\n");
    let decision = w
        .broker
        .propose_call(
            &w.cap,
            "tool:vault@1.0",
            "note.write",
            &json!({"path":"inbox/second.md","content":"second\n"}),
        )
        .unwrap();
    let escalation = match decision {
        Decision::Escalated { escalations } => escalations[0],
        other => panic!("expected budget escalation: {other:?}"),
    };
    w.broker
        .approve_escalation(escalation, 1, &w.chan, "local_session")
        .unwrap();
    agent_write(&mut w, "inbox/second.md", "second\n");

    // Supporting tables are mutable materialized state. Changing one must not
    // change the authority represented by the signed approval event.
    w.broker
        .fabric
        .conn
        .execute(
            "UPDATE escalations SET cap = 'cap:bogus', key = 'budget.count:other' WHERE id = ?1",
            [escalation],
        )
        .unwrap();

    let branch = w.branch.clone();
    assert!(matches!(
        w.broker.promote_manifest(&w.manifest, &branch),
        Ok(PromotionOutcome::Applied { .. })
    ));
}

#[test]
fn reorganization_parks_but_renders_as_moves() {
    let mut w = setup();
    // Simulate the agent reorganizing on its branch via broker-approved moves.
    for (src, dest) in [("inbox/a.md", "notes/a.md"), ("inbox/b.md", "notes/b.md")] {
        let d = w
            .broker
            .propose_call(&w.cap, "tool:vault@1.0", "note.move",
                          &json!({"src": src, "dest": dest}))
            .unwrap();
        match d {
            Decision::Allowed { ticket, .. } => {
                let (s, t) = (w.branch["fs:vault"].join(src), w.branch["fs:vault"].join(dest));
                fs::create_dir_all(t.parent().unwrap()).unwrap();
                fs::rename(s, t).unwrap();
                w.broker.record_result(ticket, b"{}").unwrap();
            }
            other => panic!("{other:?}"),
        }
    }
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Parked { promotion } => {
            let pending = w.broker.list_promotions("pending").unwrap();
            let ops = pending[0]["preview"]["ops"].as_array().unwrap();
            // A13: the reorganization must read as moves, not mass deletion.
            assert!(ops.iter().all(|o| o["op"] == "move"), "{ops:?}");
            assert_eq!(ops.len(), 2);
            w.broker.approve_promotion(promotion, &w.chan, "local_session").unwrap();
        }
        other => panic!("moves must park under default policy: {other:?}"),
    }
    assert!(w.vault.join("notes/a.md").exists());
    assert!(!w.vault.join("inbox/a.md").exists());
}

#[test]
fn gate_blocks_run_that_exceeded_its_token() {
    let mut w = setup();
    agent_write(&mut w, "inbox/ok.md", "fine\n");
    // A write the broker never authorized sneaks into the trace claiming
    // this capability — e.g. a compromised recorder. Forge it via the
    // kernel API directly, bypassing propose_call.
    w.broker
        .fabric
        .record_tool_call(
            "tool:vault@1.0",
            "note.nuke", // not in action.allow
            b"{}",
            b"{}",
            json!({"capability": w.cap, "action_class": "delete", "paths": ["inbox/a.md"]}),
            json!([{"caveat": "action.allow", "ok": true}]), // liar
            Some("irreversible"),
        )
        .unwrap();
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch) {
        Err(BrokerError::GateTraceViolation(msg)) => {
            assert!(msg.contains("action.allow"), "{msg}");
        }
        other => panic!("gate must catch the forged call: {other:?}"),
    }
    // Nothing merged: trunk untouched.
    assert!(!w.vault.join("inbox/ok.md").exists());
}

#[test]
fn sqlite_branch_change_promotes_whole_store() {
    let mut w = setup();
    // Agent writes memory on its branch copy.
    {
        let conn = Connection::open(&w.branch["db:memory"]).unwrap();
        conn.execute("INSERT INTO memories (fact) VALUES ('learned during run')", []).unwrap();
    }
    agent_write(&mut w, "inbox/c.md", "note\n");
    let branch = w.branch.clone();
    // Branch-only sqlite change is a whole-store `modify`: trunk untouched,
    // so it auto-promotes — the agent's memory updating itself every run is
    // the zero-friction default, not an approval event.
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Applied { .. } => {}
        other => panic!("branch-only memory change must auto-promote: {other:?}"),
    }
    let conn = Connection::open(&w.memory_db).unwrap();
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 2, "branch memory image installed on trunk");
}

#[test]
fn sqlite_both_changed_is_conflict_trunk_wins() {
    let mut w = setup();
    {
        let conn = Connection::open(&w.branch["db:memory"]).unwrap();
        conn.execute("INSERT INTO memories (fact) VALUES ('agent version')", []).unwrap();
    }
    // A brokered step attests the updated memory root along with the fs root.
    agent_write(&mut w, "memory-attestation.md", "memory updated\n");
    {
        let conn = Connection::open(&w.memory_db).unwrap();
        conn.execute("INSERT INTO memories (fact) VALUES ('human version')", []).unwrap();
    }
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Parked { promotion } => {
            let pending = w.broker.list_promotions("pending").unwrap();
            let conflicts = pending[0]["preview"]["conflicts"].as_array().unwrap();
            assert!(conflicts[0]["path"].as_str().unwrap().contains("db:memory"));
            w.broker.approve_promotion(promotion, &w.chan, "local_session").unwrap();
        }
        other => panic!("{other:?}"),
    }
    let conn = Connection::open(&w.memory_db).unwrap();
    let facts: Vec<String> = {
        let mut stmt = conn.prepare("SELECT fact FROM memories ORDER BY id").unwrap();
        let rows = stmt.query_map([], |r| r.get(0)).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    };
    assert_eq!(facts, vec!["base".to_string(), "human version".to_string()],
               "opaque-store conflict: trunk wins (SI-18)");
}

#[test]
fn promotion_survives_revert() {
    let mut w = setup();
    agent_write(&mut w, "inbox/c.md", "promoted content\n");
    let branch = w.branch.clone();
    let PromotionOutcome::Applied { .. } = w.broker.promote_manifest(&w.manifest, &branch).unwrap()
    else {
        panic!("expected auto-promote")
    };
    assert!(w.vault.join("inbox/c.md").exists());

    // Snapshot ancestry survives the merge: revert to the pre-run manifest
    // still works and rolls the promotion back out.
    let manifest = w.manifest.clone();
    w.broker.fabric.revert_to(&manifest).unwrap();
    assert!(!w.vault.join("inbox/c.md").exists());
    assert_eq!(fs::read_to_string(w.vault.join("inbox/a.md")).unwrap(), "base a\n");
}

#[test]
fn destructive_ops_park_then_apply_on_approval() {
    let mut w = setup();
    // Agent deletes a file through the broker (delete op class).
    agent_delete(&mut w, "inbox/b.md");
    let branch = w.branch.clone();
    let promotion = match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Parked { promotion } => promotion,
        other => panic!("delete must park: {other:?}"),
    };
    // Trunk untouched while parked.
    assert!(w.vault.join("inbox/b.md").exists());

    // The reviewed candidate is immutable. A later direct branch mutation
    // must neither widen the approved change nor invalidate the pinned CAS
    // candidate used for the approval-time remerge.
    fs::write(w.branch["fs:vault"].join("surprise.md"), "not previewed\n").unwrap();

    // Weak channel cannot approve (approval.min_auth).
    assert!(matches!(
        w.broker.approve_promotion(promotion, "chan:tg", "platform_oauth"),
        Err(BrokerError::ChannelTooWeak { .. })
    ));
    w.broker.approve_promotion(promotion, &w.chan, "local_session").unwrap();
    assert!(!w.vault.join("inbox/b.md").exists(), "approved delete applied");
    assert!(
        !w.vault.join("surprise.md").exists(),
        "approval must apply the pinned preview, not mutable branch paths"
    );

    // A second approval of the same promotion is rejected.
    assert!(matches!(
        w.broker.approve_promotion(promotion, &w.chan, "local_session"),
        Err(BrokerError::NoSuchPromotion(_))
    ));
    let _ = w.agent;
}

/// M7/A21: under `authority: {"mode":"brokered"}` every effect must be
/// capability-attributed. A kernel-recorded call with no capability in its
/// summary (the SI-7 pre-broker shape) is a gate violation, not a tolerated
/// unattributed record — declared-brokered never silently degrades to
/// observed.
#[test]
fn m7_brokered_manifest_rejects_unattributed_calls() {
    let mut w = setup_brokered();
    agent_write(&mut w, "inbox/ok.md", "fine\n");
    // An effect recorded outside the broker: no `capability` in the summary.
    w.broker
        .fabric
        .record_tool_call(
            "tool:vault@1.0",
            "note.write",
            br#"{"path":"inbox/side.md"}"#,
            b"{}",
            json!({"action_class": "write", "paths": ["inbox/side.md"]}),
            json!([]),
            Some("reversible"),
        )
        .unwrap();
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch) {
        Err(BrokerError::GateTraceViolation(msg)) => {
            assert!(msg.contains("no capability attribution"), "{msg}");
        }
        other => panic!("brokered mode must reject unattributed effects: {other:?}"),
    }
    // Nothing merged: trunk untouched.
    assert!(!w.vault.join("inbox/ok.md").exists());
}

/// M7/A21 happy path: the manifest declares brokered, mint's grant event
/// precedes every recorded effect in substrate order, and the gate promotes
/// clean. Also pins the declared shape of the sealed manifest body.
#[test]
fn m7_brokered_end_to_end_gates_clean() {
    let mut w = setup_brokered();
    let man = trace::get_object(&w.broker.fabric.conn, &w.manifest).unwrap();
    assert_eq!(man["authority"]["mode"], "brokered");

    agent_write(&mut w, "inbox/c.md", "new note\n");
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Applied { .. } => {}
        other => panic!("attributed brokered run must gate clean: {other:?}"),
    }
    assert_eq!(fs::read_to_string(w.vault.join("inbox/c.md")).unwrap(), "new note\n");

    // The forward edge exists and precedes the effect: a verified grant
    // event for this manifest at a lower substrate offset than the call.
    let events = trace::all_events(&w.broker.fabric.conn).unwrap();
    let grant = events
        .iter()
        .find(|e| e.kind == "grant" && e.manifest.as_deref() == Some(w.manifest.as_str()))
        .expect("mint countersigned a grant event");
    assert_eq!(grant.raw["body"]["capability"], json!(w.cap));
    let call = events
        .iter()
        .find(|e| e.kind == "tool_call" && e.raw["body"]["summary"]["capability"] == json!(w.cap))
        .expect("recorded call");
    assert!(grant.offset < call.offset, "grant must precede the effect");
}

/// M7/A21: observed mode (authority absent) claims nothing and forbids
/// nothing — an unattributed kernel-recorded call is tolerated at the gate,
/// exactly as before A21. The declaration only ever adds constraints.
#[test]
fn m7_observed_manifest_tolerates_unattributed_calls() {
    let mut w = setup();
    let man = trace::get_object(&w.broker.fabric.conn, &w.manifest).unwrap();
    assert!(man.get("authority").is_none(), "observed manifests omit authority");

    w.broker
        .fabric
        .record_tool_call(
            "tool:vault@1.0",
            "note.write",
            br#"{"path":"inbox/side.md"}"#,
            b"{}",
            json!({"action_class": "write", "paths": ["inbox/side.md"]}),
            json!([]),
            Some("reversible"),
        )
        .unwrap();
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Applied { .. } => {}
        other => panic!("observed mode must tolerate unattributed records: {other:?}"),
    }
}
