//! `asf vault-server` — a deliberately dumb downstream MCP server exposing
//! vault file operations. Stands in for "whatever tool server the agent
//! already uses" so the proxy has something real to front (workflow 3:
//! vault maintenance). It enforces nothing — bounding it is the broker's
//! job — except path-sandboxing to the vault root, which is basic hygiene.

use crate::mcp;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::{stdin, stdout, BufReader};
use std::path::{Path, PathBuf};

fn safe_join(vault: &Path, rel: &str) -> Result<PathBuf> {
    if rel.starts_with('/') || rel.split('/').any(|s| s == "..") {
        bail!("path escapes vault: {rel}");
    }
    Ok(vault.join(rel))
}

fn tool_defs() -> Value {
    let path_schema = json!({ "type": "object",
        "properties": { "path": { "type": "string" } }, "required": ["path"] });
    json!([
        { "name": "note.list",   "description": "List notes and folders (one level; folders end with /)",
          "inputSchema": { "type": "object",
            "properties": { "path": { "type": "string", "description": "Folder relative to vault root; empty or omitted for the root" } } } },
        { "name": "note.read",   "description": "Read a note",
          "inputSchema": path_schema },
        { "name": "note.write",  "description": "Write a note",
          "inputSchema": { "type": "object",
            "properties": { "path": { "type": "string" }, "content": { "type": "string" } },
            "required": ["path", "content"] } },
        { "name": "note.edit",   "description": "Replace one exact occurrence of old_string with new_string in a note (old_string must match exactly once)",
          "inputSchema": { "type": "object",
            "properties": { "path": { "type": "string" },
                            "old_string": { "type": "string" },
                            "new_string": { "type": "string" } },
            "required": ["path", "old_string", "new_string"] } },
        { "name": "note.move",   "description": "Move a note",
          "inputSchema": { "type": "object",
            "properties": { "src": { "type": "string" }, "dest": { "type": "string" } },
            "required": ["src", "dest"] } },
        { "name": "note.delete", "description": "Delete a note",
          "inputSchema": path_schema },
    ])
}

fn call(vault: &Path, name: &str, args: &Value) -> Result<String> {
    let s = |field: &str| -> Result<&str> {
        args.get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing argument {field}"))
    };
    Ok(match name {
        "note.list" => {
            let rel = args.get("path").and_then(Value::as_str).unwrap_or("");
            let dir = safe_join(vault, rel)?;
            let mut entries: Vec<String> = fs::read_dir(&dir)?
                .flatten()
                .map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if e.path().is_dir() { format!("{name}/") } else { name }
                })
                .collect();
            entries.sort();
            if entries.is_empty() {
                "(empty)".to_string()
            } else {
                entries.join("\n")
            }
        }
        "note.read" => fs::read_to_string(safe_join(vault, s("path")?)?)?,
        "note.write" => {
            let p = safe_join(vault, s("path")?)?;
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&p, s("content")?)?;
            format!("wrote {}", s("path")?)
        }
        "note.edit" => {
            let p = safe_join(vault, s("path")?)?;
            let (old, new) = (s("old_string")?, s("new_string")?);
            let content = fs::read_to_string(&p)?;
            match content.matches(old).count() {
                0 => bail!("old_string not found in {}", s("path")?),
                1 => {
                    fs::write(&p, content.replacen(old, new, 1))?;
                    format!("edited {}", s("path")?)
                }
                n => bail!(
                    "old_string matches {n} times in {}; include more context to make it unique",
                    s("path")?
                ),
            }
        }
        "note.move" => {
            let (src, dest) = (safe_join(vault, s("src")?)?, safe_join(vault, s("dest")?)?);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(src, dest)?;
            format!("moved {} -> {}", s("src")?, s("dest")?)
        }
        "note.delete" => {
            fs::remove_file(safe_join(vault, s("path")?)?)?;
            format!("deleted {}", s("path")?)
        }
        other => bail!("unknown tool {other}"),
    })
}

pub fn run(vault: PathBuf) -> Result<()> {
    let mut reader = BufReader::new(stdin());
    let mut out = stdout();
    while let Some(msg) = mcp::read_msg(&mut reader)? {
        let (id, method) = (msg.get("id").cloned(), msg["method"].as_str().unwrap_or(""));
        let Some(id) = id else { continue }; // notifications need no reply
        let reply = match method {
            "initialize" => mcp::response(
                &id,
                json!({
                    "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26"),
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "asf-vault-server", "version": env!("CARGO_PKG_VERSION") },
                }),
            ),
            "ping" => mcp::response(&id, json!({})),
            "tools/list" => mcp::response(&id, json!({ "tools": tool_defs() })),
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("");
                let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
                match call(&vault, name, &args) {
                    Ok(text) => mcp::tool_text(&id, &text),
                    Err(e) => mcp::tool_error(&id, &format!("{e}")),
                }
            }
            other => mcp::method_not_found(&id, other),
        };
        mcp::write_msg(&mut out, &reply)?;
    }
    Ok(())
}
