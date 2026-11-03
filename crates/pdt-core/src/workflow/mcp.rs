//! MCP tools as workflow actions: checking an `mcp.tool` node's `arguments` against the tool's
//! input schema, and turning a tool's result into a step's output. Pure.

use serde_json::{Map, Value};

fn is_reference(v: &Value) -> bool {
    v.as_str().is_some_and(|s| s.contains("{{"))
}

/// The JSON type names `v` fits (an integer is also a `number`).
fn types_of(v: &Value) -> &'static [&'static str] {
    match v {
        Value::Null => &["null"],
        Value::Bool(_) => &["boolean"],
        Value::Number(n) if n.is_i64() || n.is_u64() => &["integer", "number"],
        Value::Number(_) => &["number"],
        Value::String(_) => &["string"],
        Value::Array(_) => &["array"],
        Value::Object(_) => &["object"],
    }
}

/// Whether `v` has a type `schema` allows (a schema without `type` allows any).
fn type_fits(schema: &Value, v: &Value) -> bool {
    let allowed: Vec<&str> = match &schema["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(ts) => ts.iter().filter_map(|t| t.as_str()).collect(),
        _ => return true,
    };
    types_of(v).iter().any(|t| allowed.contains(t))
}

/// What is wrong with `arguments` for a tool whose input schema is `schema`: missing required properties, properties
/// the schema refuses (`additionalProperties: false`), and literal values of the wrong type. A value that is a
/// reference (`{{…}}`) is only known when the step runs, so it is never wrong here; nor is anything deeper than the
/// top level, which the server checks.
pub fn check_arguments(schema: &Value, arguments: &Value) -> Vec<String> {
    if is_reference(arguments) {
        return vec![];
    }
    let empty = Map::new();
    let args = match arguments {
        Value::Object(m) => m,
        Value::Null => &empty,
        other => return vec![format!("arguments must be an object, not {other}")],
    };
    let props = schema["properties"].as_object();
    let mut out = vec![];
    for r in schema["required"].as_array().into_iter().flatten().filter_map(|r| r.as_str()) {
        if !args.contains_key(r) {
            out.push(format!("{r} is required"));
        }
    }
    for (k, v) in args {
        match props.and_then(|p| p.get(k)) {
            Some(p) if !is_reference(v) && !type_fits(p, v) => {
                out.push(format!(
                    "{k} should be {}",
                    p["type"].as_str().map_or_else(|| p["type"].to_string(), String::from)
                ));
            }
            Some(_) => {}
            None if schema["additionalProperties"] == Value::Bool(false) => {
                out.push(format!("{k} is not one of the tool's arguments"));
            }
            None => {}
        }
    }
    out
}

/// A tool's result (MCP `CallToolResult` as JSON) as a step's output: its structured content if it has any, else its
/// text (parsed when it is a JSON object or list). `isError` is an error with the text.
pub fn tool_output(result: &Value) -> Result<Value, String> {
    let text: Vec<&str> = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["type"] == "text")
        .filter_map(|c| c["text"].as_str())
        .collect();
    let text = text.join("\n");
    if result["isError"] == Value::Bool(true) {
        return Err(if text.trim().is_empty() { "the tool failed".into() } else { text });
    }
    if let Some(s) = result.get("structuredContent") {
        return Ok(s.clone());
    }
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(v @ (Value::Object(_) | Value::Array(_))) => Ok(v),
        _ => Ok(Value::String(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({"type": "object", "properties": {"text": {"type": "string"}, "times": {"type": "integer"},
            "ratio": {"type": ["number", "null"]}}, "required": ["text"], "additionalProperties": false})
    }

    #[test]
    fn required_types_and_unknown_arguments_are_checked() {
        assert!(check_arguments(&schema(), &json!({"text": "hi", "times": 2, "ratio": 0.5})).is_empty());
        assert!(check_arguments(&schema(), &json!({"text": "hi", "ratio": null})).is_empty());
        assert_eq!(check_arguments(&schema(), &json!({})), ["text is required"]);
        assert_eq!(check_arguments(&schema(), &json!(null)), ["text is required"]);
        assert_eq!(check_arguments(&schema(), &json!({"text": 3})), ["text should be string"]);
        assert_eq!(check_arguments(&schema(), &json!({"text": "a", "times": 1.5})), ["times should be integer"]);
        assert_eq!(check_arguments(&schema(), &json!({"text": "a", "x": 1})), ["x is not one of the tool's arguments"]);
        assert_eq!(check_arguments(&schema(), &json!([1])), ["arguments must be an object, not [1]"]);
    }

    #[test]
    fn references_are_left_for_the_run() {
        assert!(check_arguments(&schema(), &json!({"text": "hi", "times": "{{trigger.n}}"})).is_empty());
        assert!(check_arguments(&schema(), &json!("{{steps.a.args}}")).is_empty());
        // Extra properties are fine unless the schema says otherwise; no schema checks nothing.
        assert!(check_arguments(&json!({"type": "object"}), &json!({"anything": 1})).is_empty());
        assert!(check_arguments(&json!({}), &json!({"a": 1})).is_empty());
    }

    #[test]
    fn structured_content_wins_then_text() {
        let r = json!({"content": [{"type": "text", "text": "3"}], "structuredContent": {"sum": 3}});
        assert_eq!(tool_output(&r), Ok(json!({"sum": 3})));
        let r = json!({"content": [{"type": "text", "text": "a"}, {"type": "image", "data": "x"}, {"type": "text", "text": "b"}]});
        assert_eq!(tool_output(&r), Ok(json!("a\nb")));
        assert_eq!(tool_output(&json!({"content": [{"type": "text", "text": "{\"x\": 1}"}]})), Ok(json!({"x": 1})));
        assert_eq!(tool_output(&json!({"content": [{"type": "text", "text": "42"}]})), Ok(json!("42")));
        assert_eq!(
            tool_output(&json!({"content": [{"type": "text", "text": "no"}], "isError": true})),
            Err("no".into())
        );
        assert_eq!(tool_output(&json!({"content": [], "isError": true})), Err("the tool failed".into()));
    }
}
