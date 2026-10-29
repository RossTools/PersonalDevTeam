//! Jira's actions (Phase 7e spec §6.3, PDT-85; PDT-172 actions, PDT-258 on-premise Jira and the rest): each one a
//! workflow node kind (`jira.search`, …) and a pdt-mcp tool (`jira_search`, …) from one definition, so the PM and
//! workflows do the same things the same way. Reading asks for nothing; what changes Jira (or makes a ticket) asks
//! unless the connection lets the PM act (`config.pmActs`). Fields go by id, by a name the connection maps
//! (`storyPoints`), or by their name on Jira's screen; plain values become what each field takes, from Jira's
//! metadata ([`crate::meta`]). The triggers are the plain `jira.issue.*` event triggers.

use crate::client::{JiraClient, Special};
use crate::core::{DEFAULT_DONE, JIRA, JiraSettings, issue_key, parse_issue};
use crate::meta::{self, Deployment, MissingField, Placement, Sprint, Transition};
use crate::sensor::{Import, import_issue};
use pdt_api::sdk::{Action, ActionCx, AnyAction, BoxFut, ConnectionFace, NodeFace, ToolFace};
use pdt_core::integration::ToolDecl;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use ts_rs::TS;

/// Every Jira action, in catalog and `tools/list` order.
pub(crate) fn actions() -> Vec<Arc<dyn AnyAction>> {
    vec![
        Arc::new(Check),
        Arc::new(Search),
        Arc::new(GetIssue),
        Arc::new(JiraImport),
        Arc::new(JiraCreate),
        Arc::new(Edit),
        Arc::new(JiraComment),
        Arc::new(Transitions),
        Arc::new(JiraTransition),
        Arc::new(Boards),
        Arc::new(Sprints),
        Arc::new(Board),
        Arc::new(MoveToSprint),
        Arc::new(SetEpic),
    ]
}

/// The title of the user's approval of a call to one of the asking tools (the agent's arguments, `connection` maybe
/// left out).
pub(crate) fn approval_title(tool: &str, input: &Value) -> Option<String> {
    fn t<A: Action>(a: A, v: Value) -> Option<String> {
        serde_json::from_value::<A::Input>(v).ok().map(|i| a.approval_title(&i))
    }
    let mut v = input.clone();
    if let Some(o) = v.as_object_mut() {
        o.entry("connection").or_insert(json!("Jira"));
    }
    match tool {
        "jira_import" => t(JiraImport, v),
        "jira_create" => t(JiraCreate, v),
        "jira_edit" => t(Edit, v),
        "jira_comment" => t(JiraComment, v),
        "jira_transition" => t(JiraTransition, v),
        "jira_sprint" => t(MoveToSprint, v),
        "jira_epic" => t(SetEpic, v),
        _ => None,
    }
}

/// What the PM's prompt says about the Jira tools.
pub(crate) const PM_PROMPT: &str = "- Jira (Cloud, or Server / Data Center): `jira_search` runs JQL a page at a time (pass \
`next` back as `page`), `jira_issue` reads one issue with its comments and mapped fields, `jira_transitions` lists the \
moves open to an issue and the fields each one needs, `jira_boards`, `jira_sprints` (active sprints and backlog \
sections) and `jira_board` (where each issue sits: sprint, section or backlog, and its status) read the boards, and \
`jira_check` tests the connection. These ask for nothing. `jira_import` makes a ticket from an issue (never twice), \
`jira_create` and `jira_edit` create and change issues (type, description, assignee, priority, labels, components, \
epic, and any field by id, by the name the connection maps, or by its name in Jira), `jira_comment` comments, \
`jira_transition` moves an issue with the fields its screen needs in the same call, `jira_sprint` moves issues into a \
sprint (`active`, a name or an id) or back to the backlog, and `jira_epic` puts issues in an epic or takes them out; \
each waits for the user's yes unless the Jira connection lets you act. When Jira needs a value you don't have (a \
transition's required field), ask the user for it rather than guessing.";

// ---- shared ----

const THEN: &[(&str, &str)] = &[("out", "Then")];

const CONNECTION: Option<ConnectionFace> = Some(ConnectionFace { kind: JIRA, label: "Jira" });

const fn face(decl: ToolDecl, title: &'static str, description: &'static str) -> ToolFace {
    ToolFace { decl, title, description }
}

const fn node(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    ports: &'static [(&'static str, &'static str)],
) -> Option<NodeFace> {
    Some(NodeFace { id, title, description, ports })
}

fn is_ref(s: &str) -> bool {
    s.contains("{{")
}

fn key_ok(key: &str) -> Result<(), String> {
    if is_ref(key) || issue_key(key).is_some() { Ok(()) } else { Err(format!("{key:?} is not an issue key")) }
}

fn the_key(key: &str) -> Result<String, String> {
    issue_key(key).ok_or_else(|| format!("{key:?} is not an issue key"))
}

fn keys_ok(keys: &[String]) -> Result<(), String> {
    if keys.is_empty() {
        return Err("keys is empty: name the issues".into());
    }
    keys.iter().try_for_each(|k| key_ok(k))
}

/// Keys, each in Jira's form; a list given as one comma-separated string (a reference's value) too.
fn the_keys(keys: &[String]) -> Result<Vec<String>, String> {
    let mut out = vec![];
    for k in keys.iter().flat_map(|k| k.split(',')).map(str::trim).filter(|k| !k.is_empty()) {
        out.push(the_key(k)?);
    }
    if out.is_empty() {
        return Err("keys is empty: name the issues".into());
    }
    Ok(out)
}

fn nonblank(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn short(s: &str) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 60 { format!("{}…", one.chars().take(60).collect::<String>()) } else { one }
}

fn fields_list(fields: &Map<String, Value>) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{k}={}", v.as_str().map_or_else(|| v.to_string(), String::from)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// An issue in a list: what it is, and where it sits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueSummary {
    pub key: String,
    pub url: String,
    pub summary: String,
    pub status: String,
    pub issue_type: String,
    pub assignee: Option<String>,
    pub priority: Option<String>,
    pub labels: Vec<String>,
    /// Its open sprint (active, else planned), or the backlog section it is in.
    pub sprint: Option<Sprint>,
    /// Its epic's key.
    pub epic: Option<String>,
    pub placement: Placement,
    /// The PDT ticket linked to it, if any.
    pub ticket_id: Option<String>,
}

/// The fields a list of issues asks Jira for.
fn list_fields(sp: &Special) -> Vec<String> {
    let mut f: Vec<String> =
        ["summary", "status", "assignee", "issuetype", "priority", "labels", "parent"].map(String::from).to_vec();
    f.extend(sp.sprint.iter().chain(sp.epic_link.iter()).cloned());
    f
}

fn summary_of(c: &JiraClient, sp: &Special, v: &Value, ticket: Option<String>) -> JiraIssueSummary {
    let f = &v["fields"];
    let s = |path: &[&str]| {
        let mut cur = f;
        for p in path {
            cur = &cur[*p];
        }
        cur.as_str().map(String::from)
    };
    // The Agile API's own `sprint` (the open one), else the Sprint field (every one the issue was in).
    let sprint_value = match &f["sprint"] {
        Value::Null => sp.sprint.as_ref().map(|id| f[id].clone()).unwrap_or(Value::Null),
        one => one.clone(),
    };
    let sprint = meta::current_sprint(&sprint_value, &c.mappings);
    let status = s(&["status", "name"]).unwrap_or_default();
    let parent_is_epic =
        f["parent"]["fields"]["issuetype"]["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case("epic"))
            || f["parent"]["fields"]["issuetype"]["hierarchyLevel"].as_i64() == Some(1);
    let epic = sp
        .epic_link
        .as_ref()
        .and_then(|id| f[id].as_str().map(String::from))
        .or_else(|| if parent_is_epic { f["parent"]["key"].as_str().map(String::from) } else { None });
    let key = v["key"].as_str().unwrap_or_default().to_string();
    JiraIssueSummary {
        url: c.browse(&key),
        key,
        summary: s(&["summary"]).unwrap_or_default(),
        placement: meta::placement(sprint.as_ref(), &status),
        status,
        issue_type: s(&["issuetype", "name"]).unwrap_or_default(),
        assignee: s(&["assignee", "displayName"]),
        priority: s(&["priority", "name"]),
        labels: f["labels"]
            .as_array()
            .map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        sprint,
        epic,
        ticket_id: ticket,
    }
}

async fn ticket_of(cx: &ActionCx<'_>, c: &JiraClient, key: &str) -> Result<Option<String>, String> {
    cx.st.store.ticket_for_external(JIRA, &c.conn.id, key).await.map_err(|e| e.to_string())
}

async fn summaries(
    cx: &ActionCx<'_>,
    c: &JiraClient,
    sp: &Special,
    list: &[Value],
) -> Result<Vec<JiraIssueSummary>, String> {
    let mut out = vec![];
    for v in list {
        let key = v["key"].as_str().unwrap_or_default();
        out.push(summary_of(c, sp, v, ticket_of(cx, c, key).await?));
    }
    Ok(out)
}

/// A call's fields as Jira takes them: against the screen's metadata when there is some, else by mapped id with the
/// standard fields shaped.
fn shape_fields(
    c: &JiraClient,
    given: &Map<String, Value>,
    meta: Option<&[meta::FieldMeta]>,
) -> Result<Map<String, Value>, String> {
    meta::coerce_fields(given, &c.mappings, meta.unwrap_or_default(), c.deployment)
}

// ---- jira.check ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraCheckConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraCheckOutput {
    pub connection: String,
    /// `cloud` or `server`.
    pub deployment: String,
    pub site: String,
    /// Who the connection signs in as.
    pub user: String,
    pub user_id: String,
    /// Jira's version, when it says.
    pub version: Option<String>,
}

pub struct Check;

impl Action for Check {
    type Input = JiraCheckConfig;
    type Output = JiraCheckOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_check", "Test a Jira connection").read_only().open_world().pm(),
            "Test a Jira connection",
            "Signs in to Jira with the connection and says who it is signed in as, the site, Cloud or Server / Data \
             Center, and Jira's version. The token never appears.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.check",
            "Test Jira",
            "Signs in with a Jira connection: `{{steps.<id>.user}}` is who it is; a connection that fails fails the \
             step.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn approval_title(&self, c: &JiraCheckConfig) -> String {
        format!("test the Jira connection {}", c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraCheckConfig) -> BoxFut<'a, Result<JiraCheckOutput, String>> {
        Box::pin(async move {
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let me = client.myself_raw().await?;
            let info = client.server_info().await;
            Ok(JiraCheckOutput {
                connection: client.conn.name.clone(),
                deployment: match client.deployment {
                    Deployment::Cloud => "cloud".into(),
                    Deployment::Server => "server".into(),
                },
                site: client.site().to_string(),
                user: me["displayName"].as_str().unwrap_or_default().to_string(),
                user_id: client.deployment.user_id(&me).unwrap_or_default(),
                version: info.and_then(|i| i["version"].as_str().map(String::from)),
            })
        })
    }
}

// ---- jira.search ----

/// How many issues a search page has unless told.
const SEARCH_LIMIT: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraSearchConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// JQL (`project = ABC AND sprint in openSprints() ORDER BY rank`).
    pub jql: String,
    /// At most this many issues (default 50, at most 100).
    #[serde(default)]
    pub limit: Option<usize>,
    /// The page to read: the last page's `next`.
    #[serde(default)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraSearchOutput {
    pub issues: Vec<JiraIssueSummary>,
    /// The next page's `page`, when there are more.
    pub next: Option<String>,
    /// How many issues match, when Jira says.
    #[ts(type = "number | null")]
    pub total: Option<i64>,
}

pub struct Search;

impl Action for Search {
    type Input = JiraSearchConfig;
    type Output = JiraSearchOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_search", "Search Jira with JQL").read_only().open_world().pm(),
            "Search Jira",
            "Issues a JQL query finds, a page at a time (`limit`, and `page`: the last page's `next`): each one's \
             key, summary, status, type, assignee, priority, labels, sprint, epic, where it sits on the board \
             (`placement`) and the PDT ticket linked to it.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.search",
            "Search Jira",
            "Issues a JQL query finds: `{{steps.<id>.issues}}` is a list of {key, summary, status, assignee, sprint, \
             epic, placement, ticketId}; `{{steps.<id>.next}}` reads the next page.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraSearchConfig) -> Result<(), String> {
        if c.jql.trim().is_empty() { Err("jql is empty".into()) } else { Ok(()) }
    }

    fn approval_title(&self, c: &JiraSearchConfig) -> String {
        format!("search Jira ({}): {}", c.connection, short(&c.jql))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraSearchConfig) -> BoxFut<'a, Result<JiraSearchOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let sp = client.special().await?;
            let limit = c.limit.unwrap_or(SEARCH_LIMIT).clamp(1, 100);
            let page = client.search_page(c.jql.trim(), &list_fields(&sp), false, limit, nonblank(&c.page)).await?;
            Ok(JiraSearchOutput {
                issues: summaries(cx, &client, &sp, &page.issues).await?,
                next: page.next,
                total: page.total,
            })
        })
    }
}

// ---- jira.issue ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraIssueConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The issue (`ABC-123`).
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraCommentText {
    pub id: String,
    pub author: String,
    /// Text.
    pub body: String,
    /// ms since the epoch.
    #[ts(type = "number")]
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueDetails {
    #[serde(flatten)]
    #[ts(flatten)]
    pub issue: JiraIssueSummary,
    /// Text.
    pub description: String,
    pub reporter: Option<String>,
    pub components: Vec<String>,
    pub comments: Vec<JiraCommentText>,
    /// The connection's mapped fields, by the names it gives them, as Jira has them.
    #[ts(type = "Record<string, unknown>")]
    pub fields: Map<String, Value>,
}

pub struct GetIssue;

impl Action for GetIssue {
    type Input = JiraIssueConfig;
    type Output = JiraIssueDetails;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_issue", "Read a Jira issue").read_only().open_world().pm(),
            "Read a Jira issue",
            "One issue: its summary, description, status, type, assignee, reporter, priority, labels, components, \
             sprint, epic, where it sits on the board, its comments, the connection's mapped fields, and the PDT \
             ticket linked to it.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.issue",
            "Read a Jira issue",
            "One issue with its description and comments: `{{steps.<id>.status}}`, `{{steps.<id>.sprint.name}}`, \
             `{{steps.<id>.fields.<mapped name>}}`.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraIssueConfig) -> Result<(), String> {
        key_ok(&c.key)
    }

    fn approval_title(&self, c: &JiraIssueConfig) -> String {
        format!("read {} in Jira ({})", c.key, c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraIssueConfig) -> BoxFut<'a, Result<JiraIssueDetails, String>> {
        Box::pin(async move {
            let key = the_key(&c.key)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let sp = client.special().await?;
            let mut fields = list_fields(&sp);
            fields.extend(["description", "reporter", "components", "comment", "created", "updated"].map(String::from));
            fields.extend(client.mappings.fields.iter().map(|(_, id)| id.clone()));
            let v = client
                .issue_raw(&key, &fields)
                .await?
                .ok_or_else(|| format!("Jira has no issue {key} (or the account can't see it)"))?;
            let parsed = parse_issue(client.site(), &v).unwrap_or_default();
            let mapped: Map<String, Value> =
                client.mappings.fields.iter().map(|(name, id)| (name.clone(), v["fields"][id].clone())).collect();
            Ok(JiraIssueDetails {
                issue: summary_of(&client, &sp, &v, ticket_of(cx, &client, &key).await?),
                description: parsed.description,
                reporter: parsed.reporter,
                components: v["fields"]["components"]
                    .as_array()
                    .map(|l| l.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect())
                    .unwrap_or_default(),
                comments: parsed
                    .comments
                    .into_iter()
                    .map(|c| JiraCommentText { id: c.id, author: c.author, body: c.body, created: c.created })
                    .collect(),
                fields: mapped,
            })
        })
    }
}

// ---- jira.import ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraImportConfig {
    /// The issue (`ABC-123`, `{{trigger.key}}`).
    pub key: String,
    /// The PDT project the ticket goes in (`{{trigger.projectId}}`).
    pub project: String,
    /// A `jira` connection; default the project's.
    #[serde(default)]
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraImportOutput {
    pub ticket_id: String,
    pub key: String,
    /// A ticket already linked the issue: nothing was made.
    #[serde(default)]
    pub existed: bool,
}

pub struct JiraImport;

impl Action for JiraImport {
    type Input = JiraImportConfig;
    type Output = JiraImportOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_import", "Import a Jira issue as a ticket").asks().open_world().pm(),
            "Import a Jira issue as a ticket",
            "Makes a Draft ticket in a PDT project (`project`: its id) from a Jira issue (its summary and \
             description), linked to the issue. An issue a ticket already links is never imported again: the answer \
             names that ticket (`existed`).",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.import",
            "Import a Jira issue",
            "Makes a Draft ticket from a Jira issue (its summary and description), linked to it. When a ticket \
             already links the issue nothing is made and the step leaves by `exists`: an issue is never imported \
             twice.",
            &[("imported", "Imported"), ("exists", "Already a ticket")],
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraImportConfig) -> Result<(), String> {
        if c.project.trim().is_empty() {
            return Err("project is empty: name the PDT project ({{trigger.projectId}})".into());
        }
        key_ok(&c.key)
    }

    fn approval_title(&self, c: &JiraImportConfig) -> String {
        format!("import {} as a ticket", c.key)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraImportConfig) -> BoxFut<'a, Result<JiraImportOutput, String>> {
        Box::pin(async move {
            let r = import_issue(cx.st, &c.project, &c.key, c.connection.as_deref()).await.map_err(|e| e.message())?;
            let key = issue_key(&c.key).unwrap_or(c.key);
            Ok(match r {
                Import::Imported(ticket_id) => JiraImportOutput { ticket_id, key, existed: false },
                Import::Exists(ticket_id) => JiraImportOutput { ticket_id, key, existed: true },
            })
        })
    }

    fn port(&self, out: &JiraImportOutput) -> &'static str {
        if out.existed { "exists" } else { "imported" }
    }

    fn node_output(&self, out: JiraImportOutput) -> Value {
        let quiet = out.existed;
        let mut v = serde_json::to_value(out).unwrap_or_default();
        if quiet {
            v["quiet"] = json!(true);
        }
        v
    }
}

// ---- jira.create ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraCreateConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The Jira project's key (`ABC`).
    pub project: String,
    /// Default `Task`; `Epic` makes an epic (its Epic Name is `epicName`, else the summary).
    #[serde(default = "task")]
    pub issue_type: String,
    pub summary: String,
    /// Text (sent as Jira rich text).
    #[serde(default)]
    #[schemars(extend("x-format" = "multiline"))]
    pub description: String,
    /// Who it is assigned to: `me`, a username or account id, an email or a name.
    #[serde(default)]
    pub assignee: Option<String>,
    /// A priority's name (`High`).
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Components' names.
    #[serde(default)]
    pub components: Vec<String>,
    /// The epic it goes in (`ABC-10`).
    #[serde(default)]
    pub epic: Option<String>,
    /// An epic's Epic Name (default its summary).
    #[serde(default)]
    pub epic_name: Option<String>,
    /// More fields: by id (`customfield_10016`), by a name the connection maps, or by their name in Jira, with plain
    /// values (`{"Story Points": 3, "Severity": "Sev 2"}`) or Jira's own form.
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub fields: Map<String, Value>,
}

fn task() -> String {
    "Task".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueOutput {
    pub key: String,
    pub url: String,
}

pub struct JiraCreate;

impl Action for JiraCreate {
    type Input = JiraCreateConfig;
    type Output = JiraIssueOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_create", "Create Jira issues and epics").asks().open_world().pm(),
            "Create a Jira issue",
            "Creates an issue (or an epic: `issueType` Epic) in a Jira project with its type, description, assignee, \
             priority, labels, components and epic, and any other field by id, mapped name or name in Jira. Fields \
             the create screen requires and the call leaves out are named in the error.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.create",
            "Create a Jira issue",
            "Creates an issue in a Jira project; its key is `{{steps.<id>.key}}`.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    /// What it does can't be told apart from doing it twice (PDT-131): cut short, it isn't run again on its own.
    fn rerun_safe(&self) -> bool {
        false
    }

    fn check(&self, c: &JiraCreateConfig) -> Result<(), String> {
        for (field, v) in [("connection", &c.connection), ("project", &c.project), ("summary", &c.summary)] {
            if v.trim().is_empty() {
                return Err(format!("{field} is empty"));
            }
        }
        if let Some(e) = nonblank(&c.epic) {
            key_ok(e)?;
        }
        Ok(())
    }

    fn approval_title(&self, c: &JiraCreateConfig) -> String {
        format!("Jira ({}): create a {} in {}: {}", c.connection, c.issue_type, c.project, short(&c.summary))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraCreateConfig) -> BoxFut<'a, Result<JiraIssueOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let project = c.project.trim().to_string();
            let screen = client.createmeta(&project, &c.issue_type).await?;
            let on_screen = |id: &str| screen.as_ref().is_none_or(|m| m.iter().any(|f| f.id == id));
            let mut given = c.fields.clone();
            if let Some(p) = nonblank(&c.priority) {
                given.insert("priority".into(), json!(p));
            }
            if !c.labels.is_empty() {
                given.insert("labels".into(), json!(c.labels));
            }
            if !c.components.is_empty() {
                given.insert("components".into(), json!(c.components));
            }
            let is_epic = c.issue_type.trim().eq_ignore_ascii_case("epic");
            let epic = nonblank(&c.epic).map(the_key).transpose()?;
            let mut via_agile = None;
            if is_epic || epic.is_some() {
                let sp = client.special().await?;
                if is_epic && let Some(id) = sp.epic_name.as_deref().filter(|id| on_screen(id)) {
                    given.insert(id.into(), json!(nonblank(&c.epic_name).unwrap_or(c.summary.trim())));
                }
                if let Some(e) = &epic {
                    match (sp.epic_link.as_deref().filter(|id| on_screen(id)), client.deployment) {
                        (Some(id), _) => _ = given.insert(id.into(), json!(e)),
                        (None, Deployment::Cloud) => _ = given.insert("parent".into(), json!({"key": e})),
                        (None, Deployment::Server) => via_agile = Some(e.clone()),
                    }
                }
            }
            let mut fields = shape_fields(&client, &given, screen.as_deref())?;
            if let Some(a) = nonblank(&c.assignee) {
                fields.insert("assignee".into(), client.deployment.user(&client.user(a).await?));
            }
            fields.insert("project".into(), json!({"key": project}));
            fields.insert("issuetype".into(), json!({"name": c.issue_type.trim()}));
            fields.insert("summary".into(), json!(c.summary.trim()));
            if !c.description.trim().is_empty() {
                fields.insert("description".into(), client.deployment.rich_text(&c.description));
            }
            if let Some(m) = &screen {
                let missing = meta::missing_required(m, &fields);
                if !missing.is_empty() {
                    return Err(format!(
                        "Jira needs more to create a {} in {project}: {}. Give them in `fields` (by name or id).",
                        c.issue_type,
                        meta::describe_missing(&missing)
                    ));
                }
            }
            let key = client.create(fields).await?;
            if let Some(e) = via_agile {
                client.agile_epic(Some(&e), std::slice::from_ref(&key)).await?;
            }
            Ok(JiraIssueOutput { url: client.browse(&key), key })
        })
    }
}

// ---- jira.edit ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraEditConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The issue (`ABC-123`).
    pub key: String,
    #[serde(default)]
    pub summary: Option<String>,
    /// Text (sent as Jira rich text).
    #[serde(default)]
    #[schemars(extend("x-format" = "multiline"))]
    pub description: Option<String>,
    /// Who it is assigned to: `me`, a username or account id, an email or a name; `none` unassigns it.
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    /// Its labels, all of them.
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// Its components' names, all of them.
    #[serde(default)]
    pub components: Option<Vec<String>>,
    /// More fields: by id, by a name the connection maps, or by their name in Jira; plain values or Jira's form.
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub fields: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraEditOutput {
    pub key: String,
    pub url: String,
    /// The fields it set, by id.
    pub changed: Vec<String>,
}

pub struct Edit;

impl Action for Edit {
    type Input = JiraEditConfig;
    type Output = JiraEditOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_edit", "Edit and assign Jira issues").asks().open_world().pm(),
            "Edit a Jira issue",
            "Changes an issue's summary, description, assignee (`none` unassigns), priority, labels, components, \
             and any other field by id, mapped name or name in Jira. Values are matched to what each field takes \
             (numbers, options, users) from the issue's edit screen.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.edit",
            "Edit a Jira issue",
            "Changes an issue's fields or assignee; `{{steps.<id>.changed}}` lists the fields set.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraEditConfig) -> Result<(), String> {
        key_ok(&c.key)?;
        let nothing = c.summary.is_none()
            && c.description.is_none()
            && c.assignee.is_none()
            && c.priority.is_none()
            && c.labels.is_none()
            && c.components.is_none()
            && c.fields.is_empty();
        if nothing { Err("nothing to change: give a field".into()) } else { Ok(()) }
    }

    fn approval_title(&self, c: &JiraEditConfig) -> String {
        let mut what: Vec<String> = vec![];
        if let Some(s) = &c.summary {
            what.push(format!("summary={}", short(s)));
        }
        if c.description.is_some() {
            what.push("description".into());
        }
        if let Some(a) = &c.assignee {
            what.push(format!("assignee={a}"));
        }
        if let Some(p) = &c.priority {
            what.push(format!("priority={p}"));
        }
        if let Some(l) = &c.labels {
            what.push(format!("labels={}", l.join(",")));
        }
        if let Some(l) = &c.components {
            what.push(format!("components={}", l.join(",")));
        }
        if !c.fields.is_empty() {
            what.push(fields_list(&c.fields));
        }
        format!("Jira ({}): edit {}: {}", c.connection, c.key, what.join(", "))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraEditConfig) -> BoxFut<'a, Result<JiraEditOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let key = the_key(&c.key)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let mut given = c.fields.clone();
            if let Some(s) = &c.summary {
                given.insert("summary".into(), json!(s.trim()));
            }
            if let Some(d) = &c.description {
                given.insert("description".into(), json!(d));
            }
            if let Some(p) = &c.priority {
                given.insert("priority".into(), json!(p.trim()));
            }
            if let Some(l) = &c.labels {
                given.insert("labels".into(), json!(l));
            }
            if let Some(l) = &c.components {
                given.insert("components".into(), json!(l));
            }
            let mut changed = vec![];
            if !given.is_empty() {
                let screen = client.editmeta(&key).await?;
                let fields = shape_fields(&client, &given, Some(&screen))?;
                changed.extend(fields.keys().cloned());
                client.edit(&key, fields).await?;
            }
            if let Some(a) = c.assignee.as_deref().map(str::trim) {
                let who = match a {
                    "" | "none" | "nobody" | "unassigned" => None,
                    a => Some(client.user(a).await?),
                };
                client.assign(&key, who.as_deref()).await?;
                changed.push("assignee".into());
            }
            Ok(JiraEditOutput { url: client.browse(&key), key, changed })
        })
    }
}

// ---- jira.comment ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraCommentConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    pub key: String,
    /// Text (sent as Jira rich text: a paragraph per blank-line-separated block).
    #[schemars(extend("x-format" = "multiline"))]
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraCommentOutput {
    pub key: String,
    pub comment_id: String,
}

pub struct JiraComment;

impl Action for JiraComment {
    type Input = JiraCommentConfig;
    type Output = JiraCommentOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_comment", "Comment on Jira issues").asks().open_world().pm(),
            "Comment on a Jira issue",
            "Adds a comment (text: a paragraph per blank-line-separated block) to a Jira issue.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.comment",
            "Comment on a Jira issue",
            "Adds a comment to a Jira issue; its id is `{{steps.<id>.commentId}}`.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    /// What it does can't be told apart from doing it twice (PDT-131): cut short, it isn't run again on its own.
    fn rerun_safe(&self) -> bool {
        false
    }

    fn check(&self, c: &JiraCommentConfig) -> Result<(), String> {
        if c.connection.trim().is_empty() {
            return Err("connection is empty: name a jira connection".into());
        }
        if c.body.trim().is_empty() {
            return Err("body is empty".into());
        }
        key_ok(&c.key)
    }

    fn approval_title(&self, c: &JiraCommentConfig) -> String {
        format!("Jira ({}): comment on {}: {}", c.connection, c.key, short(&c.body))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraCommentConfig) -> BoxFut<'a, Result<JiraCommentOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let key = the_key(&c.key)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let comment_id = client.comment(&key, &c.body).await?;
            Ok(JiraCommentOutput { key, comment_id })
        })
    }
}

// ---- jira.transitions ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraTransitionsOutput {
    pub key: String,
    pub status: String,
    /// The moves open to it now, each with the fields its screen has (`required` ones must be given).
    pub transitions: Vec<Transition>,
}

pub struct Transitions;

impl Action for Transitions {
    type Input = JiraIssueConfig;
    type Output = JiraTransitionsOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_transitions", "See how a Jira issue can move").read_only().open_world().pm(),
            "See a Jira issue's transitions",
            "The transitions open to an issue now (id, name, the status each leads to) and the fields each one's \
             screen has: which are required, their type, and the values a choice allows.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.transitions",
            "Jira issue's transitions",
            "The moves open to an issue now and the fields each needs: `{{steps.<id>.transitions}}`.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraIssueConfig) -> Result<(), String> {
        key_ok(&c.key)
    }

    fn approval_title(&self, c: &JiraIssueConfig) -> String {
        format!("read {}'s transitions in Jira ({})", c.key, c.connection)
    }

    fn run<'a>(
        &'a self,
        cx: &'a ActionCx<'a>,
        c: JiraIssueConfig,
    ) -> BoxFut<'a, Result<JiraTransitionsOutput, String>> {
        Box::pin(async move {
            let key = the_key(&c.key)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let issue = client.issue(&key).await?.ok_or_else(|| format!("Jira has no issue {key}"))?;
            Ok(JiraTransitionsOutput { status: issue.status, transitions: client.transitions(&key).await?, key })
        })
    }
}

// ---- jira.transition ----

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraTransitionConfig {
    /// A `jira` connection (with `key`).
    #[serde(default)]
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: Option<String>,
    /// The issue (`ABC-123`).
    #[serde(default)]
    pub key: Option<String>,
    /// Or a ticket (`{{trigger.ticketId}}`): its Jira issue, through the connection it was linked with.
    #[serde(default)]
    pub ticket: Option<String>,
    /// A transition's id or name, or the status it leads to; with `ticket`, default its project's done mapping.
    #[serde(default)]
    pub to: Option<String>,
    /// The fields the transition's screen asks for, sent with it: by id, mapped name or name in Jira, with plain
    /// values (`{"Resolution": "Fixed", "Story Points": 3}`) or Jira's form.
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub fields: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraTransitionOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub url: Option<String>,
    /// The transition it took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub transition: Option<String>,
    /// The issue's status now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<String>,
    /// Where it was asked to go, when it couldn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<String>,
    /// The transitions open to it, when none was `to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub available: Option<Vec<String>>,
    /// Why it couldn't move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    /// Nothing to tell the user: it was there already, or can't go there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub quiet: Option<bool>,
    /// It could not move (the `unavailable` port).
    #[serde(skip)]
    #[schemars(skip)]
    pub unavailable: bool,
}

pub struct JiraTransition;

/// Where a transition goes: the connection, the issue and where to.
async fn transition_target(
    cx: &ActionCx<'_>,
    c: &JiraTransitionConfig,
) -> Result<Result<(String, String, String), JiraTransitionOutput>, String> {
    Ok(Ok(match nonblank(&c.ticket) {
        Some(ticket) => {
            let links = cx.st.store.external_links(ticket, JIRA).await.map_err(|e| e.to_string())?;
            let Some(link) = links.into_iter().next() else {
                return Ok(Err(JiraTransitionOutput {
                    reason: Some("the ticket is not linked to a Jira issue".into()),
                    quiet: Some(true),
                    unavailable: true,
                    ..Default::default()
                }));
            };
            let conn = cx
                .st
                .store
                .get_connection(&link.connection_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("the Jira connection the ticket was linked with is gone")?;
            let to = match nonblank(&c.to) {
                Some(t) => t.to_string(),
                None => {
                    let summary = cx.st.store.get_ticket_summary(ticket).await.map_err(|e| e.to_string())?;
                    let project = summary.map(|t| t.project_id).unwrap_or_default();
                    let s = cx.st.store.get_project_settings(&project).await.map_err(|e| e.to_string())?;
                    s.integration::<JiraSettings>(JIRA).map(|j| j.done).unwrap_or_else(|| DEFAULT_DONE.into())
                }
            };
            (conn.name, link.key, to)
        }
        None => (
            nonblank(&c.connection).unwrap_or_default().to_string(),
            issue_key(nonblank(&c.key).unwrap_or_default()).ok_or("key is not an issue key")?,
            nonblank(&c.to).unwrap_or_default().to_string(),
        ),
    }))
}

impl Action for JiraTransition {
    type Input = JiraTransitionConfig;
    type Output = JiraTransitionOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_transition", "Move Jira issues through their workflow").asks().open_world().pm(),
            "Move a Jira issue",
            "Moves an issue (by key, or a ticket's linked issue) through the transition `to` (an id, a name, or the \
             status it leads to), with the fields its screen asks for in the same request (`fields`: numbers, \
             choices by name). Required fields left out are named in the error; jira_transitions lists them first.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.transition",
            "Move a Jira issue",
            "Moves a Jira issue (by key, or a ticket's linked issue) through the transition named `to`, or the one \
             leading to that status, with the fields its screen needs (`fields`). An issue already there counts as \
             done; no such transition from its status leaves by `unavailable`; a required field left out fails the \
             step, naming it.",
            &[("out", "Then"), ("unavailable", "Can't move there")],
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraTransitionConfig) -> Result<(), String> {
        match (nonblank(&c.key), nonblank(&c.ticket)) {
            (Some(k), None) => {
                if nonblank(&c.connection).is_none() {
                    return Err("with key, name the jira connection".into());
                }
                if nonblank(&c.to).is_none() {
                    return Err("to is empty: name a transition or status".into());
                }
                key_ok(k)
            }
            (None, Some(_)) => Ok(()),
            _ => Err("give either key (with connection) or ticket".into()),
        }
    }

    fn approval_title(&self, c: &JiraTransitionConfig) -> String {
        let what =
            nonblank(&c.key).map(String::from).or_else(|| nonblank(&c.ticket).map(|t| format!("ticket {t}'s issue")));
        let mut s = format!(
            "Jira: move {} to {}",
            what.unwrap_or_else(|| "the issue".into()),
            nonblank(&c.to).unwrap_or("its done status")
        );
        if !c.fields.is_empty() {
            s.push_str(&format!(" with {}", fields_list(&c.fields)));
        }
        s
    }

    fn run<'a>(
        &'a self,
        cx: &'a ActionCx<'a>,
        c: JiraTransitionConfig,
    ) -> BoxFut<'a, Result<JiraTransitionOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let (connection, key, to) = match transition_target(cx, &c).await? {
                Ok(t) => t,
                Err(out) => return Ok(out),
            };
            let client = JiraClient::named(cx.st, &connection).await?;
            let url = client.browse(&key);
            let issue = client.issue(&key).await?.ok_or_else(|| format!("Jira has no issue {key}"))?;
            if issue.status.eq_ignore_ascii_case(&to) {
                return Ok(JiraTransitionOutput {
                    key: Some(key),
                    url: Some(url),
                    status: Some(issue.status),
                    quiet: Some(true),
                    ..Default::default()
                });
            }
            let list = client.transitions(&key).await?;
            let Some(t) = meta::pick_transition(&list, &to) else {
                return Ok(JiraTransitionOutput {
                    available: Some(list.iter().map(|t| t.name.clone()).collect()),
                    key: Some(key),
                    url: Some(url),
                    status: Some(issue.status),
                    to: Some(to),
                    quiet: Some(true),
                    unavailable: true,
                    ..Default::default()
                });
            };
            let fields = meta::coerce_fields(&c.fields, &client.mappings, &t.fields, client.deployment)?;
            let mut missing: Vec<MissingField> = meta::missing_required(&t.fields, &fields);
            // Jira marks a field required whenever its configuration does, even when the issue has a value already,
            // and takes the transition without it then: only the ones still empty are missing.
            if !missing.is_empty() {
                let ids: Vec<String> = missing.iter().map(|m| m.id.clone()).collect();
                let now = client.issue_raw(&key, &ids).await?.unwrap_or(Value::Null);
                missing.retain(|m| !meta::has_value(&now["fields"][&m.id]));
            }
            if !missing.is_empty() {
                return Err(format!(
                    "{key}: the transition {} needs {}. Give them in `fields` (by name or id) and try again.",
                    t.name,
                    meta::describe_missing(&missing)
                ));
            }
            client.do_transition(&key, &t.id, fields).await?;
            Ok(JiraTransitionOutput {
                key: Some(key),
                url: Some(url),
                transition: Some(t.name.clone()),
                status: Some(t.to.clone()),
                ..Default::default()
            })
        })
    }

    fn port(&self, out: &JiraTransitionOutput) -> &'static str {
        if out.unavailable { "unavailable" } else { "out" }
    }
}

// ---- jira.boards ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraBoardsConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// Only a project's boards (its key).
    #[serde(default)]
    pub project: Option<String>,
    /// Only boards whose name has these words.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraBoardInfo {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    /// `scrum` or `kanban`.
    pub r#type: String,
    /// Its project's key.
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraBoardsOutput {
    pub boards: Vec<JiraBoardInfo>,
}

pub struct Boards;

impl Action for Boards {
    type Input = JiraBoardsConfig;
    type Output = JiraBoardsOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_boards", "List Jira boards").read_only().open_world().pm(),
            "List Jira boards",
            "Jira's boards (a project's, or by name): each one's id, name, type and project.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.boards",
            "Jira boards",
            "Jira's boards: `{{steps.<id>.boards}}` is a list of {id, name, type, project}.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn approval_title(&self, c: &JiraBoardsConfig) -> String {
        format!("list Jira boards ({})", c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraBoardsConfig) -> BoxFut<'a, Result<JiraBoardsOutput, String>> {
        Box::pin(async move {
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let list = client.boards(nonblank(&c.project), nonblank(&c.name)).await?;
            Ok(JiraBoardsOutput {
                boards: list
                    .iter()
                    .map(|b| JiraBoardInfo {
                        id: b["id"].as_i64().unwrap_or_default(),
                        name: b["name"].as_str().unwrap_or_default().to_string(),
                        r#type: b["type"].as_str().unwrap_or_default().to_string(),
                        project: b["location"]["projectKey"].as_str().map(String::from),
                    })
                    .collect(),
            })
        })
    }
}

// ---- jira.sprints ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraSprintsConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The board: its id or name.
    pub board: String,
    /// The sprints' states, comma-separated (default `active,future`; `closed` too for past ones).
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraSprintsOutput {
    #[ts(type = "number")]
    pub board: i64,
    pub sprints: Vec<Sprint>,
    /// The active sprints the team works in (backlog sections left out).
    pub active: Vec<Sprint>,
    /// The backlog sections (sprints the connection names as sections).
    pub sections: Vec<Sprint>,
}

pub struct Sprints;

impl Action for Sprints {
    type Input = JiraSprintsConfig;
    type Output = JiraSprintsOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_sprints", "List a Jira board's sprints").read_only().open_world().pm(),
            "List a board's sprints",
            "A board's sprints (active and planned unless `state` says): each one's id, name and state, which are \
             the active ones, and which are backlog sections rather than sprints.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.sprints",
            "Jira sprints",
            "A board's sprints: `{{steps.<id>.active}}` is the active ones (backlog sections left out), \
             `{{steps.<id>.sections}}` the backlog sections.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraSprintsConfig) -> Result<(), String> {
        if c.board.trim().is_empty() { Err("board is empty: give its id or name".into()) } else { Ok(()) }
    }

    fn approval_title(&self, c: &JiraSprintsConfig) -> String {
        format!("list the sprints of board {} ({})", c.board, c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraSprintsConfig) -> BoxFut<'a, Result<JiraSprintsOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let board = client.board_id(&c.board).await?;
            let sprints = client.sprints(board, nonblank(&c.state).unwrap_or("active,future")).await?;
            let active = sprints.iter().filter(|s| s.state == "active" && !s.backlog_section).cloned().collect();
            let sections = sprints.iter().filter(|s| s.backlog_section).cloned().collect();
            Ok(JiraSprintsOutput { board, sprints, active, sections })
        })
    }
}

// ---- jira.board ----

/// How many of a board's issues are read unless told.
const BOARD_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraBoardConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The board: its id or name.
    pub board: String,
    /// Only issues this JQL finds too (`assignee = currentUser()`).
    #[serde(default)]
    pub jql: Option<String>,
    /// At most this many issues (default 100, at most 500).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraBoardOutput {
    #[ts(type = "number")]
    pub board: i64,
    /// Active sprints first (by status), then backlog sections, planned sprints and the backlog.
    pub issues: Vec<JiraIssueSummary>,
    /// The board's issues (that the JQL finds), also those past `limit`.
    #[ts(type = "number")]
    pub total: i64,
}

pub struct Board;

impl Action for Board {
    type Input = JiraBoardConfig;
    type Output = JiraBoardOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_board", "See where issues sit on a Jira board").read_only().open_world().pm(),
            "See a Jira board",
            "A board's issues and where each one sits (`placement`): in an active sprint in its status's column, in \
             a backlog section, planned for a sprint, or in the backlog.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.board",
            "Jira board",
            "A board's issues with where each sits: `{{steps.<id>.issues}}`, each with `placement` {area: sprint, \
             section, future or backlog; sprint; status}.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraBoardConfig) -> Result<(), String> {
        if c.board.trim().is_empty() { Err("board is empty: give its id or name".into()) } else { Ok(()) }
    }

    fn approval_title(&self, c: &JiraBoardConfig) -> String {
        format!("read board {} ({})", c.board, c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraBoardConfig) -> BoxFut<'a, Result<JiraBoardOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let sp = client.special().await?;
            let board = client.board_id(&c.board).await?;
            let mut fields = list_fields(&sp);
            fields.push("sprint".into());
            let limit = c.limit.unwrap_or(BOARD_LIMIT).clamp(1, 500);
            let mut raw = vec![];
            let mut total;
            loop {
                let (page, t) = client
                    .board_issues(board, nonblank(&c.jql), &fields, raw.len(), (limit - raw.len()).min(100))
                    .await?;
                total = t;
                let n = page.len();
                raw.extend(page);
                if n == 0 || raw.len() >= limit || raw.len() as i64 >= total {
                    break;
                }
            }
            let mut issues = summaries(cx, &client, &sp, &raw).await?;
            let order = |p: &Placement| {
                ["sprint", "section", "future", "backlog"].iter().position(|a| *a == p.area).unwrap_or(4)
            };
            issues.sort_by_key(|i| order(&i.placement));
            Ok(JiraBoardOutput { board, issues, total })
        })
    }
}

// ---- jira.sprint ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraSprintConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The issues (`ABC-1`), up to 50.
    pub keys: Vec<String>,
    /// The sprint (or backlog section) to move them into: `active` (the board's one active sprint), a name, or an id.
    #[serde(default)]
    pub sprint: Option<String>,
    /// The board whose sprint `sprint` names (its id or name); not needed for a sprint's id.
    #[serde(default)]
    pub board: Option<String>,
    /// Move them out of their sprints, back to the backlog.
    #[serde(default)]
    pub backlog: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraSprintOutput {
    pub keys: Vec<String>,
    /// The sprint they are in now (None: the backlog).
    pub sprint: Option<Sprint>,
}

pub struct MoveToSprint;

/// The sprint a call names, on the board it names.
async fn find_sprint(client: &JiraClient, given: &str, board: Option<&str>) -> Result<Sprint, String> {
    let g = given.trim();
    if let Ok(id) = g.parse::<i64>() {
        return Ok(Sprint { id, name: g.to_string(), state: String::new(), backlog_section: false });
    }
    let board = board.ok_or_else(|| format!("name the board whose sprint {g:?} is (board)"))?;
    let all = client.sprints(client.board_id(board).await?, "active,future").await?;
    let names = |l: &[&Sprint]| l.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ");
    if g.eq_ignore_ascii_case("active") {
        let active: Vec<&Sprint> = all.iter().filter(|s| s.state == "active" && !s.backlog_section).collect();
        return match active.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(format!("board {board} has no active sprint")),
            many => Err(format!("board {board} has {} active sprints ({}); name one", many.len(), names(many))),
        };
    }
    all.iter().find(|s| s.name.eq_ignore_ascii_case(g)).cloned().ok_or_else(|| {
        let open: Vec<&Sprint> = all.iter().collect();
        format!("board {board} has no open sprint {g:?} (it has: {})", names(&open))
    })
}

impl Action for MoveToSprint {
    type Input = JiraSprintConfig;
    type Output = JiraSprintOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_sprint", "Move Jira issues between sprints and the backlog").asks().open_world().pm(),
            "Move Jira issues to a sprint or the backlog",
            "Moves issues into a sprint or backlog section (`sprint`: `active`, a name on `board`, or an id), or out \
             of their sprints to the backlog (`backlog`: true).",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.sprint",
            "Move Jira issues to a sprint",
            "Moves issues into a sprint (the board's active one, one by name or id, or a backlog section) or back to \
             the backlog.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraSprintConfig) -> Result<(), String> {
        keys_ok(&c.keys)?;
        match (nonblank(&c.sprint), c.backlog) {
            (Some(_), true) => Err("give sprint or backlog, not both".into()),
            (None, false) => Err("give the sprint to move them into, or backlog: true".into()),
            _ => Ok(()),
        }
    }

    fn approval_title(&self, c: &JiraSprintConfig) -> String {
        let to = match nonblank(&c.sprint) {
            Some(s) => format!("sprint {s}"),
            None => "the backlog".into(),
        };
        format!("Jira ({}): move {} to {to}", c.connection, c.keys.join(", "))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraSprintConfig) -> BoxFut<'a, Result<JiraSprintOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let keys = the_keys(&c.keys)?;
            if keys.len() > 50 {
                return Err("at most 50 issues at a time".into());
            }
            let client = JiraClient::named(cx.st, &c.connection).await?;
            match nonblank(&c.sprint) {
                Some(s) => {
                    let sprint = find_sprint(&client, s, nonblank(&c.board)).await?;
                    client.to_sprint(sprint.id, &keys).await?;
                    Ok(JiraSprintOutput { keys, sprint: Some(sprint) })
                }
                None => {
                    client.to_backlog(&keys).await?;
                    Ok(JiraSprintOutput { keys, sprint: None })
                }
            }
        })
    }
}

// ---- jira.epic ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraEpicConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The issues (`ABC-1`).
    pub keys: Vec<String>,
    /// The epic to put them in (`ABC-10`); empty takes them out of their epic.
    #[serde(default)]
    pub epic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraEpicOutput {
    pub keys: Vec<String>,
    /// Their epic now (None: none).
    pub epic: Option<String>,
}

pub struct SetEpic;

impl Action for SetEpic {
    type Input = JiraEpicConfig;
    type Output = JiraEpicOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_epic", "Put Jira issues in an epic or take them out").asks().open_world().pm(),
            "Set Jira issues' epic",
            "Puts issues in an epic (`epic`: its key), or takes them out of theirs (no `epic`), through the site's \
             Epic Link field (the connection's mapping, else found by its type) or, on Cloud without one, the \
             issue's parent.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.epic",
            "Set Jira issues' epic",
            "Puts issues in an epic, or takes them out of theirs (`epic` empty).",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraEpicConfig) -> Result<(), String> {
        keys_ok(&c.keys)?;
        nonblank(&c.epic).map_or(Ok(()), key_ok)
    }

    fn approval_title(&self, c: &JiraEpicConfig) -> String {
        match nonblank(&c.epic) {
            Some(e) => format!("Jira ({}): put {} in epic {e}", c.connection, c.keys.join(", ")),
            None => format!("Jira ({}): take {} out of their epic", c.connection, c.keys.join(", ")),
        }
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraEpicConfig) -> BoxFut<'a, Result<JiraEpicOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let keys = the_keys(&c.keys)?;
            let epic = nonblank(&c.epic).map(the_key).transpose()?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let sp = client.special().await?;
            match (sp.epic_link, client.deployment) {
                (Some(field), _) => {
                    for k in &keys {
                        let mut f = Map::new();
                        f.insert(field.clone(), json!(epic));
                        client.edit(k, f).await?;
                    }
                }
                (None, Deployment::Cloud) => {
                    for k in &keys {
                        let mut f = Map::new();
                        f.insert("parent".into(), epic.as_ref().map_or(Value::Null, |e| json!({"key": e})));
                        client.edit(k, f).await?;
                    }
                }
                (None, Deployment::Server) => client.agile_epic(epic.as_deref(), &keys).await?,
            }
            Ok(JiraEpicOutput { keys, epic })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_titles() {
        assert_eq!(the_keys(&["abc-1, ABC-2".into(), "x-3".into()]).unwrap(), ["ABC-1", "ABC-2", "X-3"]);
        assert!(the_keys(&[" ".into()]).is_err());
        assert!(keys_ok(&["{{steps.s.keys}}".into()]).is_ok());
        let t = approval_title(
            "jira_transition",
            &json!({"key": "ABC-1", "to": "Done", "fields": {"Resolution": "Fixed", "Story Points": 3}}),
        )
        .unwrap();
        assert_eq!(t, "Jira: move ABC-1 to Done with Resolution=Fixed, Story Points=3");
        let t = approval_title("jira_sprint", &json!({"keys": ["ABC-1"], "backlog": true})).unwrap();
        assert_eq!(t, "Jira (Jira): move ABC-1 to the backlog");
        assert!(approval_title("jira_search", &json!({"jql": "x"})).is_none());
    }
}
