//! `asf` — Stage 1 kernel driver.
//!
//! `asf demo [dir]` runs the milestone e2e against a real directory (default:
//! a temp dir) and narrates the ledger: manifest → traced mutations →
//! out-of-band edit → attributed drift → coherent revert → verification.

use anyhow::{bail, Context, Result};
use asf_kernel::kernel::Fabric;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use asf_kernel::trace;
use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("demo") => {
            let keep = args.get(2).cloned();
            let tmp;
            let dir = match &keep {
                Some(d) => Path::new(d).to_path_buf(),
                None => {
                    tmp = tempfile::tempdir()?;
                    tmp.path().to_path_buf()
                }
            };
            demo(&dir)?;
            if let Some(d) = keep {
                println!("\nfabric home kept at: {d}");
            }
            Ok(())
        }
        _ => {
            eprintln!("usage: asf demo [dir]");
            std::process::exit(2);
        }
    }
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
