//! Jira (PDT-85, Server / Data Center too, PDT-258). A project links to Jira in its host settings (a `jira` connection, a JQL
//! scope and the done transition). Every 60 s pdtd reads the issues of each linked project updated since its last look
//! and logs `jira.issue.created`, `.updated`, `.transitioned` and `.commented` (the first look only records the time,
//! `sensor.jira.<project>`). Event keys name the project, so two projects whose scopes overlap on one connection each
//! get their events. The mark moves only past what was logged: a failed insert leaves it for the next poll, and a poll
//! that hits the page cap moves it to the last issue it read. A failing connection is reported once
//! ([`pdt_api::sdk::connections`]). The built-in "Import assigned Jira issues" turns issues assigned to the user into tickets
//! through [`import_issue`], which `POST /api/projects/{id}/import/jira` also calls; linking is the dedupe, so an
//! issue is never imported twice.

use crate::client::JiraClient;
use crate::core::{
    self as ev, JiraIssueCommented, JiraIssueCreated, JiraIssueRef, JiraIssueTransitioned, JiraIssueUpdated,
};
use crate::core::{IssueNews, JIRA, JiraIssue, issue_key, news, parse_issue, poll_jql};
use axum::http::StatusCode;
use pdt_api::sdk::{ApiError, AppState, Imported, codes, now_ms};
use pdt_core::issue_link::LinkOrigin;
use pdt_core::ticket::NewTicket;
use pdt_core::workflow::api::{Connection, NewLogEvent};
use pdt_store::StoredExternalLink;
use serde_json::{Value, json};

/// How far back each poll reads before its last one (clocks differ; event keys make re-reads harmless).
const MARGIN_MS: i64 = 60 * 1000;

/// The event payload's issue fields.
fn issue_ref(
    project_id: &str,
    conn: &Connection,
    i: &JiraIssue,
    me: Option<&str>,
    ticket: Option<String>,
) -> JiraIssueRef {
    JiraIssueRef {
        project_id: project_id.into(),
        connection: conn.name.clone(),
        key: i.key.clone(),
        url: i.url.clone(),
        summary: i.summary.clone(),
        description: i.description.clone(),
        status: i.status.clone(),
        assignee: i.assignee.clone(),
        assigned_to_me: me.is_some() && i.assignee_id.as_deref() == me,
        reporter: i.reporter.clone(),
        issue_type: i.issue_type.clone(),
        priority: i.priority.clone(),
        labels: i.labels.clone(),
        ticket_id: ticket,
    }
}

/// The events of what changed about `issue` since `since`.
pub(crate) fn issue_events(
    project_id: &str,
    conn: &Connection,
    issue: &JiraIssue,
    me: Option<&str>,
    ticket: Option<String>,
    since: i64,
) -> Vec<NewLogEvent> {
    let r = issue_ref(project_id, conn, issue, me, ticket);
    let key = |what: &str| format!("jira:{project_id}:{}:{}:{what}", conn.id, issue.key);
    news(issue, since)
        .into_iter()
        .map(|n| match n {
            IssueNews::Created => {
                NewLogEvent::new(ev::JIRA_ISSUE_CREATED, key("created"), "jira", JiraIssueCreated { issue: r.clone() })
            }
            IssueNews::Updated { changed } => NewLogEvent::new(
                ev::JIRA_ISSUE_UPDATED,
                key(&format!("updated:{}", issue.updated_raw)),
                "jira",
                JiraIssueUpdated { issue: r.clone(), changed },
            ),
            IssueNews::Transitioned { history, from, to } => NewLogEvent::new(
                ev::JIRA_ISSUE_TRANSITIONED,
                key(&format!("status:{history}")),
                "jira",
                JiraIssueTransitioned { issue: r.clone(), from, to },
            ),
            IssueNews::Commented(c) => NewLogEvent::new(
                ev::JIRA_ISSUE_COMMENTED,
                key(&format!("comment:{}", c.id)),
                "jira",
                JiraIssueCommented {
                    issue: r.clone(),
                    comment_id: c.id.clone(),
                    author: c.author.clone(),
                    body: c.body.clone(),
                    by_me: me.is_some() && c.author_id.as_deref() == me,
                },
            ),
        })
        .collect()
}

fn baseline_key(project_id: &str) -> String {
    format!("sensor.jira.{project_id}")
}

/// Why a poll stopped: the connection failed (reported on it), or something of PDT's own (logged).
enum PollError {
    Connection(String),
    Local(String),
}

fn local(e: impl std::fmt::Display) -> PollError {
    PollError::Local(e.to_string())
}

/// One look at every linked project's recently updated issues. pdtd runs it every 60 s. Never fails: a connection
/// that fails is reported on it ([`pdt_api::sdk::connections`]), other problems are logged.
pub async fn refresh_jira(st: &AppState) {
    let projects = match st.store.list_projects().await {
        Ok(p) => p,
        Err(e) => return tracing::warn!(error = %e, "the Jira check could not list projects"),
    };
    let mut uses = pdt_api::sdk::connections::Uses::default();
    for p in projects {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Ok(settings) = st.store.get_project_settings(&p.id).await else { continue };
        let Some(jira) = settings.integration::<crate::core::JiraSettings>(JIRA) else { continue };
        match poll_project(st, &p.id, &jira).await {
            Ok(()) => uses.worked(&jira.connection),
            Err(PollError::Connection(e)) => uses.failed(&jira.connection, &e),
            Err(PollError::Local(e)) => {
                tracing::warn!(project = %p.id, error = %e, "could not log the project's Jira news");
            }
        }
    }
    uses.report(st).await;
}

async fn poll_project(st: &AppState, project_id: &str, jira: &crate::core::JiraSettings) -> Result<(), PollError> {
    let c = JiraClient::named(st, &jira.connection).await.map_err(PollError::Connection)?;
    let key = baseline_key(project_id);
    let started = now_ms();
    let seen = st.store.get_host_value(&key).await.map_err(local)?;
    // A new link, or one to another connection or scope, starts from now.
    let same =
        |v: &Value| v["connection"].as_str() == Some(c.conn.id.as_str()) && v["scope"].as_str() == Some(&jira.scope);
    let since = match seen.filter(same).and_then(|v| v["since"].as_i64()) {
        Some(s) => s,
        None => {
            let v = json!({"since": started, "connection": c.conn.id, "scope": jira.scope});
            return st.store.put_host_value(&key, &v).await.map_err(local);
        }
    };
    let minutes = (started - since + MARGIN_MS) / 60_000 + 1;
    let (mut issues, more) = c.search(&poll_jql(&jira.scope, minutes)).await.map_err(PollError::Connection)?;
    let me = c.myself().await.ok();
    let mut last_updated = since;
    for v in issues.iter_mut() {
        c.complete(v).await.map_err(PollError::Connection)?;
        let Some(issue) = parse_issue(c.site(), v) else { continue };
        let ticket = st.store.ticket_for_external(JIRA, &c.conn.id, &issue.key).await.map_err(local)?;
        for e in issue_events(project_id, &c.conn, &issue, me.as_deref(), ticket, since - MARGIN_MS) {
            // The mark stays: the next poll reads this again (keys make what was logged harmless).
            st.store.emit_event(&e, now_ms()).await.map_err(|x| local(format!("{}: {x}", issue.key)))?;
        }
        last_updated = last_updated.max(issue.updated);
    }
    // Cut short by the page cap (oldest first): read on from the last issue read, unless that would not move it.
    let next = if more && last_updated > since {
        last_updated
    } else {
        if more {
            tracing::warn!(project = project_id, "more Jira issues changed in a minute than one poll reads");
        }
        started
    };
    let v = json!({"since": next, "connection": c.conn.id, "scope": jira.scope});
    st.store.put_host_value(&key, &v).await.map_err(local)
}

/// What an import came to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Import {
    Imported(String),
    /// A ticket already links the issue.
    Exists(String),
}

/// Why an import failed: the issue isn't there, or something else.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImportError {
    NotLinked,
    BadKey(String),
    NoIssue(String),
    Failed(String),
}

impl ImportError {
    pub(crate) fn message(&self) -> String {
        match self {
            ImportError::NotLinked => "the project is not linked to Jira (its settings name no Jira connection)".into(),
            ImportError::BadKey(k) => format!("{k:?} is not an issue key like ABC-123"),
            ImportError::NoIssue(k) => format!("Jira has no issue {k} (or the account can't see it)"),
            ImportError::Failed(m) => m.clone(),
        }
    }
}

/// Makes a Draft ticket in `project_id` from the Jira issue `key` (title its summary, description its text), linked
/// to it, unless a ticket already links it. `connection`: a `jira` connection's name; default the project's.
pub(crate) async fn import_issue(
    st: &AppState,
    project_id: &str,
    key: &str,
    connection: Option<&str>,
) -> Result<Import, ImportError> {
    let failed = |e: String| ImportError::Failed(e);
    let key = issue_key(key).ok_or_else(|| ImportError::BadKey(key.to_string()))?;
    let name = match connection.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => c.to_string(),
        None => {
            let s = st.store.get_project_settings(project_id).await.map_err(|e| failed(e.to_string()))?;
            s.integration::<crate::core::JiraSettings>(JIRA).map(|j| j.connection).ok_or(ImportError::NotLinked)?
        }
    };
    let c = JiraClient::named(st, &name).await.map_err(failed)?;
    if let Some(t) = st.store.ticket_for_external(JIRA, &c.conn.id, &key).await.map_err(|e| failed(e.to_string()))? {
        return Ok(Import::Exists(t));
    }
    let issue = c.issue(&key).await.map_err(failed)?.ok_or_else(|| ImportError::NoIssue(key.clone()))?;
    let title: String = match issue.summary.trim() {
        "" => issue.key.clone(),
        s => s.chars().take(pdt_core::ticket::MAX_TITLE).collect(),
    };
    let description: String = issue.description.chars().take(pdt_core::ticket::MAX_DESCRIPTION).collect();
    let t = NewTicket { title, description: Some(description), ..Default::default() }
        .validate()
        .map_err(|e| failed(e.to_string()))?;
    let link = StoredExternalLink::new("", JIRA, &c.conn.id, &issue.key, &issue.url, LinkOrigin::Imported);
    let id = uuid::Uuid::now_v7().to_string();
    match st.store.create_linked_ticket(&id, project_id, &t, &link, pdt_core::ticket::Actor::System, now_ms()).await {
        Ok(Ok(t)) => Ok(Import::Imported(t.summary.id)),
        Ok(Err(other)) => Ok(Import::Exists(other)),
        Err(e) => Err(failed(e.to_string())),
    }
}

/// `POST /api/projects/{id}/import/jira {key}`: any issue, assigned or not ([`pdt_api::sdk::Integration::import_by_key`]).
pub(crate) async fn import_by_key(st: &AppState, project_id: &str, key: &str) -> Result<Imported, ApiError> {
    let r = import_issue(st, project_id, key, None).await.map_err(|e| {
        let (status, code) = match &e {
            ImportError::NotLinked => (StatusCode::CONFLICT, "jira_not_linked"),
            ImportError::BadKey(_) => (StatusCode::BAD_REQUEST, codes::INVALID_INPUT),
            ImportError::NoIssue(_) => (StatusCode::NOT_FOUND, "jira_issue_not_found"),
            ImportError::Failed(_) => (StatusCode::BAD_GATEWAY, "jira_failed"),
        };
        ApiError::new(status, code, e.message())
    })?;
    Ok(match r {
        Import::Imported(ticket_id) => Imported { ticket_id, existed: false },
        Import::Exists(ticket_id) => Imported { ticket_id, existed: true },
    })
}
