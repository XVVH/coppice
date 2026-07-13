//! Substrate ingestion lane: encodability + throughput on the real
//! machinery (W-9 lane 2's non-deterministic half).
//!
//! Converted trajectories are written into a REAL fabric home as
//! observed-mode manifests (M7: no authority declaration, unattributed
//! calls tolerated — exactly what a foreign trace is): registration
//! events through §4 normalization, intent capture per trajectory,
//! step-boundary manifests, signed tool_call events with floor×t0
//! checks attached, then full span verification and ledger accounting.
//! Event timestamps here are wall-clock (kernel-stamped); the
//! deterministic baseline lives in replay.rs, not here.
//!
//! Quarantined trajectories are never ingested — the negative contract
//! asserts their absence from the substrate, not just an error string.

use super::derive::{self, Mode};
use super::model::ParseOutcome;
use super::replay::tool_ref;
use anyhow::{bail, Context, Result};
use asf_kernel::broker::Broker;
use asf_kernel::kernel::Fabric;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub fn run(parsed: &ParseOutcome, home: &Path, limit: usize) -> Result<Value> {
    // Tool refs are immutable once registered (§4): a second ingest into
    // the same home would create duplicate live registrations, which
    // lookup treats as a broken state. Refuse anything but a fresh home.
    if home.join("fabric").join("fabric.db").exists() {
        bail!(
            "refusing to ingest into existing fabric home {} — immutable tool refs would \
             double-register; use a fresh --home",
            home.display()
        );
    }
    let scratch = home.join("scratch");
    std::fs::create_dir_all(&scratch)?;
    let stores = vec![StoreSpec {
        store: "fs:scratch".into(),
        tier: 1,
        kind: StoreKind::Fs,
        path: scratch,
    }];
    let fabric = Fabric::initialize(home.join("fabric"), stores)?;
    let mut broker = Broker::new(fabric).context("broker init")?;

    let human = broker
        .fabric
        .register_principal("human", "corpus-operator", &placeholder_key(0x51), None)?;
    let agent = broker.fabric.register_principal(
        "agent",
        "corpus-replayer",
        &placeholder_key(0x52),
        Some(&human),
    )?;
    let chan = broker
        .fabric
        .register_channel(&human, "local_session", b"corpus:offline", "local_session")?;

    // Pre-scan: a tool ref is immutable once registered, so each server
    // registers exactly once with the union of its tools across the
    // whole slice (a second registration would be a spec violation).
    let take = parsed.trajectories.len().min(limit);
    let mut server_actions: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    for traj in &parsed.trajectories[..take] {
        for t in &traj.tools {
            let d = derive::derive(Some(t), &t.name, Mode::Floor);
            server_actions
                .entry(t.server.clone())
                .or_default()
                .entry(t.name.clone())
                .or_insert(json!({
                    // side_effect/domain/class/store declared; reversibility,
                    // egress, surface, path_args deliberately absent so §4
                    // registration normalization applies the §0 floor to
                    // every real schema in the slice.
                    "name": t.name,
                    "side_effect": d.side_effect,
                    "domain": d.domain,
                    "class": d.class,
                    "store": d.store,
                }));
        }
    }

    let started = std::time::Instant::now();
    let mut registered: BTreeSet<String> = BTreeSet::new();
    let mut manifests = 0u64;
    let mut tool_calls = 0u64;
    let behavior = json!({
        "bundle": "corpus-ingest",
        "heuristic": derive::HEURISTIC_VERSION,
    });

    for traj in &parsed.trajectories[..take] {
        for server in traj.servers() {
            if registered.insert(server.clone()) {
                let actions: Vec<Value> = server_actions
                    .get(&server)
                    .map(|m| m.values().cloned().collect())
                    .unwrap_or_default();
                broker
                    .register_tool(&tool_ref(&server), Value::Array(actions))
                    .with_context(|| format!("registering tool:corpus/{server}"))?;
            }
        }
        let question = traj
            .question
            .clone()
            .unwrap_or_else(|| format!("corpus trajectory {}", traj.uuid));
        let intent = broker.fabric.capture_intent(
            &human,
            &chan,
            "local_session",
            &question,
            json!({"corpus": "toucan", "uuid": traj.uuid}),
            None,
        )?;
        broker
            .fabric
            .step_boundary(&human, &agent, &intent, behavior.clone())?;
        manifests += 1;

        // floor×t0 checks ride along in the §6 tool_call body (advisory
        // annotations on an observed-mode record of effects that already
        // happened upstream). Computed by the SAME per-call helper the
        // replay lane uses — recorded checks cannot drift from the
        // verdict vector.
        let cell = super::replay::floor_t0_verdicts(traj);
        for (idx, call) in traj.calls.iter().enumerate() {
            let schema = traj.tools.iter().find(|t| t.name == call.tool_name);
            let d = derive::derive(schema, &call.tool_name, Mode::Floor);
            let tref = tool_ref(schema.map(|t| t.server.as_str()).unwrap_or("unknown"));
            let args = serde_json::to_vec(&call.arguments)?;
            let result = call.result.clone().unwrap_or_else(|| "null".into());
            broker.fabric.record_tool_call(
                &tref,
                &call.tool_name,
                &args,
                result.as_bytes(),
                json!({ "action_class": d.class, "paths": Value::Null, "corpus_call": idx }),
                cell[idx].checks.clone(),
                Some(d.reversibility),
            )?;
            tool_calls += 1;
        }
    }
    let ingest_ms = started.elapsed().as_millis();

    // Verification: every span chain + signature, then the ledger must
    // explain every live root.
    let verify_started = std::time::Instant::now();
    let spans = broker.fabric.verify_all_spans()?;
    let events_verified: usize = spans.iter().map(|(_, n)| n).sum();
    let explanation = broker.fabric.explain()?;
    if !explanation.unexplained.is_empty() {
        bail!("ingest left unexplained state: {:?}", explanation.unexplained);
    }
    let verify_ms = verify_started.elapsed().as_millis();

    let mut kinds: BTreeMap<String, i64> = BTreeMap::new();
    {
        let mut stmt = broker
            .fabric
            .conn
            .prepare("SELECT kind, COUNT(*) FROM events GROUP BY kind")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (k, n) = row?;
            kinds.insert(k, n);
        }
    }
    let db_bytes = std::fs::metadata(home.join("fabric").join("fabric.db"))
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(json!({
        "mode": "observed (M7 — no authority declaration)",
        "trajectories_ingested": take,
        "quarantined_excluded": parsed.quarantined.len(),
        "manifests": manifests,
        "tool_calls": tool_calls,
        "servers_registered": registered.len(),
        "event_kinds": kinds,
        "events_verified": events_verified,
        "spans_verified": spans.len(),
        "ledger_unexplained": 0,
        "ingest_ms": ingest_ms,
        "events_per_sec": if ingest_ms > 0 {
            ((events_verified as f64) / (ingest_ms as f64 / 1000.0)).round()
        } else {
            0.0
        },
        "verify_ms": verify_ms,
        "fabric_db_bytes": db_bytes,
    }))
}

fn placeholder_key(b: u8) -> String {
    (0..32).map(|_| format!("{b:02x}")).collect()
}
