//! Minimal MCP wire helpers: newline-delimited JSON-RPC 2.0 over stdio.
//!
//! Hand-rolled deliberately (ADR 0003): a *proxy* wants to be
//! protocol-transparent — every message it does not understand must pass
//! through byte-faithfully, which a typed SDK makes harder, not easier.
//! Only `tools/call` is ever interpreted.

use anyhow::Result;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// Read one JSON-RPC message (one line). `None` on EOF.
pub fn read_msg(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        return Ok(Some(serde_json::from_str(trimmed)?));
    }
}

pub fn write_msg(writer: &mut impl Write, msg: &Value) -> Result<()> {
    let s = serde_json::to_string(msg)?;
    writer.write_all(s.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

/// Is this a request (has id + method)? Notifications have method, no id;
/// responses have id, no method.
pub fn is_request(msg: &Value) -> bool {
    msg.get("id").is_some() && msg.get("method").is_some()
}

pub fn is_response(msg: &Value) -> bool {
    msg.get("id").is_some() && msg.get("method").is_none()
}

/// Stable map key for a JSON-RPC id (number or string).
pub fn id_key(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_default()
}

pub fn response(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// An MCP tool-result carrying an error the *agent* is meant to read
/// (broker denial, escalation notice) — a tool-level error, not a protocol
/// error, so any MCP client renders it.
pub fn tool_error(id: &Value, text: &str) -> Value {
    response(
        id,
        json!({ "content": [ { "type": "text", "text": text } ], "isError": true }),
    )
}

pub fn tool_text(id: &Value, text: &str) -> Value {
    response(
        id,
        json!({ "content": [ { "type": "text", "text": text } ], "isError": false }),
    )
}

pub fn method_not_found(id: &Value, method: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id,
            "error": { "code": -32601, "message": format!("method not found: {method}") } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn blank_lines_are_ignored_and_eof_is_clean() {
        let mut wire = Cursor::new(b"\n  \n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
        let message = read_msg(&mut wire).unwrap().unwrap();
        assert!(is_request(&message));
        assert!(read_msg(&mut wire).unwrap().is_none());
    }

    #[test]
    fn malformed_json_fails_instead_of_becoming_a_message() {
        let mut wire = Cursor::new(b"{not-json}\n");
        assert!(read_msg(&mut wire).is_err());
    }

    #[test]
    fn request_notification_and_response_shapes_do_not_overlap() {
        let request = json!({"jsonrpc":"2.0","id":"x","method":"tools/list"});
        let notification = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        let response = json!({"jsonrpc":"2.0","id":"x","result":{}});
        assert!(is_request(&request));
        assert!(!is_response(&request));
        assert!(!is_request(&notification));
        assert!(!is_response(&notification));
        assert!(is_response(&response));
        assert!(!is_request(&response));
    }

    #[test]
    fn string_and_numeric_ids_route_to_distinct_keys() {
        assert_ne!(id_key(&json!(7)), id_key(&json!("7")));
        assert_eq!(response(&json!(7), json!({"ok":true}))["id"], 7);
    }
}
