//! Toucan-1.5M row normalization (W-9 lane 2 input).
//!
//! Input is the datasets-server `/rows` response shape as fetched by
//! `scripts/fetch-corpora`: `{"rows":[{"row":{uuid, messages,
//! available_tools, metadata, question, …}}]}` where `messages`,
//! `available_tools`, and `metadata` are JSON-in-strings.
//!
//! Strictness: any shape this mapping cannot express quarantines the
//! whole trajectory with a named gap kind. Recurring kinds are SI
//! candidates; nothing here interprets silently.

use super::model::{
    slug, verify_against_manifest, Call, GapLedger, ParseOutcome, ToolSchema, Trajectory,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

/// Load every `toucan/rows-*.json` under `corpora_dir`, in filename
/// order, normalizing at most `limit` rows (None = all). When a
/// FETCH.json manifest is present every consumed file is hash-verified
/// against it (fail closed on tamper/truncation).
pub fn load_dir(corpora_dir: &Path, limit: Option<usize>) -> Result<ParseOutcome> {
    let dir = corpora_dir.join("toucan");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("no toucan corpus at {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("rows-") && n.ends_with(".json"))
        })
        .collect();
    files.sort();
    let manifest: Option<Value> = std::fs::read_to_string(dir.join("FETCH.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());

    let mut out = ParseOutcome {
        revision: manifest
            .as_ref()
            .and_then(|m| m["revision"].as_str())
            .map(str::to_string),
        ..Default::default()
    };
    let mut seen = 0usize;
    'files: for f in files {
        let text = std::fs::read_to_string(&f)
            .with_context(|| format!("reading {}", f.display()))?;
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        verify_against_manifest(manifest.as_ref(), name, &text)
            .with_context(|| format!("integrity check failed for {}", f.display()))?;
        let parsed: Value =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", f.display()))?;
        for row in parsed["rows"].as_array().into_iter().flatten() {
            if let Some(max) = limit {
                if seen >= max {
                    break 'files;
                }
            }
            seen += 1;
            normalize_row(&row["row"], &mut out);
        }
    }
    Ok(out)
}

fn normalize_row(row: &Value, out: &mut ParseOutcome) {
    let uuid = row["uuid"]
        .as_str()
        .unwrap_or("(row without uuid)")
        .to_string();
    match normalize(row, &uuid, &mut out.gaps) {
        Ok(traj) => {
            if traj.calls.is_empty() {
                out.no_call_rows += 1;
            } else {
                out.trajectories.push(traj);
            }
        }
        Err(reason) => out.quarantined.push((uuid, reason)),
    }
}

/// Normalize one row; Err(reason) quarantines it. `gaps` records every
/// refusal (including quarantine reasons) with a bounded sample.
fn normalize(row: &Value, uuid: &str, gaps: &mut GapLedger) -> Result<Trajectory, String> {
    let refuse = |gaps: &mut GapLedger, kind: &str, sample: String| -> String {
        gaps.add(kind, sample);
        kind.to_string()
    };

    // ---- available_tools --------------------------------------------------
    let tools_raw: Value = match row["available_tools"].as_str().map(serde_json::from_str) {
        Some(Ok(v)) => v,
        _ => {
            return Err(refuse(
                gaps,
                "available_tools_unparseable",
                format!("{uuid}: {}", truncate(&row["available_tools"])),
            ))
        }
    };
    let tool_list = tools_raw.as_array().cloned().unwrap_or_default();
    if tool_list.is_empty() {
        return Err(refuse(gaps, "no_available_tools", uuid.to_string()));
    }

    // ---- metadata: server names + categories --------------------------------
    // Servers give the registration grouping (tool:corpus/<server>) and the
    // only domain-shaped metadata this corpus carries.
    let metadata: Value = row["metadata"]
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null);
    let mut servers: Vec<(String, Vec<String>)> = Vec::new();
    let mut server_names: Vec<(String, String)> = Vec::new(); // (slug, original)
    for s in metadata["mcp_servers"].as_array().into_iter().flatten() {
        let name = s["server_name"]
            .as_str()
            .or_else(|| s["server_info"]["name"].as_str())
            .unwrap_or("unknown-server");
        let sl = slug(name);
        // Slug identity guards: an unsluggable name (e.g. CJK-only) or
        // two distinct names collapsing to one slug would silently merge
        // server identities — the exact guess the mapping forbids.
        if sl.is_empty() {
            return Err(refuse(
                gaps,
                "server_name_not_sluggable",
                format!("{uuid}: {name}"),
            ));
        }
        match server_names.iter().find(|(s2, _)| *s2 == sl) {
            Some((_, orig)) if orig == name => continue, // duplicate listing
            Some((_, orig)) => {
                return Err(refuse(
                    gaps,
                    "server_slug_collision",
                    format!("{uuid}: '{orig}' vs '{name}' -> {sl}"),
                ))
            }
            None => {}
        }
        server_names.push((sl.clone(), name.to_string()));
        let cats = s["server_info"]["categories"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        servers.push((sl, cats));
    }
    if servers.is_empty() {
        return Err(refuse(gaps, "no_server_metadata", uuid.to_string()));
    }

    // ---- map tools to servers ----------------------------------------------
    // Toucan prefixes tool names with the server slug. Longest-prefix match;
    // a tool matching no server in a multi-server row is ambiguous → refuse.
    let mut tools = Vec::new();
    for t in &tool_list {
        let f = &t["function"];
        let name = match f["name"].as_str() {
            Some(n) => n.to_string(),
            None => {
                return Err(refuse(
                    gaps,
                    "tool_without_name",
                    format!("{uuid}: {}", truncate(t)),
                ))
            }
        };
        let server = match match_server(&name, &servers) {
            Some(s) => s,
            None if servers.len() == 1 => servers[0].clone(),
            None => {
                return Err(refuse(
                    gaps,
                    "tool_server_ambiguous",
                    format!("{uuid}: {name}"),
                ))
            }
        };
        tools.push(ToolSchema {
            server: server.0,
            name,
            description: f["description"].as_str().map(str::to_string),
            parameters: f.get("parameters").filter(|p| !p.is_null()).cloned(),
            annotations: extract_annotations(t, f),
            categories: server.1,
            provenance: None,
            source: "toucan",
        });
    }

    // ---- messages → ordered calls -------------------------------------------
    let messages: Value = match row["messages"].as_str().map(serde_json::from_str) {
        Some(Ok(v)) => v,
        _ => {
            return Err(refuse(
                gaps,
                "messages_unparseable",
                format!("{uuid}: {}", truncate(&row["messages"])),
            ))
        }
    };
    let known: std::collections::BTreeSet<&str> =
        tools.iter().map(|t| t.name.as_str()).collect();
    let mut calls: Vec<Call> = Vec::new();
    let mut unresulted: Vec<usize> = Vec::new(); // call idx awaiting a result
    for m in messages.as_array().into_iter().flatten() {
        match m["role"].as_str() {
            Some("assistant") => {
                let mut proposed = Vec::new();
                if let Some(fc) = m.get("function_call").filter(|v| !v.is_null()) {
                    proposed.push(fc.clone());
                }
                for tc in m["tool_calls"].as_array().into_iter().flatten() {
                    proposed.push(tc["function"].clone());
                }
                for fc in proposed {
                    let name = match fc["name"].as_str() {
                        Some(n) => n.to_string(),
                        None => {
                            return Err(refuse(
                                gaps,
                                "call_without_name",
                                format!("{uuid}: {}", truncate(&fc)),
                            ))
                        }
                    };
                    let arguments = match &fc["arguments"] {
                        Value::String(s) => match serde_json::from_str(s) {
                            Ok(v) => v,
                            Err(_) => {
                                return Err(refuse(
                                    gaps,
                                    "unparseable_call_arguments",
                                    format!("{uuid}: {name}: {}", truncate(&fc["arguments"])),
                                ))
                            }
                        },
                        Value::Object(_) => fc["arguments"].clone(),
                        other => {
                            return Err(refuse(
                                gaps,
                                "unparseable_call_arguments",
                                format!("{uuid}: {name}: {}", truncate(other)),
                            ))
                        }
                    };
                    unresulted.push(calls.len());
                    calls.push(Call {
                        unregistered: !known.contains(name.as_str()),
                        tool_name: name,
                        arguments,
                        result: None,
                    });
                }
            }
            Some("function") | Some("tool") => {
                // Results attach to the oldest unresulted call (Toucan
                // emits them in proposal order), and where the corpus
                // carries a `name` on the result it MUST corroborate the
                // pairing — a mismatch means the order assumption is
                // wrong for this row, so refuse rather than mis-attach.
                match unresulted.first().copied() {
                    Some(idx) => {
                        if let Some(rname) = m["name"].as_str() {
                            if rname != calls[idx].tool_name {
                                return Err(refuse(
                                    gaps,
                                    "result_name_mismatch",
                                    format!(
                                        "{uuid}: result '{rname}' vs pending '{}'",
                                        calls[idx].tool_name
                                    ),
                                ));
                            }
                        }
                        calls[idx].result = match m.get("content") {
                            None | Some(Value::Null) => None,
                            Some(Value::String(s)) => Some(s.clone()),
                            Some(other) => {
                                // Structured content is a shape this
                                // mapping does not express — gap, not a
                                // silent None.
                                return Err(refuse(
                                    gaps,
                                    "result_content_not_string",
                                    format!("{uuid}: {}", truncate(other)),
                                ));
                            }
                        };
                        unresulted.remove(0);
                    }
                    None => {
                        return Err(refuse(
                            gaps,
                            "orphan_tool_result",
                            format!("{uuid}: {}", truncate(&m["content"])),
                        ))
                    }
                }
            }
            _ => {}
        }
    }

    Ok(Trajectory {
        uuid: uuid.to_string(),
        question: row["question"].as_str().map(str::to_string),
        tools,
        calls,
    })
}

fn match_server(
    tool_name: &str,
    servers: &[(String, Vec<String>)],
) -> Option<(String, Vec<String>)> {
    let name_slug = slug(tool_name);
    servers
        .iter()
        .filter(|(s, _)| name_slug == *s || name_slug.starts_with(&format!("{s}-")))
        .max_by_key(|(s, _)| s.len())
        .cloned()
}

/// MCP ToolAnnotations survive in some serializations as an
/// `annotations` object or as bare hint keys; capture either.
fn extract_annotations(tool: &Value, function: &Value) -> Option<Value> {
    for holder in [tool, function] {
        if let Some(a) = holder.get("annotations").filter(|a| a.is_object()) {
            return Some(a.clone());
        }
        let hints: serde_json::Map<String, Value> = ["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"]
            .iter()
            .filter_map(|k| holder.get(*k).map(|v| (k.to_string(), v.clone())))
            .collect();
        if !hints.is_empty() {
            return Some(Value::Object(hints));
        }
    }
    None
}

fn truncate(v: &Value) -> String {
    // Char-boundary-safe (String::truncate panics on multibyte content).
    v.to_string().chars().take(120).collect()
}
