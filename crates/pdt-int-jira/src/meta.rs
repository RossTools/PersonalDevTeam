//! What differs between Jira sites (PDT-258), pure: Cloud or Server / Data Center, the connection's field mappings
//! (instance-specific custom field ids, the Epic Name, Epic Link and Sprint fields, backlog sections), Jira's field
//! metadata (from create, edit and transition screens) and how a plain value becomes what a field takes, the fields a
//! transition still needs, sprints as either deployment writes them, and where an issue sits on a board.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ts_rs::TS;

/// Which Jira a connection is: Cloud (email and API token, REST v3) or Server / Data Center (a personal access token
/// sent as a bearer token, REST v2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Deployment {
    #[default]
    Cloud,
    Server,
}

impl Deployment {
    /// From a connection's config (`deployment`); Cloud unless it says `server` (connections from before PDT-258).
    pub fn of(config: &Value) -> Self {
        match config.get("deployment").and_then(Value::as_str).map(str::trim) {
            Some("server") | Some("datacenter") | Some("data_center") => Deployment::Server,
            _ => Deployment::Cloud,
        }
    }

    /// The REST API's base path.
    pub fn api(self) -> &'static str {
        match self {
            Deployment::Cloud => "/rest/api/3",
            Deployment::Server => "/rest/api/2",
        }
    }

    /// Rich text as this deployment takes it: ADF on Cloud, plain text (wiki markup) on Server.
    pub fn rich_text(self, s: &str) -> Value {
        match self {
            Deployment::Cloud => crate::core::text_to_adf(s),
            Deployment::Server => json!(s),
        }
    }

    /// A user field's value: `{"accountId"}` on Cloud, `{"name"}` (the username) on Server.
    pub fn user(self, id: &str) -> Value {
        match self {
            Deployment::Cloud => json!({"accountId": id}),
            Deployment::Server => json!({"name": id}),
        }
    }

    /// The id a user is known by in a REST answer: Cloud's `accountId`, Server's `name`.
    pub fn user_id(self, user: &Value) -> Option<String> {
        let k = match self {
            Deployment::Cloud => "accountId",
            Deployment::Server => "name",
        };
        user.get(k).and_then(Value::as_str).map(String::from)
    }
}

/// A connection's mappings of this site's fields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mappings {
    /// Names a call may use for a field (`storyPoints` → `customfield_10016`), from `config.fields`.
    pub fields: Vec<(String, String)>,
    /// The Epic Name field (`customfield_10011`); found by its type when not set.
    pub epic_name: Option<String>,
    /// The Epic Link field (`customfield_10014`); found by its type when not set, and on Cloud without one an epic is
    /// the issue's `parent`.
    pub epic_link: Option<String>,
    /// The Sprint field (`customfield_10020`); found by its type when not set.
    pub sprint: Option<String>,
    /// Sprints that are sections of the backlog rather than sprints (never started, named like `Backlog: Ready`):
    /// names, case-insensitive, `*` at the end for a prefix.
    pub backlog_sections: Vec<String>,
}

fn nonblank(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

impl Mappings {
    /// From a connection's config: `fields` (`{name: id}`, or `name=id` lines), `epicNameField`, `epicLinkField`,
    /// `sprintField`, `backlogSections` (a list, or one per line).
    pub fn of(config: &Value) -> Self {
        let fields = match config.get("fields") {
            Some(Value::Object(m)) => m
                .iter()
                .filter_map(|(k, v)| Some((k.trim().to_string(), v.as_str()?.trim().to_string())))
                .filter(|(k, v)| !k.is_empty() && !v.is_empty())
                .collect(),
            Some(Value::String(s)) => s
                .lines()
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .filter(|(k, v)| !k.is_empty() && !v.is_empty())
                .collect(),
            _ => vec![],
        };
        let backlog_sections = match config.get("backlogSections") {
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
            Some(Value::String(s)) => s.lines().map(String::from).collect(),
            _ => vec![],
        };
        Mappings {
            fields,
            epic_name: nonblank(config, "epicNameField"),
            epic_link: nonblank(config, "epicLinkField"),
            sprint: nonblank(config, "sprintField"),
            backlog_sections: backlog_sections
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }

    /// The field id a mapped name stands for (case-insensitive).
    pub fn field(&self, name: &str) -> Option<&str> {
        let n = name.trim();
        self.fields.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.as_str())
    }

    /// Whether a sprint with this name is a backlog section.
    pub fn is_backlog_section(&self, sprint: &str) -> bool {
        let s = sprint.trim().to_lowercase();
        self.backlog_sections.iter().any(|p| {
            let p = p.to_lowercase();
            match p.strip_suffix('*') {
                Some(prefix) => s.starts_with(prefix.trim_end()),
                None => s == p,
            }
        })
    }
}

/// One field as a create, edit or transition screen describes it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "JiraFieldMeta")]
pub struct FieldMeta {
    /// `summary`, `customfield_10016`.
    pub id: String,
    pub name: String,
    pub required: bool,
    /// Jira fills it in when left out.
    pub has_default: bool,
    /// `string`, `number`, `option`, `array`, `user`, `priority`, … (its `schema.type`).
    pub r#type: String,
    /// An array's items' type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub items: Option<String>,
    /// A custom field's type (`com.atlassian.jira.plugin.system.customfieldtypes:select`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub custom: Option<String>,
    /// The values it allows (an option's, a priority's, …): each one's id and name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<Allowed>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "JiraAllowed")]
pub struct Allowed {
    pub id: String,
    pub name: String,
}

/// Fields from Jira's metadata: an object keyed by field id (editmeta, a transition's `fields`) or a list with
/// `fieldId` (createmeta's `fields` on Cloud, `values` on Server).
pub fn parse_fields(v: &Value) -> Vec<FieldMeta> {
    let one = |id: &str, f: &Value| {
        let s = &f["schema"];
        FieldMeta {
            id: id.to_string(),
            name: f["name"].as_str().unwrap_or(id).to_string(),
            required: f["required"].as_bool().unwrap_or(false),
            has_default: f["hasDefaultValue"].as_bool().unwrap_or(false),
            r#type: s["type"].as_str().unwrap_or("any").to_string(),
            items: s["items"].as_str().map(String::from),
            custom: s["custom"].as_str().map(String::from),
            allowed: f["allowedValues"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| Allowed {
                    id: a["id"].as_str().map(String::from).unwrap_or_else(|| a["id"].to_string()),
                    name: ["value", "name", "key"].iter().find_map(|k| a[*k].as_str()).unwrap_or_default().to_string(),
                })
                .collect(),
        }
    };
    match v {
        Value::Object(m) => m.iter().map(|(id, f)| one(f["fieldId"].as_str().unwrap_or(id), f)).collect(),
        Value::Array(a) => {
            a.iter().filter_map(|f| Some(one(f["fieldId"].as_str().or(f["key"].as_str())?, f))).collect()
        }
        _ => vec![],
    }
}

/// The field a call names, among `meta`: by id, by a mapped name, or by its name on the screen (case-insensitive).
pub fn resolve_field<'a>(name: &str, mappings: &Mappings, meta: &'a [FieldMeta]) -> Option<&'a FieldMeta> {
    let n = name.trim();
    let id = mappings.field(n).unwrap_or(n);
    meta.iter()
        .find(|f| f.id == id)
        .or_else(|| meta.iter().find(|f| f.id.eq_ignore_ascii_case(id)))
        .or_else(|| meta.iter().find(|f| f.name.eq_ignore_ascii_case(n)))
}

/// The id a call's field name stands for without metadata: a mapped name's id, else the name as given.
pub fn field_id(name: &str, mappings: &Mappings) -> String {
    mappings.field(name).unwrap_or(name.trim()).to_string()
}

/// Whether a value is already in Jira's form (an object, or a list of them): sent as it is.
fn shaped(v: &Value) -> bool {
    match v {
        Value::Object(_) => true,
        Value::Array(a) => !a.is_empty() && a.iter().all(Value::is_object),
        _ => false,
    }
}

fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    }
}

/// One of a field's allowed values by its name or id (case-insensitive), or why not.
fn pick(f: &FieldMeta, v: &Value) -> Result<Value, String> {
    let s = as_text(v);
    if f.allowed.is_empty() {
        return Ok(match f.r#type.as_str() {
            "option" | "option-with-child" => json!({"value": s}),
            _ => json!({"name": s}),
        });
    }
    f.allowed
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(&s))
        .or_else(|| f.allowed.iter().find(|a| a.id == s))
        .map(|a| json!({"id": a.id}))
        .ok_or_else(|| {
            let names: Vec<&str> = f.allowed.iter().map(|a| a.name.as_str()).collect();
            format!("{} has no option {s:?} (it takes: {})", f.name, names.join(", "))
        })
}

fn number(f: &FieldMeta, v: &Value) -> Result<Value, String> {
    match v {
        Value::Number(_) => Ok(v.clone()),
        Value::String(s) => {
            let t = s.trim();
            if let Ok(i) = t.parse::<i64>() {
                return Ok(json!(i));
            }
            t.parse::<f64>().map(|x| json!(x)).map_err(|_| format!("{} takes a number, not {s:?}", f.name))
        }
        _ => Err(format!("{} takes a number, not {v}", f.name)),
    }
}

/// A plain value (text, a number, a list) as the field takes it: numbers parsed, options and priorities matched to
/// their ids, users by id, rich text as the deployment writes it, lists split on commas. Values already in Jira's
/// form (objects) go as they are.
pub fn coerce(f: &FieldMeta, v: &Value, deployment: Deployment) -> Result<Value, String> {
    if v.is_null() || shaped(v) {
        return Ok(v.clone());
    }
    let custom = f.custom.as_deref().unwrap_or_default();
    if custom.ends_with(":gh-sprint") {
        return number(f, v);
    }
    if custom.ends_with(":gh-epic-link") {
        return Ok(json!(as_text(v).to_ascii_uppercase()));
    }
    match f.r#type.as_str() {
        "number" => number(f, v),
        "string" => {
            let s = as_text(v);
            let rich = f.id == "description" || f.id == "environment" || custom.ends_with(":textarea");
            Ok(if rich { deployment.rich_text(&s) } else { json!(s) })
        }
        "user" => Ok(deployment.user(&as_text(v))),
        "option" | "option-with-child" | "priority" | "resolution" | "version" | "component" | "securitylevel"
        | "issuetype" => pick(f, v),
        "array" => {
            let list: Vec<Value> = match v {
                Value::Array(a) => a.clone(),
                Value::String(s) => s.split(',').map(str::trim).filter(|x| !x.is_empty()).map(|x| json!(x)).collect(),
                other => vec![other.clone()],
            };
            let item = FieldMeta { r#type: f.items.clone().unwrap_or_else(|| "string".into()), ..f.clone() };
            list.iter()
                .map(|x| match item.r#type.as_str() {
                    "string" => Ok(json!(as_text(x))),
                    "number" => number(&item, x),
                    _ => coerce(&item, x, deployment),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        _ => Ok(v.clone()),
    }
}

/// A standard field's plain value in Jira's form, for a field PDT has no metadata for: a priority by name,
/// components and versions as names, labels split on commas, rich text as the deployment writes it; anything else
/// as it is.
pub fn plain(id: &str, v: &Value, d: Deployment) -> Value {
    let names = |v: &Value| match v {
        Value::Array(a) => {
            Value::Array(a.iter().map(|x| if x.is_object() { x.clone() } else { json!({"name": x}) }).collect())
        }
        Value::String(s) => {
            Value::Array(s.split(',').map(str::trim).filter(|x| !x.is_empty()).map(|x| json!({"name": x})).collect())
        }
        other => other.clone(),
    };
    match (id, v) {
        (_, Value::Object(_)) => v.clone(),
        ("priority", Value::String(s)) => json!({"name": s.trim()}),
        ("components" | "fixVersions" | "versions", _) => names(v),
        ("labels", Value::String(s)) => {
            json!(s.split(',').map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>())
        }
        ("description" | "environment", Value::String(s)) => d.rich_text(s),
        _ => v.clone(),
    }
}

/// A call's fields (by id, mapped name or screen name) as Jira takes them, against a screen's `meta`; a field the
/// screen doesn't have is sent by its id with [`plain`]'s shape (Jira says if it can't be set).
pub fn coerce_fields(
    given: &Map<String, Value>,
    mappings: &Mappings,
    meta: &[FieldMeta],
    deployment: Deployment,
) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for (k, v) in given {
        match resolve_field(k, mappings, meta) {
            Some(f) => out.insert(f.id.clone(), coerce(f, v, deployment)?),
            None => {
                let id = field_id(k, mappings);
                let shaped = plain(&id, v, deployment);
                out.insert(id, shaped)
            }
        };
    }
    Ok(out)
}

/// A field a screen needs that a call left out, for the user or the agent to give.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "JiraMissingField")]
pub struct MissingField {
    pub id: String,
    pub name: String,
    pub r#type: String,
    /// The values it takes, when it is a choice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,
}

/// The required fields of a screen (without a default) that `given` (ids) leaves out.
pub fn missing_required(meta: &[FieldMeta], given: &Map<String, Value>) -> Vec<MissingField> {
    meta.iter()
        .filter(|f| f.required && !f.has_default && given.get(&f.id).is_none_or(Value::is_null))
        .map(|f| MissingField {
            id: f.id.clone(),
            name: f.name.clone(),
            r#type: f.items.as_ref().map_or_else(|| f.r#type.clone(), |i| format!("{}[]", i)),
            allowed: f.allowed.iter().map(|a| a.name.clone()).collect(),
        })
        .collect()
}

/// What the user (or the agent) needs to give, in a line: `Resolution (one of Fixed, Won't Fix), Story Points (a
/// number)`.
pub fn describe_missing(missing: &[MissingField]) -> String {
    missing
        .iter()
        .map(|m| {
            let what = if !m.allowed.is_empty() {
                format!("one of {}", m.allowed.join(", "))
            } else if m.r#type == "number" {
                "a number".into()
            } else {
                m.r#type.clone()
            };
            format!("{} [{}] ({what})", m.name, m.id)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// One transition open to an issue now, with its screen's fields (`expand=transitions.fields`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "JiraTransition")]
pub struct Transition {
    pub id: String,
    pub name: String,
    /// The status it leads to.
    pub to: String,
    pub fields: Vec<FieldMeta>,
}

pub fn parse_transitions(v: &Value) -> Vec<Transition> {
    v["transitions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| Transition {
            id: t["id"].as_str().map(String::from).unwrap_or_else(|| t["id"].to_string()),
            name: t["name"].as_str().unwrap_or_default().to_string(),
            to: t["to"]["name"].as_str().unwrap_or_default().to_string(),
            fields: parse_fields(&t["fields"]),
        })
        .collect()
}

/// The transition `to` names: by id, by name, or the one leading to that status (case-insensitive).
pub fn pick_transition<'a>(list: &'a [Transition], to: &str) -> Option<&'a Transition> {
    let to = to.trim();
    list.iter()
        .find(|t| t.id == to)
        .or_else(|| list.iter().find(|t| t.name.eq_ignore_ascii_case(to)))
        .or_else(|| list.iter().find(|t| t.to.eq_ignore_ascii_case(to)))
}

/// A sprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "JiraSprint")]
pub struct Sprint {
    pub id: i64,
    pub name: String,
    /// `active`, `future` or `closed`.
    pub state: String,
    /// A section of the backlog (the connection's backlog sections), not a sprint the team works in.
    #[serde(default)]
    pub backlog_section: bool,
}

/// A sprint as the Agile API or an issue's Sprint field gives it: an object (Cloud, newer Server), or Server's
/// `com.atlassian.greenhopper.service.sprint.Sprint@1a[id=3,rapidViewId=1,state=ACTIVE,name=Sprint 3,…]`.
pub fn parse_sprint(v: &Value, mappings: &Mappings) -> Option<Sprint> {
    let (id, name, state) = match v {
        Value::Object(_) => (
            v["id"].as_i64().or_else(|| v["id"].as_str().and_then(|s| s.parse().ok()))?,
            v["name"].as_str().unwrap_or_default().to_string(),
            v["state"].as_str().unwrap_or_default().to_lowercase(),
        ),
        Value::String(s) => {
            let inner = s.split_once('[')?.1.trim_end_matches(']');
            let mut id = None;
            let mut name = String::new();
            let mut state = String::new();
            // `name` may hold commas: a part without `=` belongs to the one before it.
            let mut last = "";
            for part in inner.split(',') {
                match part.split_once('=') {
                    Some((k, val)) if !k.contains(' ') => {
                        last = k;
                        match k {
                            "id" => id = val.parse().ok(),
                            "name" => name = val.to_string(),
                            "state" => state = val.to_lowercase(),
                            _ => {}
                        }
                    }
                    _ if last == "name" => {
                        name.push(',');
                        name.push_str(part);
                    }
                    _ => {}
                }
            }
            (id?, name, state)
        }
        _ => return None,
    };
    let backlog_section = mappings.is_backlog_section(&name);
    Some(Sprint { id, name, state, backlog_section })
}

/// An issue's open sprint (active first, else a future one) from its Sprint field (a list) or the Agile API's
/// `sprint`.
pub fn current_sprint(v: &Value, mappings: &Mappings) -> Option<Sprint> {
    let all: Vec<Sprint> = match v {
        Value::Array(a) => a.iter().filter_map(|s| parse_sprint(s, mappings)).collect(),
        Value::Null => vec![],
        one => parse_sprint(one, mappings).into_iter().collect(),
    };
    all.iter().find(|s| s.state == "active").or_else(|| all.iter().find(|s| s.state == "future")).cloned()
}

/// Where an issue sits on a board: its status, and its sprint membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "JiraPlacement")]
pub struct Placement {
    /// `sprint` (an active sprint: on the board, in its status's column), `future` (planned for a sprint not
    /// started), `section` (a backlog section) or `backlog`.
    pub area: String,
    /// The sprint or backlog section's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sprint: Option<String>,
    /// The status (its column, in a sprint).
    pub status: String,
}

/// An issue's place from its open sprint and status.
pub fn placement(sprint: Option<&Sprint>, status: &str) -> Placement {
    let (area, name) = match sprint {
        Some(s) if s.backlog_section => ("section", Some(s.name.clone())),
        Some(s) if s.state == "active" => ("sprint", Some(s.name.clone())),
        Some(s) => ("future", Some(s.name.clone())),
        None => ("backlog", None),
    };
    Placement { area: area.into(), sprint: name, status: status.into() }
}

/// The custom type of the Epic Name, Epic Link and Sprint fields, for finding them on a site that doesn't map them.
pub const EPIC_NAME_TYPE: &str = "com.pyxis.greenhopper.jira:gh-epic-label";
pub const EPIC_LINK_TYPE: &str = "com.pyxis.greenhopper.jira:gh-epic-link";
pub const SPRINT_TYPE: &str = "com.pyxis.greenhopper.jira:gh-sprint";

/// The id of the field of a custom type in `GET /field`'s list.
pub fn field_of_type(fields: &Value, custom: &str) -> Option<String> {
    fields
        .as_array()?
        .iter()
        .find(|f| f["schema"]["custom"].as_str() == Some(custom))
        .and_then(|f| f["id"].as_str())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Vec<FieldMeta> {
        parse_fields(&json!({
            "resolution": {"required": true, "name": "Resolution", "schema": {"type": "resolution"},
                "allowedValues": [{"id": "1", "name": "Fixed"}, {"id": "2", "name": "Won't Fix"}]},
            "customfield_10016": {"required": true, "name": "Story Points", "schema": {"type": "number",
                "custom": "com.atlassian.jira.plugin.system.customfieldtypes:float"}},
            "customfield_10200": {"required": false, "name": "Severity", "schema": {"type": "option",
                "custom": "com.atlassian.jira.plugin.system.customfieldtypes:select"},
                "allowedValues": [{"id": "10300", "value": "Sev 1"}, {"id": "10301", "value": "Sev 2"}]},
            "assignee": {"required": true, "hasDefaultValue": true, "name": "Assignee", "schema": {"type": "user"}},
            "labels": {"required": false, "name": "Labels", "schema": {"type": "array", "items": "string"}},
            "components": {"required": false, "name": "Component/s", "schema": {"type": "array", "items": "component"},
                "allowedValues": [{"id": "5", "name": "Web"}]},
            "description": {"required": false, "name": "Description", "schema": {"type": "string"}},
        }))
    }

    #[test]
    fn deployments() {
        assert_eq!(Deployment::of(&json!({})), Deployment::Cloud);
        assert_eq!(Deployment::of(&json!({"deployment": "server"})), Deployment::Server);
        assert_eq!(Deployment::Server.api(), "/rest/api/2");
        assert_eq!(Deployment::Server.rich_text("a\n\nb"), json!("a\n\nb"));
        assert_eq!(Deployment::Cloud.rich_text("a")["type"], "doc");
        assert_eq!(Deployment::Server.user("jdoe"), json!({"name": "jdoe"}));
        assert_eq!(Deployment::Server.user_id(&json!({"name": "jdoe", "key": "JIRAUSER1"})).as_deref(), Some("jdoe"));
        assert_eq!(Deployment::Cloud.user_id(&json!({"accountId": "a1"})).as_deref(), Some("a1"));
    }

    #[test]
    fn mappings_from_a_connection() {
        let m = Mappings::of(&json!({"fields": {"storyPoints": "customfield_10016", "x": ""},
            "epicLinkField": " customfield_10014 ", "epicNameField": "", "backlogSections": ["Backlog: *", "Icebox"]}));
        assert_eq!(m.field("StoryPoints"), Some("customfield_10016"));
        assert_eq!(m.fields.len(), 1);
        assert_eq!((m.epic_link.as_deref(), m.epic_name.as_deref()), (Some("customfield_10014"), None));
        assert!(m.is_backlog_section("backlog: ready") && m.is_backlog_section("ICEBOX"));
        assert!(!m.is_backlog_section("Sprint 4"));
        let lines = Mappings::of(&json!({"fields": "sev=customfield_10200\nbad", "backlogSections": "A\n\nB"}));
        assert_eq!(lines.field("sev"), Some("customfield_10200"));
        assert_eq!(lines.backlog_sections, ["A", "B"]);
    }

    #[test]
    fn fields_resolve_by_id_mapping_or_name() {
        let meta = meta();
        let m = Mappings { fields: vec![("points".into(), "customfield_10016".into())], ..Default::default() };
        for name in ["customfield_10016", "points", "story points"] {
            assert_eq!(resolve_field(name, &m, &meta).unwrap().id, "customfield_10016", "{name}");
        }
        assert!(resolve_field("nope", &m, &meta).is_none());
        assert_eq!(field_id("points", &m), "customfield_10016");
        // createmeta's list form.
        let list = parse_fields(&json!([{"fieldId": "summary", "name": "Summary", "required": true,
            "schema": {"type": "string"}}]));
        assert_eq!((list[0].id.as_str(), list[0].required), ("summary", true));
    }

    #[test]
    fn values_become_what_fields_take() {
        let meta = meta();
        let f = |id: &str| meta.iter().find(|f| f.id == id).unwrap().clone();
        let d = Deployment::Server;
        assert_eq!(coerce(&f("customfield_10016"), &json!("3.5"), d).unwrap(), json!(3.5));
        assert_eq!(coerce(&f("customfield_10016"), &json!("8"), d).unwrap(), json!(8));
        assert!(coerce(&f("customfield_10016"), &json!("lots"), d).unwrap_err().contains("takes a number"));
        assert_eq!(coerce(&f("resolution"), &json!("won't fix"), d).unwrap(), json!({"id": "2"}));
        assert_eq!(coerce(&f("customfield_10200"), &json!("Sev 2"), d).unwrap(), json!({"id": "10301"}));
        assert_eq!(coerce(&f("customfield_10200"), &json!("10300"), d).unwrap(), json!({"id": "10300"}));
        let e = coerce(&f("customfield_10200"), &json!("Sev 9"), d).unwrap_err();
        assert!(e.contains("Sev 1, Sev 2"), "{e}");
        assert_eq!(coerce(&f("assignee"), &json!("jdoe"), d).unwrap(), json!({"name": "jdoe"}));
        assert_eq!(coerce(&f("assignee"), &json!("a1"), Deployment::Cloud).unwrap(), json!({"accountId": "a1"}));
        assert_eq!(coerce(&f("labels"), &json!("a, b"), d).unwrap(), json!(["a", "b"]));
        assert_eq!(coerce(&f("components"), &json!(["web"]), d).unwrap(), json!([{"id": "5"}]));
        assert_eq!(coerce(&f("description"), &json!("x"), d).unwrap(), json!("x"));
        assert_eq!(coerce(&f("description"), &json!("x"), Deployment::Cloud).unwrap()["type"], "doc");
        // Already in Jira's form: as it is.
        assert_eq!(coerce(&f("customfield_10200"), &json!({"value": "Sev 1"}), d).unwrap(), json!({"value": "Sev 1"}));
        // An option field without allowed values: by value.
        let bare = FieldMeta { r#type: "option".into(), ..Default::default() };
        assert_eq!(coerce(&bare, &json!("X"), d).unwrap(), json!({"value": "X"}));
        assert_eq!(plain("priority", &json!("High"), d), json!({"name": "High"}));
        assert_eq!(plain("components", &json!("Web, API"), d), json!([{"name": "Web"}, {"name": "API"}]));
        assert_eq!(plain("labels", &json!("a,b"), d), json!(["a", "b"]));
        assert_eq!(plain("description", &json!("x"), Deployment::Cloud)["type"], "doc");
        assert_eq!(plain("customfield_1", &json!(3), d), json!(3));
        let sprint = FieldMeta { r#type: "array".into(), custom: Some(SPRINT_TYPE.into()), ..Default::default() };
        assert_eq!(coerce(&sprint, &json!("12"), d).unwrap(), json!(12));
    }

    #[test]
    fn a_transitions_missing_fields() {
        let meta = meta();
        let m = Mappings::default();
        let given: Map<String, Value> =
            serde_json::from_value(json!({"Severity": "Sev 1", "story points": "5"})).unwrap();
        let fields = coerce_fields(&given, &m, &meta, Deployment::Server).unwrap();
        assert_eq!(fields["customfield_10016"], json!(5));
        assert_eq!(fields["customfield_10200"], json!({"id": "10300"}));
        let missing = missing_required(&meta, &fields);
        assert_eq!(missing.len(), 1, "assignee has a default: {missing:?}");
        assert_eq!(missing[0].id, "resolution");
        assert_eq!(missing[0].allowed, ["Fixed", "Won't Fix"]);
        assert_eq!(describe_missing(&missing), "Resolution [resolution] (one of Fixed, Won't Fix)");
        let list = parse_transitions(&json!({"transitions": [
            {"id": "11", "name": "Resolve", "to": {"name": "Resolved"}, "fields": {
                "resolution": {"required": true, "name": "Resolution", "schema": {"type": "resolution"}}}},
            {"id": "21", "name": "Start", "to": {"name": "In Progress"}}]}));
        assert_eq!(list[0].fields[0].id, "resolution");
        assert!(list[1].fields.is_empty());
        assert_eq!(pick_transition(&list, "resolved").unwrap().id, "11");
        assert_eq!(pick_transition(&list, "21").unwrap().name, "Start");
        assert_eq!(pick_transition(&list, "start").unwrap().id, "21");
        assert!(pick_transition(&list, "Closed").is_none());
    }

    #[test]
    fn sprints_and_placement() {
        let m = Mappings { backlog_sections: vec!["Backlog: *".into()], ..Default::default() };
        let old = json!(
            "com.atlassian.greenhopper.service.sprint.Sprint@1a2b[id=3,rapidViewId=1,state=ACTIVE,name=Sprint 3, part two,startDate=2026-10-01]"
        );
        let s = parse_sprint(&old, &m).unwrap();
        assert_eq!((s.id, s.name.as_str(), s.state.as_str()), (3, "Sprint 3, part two", "active"));
        let field = json!([{"id": 1, "name": "Sprint 1", "state": "closed"},
            {"id": 9, "name": "Backlog: Ready", "state": "future"}]);
        let cur = current_sprint(&field, &m).unwrap();
        assert!(cur.backlog_section && cur.id == 9);
        assert_eq!(placement(Some(&cur), "To Do").area, "section");
        assert_eq!(
            placement(Some(&s), "In Progress"),
            Placement {
                area: "sprint".into(),
                sprint: Some("Sprint 3, part two".into()),
                status: "In Progress".into()
            }
        );
        let next = parse_sprint(&json!({"id": "4", "name": "Sprint 4", "state": "FUTURE"}), &m).unwrap();
        assert_eq!(placement(Some(&next), "To Do").area, "future");
        assert_eq!(placement(None, "To Do").area, "backlog");
        assert!(current_sprint(&json!([{"id": 1, "name": "S", "state": "closed"}]), &m).is_none());
        assert!(current_sprint(&Value::Null, &m).is_none());
        let fields = json!([{"id": "customfield_10014", "schema": {"custom": EPIC_LINK_TYPE}}]);
        assert_eq!(field_of_type(&fields, EPIC_LINK_TYPE).as_deref(), Some("customfield_10014"));
        assert!(field_of_type(&fields, SPRINT_TYPE).is_none());
    }
}
