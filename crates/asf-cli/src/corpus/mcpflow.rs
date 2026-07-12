//! MCP-Flow tool-schema collection (census input only).
//!
//! MCP-Flow declares no license, so these files are never converted into
//! committed fixtures and never replayed into a substrate — the census
//! reports aggregates only (docs/agent-trace-corpora-2026-07-11.md).
//!
//! Layout note: the repo's `function_call/` files carry instruction→call
//! pairs WITHOUT schemas; the schemas (parameters) live in the
//! `test_data/` sets, fetched to `corpora/mcp-flow-testdata/` by
//! `scripts/fetch-corpora mcpflow-testdata`. Each test item embeds a
//! `tools` JSON-string holding an OpenAI-form candidate list drawn from
//! many servers; the server of a candidate is only recoverable from its
//! name prefix (`<server>_<tool>`), so mcp-flow server attribution is
//! best-effort and the census README says so. Dedup key: (server, name).

use super::model::{slug, GapLedger, ToolSchema};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub struct McpFlowUniverse {
    pub tools: Vec<ToolSchema>,
    pub files_scanned: u64,
    pub gaps: GapLedger,
}

pub fn load_dir(corpora_dir: &Path) -> McpFlowUniverse {
    let dir = corpora_dir.join("mcp-flow-testdata");
    let mut tools: BTreeMap<(String, String), ToolSchema> = BTreeMap::new();
    let mut gaps = GapLedger::default();
    let mut files_scanned = 0;

    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension().is_some_and(|e| e == "json")
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n != "FETCH.json")
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();

    for f in files {
        files_scanned += 1;
        let marketplace = f
            .file_stem()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split('_').next())
            .unwrap_or("unknown")
            .to_string();
        let Ok(text) = std::fs::read_to_string(&f) else {
            gaps.add("mcpflow_unreadable_file", f.display().to_string());
            continue;
        };
        let Ok(items) = serde_json::from_str::<Value>(&text) else {
            gaps.add("mcpflow_unparseable_file", f.display().to_string());
            continue;
        };
        for item in items.as_array().into_iter().flatten() {
            let list: Value = match item["tools"].as_str().map(serde_json::from_str) {
                Some(Ok(v)) => v,
                _ => {
                    gaps.add(
                        "mcpflow_item_tools_unparseable",
                        format!("{}: {}", f.display(), item["tool_name"]),
                    );
                    continue;
                }
            };
            for entry in list.as_array().into_iter().flatten() {
                let func = &entry["function"];
                let Some(name) = func["name"].as_str() else {
                    gaps.add("mcpflow_tool_without_name", format!("{entry}"));
                    continue;
                };
                // Best-effort server recovery from the `<server>_<tool>`
                // prefix convention; no prefix → the whole name groups
                // under "unknown" (counted, not guessed further).
                let server = slug(name.split('_').next().filter(|s| !s.is_empty()).unwrap_or("unknown"));
                tools
                    .entry((server.clone(), name.to_string()))
                    .or_insert_with(|| ToolSchema {
                        server,
                        name: name.to_string(),
                        description: func["description"].as_str().map(str::to_string),
                        parameters: func.get("parameters").filter(|p| !p.is_null()).cloned(),
                        annotations: extract_annotations(entry, func),
                        categories: vec![format!("marketplace:{marketplace}")],
                        source: "mcp-flow",
                    });
            }
        }
    }

    McpFlowUniverse {
        tools: tools.into_values().collect(),
        files_scanned,
        gaps,
    }
}

/// MCP ToolAnnotations survive in some serializations as an
/// `annotations` object or as bare hint keys; capture either.
fn extract_annotations(entry: &Value, function: &Value) -> Option<Value> {
    for holder in [entry, function] {
        if let Some(a) = holder.get("annotations").filter(|a| a.is_object()) {
            return Some(a.clone());
        }
        let hints: serde_json::Map<String, Value> =
            ["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"]
                .iter()
                .filter_map(|k| holder.get(*k).map(|v| (k.to_string(), v.clone())))
                .collect();
        if !hints.is_empty() {
            return Some(Value::Object(hints));
        }
    }
    None
}
