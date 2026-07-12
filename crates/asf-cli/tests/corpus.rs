//! W-9 corpus-harness contracts (CORPUS-FAIL-CLOSED in tests/contracts.tsv).
//!
//! The fixture is synthetic, Toucan-row-shaped data (no third-party
//! content): two clean trajectories — one with a hallucinated call, one
//! using the parallel tool_calls form — plus one trajectory whose call
//! arguments do not parse. The positive side pins the deterministic
//! verdict baseline (golden per-cell counts + reproducible vector hash);
//! the negative side proves quarantine is fail-closed AT THE SUBSTRATE:
//! the malformed trajectory leaves no manifest, no intent, and no
//! tool_call event — not merely an error string.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_corpora() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corpora")
}

fn run_corpus(args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_asf"))
        .arg("corpus")
        .args(args)
        .output()
        .expect("asf corpus runs");
    assert!(
        out.status.success(),
        "asf corpus {args:?} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Value::Null
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).expect("report exists"))
        .expect("report parses")
}

fn replay_into(dir: &Path) -> Value {
    run_corpus(&[
        "replay",
        "--corpora",
        fixture_corpora().to_str().unwrap(),
        "--out",
        dir.to_str().unwrap(),
    ]);
    read_json(&dir.join("replay.json"))
}

#[test]
fn corpus_sample_replay_matches_golden_verdicts() {
    let out1 = tempfile::tempdir().unwrap();
    let r1 = replay_into(out1.path());

    assert_eq!(r1["trajectories"], 2, "quarantined row must not replay");
    assert_eq!(r1["calls"], 7);
    assert_eq!(r1["evaluations"], 28, "7 calls x 4 cells");

    // Golden cells. floor-t0 is the zero-authorship posture: every call
    // denied, with reversibility/external-reach/paths all failing closed.
    let floor_t0 = &r1["cells"]["floor-t0"];
    assert_eq!(floor_t0["allow"], 0);
    assert_eq!(floor_t0["deny"], 7);
    assert_eq!(floor_t0["escalate"], 0);
    assert_eq!(floor_t0["failed_dims"]["reversibility.max"], 7);
    assert_eq!(floor_t0["failed_dims"]["external_reach"], 7);
    assert_eq!(floor_t0["failed_dims"]["paths.write"], 7);
    assert_eq!(floor_t0["failed_dims"]["action.allow"], 1);

    // heuristic-t1 is the mixed cell: reads + compensable writes pass,
    // the hallucinated call and the destructive call deny.
    let ht1 = &r1["cells"]["heuristic-t1"];
    assert_eq!(ht1["allow"], 5, "cells: {}", r1["cells"]);
    assert_eq!(ht1["deny"], 2);
    assert_eq!(ht1["escalate"], 0);
    assert_eq!(ht1["failed_dims"]["action.allow"], 1);
    assert_eq!(ht1["failed_dims"]["reversibility.max"], 1);

    // Quarantine is visible and named.
    let quarantine = r1["quarantine"].as_array().unwrap();
    assert!(
        quarantine
            .iter()
            .any(|q| q["uuid"] == "fixture-0003" && q["reason"] == "unparseable_call_arguments"),
        "quarantine: {quarantine:?}"
    );

    // Determinism: a second run reproduces the verdict vector hash.
    let out2 = tempfile::tempdir().unwrap();
    let r2 = replay_into(out2.path());
    assert_eq!(r1["verdict_vector_sha256"], r2["verdict_vector_sha256"]);
    assert_eq!(r1["verdict_vector_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(r1["vector_lines"], 28);
}

#[test]
fn corpus_hallucinated_call_denies_action_allow() {
    let out = tempfile::tempdir().unwrap();
    let r = replay_into(out.path());
    let detail = r["sample_details"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| {
            d["cell"] == "heuristic-t1" && d["uuid"] == "fixture-0001" && d["call"] == 2
        })
        .expect("hallucinated-call detail present")
        .clone();
    assert_eq!(detail["unregistered"], true);
    assert_eq!(detail["outcome"], "deny");
    assert_eq!(
        detail["failed"],
        serde_json::json!(["action.allow"]),
        "a hallucinated call must deny on action.allow and nothing else in heuristic-t1"
    );
}

#[test]
fn corpus_malformed_trajectory_is_quarantined_and_emits_no_events() {
    let home = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    run_corpus(&[
        "ingest",
        "--corpora",
        fixture_corpora().to_str().unwrap(),
        "--home",
        home.path().to_str().unwrap(),
        "--out",
        out.path().to_str().unwrap(),
    ]);
    let report = read_json(&out.path().join("ingest.json"));
    assert_eq!(report["trajectories_ingested"], 2);
    assert_eq!(report["ledger_unexplained"], 0);
    assert!(report["quarantine"]
        .as_array()
        .unwrap()
        .iter()
        .any(|q| q["uuid"] == "fixture-0003"));

    // The protected effect must NOT have occurred: the quarantined
    // trajectory contributed nothing to the substrate. Counts are exact
    // for the two clean trajectories, so any third manifest/intent or
    // extra tool_call event fails here.
    let conn =
        rusqlite::Connection::open(home.path().join("fabric").join("fabric.db")).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM objects WHERE kind='manifest'"), 2);
    assert_eq!(count("SELECT COUNT(*) FROM objects WHERE kind='intent'"), 2);
    assert_eq!(count("SELECT COUNT(*) FROM events WHERE kind='tool_call'"), 7);
    // And the two registered corpus tools are exactly alpha + beta.
    assert_eq!(count("SELECT COUNT(*) FROM objects WHERE kind='tool'"), 2);
}
