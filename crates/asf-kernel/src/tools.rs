//! ToolRegistration. Spec §4.
//!
//! Tools declare shape once; the broker countersigns (Stage 2: seals with
//! the fabric key). Everything here is *declared, trusted-mechanical*
//! metadata; the conservative defaults are applied at registration time so
//! the stored object is explicit about what will be enforced:
//! undeclared actions cannot be called; missing reversibility =
//! `irreversible`; external open-surface actions without an egress
//! declaration are treated as egress (undeclared egress = egress, §0).
//!
//! Stage 2 additions beyond the spec example (flagged SI-16): each action
//! declares `store` (which registered store it operates on — feeds the M1
//! check at mint time), `class` (the budget.count action_class it meters
//! under), and `path_args` (which argument fields carry write paths — how
//! the broker extracts paths.write targets from call args).

use crate::canon;
use crate::trace;
use ed25519_dalek::SigningKey;
use serde_json::{json, Map, Value};

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool registration malformed: {0}")]
    Malformed(String),
    #[error(transparent)]
    Canon(#[from] canon::CanonError),
    #[error(transparent)]
    Trace(#[from] trace::TraceError),
    #[error("tool {tool} has no declared action {action} — undeclared actions cannot be called")]
    UndeclaredAction { tool: String, action: String },
}

/// Registration-time normalization: apply §0/§4 conservative defaults and
/// validate. Returns the normalized actions array.
fn normalize_actions(actions: &Value) -> Result<Value, ToolError> {
    let list = actions
        .as_array()
        .ok_or_else(|| ToolError::Malformed("actions must be an array".into()))?;
    if list.is_empty() {
        return Err(ToolError::Malformed("a tool with no actions registers nothing".into()));
    }
    let mut out = Vec::new();
    for a in list {
        let mut m = a
            .as_object()
            .cloned()
            .ok_or_else(|| ToolError::Malformed("action must be an object".into()))?;
        for req in ["name", "side_effect", "domain", "class", "store"] {
            if m.get(req).and_then(Value::as_str).is_none() {
                return Err(ToolError::Malformed(format!(
                    "action missing required field '{req}': {a}"
                )));
            }
        }
        // Missing reversibility = irreversible (§0, §4).
        m.entry("reversibility".to_string())
            .or_insert(json!("irreversible"));
        if m.get("compensator").is_none() {
            m.insert("compensator".into(), Value::Null);
        }
        // Undeclared egress = egress for external open surfaces (§0, §4).
        let external = m["side_effect"] == "external";
        let surface = m
            .get("surface")
            .and_then(Value::as_str)
            .unwrap_or("open")
            .to_string();
        m.entry("surface".to_string()).or_insert(json!(surface));
        if external && surface == "open" && m.get("egress").is_none() {
            m.insert("egress".into(), json!(true));
        }
        if m.get("path_args").is_none() {
            m.insert("path_args".into(), json!([]));
        }
        out.push(Value::Object(m));
    }
    Ok(Value::Array(out))
}

/// Register (seal + store + trace) a tool. Returns the tool object id.
pub fn register(
    conn: &mut rusqlite::Connection,
    fabric_sk: &SigningKey,
    substrate_span: &str,
    tool_name: &str,
    actions: Value,
    now: &str,
) -> Result<String, ToolError> {
    let mut body = Map::new();
    body.insert("tool".into(), json!(tool_name));
    body.insert("actions".into(), normalize_actions(&actions)?);
    body.insert("registered_at".into(), json!(now));
    let sealed = canon::seal("tool", body, fabric_sk)?;
    let id = trace::put_object(conn, "tool", &sealed, now)?;
    trace::append(
        conn,
        fabric_sk,
        substrate_span,
        None,
        "register",
        json!({ "object": id, "object_kind": "tool" }),
        now,
    )?;
    Ok(id)
}

/// Look up a registered action. `tool_ref` is the registered tool name
/// (e.g. "tool:vault@1.0"). Absent tool or action → error (uncallable).
pub fn lookup_action(
    conn: &rusqlite::Connection,
    tool_ref: &str,
    action: &str,
) -> Result<Value, ToolError> {
    let found: Option<Value> = {
        let mut stmt = conn
            .prepare("SELECT raw FROM objects WHERE kind = 'tool'")
            .map_err(trace::TraceError::from)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(trace::TraceError::from)?;
        let mut hit = None;
        for raw in rows {
            let obj: Value = serde_json::from_str(&raw.map_err(trace::TraceError::from)?)
                .expect("stored objects are valid JSON");
            if obj["tool"] == tool_ref {
                hit = obj["actions"]
                    .as_array()
                    .and_then(|a| a.iter().find(|x| x["name"] == action).cloned());
                break;
            }
        }
        hit
    };
    found.ok_or_else(|| ToolError::UndeclaredAction {
        tool: tool_ref.into(),
        action: action.into(),
    })
}

/// All stores any allowed action of `tool_ref` operates on — the M1 input.
pub fn stores_for_tool(conn: &rusqlite::Connection, tool_ref: &str) -> Result<Vec<String>, ToolError> {
    let mut stores = Vec::new();
    let mut stmt = conn
        .prepare("SELECT raw FROM objects WHERE kind = 'tool'")
        .map_err(trace::TraceError::from)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(trace::TraceError::from)?;
    for raw in rows {
        let obj: Value = serde_json::from_str(&raw.map_err(trace::TraceError::from)?)
            .expect("stored objects are valid JSON");
        if obj["tool"] == tool_ref {
            for a in obj["actions"].as_array().into_iter().flatten() {
                if let Some(s) = a["store"].as_str() {
                    if !stores.contains(&s.to_string()) {
                        stores.push(s.to_string());
                    }
                }
            }
        }
    }
    Ok(stores)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;

    fn setup() -> (rusqlite::Connection, SigningKey, String) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        trace::init(&conn).unwrap();
        (conn, SigningKey::generate(&mut OsRng), trace::new_span())
    }

    fn vault_actions() -> Value {
        json!([
            { "name": "note.read",  "side_effect": "local", "surface": "fixed",
              "reversibility": "reversible", "domain": "files.vault",
              "class": "read", "store": "fs:vault", "path_args": ["path"] },
            { "name": "note.write", "side_effect": "local", "surface": "fixed",
              "reversibility": "reversible", "domain": "files.vault",
              "class": "write", "store": "fs:vault", "path_args": ["path"] },
            // Deliberately no reversibility, no egress:
            { "name": "web.fetch", "side_effect": "external",
              "domain": "web.research", "class": "read", "store": "fs:vault" },
        ])
    }

    #[test]
    fn conservative_defaults_applied_at_registration() {
        let (mut conn, sk, span) = setup();
        register(&mut conn, &sk, &span, "tool:vault@1.0", vault_actions(), "t0").unwrap();
        let fetch = lookup_action(&conn, "tool:vault@1.0", "web.fetch").unwrap();
        assert_eq!(fetch["reversibility"], "irreversible", "missing reversibility = irreversible");
        assert_eq!(fetch["egress"], true, "undeclared egress on external open surface = egress");
        assert_eq!(fetch["surface"], "open", "undeclared surface treated as open (worst case)");
    }

    #[test]
    fn undeclared_actions_cannot_be_called() {
        let (mut conn, sk, span) = setup();
        register(&mut conn, &sk, &span, "tool:vault@1.0", vault_actions(), "t0").unwrap();
        assert!(matches!(
            lookup_action(&conn, "tool:vault@1.0", "note.nuke"),
            Err(ToolError::UndeclaredAction { .. })
        ));
        assert!(lookup_action(&conn, "tool:other@1.0", "note.read").is_err());
    }

    #[test]
    fn registration_requires_class_and_store() {
        let (mut conn, sk, span) = setup();
        let bad = json!([{ "name": "x", "side_effect": "local", "domain": "d" }]);
        assert!(register(&mut conn, &sk, &span, "tool:bad@1", bad, "t0").is_err());
    }

    #[test]
    fn stores_derivation_for_m1() {
        let (mut conn, sk, span) = setup();
        register(&mut conn, &sk, &span, "tool:vault@1.0", vault_actions(), "t0").unwrap();
        assert_eq!(stores_for_tool(&conn, "tool:vault@1.0").unwrap(), vec!["fs:vault"]);
    }
}
