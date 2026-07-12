//! W-9 corpus-harness contracts (CORPUS-FAIL-CLOSED in tests/contracts.tsv).
//!
//! The fixture is synthetic, Toucan-row-shaped data (no third-party
//! content): three clean trajectories — a hallucinated call, the
//! parallel tool_calls form, and a 22-write run that exercises budget
//! metering into the Escalate outcome — plus one trajectory whose call
//! arguments do not parse. The positive side pins the deterministic
//! verdict baseline to a CONSTANT hash (any semantic change to
//! derivation, capabilities, quarantine, or the evaluator must show up
//! as a reviewed constant update, not silent drift); the negative side
//! proves quarantine and the fresh-home rule fail closed AT THE
//! SUBSTRATE, not merely as error strings.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pinned verdict-vector hash for the committed fixture. Recompute with
/// `asf corpus replay --corpora crates/asf-cli/tests/fixtures/corpora`
/// after any INTENTIONAL semantic change, and say so in the PR.
const FIXTURE_VECTOR_SHA256: &str =
    "bc1ade8dcb6bb8f60781e0f22fbfab07a1bb425d5b8d660c198f764288348d2f";

fn fixture_corpora() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corpora")
}

fn run_corpus(args: &[&str]) {
    let out = corpus_command(args);
    assert!(
        out.status.success(),
        "asf corpus {args:?} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn corpus_command(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_asf"))
        .arg("corpus")
        .args(args)
        .output()
        .expect("asf corpus runs")
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).expect("report exists"))
        .expect("report parses")
}

fn replay_into(dir: &Path, vector: bool) -> Value {
    let corpora = fixture_corpora();
    let mut args = vec![
        "replay",
        "--corpora",
        corpora.to_str().unwrap(),
        "--out",
        dir.to_str().unwrap(),
    ];
    if vector {
        args.push("--vector");
    }
    run_corpus(&args);
    read_json(&dir.join("replay.json"))
}

#[test]
fn corpus_sample_replay_matches_golden_verdicts() {
    let out1 = tempfile::tempdir().unwrap();
    let r1 = replay_into(out1.path(), false);

    assert_eq!(r1["trajectories"], 3, "quarantined row must not replay");
    assert_eq!(r1["calls"], 29);
    assert_eq!(r1["vector_lines"], 116, "29 calls x 4 cells");
    assert_eq!(
        r1["evaluations"], 112,
        "the undeclared-action call short-circuits structurally in all 4 cells"
    );

    // All four cells pinned. floor cells are the zero-authorship
    // posture: every call denied; heuristic-t1 is the mixed cell with
    // all three outcomes.
    let cell = |name: &str| r1["cells"][name].clone();
    for (name, deny, dims) in [
        (
            "floor-t0",
            29,
            serde_json::json!({"external_reach": 28, "paths.write": 28,
                "reversibility.max": 28, "structural:undeclared_action": 1}),
        ),
        (
            "floor-t1",
            29,
            serde_json::json!({"reversibility.max": 28, "structural:undeclared_action": 1}),
        ),
        (
            "heuristic-t0",
            29,
            serde_json::json!({"external_reach": 28, "paths.write": 27,
                "reversibility.max": 1, "structural:undeclared_action": 1}),
        ),
    ] {
        let c = cell(name);
        assert_eq!(c["allow"], 0, "{name}");
        assert_eq!(c["deny"], deny, "{name}");
        assert_eq!(c["escalate"], 0, "{name}");
        assert_eq!(c["failed_dims"], dims, "{name}");
    }
    let ht1 = cell("heuristic-t1");
    assert_eq!(ht1["allow"], 25);
    assert_eq!(ht1["deny"], 2);
    assert_eq!(ht1["escalate"], 2, "22 writes vs t1 budget of 20");
    assert_eq!(
        ht1["failed_dims"],
        serde_json::json!({"budget.count:write": 2, "reversibility.max": 1,
            "structural:undeclared_action": 1})
    );

    // Quarantine is visible and named.
    let quarantine = r1["quarantine"].as_array().unwrap();
    assert!(
        quarantine
            .iter()
            .any(|q| q["uuid"] == "fixture-0003" && q["reason"] == "unparseable_call_arguments"),
        "quarantine: {quarantine:?}"
    );

    // Determinism against the PINNED constant (drift-visible), plus a
    // second run for run-to-run reproduction.
    assert_eq!(r1["verdict_vector_sha256"], FIXTURE_VECTOR_SHA256);
    let out2 = tempfile::tempdir().unwrap();
    let r2 = replay_into(out2.path(), false);
    assert_eq!(r2["verdict_vector_sha256"], FIXTURE_VECTOR_SHA256);
}

#[test]
fn corpus_vector_file_bytes_hash_to_reported_value() {
    use sha2::{Digest, Sha256};
    let out = tempfile::tempdir().unwrap();
    let r = replay_into(out.path(), true);
    let bytes = std::fs::read(out.path().join("verdict-vector.txt")).expect("vector file");
    let file_hash = hex::encode(Sha256::digest(&bytes));
    assert_eq!(
        Some(file_hash.as_str()),
        r["verdict_vector_sha256"].as_str(),
        "the emitted vector file must be exactly the hashed stream"
    );
    assert_eq!(file_hash, FIXTURE_VECTOR_SHA256);
}

#[test]
fn corpus_hallucinated_call_is_denied_structurally_before_evaluation() {
    // Mirrors broker::propose_call: an undeclared action never reaches
    // the evaluator — structural denial, empty checks, identical in
    // every cell.
    let out = tempfile::tempdir().unwrap();
    let r = replay_into(out.path(), false);
    let details: Vec<&Value> = r["sample_details"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["uuid"] == "fixture-0001" && d["call"] == 2)
        .collect();
    assert_eq!(details.len(), 4, "one sample per cell");
    for d in details {
        assert_eq!(d["unregistered"], true);
        assert_eq!(d["outcome"], "deny");
        assert_eq!(
            d["failed"],
            serde_json::json!(["structural:undeclared_action"]),
            "cell {}",
            d["cell"]
        );
    }
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
    assert_eq!(report["trajectories_ingested"], 3);
    assert_eq!(report["ledger_unexplained"], 0);
    assert!(report["quarantine"]
        .as_array()
        .unwrap()
        .iter()
        .any(|q| q["uuid"] == "fixture-0003"));

    // The protected effect must NOT have occurred: the quarantined
    // trajectory contributed nothing to the substrate. Counts are exact
    // for the three clean trajectories, so any extra manifest/intent or
    // tool_call event fails here.
    let conn =
        rusqlite::Connection::open(home.path().join("fabric").join("fabric.db")).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM objects WHERE kind='manifest'"), 3);
    assert_eq!(count("SELECT COUNT(*) FROM objects WHERE kind='intent'"), 3);
    assert_eq!(count("SELECT COUNT(*) FROM events WHERE kind='tool_call'"), 29);
    // And the registered corpus tools are exactly alpha + beta + delta.
    assert_eq!(count("SELECT COUNT(*) FROM objects WHERE kind='tool'"), 3);
}

#[test]
fn corpus_reingest_into_existing_home_is_refused_and_substrate_unchanged() {
    let home = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let corpora = fixture_corpora();
    let args = [
        "ingest",
        "--corpora",
        corpora.to_str().unwrap(),
        "--home",
        home.path().to_str().unwrap(),
        "--out",
        out.path().to_str().unwrap(),
    ];
    run_corpus(&args);

    let db = home.path().join("fabric").join("fabric.db");
    let counts = |db: &Path| -> (i64, i64) {
        let conn = rusqlite::Connection::open(db).unwrap();
        (
            conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                .unwrap(),
            conn.query_row("SELECT COUNT(*) FROM objects WHERE kind='tool'", [], |r| {
                r.get(0)
            })
            .unwrap(),
        )
    };
    let before = counts(&db);

    // Second ingest into the SAME home must refuse (immutable tool refs
    // would double-register) and must leave the substrate untouched.
    let second = corpus_command(&args);
    assert!(
        !second.status.success(),
        "re-ingest into an existing home must fail"
    );
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("existing fabric home"),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(counts(&db), before, "substrate must be byte-count identical");
}
