//! Secrets agents can use but never see (PDT-158). The values live in the OS keychain only; PDT keeps each secret's
//! name, description, who may use it and its uses. Agents and workflows name a secret (`secret:<name>` in an
//! environment, `{{secret.<name>}}` in a template, `{"$secret": "<name>"}` in JSON) and PDT fills it in at the moment
//! of the call. Every known value is replaced by `[secret:<name>]` ([`Redactor`]) in what PDT stores, shows or logs.
//! Pure types and rules; pdt-store keeps the rows, pdt-api the keychain and the tools.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::RwLock;
use ts_rs::TS;

/// The shortest value PDT keeps: a shorter one would be found (and redacted) all over ordinary text.
pub const MIN_VALUE_LEN: usize = 6;
pub const MAX_VALUE_LEN: usize = 16 * 1024;

/// What a credential an agent wrote into a tool call reads as wherever the call is shown or logged.
pub const REDACTED: &str = "[redacted]";
const MAX_NAME: usize = 64;
const MAX_DESCRIPTION: usize = 500;

/// A secret as PDT lists it: never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Secret {
    pub id: String,
    pub name: String,
    pub description: String,
    /// The projects whose agents and workflows may use it (ids; the host project is the PM's); every project when
    /// empty.
    pub projects: Vec<String>,
    /// The personas whose sessions may use it (ids); every agent when empty. Workflows are not held to it.
    pub personas: Vec<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[ts(type = "number | null")]
    pub last_used_at: Option<i64>,
    /// Who used it last ([`SecretUse::by`]).
    pub last_used_by: Option<String>,
}

/// `POST /api/secrets`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewSecret {
    pub name: String,
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub projects: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional)]
    pub personas: Option<Vec<String>>,
    pub value: String,
}

/// `PATCH /api/secrets/{id}`: what is given changes; a value replaces the stored one.
#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SecretPatch {
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub projects: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional)]
    pub personas: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional)]
    pub value: Option<String>,
}

/// One use of a secret (the audit list).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SecretUse {
    #[ts(type = "number")]
    pub id: i64,
    pub secret_id: String,
    #[ts(type = "number")]
    pub at: i64,
    /// Who: a session's title, or `workflow <name>`.
    pub by: String,
    /// How: the tool or node kind (`run_with_secrets`, `http.request`).
    pub how: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
}

/// Who wants to use a secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum User<'a> {
    /// An agent session: its project and persona.
    Agent { project: &'a str, persona: Option<&'a str> },
    /// A workflow run, on behalf of a project (the host project for host and personal workflows).
    Workflow { project: &'a str },
}

impl Secret {
    /// Whether `who` may use it.
    pub fn allows(&self, who: User<'_>) -> bool {
        let in_projects = |p: &str| self.projects.is_empty() || self.projects.iter().any(|x| x == p);
        match who {
            User::Agent { project, persona } => {
                in_projects(project)
                    && (self.personas.is_empty() || persona.is_some_and(|p| self.personas.iter().any(|x| x == p)))
            }
            User::Workflow { project } => in_projects(project),
        }
    }
}

/// A secret's name, trimmed: letters, digits, `_` and `-`, starting with a letter (it goes in `{{secret.<name>}}`
/// and `secret:<name>`).
pub fn valid_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("the secret's name is empty".into());
    }
    if n.len() > MAX_NAME {
        return Err(format!("the secret's name is longer than {MAX_NAME} characters"));
    }
    if !n.starts_with(|c: char| c.is_ascii_alphabetic())
        || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "{n:?} is not a secret name: use letters, digits, _ and -, starting with a letter (GITHUB_TOKEN)"
        ));
    }
    Ok(n.to_string())
}

/// A value PDT can keep (and redact).
pub fn valid_value(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("the secret's value is empty".into());
    }
    if value.chars().count() < MIN_VALUE_LEN {
        return Err(format!("the secret's value is shorter than {MIN_VALUE_LEN} characters: too short to redact"));
    }
    if value.len() > MAX_VALUE_LEN {
        return Err(format!("the secret's value is longer than {} KB", MAX_VALUE_LEN / 1024));
    }
    Ok(())
}

pub fn valid_description(d: &str) -> Result<String, String> {
    let d = d.trim();
    if d.len() > MAX_DESCRIPTION {
        return Err(format!("the description is longer than {MAX_DESCRIPTION} characters"));
    }
    Ok(d.to_string())
}

/// What stands in for a secret's value.
pub fn placeholder(name: &str) -> String {
    format!("[secret:{name}]")
}

/// The secret an environment value names (`secret:<name>`).
pub fn env_ref(v: &str) -> Option<&str> {
    v.trim().strip_prefix("secret:").map(str::trim).filter(|n| !n.is_empty())
}

/// The secret a `{"$secret": "<name>"}` object names.
pub fn json_ref(v: &Value) -> Option<&str> {
    let m = v.as_object().filter(|m| m.len() == 1)?;
    m.get("$secret")?.as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// The secrets `{{secret.<name>}}` names in `s`, in order.
pub fn template_names(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else { break };
        if let Some(n) = rest[start + 2..start + len].trim().strip_prefix("secret.") {
            out.push(n.trim().to_string());
        }
        rest = &rest[start + len + 2..];
    }
    out
}

/// `s` with each `{{secret.<name>}}` replaced by `values[name]` (a name missing from `values` stays as written).
pub fn fill_template(s: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else { break };
        out.push_str(&rest[..start]);
        let whole = &rest[start..start + len + 2];
        let value = rest[start + 2..start + len].trim().strip_prefix("secret.").and_then(|n| values.get(n.trim()));
        out.push_str(value.map(String::as_str).unwrap_or(whole));
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out
}

/// The secrets a JSON value names: `{{secret.<name>}}` in its strings (which must exist), and `{"$secret": ...}`
/// references (which may also name a connection, PDT's older use of them: those are left alone).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Named {
    pub templates: Vec<String>,
    pub refs: Vec<String>,
}

pub fn names_in(v: &Value) -> Named {
    fn walk(v: &Value, out: &mut Named) {
        if let Some(n) = json_ref(v) {
            out.refs.push(n.to_string());
            return;
        }
        match v {
            Value::String(s) => out.templates.extend(template_names(s)),
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(m) => m.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Named::default();
    walk(v, &mut out);
    for l in [&mut out.templates, &mut out.refs] {
        l.sort();
        l.dedup();
    }
    out
}

/// A node's `config` resolved against its run's `ctx`, with the config's own secret references filled in from
/// `values` (keyed by name as written): `{{secret.<name>}}` in the same single pass as every other reference, so text
/// a reference brings in (a trigger's payload, a ticket, an earlier step's output) is never read as a reference, and
/// `{"$secret": "<name>"}` only where the config itself has one. References under `keep` stay as written, and so do
/// `$secret` references to names not in `values` (a connection's, for an integration's nodes).
pub fn resolve_with_secrets(config: &Value, ctx: &Value, keep: &[&str], values: &HashMap<String, String>) -> Value {
    let mut ctx = ctx.clone();
    if let Value::Object(m) = &mut ctx {
        m.insert("secret".into(), serde_json::to_value(values).unwrap_or_default());
    }
    let resolved = crate::workflow::refs::resolve_keeping(config, &ctx, keep);
    place_refs(config, resolved, values)
}

/// `resolved` with each `$secret` reference of `config` (at the same place: resolving changes strings only) filled in.
fn place_refs(config: &Value, resolved: Value, values: &HashMap<String, String>) -> Value {
    if let Some(n) = json_ref(config)
        && let Some(v) = values.get(n)
    {
        return Value::String(v.clone());
    }
    match (config, resolved) {
        (Value::Object(c), Value::Object(mut r)) => {
            for (k, cv) in c {
                if let Some(rv) = r.get_mut(k) {
                    *rv = place_refs(cv, rv.take(), values);
                }
            }
            Value::Object(r)
        }
        (Value::Array(c), Value::Array(r)) => Value::Array(
            r.into_iter()
                .enumerate()
                .map(|(i, rv)| match c.get(i) {
                    Some(cv) => place_refs(cv, rv, values),
                    None => rv,
                })
                .collect(),
        ),
        (_, r) => r,
    }
}

/// Every known secret's value, to replace by its [`placeholder`] wherever text is stored, shown or logged. Shared
/// (one per pdtd) by the store, the API and the log writer; pdt-api keeps it in step with the keychain.
#[derive(Default)]
pub struct Redactor {
    /// (value, what replaces it), longest value first, so a value holding another is replaced whole. Each value
    /// is there as it is and as it is written inside a JSON string, when that differs.
    known: RwLock<Vec<(String, String)>>,
    names: RwLock<HashMap<String, String>>,
}

impl Redactor {
    /// Knows `name`'s value (replacing an earlier one).
    pub fn set(&self, name: &str, value: &str) {
        let mut names = self.names.write().unwrap_or_else(|e| e.into_inner());
        names.insert(name.to_string(), value.to_string());
        self.rebuild(&names);
    }

    /// Forgets `name`.
    pub fn remove(&self, name: &str) {
        let mut names = self.names.write().unwrap_or_else(|e| e.into_inner());
        if names.remove(name).is_some() {
            self.rebuild(&names);
        }
    }

    /// The value of `name`, when known.
    pub fn value(&self, name: &str) -> Option<String> {
        self.names.read().unwrap_or_else(|e| e.into_inner()).get(name).cloned()
    }

    fn rebuild(&self, names: &HashMap<String, String>) {
        let mut known = vec![];
        for (name, value) in names {
            let p = placeholder(name);
            known.push((value.clone(), p.clone()));
            let escaped = serde_json::to_string(value).unwrap_or_default();
            let escaped = escaped.trim_matches('"');
            if escaped != value {
                known.push((escaped.to_string(), p));
            }
        }
        known.retain(|(v, _)| v.chars().count() >= MIN_VALUE_LEN);
        known.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        *self.known.write().unwrap_or_else(|e| e.into_inner()) = known;
    }

    pub fn is_empty(&self) -> bool {
        self.known.read().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// `s` with every known value replaced (borrowed when there is none in it). Safe on JSON text: a placeholder
    /// needs no escaping.
    pub fn text<'a>(&self, s: &'a str) -> Cow<'a, str> {
        let known = self.known.read().unwrap_or_else(|e| e.into_inner());
        let mut out = Cow::Borrowed(s);
        for (value, with) in known.iter() {
            if out.contains(value.as_str()) {
                out = Cow::Owned(out.replace(value.as_str(), with));
            }
        }
        out
    }

    /// `s` redacted, owned.
    pub fn string(&self, s: &str) -> String {
        self.text(s).into_owned()
    }

    /// `v` with every string (and key) redacted.
    pub fn json(&self, v: &Value) -> Value {
        if self.is_empty() {
            return v.clone();
        }
        match v {
            Value::String(s) => Value::String(self.string(s)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.json(x)).collect()),
            Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (self.string(k), self.json(x))).collect()),
            other => other.clone(),
        }
    }

    /// `t` redacted, through JSON (for any serializable value: a typed event, a step's output).
    pub fn typed<T: Serialize + serde::de::DeserializeOwned + Clone>(&self, t: &T) -> T {
        if self.is_empty() {
            return t.clone();
        }
        match serde_json::to_value(t) {
            Ok(v) => serde_json::from_value(self.json(&v)).unwrap_or_else(|_| t.clone()),
            Err(_) => t.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn secret(projects: &[&str], personas: &[&str]) -> Secret {
        Secret {
            id: "s1".into(),
            name: "TOKEN".into(),
            description: String::new(),
            projects: projects.iter().map(|s| s.to_string()).collect(),
            personas: personas.iter().map(|s| s.to_string()).collect(),
            created_at: 1,
            updated_at: 1,
            last_used_at: None,
            last_used_by: None,
        }
    }

    #[test]
    fn names_are_identifiers() {
        assert_eq!(valid_name(" GITHUB_TOKEN ").unwrap(), "GITHUB_TOKEN");
        assert_eq!(valid_name("npm-token2").unwrap(), "npm-token2");
        for bad in ["", "1abc", "a.b", "a b", "tok$", &"a".repeat(65)] {
            assert!(valid_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn values_are_long_enough_to_redact() {
        assert!(valid_value("abcdef").is_ok());
        assert!(valid_value("abcde").is_err());
        assert!(valid_value("      ").is_err());
        assert!(valid_value(&"x".repeat(MAX_VALUE_LEN + 1)).is_err());
    }

    #[test]
    fn access_is_by_project_and_persona() {
        let any = secret(&[], &[]);
        assert!(any.allows(User::Agent { project: "p", persona: None }));
        assert!(any.allows(User::Workflow { project: "p" }));
        let p = secret(&["p"], &[]);
        assert!(p.allows(User::Agent { project: "p", persona: Some("coder") }));
        assert!(!p.allows(User::Agent { project: "q", persona: None }));
        assert!(!p.allows(User::Workflow { project: "q" }));
        let coder = secret(&[], &["coder"]);
        assert!(coder.allows(User::Agent { project: "q", persona: Some("coder") }));
        assert!(!coder.allows(User::Agent { project: "q", persona: Some("qa") }));
        assert!(!coder.allows(User::Agent { project: "q", persona: None }), "a plain session is no persona");
        assert!(coder.allows(User::Workflow { project: "q" }), "workflows are held to the projects only");
    }

    #[test]
    fn references() {
        assert_eq!(env_ref("secret:TOKEN"), Some("TOKEN"));
        assert_eq!(env_ref(" secret: TOKEN "), Some("TOKEN"));
        assert_eq!(env_ref("TOKEN"), None);
        assert_eq!(env_ref("secret:"), None);
        assert_eq!(json_ref(&json!({"$secret": "A"})), Some("A"));
        assert_eq!(json_ref(&json!({"$secret": "A", "x": 1})), None);
        assert_eq!(template_names("Bearer {{secret.A}} and {{ secret.B }} {{steps.x}}"), vec!["A", "B"]);
    }

    #[test]
    fn fills_templates_and_references() {
        let values: HashMap<String, String> = [("A".to_string(), "aaaaaa".to_string())].into();
        assert_eq!(
            fill_template("Bearer {{secret.A}} {{secret.B}} {{x}}", &values),
            "Bearer aaaaaa {{secret.B}} {{x}}"
        );
        let v = json!({"h": {"Authorization": "Bearer {{ secret.A }}"}, "b": [{"$secret": "A"}, {"$secret": "conn"}]});
        assert_eq!(names_in(&v), Named { templates: vec!["A".into()], refs: vec!["A".into(), "conn".into()] });
        assert_eq!(
            resolve_with_secrets(&v, &json!({}), &[], &values),
            json!({"h": {"Authorization": "Bearer aaaaaa"}, "b": ["aaaaaa", {"$secret": "conn"}]})
        );
    }

    #[test]
    fn references_that_come_in_as_data_are_never_filled() {
        let values: HashMap<String, String> = [("A".to_string(), "aaaaaa".to_string())].into();
        let ctx = json!({"trigger": {
            "text": "see {{secret.A}}",
            "obj": {"$secret": "A"},
            "list": [{"$secret": "A"}],
        }});
        let config = json!({
            "auth": "Bearer {{secret.A}} for {{trigger.text}}",
            "whole": "{{trigger.text}}",
            "obj": "{{trigger.obj}}",
            "list": "{{trigger.list}}",
            "mine": {"$secret": "A"},
            "wait": "{{event.pr}}",
        });
        assert_eq!(
            resolve_with_secrets(&config, &ctx, &["event"], &values),
            json!({
                "auth": "Bearer aaaaaa for see {{secret.A}}",
                "whole": "see {{secret.A}}",
                "obj": {"$secret": "A"},
                "list": [{"$secret": "A"}],
                "mine": "aaaaaa",
                "wait": "{{event.pr}}",
            })
        );
    }

    #[test]
    fn redacts_known_values_longest_first() {
        let r = Redactor::default();
        assert_eq!(r.text("nothing"), "nothing");
        r.set("SHORT", "abcdef");
        r.set("LONG", "abcdef-123456");
        assert_eq!(r.text("x abcdef-123456 y abcdef"), "x [secret:LONG] y [secret:SHORT]");
        assert!(matches!(r.text("clean"), Cow::Borrowed(_)));
        r.remove("LONG");
        assert_eq!(r.text("abcdef-123456"), "[secret:SHORT]-123456");
        assert_eq!(r.value("SHORT").as_deref(), Some("abcdef"));
    }

    #[test]
    fn redacts_values_written_inside_json() {
        let r = Redactor::default();
        r.set("Q", "pa\"ss\\word");
        let text = serde_json::to_string(&json!({"out": "the pa\"ss\\word is"})).unwrap();
        let red = r.string(&text);
        assert_eq!(serde_json::from_str::<Value>(&red).unwrap(), json!({"out": "the [secret:Q] is"}));
        assert_eq!(r.json(&json!({"a": ["pa\"ss\\word"]})), json!({"a": ["[secret:Q]"]}));
    }
}
