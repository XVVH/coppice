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
use crate::kernel::{Fabric, KernelError, StateChangePlan, StateRootTransition};
use crate::promote::{self, Conflict, Op};
use crate::snapshot::{self, StoreKind};
use crate::tools::{self, ToolError};
use crate::{canon, now_rfc3339, trace};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
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
    #[error("capability closed (§5.4): {0}")]
    CapabilityClosed(String),
    #[error("invalid revocation: {0}")]
    InvalidRevocation(String),
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

#[derive(Clone)]
struct PendingCall {
    ticket: u64,
    cap: String,
    tool: String,
    action: String,
    /// Pre-injection args — what the agent sent, what the ledger stores.
    args_raw: Vec<u8>,
    checks: Value,
    reversibility: String,
    summary: Value,
}

#[derive(Default)]
struct DecisionAuthority {
    meters: BTreeMap<String, u64>,
    exemptions: BTreeMap<String, String>,
}

fn w14_required_approval_strength(
    capability_id: &str,
    capability: &Value,
) -> Result<Option<(u8, String)>, BrokerError> {
    let caveats = capability["caveats"].as_array().ok_or_else(|| {
        BrokerError::GateTraceViolation(format!(
            "capability {capability_id} has no signed caveat array"
        ))
    })?;
    let mut ranked = BTreeMap::new();
    for caveat in caveats.iter().filter(|caveat| caveat["dim"] == "approval.min_auth") {
        let minimum = caveat["min"].as_str().ok_or_else(|| {
            BrokerError::GateTraceViolation(format!(
                "capability {capability_id} has malformed approval.min_auth"
            ))
        })?;
        let rank = auth_rank(minimum).ok_or_else(|| {
            BrokerError::GateTraceViolation(format!(
                "capability {capability_id} has unknown approval strength {minimum}"
            ))
        })?;
        ranked.insert(rank, minimum.to_string());
    }
    Ok(ranked.into_iter().next_back())
}

fn w14_require_approval_strength(
    capability_id: &str,
    capability: &Value,
    auth_strength: &str,
) -> Result<(), BrokerError> {
    let Some((need_rank, need)) = w14_required_approval_strength(capability_id, capability)? else {
        return Ok(());
    };
    match auth_rank(auth_strength) {
        Some(have_rank) if have_rank >= need_rank => Ok(()),
        _ => Err(BrokerError::ChannelTooWeak {
            have: auth_strength.into(),
            need,
        }),
    }
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

    fn load_capability(&self, id: &str) -> Result<Value, BrokerError> {
        Ok(trace::load_verified_object(
            &self.fabric.conn,
            id,
            "cap",
            "capability",
            &self.fabric.fabric_vk(),
        )?)
    }

    /// Capability prerequisite shared by tool advertisement and dispatch:
    /// verified type/signature, current M2 binding, mandatory live expiry,
    /// and event-derived grant/ancestry/revocation state at the current head.
    pub fn current_advertisable_capability(&self, id: &str) -> Result<Value, BrokerError> {
        let capability = self.load_capability(id)?;
        let current = self.fabric.current_manifest()?.ok_or_else(|| {
            BrokerError::GateTraceViolation("no current manifest for capability advertisement".into())
        })?;
        if capability["bound_manifest"].as_str() != Some(current.as_str()) {
            return Err(BrokerError::GateTraceViolation(format!(
                "capability {id} is not bound to current manifest {current} (M2)"
            )));
        }
        let expiry = capability["expires_at"].as_str().ok_or_else(|| {
            BrokerError::GateTraceViolation(format!("capability {id} has no mandatory expiry"))
        })?;
        match (crate::parse_instant(&now_rfc3339()), crate::parse_instant(expiry)) {
            (Some(now), Some(bound)) if now <= bound => {}
            _ => {
                return Err(BrokerError::GateTraceViolation(format!(
                    "capability {id} is expired or has an invalid expiry"
                )))
            }
        }
        self.liveness_at_head(id)
            .map_err(BrokerError::CapabilityClosed)?;
        Ok(capability)
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
        self.mint_at(manifest_id, holder, caveats, escalatable, expires_at, &now_rfc3339())
    }

    /// Mint with an explicit `issued_at` — the timestamp-collision seam
    /// §5.4's closed-id refusal exists for: an identical body minted at the
    /// same instant reproduces a prior (possibly revoked) content id, and
    /// granting a closed id must fail loudly, never silently issue a dead
    /// token. Tests drive this seam directly.
    pub fn mint_at(
        &mut self,
        manifest_id: &str,
        holder: &str,
        caveats: Vec<Value>,
        escalatable: Vec<&str>,
        expires_at: &str,
        issued_at: &str,
    ) -> Result<String, BrokerError> {
        self.check_m1(manifest_id, &caveats)?;

        let body = capability::build(
            holder,
            manifest_id,
            None,
            issued_at,
            expires_at,
            caveats,
            escalatable,
        )?;
        let sealed = canon::seal("cap", body, self.fabric.fabric_sk())?;
        let id = sealed
            .get("id")
            .and_then(Value::as_str)
            .expect("sealed object has id")
            .to_string();
        self.refuse_closed_id(&id)?;
        trace::put_object(&self.fabric.conn, "capability", &sealed, issued_at)?;
        self.grant_event(&id, manifest_id, None)?;
        Ok(id)
    }

    /// §5.4: the broker MUST refuse to grant a closed id — restoration is a
    /// new mint (new `issued_at` ⇒ new content id), never reactivation.
    fn refuse_closed_id(&self, id: &str) -> Result<(), BrokerError> {
        let events = trace::verified_events(&self.fabric.conn, &self.fabric.fabric_vk())?;
        let revokes = a22_revoke_offsets(&events);
        match revokes.get(id) {
            Some(r) => Err(BrokerError::CapabilityClosed(format!(
                "{id} was revoked at offset {r}; closure is permanent for the id — mint a new capability"
            ))),
            None => Ok(()),
        }
    }

    /// §5.4 decision-time liveness of `cap_id` at the retained-row head
    /// observed with the verified event set in one statement snapshot.
    /// Offset authenticity/freshness remain SI-25. Err(reason) when closed
    /// or unactivated.
    fn liveness_at_head(&self, cap_id: &str) -> Result<(), String> {
        let vk = self.fabric.fabric_vk();
        let snapshot = trace::verified_event_snapshot(&self.fabric.conn, &vk)
            .map_err(|e| format!("authority view unavailable: {e} (fail closed)"))?;
        let grants = a22_grant_bindings(&snapshot.events, self.fabric.substrate_span());
        let revokes = a22_revoke_offsets(&snapshot.events);
        let chain = a22_ancestry(&self.fabric.conn, &vk, cap_id)?;
        a22_state_at(&chain, snapshot.head + 1, &grants, &revokes)
    }

    /// Reconstruct budget consumption and approval headroom exclusively from
    /// verified signed events plus this broker's in-memory dispatch
    /// reservations. `broker_meters` and `exemptions` are compatibility
    /// caches only: storage writes to either table cannot widen authority.
    fn w14_decision_authority(&self, cap_id: &str) -> Result<DecisionAuthority, BrokerError> {
        let events = trace::verified_events(&self.fabric.conn, &self.fabric.fabric_vk())?;
        let substrate = self.fabric.substrate_span();
        let ordered = w11_span_events_in_signed_sequence(&events, substrate);
        let mut bindings: BTreeMap<i64, (String, String, String, i64)> = BTreeMap::new();
        for event in &ordered {
            if event.kind != "escalation" {
                continue;
            }
            let body = &event.raw["body"];
            let (Some(id), Some(capability), Some(caveat)) = (
                body["escalation"].as_i64(),
                body["capability"].as_str(),
                body["caveat"].as_str(),
            ) else {
                continue;
            };
            let Some(manifest) = event.manifest.as_deref() else {
                continue;
            };
            match bindings.get(&id) {
                Some((old_cap, old_caveat, old_manifest, _))
                    if old_cap != capability
                        || old_caveat != caveat
                        || old_manifest != manifest =>
                {
                    return Err(BrokerError::GateTraceViolation(format!(
                        "signed escalation {id} has conflicting authority bindings"
                    )))
                }
                Some(_) => {}
                None => {
                    bindings.insert(
                        id,
                        (
                            capability.to_string(),
                            caveat.to_string(),
                            manifest.to_string(),
                            event.seq,
                        ),
                    );
                }
            }
        }

        let mut remaining: BTreeMap<(String, String, i64), i64> = BTreeMap::new();
        let mut resolved = BTreeSet::new();
        for event in &ordered {
            if event.kind != "approval" || event.raw["body"].get("escalation").is_none() {
                continue;
            }
            let body = &event.raw["body"];
            let Some(id) = body["escalation"].as_i64() else {
                continue;
            };
            if !resolved.insert(id) {
                return Err(BrokerError::GateTraceViolation(format!(
                    "signed escalation {id} has multiple resolutions"
                )));
            }
            let Some((bound_cap, bound_caveat, bound_manifest, escalation_seq)) = bindings.get(&id)
            else {
                continue; // an approval without its signed request grants nothing
            };
            if event.seq <= *escalation_seq
                || event.manifest.as_deref() != Some(bound_manifest.as_str())
                || body["capability"].as_str() != Some(bound_cap.as_str())
                || body["caveat"].as_str() != Some(bound_caveat.as_str())
            {
                continue; // doubtful activation never widens authority
            }
            if body["resolution"] != "approved" || bound_cap != cap_id {
                continue;
            }
            let cap = self.load_capability(bound_cap)?;
            if cap["bound_manifest"].as_str() != Some(bound_manifest.as_str()) {
                continue;
            }
            let Some(auth_strength) = body["auth_strength"].as_str() else {
                continue;
            };
            w14_require_approval_strength(bound_cap, &cap, auth_strength)?;
            let Some(uses) = body["uses"].as_u64().filter(|uses| *uses != 0) else {
                continue;
            };
            let Ok(uses) = i64::try_from(uses) else {
                continue;
            };
            remaining.insert((bound_cap.clone(), bound_caveat.clone(), id), uses);
        }

        let mut meters = BTreeMap::new();
        for event in events.iter().filter(|event| event.kind == "tool_call") {
            if event.raw["body"]["summary"]["capability"].as_str() != Some(cap_id) {
                continue;
            }
            w14_reserve_signed_checks(
                cap_id,
                &event.raw["body"]["checks"],
                &mut meters,
                &mut remaining,
            )?;
        }
        for pending in &self.pending {
            if pending.cap != cap_id {
                continue;
            }
            w14_reserve_signed_checks(
                cap_id,
                &pending.checks,
                &mut meters,
                &mut remaining,
            )?;
        }

        let exemptions = remaining
            .into_iter()
            .filter(|(_, uses)| *uses > 0)
            .fold(BTreeMap::new(), |mut available, ((_, caveat, id), _)| {
                available.entry(caveat).or_insert_with(|| format!("esc:{id}"));
                available
            });
        Ok(DecisionAuthority { meters, exemptions })
    }

    /// M1: stores reachable through the allowed tools must be present in the
    /// bound manifest. Applies equally to root grants and attenuated children.
    fn check_m1(&self, manifest_id: &str, caveats: &[Value]) -> Result<(), BrokerError> {
        let allow = caveats
            .iter()
            .find(|c| c["dim"] == "action.allow")
            .ok_or(BrokerError::NoActionAllow)?;
        let man = self.fabric.load_manifest(manifest_id)?;
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
        let parent = self.load_capability(parent_id)?;
        // §5.4: a parent closed at the child's grant offset cannot produce
        // a live child — refused here, and the liveness view derives the
        // same answer regardless of what rows were injected.
        self.liveness_at_head(parent_id)
            .map_err(BrokerError::CapabilityClosed)?;
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
        let id = sealed
            .get("id")
            .and_then(Value::as_str)
            .expect("sealed object has id")
            .to_string();
        self.refuse_closed_id(&id)?;
        trace::put_object(&self.fabric.conn, "capability", &sealed, &now)?;
        self.grant_event(&id, bound_manifest, Some(parent_id))?;
        Ok(id)
    }

    /// A22/§5.4: close `cap_id` early. The signed event IS the closure —
    /// permanent for the id, prospective, descendant-closing through the
    /// ancestry view (no subtree enumeration happens here). `channel` is
    /// `Some((chan, auth_strength))` for a human-originated request (any
    /// registered channel may request — closure only narrows, §3.1
    /// directionality) and `None` for a broker-mechanical cause, which must
    /// name one (never `operator_request`). The target must be a
    /// broker-verified capability: a typo'd kill fails loudly instead of
    /// poisoning an id. Re-revoking an already-closed id is permitted —
    /// kill-switch retries are idempotent in effect; the view takes the
    /// earliest offset.
    pub fn revoke_capability(
        &mut self,
        cap_id: &str,
        reason: &str,
        channel: Option<(&str, &str)>,
    ) -> Result<String, BrokerError> {
        let cap = self.load_capability(cap_id).map_err(|e| {
            BrokerError::InvalidRevocation(format!("target {cap_id} not found: {e}"))
        })?;
        let bound_manifest = cap["bound_manifest"].as_str().ok_or_else(|| {
            BrokerError::InvalidRevocation(format!("target {cap_id} has no bound_manifest"))
        })?.to_string();
        if reason.trim().is_empty() {
            return Err(BrokerError::InvalidRevocation("reason must be non-empty".into()));
        }
        if channel.is_none() && reason == "operator_request" {
            return Err(BrokerError::InvalidRevocation(
                "broker-originated revocation must carry a mechanical reason, never operator_request (§5.4)".into(),
            ));
        }
        let (chan, auth) = match channel {
            Some((c, a)) => (Value::String(c.into()), Value::String(a.into())),
            None => (Value::Null, Value::Null),
        };
        let sk = self.fabric.fabric_sk().clone();
        let span = self.fabric.substrate_span().to_string();
        let ev = trace::append(
            &mut self.fabric.conn,
            &sk,
            &span,
            Some(&bound_manifest),
            "revoke",
            json!({
                "capability": cap_id, "reason": reason,
                "channel": chan, "auth_strength": auth,
            }),
            &now_rfc3339(),
        )?;
        Ok(ev.id)
    }

    /// Derived display view: every stored capability whose ancestry passes
    /// through `cap_id` — the subtree a revoke of `cap_id` closes. Not
    /// enforcement (§5.4 cascade is definitional in the liveness view);
    /// the operator surface shows it so a kill is legible.
    pub fn capability_descendants(&self, cap_id: &str) -> Result<Vec<String>, BrokerError> {
        let mut stmt = self
            .fabric
            .conn
            .prepare("SELECT id, raw FROM objects WHERE kind = 'capability'")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let parent_of: BTreeMap<String, Option<String>> = rows
            .iter()
            .map(|(id, raw)| {
                let parent = canon::parse_fabric_json(raw)
                    .ok()
                    .and_then(|v| v["parent"].as_str().map(str::to_string));
                (id.clone(), parent)
            })
            .collect();
        let mut out = Vec::new();
        for id in parent_of.keys() {
            let mut cur = Some(id.clone());
            let mut hops = 0;
            while let Some(c) = cur {
                if c == cap_id && id != cap_id {
                    out.push(id.clone());
                    break;
                }
                hops += 1;
                if hops > parent_of.len() {
                    break; // cycle in stored rows: display view stays finite
                }
                cur = parent_of.get(&c).cloned().flatten();
            }
        }
        Ok(out)
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
        // Broker-minted and correctly typed or bust (F1/RF-19): a forged,
        // tampered, row-swapped, or mistyped capability is structurally dead.
        let cap = match self.load_capability(cap_id) {
            Ok(cap) => cap,
            Err(error) => {
                return self.deny(
                    cap_id,
                    tool,
                    action,
                    vec![],
                    Some(format!("capability object invalid (F1/RF-19): {error}")),
                    json!([]),
                )
            }
        };

        // A22/§5.4 structural precondition: closure and activation at the
        // current verified head, BEFORE caveat evaluation and before any
        // escalation could be enqueued — a closed capability's denial is
        // never escalatable, and exemptions are never consulted (closure
        // dominates). This is what denies a post-revoke call before any
        // effect; the gate's re-evaluation at the effect's durable
        // authorization offset is authoritative for what becomes durable.
        if let Err(why) = self.liveness_at_head(cap_id) {
            return self.deny(cap_id, tool, action, vec![], Some(why), json!([]));
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

        let authority = match self.w14_decision_authority(cap_id) {
            Ok(authority) => authority,
            Err(error) => {
                return self.deny(
                    cap_id,
                    tool,
                    action,
                    vec![],
                    Some(format!("signed decision authority unavailable: {error}")),
                    json!([]),
                )
            }
        };
        let mut meter = |key: &str| -> u64 {
            authority.meters.get(key).copied().unwrap_or(0)
        };
        // Peek only — a returned ticket is the in-memory reservation. Signed
        // result events make consumption durable; unsigned cache rows never
        // participate in authorization.
        let mut exempt = |key: &str| -> Option<String> {
            authority.exemptions.get(key).cloned()
        };

        let eval = evaluate::evaluate(&cap, &ctx, &mut meter, &mut exempt);
        let checks = evaluate::checks_json(&eval.checks);

        match eval.outcome {
            Outcome::Allow => {
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
        let (cap_id, manifest, key) = self.w14_pending_escalation(escalation)?;

        // approval.min_auth (§5.1): issuance and reconstruction share the
        // same strict signed-capability predicate.
        let cap = self.load_capability(&cap_id)?;
        w14_require_approval_strength(&cap_id, &cap, auth_strength)?;

        let now = now_rfc3339();
        let sk = self.fabric.fabric_sk().clone();
        let span = self.fabric.substrate_span().to_string();
        let tx = self.fabric.conn.transaction()?;
        trace::append_in_tx(
            &tx,
            &sk,
            &span,
            Some(&manifest),
            "approval",
            json!({
                "escalation": escalation, "resolution": resolution, "uses": uses,
                "capability": cap_id, "caveat": key,
                "channel": channel, "auth_strength": auth_strength,
            }),
            &now,
        )?;
        tx.execute(
            "UPDATE escalations SET status = ?2, decided_at = ?3
             WHERE id = ?1 AND cap = ?4 AND manifest = ?5 AND key = ?6",
            params![escalation, resolution, now, cap_id, manifest, key],
        )?;
        if resolution == "approved" && uses > 0 {
            tx.execute(
                "INSERT INTO exemptions (escalation, cap, key, remaining) VALUES (?1, ?2, ?3, ?4)",
                params![escalation, cap_id, key, uses],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Resolve the immutable authority binding for one escalation from the
    /// verified substrate chain. Mutable table status and selectors are never
    /// enough to create or redirect an approval.
    fn w14_pending_escalation(
        &self,
        escalation: i64,
    ) -> Result<(String, String, String), BrokerError> {
        let events = trace::verified_events(&self.fabric.conn, &self.fabric.fabric_vk())?;
        let ordered = w11_span_events_in_signed_sequence(&events, self.fabric.substrate_span());
        let mut binding: Option<(String, String, String)> = None;
        for event in ordered {
            let body = &event.raw["body"];
            if event.kind == "approval" {
                if body["escalation"].as_i64() == Some(escalation) {
                    return Err(BrokerError::NoSuchEscalation(escalation));
                }
                continue;
            }
            if event.kind != "escalation" {
                continue;
            }
            if body["escalation"].as_i64() != Some(escalation) {
                continue;
            }
            let Some(capability) = body["capability"].as_str() else {
                continue;
            };
            let Some(caveat) = body["caveat"].as_str() else {
                continue;
            };
            let Some(manifest) = event.manifest.as_deref() else {
                continue;
            };
            let candidate = (
                capability.to_string(),
                manifest.to_string(),
                caveat.to_string(),
            );
            if binding.as_ref().is_some_and(|old| old != &candidate) {
                return Err(BrokerError::GateTraceViolation(format!(
                    "signed escalation {escalation} has conflicting bindings"
                )));
            }
            binding = Some(candidate);
        }
        binding.ok_or(BrokerError::NoSuchEscalation(escalation))
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
        let p = self.pending[idx].clone();
        let appended = self.fabric.record_tool_call(
            &p.tool,
            &p.action,
            &p.args_raw,
            result,
            p.summary.clone(),
            p.checks.clone(),
            Some(&p.reversibility),
        )?;
        self.pending.remove(idx);
        Ok(appended)
    }
}

#[derive(Debug)]
pub enum PromotionOutcome {
    Applied { event: String },
    Parked { promotion: i64 },
}

const W14_PROMOTION_POLICY_CONTEXT: &str = "default-policy/manual-review/v1";

struct PromotionBinding {
    candidate_digest: String,
    branch_roots_digest: String,
}

struct PromotionApproval {
    id: i64,
    channel: String,
    auth_strength: String,
    candidate_digest: String,
}

fn w14_promotion_binding(
    id: i64,
    manifest: &str,
    preview: &Value,
) -> Result<PromotionBinding, BrokerError> {
    let branch_roots = preview["branch_roots"].as_object().ok_or_else(|| {
        BrokerError::MalformedPromotion {
            id,
            detail: "preview has no exact branch_roots map".into(),
        }
    })?;
    let branch_roots_digest = canon::sha256_hex(&canon::jcs_bytes(&Value::Object(
        branch_roots.clone(),
    ))?);
    let candidate = json!({
        "promotion": id,
        "manifest": manifest,
        "preview": preview,
        "policy_context": W14_PROMOTION_POLICY_CONTEXT,
    });
    Ok(PromotionBinding {
        candidate_digest: canon::sha256_hex(&canon::jcs_bytes(&candidate)?),
        branch_roots_digest,
    })
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

/// Count one already-authorized call into the decision-time reservation
/// view. Signed calls and in-memory pending calls use the same representation,
/// so a dispatch ticket reserves budget before its result event exists.
#[allow(clippy::collapsible_if)] // G9: prefix and applies are independently mutation-tested edges.
fn w14_reserve_signed_checks(
    cap_id: &str,
    checks: &Value,
    meters: &mut BTreeMap<String, u64>,
    remaining: &mut BTreeMap<(String, String, i64), i64>,
) -> Result<(), BrokerError> {
    let Some(checks) = checks.as_array() else {
        return Err(BrokerError::GateTraceViolation(format!(
            "authorized call for {cap_id} has no signed check array"
        )));
    };
    for check in checks {
        let Some(caveat) = check["caveat"].as_str() else {
            return Err(BrokerError::GateTraceViolation(format!(
                "authorized call for {cap_id} has a malformed caveat check"
            )));
        };
        if caveat.starts_with("budget.count:") {
            if check["meter"].get("applies") != Some(&Value::Bool(false)) {
                *meters.entry(caveat.to_string()).or_insert(0) += 1;
            }
        }
        let Some(exemption) = check["meter"]["approved_exemption"].as_str() else {
            continue;
        };
        let Some(id) = exemption.strip_prefix("esc:").and_then(|id| id.parse::<i64>().ok()) else {
            return Err(BrokerError::GateTraceViolation(format!(
                "authorized call for {cap_id} names malformed approval {exemption}"
            )));
        };
        let Some(uses) = remaining.get_mut(&(cap_id.to_string(), caveat.to_string(), id)) else {
            return Err(BrokerError::GateTraceViolation(format!(
                "authorized call for {cap_id} consumes unsigned or mismatched escalation {id}"
            )));
        };
        if *uses <= 0 {
            return Err(BrokerError::GateTraceViolation(format!(
                "authorized call for {cap_id} exceeds signed approval {id}"
            )));
        }
        *uses -= 1;
    }
    Ok(())
}

/// Consumers within one verified span use that span's signed sequence, never
/// the unsigned cross-span substrate offset (RF-16/SI-25).
fn w11_span_events_in_signed_sequence<'a>(
    events: &'a [trace::VerifiedEvent],
    span: &str,
) -> Vec<&'a trace::VerifiedEvent> {
    let mut ordered = events
        .iter()
        .filter(|event| event.span == span)
        .collect::<Vec<_>>();
    ordered.sort_by_key(|event| event.seq);
    ordered
}

/// Final call is the last tool call in the authenticated span sequence.
fn w11_final_tool_call<'a>(
    events: &'a [trace::VerifiedEvent],
    span: &str,
) -> Option<&'a trace::VerifiedEvent> {
    w11_span_events_in_signed_sequence(events, span)
        .into_iter()
        .rev()
        .find(|event| event.kind == "tool_call")
}

/// A21/M7 activation view: the earliest verified grant offset for each
/// capability bound to this manifest. Kept as a small pure helper so the
/// authority-binding predicates have a stable mutation-testing target.
fn m7_grant_offsets(
    events: &[trace::VerifiedEvent],
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

// ---- A22/§5.4 closure: the pure event-derived authority view ------------
//
// One reconstruction shared verbatim by decision time (evaluated at the
// current verified head) and gate replay (evaluated at each effect's
// durable authorization offset — today the tool_call event itself).
// Closure is a structural precondition in front of caveat evaluation,
// never a §5.1 caveat dimension, so pure-evaluator consumers (the corpus
// harness) are untouched. The a22_* functions are the stable mutation-lane
// surface, the closure duals of the m7_* activation predicates above.

/// Earliest verified revoke offset per capability id, resolved across the
/// ENTIRE substrate — never filtered by span or manifest. §5.4 doubt-never-
/// widens: a signature-verified revoke closes the capability it names even
/// when its manifest field or span placement is anomalous. `VerifiedEvent`
/// proves signed-raw/index agreement before this pure view runs; wholly
/// unsigned rows never enter it.
fn a22_revoke_offsets(events: &[trace::VerifiedEvent]) -> BTreeMap<String, i64> {
    let mut revokes = BTreeMap::new();
    for ev in events {
        if ev.kind == "revoke" {
            if let Some(cap) = ev.raw["body"]["capability"].as_str() {
                revokes.entry(cap.to_string()).or_insert(ev.offset);
            }
        }
    }
    revokes
}

/// Earliest verified grant offset per (capability, manifest) binding —
/// A21's exact-binding activation edge for every link of an ancestry chain
/// (`m7_grant_offsets` is the per-manifest projection of the same edge).
/// Activation doubt resolves to not-granted: unverifiable rows activate
/// nothing; the binding manifest and span are read from the SIGNED raw,
/// never the index columns; and a grant activates ONLY from the
/// fabric-lifetime span (§6 records authority acts there — a signed grant
/// parked on a session span is an anomaly, and anomalies never widen
/// authority). The deliberate asymmetry with [`a22_revoke_offsets`]:
/// closure tolerates placement anomalies (doubt narrows), activation
/// demands exact form.
type A22GrantBindings = BTreeMap<(String, String, Option<String>), i64>;

fn a22_grant_bindings(
    events: &[trace::VerifiedEvent],
    substrate_span: &str,
) -> A22GrantBindings {
    let mut grants = BTreeMap::new();
    for ev in events {
        if ev.kind == "grant" && ev.span == substrate_span {
            let parent = match ev.raw["body"].get("parent") {
                Some(Value::Null) => Some(None),
                Some(Value::String(parent)) => Some(Some(parent.clone())),
                _ => None,
            };
            if let (Some(cap), Some(man), Some(parent)) = (
                ev.raw["body"]["capability"].as_str(),
                ev.manifest.as_deref(),
                parent,
            ) {
                grants
                    .entry((cap.to_string(), man.to_string(), parent))
                    .or_insert(ev.offset);
            }
        }
    }
    grants
}

/// Fetch and signature-verify the attenuation ancestry of `cap_id`, leaf
/// first, root last — (id, bound_manifest) per hop. Fails closed on a
/// missing or unverifiable link and on a parent cycle.
fn a22_ancestry(
    conn: &rusqlite::Connection,
    vk: &ed25519_dalek::VerifyingKey,
    cap_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    let mut cur = cap_id.to_string();
    loop {
        if !seen.insert(cur.clone()) {
            return Err(format!(
                "ancestry of {cap_id} contains a cycle at {cur} (fail closed)"
            ));
        }
        let obj = trace::load_verified_object(conn, &cur, "cap", "capability", vk).map_err(
            |e| {
                format!(
                    "capability {cur} in ancestry of {cap_id} unavailable or mistyped: {e} (fail closed)"
                )
            },
        )?;
        let man = obj["bound_manifest"]
            .as_str()
            .ok_or_else(|| format!("capability {cur} has no bound_manifest (fail closed)"))?
            .to_string();
        let parent = obj["parent"].as_str().map(str::to_string);
        chain.push((cur.clone(), man));
        match parent {
            Some(p) => cur = p,
            None => return Ok(chain),
        }
    }
}

/// §5.4 liveness of the leaf of `chain` (leaf-first, from [`a22_ancestry`])
/// for an operation whose durable authorization offset is `o`:
///
/// 1. the leaf's exact-binding grant precedes `o` (M7's activation edge);
/// 2. no verified revoke names ANY link at an offset before `o` — the
///    quantifier is over all revokes, so closure is permanent per id
///    (revoked-before-granted is equally dead) and the ancestor clause IS
///    the descendant cascade;
/// 4. every link carries its exact-binding grant and earliest grants are
///    well-ordered along the chain (each ancestor's precedes its child's).
///
/// Condition 3 (expiry + ordinary caveats at the SI-22 clock) stays in the
/// pure evaluator. §5.2 subset semantics stay mint-time-enforced and
/// F1-signature-protected — not re-derived here.
fn a22_state_at(
    chain: &[(String, String)],
    o: i64,
    grants: &A22GrantBindings,
    revokes: &BTreeMap<String, i64>,
) -> Result<(), String> {
    // Closure first: it dominates activation and is checked before any
    // caveat or exemption is consulted.
    for (id, _) in chain {
        if let Some(r) = revokes.get(id) {
            if *r < o {
                return Err(format!(
                    "capability {id} revoked at offset {r} (§5.4; closure is permanent for the id)"
                ));
            }
        }
    }
    // Activation: exact-binding grants, well-ordered leaf-ward (walking
    // leaf → root, grant offsets strictly decrease). Each signed grant's
    // parent must exactly match the signed capability ancestry (§6).
    let mut child_grant: Option<i64> = None;
    let mut leaf_grant: Option<i64> = None;
    for (index, (id, man)) in chain.iter().enumerate() {
        let expected_parent = chain.get(index + 1).map(|(parent, _)| parent.clone());
        let g = grants
            .get(&(id.clone(), man.clone(), expected_parent.clone()))
            .ok_or_else(|| {
                format!(
                    "no verified grant binds {id} to {man} with parent {expected_parent:?} (A21/M7, fail closed)"
                )
            })?;
        if index == 0 {
            leaf_grant = Some(*g);
        }
        if let Some(cg) = child_grant {
            if *g >= cg {
                return Err(format!(
                    "ancestry grants out of order: {id} granted at {g}, its child at {cg} (§5.4, fail closed)"
                ));
            }
        }
        child_grant = Some(*g);
    }
    match chain.first() {
        Some((leaf, _)) => {
            let g = leaf_grant.expect("non-empty chain set leaf grant");
            if g < o {
                Ok(())
            } else {
                Err(format!(
                    "operation at offset {o} precedes the grant of {leaf} at {g} (M7 ordering)"
                ))
            }
        }
        None => Err("empty capability ancestry (fail closed)".into()),
    }
}

impl Broker {
    /// Read-only gate replay: re-verify a manifest's recorded trace against
    /// its authority exactly as promotion would — spans, M7 activation,
    /// §5.4 closure at each effect's offset, and full caveat re-evaluation
    /// — without merging, locking, or mutating anything. The W-8
    /// gate-replay measurement lane and operator diagnostics drive this.
    pub fn gate_replay_check(&self, manifest_id: &str) -> Result<Value, BrokerError> {
        let man = self.fabric.load_manifest(manifest_id)?;
        let span = man["trace"]["span"]
            .as_str()
            .ok_or_else(|| {
                KernelError::MalformedManifest(manifest_id.into(), "no trace.span".into())
            })?
            .to_string();
        self.gate_trace_check(manifest_id, &span)
    }

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
        // Reject an invalid authority object before drift accounting can
        // append evidence for a promotion that never existed.
        let man = self.fabric.load_manifest(manifest_id)?;
        // M8 (A20): attribute any out-of-band divergence BEFORE the merge
        // consumes live trunk, under the per-home gate lock — attribution
        // must not depend on when the human edited relative to the session.
        let _gate = self.fabric.gate_lock()?;
        self.fabric.check_drift()?;
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
            let event = self.apply_plan(manifest_id, &plan, &trace_report, "auto", None)?;
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
            let binding = w14_promotion_binding(id, manifest_id, &preview)?;
            let sk = self.fabric.fabric_sk().clone();
            let sspan = self.fabric.substrate_span().to_string();
            trace::append(
                &mut self.fabric.conn,
                &sk,
                &sspan,
                Some(manifest_id),
                "escalation",
                json!({
                    "promotion": id,
                    "manifest": manifest_id,
                    "candidate_digest": binding.candidate_digest,
                    "branch_roots_digest": binding.branch_roots_digest,
                    "policy_context": W14_PROMOTION_POLICY_CONTEXT,
                    "caveat": "promotion.policy",
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

        // A21/M7: the manifest's declared authority mode. Brokered fails
        // closed below — every effect capability-attributed, and its grant
        // event at a lower substrate offset. Observed claims nothing;
        // attributed calls are still checked in full.
        let man = self.fabric.load_manifest(manifest_id).map_err(|e| {
            BrokerError::GateTraceViolation(format!(
                "manifest {manifest_id} unavailable, mistyped, or unverifiable: {e}"
            ))
        })?;
        let brokered = man["authority"]["mode"] == "brokered";

        // Approved budget headroom: approval events grant `uses` against an
        // escalation's (cap, caveat-key). The authority binding comes from
        // the signed event itself, never the mutable escalation table.
        let substrate_span = self.fabric.substrate_span();
        trace::verify_span(&self.fabric.conn, &self.fabric.fabric_vk(), substrate_span)?;
        let mut approved: BTreeMap<(String, String), i64> = BTreeMap::new();
        // RF-16: authority never consumes EventRow selectors directly. The
        // global scan ignores wholly unsigned rows, but any signature-valid
        // row whose selectors disagree fails the gate before a merge.
        let all_events = trace::verified_events(&self.fabric.conn, &self.fabric.fabric_vk())?;
        let events = w11_span_events_in_signed_sequence(&all_events, span);
        // Grant activation offsets per capability for THIS manifest, from
        // the verified substrate span (M7 forward edge).
        let grants = m7_grant_offsets(&all_events, substrate_span, manifest_id);
        // A22/§5.4: the closure view, shared verbatim with decision time.
        // Revokes resolve by capability id across the whole substrate —
        // never filtered by the evaluating manifest (an M2 sub-agent child
        // dies with its ancestor's revoke); grants activate only from the
        // fabric-lifetime span (activation demands exact form).
        let a22_grants = a22_grant_bindings(&all_events, substrate_span);
        let a22_revokes = a22_revoke_offsets(&all_events);
        // §5.4 doubt-never-widens, loud side: a verified revoke with an
        // anomalous body or placement still closes — but every
        // inconsistency surfaces here as substrate-integrity signal,
        // ledger-visible through the promotion event's trace_check:
        // manifest field vs the target's bound_manifest, emission on an
        // unexpected span, C1 provenance pairing (channel and
        // auth_strength travel together), a missing target, or an empty
        // reason. A kill that needed anomaly tolerance to land is a kill
        // the operator should hear about.
        let mut closure_anomalies: Vec<String> = Vec::new();
        for ev in &all_events {
            if ev.kind != "revoke" {
                continue;
            }
            let body = &ev.raw["body"];
            let Some(target) = body["capability"].as_str() else {
                closure_anomalies.push(format!(
                    "revoke {} names no capability — it closes nothing (§5.4)",
                    ev.id
                ));
                continue;
            };
            if ev.span != substrate_span {
                closure_anomalies.push(format!(
                    "revoke {} of {target} emitted on span {} instead of the fabric-lifetime span — closure holds (§5.4)",
                    ev.id,
                    ev.span,
                ));
            }
            match self.load_capability(target) {
                Ok(obj) => {
                    let bound = obj["bound_manifest"].as_str();
                    let stamped = ev.raw["manifest"].as_str();
                    if bound.is_some() && stamped != bound {
                        closure_anomalies.push(format!(
                            "revoke {} stamps manifest {} but {target} is bound to {} — closure holds (§5.4)",
                            ev.id,
                            stamped.unwrap_or("null"),
                            bound.unwrap_or("?"),
                        ));
                    }
                }
                Err(error) => closure_anomalies.push(format!(
                    "revoke {} targets unavailable, mistyped, or unverifiable capability {target}: {error} — closure holds (§5.4)",
                    ev.id
                )),
            }
            let chan = body["channel"].as_str();
            let auth = body["auth_strength"].as_str();
            if chan.is_some() != auth.is_some() {
                closure_anomalies.push(format!(
                    "revoke {} of {target} has unpaired C1 provenance (channel {}, auth_strength {}) — closure holds (§5.4)",
                    ev.id,
                    chan.unwrap_or("null"),
                    auth.unwrap_or("null"),
                ));
            }
            if body["reason"].as_str().is_none_or(|r| r.trim().is_empty()) {
                closure_anomalies.push(format!(
                    "revoke {} of {target} carries no reason — closure holds (§5.4)",
                    ev.id
                ));
            }
        }
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
        // Verified ancestry per capability (§5.4 condition 4), cached —
        // chains are short and effects reuse their capability.
        let mut chains: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
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
                    let c = self.load_capability(cap_id).map_err(|e| {
                        violation(format!(
                            "capability {cap_id} unavailable, mistyped, or unverifiable: {e}"
                        ))
                    })?;
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

            // A22/§5.4: liveness at the effect's durable authorization
            // row offset — unsigned pending SI-25, and never gate time — so a
            // revoke never retro-fails honest pre-revoke work (parked
            // promotions of pre-revoke work stay approvable) and work
            // recorded after a revoke conservatively strands. Attributed
            // calls are checked in full regardless of authority mode: the
            // declaration only ever adds constraints (M7).
            let chain = match chains.get(cap_id) {
                Some(c) => c.clone(),
                None => {
                    let c = a22_ancestry(&self.fabric.conn, &self.fabric.fabric_vk(), cap_id)
                        .map_err(&violation)?;
                    chains.insert(cap_id.into(), c.clone());
                    c
                }
            };
            a22_state_at(&chain, ev.offset, &a22_grants, &a22_revokes)
                .map_err(|e| violation(format!("event {}: {e}", ev.id)))?;

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
            "closure_anomalies": closure_anomalies,
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
        let events = trace::verified_events(&self.fabric.conn, &self.fabric.fabric_vk())?;
        let last_call = w11_final_tool_call(&events, span);
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
        approval: Option<PromotionApproval>,
    ) -> Result<String, BrokerError> {
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
        let transitions = plan
            .stores
            .iter()
            .map(|store| StateRootTransition {
                store: store.store.clone(),
                before: store.trunk.clone(),
                after: store.install.clone().unwrap_or_else(|| store.trunk.clone()),
            })
            .collect();
        let event_body = json!({
            "manifest": manifest_id,
            "stores": stores_json,
            "ops": plan.ops.iter().map(Op::to_json).collect::<Vec<_>>(),
            "conflicts": plan.conflicts.iter().map(Conflict::to_json).collect::<Vec<_>>(),
            "trace_check": trace_report,
            "policy": policy,
        });
        let sk = self.fabric.fabric_sk().clone();
        let span = self.fabric.substrate_span().to_string();
        let manifest = manifest_id.to_string();
        let event = self.fabric.commit_state_change(
            StateChangePlan {
                manifest: manifest_id.to_string(),
                event_kind: "promotion".into(),
                event_body,
                transitions,
                set_current_manifest: false,
            },
            move |tx, _| {
                if let Some(approval) = approval {
                    let now = now_rfc3339();
                    tx.execute(
                        "UPDATE promotions SET status = 'applied', decided_at = ?2
                         WHERE id = ?1 AND manifest = ?3",
                        params![approval.id, now, manifest],
                    )?;
                    trace::append_in_tx(
                        tx,
                        &sk,
                        &span,
                        Some(&manifest),
                        "approval",
                        json!({
                            "promotion": approval.id,
                            "resolution": "approved",
                            "candidate_digest": approval.candidate_digest,
                            "channel": approval.channel,
                            "auth_strength": approval.auth_strength,
                        }),
                        &now,
                    )?;
                }
                Ok(())
            },
        )?;
        Ok(event.id)
    }

    pub fn list_promotions(&self, status: &str) -> Result<Vec<Value>, BrokerError> {
        let rows = {
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
            rows.collect::<Result<Vec<_>, _>>()?
        };
        if status == "pending" {
            for row in &rows {
                self.w14_verify_promotion_candidate(
                    row["id"].as_i64().unwrap_or_default(),
                    row["manifest"].as_str().unwrap_or_default(),
                    &row["preview"],
                )?;
            }
        }
        Ok(rows)
    }

    /// Prove that the mutable parked row is byte-for-byte the candidate bound
    /// by a still-unresolved signed escalation on the substrate chain.
    fn w14_verify_promotion_candidate(
        &self,
        id: i64,
        manifest: &str,
        preview: &Value,
    ) -> Result<PromotionBinding, BrokerError> {
        let binding = w14_promotion_binding(id, manifest, preview)?;
        let events = trace::verified_events(&self.fabric.conn, &self.fabric.fabric_vk())?;
        let ordered = w11_span_events_in_signed_sequence(&events, self.fabric.substrate_span());
        let mut matched = false;
        for event in ordered {
            let body = &event.raw["body"];
            if event.kind == "approval" {
                if body["promotion"].as_i64() == Some(id) {
                    return Err(BrokerError::NoSuchPromotion(id));
                }
                continue;
            }
            if event.kind != "escalation" {
                continue;
            }
            if body["promotion"].as_i64() != Some(id) {
                continue;
            }
            let exact = event.manifest.as_deref() == Some(manifest)
                && body["manifest"].as_str() == Some(manifest)
                && body["candidate_digest"].as_str()
                    == Some(binding.candidate_digest.as_str())
                && body["branch_roots_digest"].as_str()
                    == Some(binding.branch_roots_digest.as_str())
                && body["policy_context"].as_str() == Some(W14_PROMOTION_POLICY_CONTEXT);
            if !exact {
                return Err(BrokerError::MalformedPromotion {
                    id,
                    detail: "mutable candidate does not match its signed escalation binding"
                        .into(),
                });
            }
            if matched {
                return Err(BrokerError::MalformedPromotion {
                    id,
                    detail: "multiple signed candidate bindings".into(),
                });
            }
            matched = true;
        }
        if !matched {
            return Err(BrokerError::MalformedPromotion {
                id,
                detail: "no signed escalation binds this candidate".into(),
            });
        }
        Ok(binding)
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
                "SELECT manifest, preview FROM promotions WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (manifest_id, preview_raw) = row.ok_or(BrokerError::NoSuchPromotion(id))?;
        let preview: Value =
            serde_json::from_str(&preview_raw).map_err(|e| BrokerError::MalformedPromotion {
                id,
                detail: format!("preview is not JSON: {e}"),
            })?;
        let binding = self.w14_verify_promotion_candidate(id, &manifest_id, &preview)?;
        self.check_min_auth_for_manifest(&manifest_id, auth_strength)?;

        // As at the automatic gate, authenticate the authority object before
        // a rejected approval can cause drift-accounting side effects.
        let man = self.fabric.load_manifest(&manifest_id)?;

        // M8 (A20): the approval-time re-merge consumes live trunk exactly
        // like the auto gate does — same divergence check, same lock.
        let _gate = self.fabric.gate_lock()?;
        self.fabric.check_drift()?;

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
            Some(PromotionApproval {
                id,
                channel: channel.to_string(),
                auth_strength: auth_strength.to_string(),
                candidate_digest: binding.candidate_digest,
            }),
        )?;
        Ok(event)
    }

    pub fn reject_promotion(
        &mut self,
        id: i64,
        channel: &str,
        auth_strength: &str,
    ) -> Result<(), BrokerError> {
        let row: Option<(String, String)> = self
            .fabric
            .conn
            .query_row(
                "SELECT manifest, preview FROM promotions WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (manifest, preview_raw) = row.ok_or(BrokerError::NoSuchPromotion(id))?;
        let preview: Value =
            serde_json::from_str(&preview_raw).map_err(|error| BrokerError::MalformedPromotion {
                id,
                detail: format!("preview is not JSON: {error}"),
            })?;
        let binding = self.w14_verify_promotion_candidate(id, &manifest, &preview)?;
        let now = now_rfc3339();
        let sk = self.fabric.fabric_sk().clone();
        let sspan = self.fabric.substrate_span().to_string();
        let tx = self.fabric.conn.transaction()?;
        trace::append_in_tx(
            &tx,
            &sk,
            &sspan,
            Some(&manifest),
            "approval",
            json!({ "promotion": id, "resolution": "denied",
                    "candidate_digest": binding.candidate_digest,
                    "channel": channel, "auth_strength": auth_strength }),
            &now,
        )?;
        tx.execute(
            "UPDATE promotions SET status = 'rejected', decided_at = ?2
             WHERE id = ?1 AND manifest = ?3",
            params![id, now, manifest],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn check_min_auth_for_manifest(
        &self,
        manifest_id: &str,
        auth_strength: &str,
    ) -> Result<(), BrokerError> {
        // Strongest approval.min_auth among activated capabilities bound to
        // this manifest applies to gate approvals too. Capability ids come
        // from verified signed grants, not the unsigned objects.kind index;
        // every referenced object must pass the shared typed loader.
        let events = trace::verified_events(&self.fabric.conn, &self.fabric.fabric_vk())?;
        let granted: BTreeSet<String> = a22_grant_bindings(&events, self.fabric.substrate_span())
            .keys()
            .filter(|(_, manifest, _)| manifest == manifest_id)
            .map(|(capability, _, _)| capability.clone())
            .collect();
        let mut ranked = BTreeMap::new();
        for cap_id in granted {
            let cap = self.load_capability(&cap_id)?;
            if cap["bound_manifest"].as_str() != Some(manifest_id) {
                return Err(BrokerError::GateTraceViolation(format!(
                    "grant binds capability {cap_id} to {manifest_id}, but its signed object binds a different manifest"
                )));
            }
            if let Some((rank, minimum)) = w14_required_approval_strength(&cap_id, &cap)? {
                ranked.insert(rank, minimum);
            }
        }
        if let Some((need_rank, need)) = ranked.into_iter().next_back() {
            match auth_rank(auth_strength) {
                Some(have_rank) if have_rank >= need_rank => {}
                _ => {
                    return Err(BrokerError::ChannelTooWeak {
                        have: auth_strength.into(),
                        need,
                    });
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
