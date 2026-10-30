//! Dry runs (PDT-189): an action that only says what it would do still gives later steps the fields they read, as
//! stand-ins shaped like its output schema (`<ticketId>`), so the dry run goes on to the end.

use serde_json::{Map, Value, json};

/// `output` (a dry action's `{"would": …}`) with every field of `schema` (the kind's output JSON Schema) it lacks
/// filled in by a stand-in: `<name>` for text, 0, false, [] or a nested object. A schema without fields adds nothing.
pub fn with_stand_ins(output: Value, schema: &Value) -> Value {
    let Value::Object(mut out) = output else { return output };
    if let Value::Object(fill) = walk(schema, schema, "value", 0) {
        for (k, v) in fill {
            out.entry(k).or_insert(v);
        }
    }
    Value::Object(out)
}

/// Whether `s` is a dry run's stand-in (`<ticketId>`), not a real value.
pub fn is_stand_in(s: &str) -> bool {
    let s = s.trim();
    s.len() > 2 && s.starts_with('<') && s.ends_with('>') && !s[1..s.len() - 1].contains(['<', '>'])
}

const MAX_DEPTH: usize = 8;

fn walk(schema: &Value, root: &Value, name: &str, depth: usize) -> Value {
    let text = || json!(format!("<{name}>"));
    let Value::Object(s) = schema else { return text() };
    if depth > MAX_DEPTH {
        return Value::Null;
    }
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        return match r.strip_prefix('#').and_then(|p| root.pointer(p)) {
            Some(target) => walk(target, root, name, depth + 1),
            None => text(),
        };
    }
    if let Some(v) = s.get("const") {
        return v.clone();
    }
    if let Some(first) = s.get("enum").and_then(Value::as_array).and_then(|e| e.iter().find(|v| !v.is_null())) {
        return first.clone();
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(list) = s.get(key).and_then(Value::as_array) {
            let picked = list.iter().find(|x| x.get("type").and_then(Value::as_str) != Some("null"));
            if let Some(x) = picked {
                return walk(x, root, name, depth + 1);
            }
        }
    }
    let ty = match s.get("type") {
        Some(Value::String(t)) => Some(t.as_str()),
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).find(|t| *t != "null"),
        _ => None,
    };
    match ty {
        Some("string") => text(),
        Some("integer" | "number") => json!(0),
        Some("boolean") => json!(false),
        Some("array") => json!([]),
        Some("null") => Value::Null,
        Some("object") | None if s.contains_key("properties") => {
            let mut m = Map::new();
            if let Some(Value::Object(props)) = s.get("properties") {
                for (k, p) in props {
                    m.insert(k.clone(), walk(p, root, k, depth + 1));
                }
            }
            Value::Object(m)
        }
        Some("object") => json!({}),
        _ => text(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_the_fields_a_dry_output_lacks() {
        let schema = json!({
            "type": "object",
            "properties": {
                "ticketId": {"type": "string"},
                "number": {"type": "integer"},
                "existed": {"type": "boolean"},
                "labels": {"type": "array", "items": {"type": "string"}},
                "note": {"type": ["string", "null"]},
                "stage": {"$ref": "#/$defs/Stage"},
                "pr": {"$ref": "#/$defs/Pr"},
            },
            "$defs": {
                "Stage": {"type": "string", "enum": ["draft", "todo"]},
                "Pr": {"type": "object", "properties": {"url": {"type": "string"}}},
            },
        });
        let out = with_stand_ins(json!({"would": "import a ticket", "number": 7}), &schema);
        assert_eq!(
            out,
            json!({
                "would": "import a ticket", "ticketId": "<ticketId>", "number": 7, "existed": false, "labels": [],
                "note": "<note>", "stage": "draft", "pr": {"url": "<url>"},
            })
        );
    }

    #[test]
    fn a_schema_without_fields_adds_nothing() {
        assert_eq!(with_stand_ins(json!({"would": "x"}), &json!(true)), json!({"would": "x"}));
        assert_eq!(with_stand_ins(json!({"would": "x"}), &json!({})), json!({"would": "x"}));
        assert_eq!(with_stand_ins(json!("text"), &json!({"properties": {"a": {}}})), json!("text"));
    }

    #[test]
    fn a_self_referencing_schema_ends() {
        let schema = json!({"$ref": "#/$defs/N", "$defs": {"N": {"type": "object", "properties": {"next": {"$ref": "#/$defs/N"}}}}});
        let out = with_stand_ins(json!({}), &schema);
        assert!(out.get("next").is_some());
    }

    #[test]
    fn tells_stand_ins_from_values() {
        assert!(is_stand_in("<ticketId>"));
        assert!(!is_stand_in("PDT-12"));
        assert!(!is_stand_in("<>"));
        assert!(!is_stand_in("<a> and <b>"));
    }
}
