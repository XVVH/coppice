//! Broker core. Brief §5.3, spec §5 — the single enforcement chokepoint.
//!
//! Owns: capability minting (F1) and attenuation, the call-decision
//! pipeline (structural gates → conjunctive caveats → allow / deny /
//! escalate), broker-held meters, A9 batch escalations, the approval
//! surface's data model (C2: approvals enter through broker APIs — the
//! daemon's socket or CLI — never through the agent's MCP stream), and
//! credential injection (secrets join the call AFTER the trace payload is
//! stored, so neither the agent nor the ledger ever holds one).
//!
//! M1 is enforced at mint time: every store reachable through the allowed
//! tools must be covered by the bound manifest's state roots. M2 is
//! enforced at call time by the evaluator.

use crate::capability::{self, auth_rank};
use crate::evaluate::{self, CallCtx, Outcome};
use crate::kernel::{Fabric, KernelError};
use crate::promote::{self, Conflict, Op};
use crate::snapshot::{self, StoreKind};
use crate::tools::{self, ToolError};
use crate::{canon, now_rfc3339, trace};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    #[error(transparent)]
    Kernel(#[from] KernelError),
    #[error(transparent)]
    Cap(#[from] capability::CapError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error(transparent)]
    Trace(#[from] trace::TraceError),
    #[error(transparent)]
    Canon(#[from] canon::CanonError),
    #[error(transparent)]
    Snapshot(#[from] snapshot::SnapError),
    #[error("sqlite: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("M1 violation: capability would grant access to store {store} not covered by manifest {manifest} roots")]
    M1 { store: String, manifest: String },
    #[error("mint requires an action.allow caveat (a capability that allows nothing is a bug, not a grant)")]
    NoActionAllow,
    #[error("escalation {0} not found or not pending")]
    NoSuchEscalation(i64),
    #[error("approval channel too weak: {have} < required {need} (approval.min_auth)")]
    ChannelTooWeak { have: String, need: String },
    #[error("no pending call {0}")]
    NoPendingCall(u64),
    #[error("promotion gate: trace exceeds capability — {0} (nothing merged)")]
    GateTraceViolation(String),
    #[error("promotion {0} not found or not pending")]
    NoSuchPromotion(i64),
    #[error("promotion {id} has no valid immutable candidate: {detail}")]
    MalformedPromotion { id: i64, detail: String },
}

/// The broker's decision on a proposed call.
#[derive(Debug)]
pub enum Decision {
    /// Execute it: forward `forwarded_args` downstream, then report the
    /// result via [`Broker::record_result`] with the returned ticket.
    Allowed { ticket: u64, forwarded_args: Value },
    Denied {
        reasons: Vec<String>,
        structural: Option<String>,
        event: String,
    },
    /// Parked. One batch escalation per failing caveat (A9).
    Escalated { escalations: Vec<i64> },
}

struct PendingCall {
    ticket: u64,
    #[allow(dead_code)] // kept for daemon-side debugging/inspection
    cap: String,
    tool: String,
    action: String,
    /// Pre-injection args — what the agent sent, what the ledger stores.
    args_raw: Vec<u8>,
    checks: Value,
    reversibility: String,
    summary: Value,
}

pub struct Broker {
    pub fabric: Fabric,
    pending: Vec<PendingCall>,
    next_ticket: u64,
}

impl Broker {
    pub fn new(fabric: Fabric) -> Result<Self, BrokerError> {
        fabric.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS broker_meters (
                cap  TEXT NOT NULL,
                key  TEXT NOT NULL,
                used INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (cap, key)
            );
            CREATE TABLE IF NOT EXISTS escalations (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                cap        TEXT NOT NULL,
                manifest   TEXT NOT NULL,
                key        TEXT NOT NULL,
                count      INTEGER NOT NULL DEFAULT 1,
                samples    TEXT NOT NULL,
                status     TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL,
                decided_at TEXT
            );
            CREATE TABLE IF NOT EXISTS exemptions (
                escalation INTEGER NOT NULL,
                cap        TEXT NOT NULL,
                key        TEXT NOT NULL,
                remaining  INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS promotions (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                manifest     TEXT NOT NULL,
                branch_paths TEXT NOT NULL,
                preview      TEXT NOT NULL,
                status       TEXT NOT NULL DEFAULT 'pending',
                created_at   TEXT NOT NULL,
                decided_at   TEXT
            );",
        )?;
        Ok(Self {
            fabric,
            pending: Vec::new(),
            next_ticket: 1,
        })
    }

    // ---- registration & minting ----------------------------------------

    pub fn register_tool(
        &mut self,
        tool_name: &str,
        actions: Value,
    ) -> Result<String, BrokerError> {
        let now = now_rfc3339();
        let sk = self.fabric.fabric_sk().clone();
        let span = self.fabric.substrate_span().to_string();
        Ok(tools::register(
            &mut self.fabric.conn,
            &sk,
            &span,
            tool_name,
            actions,
            &now,
        )?)
    }

    /// Mint a capability bound to `manifest_id` (F1: broker-minted, sealed
    /// with the broker/fabric key). Enforces mandatory expiry and M1.
    pub fn mint(
        &mut self,
        manifest_id: &str,
        holder: &str,
        caveats: Vec<Value>,
        escalatable: Vec<&str>,
        expires_at: &str,
    ) -> Result<String, BrokerError> {
        self.check_m1(manifest_id, &caveats)?;

        let now = now_rfc3339();
        let body = capability::build(
            holder,
            manifest_id,
            None,
            &now,
            expires_at,
            caveats,
            escalatable,
        )?;
        let sealed = canon::seal("cap", body, self.fabric.fabric_sk())?;
        let id = trace::put_object(&self.fabric.conn, "capability", &sealed, &now)?;
        self.grant_event(&id, manifest_id, None)?;
        Ok(id)
    }

    /// M1: stores reachable through the allowed tools must be present in the
    /// bound manifest. Applies equally to root grants and attenuated children.
    fn check_m1(&self, manifest_id: &str, caveats: &[Value]) -> Result<(), BrokerError> {
        let allow = caveats
            .iter()
            .find(|c| c["dim"] == "action.allow")
            .ok_or(BrokerError::NoActionAllow)?;
        let man = trace::get_object(&self.fabric.conn, manifest_id)?;
        canon::verify(&man, &self.fabric.fabric_vk())?;
        let root_stores: Vec<&str> = man["state"]["roots"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r["store"].as_str())
            .collect();
        for tool in allow["tools"].as_array().into_iter().flatten() {
            let tool = tool.as_str().unwrap_or_default();
            for store in tools::stores_for_tool(&self.fabric.conn, &self.fabric.fabric_vk(), tool)?
            {
                if !root_stores.contains(&store.as_str()) {
                    return Err(BrokerError::M1 {
                        store,
                        manifest: manifest_id.into(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Attenuate `parent_id` into a child capability (§5.2). The child may
    /// bind to a different manifest (M2 hermetic sub-agents) but every
    /// dimension must be a subset; verification failure is fatal.
    pub fn attenuate(
        &mut self,
        parent_id: &str,
        holder: &str,
        bound_manifest: &str,
        caveats: Vec<Value>,
        escalatable: Vec<&str>,
        expires_at: &str,
    ) -> Result<String, BrokerError> {
        let parent = trace::get_object(&self.fabric.conn, parent_id)?;
        canon::verify(&parent, &self.fabric.fabric_vk())?;
        self.check_m1(bound_manifest, &caveats)?;
        let now = now_rfc3339();
        let body = capability::build(
            holder,
            bound_manifest,
            Some(parent_id),
            &now,
            expires_at,
            caveats,
            escalatable,
        )?;
        capability::verify_attenuation(&parent, &Value::Object(body.clone()))?;
        let sealed = canon::seal("cap", body, self.fabric.fabric_sk())?;
        let id = trace::put_object(&self.fabric.conn, "capability", &sealed, &now)?;
        self.grant_event(&id, bound_manifest, Some(parent_id))?;
        Ok(id)
    }

    fn grant_event(
        &mut self,
        cap: &str,
        manifest: &str,
        parent: Option<&str>,
    ) -> Result<(), BrokerError> {
        let sk = self.fabric.fabric_sk().clone();
        let span = self.fabric.substrate_span().to_string();
        trace::append(
            &mut self.fabric.conn,
            &sk,
            &span,
            Some(manifest),
            "grant",
            json!({ "capability": cap, "parent": parent }),
            &now_rfc3339(),
        )?;
        Ok(())
    }

    // ---- the decision pipeline ------------------------------------------

    /// Evaluate a proposed tool call under `cap_id`. Never executes anything.
    pub fn propose_call(
        &mut self,
        cap_id: &str,
        tool: &str,
        action: &str,
        args: &Value,
    ) -> Result<Decision, BrokerError> {
        let now = now_rfc3339();
        let cap = trace::get_object(&self.fabric.conn, cap_id)?;
        // Broker-minted or bust (F1): the capability must verify under the
        // broker's own key. A forged or tampered token is structurally dead.
        if canon::verify(&cap, &self.fabric.fabric_vk()).is_err() {
            return self.deny(
                cap_id,
                tool,
                action,
                vec![],
                Some("capability signature invalid (F1)".into()),
                json!([]),
            );
        }

        // Undeclared actions cannot be called (§4).
        let reg =
            match tools::lookup_action(&self.fabric.conn, &self.fabric.fabric_vk(), tool, action) {
                Ok(r) => r,
                Err(e) => {
                    return self.deny(cap_id, tool, action, vec![], Some(e.to_string()), json!([]))
                }
            };

        let manifest = self
            .fabric
            .current_manifest()?
            .unwrap_or_else(|| "man:none".to_string());
        let class = reg["class"].as_str().unwrap_or("write");
        let write_paths = if matches!(class, "write" | "delete" | "move") {
            Some(extract_paths(&reg, args))
        } else {
            None
        };
        let ctx = CallCtx {
            tool,
            action,
            reversibility: reg["reversibility"].as_str().unwrap_or("irreversible"),
            side_effect: reg["side_effect"].as_str().unwrap_or("external"),
            action_class: class,
            write_paths,
            now: &now,
            current_manifest: &manifest,
        };

        let conn = &self.fabric.conn;
        let mut meter = |key: &str| -> u64 {
            conn.query_row(
                "SELECT used FROM broker_meters WHERE cap = ?1 AND key = ?2",
                params![cap_id, key],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
            .unwrap_or(0) as u64
        };
        // Peek only — never consume here (RF-2). The broker commits the
        // decrement below, and only if the aggregate outcome is Allow.
        let mut exempt = |key: &str| -> Option<String> {
            conn.query_row(
                "SELECT escalation FROM exemptions
                 WHERE cap = ?1 AND key = ?2 AND remaining > 0 LIMIT 1",
                params![cap_id, key],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(|esc| format!("esc:{esc}"))
        };

        let eval = evaluate::evaluate(&cap, &ctx, &mut meter, &mut exempt);
        let checks = evaluate::checks_json(&eval.checks);

        match eval.outcome {
            Outcome::Allow => {
                // Consume budget + approval exemptions atomically, and ONLY
                // here (RF-2/RF-3): a denied/escalated call never reaches this
                // arm, so it burns neither. NB consumption commits at decision
                // time, not at record_result — deferring it there would let
                // two calls proposed before either records both pass the same
                // budget (fail-OPEN under pipelined proposes). Consuming now
                // keeps the meter monotonic; the residual cost is that an
                // Allowed-but-never-recorded call over-counts budget (the
                // fail-safe direction), and the promotion gate reconciles
                // authority from the signed ledger, not this meter.
                let tx = self.fabric.conn.unchecked_transaction()?;
                for c in &eval.checks {
                    if c.caveat.starts_with("budget.count:")
                        && c.meter.get("applies") != Some(&Value::Bool(false))
                    {
                        tx.execute(
                            "INSERT INTO broker_meters (cap, key, used) VALUES (?1, ?2, 1)
                             ON CONFLICT(cap, key) DO UPDATE SET used = used + 1",
                            params![cap_id, c.caveat],
                        )?;
                    }
                }
                for (key, _esc) in &eval.consumed_exemptions {
                    tx.execute(
                        "UPDATE exemptions SET remaining = remaining - 1
                         WHERE rowid = (SELECT rowid FROM exemptions
                                        WHERE cap = ?1 AND key = ?2 AND remaining > 0
                                        LIMIT 1)",
                        params![cap_id, key],
                    )?;
                }
                tx.commit()?;
                // Credential injection — AFTER checks, into the forwarded
                // copy only. reg.credentials: {"arg": <field>, "secret": <vault name>}.
                let mut forwarded = args.clone();
                if let Some(cred) = reg.get("credentials").filter(|c| !c.is_null()) {
                    if let (Some(arg), Some(secret)) =
                        (cred["arg"].as_str(), cred["secret"].as_str())
                    {
                        if let Some(v) = self
                            .fabric
                            .keystore()
                            .secret_get(secret)
                            .map_err(KernelError::from)?
                        {
                            forwarded[arg] = Value::String(v);
                        }
                    }
                }
                let ticket = self.next_ticket;
                self.next_ticket += 1;
                self.pending.push(PendingCall {
                    ticket,
                    cap: cap_id.into(),
                    tool: tool.into(),
                    action: action.into(),
                    args_raw: serde_json::to_vec(args).expect("args serialize"),
                    checks,
                    reversibility: ctx.reversibility.to_string(),
                    summary: json!({
                        "capability": cap_id,
                        "action_class": class,
                        "paths": ctx.write_paths,
                    }),
                });
                Ok(Decision::Allowed {
                    ticket,
                    forwarded_args: forwarded,
                })
            }
            Outcome::Escalate { failed } => {
                let mut ids = Vec::new();
                let sample = json!({ "tool": tool, "action": action, "paths": ctx.write_paths });
                for key in failed {
                    ids.push(self.enqueue_escalation(cap_id, &manifest, &key, &sample)?);
                }
                Ok(Decision::Escalated { escalations: ids })
            }
            Outcome::Deny { failed, structural } => {
                self.deny(cap_id, tool, action, failed, structural, checks)
            }
        }
    }

    fn deny(
        &mut self,
        cap: &str,
        tool: &str,
        action: &str,
        failed: Vec<String>,
        structural: Option<String>,
        checks: Value,
    ) -> Result<Decision, BrokerError> {
        let manifest = self.fabric.current_manifest()?;
        let sk = self.fabric.fabric_sk().clone();
        let span = self.fabric.substrate_span().to_string();
        // SI-14: mechanical broker denials recorded as `verdict` events with
        // source "broker" (§6 lacks a dedicated kind).
        let ev = trace::append(
            &mut self.fabric.conn,
            &sk,
            &span,
            manifest.as_deref(),
            "verdict",
            json!({
                "verdict": "deny", "source": "broker",
                "capability": cap, "tool": tool, "action": action,
                "failed": failed, "structural": structural, "checks": checks,
            }),
            &now_rfc3339(),
        )?;
        Ok(Decision::Denied {
            reasons: failed,
            structural,
            event: ev.id,
        })
    }

    /// A9 batching: one pending escalation row per (cap, caveat key); a
    /// repeat violation increments the batch instead of re-pinging.
    fn enqueue_escalation(
        &mut self,
        cap: &str,
        manifest: &str,
        key: &str,
        sample: &Value,
    ) -> Result<i64, BrokerError> {
        let now = now_rfc3339();
        let existing: Option<(i64, i64, String)> = self
            .fabric
            .conn
            .query_row(
                "SELECT id, count, samples FROM escalations
                 WHERE cap = ?1 AND key = ?2 AND status = 'pending'",
                params![cap, key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (id, count) = match existing {
            Some((id, count, samples)) => {
                let mut s: Vec<Value> = serde_json::from_str(&samples).unwrap_or_default();
                if s.len() < 5 {
                    s.push(sample.clone()); // §6: sample: [≤5 refs]
                }
                self.fabric.conn.execute(
                    "UPDATE escalations SET count = count + 1, samples = ?2 WHERE id = ?1",
                    params![id, serde_json::to_string(&s).expect("serialize")],
                )?;
                (id, count + 1)
            }
            None => {
                self.fabric.conn.execute(
                    "INSERT INTO escalations (cap, manifest, key, count, samples, status, created_at)
                     VALUES (?1, ?2, ?3, 1, ?4, 'pending', ?5)",
                    params![cap, manifest, key, serde_json::to_string(&vec![sample]).expect("serialize"), now],
                )?;
                (self.fabric.conn.last_insert_rowid(), 1)
            }
        };
        let sk = self.fabric.fabric_sk().clone();
        let span = self.fabric.substrate_span().to_string();
        trace::append(
            &mut self.fabric.conn,
            &sk,
            &span,
            Some(manifest),
            "escalation",
            json!({ "escalation": id, "capability": cap, "caveat": key,
                    "count": count, "sample": [sample] }),
            &now,
        )?;
        Ok(id)
    }

    // ---- the approval surface (C2) ---------------------------------------
    //
    // These entry points are reachable ONLY via broker-owned surfaces (the
    // daemon socket / `asf approve` CLI). The MCP proxy never routes agent
    // traffic here; the agent may learn an escalation id, never carry the
    // approval. Both resolutions are C1 channel-stamped.

    pub fn list_escalations(&self, status: &str) -> Result<Vec<Value>, BrokerError> {
        let mut stmt = self.fabric.conn.prepare(
            "SELECT id, cap, manifest, key, count, samples, created_at
             FROM escalations WHERE status = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([status], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "capability": r.get::<_, String>(1)?,
                "manifest": r.get::<_, String>(2)?,
                "caveat": r.get::<_, String>(3)?,
                "count": r.get::<_, i64>(4)?,
                "samples": serde_json::from_str::<Value>(&r.get::<_, String>(5)?).unwrap_or(Value::Null),
                "created_at": r.get::<_, String>(6)?,
            }))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Approve a pending escalation batch: grants `uses` exemptions for the
    /// violated caveat (one batch approval covers the batch, A9). Enforces
    /// the capability's `approval.min_auth` against the approving channel.
    pub fn approve_escalation(
        &mut self,
        escalation: i64,
        uses: i64,
        channel: &str,
        auth_strength: &str,
    ) -> Result<(), BrokerError> {
        self.resolve_escalation(escalation, "approved", uses, channel, auth_strength)
    }

    pub fn deny_escalation(
        &mut self,
        escalation: i64,
        channel: &str,
        auth_strength: &str,
    ) -> Result<(), BrokerError> {
        self.resolve_escalation(escalation, "denied", 0, channel, auth_strength)
    }

    fn resolve_escalation(
        &mut self,
        escalation: i64,
        resolution: &str,
        uses: i64,
        channel: &str,
        auth_strength: &str,
    ) -> Result<(), BrokerError> {
        let row: Option<(String, String, String)> = self
            .fabric
            .conn
            .query_row(
                "SELECT cap, manifest, key FROM escalations WHERE id = ?1 AND status = 'pending'",
                [escalation],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (cap_id, manifest, key) = row.ok_or(BrokerError::NoSuchEscalation(escalation))?;

        // approval.min_auth (§5.1): the approving channel must be strong
        // enough; unrankable strengths fail closed.
        let cap = trace::get_object(&self.fabric.conn, &cap_id)?;
        if let Some(min) = cap["caveats"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["dim"] == "approval.min_auth")
            .and_then(|c| c["min"].as_str())
        {
            let need = auth_rank(min);
            let have = auth_rank(auth_strength);
            match (need, have) {
                (Some(n), Some(h)) if h >= n => {}
                _ => {
                    return Err(BrokerError::ChannelTooWeak {
                        have: auth_strength.into(),
                        need: min.into(),
                    })
                }
            }
        }

        let now = now_rfc3339();
        self.fabric.conn.execute(
            "UPDATE escalations SET status = ?2, decided_at = ?3 WHERE id = ?1",
            params![escalation, resolution, now],
        )?;
        if resolution == "approved" && uses > 0 {
            self.fabric.conn.execute(
                "INSERT INTO exemptions (escalation, cap, key, remaining) VALUES (?1, ?2, ?3, ?4)",
                params![escalation, cap_id, key, uses],
            )?;
        }
        let sk = self.fabric.fabric_sk().clone();
        let span = self.fabric.substrate_span().to_string();
        trace::append(
            &mut self.fabric.conn,
            &sk,
            &span,
            Some(&manifest),
            "approval",
            json!({
                "escalation": escalation, "resolution": resolution, "uses": uses,
                "capability": cap_id, "caveat": key,
                "channel": channel, "auth_strength": auth_strength,   // C1
            }),
            &now,
        )?;
        Ok(())
    }

    // ---- result recording -------------------------------------------------

    /// After executing an Allowed call downstream, record the outcome. The
    /// stored args are the PRE-injection bytes; secrets never enter the
    /// payload store.
    pub fn record_result(
        &mut self,
        ticket: u64,
        result: &[u8],
    ) -> Result<trace::Appended, BrokerError> {
        let idx = self
            .pending
            .iter()
            .position(|p| p.ticket == ticket)
            .ok_or(BrokerError::NoPendingCall(ticket))?;
        let p = self.pending.remove(idx);
        Ok(self.fabric.record_tool_call(
            &p.tool,
            &p.action,
            &p.args_raw,
            result,
            p.summary.clone(),
            p.checks.clone(),
            Some(&p.reversibility),
        )?)
    }
}

#[derive(Debug)]
pub enum PromotionOutcome {
    Applied { event: String },
    Parked { promotion: i64 },
}

/// One store's merge plan.
struct StorePlan {
    store: String,
    base: String,
    branch: String,
    trunk: String,
    /// Root to install on trunk; None = trunk already has the merge result.
    install: Option<String>,
}

struct MergePlan {
    stores: Vec<StorePlan>,
    ops: Vec<Op>,
    conflicts: Vec<Conflict>,
}

/// A21/M7 activation view: the earliest verified grant offset for each
/// capability bound to this manifest. Kept as a small pure helper so the
/// authority-binding predicates have a stable mutation-testing target.
fn m7_grant_offsets(
    events: &[trace::EventRow],
    substrate_span: &str,
    manifest_id: &str,
) -> BTreeMap<String, i64> {
    let mut grants = BTreeMap::new();
    for ev in events {
        if ev.span == substrate_span
            && ev.kind == "grant"
            && ev.manifest.as_deref() == Some(manifest_id)
        {
            if let Some(cap) = ev.raw["body"]["capability"].as_str() {
                grants.entry(cap.to_string()).or_insert(ev.offset);
            }
        }
    }
    grants
}

/// Resolve the capability claimed by one effect. Observed mode may carry an
/// unattributed effect; brokered mode may not. Grant ordering is checked only
/// after the claimed capability has passed signature and M2 verification, so a
/// cross-manifest substitution retains the more precise lineage failure.
fn m7_effect_capability<'a>(
    brokered: bool,
    body: &'a Value,
    effect_id: &str,
) -> Result<Option<&'a str>, String> {
    let Some(cap_id) = body["summary"]["capability"].as_str() else {
        return if brokered {
            Err(format!(
                "event {effect_id} has no capability attribution under brokered authority (M7)"
            ))
        } else {
            Ok(None)
        };
    };

    Ok(Some(cap_id))
}

/// A21 activation check for a capability that already passed signature and
/// M2 verification. The grant for this manifest must precede the effect.
fn m7_verify_grant(
    brokered: bool,
    cap_id: &str,
    effect_id: &str,
    effect_offset: i64,
    grants: &BTreeMap<String, i64>,
) -> Result<(), String> {
    if !brokered {
        return Ok(());
    }
    match grants.get(cap_id) {
        Some(grant_offset) if *grant_offset < effect_offset => Ok(()),
        Some(_) => Err(format!(
            "event {effect_id} precedes the grant of {cap_id} (M7 ordering)"
        )),
        None => Err(format!(
            "event {effect_id}: no verified grant event binds {cap_id} to this manifest (M7)"
        )),
    }
}

impl Broker {
    /// Promote a completed branch to trunk (§5.3) — the only mutation in
    /// the system. Order is load-bearing: (1) the recorded trace is
    /// verified against the capabilities that authorized it — a violating
    /// run merges nothing; (2) three-way merge per store, trunk-wins
    /// conflicts; (3) the zero-authorship policy decides auto-apply vs
    /// park for human approval on the C2 surface.
    pub fn promote_manifest(
        &mut self,
        manifest_id: &str,
        branch_paths: &BTreeMap<String, PathBuf>,
    ) -> Result<PromotionOutcome, BrokerError> {
        // M8 (A20): attribute any out-of-band divergence BEFORE the merge
        // consumes live trunk, under the per-home gate lock — attribution
        // must not depend on when the human edited relative to the session.
        let _gate = self.fabric.gate_lock()?;
        self.fabric.check_drift()?;
        let man = trace::get_object(&self.fabric.conn, manifest_id)?;
        canon::verify(&man, &self.fabric.fabric_vk())?;
        let span = man["trace"]["span"]
            .as_str()
            .ok_or_else(|| {
                KernelError::MalformedManifest(manifest_id.into(), "no trace.span".into())
            })?
            .to_string();

        let mut trace_report = self.gate_trace_check(manifest_id, &span)?;
        let plan = self.compute_merge(&man, branch_paths)?;
        trace_report["branch_tip"] = self.verify_branch_tip(&man, &span, &plan)?;

        if promote::default_policy_allows(&plan.ops, &plan.conflicts) {
            let event = self.apply_plan(manifest_id, &plan, &trace_report, "auto")?;
            Ok(PromotionOutcome::Applied { event })
        } else {
            let preview = json!({
                "ops": plan.ops.iter().map(Op::to_json).collect::<Vec<_>>(),
                "conflicts": plan.conflicts.iter().map(Conflict::to_json).collect::<Vec<_>>(),
                "trace_check": trace_report,
                "branch_roots": plan.stores.iter()
                    .map(|sp| (sp.store.clone(), sp.branch.clone()))
                    .collect::<BTreeMap<_, _>>(),
            });
            let bp = serde_json::to_string(
                &branch_paths
                    .iter()
                    .map(|(k, v)| (k, v.to_string_lossy()))
                    .collect::<BTreeMap<_, _>>(),
            )
            .expect("paths serialize");
            self.fabric.conn.execute(
                "INSERT INTO promotions (manifest, branch_paths, preview, status, created_at)
                 VALUES (?1, ?2, ?3, 'pending', ?4)",
                params![
                    manifest_id,
                    bp,
                    serde_json::to_string(&preview).expect("serialize"),
                    now_rfc3339()
                ],
            )?;
            let id = self.fabric.conn.last_insert_rowid();
            let sk = self.fabric.fabric_sk().clone();
            let sspan = self.fabric.substrate_span().to_string();
            trace::append(
                &mut self.fabric.conn,
                &sk,
                &sspan,
                Some(manifest_id),
                "escalation",
                json!({
                    "promotion": id, "caveat": "promotion.policy",
                    "count": 1,
                    "sample": [{
                        "ops": plan.ops.iter().map(|o| o.class()).collect::<Vec<_>>(),
                        "conflicts": plan.conflicts.len(),
                    }],
                }),
                &now_rfc3339(),
            )?;
            Ok(PromotionOutcome::Parked { promotion: id })
        }
    }

    /// §5.3 trace-vs-capability: re-verify the span's chain, then replay
    /// every recorded tool_call through the SAME evaluator that authorized
    /// it at decision time — one dimension vocabulary at both moments, so
    /// the gate can never lag the caveat grammar (the pre-W-2 gate hand-
    /// rechecked four of seven dimensions and omitted `time`, the dimension
    /// where RF-1, the one fail-open bug to date, lived). Context is
    /// rebuilt from the registered action (trusted-mechanical, §4 — a
    /// forged summary class or reversibility cannot dodge the check) plus
    /// the broker-extracted write paths in the signed summary; meters and
    /// approval exemptions replay from signed events, never runtime
    /// tables; and the clock is each event's signed `at` (SI-22): "did the
    /// run do anything its token shouldn't allow" means at the moment of
    /// the call — gate latency must not retro-fail honest parked work.
    /// The broker checking its own homework matters because the gate runs
    /// later, under a different code path, over signed records: a bug (or
    /// tamper) between decision time and merge time surfaces here, before
    /// anything becomes durable.
    fn gate_trace_check(&self, manifest_id: &str, span: &str) -> Result<Value, BrokerError> {
        trace::verify_span(&self.fabric.conn, &self.fabric.fabric_vk(), span)?;
        let events = trace::events_in_span(&self.fabric.conn, span)?;

        // A21/M7: the manifest's declared authority mode. Brokered fails
        // closed below — every effect capability-attributed, and its grant
        // event at a lower substrate offset. Observed claims nothing;
        // attributed calls are still checked in full.
        let man = trace::get_object(&self.fabric.conn, manifest_id)?;
        canon::verify(&man, &self.fabric.fabric_vk()).map_err(|e| {
            BrokerError::GateTraceViolation(format!("manifest {manifest_id} unverifiable: {e}"))
        })?;
        let brokered = man["authority"]["mode"] == "brokered";

        // Approved budget headroom: approval events grant `uses` against an
        // escalation's (cap, caveat-key). The authority binding comes from
        // the signed event itself, never the mutable escalation table.
        let substrate_span = self.fabric.substrate_span();
        trace::verify_span(&self.fabric.conn, &self.fabric.fabric_vk(), substrate_span)?;
        let mut approved: BTreeMap<(String, String), i64> = BTreeMap::new();
        let all_events = trace::all_events(&self.fabric.conn)?;
        // Grant activation offsets per capability for THIS manifest, from
        // the verified substrate span (M7 forward edge).
        let grants = m7_grant_offsets(&all_events, substrate_span, manifest_id);
        for ev in &all_events {
            if ev.span != substrate_span
                || ev.kind != "approval"
                || ev.raw["body"]["resolution"] != "approved"
                || ev.raw["body"].get("escalation").is_none()
            {
                continue;
            }
            let body = &ev.raw["body"];
            let (Some(cap), Some(key), Some(uses)) = (
                body["capability"].as_str(),
                body["caveat"].as_str(),
                body["uses"].as_i64(),
            ) else {
                // Pre-hardening approvals carry no signed authority binding.
                // They cannot safely widen a gate budget; ignore fail-closed.
                continue;
            };
            if uses > 0 {
                *approved
                    .entry((cap.to_string(), key.to_string()))
                    .or_insert(0) += uses;
            }
        }

        let mut caps: BTreeMap<String, Value> = BTreeMap::new();
        // Replay ledgers, keyed (cap, caveat-key): budget consumption and
        // remaining signed-approval headroom, rebuilt in event order.
        let mut meters: BTreeMap<(String, String), u64> = BTreeMap::new();
        let mut tool_calls = 0;
        let mut unattributed = 0;
        let violation = |msg: String| BrokerError::GateTraceViolation(msg);

        for ev in &events {
            if ev.kind != "tool_call" {
                continue;
            }
            tool_calls += 1;
            let body = &ev.raw["body"];
            let Some(cap_id) = m7_effect_capability(brokered, body, &ev.id).map_err(&violation)?
            else {
                unattributed += 1; // observed mode: kernel-recorded call, nothing to check against
                continue;
            };
            let cap = match caps.get(cap_id) {
                Some(c) => c.clone(),
                None => {
                    let c = trace::get_object(&self.fabric.conn, cap_id)?;
                    canon::verify(&c, &self.fabric.fabric_vk())
                        .map_err(|e| violation(format!("capability {cap_id} unverifiable: {e}")))?;
                    if c["bound_manifest"].as_str() != Some(manifest_id) {
                        return Err(violation(format!(
                            "event {} authorized by capability bound to a different manifest (M2)",
                            ev.id
                        )));
                    }
                    caps.insert(cap_id.into(), c.clone());
                    c
                }
            };
            m7_verify_grant(brokered, cap_id, &ev.id, ev.offset, &grants).map_err(&violation)?;

            let (tool, action) = (
                body["tool"].as_str().unwrap_or(""),
                body["action"].as_str().unwrap_or(""),
            );
            // Undeclared actions cannot be called (§4) — at the gate as at
            // decision time. The registered action is the trusted-mechanical
            // source for class/reversibility/side_effect (a forged summary
            // cannot dodge the check); the signed summary supplies only the
            // broker-extracted write paths.
            let reg =
                tools::lookup_action(&self.fabric.conn, &self.fabric.fabric_vk(), tool, action)
                    .map_err(|e| violation(format!("event {}: {e}", ev.id)))?;
            let class = reg["class"].as_str().unwrap_or("write");
            let write_paths = if matches!(class, "write" | "delete" | "move") {
                // Missing paths on a path-writing class fail closed in evaluate.
                Some(
                    body["summary"]["paths"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                )
            } else {
                None
            };
            let ctx = CallCtx {
                tool,
                action,
                reversibility: reg["reversibility"].as_str().unwrap_or("irreversible"),
                side_effect: reg["side_effect"].as_str().unwrap_or("external"),
                action_class: class,
                write_paths,
                now: &ev.at,
                current_manifest: manifest_id,
            };
            let replay = {
                let mut meter = |key: &str| -> u64 {
                    meters
                        .get(&(cap_id.to_string(), key.to_string()))
                        .copied()
                        .unwrap_or(0)
                };
                let mut exempt = |key: &str| -> Option<String> {
                    (approved
                        .get(&(cap_id.to_string(), key.to_string()))
                        .copied()
                        .unwrap_or(0)
                        > 0)
                    .then(|| "signed-approval".to_string())
                };
                evaluate::evaluate(&cap, &ctx, &mut meter, &mut exempt)
            };
            match replay.outcome {
                Outcome::Allow => {
                    // Mirror the broker's Allow arm on the replay ledgers —
                    // consumption only on Allow (the RF-2 discipline).
                    for c in &replay.checks {
                        if c.caveat.starts_with("budget.count:")
                            && c.meter.get("applies") != Some(&Value::Bool(false))
                        {
                            *meters
                                .entry((cap_id.to_string(), c.caveat.clone()))
                                .or_insert(0) += 1;
                        }
                    }
                    for (key, _) in &replay.consumed_exemptions {
                        if let Some(uses) = approved.get_mut(&(cap_id.to_string(), key.clone())) {
                            *uses -= 1;
                        }
                    }
                }
                Outcome::Deny { failed, structural } => {
                    return Err(violation(format!(
                        "event {} fails re-evaluation under {cap_id}: {}",
                        ev.id,
                        structural.unwrap_or_else(|| format!("failed [{}]", failed.join(", ")))
                    )));
                }
                Outcome::Escalate { failed } => {
                    return Err(violation(format!(
                        "event {} executed without signed approval headroom: [{}]",
                        ev.id,
                        failed.join(", ")
                    )));
                }
            }
        }

        Ok(json!({
            "events": events.len(), "tool_calls": tool_calls,
            "unattributed_tool_calls": unattributed,
            "capabilities": caps.keys().collect::<Vec<_>>(), "ok": true,
        }))
    }

    fn compute_merge(
        &mut self,
        man: &Value,
        branch_paths: &BTreeMap<String, PathBuf>,
    ) -> Result<MergePlan, BrokerError> {
        let mut branch_roots = BTreeMap::new();
        for r in man["state"]["roots"].as_array().into_iter().flatten() {
            let store = r["store"].as_str().unwrap_or_default().to_string();
            let spec = self.fabric.store(&store)?.clone();
            let branch_path = branch_paths
                .get(&store)
                .ok_or_else(|| KernelError::UnknownStore(format!("{store} has no branch")))?;
            let mut branch_spec = spec;
            branch_spec.path = branch_path.clone();
            branch_roots.insert(store, snapshot::capture(&self.fabric.cas, &branch_spec)?);
        }
        self.compute_merge_from_roots(man, &branch_roots)
    }

    /// Recompute a merge against current trunk while keeping the agent side
    /// pinned to immutable CAS roots. This is the approval-time form: trunk
    /// may move after preview, but the reviewed branch candidate may not.
    fn compute_merge_from_roots(
        &mut self,
        man: &Value,
        branch_roots: &BTreeMap<String, String>,
    ) -> Result<MergePlan, BrokerError> {
        let mut stores = Vec::new();
        let mut all_ops = Vec::new();
        let mut all_conflicts = Vec::new();
        for r in man["state"]["roots"].as_array().into_iter().flatten() {
            let store = r["store"].as_str().unwrap_or_default().to_string();
            let base_root = r["root"].as_str().unwrap_or_default().to_string();
            let spec = self.fabric.store(&store)?.clone();
            let branch_root = branch_roots
                .get(&store)
                .ok_or_else(|| {
                    KernelError::UnknownStore(format!("{store} has no pinned branch root"))
                })?
                .clone();
            let trunk_root = snapshot::capture(&self.fabric.cas, &spec)?;

            let install = match spec.kind {
                StoreKind::Fs => {
                    let base_obj = snapshot::load_tree_object(&self.fabric.cas, &base_root)?;
                    let branch_obj = snapshot::load_tree_object(&self.fabric.cas, &branch_root)?;
                    let trunk_obj = snapshot::load_tree_object(&self.fabric.cas, &trunk_root)?;
                    let result = promote::three_way(
                        &promote::tree_from_object(&base_obj),
                        &promote::tree_from_object(&branch_obj),
                        &promote::tree_from_object(&trunk_obj),
                    );
                    let merged_root = snapshot::compose_tree(
                        &self.fabric.cas,
                        &result.merged,
                        &[&trunk_obj, &branch_obj, &base_obj],
                    )?;
                    all_ops.extend(result.ops);
                    all_conflicts.extend(result.conflicts);
                    (merged_root != trunk_root).then_some(merged_root)
                }
                StoreKind::Sqlite => {
                    // Opaque store: no sub-file merge exists (SI-18).
                    // Branch-only change installs the branch image; both-
                    // changed is a conflict card and trunk stands.
                    if branch_root == base_root || branch_root == trunk_root {
                        None
                    } else if trunk_root == base_root {
                        all_ops.push(Op::Modify {
                            path: format!("{store} (whole store)"),
                        });
                        Some(branch_root.clone())
                    } else {
                        all_conflicts.push(Conflict {
                            path: format!("{store} (opaque sqlite, whole store)"),
                            base: Some(base_root.clone()),
                            branch: Some(branch_root.clone()),
                            trunk: Some(trunk_root.clone()),
                            resolution: "trunk_wins",
                        });
                        None
                    }
                }
            };
            stores.push(StorePlan {
                store,
                base: base_root,
                branch: branch_root,
                trunk: trunk_root,
                install,
            });
        }
        Ok(MergePlan {
            stores,
            ops: all_ops,
            conflicts: all_conflicts,
        })
    }

    /// Bind the merge candidate to the signed trace. With no tool calls, the
    /// branch must still equal the manifest base. Otherwise every store must
    /// equal the final tool_call's `state_root_after` attestation. This closes
    /// crash/signal and direct-branch-write paths that previously let untraced
    /// state reach promotion.
    fn verify_branch_tip(
        &self,
        man: &Value,
        span: &str,
        plan: &MergePlan,
    ) -> Result<Value, BrokerError> {
        let events = trace::events_in_span(&self.fabric.conn, span)?;
        let last_call = events.iter().rev().find(|e| e.kind == "tool_call");
        let mut roots = BTreeMap::new();
        for store in &plan.stores {
            let expected = match last_call {
                Some(ev) => ev.raw["body"]["state_root_after"]
                    .get(format!("branch:{}", store.store))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        BrokerError::GateTraceViolation(format!(
                            "final tool_call {} has no branch root for {}",
                            ev.id, store.store
                        ))
                    })?
                    .to_string(),
                None => man["state"]["roots"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|r| r["store"].as_str() == Some(store.store.as_str()))
                    .and_then(|r| r["root"].as_str())
                    .ok_or_else(|| {
                        BrokerError::GateTraceViolation(format!(
                            "manifest has no base root for {}",
                            store.store
                        ))
                    })?
                    .to_string(),
            };
            if store.branch != expected {
                return Err(BrokerError::GateTraceViolation(format!(
                    "untraced branch divergence in {}: trace attests {}, branch is {}",
                    store.store, expected, store.branch
                )));
            }
            roots.insert(store.store.clone(), expected);
        }
        Ok(json!({
            "last_tool_call": last_call.map(|e| e.id.clone()),
            "roots": roots,
            "ok": true,
        }))
    }

    /// Stage every store, then swap — same coherence discipline as revert.
    fn apply_plan(
        &mut self,
        manifest_id: &str,
        plan: &MergePlan,
        trace_report: &Value,
        policy: &str,
    ) -> Result<String, BrokerError> {
        let mut prepared = Vec::new();
        for sp in &plan.stores {
            if let Some(root) = &sp.install {
                let spec = self.fabric.store(&sp.store)?.clone();
                prepared.push(snapshot::prepare_restore(&self.fabric.cas, &spec, root)?);
            }
        }
        for prep in prepared {
            snapshot::commit_restore(&self.fabric.cas, prep)?;
        }

        let stores_json: Vec<Value> = plan
            .stores
            .iter()
            .map(|sp| {
                json!({
                    "store": sp.store, "base": sp.base, "branch": sp.branch,
                    "trunk_before": sp.trunk,
                    "merged": sp.install.clone().unwrap_or_else(|| sp.trunk.clone()),
                })
            })
            .collect();
        let sk = self.fabric.fabric_sk().clone();
        let span = self.fabric.substrate_span().to_string();
        let ev = trace::append(
            &mut self.fabric.conn,
            &sk,
            &span,
            Some(manifest_id),
            "promotion",
            json!({
                "manifest": manifest_id,
                "stores": stores_json,
                "ops": plan.ops.iter().map(Op::to_json).collect::<Vec<_>>(),
                "conflicts": plan.conflicts.iter().map(Conflict::to_json).collect::<Vec<_>>(),
                "trace_check": trace_report,
                "policy": policy,
            }),
            &now_rfc3339(),
        )?;
        for sp in &plan.stores {
            let merged = sp.install.clone().unwrap_or_else(|| sp.trunk.clone());
            self.fabric
                .set_expected_root(&sp.store, &merged, ev.offset)?;
        }
        Ok(ev.id)
    }

    pub fn list_promotions(&self, status: &str) -> Result<Vec<Value>, BrokerError> {
        let mut stmt = self
            .fabric
            .conn
            .prepare("SELECT id, manifest, preview, created_at FROM promotions WHERE status = ?1 ORDER BY id")?;
        let rows = stmt.query_map([status], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "manifest": r.get::<_, String>(1)?,
                "preview": serde_json::from_str::<Value>(&r.get::<_, String>(2)?).unwrap_or(Value::Null),
                "created_at": r.get::<_, String>(3)?,
            }))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Approve a parked promotion (C2 surface, C1-stamped). The merge is
    /// recomputed fresh against current trunk — if trunk moved since the
    /// preview, new divergences still resolve trunk-wins (the safe
    /// direction), never wider than what was previewed.
    pub fn approve_promotion(
        &mut self,
        id: i64,
        channel: &str,
        auth_strength: &str,
    ) -> Result<String, BrokerError> {
        let row: Option<(String, String)> = self
            .fabric
            .conn
            .query_row(
                "SELECT manifest, preview FROM promotions WHERE id = ?1 AND status = 'pending'",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (manifest_id, preview_raw) = row.ok_or(BrokerError::NoSuchPromotion(id))?;
        self.check_min_auth_for_manifest(&manifest_id, auth_strength)?;

        // M8 (A20): the approval-time re-merge consumes live trunk exactly
        // like the auto gate does — same divergence check, same lock.
        let _gate = self.fabric.gate_lock()?;
        self.fabric.check_drift()?;

        let preview: Value =
            serde_json::from_str(&preview_raw).map_err(|e| BrokerError::MalformedPromotion {
                id,
                detail: format!("preview is not JSON: {e}"),
            })?;
        let branch_roots: BTreeMap<String, String> = preview["branch_roots"]
            .as_object()
            .ok_or_else(|| BrokerError::MalformedPromotion {
                id,
                detail: "legacy preview has no pinned branch_roots; re-gate the session".into(),
            })?
            .iter()
            .map(|(store, root)| {
                root.as_str()
                    .map(|r| (store.clone(), r.to_string()))
                    .ok_or_else(|| BrokerError::MalformedPromotion {
                        id,
                        detail: format!("branch root for {store} is not a string"),
                    })
            })
            .collect::<Result<_, _>>()?;
        let man = trace::get_object(&self.fabric.conn, &manifest_id)?;
        canon::verify(&man, &self.fabric.fabric_vk())?;
        let span = man["trace"]["span"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let mut trace_report = self.gate_trace_check(&manifest_id, &span)?;
        let plan = self.compute_merge_from_roots(&man, &branch_roots)?;
        trace_report["branch_tip"] = self.verify_branch_tip(&man, &span, &plan)?;
        let event = self.apply_plan(
            &manifest_id,
            &plan,
            &trace_report,
            &format!("approved:{id}"),
        )?;

        let now = now_rfc3339();
        self.fabric.conn.execute(
            "UPDATE promotions SET status = 'applied', decided_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        let sk = self.fabric.fabric_sk().clone();
        let sspan = self.fabric.substrate_span().to_string();
        trace::append(
            &mut self.fabric.conn,
            &sk,
            &sspan,
            Some(&manifest_id),
            "approval",
            json!({ "promotion": id, "resolution": "approved",
                    "channel": channel, "auth_strength": auth_strength }),
            &now,
        )?;
        Ok(event)
    }

    pub fn reject_promotion(
        &mut self,
        id: i64,
        channel: &str,
        auth_strength: &str,
    ) -> Result<(), BrokerError> {
        let manifest: Option<String> = self
            .fabric
            .conn
            .query_row(
                "SELECT manifest FROM promotions WHERE id = ?1 AND status = 'pending'",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        let manifest = manifest.ok_or(BrokerError::NoSuchPromotion(id))?;
        let now = now_rfc3339();
        self.fabric.conn.execute(
            "UPDATE promotions SET status = 'rejected', decided_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        let sk = self.fabric.fabric_sk().clone();
        let sspan = self.fabric.substrate_span().to_string();
        trace::append(
            &mut self.fabric.conn,
            &sk,
            &sspan,
            Some(&manifest),
            "approval",
            json!({ "promotion": id, "resolution": "denied",
                    "channel": channel, "auth_strength": auth_strength }),
            &now,
        )?;
        Ok(())
    }

    fn check_min_auth_for_manifest(
        &self,
        manifest_id: &str,
        auth_strength: &str,
    ) -> Result<(), BrokerError> {
        // Strongest approval.min_auth among verified caps bound to this
        // manifest applies to gate approvals too.
        let mut stmt = self
            .fabric
            .conn
            .prepare("SELECT raw FROM objects WHERE kind = 'capability'")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut need: Option<String> = None;
        for raw in rows {
            let cap: Value = serde_json::from_str(&raw?).expect("stored objects are valid JSON");
            if cap["bound_manifest"].as_str() != Some(manifest_id)
                || canon::verify(&cap, &self.fabric.fabric_vk()).is_err()
            {
                continue;
            }
            if let Some(min) = cap["caveats"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|c| c["dim"] == "approval.min_auth")
                .and_then(|c| c["min"].as_str())
            {
                let stronger = match (&need, auth_rank(min)) {
                    (None, Some(_)) => true,
                    (Some(cur), Some(new)) => Some(new) > auth_rank(cur),
                    _ => false,
                };
                if stronger {
                    need = Some(min.to_string());
                }
            }
        }
        if let Some(min) = need {
            match (auth_rank(&min), auth_rank(auth_strength)) {
                (Some(n), Some(h)) if h >= n => {}
                _ => {
                    return Err(BrokerError::ChannelTooWeak {
                        have: auth_strength.into(),
                        need: min,
                    })
                }
            }
        }
        Ok(())
    }
}

/// Extract write-path targets from call args using the registration's
/// declared `path_args` (§4 trusted-mechanical metadata; never inferred
/// from anything the agent claims about the target — A8 spirit).
fn extract_paths(reg: &Value, args: &Value) -> Vec<String> {
    reg["path_args"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|field| args.get(field).and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}
