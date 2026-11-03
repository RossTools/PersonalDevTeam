//! pdt-mcp's tools. Inputs are the API's request types where one exists (their JSON Schema from
//! schemars), and each tool runs the function the matching route runs, so the API and the tools can't drift.

mod inbox;
mod issue_links;
mod notes;
mod overview;
mod secrets;
mod sessions;
mod step;
mod tickets;
mod update_ticket;
mod watches;
mod workflows;

pub(crate) use step::{SubmitOutcome, add_review_comment};

use crate::error::ApiError;
use crate::integrations::Tool;
use crate::routes::{AppState, now_ms};
use crate::workflow::agent::Confirmed;
use crate::workflow::catalog::Category;
use axum::Json;
use axum::extract::State;
use pdt_core::inbox::Question;
use pdt_core::integration::ToolDecl;
use pdt_core::mcp::input_schema;
use pdt_core::pm::AskTarget;
use pdt_core::review::{CommentSide, NewReviewComment};
use pdt_core::session::Session;
use pdt_core::ticket::{
    AGENTIC_LABEL, Actor, AgentBy, By, EventKind, TicketEvent, TicketSource, TicketStage, valid_comment,
};
use pdt_core::workflow::agent::OutcomeComment;
use pdt_core::workflow::api::{Author, Scope};
use pdt_core::workflow::doc::WorkflowDoc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// What a tool call knows: PDT, the calling session and the call's arguments.
pub struct Cx<'a> {
    pub st: &'a AppState,
    pub session: &'a Session,
    /// The arguments as the agent gave them: an asking tool's Inbox item shows them (PDT-256).
    pub input: &'a Value,
    /// The user allowed this very call in the Inbox: an asking tool goes ahead without asking again.
    pub confirmed: bool,
}

/// What a call PDT makes itself (not an agent's tool call) has as arguments.
pub(crate) static NO_INPUT: Value = Value::Null;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

pub(crate) fn schema<T: JsonSchema>() -> Value {
    input_schema(crate::workflow::catalog::schema_of::<T>())
}

pub(crate) fn args<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    let v = if v.is_null() { json!({}) } else { v };
    serde_json::from_value(v).map_err(|e| format!("the arguments don't fit the tool's schema: {e}"))
}

pub(crate) fn api(e: ApiError) -> String {
    e.message().to_string()
}

fn store(e: pdt_store::StoreError) -> String {
    ApiError::from(e).message().to_string()
}

/// Areas an agent gives a ticket of `project_id`, checked against the project's configured areas (PDT-329).
async fn checked_areas(
    st: &AppState,
    project_id: &str,
    areas: Option<Vec<String>>,
) -> Result<Option<Vec<String>>, String> {
    let Some(areas) = areas else { return Ok(None) };
    let project = st.store.get_project(project_id).await.map_err(store)?.ok_or("no such project")?;
    let configured: Vec<String> =
        crate::overlap::project_areas(st, &project).await.areas.into_iter().map(|a| a.name).collect();
    pdt_core::ticket::agent_areas(&areas, &configured).map(Some)
}

/// PDT's own tools, in `tools/list` order ([`crate::integrations::core`]): one line per group.
pub(crate) fn defs() -> Vec<Tool> {
    [
        step::defs(),
        secrets::defs(),
        update_ticket::defs(),
        workflows::defs(),
        overview::defs(),
        notes::defs(),
        watches::defs(),
        tickets::defs(),
        issue_links::defs(),
        sessions::defs(),
        inbox::defs(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Runs one of PDT's own tools, in the group that lists it. Err is a message for the agent.
pub(crate) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    // No secrets near untrusted input (PDT-201).
    if matches!(name, "request_secret" | "run_with_secrets" | "write_secret_file") {
        crate::untrusted::refuse_secrets(st, cx.session).await?;
    }
    let listed = |defs: fn() -> Vec<Tool>| defs().iter().any(|t| t.decl.name == name);
    if listed(step::defs) {
        step::call(cx, name, a).await
    } else if listed(secrets::defs) {
        secrets::call(cx, name, a).await
    } else if listed(update_ticket::defs) {
        update_ticket::call(cx, name, a).await
    } else if listed(workflows::defs) {
        workflows::call(cx, name, a).await
    } else if listed(overview::defs) {
        overview::call(cx, name, a).await
    } else if listed(notes::defs) {
        notes::call(cx, name, a).await
    } else if listed(watches::defs) {
        watches::call(cx, name, a).await
    } else if listed(tickets::defs) {
        tickets::call(cx, name, a).await
    } else if listed(issue_links::defs) {
        issue_links::call(cx, name, a).await
    } else if listed(sessions::defs) {
        sessions::call(cx, name, a).await
    } else if listed(inbox::defs) {
        inbox::call(cx, name, a).await
    } else {
        Err(format!("no tool is called {name}"))
    }
}

/// An asking tool from an agent that can't ask the user per call (Codex, Devin: PDT's tools run without asking
/// there) runs where the project lets the PM act; elsewhere PDT asks the user (PDT-256). Claude asks through its
/// permission prompt, unless it runs in Bypass, which never shows that prompt (PDT-414): then it is one that can't ask.
/// A public-posting tool asks outside a workflow step, which expects it (PDT-414).
///
/// In a conversation PDT asks in the conversation itself, with an approval card that is also in the Inbox, and the call
/// waits for the answer (PDT-490). Elsewhere (a terminal), and for a tool that asks in the Inbox (PDT-475), it asks with
/// an Inbox item, and the call waits on that instead (PDT-492): the agent never ends its turn to wait.
pub(crate) async fn confirmable(cx: &Cx<'_>, tool: &str, target: &pdt_core::pm::AskTarget) -> Result<(), String> {
    use pdt_core::session::AgentKind;
    let reg = cx.st.registry();
    let table = reg.tool_table();
    if cx.confirmed {
        return Ok(());
    }
    let in_inbox = table.asks_in_inbox(tool);
    if !table.asks(tool) {
        let in_step = cx.st.store.active_agent_step(&cx.session.id).await.ok().flatten().is_some();
        if !table.public(tool) || in_step {
            return Ok(());
        }
    } else if !in_inbox {
        // One that asks in PDT's Inbox (PDT-475) asks there for every agent, whatever the project lets the PM do.
        let claude_prompts =
            cx.session.agent.unwrap_or(AgentKind::Claude) == AgentKind::Claude && !cx.session.asks_nothing();
        if claude_prompts || crate::pm::acts_without_asking(cx.st, target).await {
            return Ok(());
        }
    }
    if !in_inbox {
        // Letting an external ticket in takes the user's yes to this very call (PDT-201): never "always".
        let each_time = crate::untrusted::needs_the_user(cx.st, cx.session, tool, cx.input).await;
        if !each_time && crate::conversations::allowed_always(cx.st, &cx.session.id, tool) {
            return Ok(());
        }
        use crate::conversations::Asked;
        match crate::conversations::ask_inline(cx.st, cx.session, tool, cx.input, !each_time).await {
            Some(Asked::Allowed) => return Ok(()),
            Some(Asked::Denied(why)) => {
                let why = why.map(|w| format!(" They said: {w}")).unwrap_or_default();
                return Err(format!("The user denied this {tool} call, so PDT didn't make it.{why}"));
            }
            Some(Asked::Ended) => {
                return Err(format!("{tool} was not run: the turn ended before the user answered."));
            }
            None => {}
        }
    }
    let item = crate::workflow::agent::ask_confirm(cx.st, cx.session, tool, cx.input).await?;
    crate::notifications::kick(cx.st);
    let waited = crate::workflow::agent::wait_confirm(cx.st, &item).await;
    crate::notifications::kick(cx.st);
    match waited {
        Confirmed::Allowed(_) => {
            crate::workflow::agent::grant(cx.st, &cx.session.id, &item.id, tool, cx.input);
            Ok(())
        }
        Confirmed::Denied => Err(format!("The user denied this {tool} call in the Inbox, so PDT didn't make it.")),
        Confirmed::Ended => Err(format!("{tool} was not run: its Inbox item closed before the user answered.")),
    }
}

/// A ticket's id from its id or key (`ABC-12`; a bare number is in `project`).
pub(crate) async fn resolve_ticket(st: &AppState, given: &str, project: Option<&str>) -> Result<String, String> {
    let g = given.trim();
    if st.store.get_ticket(g).await.map_err(store)?.is_some() {
        return Ok(g.to_string());
    }
    let k = pdt_core::ticket::parse_key(g).ok_or_else(|| format!("{g} is not a ticket key or id"))?;
    let project = match &k.key {
        Some(key) => st
            .store
            .list_projects()
            .await
            .map_err(store)?
            .into_iter()
            .find(|p| p.key.eq_ignore_ascii_case(key))
            .map(|p| p.id)
            .ok_or_else(|| format!("no project has the key {key}"))?,
        None => project
            .ok_or_else(|| format!("{g} needs its project's key in front, such as ABC-{}", k.number))?
            .to_string(),
    };
    st.store
        .ticket_id_by_number(&project, k.number)
        .await
        .map_err(store)?
        .ok_or_else(|| format!("there is no ticket {g}"))
}

/// The ticket a tool means: `given` (a key or id), else the session's (its attempt's, or its agent step's).
async fn ticket_for(cx: &Cx<'_>, given: Option<&str>) -> Result<String, String> {
    if let Some(g) = given.map(str::trim).filter(|g| !g.is_empty()) {
        return resolve_ticket(cx.st, g, Some(&cx.session.project_id)).await;
    }
    let st = cx.st;
    session_ticket(st, cx.session)
        .await
        .ok_or_else(|| "this session works on no ticket; name one (`ticket`: a key such as ABC-12)".to_string())
}

/// Who a session's ticket changes and comments are by (PDT-163): the agent, its persona and the ticket it works on.
pub(crate) async fn agent_by(st: &AppState, s: &Session) -> By {
    let from = match session_ticket(st, s).await {
        Some(id) => st.store.get_ticket_summary(&id).await.ok().flatten(),
        None => None,
    };
    By::agent(AgentBy {
        session_id: s.id.clone(),
        project_id: Some(s.project_id.clone()),
        persona: s.persona.as_ref().map(|p| p.name.clone()),
        from_ticket: from.as_ref().map(|t| t.key.clone()),
        from_ticket_id: from.map(|t| t.id),
    })
}

/// The ticket a session works on: its attempt's, else its agent step's.
pub(crate) async fn session_ticket(st: &AppState, s: &Session) -> Option<String> {
    if let Some(t) = &s.ticket {
        return Some(t.id.clone());
    }
    st.store.active_agent_step(&s.id).await.ok().flatten().and_then(|a| a.ticket_id)
}
