//! Document changes agents make without asking (PDT-618): the project's "Agents may change documents" level
//! ([`pdt_core::policy::AgentDocuments`]) lets an agent create, edit and link documents on its own ticket, or across
//! the project, with no prompt. [`unasked`] resolves what a call touches and puts it to
//! [`pdt_core::document_access`]; [`record`] keeps each such change on the document and on the agent's ticket, so the
//! user can review it and roll it back.

use crate::routes::{AppState, now_ms};
use pdt_core::document::{
    CHANGE_DIFF_MAX, CHANGES_PAGE, CHANGES_PAGE_MAX, Document, DocumentBackup, DocumentChange, cap_diff,
};
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
    /// What a local document was before it ([`backup`]).
    pub backup: Option<DocumentBackup>,
}

/// What a local document is before an unasked change, so the user can restore it (PDT-709); none for another
/// provider's (its own history keeps versions) or a file too large to keep.
pub(crate) async fn backup(st: &AppState, d: &Document) -> Option<DocumentBackup> {
    if d.provider != super::local::ID {
        return None;
    }
    st.local
        .backup(&d.id, &d.provider_ref)
        .await
        .inspect_err(|e| tracing::warn!(document = %d.id, error = %e, "could not keep a copy before an agent's change"))
        .ok()
        .flatten()
}

/// Keeps a change an agent made without asking: on the document's changes, and in the history of the ticket the agent
/// works on, with the diff (cut to [`CHANGE_DIFF_MAX`]: the version or the kept copy has the rest), so the user can
/// review it and roll it back.
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
        diff: c.diff.filter(|d| !d.trim().is_empty()).map(|d| cap_diff(&d, CHANGE_DIFF_MAX)),
        agent: agent.clone(),
        restorable: c.backup.is_some(),
    };
    if let Err(e) = st.store.add_document_change(&change, c.backup.as_ref()).await {
        tracing::warn!(document = %d.id, error = %e, "could not record an agent's document change");
    }
    let Some(ticket) = &agent.from_ticket_id else { return };
    let data = json!({
        "documentId": d.id, "title": d.title, "tool": change.tool, "summary": change.summary,
        "version": change.version, "changeId": change.id,
    });
    let body = change.diff.as_deref().map(|d| pdt_core::text::code_block("diff", d));
    let by = By::agent(agent.clone());
    if let Err(e) = st.store.add_ticket_event(ticket, EventKind::Document, by, body.as_deref(), Some(data), at).await {
        tracing::warn!(ticket = %ticket, error = %e, "could not record an agent's document change on its ticket");
    }
}

/// One page of `GET /api/documents/{id}/changes`: `limit` changes (20, at most 100), those before the change `before`.
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct ChangesQuery {
    limit: Option<u32>,
    before: Option<String>,
}

/// `GET /api/documents/{id}/changes?limit=&before=`: the changes agents made to it without asking, newest first, a
/// page at a time (PDT-710).
pub(crate) async fn changes(
    axum::extract::State(st): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    crate::extract::ApiQuery(q): crate::extract::ApiQuery<ChangesQuery>,
) -> Result<axum::Json<Vec<DocumentChange>>, crate::error::ApiError> {
    let d = super::routes::document(&st, &id).await?;
    let limit = q.limit.unwrap_or(CHANGES_PAGE).clamp(1, CHANGES_PAGE_MAX);
    let before = q.before.as_deref().map(str::trim).filter(|b| !b.is_empty());
    Ok(axum::Json(st.store.document_changes(&d.id, before, limit).await?))
}

/// `POST /api/documents/{id}/changes/{change}/restore`: puts a local document back as it was before an agent's
/// unasked change (PDT-709). Every change since goes too, the user's as well.
pub(crate) async fn restore(
    axum::extract::State(st): axum::extract::State<AppState>,
    axum::extract::Path((id, change)): axum::extract::Path<(String, String)>,
) -> Result<axum::Json<super::routes::EditResult>, crate::error::ApiError> {
    let d = super::routes::document(&st, &id).await?;
    let Some(backup) = st.store.document_change_backup(&d.id, &change).await? else {
        return Err(crate::error::ApiError::not_found("a kept copy of the document before that change"));
    };
    let version = st.local.restore(&d.id, &d.provider_ref, backup).await?;
    Ok(axum::Json(super::routes::EditResult { version: Some(version), charts: vec![] }))
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
}
