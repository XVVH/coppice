//! The registration-derivation shim: what §4 conservative defaults give a
//! foreign MCP tool, in two documented modes.
//!
//! `Floor` is the normative zero-authorship result: nothing in these
//! corpora declares locality, reversibility, path arguments, or a store,
//! so §0 assigns external + egress + irreversible, class `write`, and the
//! SI-16 store field gets a synthetic `ext:<server>` (representational —
//! foreign tools have no fabric store; that mismatch is a finding, not a
//! shim bug).
//!
//! `Heuristic` is NOT normative and never feeds rules: it is
//! `corpus-heuristic-v1`, a name/annotation classifier that exists to
//! measure the distance between the floor and a plausible-but-unratified
//! classification — the gap the ratification loop is designed to close.
//! MCP ToolAnnotations, where present, override name heuristics (noting
//! that MCP itself calls them untrusted hints; whether they may ever
//! count as declared registration metadata is a spec question, not a
//! harness decision).

use super::model::ToolSchema;
use serde_json::Value;

pub const HEURISTIC_VERSION: &str = "corpus-heuristic-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Floor,
    Heuristic,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Floor => "floor",
            Mode::Heuristic => "heuristic",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Derived {
    pub side_effect: &'static str,
    pub reversibility: &'static str,
    pub class: &'static str,
    pub domain: String,
    pub store: String,
}

const READ_VERBS: &[&str] = &[
    "get", "list", "search", "read", "fetch", "query", "find", "describe",
    "retrieve", "check", "lookup", "view", "count", "browse", "stat", "info",
    "show", "ls", "cat", "head", "watch", "peek", "inspect", "ping",
];
const DESTRUCTIVE_VERBS: &[&str] = &[
    "delete", "remove", "drop", "destroy", "purge", "clear", "erase", "kill",
    "terminate", "revoke", "uninstall", "rm", "truncate",
];

/// Derive registration-shaped metadata for a call to `name` on a tool the
/// corpus may or may not have schema for (`schema` is None for
/// hallucinated calls).
pub fn derive(schema: Option<&ToolSchema>, name: &str, mode: Mode) -> Derived {
    let server = schema.map(|t| t.server.as_str()).unwrap_or("unknown");
    let floor = Derived {
        side_effect: "external",
        reversibility: "irreversible",
        class: "write",
        domain: "unclassified.external".to_string(),
        store: format!("ext:{server}"),
    };
    if mode == Mode::Floor {
        return floor;
    }

    // corpus-heuristic-v1 -------------------------------------------------
    let mut d = floor;
    if let Some(t) = schema {
        if let Some(cat) = t.categories.first() {
            d.domain = format!("cat:{}", super::model::slug(cat));
        }
    }
    let (class, reversibility) = match schema.and_then(|t| t.annotations.as_ref()) {
        Some(a) if a.get("readOnlyHint") == Some(&Value::Bool(true)) => ("read", "reversible"),
        Some(a) if a.get("destructiveHint") == Some(&Value::Bool(true)) => {
            ("delete", "irreversible")
        }
        Some(a) if a.get("destructiveHint") == Some(&Value::Bool(false)) => {
            ("write", "compensable")
        }
        _ => {
            // Strip the server prefix before verb classification, or the
            // server slug pollutes the tokens ("exa-search-create_note"
            // must classify on "create_note", not on "search"). Calls
            // without a schema (hallucinated names) classify on the full
            // name — documented noise, deny-bound via action.allow anyway.
            let stripped = schema
                .and_then(|t| name.strip_prefix(&format!("{}-", t.server)))
                .unwrap_or(name);
            classify_by_name(stripped)
        }
    };
    d.class = class;
    d.reversibility = reversibility;
    d
}

fn classify_by_name(name: &str) -> (&'static str, &'static str) {
    let lowered = name.to_ascii_lowercase();
    let tokens: Vec<&str> = lowered
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.iter().any(|t| DESTRUCTIVE_VERBS.contains(t)) {
        return ("delete", "irreversible");
    }
    let readish = tokens
        .first()
        .is_some_and(|t| READ_VERBS.contains(t))
        || tokens
            .iter()
            .any(|t| ["search", "list", "get", "read", "query", "fetch", "lookup"].contains(t));
    if readish {
        ("read", "reversible")
    } else {
        ("write", "compensable")
    }
}
