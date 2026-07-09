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
use crate::tools::{self, ToolError};
use crate::{canon, now_rfc3339, trace};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

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
}

/// The broker's decision on a proposed call.
#[derive(Debug)]
pub enum Decision {
    /// Execute it: forward `forwarded_args` downstream, then report the
    /// result via [`Broker::record_result`] with the returned ticket.
    Allowed { ticket: u64, forwarded_args: Value },
    Denied { reasons: Vec<String>, structural: Option<String>, event: String },
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
            );",
        )?;
        Ok(Self { fabric, pending: Vec::new(), next_ticket: 1 })
    }

    // ---- registration & minting ----------------------------------------

    pub fn register_tool(&mut self, tool_name: &str, actions: Value) -> Result<String, BrokerError> {
        let now = now_rfc3339();
        let sk = self.fabric.fabric_sk().clone();
        let span = self.fabric.substrate_span().to_string();
        Ok(tools::register(&mut self.fabric.conn, &sk, &span, tool_name, actions, &now)?)
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
        // M1: stores reachable via allowed tools ⊆ manifest roots.
        let allow = caveats
            .iter()
            .find(|c| c["dim"] == "action.allow")
            .ok_or(BrokerError::NoActionAllow)?;
        let man = trace::get_object(&self.fabric.conn, manifest_id)?;
        let root_stores: Vec<&str> = man["state"]["roots"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r["store"].as_str())
            .collect();
        for tool in allow["tools"].as_array().into_iter().flatten() {
            let tool = tool.as_str().unwrap_or_default();
            for store in tools::stores_for_tool(&self.fabric.conn, tool)? {
                if !root_stores.contains(&store.as_str()) {
                    return Err(BrokerError::M1 { store, manifest: manifest_id.into() });
                }
            }
        }

        let now = now_rfc3339();
        let body = capability::build(holder, manifest_id, None, &now, expires_at, caveats, escalatable)?;
        let sealed = canon::seal("cap", body, self.fabric.fabric_sk())?;
        let id = trace::put_object(&self.fabric.conn, "capability", &sealed, &now)?;
        self.grant_event(&id, manifest_id, None)?;
        Ok(id)
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
        let now = now_rfc3339();
        let body = capability::build(
            holder, bound_manifest, Some(parent_id), &now, expires_at, caveats, escalatable,
        )?;
        capability::verify_attenuation(&parent, &Value::Object(body.clone()))?;
        let sealed = canon::seal("cap", body, self.fabric.fabric_sk())?;
        let id = trace::put_object(&self.fabric.conn, "capability", &sealed, &now)?;
        self.grant_event(&id, bound_manifest, Some(parent_id))?;
        Ok(id)
    }

    fn grant_event(&mut self, cap: &str, manifest: &str, parent: Option<&str>) -> Result<(), BrokerError> {
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
            return self.deny(cap_id, tool, action, vec![], Some("capability signature invalid (F1)".into()), json!([]));
        }

        // Undeclared actions cannot be called (§4).
        let reg = match tools::lookup_action(&self.fabric.conn, tool, action) {
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
        let mut exempt = |key: &str| -> Option<String> {
            let row: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT rowid, escalation FROM exemptions
                     WHERE cap = ?1 AND key = ?2 AND remaining > 0 LIMIT 1",
                    params![cap_id, key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .ok()
                .flatten();
            row.map(|(rowid, esc)| {
                conn.execute(
                    "UPDATE exemptions SET remaining = remaining - 1 WHERE rowid = ?1",
                    [rowid],
                )
                .ok();
                format!("esc:{esc}")
            })
        };

        let eval = evaluate::evaluate(&cap, &ctx, &mut meter, &mut exempt);
        let checks = evaluate::checks_json(&eval.checks);

        match eval.outcome {
            Outcome::Allow => {
                // Bump run-window meters for the class this call consumed.
                for c in &eval.checks {
                    if c.caveat.starts_with("budget.count:") {
                        if c.meter.get("applies") == Some(&Value::Bool(false)) {
                            continue;
                        }
                        self.fabric.conn.execute(
                            "INSERT INTO broker_meters (cap, key, used) VALUES (?1, ?2, 1)
                             ON CONFLICT(cap, key) DO UPDATE SET used = used + 1",
                            params![cap_id, c.caveat],
                        )?;
                    }
                }
                // Credential injection — AFTER checks, into the forwarded
                // copy only. reg.credentials: {"arg": <field>, "secret": <vault name>}.
                let mut forwarded = args.clone();
                if let Some(cred) = reg.get("credentials").filter(|c| !c.is_null()) {
                    if let (Some(arg), Some(secret)) =
                        (cred["arg"].as_str(), cred["secret"].as_str())
                    {
                        if let Some(v) = self.fabric.keystore().secret_get(secret).map_err(KernelError::from)? {
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
                Ok(Decision::Allowed { ticket, forwarded_args: forwarded })
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
        Ok(Decision::Denied { reasons: failed, structural, event: ev.id })
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
    pub fn record_result(&mut self, ticket: u64, result: &[u8]) -> Result<trace::Appended, BrokerError> {
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
