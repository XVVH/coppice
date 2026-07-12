//! Pure decision-time replay: the deterministic W-8 regression vector.
//!
//! Every normalized call is evaluated through `asf_kernel::evaluate` —
//! the same seven-dimension evaluator the broker and the gate share —
//! under four fixed cells: derivation {floor, heuristic} × capability
//! {t0 probation, t1 working}. No substrate, no wall clock, no
//! randomness: timestamps derive from call order, capabilities are
//! fixed JSON, meters live in memory with the broker's consume-on-Allow
//! semantics (RF-2/RF-3). Calls to undeclared actions mirror the broker
//! exactly: structural denial BEFORE evaluation (`broker::propose_call`
//! denies on lookup failure with empty checks), rendered as the stable
//! token `structural:undeclared_action`.
//!
//! Re-running on the same corpus revision with the same harness commit
//! MUST reproduce the verdict vector byte-for-byte; its sha256 is the
//! baseline W-8 verifies verdict-invariance against (no verdict may
//! change on corpora containing no revoke events).

use super::derive::{self, Mode};
use super::model::{ParseOutcome, Trajectory};
use asf_kernel::evaluate::{checks_json, evaluate, CallCtx, Outcome};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const BASE_TIME: &str = "2026-01-01T00:00:00Z";
/// Far past any reachable per-call timestamp: `ts()` advances one
/// second per call index, so expiry-as-artifact would need a ~31M-call
/// trajectory. (At the original +2h ceiling, call index 7,201 would
/// have started mass-denying as `structural:capability expired` —
/// a harness artifact masquerading as a verdict.)
pub const CAP_EXPIRES_AT: &str = "2027-01-01T00:00:00Z";
pub const VECTOR_FORMAT: &str = "w9-verdict-vector-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    T0,
    T1,
}

impl Tier {
    fn as_str(self) -> &'static str {
        match self {
            Tier::T0 => "t0",
            Tier::T1 => "t1",
        }
    }
}

pub const CELLS: [(Mode, Tier); 4] = [
    (Mode::Floor, Tier::T0),
    (Mode::Floor, Tier::T1),
    (Mode::Heuristic, Tier::T0),
    (Mode::Heuristic, Tier::T1),
];

fn cell_name(mode: Mode, tier: Tier) -> String {
    format!("{}-{}", mode.as_str(), tier.as_str())
}

/// t0 mirrors the broker-demo probation capability (the §5.1 "default
/// probation caveat" posture); t1 is the shape an integrator would mint
/// for a path-less external tool: live reach, a working budget, and no
/// paths.write caveat (foreign tools declare no path_args, so a paths
/// caveat can only fail closed — carrying it is the t0 finding).
pub(crate) fn cap_json(
    tier: Tier,
    cap_id: &str,
    manifest: &str,
    tools: &[Value],
    actions: &[Value],
) -> Value {
    let mut caveats = vec![
        json!({"dim": "action.allow", "tools": tools, "actions": actions}),
        json!({"dim": "reversibility.max", "max": "compensable"}),
    ];
    match tier {
        Tier::T0 => {
            caveats.push(json!({"dim": "external_reach", "mode": "none"}));
            caveats.push(json!({"dim": "budget.count", "action_class": "write", "max": 2, "window": "run"}));
            caveats.push(json!({"dim": "paths.write", "globs": ["inbox/**", "MOCs/**"]}));
        }
        Tier::T1 => {
            caveats.push(json!({"dim": "external_reach", "mode": "live"}));
            caveats.push(json!({"dim": "budget.count", "action_class": "write", "max": 20, "window": "run"}));
        }
    }
    caveats.push(json!({"dim": "approval.min_auth", "min": "local_session"}));
    json!({
        "id": cap_id,
        "holder": "prin:corpus-agent",
        "bound_manifest": manifest,
        "issued_at": BASE_TIME,
        "expires_at": CAP_EXPIRES_AT,
        "caveats": caveats,
        "on_violation": {"default": "deny", "escalatable": ["budget.count:write"]},
    })
}

/// One call's replay verdict. `evaluated` is false for the structural
/// short-circuit (undeclared action), where the broker never reaches
/// the evaluator and no checks exist.
pub(crate) struct CallVerdict {
    pub outcome: &'static str,
    pub failed: Vec<String>,
    pub checks: Value,
    pub evaluated: bool,
}

/// Evaluate one corpus call exactly the way `broker::propose_call`
/// would: undeclared action → structural deny before evaluation;
/// otherwise registered-derivation context, conjunctive evaluation, and
/// budget consumption only on Allow (RF-2/RF-3), mutating `meters`.
/// Shared by the replay loop and the ingest lane so their recorded
/// checks can never drift apart.
pub(crate) fn eval_call(
    cap: &Value,
    man_id: &str,
    traj: &Trajectory,
    call: &super::model::Call,
    idx: usize,
    mode: Mode,
    meters: &mut BTreeMap<String, u64>,
) -> CallVerdict {
    if call.unregistered {
        return CallVerdict {
            outcome: "deny",
            failed: vec!["structural:undeclared_action".to_string()],
            checks: json!([]),
            evaluated: false,
        };
    }
    let schema = traj.tools.iter().find(|t| t.name == call.tool_name);
    let d = derive::derive(schema, &call.tool_name, mode);
    let tref = tool_ref(schema.map(|t| t.server.as_str()).unwrap_or("unknown"));
    // Mirror broker::propose_call context assembly exactly: paths
    // extracted only for write-shaped classes, from declared path_args —
    // foreign tools declare none, so a write-shaped call carries
    // Some(vec![]) and paths.write (where the capability has it) fails
    // closed.
    let write_paths = if matches!(d.class, "write" | "delete" | "move") {
        Some(Vec::new())
    } else {
        None
    };
    let now = ts(idx as i64);
    let ctx = CallCtx {
        tool: &tref,
        action: &call.tool_name,
        reversibility: d.reversibility,
        side_effect: d.side_effect,
        action_class: d.class,
        write_paths,
        now: &now,
        current_manifest: man_id,
    };
    let eval = evaluate(
        cap,
        &ctx,
        &mut |key| meters.get(key).copied().unwrap_or(0),
        &mut |_| None,
    );
    let (outcome, failed): (&'static str, Vec<String>) = match &eval.outcome {
        Outcome::Allow => {
            // Broker consumption semantics: budgets meter only on
            // Allow (RF-2/RF-3), only where they applied.
            for c in &eval.checks {
                if c.caveat.starts_with("budget.count:")
                    && c.meter.get("applies") != Some(&Value::Bool(false))
                {
                    *meters.entry(c.caveat.clone()).or_insert(0) += 1;
                }
            }
            ("allow", Vec::new())
        }
        Outcome::Deny { failed, structural } => {
            let mut dims = failed.clone();
            if let Some(s) = structural {
                dims.push(format!("structural:{s}"));
            }
            ("deny", dims)
        }
        Outcome::Escalate { failed } => ("escalate", failed.clone()),
    };
    CallVerdict {
        outcome,
        failed,
        checks: checks_json(&eval.checks),
        evaluated: true,
    }
}

/// The floor×t0 verdicts for one trajectory — the cell the ingest lane
/// records into §6 tool_call bodies. Same helper, same capability
/// shape, zero drift.
pub(crate) fn floor_t0_verdicts(traj: &Trajectory) -> Vec<CallVerdict> {
    let man_id = format!("man:corpus-{}", traj.uuid);
    let cap_id = format!("cap:corpus-{}-t0", traj.uuid);
    let (tool_refs, actions) = allow_lists(traj);
    let cap = cap_json(Tier::T0, &cap_id, &man_id, &tool_refs, &actions);
    let mut meters: BTreeMap<String, u64> = BTreeMap::new();
    traj.calls
        .iter()
        .enumerate()
        .map(|(idx, call)| eval_call(&cap, &man_id, traj, call, idx, Mode::Floor, &mut meters))
        .collect()
}

fn allow_lists(traj: &Trajectory) -> (Vec<Value>, Vec<Value>) {
    let tool_refs = traj
        .servers()
        .iter()
        .map(|s| Value::String(tool_ref(s)))
        .collect();
    let actions = traj
        .tools
        .iter()
        .map(|t| Value::String(t.name.clone()))
        .collect();
    (tool_refs, actions)
}

#[derive(Debug, Default, Clone)]
pub struct CellStats {
    pub allow: u64,
    pub deny: u64,
    pub escalate: u64,
    pub failed_dims: BTreeMap<String, u64>,
}

#[derive(Debug)]
pub struct ReplayResult {
    pub cells: BTreeMap<String, CellStats>,
    pub calls_per_trajectory: Vec<usize>,
    pub unregistered_calls: u64,
    pub vector_sha256: String,
    pub vector_lines: u64,
    /// Evaluator invocations (structural short-circuits excluded).
    pub evaluations: u64,
    pub wall_ms: u128,
    /// First N per-call details per cell, for fixtures and spot reads.
    pub sample_details: Vec<Value>,
}

const SAMPLE_DETAILS_PER_CELL: usize = 50;

pub fn run(parsed: &ParseOutcome, vector_out: Option<&std::path::Path>) -> Result<ReplayResult> {
    let mut cells: BTreeMap<String, CellStats> = CELLS
        .iter()
        .map(|(m, t)| (cell_name(*m, *t), CellStats::default()))
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(VECTOR_FORMAT.as_bytes());
    hasher.update(b"\n");
    let mut vector_file = match vector_out {
        Some(p) => Some(
            std::fs::File::create(p)
                .with_context(|| format!("creating vector file {}", p.display()))?,
        ),
        None => None,
    };
    if let Some(f) = vector_file.as_mut() {
        use std::io::Write;
        writeln!(f, "{VECTOR_FORMAT}").context("writing vector header")?;
    }

    let mut sample_details = Vec::new();
    let mut vector_lines = 0u64;
    let mut evaluations = 0u64;
    let unregistered_calls = parsed
        .trajectories
        .iter()
        .flat_map(|t| &t.calls)
        .filter(|c| c.unregistered)
        .count() as u64;

    let started = std::time::Instant::now();
    for (mode, tier) in CELLS {
        let cell = cell_name(mode, tier);
        let mut cell_samples = 0usize;
        for traj in &parsed.trajectories {
            let stats = cells.get_mut(&cell).expect("cell exists");
            let man_id = format!("man:corpus-{}", traj.uuid);
            let cap_id = format!("cap:corpus-{}-{}", traj.uuid, tier.as_str());
            let (tool_refs, actions) = allow_lists(traj);
            let cap = cap_json(tier, &cap_id, &man_id, &tool_refs, &actions);
            let mut meters: BTreeMap<String, u64> = BTreeMap::new();

            for (idx, call) in traj.calls.iter().enumerate() {
                let v = eval_call(&cap, &man_id, traj, call, idx, mode, &mut meters);
                if v.evaluated {
                    evaluations += 1;
                }
                match v.outcome {
                    "allow" => stats.allow += 1,
                    "escalate" => stats.escalate += 1,
                    _ => stats.deny += 1,
                }
                for dim in &v.failed {
                    *stats.failed_dims.entry(dim.clone()).or_insert(0) += 1;
                }
                let schema = traj.tools.iter().find(|t| t.name == call.tool_name);
                let tref = tool_ref(schema.map(|t| t.server.as_str()).unwrap_or("unknown"));
                let line = format!(
                    "{cell}|{}|{}|{}.{}|{}|{}",
                    traj.uuid,
                    idx,
                    tref,
                    call.tool_name,
                    v.outcome,
                    v.failed.join(",")
                );
                hasher.update(line.as_bytes());
                hasher.update(b"\n");
                if let Some(f) = vector_file.as_mut() {
                    use std::io::Write;
                    writeln!(f, "{line}").context("writing vector line")?;
                }
                vector_lines += 1;
                if cell_samples < SAMPLE_DETAILS_PER_CELL {
                    cell_samples += 1;
                    sample_details.push(json!({
                        "cell": cell,
                        "uuid": traj.uuid,
                        "call": idx,
                        "action": call.tool_name,
                        "unregistered": call.unregistered,
                        "outcome": v.outcome,
                        "failed": v.failed,
                    }));
                }
            }
        }
    }
    let wall_ms = started.elapsed().as_millis();

    Ok(ReplayResult {
        cells,
        calls_per_trajectory: parsed.trajectories.iter().map(|t| t.calls.len()).collect(),
        unregistered_calls,
        vector_sha256: hex::encode(hasher.finalize()),
        vector_lines,
        evaluations,
        wall_ms,
        sample_details,
    })
}

pub fn tool_ref(server: &str) -> String {
    format!("tool:corpus/{server}")
}

/// Deterministic per-call timestamp: BASE_TIME + call index seconds.
pub(crate) fn ts(secs: i64) -> String {
    let base = asf_kernel::parse_instant(BASE_TIME).expect("BASE_TIME parses");
    (base + time::Duration::seconds(secs))
        .format(&time::format_description::well_known::Rfc3339)
        .expect("rfc3339 formats")
}

pub fn report_json(r: &ReplayResult, parsed: &ParseOutcome, row_limit: Option<usize>) -> Value {
    let total_calls: usize = r.calls_per_trajectory.iter().sum();
    json!({
        "format": VECTOR_FORMAT,
        "heuristic": derive::HEURISTIC_VERSION,
        "base_time": BASE_TIME,
        "cap_expires_at": CAP_EXPIRES_AT,
        "corpus_revision": parsed.revision,
        "row_limit": row_limit,
        "trajectories": parsed.trajectories.len(),
        "quarantined": parsed.quarantined.len(),
        "no_call_rows": parsed.no_call_rows,
        "calls": total_calls,
        "unregistered_calls": r.unregistered_calls,
        "evaluations": r.evaluations,
        "wall_ms": r.wall_ms,
        "evals_per_sec": if r.wall_ms > 0 {
            (r.evaluations as f64 / (r.wall_ms as f64 / 1000.0)).round()
        } else {
            0.0
        },
        "verdict_vector_sha256": r.vector_sha256,
        "vector_lines": r.vector_lines,
        "cells": r.cells.iter().map(|(name, s)| {
            (name.clone(), json!({
                "allow": s.allow,
                "deny": s.deny,
                "escalate": s.escalate,
                "failed_dims": s.failed_dims,
            }))
        }).collect::<serde_json::Map<String, Value>>(),
        "sample_details": r.sample_details,
    })
}
