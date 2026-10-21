//! pdt-mcp's protocol (Phase 7b spec §2): JSON-RPC 2.0 messages, MCP's handshake and tool results, and the session
//! tokens. Pure: pdt-api serves it at `/api/mcp`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The protocol versions pdt-mcp speaks, newest first.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// The name agents see the server under (`mcp__pdt__get_ticket` in Claude).
pub const SERVER_NAME: &str = "pdt";

/// JSON-RPC error codes.
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub jsonrpc: String,
    /// None: a notification (no answer).
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0", id, result: Some(result), error: None }
    }
    pub fn err(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self { jsonrpc: "2.0", id, result: None, error: Some(RpcError { code, message: message.into() }) }
    }
}

/// The version to answer `initialize` with: the client's if pdt-mcp speaks it, else the newest.
pub fn negotiate(requested: Option<&str>) -> &'static str {
    requested.and_then(|r| PROTOCOL_VERSIONS.iter().find(|v| **v == r).copied()).unwrap_or(PROTOCOL_VERSIONS[0])
}

/// `initialize`'s result.
pub fn initialize_result(requested: Option<&str>, version: &str, instructions: &str) -> Value {
    json!({
        "protocolVersion": negotiate(requested),
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": SERVER_NAME, "title": crate::brand::PRODUCT_NAME, "version": version},
        "instructions": instructions,
    })
}

/// One tool as `tools/list` gives it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    /// `readOnlyHint` and the like: an agent may run read-only tools without asking (Claude's plan mode).
    pub annotations: Value,
}

/// A tool call's result: text the agent reads, plus the JSON it came from.
pub fn tool_result(value: &Value) -> Value {
    let text = match value {
        Value::String(s) => s.clone(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    };
    let mut r = json!({"content": [{"type": "text", "text": text}], "isError": false});
    if value.is_object() {
        r["structuredContent"] = value.clone();
    }
    r
}

/// A tool call that failed in a way the agent can act on.
pub fn tool_error(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

/// A JSON Schema fit for a tool's `inputSchema`: an object schema, with schemars' `$schema` and `title` dropped.
pub fn input_schema(mut schema: Value) -> Value {
    if let Value::Object(m) = &mut schema {
        m.remove("$schema");
        m.remove("title");
        m.entry("type").or_insert(json!("object"));
        if !m.contains_key("properties") {
            m.insert("properties".into(), json!({}));
        }
    }
    schema
}

/// The token for a session: `<session id>.<mac>`, where `mac(session id)` is the host key's HMAC.
pub fn token(session_id: &str, mac: &str) -> String {
    format!("{session_id}.{mac}")
}

/// The session id and MAC of a bearer token (`Bearer <session id>.<mac>`).
pub fn parse_token(header: &str) -> Option<(&str, &str)> {
    let t = header.strip_prefix("Bearer ").or_else(|| header.strip_prefix("bearer "))?.trim();
    let (id, mac) = t.rsplit_once('.')?;
    (!id.is_empty() && !mac.is_empty()).then_some((id, mac))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_a_known_version_or_the_newest() {
        assert_eq!(negotiate(Some("2025-06-18")), "2025-06-18");
        assert_eq!(negotiate(Some("1999-01-01")), PROTOCOL_VERSIONS[0]);
        assert_eq!(negotiate(None), PROTOCOL_VERSIONS[0]);
        let r = initialize_result(Some("2025-03-26"), "1.0", "hi");
        assert_eq!(r["protocolVersion"], "2025-03-26");
        assert_eq!(r["serverInfo"]["name"], "pdt");
        assert_eq!(r["capabilities"]["tools"]["listChanged"], false);
    }

    #[test]
    fn requests_and_notifications_parse_and_responses_serialize() {
        let r: Request = serde_json::from_value(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"})).unwrap();
        assert_eq!((r.id, r.method.as_str(), r.params), (Some(json!(3)), "tools/list", Value::Null));
        let n: Request =
            serde_json::from_value(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).unwrap();
        assert!(n.id.is_none());
        let ok = serde_json::to_value(Response::ok(json!("a"), json!({}))).unwrap();
        assert_eq!(ok, json!({"jsonrpc": "2.0", "id": "a", "result": {}}));
        let e = serde_json::to_value(Response::err(json!(1), METHOD_NOT_FOUND, "no")).unwrap();
        assert_eq!(e["error"]["code"], -32601);
        assert!(e.get("result").is_none());
    }

    #[test]
    fn results_carry_text_and_structure() {
        let r = tool_result(&json!({"ok": true}));
        assert_eq!(r["isError"], false);
        assert_eq!(r["structuredContent"]["ok"], true);
        assert!(r["content"][0]["text"].as_str().unwrap().contains("\"ok\": true"));
        assert_eq!(tool_result(&json!("done"))["content"][0]["text"], "done");
        assert!(tool_result(&json!("done")).get("structuredContent").is_none());
        assert_eq!(tool_error("nope")["isError"], true);
    }

    #[test]
    fn schemas_lose_their_header_and_tokens_split() {
        let s = input_schema(json!({"$schema": "x", "title": "T", "type": "object", "properties": {"a": {}}}));
        assert_eq!(s, json!({"type": "object", "properties": {"a": {}}}));
        assert_eq!(input_schema(json!({"type": "object"}))["properties"], json!({}));
        let t = token("0199-abc", "ff00");
        assert_eq!(parse_token(&format!("Bearer {t}")), Some(("0199-abc", "ff00")));
        assert_eq!(parse_token("Bearer nodot"), None);
        assert_eq!(parse_token("Basic a.b"), None);
    }
}
