//! Process-level smoke test: real `asf proxy` (broker daemon) fronting a
//! real `asf vault-server`, driven over stdio exactly as an MCP client
//! would, with approvals over the daemon's Unix socket (C2 in the flesh:
//! the MCP stream has no approval verb at all).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Proxy {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Proxy {
    fn start(home: &std::path::Path, vault: &std::path::Path) -> Self {
        let asf = env!("CARGO_BIN_EXE_asf");
        let mut child = Command::new(asf)
            .args([
                "proxy",
                "--home", home.to_str().unwrap(),
                "--vault", vault.to_str().unwrap(),
                "--downstream", asf, "vault-server", "--vault", vault.to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn proxy");
        let stdin = Some(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self { child, stdin, stdout, next_id: 1 }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let stdin = self.stdin.as_mut().expect("session still open");
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            if self.stdout.read_line(&mut line).unwrap() == 0 {
                panic!("proxy closed while awaiting response to {method}");
            }
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(line.trim()).unwrap();
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    fn call_tool(&mut self, name: &str, args: Value) -> Value {
        self.request("tools/call", json!({ "name": name, "arguments": args }))["result"].clone()
    }

    /// End the session cleanly: EOF on stdin triggers the promotion gate,
    /// then the daemon exits.
    fn finish(mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
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
    writeln!(stream, "{cmd}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(line.trim()).unwrap()
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

    // tools/list is filtered to the session grant: the downstream offers 4
    // tools, but note.delete (irreversible, outside action.allow, neither
    // dimension escalatable) is unreachable — so it is not advertised.
    let tools = p.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["note.read", "note.write", "note.move"], "filtered advertisement");

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
