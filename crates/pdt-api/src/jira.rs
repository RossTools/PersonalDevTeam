//! Jira Cloud (Phase 7e spec §6, PDT-85). A project links to Jira in its host settings (a `jira` connection, a JQL
//! scope and the done transition). Every 60 s pdtd reads the issues of each linked project updated since its last look
//! and logs `jira.issue.created`, `.updated`, `.transitioned` and `.commented` (the first look only records the time,
//! `sensor.jira.<project>`). Event keys name the project, so two projects whose scopes overlap on one connection each
//! get their events. The mark moves only past what was logged: a failed insert leaves it for the next poll, and a poll
//! that hits the page cap moves it to the last issue it read. A failing connection is reported once
//! ([`crate::connection_health`]). The built-in "Import assigned Jira issues" turns issues assigned to the user into tickets
//! through [`import_issue`], which `POST /api/projects/{id}/jira/import` also calls; linking is the dedupe, so an
//! issue is never imported twice.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use crate::workflow::nodes::http::{client, read_body, send_error};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::jira::{FIELDS, IssueNews, JIRA, JiraIssue, issue_key, news, parse_issue, poll_jql, site, text_to_adf};
use pdt_core::ticket::{NewTicket, Ticket};
use pdt_core::workflow::api::{Connection, NewLogEvent};
use pdt_core::workflow::events::{
    self as ev, JiraIssueCommented, JiraIssueCreated, JiraIssueRef, JiraIssueTransitioned, JiraIssueUpdated,
};
use pdt_store::StoredExternalLink;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

/// How far back each poll reads before its last one (clocks differ; event keys make re-reads harmless).
const MARGIN_MS: i64 = 60 * 1000;

/// The most pages one poll reads (of [`PAGE`] issues); the next poll reads on from where it stopped.
const MAX_PAGES: usize = 5;
const PAGE: usize = 100;

/// A `jira` connection ready to call: its site, and the email and API token it signs in with.
pub(crate) struct JiraClient {
    pub conn: Connection,
    email: String,
    token: String,
    /// The longest one call may take (`AppState::sensor_timeout`).
    timeout: std::time::Duration,
}

impl JiraClient {
    /// The client of the connection called `name`, or why not.
    pub(crate) async fn named(st: &AppState, name: &str) -> Result<JiraClient, String> {
        let (conn, secret) = crate::workflow::routes::connection_with_secret(st, name).await?;
        if conn.kind != JIRA {
            return Err(format!("{} is a {} connection, not {JIRA}", conn.name, conn.kind));
        }
        if site(&conn.url).is_empty() {
            return Err(format!("{} has no site address (https://<site>.atlassian.net)", conn.name));
        }
        let email = conn.config.get("email").and_then(|e| e.as_str()).unwrap_or_default().trim().to_string();
        if email.is_empty() {
            return Err(format!("{} names no email to sign in with", conn.name));
        }
        let token = secret.filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("{} has no API token", conn.name))?;
        Ok(JiraClient { conn, email, token, timeout: st.sensor_timeout })
    }

    pub(crate) fn site(&self) -> &str {
        site(&self.conn.url)
    }

    /// One REST call: the JSON answer (`Null` for an empty one), `Ok(None)` for a 404.
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<Option<Value>, String> {
        let url = format!("{}{path}", self.site());
        let mut req = client()
            .request(method.clone(), &url)
            .basic_auth(&self.email, Some(&self.token))
            .header("accept", "application/json")
            .timeout(self.timeout);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| format!("Jira {method} {path}: {}", send_error(e)))?;
        let status = resp.status().as_u16();
        let bytes = read_body(resp, 8 * 1024 * 1024).await?;
        if status == 404 {
            return Ok(None);
        }
        let v: Value =
            if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
        if !(200..300).contains(&status) {
            let msg = v["errorMessages"]
                .as_array()
                .and_then(|m| m.first())
                .and_then(|m| m.as_str())
                .map(String::from)
                .or_else(|| {
                    v["errors"].as_object().filter(|o| !o.is_empty()).map(|o| Value::Object(o.clone()).to_string())
                })
                .unwrap_or_else(|| String::from_utf8_lossy(&bytes).chars().take(300).collect());
            return Err(format!("Jira answered {status} to {method} {path}: {msg}"));
        }
        Ok(Some(v))
    }

    async fn found(&self, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<Value, String> {
        self.call(method, path, body).await?.ok_or_else(|| format!("Jira has no {path}"))
    }

    /// The account the connection signs in as (its `accountId`).
    pub(crate) async fn myself(&self) -> Result<String, String> {
        let v = self.found(reqwest::Method::GET, "/rest/api/3/myself", None).await?;
        v["accountId"].as_str().map(String::from).ok_or_else(|| "Jira did not say who the account is".into())
    }

    /// The issues `jql` finds, with their changelog (up to [`MAX_PAGES`] pages), and whether there were more.
    pub(crate) async fn search(&self, jql: &str) -> Result<(Vec<Value>, bool), String> {
        let mut out = vec![];
        let mut token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut body = json!({"jql": jql, "fields": FIELDS, "expand": "changelog", "maxResults": PAGE});
            if let Some(t) = &token {
                body["nextPageToken"] = json!(t);
            }
            let v = self.found(reqwest::Method::POST, "/rest/api/3/search/jql", Some(&body)).await?;
            out.extend(v["issues"].as_array().cloned().unwrap_or_default());
            token = v["nextPageToken"].as_str().map(String::from);
            if token.is_none() || v["isLast"].as_bool() == Some(true) {
                return Ok((out, false));
            }
        }
        Ok((out, true))
    }

    /// Fills in the newest changelog entries and comments of a search result whose lists Jira cut short (a search
    /// returns at most 100 of each, oldest first), so recent ones are never missed.
    async fn complete(&self, v: &mut Value) -> Result<(), String> {
        let key = v["key"].as_str().unwrap_or_default().to_string();
        let short = |list: &Value, total: &Value| {
            total.as_u64().is_some_and(|t| t > list.as_array().map_or(0, |l| l.len()) as u64)
        };
        if short(&v["changelog"]["histories"], &v["changelog"]["total"]) {
            let total = v["changelog"]["total"].as_u64().unwrap_or(0);
            let start = total.saturating_sub(PAGE as u64);
            let path = format!("/rest/api/3/issue/{key}/changelog?startAt={start}&maxResults={PAGE}");
            if let Some(page) = self.call(reqwest::Method::GET, &path, None).await? {
                v["changelog"]["histories"] = page["values"].clone();
            }
        }
        if short(&v["fields"]["comment"]["comments"], &v["fields"]["comment"]["total"]) {
            let path = format!("/rest/api/3/issue/{key}/comment?orderBy=-created&maxResults={PAGE}");
            if let Some(page) = self.call(reqwest::Method::GET, &path, None).await? {
                v["fields"]["comment"]["comments"] = page["comments"].clone();
            }
        }
        Ok(())
    }

    /// One issue by key (None: no such issue, or the account can't see it).
    pub(crate) async fn issue(&self, key: &str) -> Result<Option<JiraIssue>, String> {
        let path = format!("/rest/api/3/issue/{key}?fields={}", FIELDS.join(","));
        Ok(self.call(reqwest::Method::GET, &path, None).await?.and_then(|v| parse_issue(self.site(), &v)))
    }

    /// The transitions open to an issue now: `(id, name, the status it leads to)`.
    pub(crate) async fn transitions(&self, key: &str) -> Result<Vec<(String, String, String)>, String> {
        let v = self.found(reqwest::Method::GET, &format!("/rest/api/3/issue/{key}/transitions"), None).await?;
        Ok(v["transitions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                (
                    t["id"].as_str().unwrap_or_default().to_string(),
                    t["name"].as_str().unwrap_or_default().to_string(),
                    t["to"]["name"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect())
    }

    pub(crate) async fn do_transition(&self, key: &str, id: &str) -> Result<(), String> {
        let body = json!({"transition": {"id": id}});
        self.found(reqwest::Method::POST, &format!("/rest/api/3/issue/{key}/transitions"), Some(&body)).await?;
        Ok(())
    }

    /// Adds a comment (text, sent as ADF); returns its id.
    pub(crate) async fn comment(&self, key: &str, text: &str) -> Result<String, String> {
        let body = json!({"body": text_to_adf(text)});
        let v = self.found(reqwest::Method::POST, &format!("/rest/api/3/issue/{key}/comment"), Some(&body)).await?;
        Ok(v["id"].as_str().unwrap_or_default().to_string())
    }

    /// Creates an issue from `fields` (REST v3's); returns its key.
    pub(crate) async fn create(&self, fields: Value) -> Result<String, String> {
        let v = self.found(reqwest::Method::POST, "/rest/api/3/issue", Some(&json!({"fields": fields}))).await?;
        v["key"].as_str().map(String::from).ok_or_else(|| "Jira made the issue but did not say its key".into())
    }
}

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
/// that fails is reported on it ([`crate::connection_health`]), other problems are logged.
pub async fn refresh_jira(st: &AppState) {
    let projects = match st.store.list_projects().await {
        Ok(p) => p,
        Err(e) => return tracing::warn!(error = %e, "the Jira check could not list projects"),
    };
    let mut uses = crate::connection_health::Uses::default();
    for p in projects {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Ok(settings) = st.store.get_project_settings(&p.id).await else { continue };
        let Some(jira) = settings.jira else { continue };
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

async fn poll_project(st: &AppState, project_id: &str, jira: &pdt_core::jira::JiraSettings) -> Result<(), PollError> {
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
            s.jira.map(|j| j.connection).ok_or(ImportError::NotLinked)?
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
    let link = StoredExternalLink {
        ticket_id: String::new(),
        system: JIRA.into(),
        connection_id: c.conn.id.clone(),
        key: issue.key.clone(),
        url: issue.url.clone(),
    };
    let id = uuid::Uuid::now_v7().to_string();
    match st.store.create_linked_ticket(&id, project_id, &t, &link, now_ms()).await {
        Ok(Ok(t)) => Ok(Import::Imported(t.summary.id)),
        Ok(Err(other)) => Ok(Import::Exists(other)),
        Err(e) => Err(failed(e.to_string())),
    }
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraImportInput {
    /// `ABC-123`.
    pub key: String,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraImportResult {
    pub ticket: Ticket,
    /// A ticket already linked the issue (nothing new was made).
    pub existed: bool,
}

/// `POST /api/projects/{id}/jira/import {key}`: any issue, assigned or not.
pub(crate) async fn import_route(
    State(st): State<AppState>,
    Path(project_id): Path<String>,
    payload: Result<Json<JiraImportInput>, JsonRejection>,
) -> Result<(StatusCode, Json<JiraImportResult>), ApiError> {
    let Json(input) = payload?;
    st.store.get_project(&project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let r = import_issue(&st, &project_id, &input.key, None).await.map_err(|e| {
        let (status, code) = match &e {
            ImportError::NotLinked => (StatusCode::CONFLICT, "jira_not_linked"),
            ImportError::BadKey(_) => (StatusCode::BAD_REQUEST, "invalid_input"),
            ImportError::NoIssue(_) => (StatusCode::NOT_FOUND, "jira_issue_not_found"),
            ImportError::Failed(_) => (StatusCode::BAD_GATEWAY, "jira_failed"),
        };
        ApiError::new(status, code, e.message())
    })?;
    let (id, existed) = match r {
        Import::Imported(id) => (id, false),
        Import::Exists(id) => (id, true),
    };
    let ticket = st.store.get_ticket(&id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
    let ticket = crate::overlap::decorate(&st, ticket).await?;
    let status = if existed { StatusCode::OK } else { StatusCode::CREATED };
    Ok((status, Json(JiraImportResult { ticket, existed })))
}
