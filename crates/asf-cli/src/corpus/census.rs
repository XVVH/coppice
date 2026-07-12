//! Zero-authorship default census (W-9 lane 1).
//!
//! For every tool schema in the universe: which §4 registration fields
//! could a broker derive from what the schema actually declares, and
//! which fall to the §0 conservative floor? The census is empirical
//! input to the domain-taxonomy open problem — input, not governance —
//! and the heuristic columns are explicitly non-normative
//! (`corpus-heuristic-v1` measures the floor↔classifier gap; it never
//! feeds rules).

use super::derive::{self, Mode};
use super::model::ToolSchema;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default)]
struct SourceStats {
    tools: u64,
    servers: BTreeSet<String>,
    description: u64,
    parameters: u64,
    annotations_any: u64,
    read_only_hint: u64,
    destructive_hint: u64,
    idempotent_hint: u64,
    open_world_hint: u64,
    /// Tools with at least one reversibility-shaped hint (readOnlyHint
    /// OR destructiveHint) — per-tool set membership, never a sum of
    /// key counts (a full annotation block must count once, not twice).
    reversibility_signal: u64,
    /// Corpus-DECLARED categories only; harness provenance never counts.
    categories: u64,
    /// Harness provenance tags (bookkeeping: which file/marketplace the
    /// schema came from). Reported so the categories/provenance split
    /// is auditable; contributes to NO derivability figure.
    provenance: u64,
    class_read: u64,
    class_write: u64,
    class_delete: u64,
}

pub fn run(universe: &[ToolSchema]) -> Value {
    let mut by_source: BTreeMap<&'static str, SourceStats> = BTreeMap::new();
    for t in universe {
        let s = by_source.entry(t.source).or_default();
        s.tools += 1;
        s.servers.insert(t.server.clone());
        if t.description.as_deref().is_some_and(|d| !d.is_empty()) {
            s.description += 1;
        }
        if t
            .parameters
            .as_ref()
            .is_some_and(|p| p.get("properties").is_some() || p.get("type").is_some())
        {
            s.parameters += 1;
        }
        if let Some(a) = &t.annotations {
            s.annotations_any += 1;
            let read_only = a.get("readOnlyHint").is_some();
            let destructive = a.get("destructiveHint").is_some();
            if read_only {
                s.read_only_hint += 1;
            }
            if destructive {
                s.destructive_hint += 1;
            }
            if read_only || destructive {
                s.reversibility_signal += 1;
            }
            if a.get("idempotentHint").is_some() {
                s.idempotent_hint += 1;
            }
            if a.get("openWorldHint").is_some() {
                s.open_world_hint += 1;
            }
        }
        if !t.categories.is_empty() {
            s.categories += 1;
        }
        if t.provenance.is_some() {
            s.provenance += 1;
        }
        let d = derive::derive(Some(t), &t.name, Mode::Heuristic);
        match d.class {
            "read" => s.class_read += 1,
            "delete" => s.class_delete += 1,
            _ => s.class_write += 1,
        }
    }

    let totals = |f: fn(&SourceStats) -> u64| -> u64 { by_source.values().map(f).sum() };
    let n_tools = totals(|s| s.tools);
    let pct = |n: u64| -> f64 {
        if n_tools == 0 {
            0.0
        } else {
            (n as f64 * 1000.0 / n_tools as f64).round() / 10.0
        }
    };

    // Derivability of each registration-required field from declared
    // metadata alone (the zero-authorship question). side_effect, store,
    // and path_args are structurally underivable from these corpora:
    // nothing in an MCP tool schema declares locality, a fabric store
    // binding (SI-16), or which argument fields carry write paths.
    let categories = totals(|s| s.categories);
    let reversibility_signal = totals(|s| s.reversibility_signal);

    json!({
        "universe": {
            "tools": n_tools,
            "servers_by_source_sum": by_source.values().map(|s| s.servers.len() as u64).sum::<u64>(),
            "servers_note": "per-source counts only; mcp-flow server identity is best-effort name-prefix recovery (an UPPER BOUND — underscore-less tool names become their own 'server', and this subset-derived count exceeds upstream's declared 1,166 total); cross-source dedup is not attempted",
            "by_source": by_source.iter().map(|(src, s)| {
                (src.to_string(), json!({
                    "tools": s.tools,
                    "servers": s.servers.len(),
                    "description_present": s.description,
                    "parameters_present": s.parameters,
                    "annotations_any": s.annotations_any,
                    "readOnlyHint": s.read_only_hint,
                    "destructiveHint": s.destructive_hint,
                    "idempotentHint": s.idempotent_hint,
                    "openWorldHint": s.open_world_hint,
                    "declared_categories_present": s.categories,
                    "provenance_tag_present": s.provenance,
                    "heuristic_class": {
                        "read": s.class_read,
                        "write": s.class_write,
                        "delete": s.class_delete,
                    },
                }))
            }).collect::<serde_json::Map<String, Value>>(),
        },
        "zero_authorship_derivability": {
            "side_effect": { "derivable": 0, "pct": 0.0,
                "note": "nothing declares locality; §0 floors every tool to external" },
            "egress": { "derivable": 0, "pct": 0.0,
                "note": "undeclared egress = egress on external open surfaces (§0)" },
            "reversibility": { "derivable": reversibility_signal, "pct": pct(reversibility_signal),
                "note": "tools carrying readOnlyHint or destructiveHint (set membership, not a key sum); MCP itself marks these untrusted hints" },
            "action_class": { "derivable": reversibility_signal, "pct": pct(reversibility_signal),
                "note": "same reversibility-shaped hints; idempotent/openWorld-only annotations classify nothing" },
            "domain": { "derivable": categories, "pct": pct(categories),
                "note": "corpus-DECLARED server categories only (Toucan crawler labels — third-party, not §4 self-declared registration metadata); harness provenance tags count as nothing; taxonomy governance stays open" },
            "store": { "derivable": 0, "pct": 0.0,
                "note": "SI-16 store binding has no foreign analogue; harness uses a representational ext:<server>" },
            "path_args": { "derivable": 0, "pct": 0.0,
                "note": "no schema names its path-carrying arguments; paths.write can only fail closed on write-shaped foreign calls" },
        },
        "conservative_floor": {
            "irreversible_pct": pct(n_tools.saturating_sub(reversibility_signal)),
            "egress_pct": 100.0,
            "note": "share of the universe the free defaults treat as irreversible egress absent ratified reclassification",
        },
        "heuristic": {
            "version": derive::HEURISTIC_VERSION,
            "normative": false,
            "note": "name/annotation classifier measuring the floor↔classifier gap; never feeds rules",
        },
    })
}

pub fn render_md(census: &Value, gaps: &Value) -> String {
    let mut md = String::new();
    md.push_str("# W-9 census — zero-authorship defaults over the tool universe\n\n");
    md.push_str(&format!(
        "Universe: **{} tools**. Server counts are per-source only — \
         mcp-flow server identity is best-effort name-prefix recovery \
         (an upper bound, not distinct servers); see census.json \
         `servers_note`.\n\n",
        census["universe"]["tools"]
    ));
    md.push_str("| source | tools | servers (upper bound) | desc % | params % | annotations % | declared categories % | read/write/delete (heuristic) |\n");
    md.push_str("|---|---|---|---|---|---|---|---|\n");
    if let Some(map) = census["universe"]["by_source"].as_object() {
        for (src, s) in map {
            let n = s["tools"].as_u64().unwrap_or(0).max(1);
            let p = |k: &str| s[k].as_u64().unwrap_or(0) * 100 / n;
            md.push_str(&format!(
                "| {src} | {} | {} | {}% | {}% | {}% | {}% | {}/{}/{} |\n",
                s["tools"],
                s["servers"],
                p("description_present"),
                p("parameters_present"),
                p("annotations_any"),
                p("declared_categories_present"),
                s["heuristic_class"]["read"],
                s["heuristic_class"]["write"],
                s["heuristic_class"]["delete"],
            ));
        }
    }
    md.push_str("\n## Zero-authorship derivability (§0/§4, per registration field)\n\n");
    md.push_str("| field | derivable | % | note |\n|---|---|---|---|\n");
    if let Some(map) = census["zero_authorship_derivability"].as_object() {
        for (field, v) in map {
            md.push_str(&format!(
                "| {field} | {} | {}% | {} |\n",
                v["derivable"],
                v["pct"],
                v["note"].as_str().unwrap_or("")
            ));
        }
    }
    md.push_str(&format!(
        "\n**Conservative floor:** {}% of the universe is irreversible-egress under the free defaults.\n",
        census["conservative_floor"]["irreversible_pct"]
    ));
    md.push_str("\n## Encodability gaps (SI candidates, bounded samples)\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(gaps).unwrap_or_default());
    md.push_str("\n```\n");
    md
}
