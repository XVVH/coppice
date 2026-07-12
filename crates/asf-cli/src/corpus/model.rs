//! Normalized corpus model plus the encodability gap ledger (W-9).
//!
//! Foreign trajectories normalize into exactly this shape; anything the
//! mapping cannot express becomes a [`GapLedger`] entry and quarantines
//! its trajectory — never a guess (the SI discipline applied to data:
//! `docs/agent-trace-corpora-2026-07-11.md`).

use serde_json::Value;
use std::collections::BTreeMap;

/// One tool schema as the corpus presents it (an MCP tool, usually
/// serialized in OpenAI function form, which drops MCP ToolAnnotations).
#[derive(Debug, Clone)]
pub struct ToolSchema {
    /// Server slug; registration groups tools per server as
    /// `tool:corpus/<server>` with one action per MCP tool.
    pub server: String,
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<Value>,
    /// MCP ToolAnnotations if the source preserved them (readOnlyHint,
    /// destructiveHint, idempotentHint, openWorldHint).
    pub annotations: Option<Value>,
    /// Server-level categories/tags THE CORPUS DECLARES (third-party
    /// labels, e.g. Toucan's crawler-assigned server categories). Never
    /// harness-injected — provenance goes in `provenance`, and only
    /// this field may count as domain-shaped signal in the census.
    pub categories: Vec<String>,
    /// Where the harness got the schema (e.g. mcp-flow marketplace
    /// file). Bookkeeping only; counts as NOTHING in derivability.
    pub provenance: Option<String>,
    pub source: &'static str,
}

/// One proposed call, in trajectory order.
#[derive(Debug, Clone)]
pub struct Call {
    pub tool_name: String,
    pub arguments: Value,
    pub result: Option<String>,
    /// True when the called name matches no available tool (a
    /// hallucinated call) — replayed, and expected to fail action.allow.
    pub unregistered: bool,
}

#[derive(Debug, Clone)]
pub struct Trajectory {
    pub uuid: String,
    pub question: Option<String>,
    pub tools: Vec<ToolSchema>,
    pub calls: Vec<Call>,
}

impl Trajectory {
    /// Server slugs in first-seen order (registration + action.allow).
    pub fn servers(&self) -> Vec<String> {
        let mut out = Vec::new();
        for t in &self.tools {
            if !out.contains(&t.server) {
                out.push(t.server.clone());
            }
        }
        out
    }
}

/// Counts + bounded samples per gap kind. A gap is a corpus shape the
/// §3/§6 mapping refuses to interpret silently; recurring kinds are SI
/// candidates, not parser bugs to paper over.
#[derive(Debug, Default, Clone)]
pub struct GapLedger {
    pub kinds: BTreeMap<String, (u64, Vec<String>)>,
}

impl GapLedger {
    pub fn add(&mut self, kind: &str, sample: String) {
        let entry = self.kinds.entry(kind.to_string()).or_default();
        entry.0 += 1;
        if entry.1.len() < 3 {
            // Char-boundary-safe clip: byte truncation panics on multibyte
            // content, and real corpus samples contain plenty of it.
            entry.1.push(sample.chars().take(200).collect());
        }
    }

    pub fn merge(&mut self, other: GapLedger) {
        for (kind, (n, samples)) in other.kinds {
            let entry = self.kinds.entry(kind).or_default();
            entry.0 += n;
            for s in samples {
                if entry.1.len() < 3 {
                    entry.1.push(s);
                }
            }
        }
    }

    pub fn to_json(&self) -> Value {
        Value::Object(
            self.kinds
                .iter()
                .map(|(k, (n, samples))| {
                    (
                        k.clone(),
                        serde_json::json!({ "count": n, "samples": samples }),
                    )
                })
                .collect(),
        )
    }
}

/// The result of normalizing a corpus directory.
#[derive(Debug, Default)]
pub struct ParseOutcome {
    pub trajectories: Vec<Trajectory>,
    /// (uuid, reason) — trajectories excluded whole; fail closed, both
    /// lanes (a partially-guessed trajectory is worse than none).
    pub quarantined: Vec<(String, String)>,
    pub gaps: GapLedger,
    /// Rows that parsed cleanly but proposed no tool call at all.
    pub no_call_rows: u64,
    /// Upstream dataset revision from FETCH.json, when present (absent
    /// for fixtures). Recorded into reports so a baseline names its
    /// inputs from the machine manifest, never by hand.
    pub revision: Option<String>,
}

/// Verify a consumed corpus file against the fetch manifest, when one
/// exists. FETCH.json present + file unlisted or hash-mismatched = hard
/// error: the reproducibility chain is enforced at load, not decorative.
/// No manifest (committed fixtures) = nothing to verify.
pub fn verify_against_manifest(
    manifest: Option<&serde_json::Value>,
    file_name: &str,
    bytes: &str,
) -> anyhow::Result<()> {
    let Some(m) = manifest else { return Ok(()) };
    let recorded = m["files"][file_name]["sha256"].as_str();
    let Some(recorded) = recorded else {
        anyhow::bail!("{file_name} is not listed in FETCH.json (partial or foreign file)")
    };
    use sha2::{Digest, Sha256};
    let actual = format!("sha256:{}", hex::encode(Sha256::digest(bytes.as_bytes())));
    if actual != recorded {
        anyhow::bail!(
            "{file_name} does not match FETCH.json ({actual} != {recorded}); refetch or delete"
        );
    }
    Ok(())
}

/// Slug used for server ids and tool refs: lowercase alnum runs joined
/// by single hyphens (deterministic, filesystem/id safe).
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}
