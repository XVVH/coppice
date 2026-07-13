#![cfg(unix)]

//! Deterministic cross-process serialization tests. These use the integration
//! test executable itself as a helper process so Linux and macOS exercise the
//! same `Fabric::gate_lock` implementation used by promotion and revert.

use asf_kernel::kernel::Fabric;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(15);

fn store(vault: PathBuf) -> Vec<StoreSpec> {
    vec![StoreSpec {
        store: "fs:vault".into(),
        tier: 1,
        kind: StoreKind::Fs,
        path: vault,
    }]
}

fn wait_for_path(path: &Path, context: &str) {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {context}: {}", path.display());
}

fn wait_for_child(mut child: Child, context: &str) {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return,
            Ok(Some(status)) => panic!("{context} exited unsuccessfully: {status}"),
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => panic!("could not wait for {context}: {error}"),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("timed out waiting for {context}");
}

/// Helper process selected explicitly by `gate_lock_blocks_another_process`.
/// In an ordinary test run the environment is absent and this is a no-op.
#[test]
fn gate_lock_child_helper() {
    let Ok(home) = std::env::var("ASF_GATE_HOME") else {
        return;
    };
    let vault = PathBuf::from(std::env::var("ASF_GATE_VAULT").unwrap());
    let started = PathBuf::from(std::env::var("ASF_GATE_STARTED").unwrap());
    let acquired = PathBuf::from(std::env::var("ASF_GATE_ACQUIRED").unwrap());
    let release = PathBuf::from(std::env::var("ASF_GATE_RELEASE").unwrap());
    let fabric = Fabric::open_existing_with_stores(home, store(vault)).unwrap();
    fs::write(&started, b"waiting").unwrap();
    let _guard = fabric.gate_lock().unwrap();
    fs::write(&acquired, b"acquired").unwrap();
    wait_for_path(&release, "parent release signal");
}

#[test]
fn gate_lock_blocks_another_process() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("fabric");
    let vault = tmp.path().join("vault");
    let started = tmp.path().join("child-started");
    let acquired = tmp.path().join("child-acquired");
    let release = tmp.path().join("release-child");
    fs::create_dir_all(&vault).unwrap();
    let fabric = Fabric::initialize(&home, store(vault.clone())).unwrap();
    let guard = fabric.gate_lock().unwrap();

    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "gate_lock_child_helper",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ASF_GATE_HOME", &home)
        .env("ASF_GATE_VAULT", &vault)
        .env("ASF_GATE_STARTED", &started)
        .env("ASF_GATE_ACQUIRED", &acquired)
        .env("ASF_GATE_RELEASE", &release)
        .spawn()
        .unwrap();

    wait_for_path(&started, "child reaching gate lock");
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        !acquired.exists(),
        "a second process entered the fabric gate while the first held it"
    );

    drop(guard);
    wait_for_path(&acquired, "child acquiring released gate lock");
    fs::write(&release, b"release").unwrap();
    wait_for_child(child, "gate-lock helper");
}
