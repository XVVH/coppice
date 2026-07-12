//! `asf corpus` — the W-9 foreign-trace corpus harness.
//!
//! Offline by construction: reads corpora fetched by
//! `scripts/fetch-corpora`, registers nothing actuation-scoped, and
//! produces reports + the deterministic verdict vector described in
//! `docs/agent-trace-corpora-2026-07-11.md`. Boundaries enforced here:
//! corpus material is fixtures/calibration only (never founding
//! examples, R1/R6), and quarantine is fail-closed — a trajectory the
//! mapping cannot express contributes gaps, not events.
//!
//!   asf corpus census   --corpora DIR --out DIR
//!   asf corpus replay   --corpora DIR --out DIR [--limit N] [--vector]
//!   asf corpus ingest   --corpora DIR --out DIR --home DIR [--limit N]
//!   asf corpus baseline --corpora DIR --out DIR --home DIR
//!                       [--limit N] [--ingest-limit N]

mod census;
mod derive;
mod ingest;
mod mcpflow;
mod model;
mod replay;
mod toucan;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn cli(args: &[String]) -> Result<()> {
    let sub = args.first().map(String::as_str).unwrap_or("");
    let corpora = flag(args, "--corpora")
        .map(PathBuf::from)
        .context("corpus needs --corpora <dir> (see scripts/fetch-corpora)")?;
    let out = flag(args, "--out")
        .map(PathBuf::from)
        .context("corpus needs --out <dir>")?;
    std::fs::create_dir_all(&out)?;
    // Limits parse strictly: in a tool whose contract is "same inputs →
    // same hash", a typo must error, never silently widen the run.
    let limit = parse_limit(args, "--limit")?;

    match sub {
        "census" => run_census(&corpora, &out),
        "replay" => run_replay(&corpora, &out, limit, args.iter().any(|a| a == "--vector")),
        "ingest" => {
            let home = flag(args, "--home")
                .map(PathBuf::from)
                .context("corpus ingest needs --home <dir>")?;
            run_ingest(&corpora, &out, &home, limit.unwrap_or(500))
        }
        "baseline" => {
            let home = flag(args, "--home")
                .map(PathBuf::from)
                .context("corpus baseline needs --home <dir>")?;
            let ingest_limit = parse_limit(args, "--ingest-limit")?.unwrap_or(500);
            run_census(&corpora, &out)?;
            run_replay(&corpora, &out, limit, false)?;
            run_ingest(&corpora, &out, &home, ingest_limit)
        }
        other => bail!("unknown corpus subcommand '{other}' (census|replay|ingest|baseline)"),
    }
}

fn run_census(corpora: &Path, out: &Path) -> Result<()> {
    let parsed = toucan::load_dir(corpora, None)?;
    let mut universe: Vec<model::ToolSchema> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for traj in &parsed.trajectories {
        for t in &traj.tools {
            if seen.insert((t.source, t.server.clone(), t.name.clone())) {
                universe.push(t.clone());
            }
        }
    }
    let flow = mcpflow::load_dir(corpora);
    for t in &flow.tools {
        if seen.insert((t.source, t.server.clone(), t.name.clone())) {
            universe.push(t.clone());
        }
    }
    let mut gaps = parsed.gaps.clone();
    gaps.merge(flow.gaps.clone());

    let mut census = census::run(&universe);
    census["inputs"] = json!({
        "toucan_revision": parsed.revision,
        "mcpflow_revision": flow.revision,
        "toucan_rows_parsed": parsed.trajectories.len() + parsed.quarantined.len(),
        "mcpflow_files_scanned": flow.files_scanned,
    });
    let gaps_json = gaps.to_json();
    write_json(&out.join("census.json"), &census)?;
    write_json(&out.join("gaps.json"), &gaps_json)?;
    std::fs::write(out.join("census.md"), census::render_md(&census, &gaps_json))?;
    println!(
        "census: {} tools ({} toucan-trajectory rows, {} mcp-flow files) -> {}",
        census["universe"]["tools"],
        parsed.trajectories.len(),
        flow.files_scanned,
        out.display()
    );
    Ok(())
}

fn run_replay(corpora: &Path, out: &Path, limit: Option<usize>, vector: bool) -> Result<()> {
    let parsed = toucan::load_dir(corpora, limit)?;
    if parsed.trajectories.is_empty() {
        bail!("no replayable trajectories under {}", corpora.display());
    }
    let vector_path = out.join("verdict-vector.txt");
    let result = replay::run(&parsed, vector.then_some(vector_path.as_path()))?;
    let mut report = replay::report_json(&result, &parsed, limit);
    report["quarantine"] = json!(parsed
        .quarantined
        .iter()
        .map(|(uuid, reason)| json!({"uuid": uuid, "reason": reason}))
        .collect::<Vec<_>>());
    report["gaps"] = parsed.gaps.to_json();
    write_json(&out.join("replay.json"), &report)?;
    std::fs::write(out.join("replay.md"), render_replay_md(&report))?;
    println!(
        "replay: {} trajectories, {} calls x4 cells = {} evaluations in {}ms ({} evals/s)\n  vector sha256: {}\n  -> {}",
        report["trajectories"],
        report["calls"],
        report["evaluations"],
        report["wall_ms"],
        report["evals_per_sec"],
        report["verdict_vector_sha256"],
        out.display()
    );
    Ok(())
}

fn run_ingest(corpora: &Path, out: &Path, home: &Path, limit: usize) -> Result<()> {
    let parsed = toucan::load_dir(corpora, None)?;
    let mut report = ingest::run(&parsed, home, limit)?;
    report["quarantine"] = json!(parsed
        .quarantined
        .iter()
        .map(|(uuid, reason)| json!({"uuid": uuid, "reason": reason}))
        .collect::<Vec<_>>());
    write_json(&out.join("ingest.json"), &report)?;
    println!(
        "ingest: {} trajectories -> {} manifests, {} tool_calls, {} events verified across {} spans in {}ms ({} events/s)",
        report["trajectories_ingested"],
        report["manifests"],
        report["tool_calls"],
        report["events_verified"],
        report["spans_verified"],
        report["ingest_ms"],
        report["events_per_sec"],
    );
    Ok(())
}

fn render_replay_md(r: &Value) -> String {
    let mut md = String::new();
    md.push_str("# W-9 verdict baseline — deterministic replay\n\n");
    md.push_str(&format!(
        "{} trajectories, {} calls, {} quarantined rows, {} no-call rows; {} unregistered (hallucinated) calls.\n\n",
        r["trajectories"], r["calls"], r["quarantine"].as_array().map(Vec::len).unwrap_or(0),
        r["no_call_rows"], r["unregistered_calls"]
    ));
    md.push_str(&format!(
        "Verdict vector `{}` sha256: `{}` ({} lines). Heuristic: `{}` (non-normative).\n\n",
        r["format"].as_str().unwrap_or("?"),
        r["verdict_vector_sha256"].as_str().unwrap_or("?"),
        r["vector_lines"],
        r["heuristic"].as_str().unwrap_or("?"),
    ));
    md.push_str(&format!(
        "**W-8 regression contract:** re-running this replay on the same corpus revision MUST reproduce this hash; no verdict may change on corpora containing no revoke events.\n\nPerf: {} evaluations in {} ms (~{} evals/sec).\n\n",
        r["evaluations"], r["wall_ms"], r["evals_per_sec"]
    ));
    md.push_str("| cell | allow | deny | escalate | top failed dims |\n|---|---|---|---|---|\n");
    if let Some(cells) = r["cells"].as_object() {
        for (name, c) in cells {
            let mut dims: Vec<(String, u64)> = c["failed_dims"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.as_u64().unwrap_or(0)))
                        .collect()
                })
                .unwrap_or_default();
            dims.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let top: Vec<String> = dims
                .into_iter()
                .take(3)
                .map(|(k, v)| format!("{k} ({v})"))
                .collect();
            md.push_str(&format!(
                "| {name} | {} | {} | {} | {} |\n",
                c["allow"], c["deny"], c["escalate"],
                top.join(", ")
            ));
        }
    }
    md.push_str("\nCells: `floor` = §0 conservative defaults (normative zero-authorship result); `heuristic` = corpus-heuristic-v1 (non-normative, measures the floor↔classifier gap). `t0` = broker-demo probation capability; `t1` = working capability an integrator would mint for a path-less external tool.\n");
    md
}

fn write_json(path: &Path, v: &Value) -> Result<()> {
    std::fs::write(path, serde_json::to_string_pretty(v)? + "\n")
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn parse_limit(args: &[String], name: &str) -> Result<Option<usize>> {
    match flag(args, name) {
        None => Ok(None),
        Some(v) => v
            .parse::<usize>()
            .map(Some)
            .map_err(|_| anyhow::anyhow!("{name} must be a plain integer, got '{v}'")),
    }
}
