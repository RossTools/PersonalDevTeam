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

mod board;
mod boards;
mod check;
mod comment;
mod create;
mod edit;
mod epic;
mod import;
mod issue;
mod search;
mod sprint;
mod sprints;
mod transition;
mod transitions;

pub use board::*;
pub use boards::*;
pub use check::*;
pub use comment::*;
pub use create::*;
pub use edit::*;
pub use epic::*;
pub use import::*;
pub use issue::*;
pub use search::*;
pub use sprint::*;
pub use sprints::*;
pub use transition::*;
pub use transitions::*;

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
