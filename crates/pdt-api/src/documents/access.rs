//! Document changes agents make without asking (PDT-618): the project's "Agents may change documents" level
//! ([`pdt_core::policy::AgentDocuments`]) lets an agent create, edit and link documents on its own ticket, or across
//! the project, with no prompt. [`unasked`] resolves what a call touches and puts it to
//! [`pdt_core::document_access`]; [`record`] keeps each such change on the document and on the agent's ticket, so the
//! user can review it and roll it back.

use crate::routes::{AppState, now_ms};
use pdt_core::document::{Document, DocumentChange};
use pdt_core::document_access::{Caller, DocumentCall, allows};
use pdt_core::policy::AgentDocuments;
use pdt_core::session::Session;
use pdt_core::ticket::{By, EventKind};
use serde_json::{Value, json};

/// What a document tool's call wants to touch, as the agent named it (keys, ids or titles).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Wants<'a> {
    Create { project: Option<&'a str>, ticket: Option<&'a str> },
    Change { document: &'a str },
    Link { document: &'a str, project: Option<&'a str>, ticket: Option<&'a str> },
}

impl<'a> Wants<'a> {
    /// A call of one of the tools the setting covers; None for the others (comment replies and resolves keep asking).
    pub(crate) fn of_call(tool: &str, input: &'a Value) -> Option<Self> {
        let field = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
        match tool {
            "document_create" => Some(Wants::Create { project: field("project"), ticket: field("ticket") }),
            "doc_edit" | "sheet_edit" => Some(Wants::Change { document: field("document")? }),
            "document_link" => {
                Some(Wants::Link { document: field("document")?, project: field("project"), ticket: field("ticket") })
            }
            _ => None,
        }
    }
}

fn given(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|v| !v.is_empty())
}

/// Whether the session's project lets it make this call without asking the user. A session that reads input from
/// outside PDT (PDT-646: triage, a run an issue or a webhook started) always asks, whatever the level: the setting is
/// for the project's own agents' work, not for text an outsider wrote steering them.
pub(crate) async fn unasked(st: &AppState, s: &Session, wants: Wants<'_>) -> bool {
    if crate::untrusted::triaging(st, s).await {
        return false;
    }
    let level = match st.store.get_project_settings(&s.project_id).await {
        Ok(p) => p.policy.agent_documents,
        Err(_) => return false,
    };
    if level == AgentDocuments::Off {
        return false;
    }
    let own = crate::mcp::tools::session_ticket(st, s).await;
    let caller = Caller { project: &s.project_id, ticket: own.as_deref() };
    // A name PDT can't resolve asks (and the call then fails on its own).
    let project = |p| project_of(st, p);
    let ticket = |t| ticket_of(st, s, t);
    let links = |d: &Document| {
        (d.projects.iter().map(|p| p.id.clone()).collect::<Vec<_>>(), d.tickets.iter().map(|t| t.id.clone()).collect())
    };
    match wants {
        Wants::Create { project: p, ticket: t } => {
            let (Ok(p), Ok(t)) = (project(p).await, ticket(t).await) else { return false };
            // With neither, `document_create` links it to the session's own ticket.
            let t = if p.is_none() && t.is_none() { own.clone() } else { t };
            allows(level, caller, DocumentCall::Create { project: p.as_deref(), ticket: t.as_deref() })
        }
        Wants::Change { document } => {
            let Ok(d) = super::tools::find(st, document).await else { return false };
            let (projects, tickets): (Vec<String>, Vec<String>) = links(&d);
            allows(level, caller, DocumentCall::Change { projects: &projects, tickets: &tickets })
        }
        Wants::Link { document, project: p, ticket: t } => {
            let Ok(d) = super::tools::find(st, document).await else { return false };
            let (Ok(p), Ok(t)) = (project(p).await, ticket(t).await) else { return false };
            if p.is_none() && t.is_none() {
                return false;
            }
            let (projects, tickets): (Vec<String>, Vec<String>) = links(&d);
            let call = DocumentCall::Link {
                projects: &projects,
                tickets: &tickets,
                project: p.as_deref(),
                ticket: t.as_deref(),
            };
            allows(level, caller, call)
        }
    }
}

async fn project_of(st: &AppState, p: Option<&str>) -> Result<Option<String>, String> {
    match given(p) {
        Some(p) => crate::pm::project_id(st, p).await.map(Some),
        None => Ok(None),
    }
}

async fn ticket_of(st: &AppState, s: &Session, t: Option<&str>) -> Result<Option<String>, String> {
    match given(t) {
        Some(t) => crate::mcp::tools::resolve_ticket(st, t, Some(&s.project_id)).await.map(Some),
        None => Ok(None),
    }
}

/// [`unasked`] for a call by tool name (`mcp__pdt__doc_edit` or `doc_edit`) and its arguments.
pub(crate) async fn unasked_call(st: &AppState, s: &Session, tool: &str, input: &Value) -> bool {
    let short = tool.strip_prefix(&format!("mcp__{}__", pdt_core::mcp::SERVER_NAME)).unwrap_or(tool);
    match Wants::of_call(short, input) {
        Some(w) => unasked(st, s, w).await,
        None => false,
    }
}

/// What an unasked change did.
pub(crate) struct Change {
    pub tool: &'static str,
    pub summary: String,
    pub version: Option<String>,
    pub diff: Option<String>,
}

/// Keeps a change an agent made without asking: on the document's changes, and in the history of the ticket the agent
/// works on, with the diff, so the user can review it and roll it back.
pub(crate) async fn record(st: &AppState, s: &Session, d: &Document, c: Change) {
    let agent = crate::mcp::tools::agent_of(st, s).await;
    let at = now_ms();
    let change = DocumentChange {
        id: uuid::Uuid::now_v7().to_string(),
        document_id: d.id.clone(),
        at,
        tool: c.tool.into(),
        summary: c.summary,
        version: c.version,
        diff: c.diff.filter(|d| !d.trim().is_empty()),
        agent: agent.clone(),
    };
    if let Err(e) = st.store.add_document_change(&change).await {
        tracing::warn!(document = %d.id, error = %e, "could not record an agent's document change");
    }
    let Some(ticket) = &agent.from_ticket_id else { return };
    let data = json!({
        "documentId": d.id, "title": d.title, "tool": change.tool, "summary": change.summary,
        "version": change.version, "changeId": change.id,
    });
    let body = change.diff.as_deref().map(diff_block);
    let by = By::agent(agent.clone());
    if let Err(e) = st.store.add_ticket_event(ticket, EventKind::Document, by, body.as_deref(), Some(data), at).await {
        tracing::warn!(ticket = %ticket, error = %e, "could not record an agent's document change on its ticket");
    }
}

/// `GET /api/documents/{id}/changes`: the changes agents made to it without asking, newest first.
pub(crate) async fn changes(
    axum::extract::State(st): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<axum::Json<Vec<DocumentChange>>, crate::error::ApiError> {
    let d = super::routes::document(&st, &id).await?;
    Ok(axum::Json(st.store.document_changes(&d.id).await?))
}

/// A diff as a Markdown code block, fenced longer than any run of backticks in it.
fn diff_block(diff: &str) -> String {
    let longest = diff.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}diff\n{}\n{fence}", diff.trim_end_matches('\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_of_the_tools_the_setting_covers() {
        let edit = json!({"document": " Spec "});
        assert!(matches!(Wants::of_call("doc_edit", &edit), Some(Wants::Change { document: "Spec" })));
        assert!(matches!(Wants::of_call("sheet_edit", &edit), Some(Wants::Change { .. })));
        let create = json!({"title": "x", "ticket": "PDT-1", "project": ""});
        assert!(matches!(
            Wants::of_call("document_create", &create),
            Some(Wants::Create { project: None, ticket: Some("PDT-1") })
        ));
        let link = json!({"document": "d", "project": "PDT"});
        assert!(matches!(
            Wants::of_call("document_link", &link),
            Some(Wants::Link { document: "d", project: Some("PDT"), ticket: None })
        ));
        assert!(Wants::of_call("comment_reply", &edit).is_none(), "comment replies keep asking");
        assert!(Wants::of_call("comment_resolve", &edit).is_none());
        assert!(Wants::of_call("doc_edit", &json!({})).is_none());
    }

    #[test]
    fn diffs_go_in_a_fence_longer_than_their_backticks() {
        assert_eq!(diff_block("-a\n+b\n"), "```diff\n-a\n+b\n```");
        assert_eq!(diff_block("+````x"), "`````diff\n+````x\n`````");
    }
}
