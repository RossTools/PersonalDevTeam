//! Data references in node configs (Phase 7a spec §3.2): `{{trigger.pr}}`, `{{steps.finish.outcome}}`,
//! `{{state.baseline}}`. A string that is exactly one reference takes the value with its JSON type; any other string is
//! interpolated as text (a missing value as nothing).

use serde_json::Value;

/// The value at a dotted path (`a.b.0.c`); numeric segments index arrays.
pub fn lookup<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path.split('.').map(str::trim).filter(|s| !s.is_empty()) {
        cur = match cur {
            Value::Object(m) => m.get(seg)?,
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// The path when `s` is exactly one `{{path}}`.
pub fn whole_ref(s: &str) -> Option<&str> {
    let inner = s.trim().strip_prefix("{{")?.strip_suffix("}}")?;
    (!inner.contains("{{") && !inner.contains("}}")).then(|| inner.trim())
}

/// Whether `s` holds a reference.
pub fn has_ref(s: &str) -> bool {
    s.find("{{").is_some_and(|i| s[i..].contains("}}"))
}

fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `s` with its references filled in from `ctx`.
pub fn resolve_str(s: &str, ctx: &Value) -> Value {
    resolve_str_keeping(s, ctx, None)
}

fn root_is(path: &str, keep: Option<&str>) -> bool {
    keep.is_some_and(|k| path.trim().split('.').next() == Some(k))
}

fn resolve_str_keeping(s: &str, ctx: &Value, keep: Option<&str>) -> Value {
    if let Some(path) = whole_ref(s) {
        if root_is(path, keep) {
            return Value::String(s.to_string());
        }
        return lookup(ctx, path).cloned().unwrap_or(Value::Null);
    }
    if !has_ref(s) {
        return Value::String(s.to_string());
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else { break };
        out.push_str(&rest[..start]);
        let path = &rest[start + 2..start + len];
        if root_is(path, keep) {
            out.push_str(&rest[start..start + len + 2]);
        } else {
            out.push_str(&lookup(ctx, path.trim()).map(text_of).unwrap_or_default());
        }
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    Value::String(out)
}

/// A config with every string's references filled in (objects and arrays recursively).
pub fn resolve(v: &Value, ctx: &Value) -> Value {
    resolve_keeping(v, ctx, None)
}

/// [`resolve`], leaving references under `keep` (`event`: what a wait is waiting for) as they are.
pub fn resolve_keeping(v: &Value, ctx: &Value, keep: Option<&str>) -> Value {
    match v {
        Value::String(s) => resolve_str_keeping(s, ctx, keep),
        Value::Array(a) => Value::Array(a.iter().map(|x| resolve_keeping(x, ctx, keep)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), resolve_keeping(x, ctx, keep))).collect()),
        other => other.clone(),
    }
}

/// The references a config uses (validation and the 7d data picker).
pub fn refs_in(v: &Value) -> Vec<String> {
    let mut out = vec![];
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => {
                let mut rest = s.as_str();
                while let Some(start) = rest.find("{{") {
                    let Some(len) = rest[start..].find("}}") else { break };
                    out.push(rest[start + 2..start + len].trim().to_string());
                    rest = &rest[start + len + 2..];
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(m) => m.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    walk(v, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> Value {
        json!({"trigger": {"pr": 12, "ok": true, "jobs": ["a", "b"], "notice": {"text": "hi"}}, "state": {}})
    }

    #[test]
    fn a_whole_reference_keeps_its_type() {
        assert_eq!(resolve_str("{{trigger.pr}}", &ctx()), json!(12));
        assert_eq!(resolve_str(" {{ trigger.ok }} ", &ctx()), json!(true));
        assert_eq!(resolve_str("{{trigger.jobs.1}}", &ctx()), json!("b"));
        assert_eq!(resolve_str("{{trigger.nope}}", &ctx()), Value::Null);
    }

    #[test]
    fn other_strings_interpolate() {
        assert_eq!(resolve_str("PR #{{trigger.pr}}: {{trigger.notice.text}}", &ctx()), json!("PR #12: hi"));
        assert_eq!(resolve_str("[{{trigger.missing}}]", &ctx()), json!("[]"));
        assert_eq!(resolve_str("{{trigger.pr}}{{trigger.pr}}", &ctx()), json!("1212"));
        assert_eq!(resolve_str("plain {{ unclosed", &ctx()), json!("plain {{ unclosed"));
    }

    #[test]
    fn resolves_nested_configs_and_lists_refs() {
        let cfg = json!({"a": ["{{trigger.pr}}", 3], "b": {"c": "x {{state.k}}"}});
        assert_eq!(resolve(&cfg, &ctx()), json!({"a": [12, 3], "b": {"c": "x "}}));
        let mut refs = refs_in(&cfg);
        refs.sort();
        assert_eq!(refs, vec!["state.k", "trigger.pr"]);
    }

    #[test]
    fn keeps_references_under_a_root_for_later() {
        let cfg = json!({"left": "{{event.pr}}", "right": "{{trigger.pr}}", "t": "#{{event.pr}} vs {{trigger.pr}}"});
        let got = resolve_keeping(&cfg, &ctx(), Some("event"));
        assert_eq!(got, json!({"left": "{{event.pr}}", "right": 12, "t": "#{{event.pr}} vs 12"}));
        assert_eq!(resolve(&got, &json!({"event": {"pr": 12}}))["left"], json!(12));
    }
}
