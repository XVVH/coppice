//! `asf` — Agent State Fabric driver.
//!
//! - `asf demo [dir]`         — milestone 1: kernel round-trip, narrated
//! - `asf broker-demo [dir]`  — milestone 2: broker pipeline, narrated
//! - `asf proxy --home H --vault V --downstream CMD [ARGS…]` — the MCP
//!   proxy daemon (brief §5.3) with its C2 approval socket
//! - `asf vault-server --vault V` — toy downstream MCP server
//! - `asf approve --home H list|approve <id> [--uses N]|deny <id>` — the
//!   human side of the C2 surface (separate terminal, never the agent)
//! - `asf recover --home H --vault V [man:… …]` — gate sessions a dead
//!   proxy left stranded (RF-9); explicit ids for pre-marker branches

mod broker_demo;
mod mcp;
mod proxy;
mod vault_server;

use anyhow::{bail, Context, Result};
use asf_kernel::kernel::Fabric;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use asf_kernel::trace;
use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("demo") | Some("broker-demo") => {
            let which = args[1].clone();
            let keep = args.get(2).cloned();
            let tmp;
            let dir = match &keep {
                Some(d) => Path::new(d).to_path_buf(),
                None => {
                    tmp = tempfile::tempdir()?;
                    tmp.path().to_path_buf()
                }
            };
            if which == "demo" {
                demo(&dir)?;
            } else {
                broker_demo::run(&dir)?;
            }
            if let Some(d) = keep {
                println!("\nfabric home kept at: {d}");
            }
            Ok(())
        }
        Some("vault-server") => {
            let vault = flag(&args, "--vault").context("vault-server needs --vault <dir>")?;
            vault_server::run(PathBuf::from(vault))
        }
        Some("proxy") => {
            let home = flag(&args, "--home").context("proxy needs --home <dir>")?;
            let vault = flag(&args, "--vault").context("proxy needs --vault <dir>")?;
            let dpos = args
                .iter()
                .position(|a| a == "--downstream")
                .context("proxy needs --downstream <cmd> [args…]")?;
            let downstream: Vec<String> = args[dpos + 1..].to_vec();
            if downstream.is_empty() {
                bail!("--downstream needs a command");
            }
            proxy::run(PathBuf::from(home), PathBuf::from(vault), downstream)
        }
        Some("approve") => {
            let home = flag(&args, "--home").context("approve needs --home <dir>")?;
            let sub = args.get(args.iter().position(|a| a == "--home").unwrap() + 2)
                .map(String::as_str)
                .context("approve needs list|approve <id>|deny <id>|promotions|promote <id>|reject <id>")?;
            let id = args
                .iter()
                .filter_map(|a| a.parse::<i64>().ok())
                .next();
            let uses = flag(&args, "--uses").and_then(|u| u.parse().ok()).unwrap_or(1);
            proxy::approve_cli(Path::new(&home), sub, id, uses)
        }
        Some("recover") => {
            let home = flag(&args, "--home").context("recover needs --home <dir>")?;
            let vault = flag(&args, "--vault").context("recover needs --vault <dir>")?;
            let manifests: Vec<String> = args
                .iter()
                .filter(|a| a.starts_with("man:"))
                .cloned()
                .collect();
            proxy::recover_cli(Path::new(&home), Path::new(&vault), &manifests)
        }
        Some("revert") => {
            let home = flag(&args, "--home").context("revert needs --home <dir>")?;
            let manifest = args
                .iter()
                .find(|a| a.starts_with("man:"))
                .context("revert needs a manifest id (man:…)")?
                .clone();
            let mut fabric = Fabric::open_existing(Path::new(&home).join("fabric"))?;
            fabric.revert_to(&manifest)?;
            println!("reverted all roots to {manifest}");
            Ok(())
        }
        Some("ledger") => {
            let home = flag(&args, "--home").context("ledger needs --home <dir>")?;
            let mut fabric = Fabric::open_existing(Path::new(&home).join("fabric"))?;
            // Attribute any out-of-band edits first so the accounting below
            // is against a current picture, not a stale one.
            for d in fabric.check_drift()? {
                println!(
                    "note: drift in {} attributed {} ({})",
                    d.store, d.attribution, d.event_id
                );
            }
            let explanation = fabric.explain()?;
            for l in &explanation.lines {
                println!("[{:>4}] {:<11} {}", l.offset, l.kind, l.line);
            }
            if explanation.unexplained.is_empty() {
                println!("\nevery live root is explained by the ledger.");
                Ok(())
            } else {
                bail!("UNEXPLAINED STATE: {:?}", explanation.unexplained)
            }
        }
        Some("stats") => {
            let home = flag(&args, "--home").context("stats needs --home <dir>")?;
            stats(Path::new(&home))
        }
        _ => {
            eprintln!(
                "usage:\n  asf demo [dir]\n  asf broker-demo [dir]\n  asf vault-server --vault <dir>\n  asf proxy --home <dir> --vault <dir> --downstream <cmd> [args…]\n  asf approve --home <dir> list|approve <id> [--uses N]|deny <id>|promotions|promote <id>|reject <id>\n  asf recover --home <dir> --vault <dir> [man:… …]\n  asf revert --home <dir> <man:…>\n  asf ledger --home <dir>\n  asf stats --home <dir>"
            );
            std::process::exit(2);
        }
    }
}

/// The tripwire numbers (ADR 0002) plus general substrate health.
fn stats(home: &Path) -> Result<()> {
    let fabric_dir = home.join("fabric");
    let conn = Connection::open(fabric_dir.join("fabric.db"))?;
    println!("── events ──");
    let mut stmt = conn.prepare("SELECT kind, COUNT(*) FROM events GROUP BY kind ORDER BY 2 DESC")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut total = 0;
    for row in rows {
        let (kind, n) = row?;
        total += n;
        println!("  {kind:<12} {n}");
    }
    let spans: i64 = conn.query_row("SELECT COUNT(DISTINCT span) FROM events", [], |r| r.get(0))?;
    println!("  total        {total} across {spans} span(s)");

    println!("── objects ──");
    let mut stmt = conn.prepare("SELECT kind, COUNT(*) FROM objects GROUP BY kind ORDER BY 2 DESC")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    for row in rows {
        let (kind, n) = row?;
        println!("  {kind:<12} {n}");
    }

    println!("── payloads ──");
    let (n, bytes): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM payloads",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let tomb: i64 = conn.query_row("SELECT COUNT(*) FROM tombstones", [], |r| r.get(0))?;
    println!("  {n} payload(s), {bytes} plaintext byte(s), {tomb} shredded");

    println!("── storage (ADR 0002 tripwires) ──");
    let mut cas_blobs = 0u64;
    let mut cas_bytes = 0u64;
    for entry in walk(&fabric_dir.join("cas")) {
        cas_blobs += 1;
        cas_bytes += entry;
    }
    let db_bytes = fs::metadata(fabric_dir.join("fabric.db")).map(|m| m.len()).unwrap_or(0);
    println!("  CAS: {cas_blobs} blob(s), {} KiB", cas_bytes / 1024);
    println!("  fabric.db: {} KiB", db_bytes / 1024);
    let branches = fabric_dir
        .join("branches")
        .read_dir()
        .map(|d| d.count())
        .unwrap_or(0);
    println!("  branches on disk: {branches}");
    Ok(())
}

fn walk(dir: &Path) -> Vec<u64> {
    let mut sizes = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(m) = e.metadata() {
                sizes.push(m.len());
            }
        }
    }
    sizes
}

fn banner(s: &str) {
    println!("\n━━━ {s} ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
}

fn placeholder_key(b: u8) -> String {
    (0..32).map(|_| format!("{b:02x}")).collect()
}

fn demo(dir: &Path) -> Result<()> {
    // -- world setup ------------------------------------------------------
    banner("setup: vault + memory db");
    let vault = dir.join("vault");
    fs::create_dir_all(vault.join("inbox"))?;
    fs::write(vault.join("inbox/todo.md"), "- water the plants\n")?;
    fs::write(vault.join("index.md"), "# Vault\n")?;
    let memory_db = dir.join("memory.db");
    {
        let conn = Connection::open(&memory_db)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (id INTEGER PRIMARY KEY, fact TEXT NOT NULL);
             DELETE FROM memories;
             INSERT INTO memories (fact) VALUES ('user prefers markdown');",
        )?;
    }
    println!("vault: {} | memory: {}", vault.display(), memory_db.display());

    let stores = vec![
        StoreSpec { store: "fs:vault".into(), tier: 1, kind: StoreKind::Fs, path: vault.clone() },
        StoreSpec { store: "db:memory".into(), tier: 1, kind: StoreKind::Sqlite, path: memory_db.clone() },
    ];
    let mut fabric = Fabric::open(dir.join("fabric"), stores)?;

    // -- principals, channel, intent ---------------------------------------
    banner("register principals + local_session channel, capture intent");
    let human = fabric.register_principal("human", "josh", &placeholder_key(1), None)?;
    let agent = fabric.register_principal("agent", "hermes-vault", &placeholder_key(2), Some(&human))?;
    let chan = fabric.register_channel(&human, "local_session", b"tty:local", "local_session")?;
    let intent = fabric.capture_intent(
        &human, &chan, "local_session",
        "tidy the vault inbox and remember what you filed",
        json!({}), None,
    )?;
    println!("human  {human}\nagent  {agent}\nintent {intent}");

    // -- run 1 --------------------------------------------------------------
    banner("step boundary → manifest 1 (both roots captured)");
    let behavior = json!({
        "bundle": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        "skills": [{ "skill": "vault-filing", "version": "sha256:aaaa", "domains": ["files.vault"] }],
    });
    let s1 = fabric.step_boundary(&human, &agent, &intent, behavior.clone())?;
    println!("manifest {}\nspan     {}", s1.manifest, s1.span);

    banner("agent run: file a note, remember it — every write traced");
    fs::write(vault.join("inbox/todo.md"), "- water the plants\n- filed!\n")?;
    fs::create_dir_all(vault.join("MOCs"))?;
    fs::write(vault.join("MOCs/plants.md"), "# Plants MOC\n")?;
    {
        let conn = Connection::open(&memory_db)?;
        conn.execute("INSERT INTO memories (fact) VALUES ('filed plants note under MOCs')", [])?;
    }
    fabric.record_tool_call(
        "tool:vault@1.0", "note.file",
        br#"{"src":"inbox/todo.md","dest":"MOCs/plants.md"}"#, br#"{"ok":true}"#,
        json!({ "paths": ["inbox/todo.md","MOCs/plants.md"] }),
        json!([]),
        Some("reversible"),
    )?;
    println!("vault + memory mutated; tool_call recorded with state_root_after");

    banner("step boundary → manifest 2 (child of manifest 1; no drift)");
    let s2 = fabric.step_boundary(&human, &agent, &intent, behavior)?;
    println!(
        "manifest {} (parent {}), drift: {}",
        s2.manifest,
        trace::get_object(&fabric.conn, &s2.manifest)?["parent"].as_str().unwrap_or("?"),
        s2.drift.len(),
    );

    // -- out-of-band edit ----------------------------------------------------
    banner("out-of-band: human edits index.md directly (no agent, no trace)");
    fs::write(vault.join("index.md"), "# Vault\nhand edit, no agent involved\n")?;
    let drift = fabric.check_drift()?;
    for d in &drift {
        println!(
            "drift in {}: expected {}.. observed {}.. → attributed {} (quiet, single-human default)",
            d.store, &d.expected_root[7..19], &d.observed_root[7..19], d.attribution
        );
    }
    if drift.len() != 1 {
        bail!("expected exactly one drift report, got {}", drift.len());
    }

    // -- revert ---------------------------------------------------------------
    banner("revert to manifest 1 — ALL roots restored together");
    fabric.revert_to(&s1.manifest)?;
    let todo = fs::read_to_string(vault.join("inbox/todo.md"))?;
    let index = fs::read_to_string(vault.join("index.md"))?;
    let facts: Vec<String> = {
        let conn = Connection::open(&memory_db)?;
        let mut stmt = conn.prepare("SELECT fact FROM memories ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    println!("inbox/todo.md: {todo:?}");
    println!("index.md:      {index:?}  (hand edit rolled back too)");
    println!("memories:      {facts:?}  (memory coherent with vault)");
    if todo != "- water the plants\n" || facts.len() != 1 || vault.join("MOCs").exists() {
        bail!("revert did not restore expected state");
    }
    if !fabric.check_drift()?.is_empty() {
        bail!("post-revert drift should be empty");
    }

    // -- verification -----------------------------------------------------------
    banner("verify: hash chains, signatures, ledger accounting");
    for (span, n) in fabric.verify_all_spans()? {
        println!("span {}… — {} event(s), chain + signatures OK", &span[5..17], n);
    }
    let explanation = fabric.explain()?;
    println!("\nTHE LEDGER:");
    for l in &explanation.lines {
        println!("  [{:>3}] {:<10} {}", l.offset, l.kind, l.line);
    }
    if !explanation.unexplained.is_empty() {
        bail!("UNEXPLAINED STATE: {:?}", explanation.unexplained);
    }
    println!("\nevery live root is explained by the ledger — nothing unaccounted for.");

    // -- crypto-shredding ----------------------------------------------------
    banner("crypto-shred the intent text — substance gone, structure intact");
    let intent_obj = trace::get_object(&fabric.conn, &intent)?;
    let text_ref: asf_kernel::payload::PayloadRef =
        serde_json::from_value(intent_obj["text"].clone()).context("intent text ref")?;
    fabric.shred_payload(&text_ref.hash, "demo_ttl")?;
    match fabric.get_payload(&text_ref) {
        Err(e) => println!("payload now resolves to: {e}"),
        Ok(_) => bail!("shredded payload still readable!"),
    }
    fabric.verify_all_spans()?;
    println!("chains still verify: the ledger records THAT it forgot, never what.");

    banner("kernel round-trip complete");
    Ok(())
}
