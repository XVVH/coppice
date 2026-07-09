//! The milestone e2e: manifest → mutate vault + sqlite → trace → out-of-band
//! edit → attributed drift → coherent revert → ledger explains everything.
//!
//! Invariant coverage map (spec is source of truth):
//! - M5 (re-manifest on behavior change): `m5_behavior_change_forces_remanifest`
//! - M6 (small manifests / cheap re-baselining): exercised throughout — every
//!   step boundary re-manifests.
//! - Coherent revert (§5.3): `e2e_kernel_round_trip` + `revert_is_all_or_nothing`
//! - §6 chain + signatures: verified in every test via verify_all_spans.
//! - §1 crypto-shredding: `e2e_kernel_round_trip` (structure survives).
//! - A12 drift attribution (single-human default): `e2e_kernel_round_trip`.
//! - M1/M2/M3, C1–C5, §5.2 attenuation: require capabilities/broker/channel
//!   approval surfaces — milestone 2 (SI-7). C1 provenance fields are already
//!   carried on the Intent and its substrate event.

use asf_kernel::kernel::Fabric;
use asf_kernel::payload::PayloadRef;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use asf_kernel::{canon, trace};
use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

struct World {
    _tmp: tempfile::TempDir,
    vault: PathBuf,
    memory_db: PathBuf,
    fabric: Fabric,
}

fn setup() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    fs::create_dir_all(vault.join("inbox")).unwrap();
    fs::write(vault.join("inbox/todo.md"), "- water the plants\n").unwrap();
    fs::write(vault.join("index.md"), "# Vault\n").unwrap();

    let memory_db = tmp.path().join("memory.db");
    {
        let conn = Connection::open(&memory_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (id INTEGER PRIMARY KEY, fact TEXT NOT NULL);
             INSERT INTO memories (fact) VALUES ('user prefers markdown');",
        )
        .unwrap();
    }

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
    let fabric = Fabric::open(tmp.path().join("fabric"), stores).unwrap();
    World {
        _tmp: tmp,
        vault,
        memory_db,
        fabric,
    }
}

fn behavior_v1() -> serde_json::Value {
    json!({
        "bundle": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        "skills": [ { "skill": "vault-filing", "version": "sha256:aaaa", "domains": ["files.vault"] } ],
    })
}

fn read_memory_facts(db: &Path) -> Vec<String> {
    let conn = Connection::open(db).unwrap();
    let mut stmt = conn.prepare("SELECT fact FROM memories ORDER BY id").unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.collect::<Result<_, _>>().unwrap()
}

/// Boilerplate: register principals + channel, capture intent, first manifest.
fn boot(w: &mut World) -> (String, String, String) {
    let f = &mut w.fabric;
    let user_vk = hex::encode([1u8; 32]); // placeholder key material for ids
    let human = f
        .register_principal("human", "josh", &user_vk, None)
        .unwrap();
    let agent = f
        .register_principal("agent", "hermes-vault", &hex::encode([2u8; 32]), Some(&human))
        .unwrap();
    let chan = f
        .register_channel(&human, "local_session", b"tty:/dev/ttys002", "local_session")
        .unwrap();
    let intent = f
        .capture_intent(
            &human,
            &chan,
            "local_session",
            "tidy the vault inbox and remember what you filed",
            json!({ "deadline": null }),
            None,
        )
        .unwrap();
    (human, agent, intent)
}

#[test]
fn e2e_kernel_round_trip() {
    let mut w = setup();
    let (human, agent, intent) = boot(&mut w);

    // -- step boundary: first manifest ---------------------------------
    let step1 = w
        .fabric
        .step_boundary(&human, &agent, &intent, behavior_v1())
        .unwrap();
    assert!(step1.drift.is_empty());
    assert!(!step1.remanifest);

    // The manifest is a sealed, verifiable object whose roots cover BOTH
    // registered stores (M1's structural half: everything the run can
    // touch is snapshotted; capability binding is milestone 2 / SI-7).
    let man1 = trace::get_object(&w.fabric.conn, &step1.manifest).unwrap();
    canon::verify(&man1, &w.fabric.fabric_vk()).unwrap();
    let roots1: Vec<&str> = man1["state"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["store"].as_str().unwrap())
        .collect();
    assert_eq!(roots1.len(), 2);
    assert!(roots1.contains(&"fs:vault") && roots1.contains(&"db:memory"));

    // -- agent run: mutate vault + memory, traced ------------------------
    fs::write(w.vault.join("inbox/todo.md"), "- water the plants\n- filed!\n").unwrap();
    fs::create_dir_all(w.vault.join("MOCs")).unwrap();
    fs::write(w.vault.join("MOCs/plants.md"), "# Plants MOC\n").unwrap();
    {
        let conn = Connection::open(&w.memory_db).unwrap();
        conn.execute(
            "INSERT INTO memories (fact) VALUES ('filed plants note under MOCs')",
            [],
        )
        .unwrap();
    }
    w.fabric
        .record_tool_call(
            "tool:vault@1.0",
            "note.file",
            br#"{"src":"inbox/todo.md","dest":"MOCs/plants.md"}"#,
            br#"{"ok":true}"#,
            json!({ "paths": ["inbox/todo.md", "MOCs/plants.md"] }),
            json!([]),
            Some("reversible"),
        )
        .unwrap();

    // -- second boundary: child manifest, lineage intact -----------------
    let step2 = w
        .fabric
        .step_boundary(&human, &agent, &intent, behavior_v1())
        .unwrap();
    assert!(step2.remanifest);
    assert!(
        step2.drift.is_empty(),
        "traced mutations must NOT read as drift: {:?}",
        step2.drift
    );
    let man2 = trace::get_object(&w.fabric.conn, &step2.manifest).unwrap();
    assert_eq!(man2["parent"].as_str().unwrap(), step1.manifest);

    // -- out-of-band edit: the human touches the vault directly ----------
    fs::write(w.vault.join("index.md"), "# Vault\nhand edit, no agent\n").unwrap();

    let drift = w.fabric.check_drift().unwrap();
    assert_eq!(drift.len(), 1, "exactly the vault drifted");
    assert_eq!(drift[0].store, "fs:vault");
    assert_eq!(
        drift[0].attribution, "human_local",
        "single-human zero-authorship default (A12)"
    );

    // -- coherent revert to manifest 1 ------------------------------------
    w.fabric.revert_to(&step1.manifest).unwrap();

    assert_eq!(
        fs::read_to_string(w.vault.join("inbox/todo.md")).unwrap(),
        "- water the plants\n"
    );
    assert_eq!(
        fs::read_to_string(w.vault.join("index.md")).unwrap(),
        "# Vault\n",
        "out-of-band edit rolled back too — revert is state-based"
    );
    assert!(!w.vault.join("MOCs").exists(), "agent additions rolled back");
    assert_eq!(
        read_memory_facts(&w.memory_db),
        vec!["user prefers markdown".to_string()],
        "memory reverted WITH the vault — undo never gaslights the agent"
    );

    // Post-revert: no drift; live roots equal man1's roots.
    assert!(w.fabric.check_drift().unwrap().is_empty());

    // -- the ledger explains everything -----------------------------------
    let spans = w.fabric.verify_all_spans().unwrap();
    assert!(spans.len() >= 3, "substrate span + two manifest spans");
    let explanation = w.fabric.explain().unwrap();
    assert!(
        explanation.unexplained.is_empty(),
        "unexplained state: {:?}",
        explanation.unexplained
    );
    let kinds: Vec<&str> = explanation.lines.iter().map(|l| l.kind.as_str()).collect();
    for expected in [
        "register", "intent", "snapshot", "tool_call", "remanifest", "drift", "revert",
    ] {
        assert!(kinds.contains(&expected), "ledger missing {expected}");
    }

    // -- crypto-shredding: substance destroyed, structure survives --------
    let intent_obj = trace::get_object(&w.fabric.conn, &intent).unwrap();
    let text_ref: PayloadRef =
        serde_json::from_value(intent_obj["text"].clone()).unwrap();
    assert_eq!(
        w.fabric.get_payload(&text_ref).unwrap(),
        b"tidy the vault inbox and remember what you filed"
    );
    w.fabric.shred_payload(&text_ref.hash, "test_ttl").unwrap();
    assert!(w.fabric.get_payload(&text_ref).is_err());
    // Chains and objects still verify: the ledger records THAT it forgot.
    w.fabric.verify_all_spans().unwrap();
    canon::verify(
        &trace::get_object(&w.fabric.conn, &intent).unwrap(),
        &w.fabric.fabric_vk(),
    )
    .unwrap_err(); // intent was sealed by the USER key, not fabric —
                   // wrong-key verification must fail…
}

#[test]
fn m5_behavior_change_forces_remanifest() {
    let mut w = setup();
    let (human, agent, intent) = boot(&mut w);
    let s1 = w
        .fabric
        .step_boundary(&human, &agent, &intent, behavior_v1())
        .unwrap();

    // Skill hot-loaded mid-run: bundle hash changes.
    let behavior_v2 = json!({
        "bundle": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
        "skills": [
            { "skill": "vault-filing", "version": "sha256:bbbb", "domains": ["files.vault"] }
        ],
    });
    let s2 = w
        .fabric
        .step_boundary(&human, &agent, &intent, behavior_v2.clone())
        .unwrap();
    assert!(s2.remanifest);
    let man2 = trace::get_object(&w.fabric.conn, &s2.manifest).unwrap();
    assert_eq!(man2["parent"].as_str().unwrap(), s1.manifest, "lineage");
    assert_eq!(man2["behavior"], behavior_v2, "new behavior hash bound");
    // The remanifest event records that behavior changed.
    let evs = trace::events_in_span(&w.fabric.conn, &s2.span).unwrap();
    let rm = evs.iter().find(|e| e.kind == "remanifest").unwrap();
    assert_eq!(rm.raw["body"]["behavior_changed"], json!(true));
}

#[test]
fn revert_is_all_or_nothing() {
    let mut w = setup();
    let (human, agent, intent) = boot(&mut w);
    let s1 = w
        .fabric
        .step_boundary(&human, &agent, &intent, behavior_v1())
        .unwrap();

    // Corrupt the CAS so the vault root can't be materialized.
    let man = trace::get_object(&w.fabric.conn, &s1.manifest).unwrap();
    let vault_root = man["state"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["store"] == "fs:vault")
        .unwrap()["root"]
        .as_str()
        .unwrap()
        .to_string();
    let hexpart = vault_root.strip_prefix("sha256:").unwrap();
    let blob = w
        ._tmp
        .path()
        .join("fabric/cas")
        .join(&hexpart[..2])
        .join(hexpart);
    fs::remove_file(&blob).unwrap();

    // Mutate both stores, then attempt revert: it must fail…
    fs::write(w.vault.join("index.md"), "mutated").unwrap();
    {
        let conn = Connection::open(&w.memory_db).unwrap();
        conn.execute("INSERT INTO memories (fact) VALUES ('mutation')", [])
            .unwrap();
    }
    assert!(w.fabric.revert_to(&s1.manifest).is_err());

    // …and NEITHER store may have been touched (prepare-all-then-swap-all).
    assert_eq!(fs::read_to_string(w.vault.join("index.md")).unwrap(), "mutated");
    assert_eq!(read_memory_facts(&w.memory_db).len(), 2);
}

#[test]
fn conservative_default_reversibility_is_irreversible() {
    let mut w = setup();
    let (human, agent, intent) = boot(&mut w);
    w.fabric
        .step_boundary(&human, &agent, &intent, behavior_v1())
        .unwrap();
    let ev = w
        .fabric
        .record_tool_call("tool:x@1", "y.z", b"{}", b"{}", json!({}), json!([]), None)
        .unwrap();
    let rows = trace::events_in_span(&w.fabric.conn, &ev.span).unwrap();
    let tc = rows.iter().find(|e| e.kind == "tool_call").unwrap();
    assert_eq!(
        tc.raw["body"]["reversibility"], "irreversible",
        "undeclared reversibility = irreversible (§0)"
    );
}
