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
    /// The PM persona has it.
    pub pm: bool,
    /// It always asks in PDT's Inbox (PDT-475), for every agent and whatever the project's policy: never Claude's
    /// permission prompt (so it is in `--allowedTools`), and never answered for the user. Implies [`ToolDecl::asks`].
    pub inbox_only: bool,
    /// It posts where anyone can read it (a forge comment, a pull request): outside a workflow step, which expects it,
    /// the call asks in PDT's Inbox, so an agent can't post what it read without the user's yes.
    pub public: bool,
}

impl ToolDecl {
    /// A tool that changes PDT and asks for nothing, not the PM's; the flags are set with the builders.
    pub const fn new(name: &'static str, does: &'static str) -> Self {
        Self {
            name,
            does,
            read_only: false,
            asks: false,
            open_world: false,
            destructive: false,
            pm: false,
            inbox_only: false,
            public: false,
        }
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
    pub const fn asks_in_inbox(self) -> Self {
        Self { asks: true, inbox_only: true, ..self }
    }
    pub const fn public(self) -> Self {
        Self { public: true, ..self }
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

    /// Whether a call to `tool` always asks in PDT's Inbox ([`ToolDecl::inbox_only`]).
    pub fn asks_in_inbox(&self, tool: &str) -> bool {
        self.get(tool).is_some_and(|t| t.inbox_only)
    }

    /// Whether a call to `tool` posts publicly ([`ToolDecl::public`]).
    pub fn public(&self, tool: &str) -> bool {
        self.get(tool).is_some_and(|t| t.public)
    }

    /// Whether Claude asks before a call to `tool` (its permission prompt): an asking tool that doesn't ask in PDT's
    /// Inbox instead, so it stays out of `--allowedTools`.
    pub fn claude_asks(&self, tool: &str) -> bool {
        self.get(tool).is_some_and(|t| t.asks && !t.inbox_only)
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

/// Where a plugin (an integration, as the UI calls it) sits in Settings → Plugins (PDT-228).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PluginCategory {
    /// Forges, trackers and CI.
    Dev,
    /// Document providers (Google's Docs and Sheets).
    Documents,
    Chat,
    Media,
    /// PDT's own general pieces: incoming webhooks, HTTP services, MCP servers.
    General,
}

/// A plugin's icon: a letter or two on a colour (a CSS colour; the brand's, so it is the plugin's, not the theme's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icon {
    pub letters: &'static str,
    pub color: &'static str,
}

/// An integration PDT knows of, whether or not this build has it (PDT-173): the list that names them outside their own
/// crates. Each one's crate is `crates/pdt-int-<id>` and pdtd's cargo feature `<id>`. An instance enables the ones it
/// uses (Settings → Plugins); this list says which an upgraded install already used, and names the integration a
/// workflow's node kind belongs to when it is turned off or not built in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownIntegration {
    pub id: &'static str,
    pub label: &'static str,
    /// What it adds, in a line (Settings → Plugins), whether or not this build has it.
    pub description: &'static str,
    pub category: PluginCategory,
    pub icon: Icon,
    /// The kinds of connection it adds.
    pub connection_kinds: &'static [&'static str],
    /// Its node kinds' (and event types') id prefixes, with their dot.
    pub kind_prefixes: &'static [&'static str],
}

pub const KNOWN: &[KnownIntegration] = &[
    KnownIntegration {
        id: "discord",
        label: "Discord",
        description: "Post to Discord channels from workflows, and let the PM manage a Discord server through a bot.",
        category: PluginCategory::Chat,
        icon: Icon { letters: "D", color: "#5865f2" },
        connection_kinds: &["discord_webhook", "discord_bot"],
        kind_prefixes: &["discord."],
    },
    KnownIntegration {
        id: "jira",
        label: "Jira",
        description: "Link projects to Jira Cloud or Server / Data Center: issues assigned to you become tickets, Done \
                      moves the issue, and the PM and workflows search, edit and move issues, sprints and epics.",
        category: PluginCategory::Dev,
        icon: Icon { letters: "J", color: "#2684ff" },
        connection_kinds: &["jira"],
        kind_prefixes: &["jira."],
    },
    KnownIntegration {
        id: "buildkite",
        label: "Buildkite",
        description: "CI of tickets' branches from Buildkite pipelines, shown and acted on like the pull request's CI.",
        category: PluginCategory::Dev,
        icon: Icon { letters: "BK", color: "#14cc80" },
        connection_kinds: &["buildkite"],
        kind_prefixes: &["buildkite."],
    },
    KnownIntegration {
        id: "gitlab",
        label: "GitLab",
        description: "Merge requests, pipelines, issues and releases for projects on gitlab.com or your own GitLab, as \
                      your GitLab login.",
        category: PluginCategory::Dev,
        icon: Icon { letters: "GL", color: "#fc6d26" },
        connection_kinds: &[],
        kind_prefixes: &[],
    },
    KnownIntegration {
        id: "google",
        label: "Google",
        description: "Google Docs and Sheets as documents: pick files from your Drive, make new ones, and let agents \
                      edit them (formulas, formatting and charts too) and answer their comments.",
        category: PluginCategory::Documents,
        icon: Icon { letters: "G", color: "#4285f4" },
        connection_kinds: &["google"],
        kind_prefixes: &[],
    },
    KnownIntegration {
        id: "arr",
        label: "Sonarr and Radarr",
        description: "Search Radarr and Sonarr, add films and series, and read their download queues from workflows.",
        category: PluginCategory::Media,
        icon: Icon { letters: "R", color: "#e6a817" },
        connection_kinds: &["sonarr", "radarr"],
        kind_prefixes: &["sonarr.", "radarr."],
    },
    KnownIntegration {
        id: "scryer",
        label: "Scryer",
        description: "Films, series and anime from a Scryer media manager: workflow actions, and the PM's tools for its \
                      library and settings.",
        category: PluginCategory::Media,
        icon: Icon { letters: "S", color: "#7c3aed" },
        connection_kinds: &["scryer"],
        kind_prefixes: &["scryer."],
    },
    KnownIntegration {
        id: "plex",
        label: "Plex",
        description: "Search a Plex server, list what it added lately and refresh its libraries from workflows.",
        category: PluginCategory::Media,
        icon: Icon { letters: "P", color: "#cc7b19" },
        connection_kinds: &["plex"],
        kind_prefixes: &["plex."],
    },
];

/// The known integration with this id.
pub fn known(id: &str) -> Option<&'static KnownIntegration> {
    KNOWN.iter().find(|k| k.id == id)
}

/// The known integration a node kind (or event type) belongs to.
pub fn owner_of_kind(kind: &str) -> Option<&'static KnownIntegration> {
    KNOWN.iter().find(|k| k.kind_prefixes.iter().any(|p| kind.starts_with(p)))
}

/// The known integration a kind of connection belongs to.
pub fn owner_of_connection(kind: &str) -> Option<&'static KnownIntegration> {
    KNOWN.iter().find(|k| k.connection_kinds.contains(&kind))
}

/// The integrations an install uses, in [`KNOWN`]'s order: those with a connection of one of their kinds, or a
/// workflow with one of their node kinds. What an upgraded install starts with enabled.
pub fn in_use<'a>(
    connection_kinds: impl IntoIterator<Item = &'a str>,
    node_kinds: impl IntoIterator<Item = &'a str>,
) -> Vec<&'static str> {
    let mut used: Vec<&'static str> = connection_kinds
        .into_iter()
        .filter_map(owner_of_connection)
        .chain(node_kinds.into_iter().filter_map(owner_of_kind))
        .map(|k| k.id)
        .collect();
    used.sort_by_key(|id| KNOWN.iter().position(|k| k.id == *id));
    used.dedup();
    used
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
    /// A connection's details in a line on its row: `{key}` is its config's value (`#{channel}`). Not shown when
    /// a key it names has no text value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub summary: Option<String>,
    /// A link the dialog shows once the settings it names are filled in (a bot's invite link).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub link: Option<ConnectionLink>,
    /// Its integration says how a connection of this kind is doing beyond OK or Failing
    /// (`GET /api/connections/{id}/status`, [`ConnectionStatus`]).
    #[serde(default)]
    pub status: bool,
    /// The PM acts through it: the switch that lets it act without asking (`config.pmActs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pm_acts: Option<String>,
}

/// A link made from a connection's settings, shown in its dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionLink {
    pub title: String,
    /// The address: `{key}` is that setting's text, URL-encoded. Shown once each one it names is filled in and
    /// matches its pattern.
    pub url: String,
    /// What to do with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hint: Option<String>,
}

/// How a connection is doing, in a line on its row (`GET /api/connections/{id}/status`): `{label}: {state}`, and
/// why, when there is a reason.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionStatus {
    pub label: String,
    pub state: String,
    #[serde(default)]
    pub tone: StatusTone,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

/// How a [`ConnectionStatus`]'s state is shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StatusTone {
    #[default]
    Neutral,
    Ok,
    Failed,
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
    /// What an empty value becomes (a project's integration setting); a switch's is `true` or `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default: Option<String>,
    /// A regular expression a value given must match in the form (`^\d+$`: digits only); the integration checks it
    /// again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pattern: Option<String>,
    /// Typed as a number (a phone's keypad).
    #[serde(default)]
    pub numeric: bool,
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
    /// On or off, kept as `true` or `false`.
    Switch,
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
    /// When the other setting does not have the value (`""`: once it is filled in).
    #[serde(default)]
    pub not: bool,
}

impl FieldWhen {
    pub fn new(key: &str, value: &str) -> Self {
        Self { key: key.into(), value: value.into(), not: false }
    }

    /// When setting `key` is filled in.
    pub fn filled(key: &str) -> Self {
        Self { key: key.into(), value: String::new(), not: true }
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
    /// A linked project's tickets list can import one of the integration's issues by its key
    /// (`POST /api/projects/{id}/import/{integration}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub import: Option<ImportByKey>,
}

/// How a linked project imports one issue by its key, as a Draft ticket linked to it: the words of its button and
/// dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportByKey {
    /// The button and the dialog's title (`Import from Jira`).
    pub title: String,
    /// What it does, under the dialog's title.
    pub description: String,
    /// An example key.
    pub placeholder: String,
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
                FieldType::Switch => match given(&f.key) {
                    None => Value::Bool(f.default.as_deref() == Some("true")),
                    Some(Value::Bool(b)) => Value::Bool(*b),
                    Some(_) => return Err(format!("{} is on or off", f.label)),
                },
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

    #[test]
    fn a_tool_that_asks_in_the_inbox_asks_but_never_through_claude() {
        let t = ToolTable::new([
            ToolDecl::new("plain", ""),
            ToolDecl::new("prompted", "").asks(),
            ToolDecl::new("inbox", "").asks_in_inbox().pm(),
        ]);
        assert_eq!(t.asking_tools(), ["prompted", "inbox"]);
        assert!(t.claude_asks("prompted") && !t.claude_asks("inbox") && !t.claude_asks("plain"));
        assert!(t.asks_in_inbox("inbox") && !t.asks_in_inbox("prompted") && !t.asks_in_inbox("missing"));
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
    fn project_settings_keep_a_switch_as_a_bool() {
        let s = ProjectSettingsSchema {
            fields: vec![ConfigField {
                r#type: Some(FieldType::Switch),
                default: Some("true".into()),
                ..ConfigField::new("on", "On")
            }],
            ..schema()
        };
        assert_eq!(s.apply(&map(json!({"connection": "t"})), None).unwrap().unwrap()["on"], true);
        assert_eq!(s.apply(&map(json!({"connection": "t", "on": false})), None).unwrap().unwrap()["on"], false);
        assert!(s.apply(&map(json!({"connection": "t", "on": "yes"})), None).is_err());
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
    fn integrations_in_use_come_from_connections_and_node_kinds() {
        assert_eq!(in_use(["scryer", "http", "discord_bot"], []), ["discord", "scryer"]);
        assert_eq!(in_use(["webhook"], ["jira.import", "condition", "radarr.add"]), ["jira", "arr"]);
        assert!(in_use([], ["manual"]).is_empty());
        assert_eq!(owner_of_kind("sonarr.queue").unwrap().id, "arr");
        let mut ids: Vec<_> = KNOWN.iter().map(|k| k.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), KNOWN.len());
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

/// A plugin's account sign-in in the browser (PDT-378): accounts that sign in on the service's own consent page, in
/// a window of their own, which comes back to PDT and closes. Settings → Plugins shows it on the plugin's page. Its
/// routes are under [`SignIn::api`]: `GET {api}/accounts` ([`SignInAccount`]), `POST {api}/sign-in` (adds an
/// account, with PDT's own client), `POST`/`DELETE {api}/accounts/{id}/sign-in` ([`AccountSignInInput`], [`AccountSignInStart`]),
/// `DELETE {api}/accounts/{id}/agents`; the service sends the browser back to `{api}/callback`, whose page posts
/// `{pdtSignIn: ok}` to the window that opened it. With [`SignIn::paste_code`], `POST {api}/sign-in/code`
/// ([`AccountSignInCode`], [`AccountSignInDone`]) finishes one with a code the website's page showed instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SignIn {
    /// Where its routes are (`/api/<plugin>`).
    pub api: String,
    /// Who an account signs in with, for people: "Sign in with {service}".
    pub service: String,
    /// The connection kind an account is.
    pub kind: String,
    /// PDT has its own client: Sign in with … adds an account (instead of Add), and a connection of the kind is
    /// only for the user's own client (under Advanced).
    pub built_in: bool,
    /// What the section says with PDT's own client.
    pub description: String,
    /// What Advanced says, with PDT's own client, before the redirect URI to add.
    pub own_client: String,
    /// A sign-in through the website's page can be finished by pasting the code it shows, when it can't send the
    /// browser back to PDT (a corporate proxy, browser isolation).
    #[serde(default)]
    pub paste_code: bool,
}

/// An account (connection) a plugin signs in ([`SignIn`]), as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SignInAccount {
    pub connection_id: String,
    pub name: String,
    pub signed_in: bool,
    /// The account's email, once signed in.
    pub email: Option<String>,
    /// Has the OAuth client signing in needs, its own or PDT's.
    pub has_client: bool,
    /// Signs in with PDT's own client; else with its own.
    pub built_in: bool,
    /// Agents may use it: with PDT's own client, once the user allowed them (a sign-in of their own); with its own
    /// client, whenever it is signed in.
    pub agents: bool,
    /// It opens its document provider's file picker ([`crate::document::ProviderPicker`]).
    pub picker: bool,
    /// What it lacks, in a line on its row (`No file picker (…)`).
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST {api}/sign-in` and `POST {api}/accounts/{id}/sign-in` ([`SignIn`]).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AccountSignInInput {
    /// The address PDT is open at in the browser (`http://127.0.0.1:7420`): the service sends the browser back there.
    pub origin: String,
    /// Asks the account's consent for agents (PDT's agentic client) instead of signing it in.
    #[serde(default)]
    #[ts(optional)]
    pub agent: Option<bool>,
}

/// `POST {api}/sign-in/code` ([`SignIn::paste_code`]).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AccountSignInCode {
    /// What the website's page showed to paste (or the address it sent the browser back to).
    pub code: String,
}

/// A sign-in finished with a pasted code.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AccountSignInDone {
    /// What happened, for the user (`PDT is signed in to Google as …`).
    pub message: String,
}

/// Where a sign-in goes ([`SignIn`]).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AccountSignInStart {
    /// The service's consent page, to open in a new window.
    pub url: String,
    /// The address the service sends the browser back to (it must be one of the OAuth client's redirect URIs).
    pub redirect_uri: String,
    /// With PDT's own client, the website's page to open first: once it says it is ready, the window that opened it
    /// sends it `url` (`{pdtRelayTo: url}`), so it sends the answer back only to the PDT that started the sign-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub relay: Option<String>,
}
