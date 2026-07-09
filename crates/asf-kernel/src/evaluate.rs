//! Call-time capability evaluation. Spec §5, §5.1; brief §5.4 layer 1.
//!
//! Deterministic, injection-proof, conjunctive: every caveat on the
//! capability is checked; ALL must pass. Unknown dimensions fail closed and
//! are never escalatable — we cannot safely ask a human to waive a
//! restriction we cannot explain. Structural failures (expiry, M2 manifest
//! binding) deny outright and bypass escalation entirely.
//!
//! The evaluator is pure: meters and approval exemptions come in as
//! functions, decisions go out as data. The broker owns the side effects.

use crate::capability::{
    auth_rank, caveat_key, glob_matches, reach_rank, reversibility_rank,
};
use serde_json::{json, Value};

/// Everything the evaluator may know about the proposed call. Built by the
/// broker exclusively from *registered* tool metadata and broker-observed
/// state — never from agent-supplied claims (the A8 principle applied to
/// evaluation generally).
pub struct CallCtx<'a> {
    pub tool: &'a str,
    pub action: &'a str,
    /// From registration, conservative defaults already applied (§4).
    pub reversibility: &'a str,
    pub side_effect: &'a str,
    pub action_class: &'a str,
    /// `None` = the action writes no paths; `Some(paths)` = it writes
    /// exactly these (broker-extracted). `Some(empty)` fails closed.
    pub write_paths: Option<Vec<String>>,
    pub now: &'a str,
    /// The fabric's current manifest id (M2).
    pub current_manifest: &'a str,
}

#[derive(Debug, Clone)]
pub struct Check {
    /// Caveat key (dim, or dim:class for budgets).
    pub caveat: String,
    pub ok: bool,
    /// Meter / explanation payload recorded in the trace (§6 checks).
    pub meter: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Allow,
    /// Denied outright: structural failure or non-escalatable caveat.
    Deny { failed: Vec<String>, structural: Option<String> },
    /// All failures sit in on_violation.escalatable — park for a human.
    Escalate { failed: Vec<String> },
}

#[derive(Debug, Clone)]
pub struct Evaluation {
    pub checks: Vec<Check>,
    pub outcome: Outcome,
    /// `(caveat_key, escalation_id)` for each approval exemption that made a
    /// failing check pass. The evaluator is side-effect-free (RF-2): it only
    /// *reports* what would be consumed; the broker commits the decrement
    /// solely on `Outcome::Allow`, so a denied call never burns an exemption.
    pub consumed_exemptions: Vec<(String, String)>,
}

/// Evaluate `cap` against a proposed call.
/// `meter_used(key)` returns prior consumption for a budget caveat key;
/// `exempt(key)` *peeks* whether an unconsumed approval exemption is
/// available for the key and returns its escalation id — it MUST NOT consume
/// (RF-2). Consumption is the broker's, and only on Allow.
pub fn evaluate(
    cap: &Value,
    ctx: &CallCtx<'_>,
    meter_used: &mut dyn FnMut(&str) -> u64,
    exempt: &mut dyn FnMut(&str) -> Option<String>,
) -> Evaluation {
    // ---- structural gates: deny outright, never escalate --------------
    let structural_deny = |why: String| Evaluation {
        checks: vec![],
        outcome: Outcome::Deny { failed: vec![], structural: Some(why) },
        consumed_exemptions: vec![],
    };
    match cap.get("expires_at").and_then(Value::as_str) {
        Some(exp) if ctx.now <= exp => {}
        Some(exp) => return structural_deny(format!("capability expired at {exp}")),
        None => return structural_deny("capability has no expiry (mandatory, §5)".into()),
    }
    match cap.get("bound_manifest").and_then(Value::as_str) {
        Some(m) if m == ctx.current_manifest => {}
        Some(m) => {
            return structural_deny(format!(
                "M2 violation: capability bound to {m}, current manifest is {}",
                ctx.current_manifest
            ))
        }
        None => return structural_deny("capability has no bound_manifest (M2)".into()),
    }

    // ---- conjunctive caveat checks -------------------------------------
    let empty = Vec::new();
    let caveats = cap.get("caveats").and_then(Value::as_array).unwrap_or(&empty);
    let mut checks = Vec::new();
    let mut unknown_failed = false;

    for cav in caveats {
        let key = match caveat_key(cav) {
            Ok(k) => k,
            Err(_) => {
                checks.push(Check {
                    caveat: format!("{cav}"),
                    ok: false,
                    meter: json!({"reason": "malformed caveat (fail closed)"}),
                });
                unknown_failed = true;
                continue;
            }
        };
        let dim = cav["dim"].as_str().expect("keyed caveat has dim");
        let check = match dim {
            "action.allow" => {
                let tools = cav["tools"].as_array().map(|a| a.iter().any(|t| t == ctx.tool));
                let actions = cav["actions"].as_array().map(|a| a.iter().any(|x| x == ctx.action));
                Check {
                    caveat: key.clone(),
                    ok: tools == Some(true) && actions == Some(true),
                    meter: Value::Null,
                }
            }
            "reversibility.max" => {
                let max = cav["max"].as_str().and_then(reversibility_rank);
                let have = reversibility_rank(ctx.reversibility);
                Check {
                    caveat: key.clone(),
                    ok: matches!((max, have), (Some(m), Some(h)) if h <= m),
                    meter: json!({"action_class": ctx.reversibility, "max": cav["max"]}),
                }
            }
            "external_reach" => {
                let allowed = cav["mode"].as_str().and_then(reach_rank);
                let ok = if ctx.side_effect == "external" {
                    // Stage 2 has no mock router: an external call needs "live".
                    allowed == reach_rank("live")
                } else {
                    true
                };
                Check { caveat: key.clone(), ok, meter: json!({"mode": cav["mode"]}) }
            }
            "budget.count" => {
                let applies = cav["action_class"].as_str() == Some(ctx.action_class);
                if !applies {
                    Check { caveat: key.clone(), ok: true, meter: json!({"applies": false}) }
                } else {
                    let max = cav["max"].as_u64().unwrap_or(0);
                    let window = cav["window"].as_str().unwrap_or("");
                    if window != "run" {
                        // Only the per-capability "run" window is metered in
                        // Stage 2; anything else fails closed, honestly.
                        Check {
                            caveat: key.clone(),
                            ok: false,
                            meter: json!({"reason": format!("window '{window}' unsupported (fail closed)")}),
                        }
                    } else {
                        let used = meter_used(&key);
                        Check {
                            caveat: key.clone(),
                            ok: used < max,
                            meter: json!({"used": used, "max": max}),
                        }
                    }
                }
            }
            "paths.write" => {
                let globs: Vec<&str> = cav["globs"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                let (ok, meter) = match &ctx.write_paths {
                    None => (true, json!({"applies": false})),
                    Some(paths) if paths.is_empty() => (
                        false,
                        json!({"reason": "write action with no extractable paths (fail closed)"}),
                    ),
                    Some(paths) => {
                        let bad: Vec<&String> = paths
                            .iter()
                            .filter(|p| !globs.iter().any(|g| glob_matches(g, p)))
                            .collect();
                        (bad.is_empty(), json!({"paths": paths, "out_of_scope": bad}))
                    }
                };
                Check { caveat: key.clone(), ok, meter }
            }
            "time" => {
                let ok_before = cav
                    .get("not_before")
                    .and_then(Value::as_str)
                    .map(|nb| ctx.now >= nb)
                    .unwrap_or(true);
                let ok_after = cav
                    .get("not_after")
                    .and_then(Value::as_str)
                    .map(|na| ctx.now <= na)
                    .unwrap_or(true);
                Check { caveat: key.clone(), ok: ok_before && ok_after, meter: Value::Null }
            }
            "approval.min_auth" => {
                // Governs the approval/amendment channel, not the call itself;
                // recorded so the trace shows it was present. Validity gate:
                // an unrankable strength would fail closed at approval time.
                let ok = cav["min"].as_str().and_then(auth_rank).is_some();
                Check { caveat: key.clone(), ok, meter: json!({"gates": "approvals"}) }
            }
            other => {
                unknown_failed = true;
                Check {
                    caveat: key.clone(),
                    ok: false,
                    meter: json!({"reason": format!("unknown dimension '{other}' (fail closed, §0)")}),
                }
            }
        };
        checks.push(check);
    }

    // ---- approval exemptions (A9) --------------------------------------
    // A failing, escalatable check with a granted exemption flips to ok;
    // unknown dimensions never flip.
    let escalatable: Vec<String> = cap["on_violation"]["escalatable"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let mut consumed_exemptions = Vec::new();
    for c in checks.iter_mut() {
        if !c.ok
            && escalatable.contains(&c.caveat)
            && c.meter.get("reason").and_then(Value::as_str).map(|r| r.contains("unknown")) != Some(true)
        {
            if let Some(esc_id) = exempt(&c.caveat) {
                c.ok = true;
                consumed_exemptions.push((c.caveat.clone(), esc_id.clone()));
                c.meter = json!({"approved_exemption": esc_id, "was": c.meter});
            }
        }
    }

    let failed: Vec<String> =
        checks.iter().filter(|c| !c.ok).map(|c| c.caveat.clone()).collect();
    let outcome = if failed.is_empty() {
        Outcome::Allow
    } else if !unknown_failed && failed.iter().all(|k| escalatable.contains(k)) {
        Outcome::Escalate { failed }
    } else {
        Outcome::Deny { failed, structural: None }
    };
    // consumed_exemptions is reported regardless of outcome; the broker
    // consumes it only when `outcome == Allow` (RF-2). On Deny/Escalate the
    // peeked exemptions are left untouched.
    Evaluation { checks, outcome, consumed_exemptions }
}

/// Render checks for the §6 tool_call body.
pub fn checks_json(checks: &[Check]) -> Value {
    Value::Array(
        checks
            .iter()
            .map(|c| json!({"caveat": c.caveat, "ok": c.ok, "meter": c.meter}))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cap(escalatable: Vec<&str>, extra_caveats: Vec<Value>) -> Value {
        let mut caveats = vec![
            json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.write","note.read"]}),
            json!({"dim":"reversibility.max","max":"compensable"}),
            json!({"dim":"paths.write","globs":["inbox/**"]}),
            json!({"dim":"budget.count","action_class":"write","max":2,"window":"run"}),
        ];
        caveats.extend(extra_caveats);
        json!({
            "id": "cap:test", "holder": "prin:agent", "bound_manifest": "man:current",
            "issued_at": "2026-07-08T00:00:00Z", "expires_at": "2026-07-08T12:00:00Z",
            "caveats": caveats,
            "on_violation": { "default": "deny", "escalatable": escalatable },
        })
    }

    fn ctx<'a>(action: &'a str, paths: Option<Vec<String>>) -> CallCtx<'a> {
        CallCtx {
            tool: "tool:vault@1.0",
            action,
            reversibility: "reversible",
            side_effect: "local",
            action_class: "write",
            write_paths: paths,
            now: "2026-07-08T06:00:00Z",
            current_manifest: "man:current",
        }
    }

    fn eval(cap: &Value, ctx: &CallCtx, used: u64) -> Evaluation {
        evaluate(cap, ctx, &mut |_| used, &mut |_| None)
    }

    #[test]
    fn clean_call_allows_with_full_check_record() {
        let e = eval(&cap(vec![], vec![]), &ctx("note.write", Some(vec!["inbox/a.md".into()])), 0);
        assert_eq!(e.outcome, Outcome::Allow);
        assert_eq!(e.checks.len(), 4);
        assert!(e.checks.iter().all(|c| c.ok));
    }

    #[test]
    fn out_of_scope_path_denies() {
        let e = eval(&cap(vec![], vec![]), &ctx("note.write", Some(vec!["secrets/key.md".into()])), 0);
        assert!(matches!(e.outcome, Outcome::Deny { ref failed, .. } if failed == &vec!["paths.write".to_string()]));
    }

    #[test]
    fn unlisted_action_denies() {
        let e = eval(&cap(vec![], vec![]), &ctx("note.delete", None), 0);
        assert!(matches!(e.outcome, Outcome::Deny { ref failed, .. } if failed.contains(&"action.allow".to_string())));
    }

    #[test]
    fn budget_exhaustion_escalates_when_escalatable_else_denies() {
        let c_esc = cap(vec!["budget.count:write"], vec![]);
        let e = eval(&c_esc, &ctx("note.write", Some(vec!["inbox/a.md".into()])), 2);
        assert!(matches!(e.outcome, Outcome::Escalate { .. }));

        let c_no = cap(vec![], vec![]);
        let e = eval(&c_no, &ctx("note.write", Some(vec!["inbox/a.md".into()])), 2);
        assert!(matches!(e.outcome, Outcome::Deny { .. }));
    }

    #[test]
    fn approval_exemption_flips_the_check() {
        let c = cap(vec!["budget.count:write"], vec![]);
        let e = evaluate(
            &c,
            &ctx("note.write", Some(vec!["inbox/a.md".into()])),
            &mut |_| 2,
            &mut |key| (key == "budget.count:write").then(|| "esc:7".to_string()),
        );
        assert_eq!(e.outcome, Outcome::Allow);
        let budget = e.checks.iter().find(|c| c.caveat == "budget.count:write").unwrap();
        assert_eq!(budget.meter["approved_exemption"], "esc:7");
    }

    #[test]
    fn unknown_dimension_fails_closed_and_never_escalates() {
        // Even listed as escalatable and with an exemption on offer, an
        // unknown dim stays failed and forces Deny.
        let c = cap(
            vec!["quantum.entanglement"],
            vec![json!({"dim":"quantum.entanglement","max":3})],
        );
        let e = evaluate(
            &c,
            &ctx("note.write", Some(vec!["inbox/a.md".into()])),
            &mut |_| 0,
            &mut |_| Some("esc:9".into()),
        );
        assert!(matches!(e.outcome, Outcome::Deny { ref failed, .. }
            if failed.contains(&"quantum.entanglement".to_string())));
    }

    #[test]
    fn structural_gates_deny_outright() {
        // Expired.
        let mut c = cap(vec![], vec![]);
        c["expires_at"] = json!("2026-07-08T01:00:00Z");
        let e = eval(&c, &ctx("note.read", None), 0);
        assert!(matches!(e.outcome, Outcome::Deny { structural: Some(ref s), .. } if s.contains("expired")));

        // M2: wrong manifest.
        let c = cap(vec![], vec![]);
        let mut cx = ctx("note.read", None);
        cx.current_manifest = "man:other";
        let e = eval(&c, &cx, 0);
        assert!(matches!(e.outcome, Outcome::Deny { structural: Some(ref s), .. } if s.contains("M2")));
    }

    #[test]
    fn irreversible_action_blocked_by_reversibility_max() {
        let c = cap(vec![], vec![]);
        let mut cx = ctx("note.write", Some(vec!["inbox/a.md".into()]));
        cx.reversibility = "irreversible";
        let e = eval(&c, &cx, 0);
        assert!(matches!(e.outcome, Outcome::Deny { ref failed, .. }
            if failed.contains(&"reversibility.max".to_string())));
    }

    #[test]
    fn unsupported_budget_window_fails_closed() {
        let mut c = cap(vec![], vec![]);
        c["caveats"][3] = json!({"dim":"budget.count","action_class":"write","max":2,"window":"P7D"});
        let e = eval(&c, &ctx("note.write", Some(vec!["inbox/a.md".into()])), 0);
        assert!(matches!(e.outcome, Outcome::Deny { ref failed, .. }
            if failed.contains(&"budget.count:write".to_string())));
    }
}
