//! Promotion-gate invariant suite. Spec §5.3 (A11, A13).
//!
//! Coverage map:
//! - Three-way merge, trunk-wins conflicts: unit tests in promote.rs +
//!   `concurrent_human_and_agent_edits_merge` here (over real stores)
//! - Rename detection / mass-rename-not-mass-delete: unit tests in
//!   promote.rs + `reorganization_parks_but_renders_as_moves`
//! - Trace-vs-capability at the gate (W-2 unified replay): the gate re-runs
//!   the decision-time evaluator over each signed call — forged calls die at
//!   re-evaluation (`gate_blocks_run_that_exceeded_its_token`), undeclared
//!   actions fail closed (`gate_rejects_undeclared_action_in_trace`), the
//!   RF-1 dimension class is caught
//!   (`gate_catches_time_violation_the_decision_evaluator_missed`), and the
//!   replay clock is the event's signed `at` (SI-22) so honest work survives
//!   gate latency (`si22_gate_clock_is_the_events_at_not_gate_time`)
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
//!   Stored capabilities remain inert without an exact, preceding grant:
//!   missing, late, wrong-kind, and wrong-manifest grant claims are rejected
//!   by the four `m7_*_cannot_activate_capability` adversarial cases below.
//! - A21 adversarial substitution/lineage cases: cross-manifest capability
//!   substitution dies at the gate's M2 arm
//!   (`gate_rejects_call_attributed_to_another_manifests_capability`),
//!   unresolvable attribution fails closed
//!   (`gate_fails_closed_on_unresolvable_capability_attribution`), a
//!   multi-grant lineage gates clean
//!   (`m7_brokered_run_with_two_grants_gates_clean`). Grant deletion and
//!   reorder tamper belong to chain verification (trace.rs
//!   `deleting_a_middle_event_breaks_the_chain`; offset-integrity residual
//!   tracked as RF-13).
//! - A22/§5.4 capability closure (the `a22_*` block below; decision-time
//!   cases in tests/broker.rs): liveness at each effect's durable
//!   authorization offset — non-retroactive (`a22_pre_revoke_work…`,
//!   `a22_parked_promotion…`), dispatch-vs-revoke total order in both
//!   directions (`a22_effect_recorded_after…`, `a22_inflight_ticket…`),
//!   descendant cascade across manifests (`a22_ancestor_revoke…`) with
//!   child-only isolation (`a22_child_revoke…`), doubt-never-widens on
//!   both edges (`a22_unsigned_revocation_row…`,
//!   `a22_anomalous_manifest_revoke…`), permanence (`a22_same_id_regrant…`,
//!   `a22_revoked_before_first_grant…`), and ancestry well-ordering
//!   (`a22_ancestor_grants…`).

use asf_kernel::broker::{Broker, BrokerError, Decision, PromotionOutcome};
use asf_kernel::kernel::{AuthorityMode, Fabric};
use asf_kernel::keys::Role;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use asf_kernel::{canon, capability, trace};
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
    human: String,
    agent: String,
    intent: String,
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
        StoreSpec {
            store: "fs:vault".into(),
            tier: 1,
            kind: StoreKind::Fs,
            path: vault.clone(),
        },
        StoreSpec {
            store: "db:memory".into(),
            tier: 1,
            kind: StoreKind::Sqlite,
            path: memory_db.clone(),
        },
    ];
    let mut fabric = Fabric::open(tmp.path().join("fabric"), stores).unwrap();
    let human = fabric
        .register_principal("human", "josh", "01", None)
        .unwrap();
    let agent = fabric
        .register_principal("agent", "hermes", "02", Some(&human))
        .unwrap();
    let chan = fabric
        .register_channel(&human, "local_session", b"tty", "local_session")
        .unwrap();
    let intent = fabric
        .capture_intent(
            &human,
            &chan,
            "local_session",
            "vault maintenance",
            json!({}),
            None,
        )
        .unwrap();
    let step = fabric
        .step_boundary_with_mode(
            &human,
            &agent,
            &intent,
            json!({"bundle":"sha256:g","skills":[]}),
            mode,
        )
        .unwrap();
    let branch = fabric.create_branch(&step.manifest).unwrap();

    let mut broker = Broker::new(fabric).unwrap();
    broker
        .register_tool("tool:vault@1.0", vault_actions())
        .unwrap();
    let cap = broker
        .mint(
            &step.manifest,
            &agent,
            caveats(),
            vec![],
            "2027-01-01T00:00:00Z",
        )
        .unwrap();
    World {
        _tmp: tmp,
        vault,
        memory_db,
        broker,
        human,
        agent,
        intent,
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
        .propose_call(
            &w.cap,
            "tool:vault@1.0",
            "note.write",
            &json!({"path": rel, "content": content}),
        )
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

/// Store a valid fabric-signed capability without emitting its activating
/// grant event. A15 makes this row an inert materialized view.
fn store_ungranted_capability(w: &mut World) -> String {
    let sk = w
        .broker
        .fabric
        .keystore()
        .signing_key(Role::Fabric)
        .unwrap();
    let body = capability::build(
        &w.agent,
        &w.manifest,
        None,
        "2026-07-10T00:00:00Z",
        "2027-01-01T00:00:00Z",
        caveats(),
        vec![],
    )
    .unwrap();
    let sealed = canon::seal("cap", body, &sk).unwrap();
    trace::put_object(
        &w.broker.fabric.conn,
        "capability",
        &sealed,
        "2026-07-10T00:00:00Z",
    )
    .unwrap()
}

fn append_substrate_event(w: &mut World, manifest: &str, kind: &str, body: Value) {
    let sk = w
        .broker
        .fabric
        .keystore()
        .signing_key(Role::Fabric)
        .unwrap();
    let span = trace::meta_get(&w.broker.fabric.conn, "substrate_span")
        .unwrap()
        .expect("substrate span");
    trace::append(
        &mut w.broker.fabric.conn,
        &sk,
        &span,
        Some(manifest),
        kind,
        body,
        "2026-07-10T00:00:01Z",
    )
    .unwrap();
}

/// Record an effect that claims a capability without going through broker
/// evaluation. The gate must independently reconstruct and verify authority.
fn record_claimed_write(w: &mut World, capability: &str, rel: &str) {
    let path = w.branch["fs:vault"].join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "claimed write\n").unwrap();
    w.broker
        .fabric
        .record_tool_call(
            "tool:vault@1.0",
            "note.write",
            serde_json::to_string(&json!({"path": rel}))
                .unwrap()
                .as_bytes(),
            b"{}",
            json!({
                "capability": capability,
                "action_class": "write",
                "paths": [rel]
            }),
            json!([]),
            Some("reversible"),
        )
        .unwrap();
}

fn assert_m7_rejected_without_promotion(w: &mut World, expected: &str, rel: &str) {
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch) {
        Err(BrokerError::GateTraceViolation(msg)) => {
            assert!(msg.contains(expected), "{msg}");
        }
        other => panic!("invalid grant lineage must fail closed: {other:?}"),
    }
    assert!(!w.vault.join(rel).exists(), "rejected effect reached trunk");
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

            w.broker
                .approve_promotion(promotion, &w.chan, "local_session")
                .unwrap();
        }
        other => panic!("expected park on conflict: {other:?}"),
    }

    // Merged trunk: human's conflict version won, human's b.md stands,
    // agent's non-conflicting addition landed.
    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/a.md")).unwrap(),
        "human edit of a\n"
    );
    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/b.md")).unwrap(),
        "human edit\n"
    );
    assert_eq!(
        fs::read_to_string(w.vault.join("new.md")).unwrap(),
        "agent addition\n"
    );

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
    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/c.md")).unwrap(),
        "new note\n"
    );
    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/a.md")).unwrap(),
        "improved a\n"
    );
    assert!(
        w.broker.fabric.check_drift().unwrap().is_empty(),
        "promotion re-baselined expectations"
    );
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
            .propose_call(
                &w.cap,
                "tool:vault@1.0",
                "note.move",
                &json!({"src": src, "dest": dest}),
            )
            .unwrap();
        match d {
            Decision::Allowed { ticket, .. } => {
                let (s, t) = (
                    w.branch["fs:vault"].join(src),
                    w.branch["fs:vault"].join(dest),
                );
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
            w.broker
                .approve_promotion(promotion, &w.chan, "local_session")
                .unwrap();
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
    // A registered action the broker never authorized sneaks into the trace
    // claiming a capability whose allowlist excludes it — e.g. a compromised
    // recorder. Forge it via the kernel API directly, bypassing propose_call;
    // the W-2 replay must catch it regardless of the recorded `checks` lie.
    let write_only = w
        .broker
        .mint(
            &w.manifest,
            &w.agent,
            vec![
                json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.write"]}),
                json!({"dim":"paths.write","globs":["**"]}),
            ],
            vec![],
            "2027-01-01T00:00:00Z",
        )
        .unwrap();
    w.broker
        .fabric
        .record_tool_call(
            "tool:vault@1.0",
            "note.delete", // registered, but outside write_only's allowlist
            br#"{"path":"inbox/a.md"}"#,
            b"{}",
            json!({"capability": write_only, "action_class": "delete", "paths": ["inbox/a.md"]}),
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

/// §4 at the gate: an action absent from the tool's registration cannot be
/// re-evaluated against anything — the gate fails closed on it exactly as
/// the decision path refuses to call it.
#[test]
fn gate_rejects_undeclared_action_in_trace() {
    let mut w = setup();
    agent_write(&mut w, "inbox/ok.md", "fine\n");
    w.broker
        .fabric
        .record_tool_call(
            "tool:vault@1.0",
            "note.nuke", // never registered
            b"{}",
            b"{}",
            json!({"capability": w.cap, "action_class": "delete", "paths": ["inbox/a.md"]}),
            json!([]),
            Some("irreversible"),
        )
        .unwrap();
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch) {
        Err(BrokerError::GateTraceViolation(msg)) => {
            assert!(msg.contains("undeclared actions cannot be called"), "{msg}");
        }
        other => panic!("undeclared action in the trace must fail the gate: {other:?}"),
    }
    assert!(!w.vault.join("inbox/ok.md").exists());
}

#[test]
fn sqlite_branch_change_promotes_whole_store() {
    let mut w = setup();
    // Agent writes memory on its branch copy.
    {
        let conn = Connection::open(&w.branch["db:memory"]).unwrap();
        conn.execute(
            "INSERT INTO memories (fact) VALUES ('learned during run')",
            [],
        )
        .unwrap();
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
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2, "branch memory image installed on trunk");
}

#[test]
fn sqlite_both_changed_is_conflict_trunk_wins() {
    let mut w = setup();
    {
        let conn = Connection::open(&w.branch["db:memory"]).unwrap();
        conn.execute("INSERT INTO memories (fact) VALUES ('agent version')", [])
            .unwrap();
    }
    // A brokered step attests the updated memory root along with the fs root.
    agent_write(&mut w, "memory-attestation.md", "memory updated\n");
    {
        let conn = Connection::open(&w.memory_db).unwrap();
        conn.execute("INSERT INTO memories (fact) VALUES ('human version')", [])
            .unwrap();
    }
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Parked { promotion } => {
            let pending = w.broker.list_promotions("pending").unwrap();
            let conflicts = pending[0]["preview"]["conflicts"].as_array().unwrap();
            assert!(conflicts[0]["path"].as_str().unwrap().contains("db:memory"));
            w.broker
                .approve_promotion(promotion, &w.chan, "local_session")
                .unwrap();
        }
        other => panic!("{other:?}"),
    }
    let conn = Connection::open(&w.memory_db).unwrap();
    let facts: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT fact FROM memories ORDER BY id")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get(0)).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    };
    assert_eq!(
        facts,
        vec!["base".to_string(), "human version".to_string()],
        "opaque-store conflict: trunk wins (SI-18)"
    );
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
    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/a.md")).unwrap(),
        "base a\n"
    );
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
        w.broker
            .approve_promotion(promotion, "chan:tg", "platform_oauth"),
        Err(BrokerError::ChannelTooWeak { .. })
    ));
    w.broker
        .approve_promotion(promotion, &w.chan, "local_session")
        .unwrap();
    assert!(
        !w.vault.join("inbox/b.md").exists(),
        "approved delete applied"
    );
    assert!(
        !w.vault.join("surprise.md").exists(),
        "approval must apply the pinned preview, not mutable branch paths"
    );

    // A second approval of the same promotion is rejected.
    assert!(matches!(
        w.broker
            .approve_promotion(promotion, &w.chan, "local_session"),
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

/// A21/A15: a valid signed capability object is only a materialized view.
/// Without its signed grant event it conveys no authority.
#[test]
fn m7_capability_without_grant_is_inert() {
    let mut w = setup_brokered();
    let cap = store_ungranted_capability(&mut w);
    record_claimed_write(&mut w, &cap, "inbox/ungranted.md");

    assert_m7_rejected_without_promotion(&mut w, "no verified grant event", "inbox/ungranted.md");
}

/// A21 ordering is prospective: a grant appended after an effect cannot
/// retroactively authorize that effect.
#[test]
fn m7_grant_after_effect_does_not_retroactively_authorize() {
    let mut w = setup_brokered();
    let cap = store_ungranted_capability(&mut w);
    record_claimed_write(&mut w, &cap, "inbox/late-grant.md");
    let manifest = w.manifest.clone();
    append_substrate_event(&mut w, &manifest, "grant", json!({"capability": cap}));

    assert_m7_rejected_without_promotion(&mut w, "precedes the grant", "inbox/late-grant.md");
}

/// An event mentioning a capability does not activate it unless the signed
/// event kind is exactly `grant`.
#[test]
fn m7_non_grant_event_cannot_activate_capability() {
    let mut w = setup_brokered();
    let cap = store_ungranted_capability(&mut w);
    let manifest = w.manifest.clone();
    append_substrate_event(
        &mut w,
        &manifest,
        "ratification",
        json!({"capability": cap}),
    );
    record_claimed_write(&mut w, &cap, "inbox/wrong-kind.md");

    assert_m7_rejected_without_promotion(&mut w, "no verified grant event", "inbox/wrong-kind.md");
}

/// A grant is scoped by its signed manifest edge. A grant carrying another
/// manifest id cannot activate a capability for this run.
#[test]
fn m7_grant_for_other_manifest_cannot_activate_capability() {
    let mut w = setup_brokered();
    let cap = store_ungranted_capability(&mut w);
    append_substrate_event(
        &mut w,
        "man:another-lineage",
        "grant",
        json!({"capability": cap}),
    );
    record_claimed_write(&mut w, &cap, "inbox/wrong-manifest.md");

    assert_m7_rejected_without_promotion(
        &mut w,
        "no verified grant event",
        "inbox/wrong-manifest.md",
    );
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
    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/c.md")).unwrap(),
        "new note\n"
    );

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
    assert!(
        man.get("authority").is_none(),
        "observed manifests omit authority"
    );

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

/// Adversarial (A21 substitution): a call attributed to a capability bound
/// to a DIFFERENT manifest must die at the gate's M2 arm. The claimed cap is
/// a real, verifiable object — it just doesn't belong to this run's fork.
#[test]
fn gate_rejects_call_attributed_to_another_manifests_capability() {
    let mut w = setup_brokered();
    let old_cap = w.cap.clone();

    // Re-manifest (the M5 shape): a new brokered manifest supersedes the
    // first; the old capability stays bound to the old manifest.
    let step2 = w
        .broker
        .fabric
        .step_boundary_with_mode(
            &w.human,
            &w.agent,
            &w.intent,
            json!({"bundle":"sha256:g2","skills":[]}),
            AuthorityMode::Brokered,
        )
        .unwrap();
    let branch2 = w.broker.fabric.create_branch(&step2.manifest).unwrap();

    // Forge: an effect in the new span claiming the OLD manifest's cap.
    w.broker
        .fabric
        .record_tool_call(
            "tool:vault@1.0",
            "note.write",
            br#"{"path":"inbox/x.md"}"#,
            b"{}",
            json!({"capability": old_cap, "action_class": "write", "paths": ["inbox/x.md"]}),
            json!([]),
            Some("reversible"),
        )
        .unwrap();

    match w.broker.promote_manifest(&step2.manifest, &branch2) {
        Err(BrokerError::GateTraceViolation(msg)) => {
            assert!(msg.contains("different manifest"), "{msg}");
        }
        other => panic!("cross-manifest capability substitution must be rejected: {other:?}"),
    }
}

/// Adversarial (A21 substitution): attribution to a capability id that
/// resolves to no stored object fails the promotion outright — the check is
/// never skipped, and nothing merges.
#[test]
fn gate_fails_closed_on_unresolvable_capability_attribution() {
    let mut w = setup_brokered();
    agent_write(&mut w, "inbox/ok.md", "fine\n");
    w.broker
        .fabric
        .record_tool_call(
            "tool:vault@1.0",
            "note.write",
            br#"{"path":"inbox/ghost.md"}"#,
            b"{}",
            json!({
                "capability": "cap:0000000000000000000000000000000000000000000000000000000000000000",
                "action_class": "write",
                "paths": ["inbox/ghost.md"]
            }),
            json!([]),
            Some("reversible"),
        )
        .unwrap();
    let branch = w.branch.clone();
    assert!(
        w.broker.promote_manifest(&w.manifest, &branch).is_err(),
        "unresolvable capability attribution must fail the gate"
    );
    // Nothing merged: trunk untouched.
    assert!(!w.vault.join("inbox/ok.md").exists());
}

/// A21: multiple grants over one manifest are legal (re-mint, attenuation) —
/// the authority lineage is the offset-ordered grant set. A brokered run
/// using both the parent capability and an attenuated child gates clean,
/// each claimed capability covered by its own preceding grant.
#[test]
fn m7_brokered_run_with_two_grants_gates_clean() {
    let mut w = setup_brokered();
    let child_caveats = vec![
        json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.write"]}),
        json!({"dim":"paths.write","globs":["inbox/**"]}),
        json!({"dim":"budget.count","action_class":"write","max":2,"window":"run"}),
        json!({"dim":"approval.min_auth","min":"local_session"}),
    ];
    let child = w
        .broker
        .attenuate(
            &w.cap,
            &w.agent,
            &w.manifest,
            child_caveats,
            vec![],
            "2026-12-01T00:00:00Z",
        )
        .unwrap();

    agent_write(&mut w, "inbox/parent.md", "via parent cap\n");
    // A write through the attenuated child capability.
    let d = w
        .broker
        .propose_call(
            &child,
            "tool:vault@1.0",
            "note.write",
            &json!({"path": "inbox/child.md", "content": "via child cap\n"}),
        )
        .unwrap();
    match d {
        Decision::Allowed { ticket, .. } => {
            fs::write(
                w.branch["fs:vault"].join("inbox/child.md"),
                "via child cap\n",
            )
            .unwrap();
            w.broker.record_result(ticket, b"{}").unwrap();
        }
        other => panic!("expected allow under child cap: {other:?}"),
    }

    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Applied { .. } => {}
        other => panic!("two-grant brokered run must gate clean: {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/parent.md")).unwrap(),
        "via parent cap\n"
    );
    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/child.md")).unwrap(),
        "via child cap\n"
    );

    // Both grants exist for this manifest, and each precedes the call that
    // claimed its capability.
    let events = trace::all_events(&w.broker.fabric.conn).unwrap();
    let grant_offsets: BTreeMap<String, i64> = events
        .iter()
        .filter(|e| e.kind == "grant" && e.manifest.as_deref() == Some(w.manifest.as_str()))
        .filter_map(|e| {
            e.raw["body"]["capability"]
                .as_str()
                .map(|c| (c.to_string(), e.offset))
        })
        .collect();
    assert_eq!(grant_offsets.len(), 2);
    for ev in events.iter().filter(|e| e.kind == "tool_call") {
        if let Some(cap) = ev.raw["body"]["summary"]["capability"].as_str() {
            assert!(
                grant_offsets[cap] < ev.offset,
                "grant precedes each claimed effect"
            );
        }
    }
}

/// The RF-1 regression class, closed at the gate (W-2): the pre-replay gate
/// hand-rechecked four of seven dimensions and omitted `time` — a decision-
/// time evaluator bug admitting a call outside its window would have merged
/// unchallenged. The unified replay evaluates every dimension the capability
/// carries, clocked at each event's signed `at`.
#[test]
fn gate_catches_time_violation_the_decision_evaluator_missed() {
    let mut w = setup();
    let mut expired_window = caveats();
    expired_window.push(json!({"dim":"time","not_after":"2020-01-01T00:00:00Z"}));
    let cap = w
        .broker
        .mint(
            &w.manifest,
            &w.agent,
            expired_window,
            vec![],
            "2027-01-01T00:00:00Z",
        )
        .unwrap();
    // Recorded as if authorized — the window says it never should have been.
    record_claimed_write(&mut w, &cap, "inbox/late.md");

    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch) {
        Err(BrokerError::GateTraceViolation(msg)) => {
            assert!(msg.contains("time"), "{msg}");
        }
        other => panic!("out-of-window call must fail re-evaluation: {other:?}"),
    }
    assert!(!w.vault.join("inbox/late.md").exists());
}

/// SI-22: the replay clock is each event's signed `at`, never gate-time now.
/// An honest call made inside its time window must still promote after the
/// window closes — gate latency (parked promotions, RF-9 recovery, coarse
/// sessions) must not retro-fail work that was authorized when it happened.
#[test]
fn si22_gate_clock_is_the_events_at_not_gate_time() {
    let mut w = setup();
    let not_after = (time::OffsetDateTime::now_utc() + time::Duration::milliseconds(1500))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    let mut closing_window = caveats();
    closing_window.push(json!({"dim":"time","not_after": not_after}));
    w.cap = w
        .broker
        .mint(
            &w.manifest,
            &w.agent,
            closing_window,
            vec![],
            "2027-01-01T00:00:00Z",
        )
        .unwrap();
    agent_write(&mut w, "inbox/in-window.md", "authorized in time\n");

    // Let the window close before the gate runs.
    std::thread::sleep(std::time::Duration::from_millis(1700));
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Applied { .. } => {}
        other => panic!("in-window work must survive a gate after the window: {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/in-window.md")).unwrap(),
        "authorized in time\n"
    );
}

// ---- A22/§5.4 capability closure at the gate ------------------------------
//
// Liveness is evaluated at each effect's durable authorization offset —
// its own signed offset, never gate time — via the same pure view decision
// time enforces at the current head. Decision-time closure cases live in
// tests/broker.rs; here: non-retroactivity (including through parked
// promotions), the dispatch-vs-revoke total order in both directions,
// descendant cascade across manifest boundaries, doubt-never-widens on
// both edges, permanence, and ancestry well-ordering.

/// Sign a capability with the fabric key and store its object row WITHOUT
/// any grant — optionally as a child of `parent` — for adversarial lineage
/// construction. (`store_ungranted_capability` is the parentless form.)
fn store_capability_row(w: &mut World, parent: Option<&str>, issued_at: &str) -> String {
    let sk = w
        .broker
        .fabric
        .keystore()
        .signing_key(Role::Fabric)
        .unwrap();
    let body = capability::build(
        &w.agent,
        &w.manifest,
        parent,
        issued_at,
        "2027-01-01T00:00:00Z",
        caveats(),
        vec![],
    )
    .unwrap();
    let sealed = canon::seal("cap", body, &sk).unwrap();
    trace::put_object(&w.broker.fabric.conn, "capability", &sealed, issued_at).unwrap()
}

fn revoke(w: &mut World, cap: &str, reason: &str) {
    let chan = w.chan.clone();
    w.broker
        .revoke_capability(cap, reason, Some((&chan, "local_session")))
        .unwrap();
}

#[test]
fn a22_pre_revoke_work_promotes_after_revoke() {
    let mut w = setup_brokered();
    agent_write(&mut w, "inbox/pre.md", "authorized before closure\n");
    let cap = w.cap.clone();
    revoke(&mut w, &cap, "operator_request");

    // Read-only replay agrees before anything merges.
    let report = w.broker.gate_replay_check(&w.manifest).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["closure_anomalies"].as_array().unwrap().len(), 0);

    // Non-retroactivity: effects durably authorized before R remain
    // historically valid; the revoke closes future authority only.
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Applied { .. } => {}
        other => panic!("pre-revoke work must promote: {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/pre.md")).unwrap(),
        "authorized before closure\n"
    );
}

#[test]
fn a22_effect_recorded_after_revoke_fails_gate() {
    let mut w = setup_brokered();
    agent_write(&mut w, "inbox/pre.md", "fine\n");
    let cap = w.cap.clone();
    revoke(&mut w, &cap, "compromise");
    // An effect recorded after the revoke — its durable authorization
    // offset follows R, so the branch conservatively strands.
    record_claimed_write(&mut w, &cap, "inbox/post.md");
    assert_m7_rejected_without_promotion(&mut w, "revoked", "inbox/post.md");
}

#[test]
fn a22_inflight_ticket_recorded_after_revoke_strands_branch() {
    let mut w = setup_brokered();
    // Dispatch vs revoke, the other direction of the single signed total
    // order: the broker said Allowed BEFORE the revoke, but the effect's
    // first signed record lands AFTER it. An in-memory ticket is never
    // authorization evidence (§5.4) — the branch strands, revert remains.
    let d = w
        .broker
        .propose_call(
            &w.cap,
            "tool:vault@1.0",
            "note.write",
            &json!({"path": "inbox/racy.md", "content": "in flight\n"}),
        )
        .unwrap();
    let ticket = match d {
        Decision::Allowed { ticket, .. } => ticket,
        other => panic!("expected Allowed pre-revoke: {other:?}"),
    };
    let cap = w.cap.clone();
    revoke(&mut w, &cap, "compromise");
    let p = w.branch["fs:vault"].join("inbox/racy.md");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, "in flight\n").unwrap();
    w.broker.record_result(ticket, b"{}").unwrap();

    assert_m7_rejected_without_promotion(&mut w, "revoked", "inbox/racy.md");
}

#[test]
fn a22_ancestor_revoke_closes_cross_manifest_child() {
    let mut w = setup_brokered();
    let parent = w.cap.clone();
    // A hermetic sub-agent fork (M2): child manifest, child capability
    // bound to it, attenuated from the parent capability on manifest 1.
    let behavior = json!({"bundle":"sha256:g","skills":[]});
    let step2 = w
        .broker
        .fabric
        .step_boundary_with_mode(&w.human, &w.agent, &w.intent, behavior, AuthorityMode::Brokered)
        .unwrap();
    let branch2 = w.broker.fabric.create_branch(&step2.manifest).unwrap();
    let child = w
        .broker
        .attenuate(&parent, &w.agent, &step2.manifest, caveats(), vec![], "2027-01-01T00:00:00Z")
        .unwrap();

    // Revoking the parent closes the whole descendant subtree — including
    // a child bound to a DIFFERENT manifest. Revokes resolve by capability
    // id across the substrate, never filtered by the evaluating manifest.
    revoke(&mut w, &parent, "compromise");

    let p = branch2["fs:vault"].join("inbox/sub.md");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, "sub-agent write\n").unwrap();
    w.broker
        .fabric
        .record_tool_call(
            "tool:vault@1.0",
            "note.write",
            serde_json::to_string(&json!({"path": "inbox/sub.md"}))
                .unwrap()
                .as_bytes(),
            b"{}",
            json!({ "capability": child, "action_class": "write", "paths": ["inbox/sub.md"] }),
            json!([]),
            Some("reversible"),
        )
        .unwrap();

    match w.broker.promote_manifest(&step2.manifest, &branch2) {
        Err(BrokerError::GateTraceViolation(msg)) => {
            assert!(msg.contains("revoked"), "{msg}");
            assert!(msg.contains(&parent), "the closed ANCESTOR is named: {msg}");
        }
        other => panic!("ancestor revoke must close the cross-manifest child: {other:?}"),
    }
    assert!(!w.vault.join("inbox/sub.md").exists(), "stranded effect reached trunk");
}

#[test]
fn a22_child_revoke_leaves_parent_promotable() {
    let mut w = setup_brokered();
    let parent = w.cap.clone();
    let child = w
        .broker
        .attenuate(&parent, &w.agent, &w.manifest, caveats(), vec![], "2027-01-01T00:00:00Z")
        .unwrap();
    revoke(&mut w, &child, "operator_request");
    // Parent work after a CHILD revoke: parent and siblings stay live.
    agent_write(&mut w, "inbox/parent-still-live.md", "yes\n");
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Applied { .. } => {}
        other => panic!("child revoke must not close the parent: {other:?}"),
    }
    assert!(w.vault.join("inbox/parent-still-live.md").exists());
}

#[test]
fn a22_unsigned_revocation_row_moves_nothing() {
    let mut w = setup_brokered();
    agent_write(&mut w, "inbox/a22.md", "live\n");
    // Unsigned state is not part of the view (§5.4): a raw row shaped like
    // a revoke, on its own span, with no valid signature — injected by a
    // DB writer — must close nothing at decision time or at the gate.
    let fake_raw = json!({
        "id": "evt:fake", "span": "span:fake", "seq": 0, "prev": null,
        "manifest": w.manifest, "at": "2026-07-12T00:00:00Z",
        "kind": "revoke",
        "body": { "capability": w.cap, "reason": "forged", "channel": null, "auth_strength": null },
        "sig": { "key_id": "fabric", "alg": "ed25519", "value": "00" }
    });
    w.broker
        .fabric
        .conn
        .execute(
            "INSERT INTO events (id, span, seq, prev, manifest, at, kind, raw)
             VALUES ('evt:fake', 'span:fake', 0, NULL, ?1, '2026-07-12T00:00:00Z', 'revoke', ?2)",
            rusqlite::params![w.manifest, serde_json::to_string(&fake_raw).unwrap()],
        )
        .unwrap();

    // Decision time: still live.
    match w
        .broker
        .propose_call(&w.cap, "tool:vault@1.0", "note.write", &json!({"path":"inbox/b22.md","content":"x"}))
        .unwrap()
    {
        Decision::Allowed { ticket, .. } => {
            let p = w.branch["fs:vault"].join("inbox/b22.md");
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "x").unwrap();
            w.broker.record_result(ticket, b"{}").unwrap();
        }
        other => panic!("unsigned revoke row must move nothing: {other:?}"),
    }
    // Gate: promotes clean.
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Applied { .. } => {}
        other => panic!("unsigned revoke row must not strand the branch: {other:?}"),
    }
    assert!(w.vault.join("inbox/a22.md").exists());
}

#[test]
fn a22_anomalous_manifest_revoke_still_closes_loudly() {
    let mut w = setup_brokered();
    agent_write(&mut w, "inbox/pre-anomaly.md", "authorized before closure\n");
    // A signature-verified revoke whose manifest field disagrees with the
    // target's bound_manifest: doubt never widens — the closure HOLDS (a
    // kill switch that silently no-ops on malformed emission is the worst
    // outcome) and the inconsistency surfaces loudly at the gate.
    let (cap, chan) = (w.cap.clone(), w.chan.clone());
    append_substrate_event(
        &mut w,
        "man:wrong",
        "revoke",
        json!({
            "capability": cap, "reason": "compromise",
            "channel": chan, "auth_strength": "local_session"
        }),
    );

    // Closure holds at decision time.
    match w
        .broker
        .propose_call(&w.cap, "tool:vault@1.0", "note.write", &json!({"path":"inbox/x.md","content":"x"}))
        .unwrap()
    {
        Decision::Denied { structural: Some(s), .. } => assert!(s.contains("revoked"), "{s}"),
        other => panic!("anomalous verified revoke must still close: {other:?}"),
    }

    // Pre-revoke work still promotes (non-retroactive), and the anomaly is
    // ledger-visible through the promotion event's trace_check.
    let branch = w.branch.clone();
    match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Applied { .. } => {}
        other => panic!("pre-revoke work must promote: {other:?}"),
    }
    let events = trace::all_events(&w.broker.fabric.conn).unwrap();
    let promo = events.iter().rev().find(|e| e.kind == "promotion").unwrap();
    let anomalies = promo.raw["body"]["trace_check"]["closure_anomalies"]
        .as_array()
        .unwrap();
    assert_eq!(anomalies.len(), 1, "the anomaly is loud: {anomalies:?}");
    assert!(
        anomalies[0].as_str().unwrap().contains("closure holds"),
        "{anomalies:?}"
    );
}

#[test]
fn a22_same_id_regrant_cannot_reactivate() {
    let mut w = setup_brokered();
    let cap = w.cap.clone();
    revoke(&mut w, &cap, "compromise");
    // Permanence is the condition-2 quantifier: ALL revokes before O count,
    // not merely those after the latest grant — a later grant of the same
    // id activates nothing.
    let man = w.manifest.clone();
    append_substrate_event(
        &mut w,
        &man,
        "grant",
        json!({ "capability": cap, "parent": null }),
    );
    match w
        .broker
        .propose_call(&cap, "tool:vault@1.0", "note.write", &json!({"path":"inbox/z.md","content":"x"}))
        .unwrap()
    {
        Decision::Denied { structural: Some(s), .. } => assert!(s.contains("revoked"), "{s}"),
        other => panic!("re-granted closed id must stay dead: {other:?}"),
    }
    record_claimed_write(&mut w, &cap, "inbox/regrant.md");
    assert_m7_rejected_without_promotion(&mut w, "revoked", "inbox/regrant.md");
}

#[test]
fn a22_revoked_before_first_grant_is_dead() {
    let mut w = setup_brokered();
    // R < G: an id revoked before it was ever granted is permanently
    // poisoned — a capability minted to it later is born dead.
    let doomed = store_capability_row(&mut w, None, "2026-07-10T00:00:00Z");
    let (man, chan) = (w.manifest.clone(), w.chan.clone());
    append_substrate_event(
        &mut w,
        &man,
        "revoke",
        json!({
            "capability": doomed, "reason": "compromise",
            "channel": chan, "auth_strength": "local_session"
        }),
    );
    let man = w.manifest.clone();
    append_substrate_event(
        &mut w,
        &man,
        "grant",
        json!({ "capability": doomed, "parent": null }),
    );
    record_claimed_write(&mut w, &doomed, "inbox/doomed.md");
    assert_m7_rejected_without_promotion(&mut w, "revoked", "inbox/doomed.md");
}

#[test]
fn a22_parked_promotion_of_pre_revoke_work_remains_approvable() {
    let mut w = setup_brokered();
    // Destructive class parks under zero-authorship policy…
    agent_delete(&mut w, "inbox/b.md");
    let branch = w.branch.clone();
    let promotion = match w.broker.promote_manifest(&w.manifest, &branch).unwrap() {
        PromotionOutcome::Parked { promotion } => promotion,
        other => panic!("delete must park: {other:?}"),
    };
    // …the capability is revoked while the promotion waits…
    let cap = w.cap.clone();
    revoke(&mut w, &cap, "operator_request");
    // …and the parked promotion of PRE-revoke work remains approvable:
    // promotion ratifies past recorded work at its own offsets; the merge
    // is the human's act. Only future authority died (§5.4).
    let chan = w.chan.clone();
    w.broker
        .approve_promotion(promotion, &chan, "local_session")
        .unwrap();
    assert!(!w.vault.join("inbox/b.md").exists(), "approved delete applied");
}

#[test]
fn a22_ancestor_grants_must_be_well_ordered() {
    let mut w = setup_brokered();
    // Adversarial lineage: an ungranted parent row, a child row granted
    // ahead of it. Liveness requires every link's exact-binding grant with
    // earliest grants well-ordered along the chain (§5.4 condition 4).
    let parent = store_ungranted_capability(&mut w);
    let child = store_capability_row(&mut w, Some(&parent), "2026-07-10T00:00:01Z");
    let man = w.manifest.clone();
    append_substrate_event(
        &mut w,
        &man,
        "grant",
        json!({ "capability": child, "parent": parent }),
    );

    // Missing ancestor grant fails closed.
    record_claimed_write(&mut w, &child, "inbox/orphan.md");
    assert_m7_rejected_without_promotion(&mut w, "no verified grant binds", "inbox/orphan.md");

    // Granting the parent AFTER the child is a disordered lineage — still
    // fail closed (an injected object row + late grant activates nothing).
    let man = w.manifest.clone();
    append_substrate_event(
        &mut w,
        &man,
        "grant",
        json!({ "capability": parent, "parent": null }),
    );
    assert_m7_rejected_without_promotion(&mut w, "out of order", "inbox/orphan.md");
}

#[test]
fn a22_non_grant_event_cannot_activate_capability_in_observed_mode() {
    // Observed mode skips the brokered-only m7 ordering check, so the a22
    // view is the ONLY activation check for attributed calls (which are
    // checked in full whenever a capability exists — M7). A fabric-signed
    // NON-grant event that happens to carry a capability id in its body
    // and a manifest field — broker denial verdicts legitimately look
    // exactly like this — must not activate anything (the view builder
    // keys strictly on kind == "grant").
    let mut w = setup();
    let ungranted = store_ungranted_capability(&mut w);
    let man = w.manifest.clone();
    append_substrate_event(
        &mut w,
        &man,
        "verdict",
        json!({
            "verdict": "deny", "source": "broker", "capability": ungranted,
            "tool": "tool:vault@1.0", "action": "note.write",
            "failed": [], "structural": null, "checks": []
        }),
    );
    record_claimed_write(&mut w, &ungranted, "inbox/nogrant.md");
    assert_m7_rejected_without_promotion(&mut w, "no verified grant binds", "inbox/nogrant.md");
}
