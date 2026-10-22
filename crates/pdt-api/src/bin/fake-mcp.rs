//! A stand-in MCP server (stdio, newline-delimited JSON-RPC) for the `mcp.tool` action's tests (Phase 7e-3), the same
//! on every OS. It speaks protocol version 2025-06-18, as most servers in the wild do. Its tools:
//!
//! - `echo {text, times?}`: the text, `times` times, as text content
//! - `add {a, b}`: structured content `{sum}`
//! - `fail`: an `isError` result saying `it broke`
//! - `whoami`: text content holding JSON `{secret: $FAKE_MCP_SECRET, pid}` (which process answered, and its secret)

use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn tools() -> Value {
    json!([
        {"name": "echo", "title": "Echo", "description": "Says the text back.",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}, "times": {"type": "integer"}},
                "required": ["text"], "additionalProperties": false}},
        {"name": "add", "description": "Adds two numbers.",
            "inputSchema": {"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}},
                "required": ["a", "b"]},
            "outputSchema": {"type": "object", "properties": {"sum": {"type": "number"}}}},
        {"name": "fail", "description": "Always fails.", "inputSchema": {"type": "object"}},
        {"name": "whoami", "description": "Which process answered.", "inputSchema": {"type": "object"}}
    ])
}

fn text(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}]})
}

fn call(params: &Value) -> Value {
    let args = &params["arguments"];
    match params["name"].as_str().unwrap_or_default() {
        "echo" => text(args["text"].as_str().unwrap_or_default().repeat(args["times"].as_u64().unwrap_or(1) as usize)),
        "add" => {
            let sum = args["a"].as_f64().unwrap_or(0.0) + args["b"].as_f64().unwrap_or(0.0);
            json!({"content": [{"type": "text", "text": sum.to_string()}], "structuredContent": {"sum": sum}})
        }
        "fail" => json!({"content": [{"type": "text", "text": "it broke"}], "isError": true}),
        "whoami" => {
            text(json!({"secret": std::env::var("FAKE_MCP_SECRET").ok(), "pid": std::process::id()}).to_string())
        }
        other => json!({"content": [{"type": "text", "text": format!("no tool {other}")}], "isError": true}),
    }
}

/// The answer to one request (None for a notification).
fn answer(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let result = match msg["method"].as_str().unwrap_or_default() {
        "initialize" => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
            "serverInfo": {"name": "fake-mcp", "version": "1"}}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => call(&msg["params"]),
        "ping" => json!({}),
        other => {
            let error = json!({"code": -32601, "message": format!("no method {other}")});
            return Some(json!({"jsonrpc": "2.0", "id": id, "error": error}));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        eprintln!("fake-mcp got {}", msg["method"]);
        if let Some(a) = answer(&msg) {
            let _ = writeln!(out, "{a}");
            let _ = out.flush();
        }
    }
}
