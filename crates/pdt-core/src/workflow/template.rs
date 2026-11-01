//! Workflow templates (Phase 7d spec §5): a document with parameters. The document uses `{{params.<id>}}`;
//! instantiating fills them in (a string that is exactly one reference takes the value with its type) and leaves every
//! other reference alone.

use super::doc::WorkflowDoc;
use super::refs::resolve_only;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ParamType {
    /// One line of text.
    String,
    /// Several lines (a message).
    Text,
    Number,
    Boolean,
    /// One of `choices`.
    Choice,
    /// A connection's name (of `connectionKind`, when given): the form offers the connections there are.
    Connection,
    /// A list of text (labels): a JSON list, or one string with the items comma-separated.
    List,
    /// A length of time, in milliseconds in the document: a number of ms, or text such as `90m`, `6h`, `3d`.
    Duration,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TemplateParam {
    /// `{{params.<id>}}` in the document.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub kind: ParamType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "unknown")]
    pub default: Option<Value>,
    /// Instantiating needs a value (when there is no default).
    #[serde(default)]
    pub required: bool,
    /// A `connection` param's kind (`http`); none: any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub connection_kind: Option<String>,
}

/// Where a template sits in the New workflow gallery (PDT-229).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "TemplateCategory")]
pub enum Category {
    /// CI and pull requests.
    CiPr,
    Tickets,
    Issues,
    Notifications,
    Releases,
    /// The PM's conversations.
    Pm,
    /// The user's own templates, and anything else.
    #[default]
    Other,
}

/// A template as the API lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "WorkflowTemplate")]
pub struct Template {
    /// `builtin:<id>` for the ones PDT ships.
    pub id: String,
    pub name: String,
    pub description: String,
    pub builtin: bool,
    /// Its group in the gallery; the user's own are [`Category::Other`].
    #[serde(default)]
    pub category: Category,
    pub params: Vec<TemplateParam>,
    pub doc: WorkflowDoc,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A value of the param's type, or why not.
fn typed(p: &TemplateParam, v: &Value) -> Result<Value, String> {
    let bad = || format!("{} must be {}", p.id, describe(p));
    match p.kind {
        ParamType::String | ParamType::Text | ParamType::Connection => v.as_str().map(|s| json!(s)).ok_or_else(bad),
        ParamType::Number => match v {
            Value::Number(_) => Ok(v.clone()),
            Value::String(s) => s
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(|f| serde_json::Number::from_f64(f).map(Value::Number))
                .map(|n| match n.as_f64() {
                    Some(f) if f.fract() == 0.0 && f.abs() < 9e15 => json!(f as i64),
                    _ => n,
                })
                .ok_or_else(bad),
            _ => Err(bad()),
        },
        ParamType::Boolean => v.as_bool().map(Value::Bool).ok_or_else(bad),
        ParamType::Choice => match v.as_str() {
            Some(s) if p.choices.iter().any(|c| c == s) => Ok(json!(s)),
            _ => Err(bad()),
        },
        ParamType::Duration => duration_ms(v).map(|ms| json!(ms)).ok_or_else(bad),
        ParamType::List => {
            let items: Vec<&str> = match v {
                Value::String(s) => s.split(',').collect(),
                Value::Array(a) => a.iter().map(|x| x.as_str().ok_or_else(bad)).collect::<Result<_, _>>()?,
                _ => return Err(bad()),
            };
            let items: Vec<&str> = items.into_iter().map(str::trim).filter(|s| !s.is_empty()).collect();
            Ok(json!(items))
        }
    }
}

/// Milliseconds from a whole number of them, or from text: digits with an optional unit (`ms`, `s`, `m`, `h`, `d`).
fn duration_ms(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => {
            let s = s.trim().to_ascii_lowercase();
            let digits = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
            let (n, unit) = s.split_at(digits);
            let scale: u64 = match unit.trim() {
                "" | "ms" => 1,
                "s" => 1000,
                "m" => 60_000,
                "h" => 3_600_000,
                "d" => 86_400_000,
                _ => return None,
            };
            n.parse::<u64>().ok()?.checked_mul(scale)
        }
        _ => None,
    }
}

fn describe(p: &TemplateParam) -> String {
    match p.kind {
        ParamType::String | ParamType::Text => "text".into(),
        ParamType::Connection => "a connection's name".into(),
        ParamType::Number => "a number".into(),
        ParamType::Boolean => "true or false".into(),
        ParamType::Choice => format!("one of {}", p.choices.join(", ")),
        ParamType::List => "a list of text".into(),
        ParamType::Duration => "a duration (milliseconds, or such as 90m, 6h, 3d)".into(),
    }
}

/// Whether a param list makes sense: ids unique and usable in a reference, choices for a choice, defaults of the
/// right type.
pub fn check_params(params: &[TemplateParam]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for p in params {
        if !valid_id(&p.id) {
            return Err(format!("a param id must be letters, digits and _ (not {:?})", p.id));
        }
        if !seen.insert(p.id.as_str()) {
            return Err(format!("two params are called {}", p.id));
        }
        if p.title.trim().is_empty() {
            return Err(format!("param {} needs a title", p.id));
        }
        if p.kind == ParamType::Choice && p.choices.is_empty() {
            return Err(format!("param {} is a choice with no choices", p.id));
        }
        if let Some(d) = &p.default {
            typed(p, d).map_err(|e| format!("the default of {e}"))?;
        }
    }
    Ok(())
}

/// The value of every param: `given`, else the default; a missing required one or a value of the wrong type is an
/// error, and so is a value for a param the template doesn't have.
pub fn param_values(params: &[TemplateParam], given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    if let Some(k) = given.keys().find(|k| !params.iter().any(|p| &p.id == *k)) {
        return Err(format!("the template has no param {k}"));
    }
    let mut out = Map::new();
    for p in params {
        let v = match given.get(&p.id).filter(|v| !v.is_null()).or(p.default.as_ref()) {
            Some(v) => typed(p, v)?,
            None if p.required => return Err(format!("{} ({}) is required", p.title, p.id)),
            None => placeholder(p),
        };
        out.insert(p.id.clone(), v);
    }
    Ok(out)
}

/// A value of the param's type for checking a template without real values: its default, or an empty one.
fn placeholder(p: &TemplateParam) -> Value {
    if let Some(v) = p.default.as_ref().and_then(|d| typed(p, d).ok()) {
        return v;
    }
    match p.kind {
        ParamType::String | ParamType::Text | ParamType::Connection => json!(""),
        ParamType::Number | ParamType::Duration => json!(0),
        ParamType::Boolean => json!(false),
        ParamType::Choice => json!(p.choices.first().cloned().unwrap_or_default()),
        ParamType::List => json!([]),
    }
}

/// Every param at its default or a placeholder, so a template's document can be validated. A required text param
/// with no default stays a reference (`{{params.repo}}`): a value only instantiating gives, which kinds check as one.
pub fn placeholder_values(params: &[TemplateParam]) -> Map<String, Value> {
    params
        .iter()
        .map(|p| {
            let v = match p.kind {
                ParamType::String | ParamType::Text | ParamType::Connection if p.required && p.default.is_none() => {
                    json!(format!("{{{{params.{}}}}}", p.id))
                }
                _ => placeholder(p),
            };
            (p.id.clone(), v)
        })
        .collect()
}

/// The optional integration (or GitHub) a document needs: the first whose node kinds it uses (PDT-229).
pub fn plugin_of(doc: &WorkflowDoc) -> Option<(&'static str, &'static str)> {
    let kinds = || doc.nodes.iter().map(|n| n.kind.as_str());
    kinds()
        .find_map(crate::integration::owner_of_kind)
        .map(|k| (k.id, k.label))
        .or_else(|| kinds().any(|k| k.starts_with("github.")).then_some(("github", "GitHub")))
}

/// The template's document with `{{params.x}}` filled in from `values`.
pub fn fill(doc: &WorkflowDoc, values: &Map<String, Value>) -> Result<WorkflowDoc, String> {
    let v = serde_json::to_value(doc).map_err(|e| e.to_string())?;
    let filled = resolve_only(&v, &json!({ "params": values }), "params");
    serde_json::from_value(filled).map_err(|e| format!("the filled-in document doesn't read: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(id: &str, kind: ParamType) -> TemplateParam {
        TemplateParam {
            id: id.into(),
            title: id.to_uppercase(),
            description: String::new(),
            kind,
            choices: vec![],
            default: None,
            required: false,
            connection_kind: None,
        }
    }

    fn doc() -> WorkflowDoc {
        serde_json::from_value(json!({"name": "Tell {{params.who}}", "nodes": [
            {"id": "t", "kind": "ci.run_failed", "version": 1},
            {"id": "s", "kind": "session.send", "version": 1, "config": {
                "attempt": "{{trigger.attemptId}}", "text": "{{params.prefix}} {{trigger.notice.text}}",
                "quiet": "{{params.quiet}}", "n": "{{params.n}}"}}],
            "edges": [{"from": "t", "to": "s"}]}))
        .unwrap()
    }

    #[test]
    fn fills_params_and_keeps_other_references() {
        let values = json!({"who": "the agent", "prefix": "CI:", "quiet": true, "n": 2});
        let d = fill(&doc(), values.as_object().unwrap()).unwrap();
        assert_eq!(d.name, "Tell the agent");
        let c = &d.nodes[1].config;
        assert_eq!(c["text"], json!("CI: {{trigger.notice.text}}"));
        assert_eq!(c["attempt"], json!("{{trigger.attemptId}}"));
        assert_eq!((c["quiet"].clone(), c["n"].clone()), (json!(true), json!(2)), "a whole reference keeps its type");
    }

    #[test]
    fn values_come_from_given_then_defaults_and_are_typed() {
        let mut on = param("on", ParamType::Boolean);
        on.default = Some(json!(true));
        let mut stage = param("stage", ParamType::Choice);
        stage.choices = vec!["review".into(), "done".into()];
        stage.required = true;
        let n = param("n", ParamType::Number);
        let ps = vec![on, stage, n];
        let given = json!({"stage": "done", "n": "3"});
        let v = param_values(&ps, given.as_object().unwrap()).unwrap();
        assert_eq!(Value::Object(v), json!({"on": true, "stage": "done", "n": 3}));
        let err = |g: Value| param_values(&ps, g.as_object().unwrap()).unwrap_err();
        assert!(err(json!({})).contains("required"));
        assert!(err(json!({"stage": "nope"})).contains("one of review, done"));
        assert!(err(json!({"stage": "done", "on": "yes"})).contains("true or false"));
        assert!(err(json!({"stage": "done", "x": 1})).contains("no param x"));
        assert_eq!(Value::Object(placeholder_values(&ps)), json!({"on": true, "stage": "review", "n": 0}));
        let labels = [param("labels", ParamType::List)];
        let list = |g: Value| param_values(&labels, g.as_object().unwrap());
        assert_eq!(list(json!({"labels": "automerge, docs ,"})).unwrap()["labels"], json!(["automerge", "docs"]));
        assert_eq!(list(json!({"labels": ["a", " b "]})).unwrap()["labels"], json!(["a", "b"]));
        assert_eq!(list(json!({})).unwrap()["labels"], json!([]));
        assert!(list(json!({"labels": [1]})).unwrap_err().contains("a list of text"));
        let wait = [param("wait", ParamType::Duration)];
        let ms = |g: Value| param_values(&wait, g.as_object().unwrap()).map(|v| v["wait"].clone());
        assert_eq!(ms(json!({"wait": 5000})).unwrap(), json!(5000));
        assert_eq!(ms(json!({"wait": "6h"})).unwrap(), json!(6 * 3_600_000));
        assert_eq!(ms(json!({"wait": " 3D "})).unwrap(), json!(3 * 86_400_000));
        assert_eq!(ms(json!({"wait": "90m"})).unwrap(), json!(90 * 60_000));
        assert_eq!(ms(json!({"wait": "250"})).unwrap(), json!(250));
        for bad in [json!(-1), json!(1.5), json!("3 weeks"), json!("h"), json!(true)] {
            assert!(ms(json!({"wait": bad})).unwrap_err().contains("a duration"), "{bad}");
        }
        let mut repo = param("repo", ParamType::String);
        repo.required = true;
        assert_eq!(Value::Object(placeholder_values(&[repo])), json!({"repo": "{{params.repo}}"}), "left for later");
    }

    #[test]
    fn a_document_names_the_integration_it_needs() {
        let with = |kinds: &[&str]| {
            let nodes: Vec<Value> = kinds
                .iter()
                .enumerate()
                .map(|(i, k)| json!({"id": format!("n{i}"), "kind": k, "version": 1}))
                .collect();
            plugin_of(&serde_json::from_value(json!({"name": "x", "nodes": nodes})).unwrap())
        };
        assert_eq!(with(&["manual", "session.send"]), None);
        assert_eq!(
            with(&["github.issue.opened", "discord.post"]),
            Some(("discord", "Discord")),
            "the optional one first"
        );
        assert_eq!(with(&["github.issue.opened", "agent"]), Some(("github", "GitHub")));
        assert_eq!(with(&["manual", "jira.comment"]), Some(("jira", "Jira")));
    }

    #[test]
    fn checks_the_param_list() {
        assert!(check_params(&[param("ok_1", ParamType::String)]).is_ok());
        assert!(check_params(&[param("1x", ParamType::String)]).unwrap_err().contains("letters"));
        assert!(
            check_params(&[param("a", ParamType::String), param("a", ParamType::Text)]).unwrap_err().contains("two")
        );
        assert!(check_params(&[param("c", ParamType::Choice)]).unwrap_err().contains("no choices"));
        let mut bad = param("b", ParamType::Boolean);
        bad.default = Some(json!("x"));
        assert!(check_params(&[bad]).unwrap_err().contains("default"));
    }
}
