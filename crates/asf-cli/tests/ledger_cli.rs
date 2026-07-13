use asf_kernel::kernel::Fabric;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tree_bytes(root: &Path) -> std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(
        root: &Path,
        path: &Path,
        out: &mut std::collections::BTreeMap<PathBuf, Option<Vec<u8>>>,
    ) {
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        let metadata = std::fs::symlink_metadata(path).unwrap();
        if metadata.is_dir() {
            out.insert(relative, None);
            let mut children = std::fs::read_dir(path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                visit(root, &child, out);
            }
        } else {
            out.insert(relative, Some(std::fs::read(path).unwrap()));
        }
    }

    let mut out = std::collections::BTreeMap::new();
    visit(root, root, &mut out);
    out
}

fn initialized_home(root: &Path) -> PathBuf {
    let home = root.join("home");
    let vault = root.join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let stores = vec![StoreSpec {
        store: "fs:vault".into(),
        tier: 1,
        kind: StoreKind::Fs,
        path: vault,
    }];
    let mut fabric = Fabric::initialize(home.join("fabric"), stores).unwrap();
    let key = "01".repeat(32);
    let human = fabric.register_principal("human", "operator", &key, None).unwrap();
    let agent = fabric
        .register_principal("agent", "worker", &key, Some(&human))
        .unwrap();
    let channel = fabric
        .register_channel(&human, "local_session", b"tty:test", "local_session")
        .unwrap();
    let intent = fabric
        .capture_intent(
            &human,
            &channel,
            "local_session",
            "inspect the ledger",
            json!({}),
            None,
        )
        .unwrap();
    fabric
        .step_boundary(
            &human,
            &agent,
            &intent,
            json!({"bundle":"sha256:test", "skills":[]}),
        )
        .unwrap();
    home
}

fn ledger(home: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_asf"))
        .args(["ledger", "--home", home.to_str().unwrap()])
        .args(extra)
        .output()
        .expect("run asf ledger")
}

#[test]
fn ledger_shows_timestamps_and_full_event_detail() {
    let tmp = tempfile::tempdir().unwrap();
    let home = initialized_home(tmp.path());
    let conn = Connection::open(home.join("fabric/fabric.db")).unwrap();
    let (offset, at): (i64, String) = conn
        .query_row(
            "SELECT offset, at FROM events ORDER BY offset LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    let listing = ledger(&home, &[]);
    assert!(listing.status.success());
    let listing = String::from_utf8(listing.stdout).unwrap();
    assert!(listing.contains(&at), "ledger omitted event timestamp:\n{listing}");

    let offset_arg = offset.to_string();
    let detail = ledger(&home, &["--event", &offset_arg]);
    assert!(detail.status.success());
    let event: Value = serde_json::from_slice(&detail.stdout).unwrap();
    assert_eq!(event["at"], at);
    assert_eq!(event["kind"], "register");
    assert!(event["id"].as_str().unwrap().starts_with("evt:"));
    assert!(event["sig"].is_object(), "detail must include the signed record");
}

#[test]
fn ledger_rejects_uninitialized_home_and_unknown_event_without_creating_state() {
    let tmp = tempfile::tempdir().unwrap();
    let missing_home = tmp.path().join("missing-home");
    let missing = ledger(&missing_home, &[]);
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("fabric is not initialized"), "{stderr}");
    assert!(stderr.contains("fabric.db"), "{stderr}");
    assert!(
        !missing_home.exists(),
        "inspection of an uninitialized home must not create state"
    );

    let home = initialized_home(tmp.path());
    let before = std::fs::metadata(home.join("fabric/fabric.db")).unwrap().len();
    let unknown = ledger(&home, &["--event", "999999"]);
    assert!(!unknown.status.success());
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("offset 999999 not found")
    );
    let after = std::fs::metadata(home.join("fabric/fabric.db")).unwrap().len();
    assert_eq!(before, after, "failed detail lookup must not mutate the ledger");
}

#[test]
fn ledger_integrity_failure_is_loud_and_preserves_home() {
    for case in [
        "invalid-signature",
        "selector-mismatch",
        "malformed-raw",
        "broken-chain",
        "signed-sequence-reorder",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let home = initialized_home(tmp.path());
        let database = home.join("fabric/fabric.db");
        let conn = Connection::open(&database).unwrap();
        let target_offset: i64 = conn
            .query_row("SELECT MIN(offset) FROM events", [], |row| row.get(0))
            .unwrap();

        match case {
            "invalid-signature" => {
                let raw: String = conn
                    .query_row(
                        "SELECT raw FROM events WHERE offset = ?1",
                        [target_offset],
                        |row| row.get(0),
                    )
                    .unwrap();
                let mut event: Value = serde_json::from_str(&raw).unwrap();
                event["body"]["object_kind"] = json!("forged");
                conn.execute_batch("DROP TRIGGER events_append_only_u;").unwrap();
                conn.execute(
                    "UPDATE events SET raw = ?1 WHERE offset = ?2",
                    (serde_json::to_string(&event).unwrap(), target_offset),
                )
                .unwrap();
            }
            "selector-mismatch" => {
                conn.execute_batch("DROP TRIGGER events_append_only_u;").unwrap();
                conn.execute(
                    "UPDATE events SET kind = 'grant' WHERE offset = ?1",
                    [target_offset],
                )
                .unwrap();
            }
            "malformed-raw" => {
                conn.execute_batch("DROP TRIGGER events_append_only_u;").unwrap();
                conn.execute("UPDATE events SET raw = '{' WHERE offset = ?1", [target_offset])
                    .unwrap();
            }
            "broken-chain" => {
                let span: String = conn
                    .query_row(
                        "SELECT span FROM events GROUP BY span HAVING COUNT(*) >= 3 LIMIT 1",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                conn.execute_batch("DROP TRIGGER events_append_only_d;").unwrap();
                conn.execute("DELETE FROM events WHERE span = ?1 AND seq = 1", [&span])
                    .unwrap();
            }
            "signed-sequence-reorder" => {
                let span: String = conn
                    .query_row(
                        "SELECT span FROM events GROUP BY span HAVING COUNT(*) >= 2 LIMIT 1",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                let first: i64 = conn
                    .query_row(
                        "SELECT offset FROM events WHERE span = ?1 AND seq = 0",
                        [&span],
                        |row| row.get(0),
                    )
                    .unwrap();
                let second: i64 = conn
                    .query_row(
                        "SELECT offset FROM events WHERE span = ?1 AND seq = 1",
                        [&span],
                        |row| row.get(0),
                    )
                    .unwrap();
                conn.execute_batch("DROP TRIGGER events_append_only_u;").unwrap();
                conn.execute("UPDATE events SET offset = -1 WHERE offset = ?1", [first])
                    .unwrap();
                conn.execute("UPDATE events SET offset = ?1 WHERE offset = ?2", [first, second])
                    .unwrap();
                conn.execute("UPDATE events SET offset = ?1 WHERE offset = -1", [second])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        drop(conn);

        let before = tree_bytes(&home);
        let offset_arg = target_offset.to_string();
        let detail = ledger(&home, &["--event", &offset_arg]);
        assert!(!detail.status.success(), "{case}: anomalous detail must fail");
        assert!(
            !detail.stdout.is_empty(),
            "{case}: anomalous raw record must remain inspectable"
        );
        assert!(
            String::from_utf8_lossy(&detail.stderr).contains("ledger integrity failure"),
            "{case}: {}",
            String::from_utf8_lossy(&detail.stderr)
        );
        assert_eq!(tree_bytes(&home), before, "{case}: detail mutated protected state");

        let listing = ledger(&home, &[]);
        assert!(!listing.status.success(), "{case}: compromised listing must fail");
        assert!(
            String::from_utf8_lossy(&listing.stderr).contains("ledger integrity failure"),
            "{case}: {}",
            String::from_utf8_lossy(&listing.stderr)
        );
        assert!(
            !String::from_utf8_lossy(&listing.stdout).contains("note: drift"),
            "{case}: integrity must preflight before drift attribution"
        );
        assert_eq!(tree_bytes(&home), before, "{case}: listing mutated protected state");
    }
}
