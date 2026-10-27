//! What an integration declares (PDT-171): its pdt-mcp tools and connection kinds. pdt-api's `Integration` trait hands
//! these in, with its node kinds and event types, and its registry gathers every integration's into the tables here.
//! Pure: nothing in this crate lists the integrations.

use crate::persona::{DEFAULT_TOOLS, PersonaTool};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One pdt-mcp tool as an integration declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDecl {
    pub name: &'static str,
    /// What it lets the agent do, in a line (the Personas page's tool list, PDT-110).
    pub does: &'static str,
    /// It changes nothing (MCP's `readOnlyHint`).
    pub read_only: bool,
    /// It acts for the user (PDT-81): the agent asks before each call (Claude's permission prompt, PDT's approval card)
    /// unless the project's policy (for an integration's tools, their connection) lets the PM act, so it is never in Claude's
    /// `--allowedTools`.
    pub asks: bool,
    /// It reaches outside PDT (MCP's `openWorldHint`).
    pub open_world: bool,
    /// It deletes something (MCP's `destructiveHint`).
    pub destructive: bool,
    /// The PM persona has it (PDT-81 spec §4).
    pub pm: bool,
}

impl ToolDecl {
    /// A tool that changes PDT and asks for nothing, not the PM's; the flags are set with the builders.
    pub const fn new(name: &'static str, does: &'static str) -> Self {
        Self { name, does, read_only: false, asks: false, open_world: false, destructive: false, pm: false }
    }
    pub const fn read_only(self) -> Self {
        Self { read_only: true, ..self }
    }
    pub const fn asks(self) -> Self {
        Self { asks: true, ..self }
    }
    pub const fn open_world(self) -> Self {
        Self { open_world: true, ..self }
    }
    pub const fn destructive(self) -> Self {
        Self { destructive: true, ..self }
    }
    pub const fn pm(self) -> Self {
        Self { pm: true, ..self }
    }
}

/// Every pdt-mcp tool PDT has, in the order `tools/list` gives them: the registered integrations' tools, in turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolTable {
    tools: Vec<ToolDecl>,
}

impl ToolTable {
    /// The table of `tools`; a name given twice keeps its first.
    pub fn new(tools: impl IntoIterator<Item = ToolDecl>) -> Self {
        let mut out: Vec<ToolDecl> = vec![];
        for t in tools {
            if !out.iter().any(|x| x.name == t.name) {
                out.push(t);
            }
        }
        Self { tools: out }
    }

    pub fn all(&self) -> &[ToolDecl] {
        &self.tools
    }

    pub fn get(&self, name: &str) -> Option<&ToolDecl> {
        self.tools.iter().find(|t| t.name == name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Every tool's name, in order.
    pub fn names(&self) -> Vec<&'static str> {
        self.tools.iter().map(|t| t.name).collect()
    }

    /// Whether a call to `tool` needs the user's yes ([`ToolDecl::asks`]).
    pub fn asks(&self, tool: &str) -> bool {
        self.get(tool).is_some_and(|t| t.asks)
    }

    /// The PM persona's tools, in order.
    pub fn pm_tools(&self) -> Vec<&'static str> {
        self.tools.iter().filter(|t| t.pm).map(|t| t.name).collect()
    }

    /// The tools that ask, in order.
    pub fn asking_tools(&self) -> Vec<&'static str> {
        self.tools.iter().filter(|t| t.asks).map(|t| t.name).collect()
    }

    /// Every tool as the Personas page lists it (`GET /api/personas/tools`), in order.
    pub fn persona_tools(&self) -> Vec<PersonaTool> {
        self.tools
            .iter()
            .map(|t| PersonaTool {
                name: t.name.to_string(),
                does: t.does.to_string(),
                asks: t.asks,
                default: DEFAULT_TOOLS.contains(&t.name),
            })
            .collect()
    }
}

/// A kind of connection the workflow actions (and the PM's tools) use: what it needs, and how to get its secret. A
/// connection's secret is kept in the OS keychain; PDT never shows it again. `GET /api/connections/kinds` lists them
/// for the Connections page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionKind {
    pub id: String,
    pub label: String,
    /// Its section on the Connections page.
    pub plural: String,
    /// How to set one up, under the section's title.
    pub setup: String,
    /// What it is for, in the kind picker.
    pub description: String,
    /// Asks for a base URL.
    pub url: bool,
    /// What the secret is, and where to find it.
    pub secret_label: String,
    pub secret_hint: String,
    /// A random secret can be generated (an incoming webhook's).
    #[serde(default)]
    pub generate: bool,
    /// Asks how the secret is sent (an HTTP service).
    #[serde(default)]
    pub auth: bool,
    /// The secret may be left out.
    #[serde(default)]
    pub secret_optional: bool,
    /// The address's example, when it isn't a plain base URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub url_placeholder: Option<String>,
    /// The address's hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub url_hint: Option<String>,
    /// The address may be left empty (then PDT uses the service's own).
    #[serde(default)]
    pub url_optional: bool,
    /// Asks for an address only when this setting has this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub url_when: Option<FieldWhen>,
    /// Settings of its own, kept in the connection's config.
    #[serde(default)]
    pub fields: Vec<ConfigField>,
    /// Asks for a Discord server and its bot's application (a Discord bot).
    #[serde(default)]
    pub discord_bot: bool,
    /// The PM acts through it: the switch that lets it act without asking (`config.pmActs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pm_acts: Option<String>,
}

/// One setting of a connection kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConfigField {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub r#type: Option<FieldType>,
    /// A `choice`'s options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub options: Option<Vec<FieldOption>>,
    /// Shown only when another setting has this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub when: Option<FieldWhen>,
    /// May be left empty (otherwise a shown setting is required).
    #[serde(default)]
    pub optional: bool,
    /// What an empty value becomes (a project's integration setting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default: Option<String>,
}

impl ConfigField {
    /// A text setting; the rest is set on the value.
    pub fn new(key: &str, label: &str) -> Self {
        Self { key: key.into(), label: label.into(), ..Default::default() }
    }
}

/// How a setting is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FieldType {
    /// Plain text (the default).
    Text,
    /// A list, one per line.
    Lines,
    /// `NAME=value` lines.
    Env,
    /// One of the field's options.
    Choice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FieldOption {
    pub value: String,
    pub label: String,
}

/// A condition on another setting's value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FieldWhen {
    pub key: String,
    pub value: String,
}

impl FieldWhen {
    pub fn new(key: &str, value: &str) -> Self {
        Self { key: key.into(), value: value.into() }
    }
}

/// What a project sets for an integration on this host (PDT-174): the connection it links through, and the
/// integration's own fields. The project settings form is made from it, and a project's settings for the integration
/// are kept as a JSON object under its id: `connection` and each field's key (text and choices as strings, lists as
/// arrays of strings). `GET /api/integrations/project-settings` lists them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectSettingsSchema {
    /// The integration's id, the key of its settings.
    pub integration: String,
    pub label: String,
    /// What linking a project does.
    pub description: String,
    /// The kind of connection the project links through; no connection, no link.
    pub connection_kind: String,
    /// Its settings, shown once a connection is picked. Text, choice and lines only.
    #[serde(default)]
    pub fields: Vec<ConfigField>,
}

/// A project's settings for one integration: `connection` and the schema's fields.
pub type IntegrationSettings = serde_json::Map<String, serde_json::Value>;

impl ProjectSettingsSchema {
    /// The settings `input` makes over `current`. A key left out keeps its current value (or its default); an empty
    /// value takes the default. None: unlinked (no connection). Err is a message for the user. The connection's kind
    /// is not checked here (that needs the store).
    pub fn apply(
        &self,
        input: &IntegrationSettings,
        current: Option<&IntegrationSettings>,
    ) -> Result<Option<IntegrationSettings>, String> {
        use serde_json::Value;
        if let Some(k) = input.keys().find(|k| *k != "connection" && !self.fields.iter().any(|f| f.key == **k)) {
            return Err(format!("{} has no setting {k}", self.label));
        }
        let given = |key: &str| input.get(key).or_else(|| current.and_then(|c| c.get(key))).filter(|v| !v.is_null());
        let connection = match given("connection") {
            None => "",
            Some(Value::String(s)) => s.trim(),
            Some(_) => return Err(format!("{}'s connection is a name", self.label)),
        };
        if connection.is_empty() {
            return Ok(None);
        }
        let mut out = IntegrationSettings::new();
        out.insert("connection".into(), Value::String(connection.into()));
        for f in &self.fields {
            let v = match f.r#type.unwrap_or(FieldType::Text) {
                FieldType::Lines => {
                    let mut list: Vec<String> = vec![];
                    match given(&f.key) {
                        None => {}
                        Some(Value::Array(items)) => {
                            for item in items {
                                let Value::String(s) = item else {
                                    return Err(format!("{} is a list of text", f.label));
                                };
                                let s = s.trim();
                                if !s.is_empty() && !list.iter().any(|x| x == s) {
                                    list.push(s.into());
                                }
                            }
                        }
                        Some(_) => return Err(format!("{} is a list of text", f.label)),
                    }
                    if list.is_empty() && !f.optional {
                        return Err(format!("{} needs at least one", f.label));
                    }
                    Value::Array(list.into_iter().map(Value::String).collect())
                }
                FieldType::Text | FieldType::Choice | FieldType::Env => {
                    let text = match given(&f.key) {
                        None => "",
                        Some(Value::String(s)) => s.trim(),
                        Some(_) => return Err(format!("{} is text", f.label)),
                    };
                    let text = if text.is_empty() { f.default.as_deref().unwrap_or("") } else { text };
                    if text.is_empty() && !f.optional {
                        return Err(format!("{} is required", f.label));
                    }
                    if let Some(options) = f.options.as_ref().filter(|_| !text.is_empty())
                        && !options.iter().any(|o| o.value == text)
                    {
                        return Err(format!("{text:?} is not one of {}'s options", f.label));
                    }
                    Value::String(text.into())
                }
            };
            out.insert(f.key.clone(), v);
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn schema() -> ProjectSettingsSchema {
        ProjectSettingsSchema {
            integration: "tracker".into(),
            label: "Tracker".into(),
            connection_kind: "tracker".into(),
            fields: vec![
                ConfigField { optional: true, ..ConfigField::new("scope", "Scope") },
                ConfigField { default: Some("Done".into()), ..ConfigField::new("done", "Done") },
                ConfigField { r#type: Some(FieldType::Lines), optional: true, ..ConfigField::new("tags", "Tags") },
            ],
            ..Default::default()
        }
    }

    fn map(v: Value) -> IntegrationSettings {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn project_settings_take_defaults_and_tidy_lists() {
        let got = schema().apply(&map(json!({"connection": " t ", "tags": ["a", " a ", "", "b"]})), None).unwrap();
        assert_eq!(got, Some(map(json!({"connection": "t", "scope": "", "done": "Done", "tags": ["a", "b"]}))));
    }

    #[test]
    fn project_settings_left_out_are_kept_and_no_connection_unlinks() {
        let current = map(json!({"connection": "t", "scope": "x = 1", "done": "Close", "tags": ["a"]}));
        let got = schema().apply(&map(json!({"done": ""})), Some(&current)).unwrap().unwrap();
        assert_eq!(got, map(json!({"connection": "t", "scope": "x = 1", "done": "Done", "tags": ["a"]})));
        assert_eq!(schema().apply(&map(json!({"connection": ""})), Some(&current)).unwrap(), None);
        assert_eq!(schema().apply(&map(json!({"scope": "y"})), None).unwrap(), None, "nothing to link through");
    }

    #[test]
    fn project_settings_refuse_unknown_keys_and_wrong_types() {
        let s = schema();
        assert!(s.apply(&map(json!({"connection": "t", "nope": 1})), None).unwrap_err().contains("nope"));
        assert!(s.apply(&map(json!({"connection": "t", "tags": "a"})), None).is_err());
        assert!(s.apply(&map(json!({"connection": "t", "scope": 3})), None).is_err());
        assert!(s.apply(&map(json!({"connection": 3})), None).is_err());
        let required = ProjectSettingsSchema { fields: vec![ConfigField::new("must", "Must")], ..schema() };
        assert!(required.apply(&map(json!({"connection": "t"})), None).unwrap_err().contains("Must"));
        let choice = ProjectSettingsSchema {
            fields: vec![ConfigField {
                r#type: Some(FieldType::Choice),
                options: Some(vec![FieldOption { value: "a".into(), label: "A".into() }]),
                ..ConfigField::new("pick", "Pick")
            }],
            ..schema()
        };
        assert!(choice.apply(&map(json!({"connection": "t", "pick": "b"})), None).is_err());
        assert!(choice.apply(&map(json!({"connection": "t", "pick": "a"})), None).unwrap().is_some());
    }

    #[test]
    fn a_table_keeps_the_first_of_a_name_and_answers_by_flag() {
        let t = ToolTable::new([
            ToolDecl::new("get_ticket", "Read").read_only().pm(),
            ToolDecl::new("create_ticket", "Create").asks().pm(),
            ToolDecl::new("get_ticket", "Again"),
            ToolDecl::new("update_ticket", "Rewrite"),
        ]);
        assert_eq!(t.names(), ["get_ticket", "create_ticket", "update_ticket"]);
        assert_eq!(t.get("get_ticket").unwrap().does, "Read");
        assert!(t.asks("create_ticket") && !t.asks("get_ticket") && !t.asks("nope"));
        assert_eq!(t.pm_tools(), ["get_ticket", "create_ticket"]);
        assert_eq!(t.asking_tools(), ["create_ticket"]);
        let listed = t.persona_tools();
        assert!(listed[0].default && !listed[1].default && listed[1].asks);
    }

    #[test]
    fn a_connection_kind_leaves_out_what_it_does_not_set() {
        let k = ConnectionKind {
            id: "x".into(),
            fields: vec![ConfigField { r#type: Some(FieldType::Lines), ..ConfigField::new("args", "Arguments") }],
            ..Default::default()
        };
        let v = serde_json::to_value(&k).unwrap();
        assert!(v.get("pmActs").is_none() && v.get("urlWhen").is_none());
        assert_eq!(v["fields"][0]["type"], "lines");
        assert!(v["fields"][0].get("options").is_none());
    }
}
