//! A Run now trigger's declared inputs (PDT-301): the fields Run now's form asks for, and the check every manual start
//! (the API, the PM's `workflow_run`, a sub-workflow call) puts the input through: defaults filled in, a field hidden by
//! its `showIf` left out, unknown or invalid values refused with a message that lists the fields.

use super::refs::resolve_str;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InputType {
    /// One line of text.
    #[default]
    Text,
    Number,
    Boolean,
    /// One of `choices`.
    Choice,
}

/// One of a choice field's values, with what the form shows for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct InputChoice {
    pub value: String,
    /// Empty: the value.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[ts(as = "Option<String>", optional)]
    pub label: String,
}

/// Shown (and read) only while another field is shown and has this value (`""` matches a field left empty).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ShowIf {
    /// An earlier field's id.
    pub field: String,
    #[ts(type = "unknown")]
    pub equals: Value,
}

/// A field Run now asks for: the run's input has it as `{{trigger.<id>}}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct InputField {
    pub id: String,
    /// Empty: the id.
    pub label: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub kind: InputType,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<InputChoice>>", optional)]
    pub choices: Vec<InputChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub default: Option<Value>,
    pub required: bool,
    /// A short hint under the field.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[ts(as = "Option<String>", optional)]
    pub help: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub show_if: Option<ShowIf>,
}

/// A confirmation Run now asks before it starts: `text` may name the inputs (`{{trigger.bump}}`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct RunConfirm {
    pub text: String,
    /// Only for these inputs; none: every run.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub show_if: Option<ShowIf>,
}

/// Input keys PDT itself adds to a manual run (the PM's project, a ticket's run), allowed beside the declared fields.
pub const PASSED_THROUGH: &[&str] = &["projectId", "ticketId"];

impl InputField {
    pub fn label(&self) -> &str {
        if self.label.trim().is_empty() { &self.id } else { &self.label }
    }

    /// What its value must be: `patch, minor or major`.
    fn what(&self) -> String {
        match self.kind {
            InputType::Text => "text".to_string(),
            InputType::Number => "a number".into(),
            InputType::Boolean => "true or false".into(),
            InputType::Choice => one_of(&self.choices.iter().map(|c| c.value.as_str()).collect::<Vec<_>>()),
        }
    }

    /// The field in a message: `bump (patch, minor or major)`.
    fn describe(&self) -> String {
        let what = self.what();
        let req = if self.required { ", required" } else { "" };
        match &self.show_if {
            Some(s) if s.equals == json!("") => format!("{} ({what}{req}, when {} is empty)", self.id, s.field),
            Some(s) => format!("{} ({what}{req}, when {} is {})", self.id, s.field, text_of(&s.equals)),
            None => format!("{} ({what}{req})", self.id),
        }
    }

    /// `v` as this field's type, or None.
    fn typed(&self, v: &Value) -> Option<Value> {
        match (self.kind, v) {
            (InputType::Text, Value::String(_)) => Some(v.clone()),
            (InputType::Text, Value::Number(_) | Value::Bool(_)) => Some(json!(v.to_string())),
            (InputType::Number, Value::Number(_)) => Some(v.clone()),
            (InputType::Number, Value::String(s)) => {
                let f = s.trim().parse::<f64>().ok()?;
                if f.fract() == 0.0 && f.abs() < 9e15 { Some(json!(f as i64)) } else { Some(json!(f)) }
            }
            (InputType::Boolean, Value::Bool(_)) => Some(v.clone()),
            (InputType::Boolean, Value::String(s)) => match s.trim() {
                "true" => Some(json!(true)),
                "false" => Some(json!(false)),
                _ => None,
            },
            (InputType::Choice, Value::String(s)) => self.choices.iter().any(|c| &c.value == s).then(|| v.clone()),
            _ => None,
        }
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn one_of(items: &[&str]) -> String {
    match items {
        [] => "nothing".into(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// Whether a `showIf` holds for the values read so far (a field left empty is `""`).
fn holds(show_if: Option<&ShowIf>, values: &Map<String, Value>) -> bool {
    show_if.is_none_or(|s| values.get(&s.field).unwrap_or(&json!("")) == &s.equals)
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether declared fields make sense: ids usable in `{{trigger.<id>}}` and unique, choices for a choice, defaults of
/// the field's type, a `showIf` naming an earlier field (and the confirmation's, any field).
pub fn check_fields(fields: &[InputField], confirm: Option<&RunConfirm>) -> Result<(), String> {
    let mut seen: Vec<&str> = vec![];
    let names = |s: &ShowIf, seen: &[&str], whose: &str| {
        if seen.contains(&s.field.as_str()) {
            Ok(())
        } else {
            Err(format!("{whose}'s showIf names {}, which is no earlier field", s.field))
        }
    };
    for f in fields {
        if !valid_id(&f.id) {
            return Err(format!("an input's id must be letters, digits and _ (not {:?})", f.id));
        }
        if PASSED_THROUGH.contains(&f.id.as_str()) {
            return Err(format!("{} is PDT's own input: call the field something else", f.id));
        }
        if seen.contains(&f.id.as_str()) {
            return Err(format!("two inputs are called {}", f.id));
        }
        if f.kind == InputType::Choice {
            if f.choices.is_empty() {
                return Err(format!("input {} is a choice with no choices", f.id));
            }
            if f.choices.iter().any(|c| c.value.is_empty()) {
                return Err(format!("input {} has a choice with no value", f.id));
            }
        }
        if let Some(d) = &f.default
            && f.typed(d).is_none()
        {
            return Err(format!("the default of input {} isn't {}", f.id, f.what()));
        }
        if let Some(s) = &f.show_if {
            names(s, &seen, &f.id)?;
        }
        seen.push(&f.id);
    }
    if let Some(c) = confirm {
        if c.text.trim().is_empty() {
            return Err("the confirmation has no text".into());
        }
        if let Some(s) = &c.show_if {
            names(s, &seen, "the confirmation")?;
        }
    }
    Ok(())
}

/// The fields, for a message: `the inputs are type (stable or prerelease), bump (…)`.
fn listing(fields: &[InputField]) -> String {
    if fields.is_empty() {
        return "it declares none".into();
    }
    format!("the inputs are {}", fields.iter().map(InputField::describe).collect::<Vec<_>>().join(", "))
}

/// A manual run's input checked against the declared fields: each shown field's value (given, else its default), a
/// hidden field's left out; the keys in [`PASSED_THROUGH`] kept. Without declared fields the input is taken as it is.
pub fn apply(fields: &[InputField], input: &Value) -> Result<Value, String> {
    let given = match input {
        Value::Null => Map::new(),
        Value::Object(m) => m.clone(),
        _ if fields.is_empty() => return Ok(input.clone()),
        _ => return Err(format!("the input must be an object: {}", listing(fields))),
    };
    if fields.is_empty() {
        return Ok(Value::Object(given));
    }
    if let Some(k) = given.keys().find(|k| !PASSED_THROUGH.contains(&k.as_str()) && !fields.iter().any(|f| &f.id == *k))
    {
        return Err(format!("there is no input {k}: {}", listing(fields)));
    }
    let mut out = Map::new();
    let mut shown: Vec<&str> = vec![];
    for f in fields {
        let visible = f.show_if.as_ref().is_none_or(|s| shown.contains(&s.field.as_str()));
        if !visible || !holds(f.show_if.as_ref(), &out) {
            continue;
        }
        shown.push(&f.id);
        let value = match given.get(&f.id).filter(|v| !v.is_null() && v.as_str() != Some("")) {
            Some(v) => Some(f.typed(v).ok_or_else(|| format!("{} must be {}: {}", f.id, f.what(), listing(fields)))?),
            None => f.default.as_ref().and_then(|d| f.typed(d)),
        };
        match value {
            Some(v) => {
                out.insert(f.id.clone(), v);
            }
            None if f.required => return Err(format!("{} ({}) is required: {}", f.label(), f.id, listing(fields))),
            None => {}
        }
    }
    for k in PASSED_THROUGH {
        if let Some(v) = given.get(*k) {
            out.insert((*k).to_string(), v.clone());
        }
    }
    Ok(Value::Object(out))
}

/// The confirmation for these (checked) inputs, its references filled in; None when it doesn't apply.
pub fn confirm_text(confirm: Option<&RunConfirm>, input: &Value) -> Option<String> {
    let c = confirm?;
    let values = input.as_object().cloned().unwrap_or_default();
    if !holds(c.show_if.as_ref(), &values) {
        return None;
    }
    resolve_str(&c.text, &json!({"trigger": input})).as_str().map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Vec<InputField> {
        serde_json::from_value(json!([
            {"id": "type", "label": "Type", "type": "choice", "default": "nightly", "required": true,
             "choices": [{"value": "stable", "label": "Stable"}, {"value": "nightly", "label": "Nightly"}]},
            {"id": "version", "type": "text", "showIf": {"field": "type", "equals": "stable"}},
            {"id": "bump", "type": "choice", "default": "patch", "showIf": {"field": "version", "equals": ""},
             "choices": [{"value": "patch"}, {"value": "minor"}, {"value": "major"}]},
            {"id": "dry", "type": "boolean", "default": false},
            {"id": "n", "type": "number"},
        ]))
        .unwrap()
    }

    #[test]
    fn defaults_fill_in_and_hidden_fields_are_left_out() {
        let f = fields();
        assert_eq!(apply(&f, &Value::Null).unwrap(), json!({"type": "nightly", "dry": false}));
        assert_eq!(
            apply(&f, &json!({"bump": "major", "version": "9.9.9"})).unwrap(),
            json!({"type": "nightly", "dry": false}),
            "bump and version only for a stable release"
        );
        assert_eq!(
            apply(&f, &json!({"type": "stable", "version": "", "n": "3", "dry": "true", "projectId": "p"})).unwrap(),
            json!({"type": "stable", "bump": "patch", "dry": true, "n": 3, "projectId": "p"})
        );
        assert_eq!(
            apply(&f, &json!({"type": "stable", "version": "1.2.0", "bump": "minor"})).unwrap(),
            json!({"type": "stable", "version": "1.2.0", "dry": false}),
            "an exact version instead of a bump"
        );
    }

    #[test]
    fn bad_input_is_refused_with_the_fields_listed() {
        let f = fields();
        let err = |v: Value| apply(&f, &v).unwrap_err();
        let e = err(json!({"channel": "nightly"}));
        assert!(e.contains("there is no input channel"), "{e}");
        assert!(e.contains("type (stable or nightly, required)"), "{e}");
        assert!(e.contains("bump (patch, minor or major, when version is empty)"), "{e}");
        assert!(err(json!({"type": "beta"})).contains("type must be stable or nightly"));
        assert!(err(json!({"type": "stable", "bump": "huge"})).contains("bump must be"));
        assert!(err(json!({"n": "lots"})).contains("n must be a number"));
        assert!(err(json!("stable")).contains("must be an object"));
        let mut req = fields();
        req[0].default = None;
        assert!(apply(&req, &json!({})).unwrap_err().contains("Type (type) is required"));
    }

    #[test]
    fn without_fields_the_input_is_taken_as_it_is() {
        assert_eq!(apply(&[], &json!({"anything": 1})).unwrap(), json!({"anything": 1}));
        assert_eq!(apply(&[], &Value::Null).unwrap(), json!({}));
    }

    #[test]
    fn checks_the_declared_fields() {
        assert!(check_fields(&fields(), None).is_ok());
        let with = |patch: Value| {
            let mut v = serde_json::to_value(fields()).unwrap();
            v.as_array_mut().unwrap().push(patch);
            check_fields(&serde_json::from_value::<Vec<InputField>>(v).unwrap(), None)
        };
        assert!(with(json!({"id": "1x"})).unwrap_err().contains("letters"));
        assert!(with(json!({"id": "bump"})).unwrap_err().contains("two inputs"));
        assert!(with(json!({"id": "projectId"})).unwrap_err().contains("PDT's own"));
        assert!(with(json!({"id": "c", "type": "choice"})).unwrap_err().contains("no choices"));
        assert!(with(json!({"id": "c", "type": "number", "default": "x"})).unwrap_err().contains("default"));
        assert!(
            with(json!({"id": "c", "type": "choice", "choices": [{"value": "a"}], "default": "b"}))
                .unwrap_err()
                .contains("default")
        );
        assert!(
            with(json!({"id": "c", "showIf": {"field": "later", "equals": 1}})).unwrap_err().contains("no earlier")
        );
        let confirm = |s: Value| serde_json::from_value::<RunConfirm>(s).unwrap();
        assert!(check_fields(&fields(), Some(&confirm(json!({"text": " "})))).is_err());
        assert!(
            check_fields(&fields(), Some(&confirm(json!({"text": "ok", "showIf": {"field": "x", "equals": 1}}))))
                .is_err()
        );
    }

    #[test]
    fn the_confirmation_names_the_inputs_and_applies_by_its_show_if() {
        let c: RunConfirm = serde_json::from_value(json!({
            "text": "This publishes a stable release ({{trigger.bump}}{{trigger.version}}). Continue?",
            "showIf": {"field": "type", "equals": "stable"}}))
        .unwrap();
        let input = apply(&fields(), &json!({"type": "stable", "bump": "minor"})).unwrap();
        assert_eq!(
            confirm_text(Some(&c), &input).as_deref(),
            Some("This publishes a stable release (minor). Continue?")
        );
        let exact = apply(&fields(), &json!({"type": "stable", "version": "2.0.0"})).unwrap();
        assert_eq!(
            confirm_text(Some(&c), &exact).as_deref(),
            Some("This publishes a stable release (2.0.0). Continue?")
        );
        assert_eq!(confirm_text(Some(&c), &apply(&fields(), &json!({})).unwrap()), None, "a nightly asks nothing");
        assert_eq!(confirm_text(None, &input), None);
    }
}
