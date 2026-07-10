//! Capabilities and the caveat grammar. Spec §5, §5.1, §5.2.
//!
//! Broker-minted (F1, settled), mandatory expiry, bound to a manifest (M2),
//! conjunctive caveats — ALL must pass — and unknown caveat dimensions fail
//! closed (§0): an unrecognized restriction cannot be safely ignored, so a
//! capability carrying a dimension this evaluator does not implement denies
//! every call it applies to.
//!
//! Caveats are stored as JSON objects `{"dim": "...", ...params}` so unknown
//! dimensions are representable, hashed, and preserved (§0 extensibility) —
//! exactly what fail-closed needs.

use serde_json::{json, Map, Value};

#[derive(Debug, thiserror::Error)]
pub enum CapError {
    #[error("capability missing required field {0}")]
    Missing(&'static str),
    #[error("expiry is mandatory (spec §5)")]
    NoExpiry,
    #[error("attenuation invalid: {0}")]
    Attenuation(String),
    #[error("malformed caveat: {0}")]
    MalformedCaveat(String),
}

/// Total order on reversibility classes (§4/§5.1). Lower = safer.
pub fn reversibility_rank(class: &str) -> Option<u8> {
    match class {
        "reversible" => Some(0),
        "compensable" => Some(1),
        "irreversible" => Some(2),
        _ => None,
    }
}

/// Total order on external reach (§5.1). Lower = less reach.
pub fn reach_rank(mode: &str) -> Option<u8> {
    match mode {
        "none" => Some(0),
        "mocks" => Some(1),
        "live" => Some(2),
        _ => None,
    }
}

/// Total order on channel auth strength, for `approval.min_auth`.
/// §2.1 lists strengths but never orders them; C4 groups local_session with
/// passkey as the strong tier, so they rank equal here (see SI-15).
pub fn auth_rank(strength: &str) -> Option<u8> {
    match strength {
        "unverified" => Some(0),
        "platform_oauth" => Some(1),
        "passkey" => Some(2),
        "local_session" => Some(2),
        _ => None,
    }
}

/// Dimensions this evaluator knows how to check mechanically. Anything not
/// listed fails closed at evaluation time and cannot be attenuation-verified.
pub const KNOWN_DIMS: &[&str] = &[
    "action.allow",
    "reversibility.max",
    "external_reach",
    "budget.count",
    "paths.write",
    "time",
    "approval.min_auth",
];

/// A caveat's identity key for matching parent↔child instances during
/// attenuation: most dims are singletons; budget.count is per action_class.
pub fn caveat_key(cav: &Value) -> Result<String, CapError> {
    let dim = cav
        .get("dim")
        .and_then(Value::as_str)
        .ok_or_else(|| CapError::MalformedCaveat(format!("no dim: {cav}")))?;
    Ok(match dim {
        "budget.count" => format!(
            "{dim}:{}",
            cav.get("action_class").and_then(Value::as_str).unwrap_or("*")
        ),
        _ => dim.to_string(),
    })
}

// ---------------------------------------------------------------------
// Glob matching — paths.write. Deliberately tiny and strict: `**` matches
// any number of path segments, `*` matches within a single segment. Used
// both for call-time checks and (conservatively) for attenuation.
// ---------------------------------------------------------------------

fn segment_matches(pat: &str, seg: &str) -> bool {
    // Wildcard match within one segment: '*' = any run of chars.
    let (mut pi, mut si) = (0usize, 0usize);
    let (p, s): (Vec<char>, Vec<char>) = (pat.chars().collect(), seg.chars().collect());
    let (mut star, mut mark) = (None, 0usize);
    while si < s.len() {
        if pi < p.len() && (p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Does `path` (relative, '/'-separated) match `glob`?
pub fn glob_matches(glob: &str, path: &str) -> bool {
    fn rec(pats: &[&str], segs: &[&str]) -> bool {
        match (pats.first(), segs.first()) {
            (None, None) => true,
            (Some(&"**"), _) => {
                // '**' consumes zero or more segments.
                rec(&pats[1..], segs) || (!segs.is_empty() && rec(pats, &segs[1..]))
            }
            (Some(p), Some(s)) => segment_matches(p, s) && rec(&pats[1..], &segs[1..]),
            _ => false,
        }
    }
    let pats: Vec<&str> = glob.split('/').filter(|s| !s.is_empty()).collect();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    rec(&pats, &segs)
}

/// Conservative glob-subset test for attenuation (SI-12): is every path
/// matched by `child` also matched by `parent`? True glob-subset is not
/// mechanically simple, so we accept only the cases we can prove:
/// identical globs; parent `**`; parent `prefix/**` covering a child whose
/// literal prefix sits under it. Everything else is not-a-subset (fail
/// closed — the child must be rewritten more plainly).
pub fn glob_covers(parent: &str, child: &str) -> bool {
    if parent == child || parent == "**" {
        return true;
    }
    if let Some(pre) = parent.strip_suffix("/**") {
        if pre.chars().any(|c| c == '*') {
            return false; // wildcard prefixes are beyond the conservative rule
        }
        return child == pre
            || child
                .strip_prefix(pre)
                .map(|rest| rest.starts_with('/'))
                .unwrap_or(false);
    }
    false
}

// ---------------------------------------------------------------------
// Attenuation (§5.2)
// ---------------------------------------------------------------------

fn as_str_set(v: &Value, field: &str) -> Vec<String> {
    v.get(field)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn subset(child: &[String], parent: &[String]) -> bool {
    child.iter().all(|c| parent.contains(c))
}

/// Per-dimension subset check: is `child` no wider than `parent`?
/// Unknown dimensions cannot be verified → error (fail closed).
fn dim_subset(dim: &str, parent: &Value, child: &Value) -> Result<bool, CapError> {
    Ok(match dim {
        "action.allow" => {
            subset(&as_str_set(child, "tools"), &as_str_set(parent, "tools"))
                && subset(&as_str_set(child, "actions"), &as_str_set(parent, "actions"))
        }
        "reversibility.max" => {
            let p = parent.get("max").and_then(Value::as_str).and_then(reversibility_rank);
            let c = child.get("max").and_then(Value::as_str).and_then(reversibility_rank);
            matches!((p, c), (Some(p), Some(c)) if c <= p)
        }
        "external_reach" => {
            let p = parent.get("mode").and_then(Value::as_str).and_then(reach_rank);
            let c = child.get("mode").and_then(Value::as_str).and_then(reach_rank);
            matches!((p, c), (Some(p), Some(c)) if c <= p)
        }
        "budget.count" => {
            let same_class = parent.get("action_class") == child.get("action_class");
            let p = parent.get("max").and_then(Value::as_u64);
            let c = child.get("max").and_then(Value::as_u64);
            let window_ok = parent.get("window") == child.get("window"); // conservative
            same_class && window_ok && matches!((p, c), (Some(p), Some(c)) if c <= p)
        }
        "paths.write" => {
            let p = as_str_set(parent, "globs");
            as_str_set(child, "globs")
                .iter()
                .all(|cg| p.iter().any(|pg| glob_covers(pg, cg)))
        }
        "time" => {
            // Child window must sit within the parent window. Instant
            // comparison, not lexical (RF-1): a present-but-unparseable bound
            // fails closed (not a subset).
            let ok_start = match (
                parent.get("not_before").and_then(Value::as_str),
                child.get("not_before").and_then(Value::as_str),
            ) {
                (Some(p), Some(c)) => matches!(
                    (crate::parse_instant(c), crate::parse_instant(p)),
                    (Some(c), Some(p)) if c >= p
                ),
                (Some(_), None) => false, // child unbounded where parent bounded
                (None, _) => true,
            };
            let ok_end = match (
                parent.get("not_after").and_then(Value::as_str),
                child.get("not_after").and_then(Value::as_str),
            ) {
                (Some(p), Some(c)) => matches!(
                    (crate::parse_instant(c), crate::parse_instant(p)),
                    (Some(c), Some(p)) if c <= p
                ),
                (Some(_), None) => false,
                (None, _) => true,
            };
            ok_start && ok_end
        }
        "approval.min_auth" => {
            // Child may require STRONGER approval channels, never weaker.
            let p = parent.get("min").and_then(Value::as_str).and_then(auth_rank);
            let c = child.get("min").and_then(Value::as_str).and_then(auth_rank);
            matches!((p, c), (Some(p), Some(c)) if c >= p)
        }
        other => {
            return Err(CapError::Attenuation(format!(
                "unknown dimension {other} cannot be subset-verified (fail closed)"
            )))
        }
    })
}

/// Verify §5.2: per dimension, child ⊆ parent; every parent dimension must
/// be present in the child (omitting one would widen); children may add
/// dimensions; expiry no later. Failure invalidates the capability outright.
pub fn verify_attenuation(parent: &Value, child: &Value) -> Result<(), CapError> {
    let att = |m: String| CapError::Attenuation(m);
    if child.get("parent").and_then(Value::as_str) != parent.get("id").and_then(Value::as_str) {
        return Err(att("child.parent must reference parent capability id".into()));
    }
    let (pexp, cexp) = (
        parent.get("expires_at").and_then(Value::as_str).ok_or(CapError::NoExpiry)?,
        child.get("expires_at").and_then(Value::as_str).ok_or(CapError::NoExpiry)?,
    );
    match (crate::parse_instant(cexp), crate::parse_instant(pexp)) {
        (Some(child), Some(parent)) if child <= parent => {}
        (Some(_), Some(_)) => {
            return Err(att(format!("child expiry {cexp} later than parent {pexp}")))
        }
        _ => {
            return Err(att(
                "unparseable parent or child expiry (fail closed)".into(),
            ))
        }
    }
    if child.get("bound_manifest").is_none() {
        return Err(att("child missing bound_manifest (M2)".into()));
    }

    let empty = Vec::new();
    let pcavs = parent.get("caveats").and_then(Value::as_array).unwrap_or(&empty);
    let ccavs = child.get("caveats").and_then(Value::as_array).unwrap_or(&empty);
    let child_by_key: std::collections::BTreeMap<String, &Value> = ccavs
        .iter()
        .map(|c| Ok((caveat_key(c)?, c)))
        .collect::<Result<_, CapError>>()?;

    for pcav in pcavs {
        let key = caveat_key(pcav)?;
        let dim = pcav["dim"].as_str().expect("keyed caveat has dim");
        let ccav = child_by_key.get(&key).ok_or_else(|| {
            att(format!("parent dimension {key} absent in child — omission widens"))
        })?;
        if !dim_subset(dim, pcav, ccav)? {
            return Err(att(format!("dimension {key}: child is not a subset of parent")));
        }
    }
    Ok(())
}

/// Build an (unsealed) capability body. The broker seals it (F1).
pub fn build(
    holder: &str,
    bound_manifest: &str,
    parent: Option<&str>,
    issued_at: &str,
    expires_at: &str,
    caveats: Vec<Value>,
    escalatable: Vec<&str>,
) -> Result<Map<String, Value>, CapError> {
    if expires_at.is_empty() {
        return Err(CapError::NoExpiry);
    }
    let mut body = Map::new();
    body.insert("parent".into(), parent.map(Value::from).unwrap_or(Value::Null));
    body.insert("holder".into(), json!(holder));
    body.insert("bound_manifest".into(), json!(bound_manifest));
    body.insert("issued_at".into(), json!(issued_at));
    body.insert("expires_at".into(), json!(expires_at));
    body.insert("caveats".into(), Value::Array(caveats));
    body.insert(
        "on_violation".into(),
        json!({ "default": "deny", "escalatable": escalatable }),
    );
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(id: &str, parent: Option<&str>, expires: &str, caveats: Vec<Value>) -> Value {
        json!({
            "id": id,
            "parent": parent,
            "holder": "prin:agent",
            "bound_manifest": "man:x",
            "issued_at": "2026-07-08T00:00:00Z",
            "expires_at": expires,
            "caveats": caveats,
            "on_violation": { "default": "deny", "escalatable": [] },
        })
    }

    fn parent_cap() -> Value {
        cap(
            "cap:parent",
            None,
            "2026-07-08T12:00:00Z",
            vec![
                json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.read","note.write","note.move"]}),
                json!({"dim":"reversibility.max","max":"compensable"}),
                json!({"dim":"paths.write","globs":["inbox/**","MOCs/**"]}),
                json!({"dim":"budget.count","action_class":"write","max":10,"window":"run"}),
            ],
        )
    }

    #[test]
    fn glob_semantics() {
        assert!(glob_matches("inbox/**", "inbox/a/b.md"));
        assert!(glob_matches("inbox/**", "inbox"));
        assert!(glob_matches("**", "anything/at/all"));
        assert!(glob_matches("inbox/*.md", "inbox/x.md"));
        assert!(!glob_matches("inbox/*.md", "inbox/sub/x.md"));
        assert!(!glob_matches("inbox/**", "MOCs/x.md"));
    }

    #[test]
    fn glob_cover_is_conservative() {
        assert!(glob_covers("inbox/**", "inbox/drafts/**"));
        assert!(glob_covers("inbox/**", "inbox/*.md"));
        assert!(glob_covers("**", "anything/**"));
        assert!(!glob_covers("inbox/*.md", "inbox/a.md")); // provable, but not by our rule — stays closed
        assert!(!glob_covers("inbox/**", "MOCs/**"));
        assert!(!glob_covers("in*/**", "inbox/**")); // wildcard prefix → refuse to reason
    }

    #[test]
    fn valid_attenuation_narrows_everything() {
        let parent = parent_cap();
        let child = cap(
            "cap:child",
            Some("cap:parent"),
            "2026-07-08T11:00:00Z", // earlier expiry: ok
            vec![
                json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.read","note.write"]}),
                json!({"dim":"reversibility.max","max":"reversible"}),
                json!({"dim":"paths.write","globs":["inbox/drafts/**"]}),
                json!({"dim":"budget.count","action_class":"write","max":3,"window":"run"}),
                json!({"dim":"time","not_after":"2026-07-08T10:00:00Z"}), // added dim: legal
            ],
        );
        verify_attenuation(&parent, &child).unwrap();
    }

    #[test]
    fn attenuation_identity_is_legal() {
        // M5 self-delegation re-uses the capability: same caveats, same expiry.
        let parent = parent_cap();
        let mut child = parent.clone();
        child["id"] = json!("cap:child");
        child["parent"] = json!("cap:parent");
        verify_attenuation(&parent, &child).unwrap();
    }

    #[test]
    fn child_expiry_is_compared_as_an_instant() {
        let parent = cap(
            "cap:parent",
            None,
            "2026-07-08T12:00:00Z",
            vec![],
        );
        let child = cap(
            "cap:child",
            Some("cap:parent"),
            "2026-07-08T12:00:00.5Z",
            vec![],
        );
        assert!(verify_attenuation(&parent, &child).is_err());

        let mut malformed = child;
        malformed["expires_at"] = json!("later");
        assert!(verify_attenuation(&parent, &malformed).is_err());
    }

    #[test]
    fn widening_attempts_all_rejected() {
        let parent = parent_cap();
        let base = |caveats: Vec<Value>| cap("cap:child", Some("cap:parent"), "2026-07-08T12:00:00Z", caveats);
        let parent_cavs = parent["caveats"].as_array().unwrap().clone();

        // 1. Later expiry.
        let c = cap("cap:child", Some("cap:parent"), "2026-07-09T00:00:00Z", parent_cavs.clone());
        assert!(verify_attenuation(&parent, &c).is_err());

        // 2. Extra action.
        let mut cavs = parent_cavs.clone();
        cavs[0] = json!({"dim":"action.allow","tools":["tool:vault@1.0"],"actions":["note.read","note.write","note.move","note.delete"]});
        assert!(verify_attenuation(&parent, &base(cavs)).is_err());

        // 3. Worse reversibility.
        let mut cavs = parent_cavs.clone();
        cavs[1] = json!({"dim":"reversibility.max","max":"irreversible"});
        assert!(verify_attenuation(&parent, &base(cavs)).is_err());

        // 4. Wider paths.
        let mut cavs = parent_cavs.clone();
        cavs[2] = json!({"dim":"paths.write","globs":["**"]});
        assert!(verify_attenuation(&parent, &base(cavs)).is_err());

        // 5. Bigger budget.
        let mut cavs = parent_cavs.clone();
        cavs[3] = json!({"dim":"budget.count","action_class":"write","max":100,"window":"run"});
        assert!(verify_attenuation(&parent, &base(cavs)).is_err());

        // 6. Dropping a parent dimension (omission widens).
        let cavs = parent_cavs[..3].to_vec();
        assert!(verify_attenuation(&parent, &base(cavs)).is_err());

        // 7. Wrong parent reference.
        let mut c = base(parent_cavs.clone());
        c["parent"] = json!("cap:someone-else");
        assert!(verify_attenuation(&parent, &c).is_err());
    }

    #[test]
    fn unknown_dimension_cannot_be_attenuation_verified() {
        let mut parent = parent_cap();
        parent["caveats"]
            .as_array_mut()
            .unwrap()
            .push(json!({"dim":"quantum.entanglement","max":3}));
        let mut child = parent.clone();
        child["id"] = json!("cap:child");
        child["parent"] = json!("cap:parent");
        // Even identity attenuation fails: we cannot prove subset for a
        // dimension we do not understand.
        assert!(verify_attenuation(&parent, &child).is_err());
    }

    #[test]
    fn expiry_is_mandatory() {
        assert!(matches!(
            build("prin:a", "man:x", None, "t", "", vec![], vec![]),
            Err(CapError::NoExpiry)
        ));
    }
}
