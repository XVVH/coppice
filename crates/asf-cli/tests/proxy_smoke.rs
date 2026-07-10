//! Process-level smoke test: real `asf proxy` (broker daemon) fronting a
//! real `asf vault-server`, driven over stdio exactly as an MCP client
//! would, with approvals over the daemon's Unix socket (C2 in the flesh:
//! the MCP stream has no approval verb at all).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const PROCESS_TIMEOUT: Duration = Duration::from_secs(15);

enum ChildOutput {
    Line(String),
    Closed(String),
}

struct Proxy {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: mpsc::Receiver<ChildOutput>,
    stderr: Arc<Mutex<String>>,
    next_id: i64,
}

impl Proxy {
    fn start(home: &std::path::Path, vault: &std::path::Path) -> Self {
        let asf = env!("CARGO_BIN_EXE_asf");
        Self::start_with_downstream(
            home,
            vault,
            &[
                asf.to_string(),
                "vault-server".into(),
                "--vault".into(),
                vault.to_string_lossy().into_owned(),
            ],
        )
    }

    fn start_with_downstream(
        home: &std::path::Path,
        vault: &std::path::Path,
        downstream: &[String],
    ) -> Self {
        let asf = env!("CARGO_BIN_EXE_asf");
        let mut child = Command::new(asf)
            .args([
                "proxy",
                "--home", home.to_str().unwrap(),
                "--vault", vault.to_str().unwrap(),
                "--downstream",
            ])
            .args(downstream)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn proxy");
        let stdin = Some(child.stdin.take().unwrap());
        let child_stdout = child.stdout.take().unwrap();
        let (stdout_tx, stdout) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(child_stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = stdout_tx.send(ChildOutput::Closed("stdout reached EOF".into()));
                        break;
                    }
                    Ok(_) => {
                        if stdout_tx.send(ChildOutput::Line(line)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = stdout_tx.send(ChildOutput::Closed(format!(
                            "stdout read failed: {error}"
                        )));
                        break;
                    }
                }
            }
        });

        let child_stderr = child.stderr.take().unwrap();
        let stderr = Arc::new(Mutex::new(String::new()));
        let stderr_reader = stderr.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(child_stderr).lines() {
                let mut captured = stderr_reader.lock().expect("stderr capture lock");
                match line {
                    Ok(line) => {
                        captured.push_str(&line);
                        captured.push('\n');
                    }
                    Err(error) => {
                        captured.push_str(&format!("<stderr read failed: {error}>\n"));
                        break;
                    }
                }
            }
        });

        Self {
            child,
            stdin,
            stdout,
            stderr,
            next_id: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let stdin = self.stdin.as_mut().expect("session still open");
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        self.read_response(method, id)
    }

    fn read_response(&mut self, method: &str, id: i64) -> Value {
        loop {
            let line = match self.stdout.recv_timeout(PROCESS_TIMEOUT) {
                Ok(ChildOutput::Line(line)) => line,
                Ok(ChildOutput::Closed(reason)) => {
                    panic!(
                        "proxy closed while awaiting response to {method}: {reason}; {}",
                        self.diagnostics()
                    )
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!(
                        "proxy timed out after {PROCESS_TIMEOUT:?} awaiting response to {method}; {}",
                        self.diagnostics()
                    )
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!(
                        "proxy stdout reader disappeared awaiting response to {method}; {}",
                        self.diagnostics()
                    )
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(line.trim()).unwrap_or_else(|error| {
                panic!(
                    "proxy emitted invalid JSON while awaiting {method}: {error}; line={line:?}; {}",
                    self.diagnostics()
                )
            });
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    fn call_tool(&mut self, name: &str, args: Value) -> Value {
        self.request("tools/call", json!({ "name": name, "arguments": args }))["result"].clone()
    }

    /// Queue EOF while a tool call is still downstream, then wait for its
    /// response and the gate. The proxy must record the result before it can
    /// observe EOF and promote the branch.
    fn call_tool_then_eof(mut self, name: &str, args: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": args},
        });
        let mut stdin = self.stdin.take().expect("session still open");
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        drop(stdin);
        let response = self.read_response("tools/call+EOF", id);
        self.wait_for_exit("queued EOF promotion", true);
        response["result"].clone()
    }

    /// Issue one call and report whether a response arrived before the proxy
    /// disconnected. Used when SIGTERM is expected to win an in-flight race.
    fn call_tool_until_disconnect(mut self, name: &str, args: Value) -> bool {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": args},
        });
        let stdin = self.stdin.as_mut().expect("session still open");
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        loop {
            match self.stdout.recv_timeout(PROCESS_TIMEOUT) {
                Ok(ChildOutput::Line(line)) => {
                    let value: Value = serde_json::from_str(line.trim()).unwrap();
                    if value.get("id") == Some(&json!(id)) {
                        return true;
                    }
                }
                Ok(ChildOutput::Closed(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return false;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!(
                        "proxy did not disconnect within {PROCESS_TIMEOUT:?}; {}",
                        self.diagnostics()
                    );
                }
            }
        }
    }

    /// End the session cleanly: EOF on stdin triggers the promotion gate,
    /// then the daemon exits.
    fn finish(mut self) {
        drop(self.stdin.take());
        self.wait_for_exit("EOF promotion", true);
    }

    fn diagnostics(&mut self) -> String {
        let status = self
            .child
            .try_wait()
            .map(|status| format!("child_status={status:?}"))
            .unwrap_or_else(|error| format!("child_status_error={error}"));
        let stderr = self.stderr.lock().expect("stderr capture lock").clone();
        format!("{status}; stderr={stderr:?}")
    }

    fn wait_for_exit(&mut self, context: &str, require_success: bool) {
        let deadline = Instant::now() + PROCESS_TIMEOUT;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(status)) if status.success() || !require_success => return,
                Ok(Some(status)) => panic!(
                    "proxy exited unsuccessfully during {context}: {status}; {}",
                    self.diagnostics()
                ),
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(error) => panic!(
                    "could not wait for proxy during {context}: {error}; {}",
                    self.diagnostics()
                ),
            }
        }
        let diagnostics = self.diagnostics();
        let _ = self.child.kill();
        let _ = self.child.wait();
        panic!("proxy did not exit during {context} within {PROCESS_TIMEOUT:?}; {diagnostics}");
    }
}

impl Proxy {
    /// Kill without ceremony (SIGKILL): the promotion gate cannot run.
    fn sigkill(mut self) {
        let _ = self.child.kill();
        self.wait_for_exit("SIGKILL", false);
    }

    /// What a real MCP client does on shutdown (RF-9): a signal, not EOF.
    fn sigterm(mut self) {
        Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .expect("send SIGTERM");
        self.wait_for_exit("SIGTERM promotion", true);
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn approve_via_socket(home: &std::path::Path, cmd: Value) -> Value {
    // The socket appears once the daemon binds it; give it a moment.
    let sock = home.join("approvals.sock");
    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut stream = UnixStream::connect(&sock).expect("approval socket");
    stream.set_read_timeout(Some(PROCESS_TIMEOUT)).unwrap();
    stream.set_write_timeout(Some(PROCESS_TIMEOUT)).unwrap();
    writeln!(stream, "{cmd}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(line.trim()).unwrap()
}

fn wait_for_path(path: &std::path::Path, context: &str) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {context}: {}", path.display());
}

/// A deterministic downstream: mutate the rewritten branch, announce that
/// the mutation happened, then withhold the MCP result until released.
fn barrier_downstream(
    tmp: &tempfile::TempDir,
    vault: &std::path::Path,
) -> (Vec<String>, std::path::PathBuf, std::path::PathBuf) {
    let script = tmp.path().join("barrier-downstream.sh");
    let ready = tmp.path().join("downstream-mutated");
    let release = tmp.path().join("release-downstream");
    std::fs::write(
        &script,
        r#"vault="$1"
while IFS= read -r line; do
  case "$line" in
    *'"method":"tools/call"'*)
      printf '%s' 'barrier mutation' > "$vault/inflight.md"
      : > "$ASF_READY"
      while [ ! -e "$ASF_RELEASE" ]; do sleep 0.01; done
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"barrier complete"}],"isError":false}}'
      ;;
  esac
done
"#,
    )
    .unwrap();
    (
        vec![
            "/usr/bin/env".into(),
            format!("ASF_READY={}", ready.display()),
            format!("ASF_RELEASE={}", release.display()),
            "/bin/sh".into(),
            script.to_string_lossy().into_owned(),
            vault.to_string_lossy().into_owned(),
        ],
        ready,
        release,
    )
}

fn live_marker_count(home: &std::path::Path) -> i64 {
    let conn = rusqlite::Connection::open(home.join("fabric/fabric.db")).unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM meta WHERE key LIKE 'session_live:%'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

/// EOF may be queued while a downstream call is blocked, but the proxy's
/// single request loop must record the completed call before observing EOF
/// and entering the gate.
#[test]
fn eof_queued_during_tool_call_records_before_promotion() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let (downstream, ready, release) = barrier_downstream(&tmp, &vault);
    let proxy = Proxy::start_with_downstream(&home, &vault, &downstream);

    let call = std::thread::spawn(move || {
        proxy.call_tool_then_eof(
            "note.write",
            json!({"path":"inflight.md","content":"barrier mutation"}),
        )
    });
    wait_for_path(&ready, "downstream branch mutation");
    assert!(
        !vault.join("inflight.md").exists(),
        "the barrier mutation must still be branch-only"
    );
    std::fs::write(&release, b"go").unwrap();
    let result = call.join().unwrap();

    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(
        std::fs::read_to_string(vault.join("inflight.md")).unwrap(),
        "barrier mutation"
    );
    assert_eq!(promotion_count(&home), 1);
    assert_eq!(live_marker_count(&home), 0);
}

/// A catchable signal can arrive after the downstream mutated the branch but
/// before its result was traced. The gate must reject that branch tip, keep
/// trunk unchanged, and retain the live marker for explicit recovery rather
/// than silently promoting untraced state.
#[test]
fn sigterm_after_mutation_before_result_fails_honestly() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let (downstream, ready, release) = barrier_downstream(&tmp, &vault);
    let proxy = Proxy::start_with_downstream(&home, &vault, &downstream);
    let pid = proxy.child.id();

    let call = std::thread::spawn(move || {
        proxy.call_tool_until_disconnect(
            "note.write",
            json!({"path":"inflight.md","content":"barrier mutation"}),
        )
    });
    wait_for_path(&ready, "downstream branch mutation");
    assert!(!vault.join("inflight.md").exists());
    Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(!call.join().unwrap(), "SIGTERM should win before a response exists");
    std::fs::write(&release, b"let orphaned downstream exit").unwrap();

    assert!(!vault.join("inflight.md").exists(), "untraced state reached trunk");
    assert_eq!(promotion_count(&home), 0);
    assert_eq!(live_marker_count(&home), 1, "failed gate remains recoverable");

    let recovery = Command::new(env!("CARGO_BIN_EXE_asf"))
        .args([
            "recover",
            "--home",
            home.to_str().unwrap(),
            "--vault",
            vault.to_str().unwrap(),
        ])
        .output()
        .expect("run recovery");
    assert!(recovery.status.success());
    let recovery_stderr = String::from_utf8_lossy(&recovery.stderr);
    assert!(
        recovery_stderr.contains("untraced branch divergence")
            && recovery_stderr.contains("will retry next start"),
        "recovery must report the stranded untraced branch: {recovery_stderr}"
    );
    assert_eq!(promotion_count(&home), 0);
    assert_eq!(live_marker_count(&home), 1);

    let ledger = Command::new(env!("CARGO_BIN_EXE_asf"))
        .args(["ledger", "--home", home.to_str().unwrap()])
        .output()
        .expect("run ledger");
    assert!(ledger.status.success());
    assert!(
        String::from_utf8_lossy(&ledger.stdout)
            .contains("every live root is explained by the ledger"),
        "trunk accounting must remain honest"
    );
}

/// Approval and retry begin together on separate daemon surfaces. Whichever
/// acquires the broker first, exactly one retry must eventually execute under
/// the bounded exemption—never zero and never two.
#[test]
fn approval_racing_retry_preserves_one_bounded_use() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let mut proxy = Proxy::start(&home, &vault);
    init_session(&mut proxy);
    for i in 0..20 {
        let result = proxy.call_tool(
            "note.write",
            json!({"path":format!("n{i}.md"),"content":"x"}),
        );
        assert_eq!(result["isError"], false, "write {i}: {result}");
    }
    let args = json!({"path":"racing.md","content":"one bounded use"});
    assert_eq!(proxy.call_tool("note.write", args.clone())["isError"], true);
    let list = approve_via_socket(&home, json!({"cmd":"list"}));
    let id = list["escalations"][0]["id"].as_i64().unwrap();

    let barrier = Arc::new(std::sync::Barrier::new(3));
    let approval_barrier = barrier.clone();
    let approval_home = home.clone();
    let approval = std::thread::spawn(move || {
        approval_barrier.wait();
        approve_via_socket(
            &approval_home,
            json!({"cmd":"approve","id":id,"uses":1}),
        )
    });
    let call_barrier = barrier.clone();
    let first_args = args.clone();
    let retry = std::thread::spawn(move || {
        call_barrier.wait();
        let result = proxy.call_tool("note.write", first_args);
        (proxy, result)
    });
    barrier.wait();

    let approved = approval.join().unwrap();
    assert_eq!(approved["ok"], true, "{approved}");
    let (mut proxy, first_retry) = retry.join().unwrap();
    if first_retry["isError"] == true {
        let second_retry = proxy.call_tool("note.write", args);
        assert_eq!(
            second_retry["isError"], false,
            "a race-losing retry must leave the approved use available: {second_retry}"
        );
    }
    let extra = proxy.call_tool(
        "note.write",
        json!({"path":"extra.md","content":"must remain parked"}),
    );
    assert_eq!(extra["isError"], true, "the one-use approval widened: {extra}");
    proxy.finish();

    assert_eq!(
        std::fs::read_to_string(vault.join("racing.md")).unwrap(),
        "one bounded use"
    );
    assert!(!vault.join("extra.md").exists());
    assert_eq!(promotion_count(&home), 1);
}

fn init_session(p: &mut Proxy) {
    let init = p.request("initialize", json!({ "protocolVersion": "2025-03-26",
        "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }));
    assert_eq!(init["result"]["serverInfo"]["name"], "asf-vault-server");
}

fn promotion_count(home: &std::path::Path) -> i64 {
    let conn = rusqlite::Connection::open(home.join("fabric/fabric.db")).unwrap();
    conn.query_row("SELECT COUNT(*) FROM events WHERE kind = 'promotion'", [], |r| r.get(0))
        .unwrap()
}

fn drift_count(home: &std::path::Path) -> i64 {
    let conn = rusqlite::Connection::open(home.join("fabric/fabric.db")).unwrap();
    conn.query_row("SELECT COUNT(*) FROM events WHERE kind = 'drift'", [], |r| r.get(0))
        .unwrap()
}

fn pending_promotions(home: &std::path::Path) -> i64 {
    let conn = rusqlite::Connection::open(home.join("fabric/fabric.db")).unwrap();
    conn.query_row("SELECT COUNT(*) FROM promotions WHERE status = 'pending'", [], |r| r.get(0))
        .unwrap()
}

/// DF-P6: ordinary short sessions must form a clean chain without inventing
/// drift for untouched vault or memory state.
#[test]
fn multiple_clean_sessions_do_not_create_false_drift() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();

    let mut first = Proxy::start(&home, &vault);
    init_session(&mut first);
    let result = first.call_tool(
        "note.write",
        json!({"path":"first.md","content":"session one"}),
    );
    assert_eq!(result["isError"], false, "{result}");
    first.finish();

    let mut second = Proxy::start(&home, &vault);
    init_session(&mut second);
    let result = second.call_tool("note.read", json!({"path":"first.md"}));
    assert_eq!(result["content"][0]["text"], "session one");
    second.finish();

    assert_eq!(promotion_count(&home), 2, "each clean session gates once");
    assert_eq!(drift_count(&home), 0, "untouched state must not produce false drift");
}

/// DF-N3: every filesystem-writing surface rejects absolute/parent escapes;
/// the denial happens before any path outside the branch can be touched.
#[test]
fn filesystem_actions_cannot_escape_the_vault() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::write(vault.join("safe.md"), "safe").unwrap();
    let outside = tmp.path().join("outside.md");

    let mut proxy = Proxy::start(&home, &vault);
    init_session(&mut proxy);
    for (tool, args) in [
        (
            "note.write",
            json!({"path":"../outside.md","content":"escaped"}),
        ),
        (
            "note.edit",
            json!({"path":"../outside.md","old_string":"x","new_string":"y"}),
        ),
        (
            "note.move",
            json!({"src":"safe.md","dest":"../outside.md"}),
        ),
        (
            "note.read",
            json!({"path":outside.to_string_lossy()}),
        ),
        ("note.list", json!({"path":"../"})),
    ] {
        let result = proxy.call_tool(tool, args);
        assert_eq!(result["isError"], true, "{tool} accepted an escape: {result}");
        assert!(!outside.exists(), "{tool} wrote outside the vault");
    }
    proxy.finish();
    assert_eq!(std::fs::read_to_string(vault.join("safe.md")).unwrap(), "safe");
}

/// DF-N5: a denial is a terminal resolution of that batch, not a hidden
/// exemption. Retrying creates another parked request and executes nothing.
#[test]
fn denied_escalation_does_not_authorize_retry() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();

    let mut proxy = Proxy::start(&home, &vault);
    init_session(&mut proxy);
    for i in 0..20 {
        let result = proxy.call_tool(
            "note.write",
            json!({"path":format!("n{i}.md"),"content":"x"}),
        );
        assert_eq!(result["isError"], false, "write {i}: {result}");
    }
    let blocked = json!({"path":"blocked.md","content":"must not land"});
    let result = proxy.call_tool("note.write", blocked.clone());
    assert_eq!(result["isError"], true, "over-budget write should park");

    let list = approve_via_socket(&home, json!({"cmd":"list"}));
    let id = list["escalations"][0]["id"].as_i64().unwrap();
    let denied = approve_via_socket(&home, json!({"cmd":"deny","id":id}));
    assert_eq!(denied["ok"], true, "{denied}");

    let retry = proxy.call_tool("note.write", blocked);
    assert_eq!(retry["isError"], true, "denial must not grant an exemption");
    assert!(
        retry["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("parked")),
        "retry should create a fresh visible escalation: {retry}"
    );
    proxy.finish();
    assert!(!vault.join("blocked.md").exists());
}

/// RF-9, the crash-safe layer: a SIGKILLed proxy strands its session; the
/// next bootstrap finds the live-marker, gates the branch, and the write
/// lands on trunk before the new session takes its snapshot.
#[test]
fn sigkilled_session_is_recovered_by_next_bootstrap() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();

    let mut p = Proxy::start(&home, &vault);
    init_session(&mut p);
    let r = p.call_tool("note.write", json!({ "path": "stranded.md", "content": "survives" }));
    assert_eq!(r["isError"], false, "{r}");
    p.sigkill();

    assert!(!vault.join("stranded.md").exists(), "SIGKILL must strand the branch");
    assert_eq!(promotion_count(&home), 0, "no gate ran");

    // Next session's bootstrap recovers the strand. The initialize
    // round-trip proves bootstrap (and therefore recovery) completed.
    let mut p2 = Proxy::start(&home, &vault);
    init_session(&mut p2);
    assert_eq!(
        std::fs::read_to_string(vault.join("stranded.md")).unwrap(),
        "survives",
        "recovery promoted the stranded session to trunk"
    );
    assert_eq!(promotion_count(&home), 1);

    // The recovered state is the new session's base — visible through it.
    let r = p2.call_tool("note.read", json!({ "path": "stranded.md" }));
    assert_eq!(r["content"][0]["text"], "survives");
    p2.finish();

    // Recovery is once-only: the second session's own promotion is the
    // only new gate run (no double-promotion of the stranded branch).
    assert_eq!(promotion_count(&home), 2);
}

/// RF-9, the polish layer: SIGTERM (what MCP clients actually send) runs
/// the gate before exit — work lands without waiting for the next session.
#[test]
fn sigterm_runs_the_gate_before_exit() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();

    let mut p = Proxy::start(&home, &vault);
    init_session(&mut p);
    let r = p.call_tool("note.write", json!({ "path": "graceful.md", "content": "landed" }));
    assert_eq!(r["isError"], false, "{r}");
    p.sigterm();

    assert_eq!(
        std::fs::read_to_string(vault.join("graceful.md")).unwrap(),
        "landed",
        "SIGTERM path promoted before exit"
    );
    assert_eq!(promotion_count(&home), 1);

    // And the marker is cleared: the next bootstrap has nothing to recover.
    let mut p2 = Proxy::start(&home, &vault);
    init_session(&mut p2);
    assert_eq!(promotion_count(&home), 1, "nothing re-gated");
    p2.finish();
}

/// M8 (A20, resolves SI-20): attribution is timing-independent. The same
/// hand edit gets one drift event with A12 attribution and an A13 op
/// summary whether it happens mid-session (attributed at the gate, BEFORE
/// the merge consumes live trunk) or between sessions (attributed at the
/// next boundary) — and exactly one per divergence window.
#[test]
fn m8_attribution_is_timing_independent() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();

    // Timing 1: mid-session, branch-untouched path.
    let mut p = Proxy::start(&home, &vault);
    init_session(&mut p);
    let r = p.call_tool("note.write", json!({ "path": "agent-note.md", "content": "brokered" }));
    assert_eq!(r["isError"], false, "{r}");
    std::fs::write(vault.join("hand-note.md"), "out-of-band, mid-session").unwrap();
    p.sigterm();

    // Both survive on trunk, and the promotion still auto-applies…
    assert_eq!(std::fs::read_to_string(vault.join("agent-note.md")).unwrap(), "brokered");
    assert_eq!(
        std::fs::read_to_string(vault.join("hand-note.md")).unwrap(),
        "out-of-band, mid-session"
    );
    assert_eq!(promotion_count(&home), 1, "clean additive run auto-promoted");
    // …but the hand edit was attributed FIRST: one drift event, before the
    // promotion in ledger order, naming the path.
    assert_eq!(drift_count(&home), 1, "M8: mid-session edit must be attributed");
    let conn = rusqlite::Connection::open(home.join("fabric/fabric.db")).unwrap();
    let (drift_off, drift_raw): (i64, String) = conn
        .query_row(
            "SELECT offset, raw FROM events WHERE kind = 'drift'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let promo_off: i64 = conn
        .query_row("SELECT offset FROM events WHERE kind = 'promotion'", [], |r| r.get(0))
        .unwrap();
    assert!(drift_off < promo_off, "drift must precede the merge that consumes it");
    let body = serde_json::from_str::<Value>(&drift_raw).unwrap()["body"].clone();
    assert_eq!(body["attribution"], "human_local");
    assert!(
        body["ops"].to_string().contains("hand-note.md"),
        "drift narrative must name its paths: {body}"
    );
    drop(conn);

    // Timing 2: between sessions — identical act, equivalent narrative.
    std::fs::write(vault.join("hand-note-2.md"), "out-of-band, between sessions").unwrap();
    let mut p2 = Proxy::start(&home, &vault);
    init_session(&mut p2);
    assert_eq!(drift_count(&home), 2, "M8: exactly one drift per divergence window");
    let conn = rusqlite::Connection::open(home.join("fabric/fabric.db")).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT raw FROM events WHERE kind = 'drift' ORDER BY offset DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let body = serde_json::from_str::<Value>(&raw).unwrap()["body"].clone();
    assert_eq!(body["attribution"], "human_local");
    assert!(
        body["ops"].to_string().contains("hand-note-2.md"),
        "between-session narrative must be equivalent: {body}"
    );
    drop(conn);
    p2.finish();

    // No re-attribution: the windows closed, the count stands.
    assert_eq!(drift_count(&home), 2);
}

/// The conflict timing of M8: a mid-session hand edit to a path the
/// branch DID touch is attributed at the gate (drift before the plan is
/// computed) AND parks as a both-changed conflict — the gate never
/// auto-resolves in the agent's favor, and trunk keeps the human's
/// version pending approval.
#[test]
fn si20_midsession_edit_to_branch_touched_path_parks_as_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::write(vault.join("note.md"), "base").unwrap();

    let mut p = Proxy::start(&home, &vault);
    init_session(&mut p);
    let r = p.call_tool("note.write", json!({ "path": "note.md", "content": "agent version" }));
    assert_eq!(r["isError"], false, "{r}");

    std::fs::write(vault.join("note.md"), "human version").unwrap();

    p.sigterm();

    assert_eq!(
        std::fs::read_to_string(vault.join("note.md")).unwrap(),
        "human version",
        "trunk-wins: the human's edit stands while the conflict awaits approval"
    );
    assert_eq!(promotion_count(&home), 0, "conflicted run must not auto-promote");
    assert_eq!(pending_promotions(&home), 1, "parked for the C2 surface");
    assert_eq!(drift_count(&home), 1, "M8: the conflict timing is attributed too");

    // RF-12: the parked promotion must be legible in the ledger — the
    // promotion id and the ops/conflict preview, not "ESCALATE #null".
    let out = Command::new(env!("CARGO_BIN_EXE_asf"))
        .args(["ledger", "--home", home.to_str().unwrap()])
        .output()
        .expect("run asf ledger");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("PARKED promotion #1") && text.contains("conflict"),
        "parked promotion illegible in ledger:\n{text}"
    );
    assert!(!text.contains("#null"), "RF-12 regression:\n{text}");

    // And resolving it must be legible too — the approval line carries the
    // promotion id, not "#null" (the RF-12 sibling).
    let out = Command::new(env!("CARGO_BIN_EXE_asf"))
        .args(["approve", "--home", home.to_str().unwrap(), "reject", "1"])
        .output()
        .expect("run asf approve reject");
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"ok\":true"));
    let out = Command::new(env!("CARGO_BIN_EXE_asf"))
        .args(["ledger", "--home", home.to_str().unwrap()])
        .output()
        .expect("run asf ledger");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("APPROVAL promotion #1 denied"),
        "promotion resolution illegible:\n{text}"
    );
    assert!(!text.contains("#null"), "RF-12 sibling regression:\n{text}");
}

/// note.edit (@1.2): targeted single-occurrence replacement — the
/// whole-document rewrite gap from DF-P2. Metered as a write.
#[test]
fn note_edit_replaces_one_unique_occurrence_and_meters_as_write() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::write(vault.join("links.md"), "see [[old-note]] and [[other]] and [[old-note]]").unwrap();

    let mut p = Proxy::start(&home, &vault);
    init_session(&mut p);

    // Ambiguous target → error, nothing changed.
    let r = p.call_tool("note.edit", json!({ "path": "links.md",
        "old_string": "[[old-note]]", "new_string": "[[new-note]]" }));
    assert_eq!(r["isError"], true, "ambiguous edit must be rejected: {r}");
    assert!(r["content"][0]["text"].as_str().unwrap().contains("2 times"));

    // Absent target → error.
    let r = p.call_tool("note.edit", json!({ "path": "links.md",
        "old_string": "[[missing]]", "new_string": "x" }));
    assert_eq!(r["isError"], true);

    // Unique target → replaced, once, visible via read-your-writes.
    let r = p.call_tool("note.edit", json!({ "path": "links.md",
        "old_string": "and [[other]]", "new_string": "and [[renamed]]" }));
    assert_eq!(r["isError"], false, "{r}");
    let r = p.call_tool("note.read", json!({ "path": "links.md" }));
    assert_eq!(r["content"][0]["text"], "see [[old-note]] and [[renamed]] and [[old-note]]");

    // Edits consume the write budget — including the two the DOWNSTREAM
    // rejected above: the broker allowed them, and consumption is
    // only-on-Allow (RF-3). 3 edit allows + 17 writes = 20; the 21st
    // write-class call parks. A read-classed edit would sail through.
    for i in 0..17 {
        let r = p.call_tool("note.write", json!({ "path": format!("n{i}.md"), "content": "x" }));
        assert_eq!(r["isError"], false, "write {i}: {r}");
    }
    let r = p.call_tool("note.edit", json!({ "path": "links.md",
        "old_string": "[[renamed]]", "new_string": "[[blocked]]" }));
    assert_eq!(r["isError"], true, "21st write-class call must escalate: {r}");
    assert!(r["content"][0]["text"].as_str().unwrap().contains("parked"));
    p.finish();
}

/// note.list (@1.1): enumeration is the prerequisite for every
/// vault-maintenance workflow — without it the agent can only touch paths
/// it is told about. Read-class: unmetered, no path scoping.
#[test]
fn note_list_enumerates_without_consuming_write_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(vault.join("patterns")).unwrap();
    std::fs::write(vault.join("patterns/a.md"), "A").unwrap();
    std::fs::write(vault.join("patterns/b.md"), "B").unwrap();
    std::fs::write(vault.join("index.md"), "root note").unwrap();

    let mut p = Proxy::start(&home, &vault);
    init_session(&mut p);

    // Root listing: folders marked, sorted.
    let r = p.call_tool("note.list", json!({}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["content"][0]["text"], "index.md\npatterns/");

    // Subfolder listing — the exact workflow that was dead without this.
    let r = p.call_tool("note.list", json!({ "path": "patterns" }));
    assert_eq!(r["content"][0]["text"], "a.md\nb.md");

    // Escape attempts still rejected by the downstream.
    let r = p.call_tool("note.list", json!({ "path": "../" }));
    assert_eq!(r["isError"], true);

    // Listing is read-class: the full write budget (20) remains spendable.
    for i in 0..20 {
        let r = p.call_tool("note.write", json!({ "path": format!("n{i}.md"), "content": "x" }));
        assert_eq!(r["isError"], false, "write {i} blocked — did list consume budget? {r}");
    }
    p.finish();
}

#[test]
fn proxied_session_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(vault.join("inbox")).unwrap();
    std::fs::write(vault.join("inbox/todo.md"), "- tidy\n").unwrap();

    let mut p = Proxy::start(&home, &vault);

    // MCP handshake passes through to the downstream server untouched.
    let init = p.request("initialize", json!({ "protocolVersion": "2025-03-26",
        "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }));
    assert_eq!(init["result"]["serverInfo"]["name"], "asf-vault-server");

    // tools/list is filtered to the session grant: the downstream offers 5
    // tools, but note.delete (irreversible, outside action.allow, neither
    // dimension escalatable) is unreachable — so it is not advertised.
    let tools = p.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["note.list", "note.read", "note.write", "note.edit", "note.move"],
        "filtered advertisement"
    );

    // Allowed write goes through — landing on the session BRANCH, not
    // trunk: the live vault only changes at promotion.
    let r = p.call_tool("note.write", json!({ "path": "inbox/hello.md", "content": "hi" }));
    assert_eq!(r["isError"], false, "{r}");
    assert!(
        !vault.join("inbox/hello.md").exists(),
        "trunk must not change before promotion"
    );

    // Even though note.delete was never advertised, enforcement does not
    // depend on the advertisement: calling it anyway is denied at the
    // broker; the file survives; the agent sees a tool-error, not silence.
    let r = p.call_tool("note.delete", json!({ "path": "inbox/todo.md" }));
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("denied"));
    assert!(vault.join("inbox/todo.md").exists());

    // Exhaust the write budget (20/run; one used already) → escalation.
    for i in 1..20 {
        let r = p.call_tool("note.write", json!({ "path": format!("inbox/n{i}.md"), "content": "x" }));
        assert_eq!(r["isError"], false, "write {i}: {r}");
    }
    let r = p.call_tool("note.write", json!({ "path": "inbox/overflow.md", "content": "x" }));
    assert_eq!(r["isError"], true);
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("parked") && text.contains("escalation"), "{text}");
    assert!(!vault.join("inbox/overflow.md").exists());

    // C2: the approval lives on the daemon's socket, not in the MCP stream.
    let list = approve_via_socket(&home, json!({ "cmd": "list" }));
    let esc_id = list["escalations"][0]["id"].as_i64().expect("one pending batch");
    let ok = approve_via_socket(&home, json!({ "cmd": "approve", "id": esc_id, "uses": 1 }));
    assert_eq!(ok["ok"], true, "{ok}");

    // Retry now passes under the approved exemption — still branch-only.
    let r = p.call_tool("note.write", json!({ "path": "inbox/overflow.md", "content": "x" }));
    assert_eq!(r["isError"], false, "{r}");
    assert!(!vault.join("inbox/overflow.md").exists(), "still pre-promotion");

    // And there is no in-band approval verb: an invented method falls
    // through to the downstream, which rejects it.
    let resp = p.request("asf/approve", json!({ "id": esc_id }));
    assert!(resp.get("error").is_some(), "in-band approval must not exist: {resp}");

    // Session end (EOF) → promotion gate. This session is pure adds with
    // no trunk divergence → zero-authorship policy auto-promotes.
    p.finish();
    assert_eq!(
        std::fs::read_to_string(vault.join("inbox/hello.md")).unwrap(),
        "hi",
        "promotion landed the session's writes on trunk"
    );
    assert!(vault.join("inbox/overflow.md").exists());
    assert!(vault.join("inbox/todo.md").exists(), "denied delete never happened");

    // The ledger recorded the whole session including the promotion.
    let conn = rusqlite::Connection::open(home.join("fabric/fabric.db")).unwrap();
    let kinds: Vec<String> = {
        let mut stmt = conn.prepare("SELECT DISTINCT kind FROM events").unwrap();
        let rows = stmt.query_map([], |r| r.get(0)).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    };
    for k in ["register", "intent", "snapshot", "grant", "tool_call", "verdict",
              "escalation", "approval", "promotion"] {
        assert!(kinds.contains(&k.to_string()), "ledger missing {k} events (has {kinds:?})");
    }
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM events WHERE kind = 'tool_call'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 21, "20 budgeted writes + 1 exempted write");
}
