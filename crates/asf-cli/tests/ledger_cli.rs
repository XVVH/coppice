use asf_kernel::kernel::Fabric;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
