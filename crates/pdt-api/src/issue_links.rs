//! Linking a ticket to an issue that is already in a tracker (PDT-481), and its plain links. Each tracker plugin
//! recognizes its own URLs and keys ([`crate::Integration::find_issue`]) and reads the issue's title and state through
//! the project's connection ([`crate::Integration::read_issue`]); nothing here names a tracker. A link made by hand
//! records that (`manual`), and Done leaves its issue alone unless the user turns on "Close it when this ticket is
//! done" ([`pdt_store::Store::set_close_on_done`]). The ticket view, `link_issue` / `unlink_issue` and
//! `create_ticket`'s `external` come here.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::forge::ForgeKind;
use pdt_core::issue_link::{IssueState, LinkOrigin, repo_issue_key};
use pdt_core::ticket::{By, Ticket};
use pdt_store::{StoredExternalLink, TicketError};
use serde::Deserialize;
use ts_rs::TS;

/// An issue a tracker plugin found from what someone gave (a URL or a key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundIssue {
    /// What the link is read through: the connection's id, or the repository (`owner/name`, lower case).
    pub connection_id: String,
    /// `ABC-123`, `owner/name#12`.
    pub key: String,
    pub url: String,
    pub title: String,
    pub state: IssueState,
}

/// What a tracker says of a linked issue now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueNow {
    pub title: String,
    pub state: IssueState,
}

/// What linking found: the tracker (its plugin's id) and the issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recognized {
    pub system: String,
    pub issue: FoundIssue,
}

/// Why an issue couldn't be linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// No tracker plugin that is on recognizes it: a plain link, if it is an address.
    Unrecognized(String),
    /// A tracker recognized it but couldn't read it (no such issue, no login, the tracker is down).
    Tracker(String),
    /// Another ticket links it already (its key).
    Elsewhere(String),
}

impl LinkError {
    pub fn message(&self, given: &str) -> String {
        match self {
            LinkError::Unrecognized(_) => format!(
                "{given:?} is not an issue of a tracker PDT has on (GitHub, GitLab, Jira…): give its address or key \
                 (owner/name#12, ABC-123), or add it as a plain link"
            ),
            LinkError::Tracker(m) => m.clone(),
            LinkError::Elsewhere(key) => format!("{key} links that issue already: an issue is linked to one ticket"),
        }
    }
}

impl From<LinkError> for ApiError {
    fn from(e: LinkError) -> Self {
        match &e {
            LinkError::Unrecognized(given) => {
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "unrecognized", e.message(given))
            }
            LinkError::Tracker(m) => ApiError::new(StatusCode::BAD_GATEWAY, "tracker_error", m.clone()),
            LinkError::Elsewhere(_) => ApiError::new(StatusCode::CONFLICT, "already_linked", e.message("")),
        }
    }
}

/// The tracker plugin that recognizes `given` for project `project_id`, and the issue it read; the first enabled
/// plugin that recognizes it wins.
pub async fn recognize(st: &AppState, project_id: &str, given: &str) -> Result<Recognized, LinkError> {
    let given = given.trim();
    if given.is_empty() {
        return Err(LinkError::Unrecognized(String::new()));
    }
    let registry = st.registry();
    for int in registry.integrations() {
        if let Some(found) = int.find_issue(st, project_id, given).await {
            return found.map(|issue| Recognized { system: int.id().to_string(), issue }).map_err(LinkError::Tracker);
        }
    }
    Err(LinkError::Unrecognized(given.to_string()))
}

/// Links ticket `ticket_id` to the issue `found`, by hand, on behalf of `by`. `close_on_done` None: off, as a manual
/// link's default.
pub async fn link_found(
    st: &AppState,
    ticket_id: &str,
    found: &Recognized,
    close_on_done: Option<bool>,
    by: impl Into<By>,
) -> Result<Ticket, ApiError> {
    let i = &found.issue;
    let mut link =
        StoredExternalLink::new(ticket_id, &found.system, &i.connection_id, &i.key, &i.url, LinkOrigin::Manual);
    link.close_on_done = close_on_done.unwrap_or(link.close_on_done);
    (link.title, link.state) = (Some(i.title.clone()), Some(i.state));
    match st.store.link_external(&link, by, now_ms()).await? {
        Ok(t) => Ok(t),
        Err(other) if other == ticket_id => {
            Err(ApiError::new(StatusCode::CONFLICT, "already_linked", format!("it links {} already", i.key)))
        }
        Err(other) => {
            let key = st.store.get_ticket_summary(&other).await?.map_or(other, |t| t.key);
            Err(LinkError::Elsewhere(key).into())
        }
    }
}

/// Reads again the ticket's linked issues not read in [`pdt_core::issue_link::REFRESH_AFTER_MS`], through their
/// trackers; one that can't be read keeps what it had and waits as long before it is tried again.
pub async fn refresh(st: &AppState, ticket_id: &str) -> Result<(), ApiError> {
    let now = now_ms();
    let links = st.store.issue_links_to_read(ticket_id, now - pdt_core::issue_link::REFRESH_AFTER_MS).await?;
    if links.is_empty() {
        return Ok(());
    }
    let registry = st.registry();
    for link in links {
        let Some(int) = registry.integrations().iter().find(|i| i.id() == link.system) else { continue };
        match int.read_issue(st, &link).await {
            Some(Ok(n)) => st.store.issue_seen(&link, &n.title, n.state, now).await?,
            Some(Err(e)) => {
                tracing::debug!(system = %link.system, key = %link.key, error = %e, "could not read a linked issue");
                st.store.issue_checked(&link, now).await?;
            }
            None => st.store.issue_checked(&link, now).await?,
        }
    }
    Ok(())
}

// ---- forges' issues, for the forge plugins (GitHub, GitLab) ----

/// An issue page's host (lower case), repository and number, as a forge's URLs have them.
pub type IssuePage = fn(&str) -> Option<(String, String, u64)>;

/// The issue `given` names on forge `kind`, for project `project_id`: its page on a host the forge claims (`page`
/// parses it), or `owner/name#12` / `#12` when the project is on that forge. None: not one of its issues.
pub async fn find_forge_issue(
    st: &AppState,
    kind: &ForgeKind,
    project_id: &str,
    given: &str,
    page: IssuePage,
) -> Option<Result<FoundIssue, String>> {
    let (api, repo, number) = if let Some((host, repo, number)) = page(given) {
        let plugin = st.forge_plugin(kind)?;
        if !plugin.claims_host(&host) {
            return None;
        }
        (plugin.forge_on(&host), repo, number)
    } else {
        let (repo, number) = repo_issue_key(given)?;
        let project = st.store.get_project(project_id).await.ok()??;
        let f = crate::forge::project_forge(st, project_id, &project.repo_path).await?;
        if f.kind != *kind {
            return None;
        }
        (f.api, repo.unwrap_or(f.repo), number)
    };
    Some(read_forge_issue(api, repo, number).await.map(|(d, repo)| FoundIssue {
        connection_id: pdt_core::github_issue::repo_id(&repo),
        key: pdt_core::github_issue::issue_key(&repo, number),
        url: d.issue.url.clone(),
        title: d.issue.title.clone(),
        state: if d.closed { IssueState::Closed } else { IssueState::Open },
    }))
}

/// What forge `kind` says now of a linked issue, from its page's address (`page` parses it). None: the link is not
/// on one of its hosts.
pub async fn read_forge_link(
    st: &AppState,
    kind: &ForgeKind,
    link: &StoredExternalLink,
    page: IssuePage,
) -> Option<Result<IssueNow, String>> {
    let (host, repo, number) = page(&link.url)?;
    let plugin = st.forge_plugin(kind)?;
    let api = plugin.forge_on(&host);
    Some(read_forge_issue(api, repo, number).await.map(|(d, _)| IssueNow {
        title: d.issue.title,
        state: if d.closed { IssueState::Closed } else { IssueState::Open },
    }))
}

async fn read_forge_issue(
    api: std::sync::Arc<dyn pdt_forge::Forge>,
    repo: String,
    number: u64,
) -> Result<(pdt_forge::IssueDetail, String), String> {
    let name = api.name().to_string();
    let r = repo.clone();
    let d = tokio::task::spawn_blocking(move || api.issue(&r, number))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{name} has no issue {repo}#{number} (or PDT's {name} login can't see it)"))?;
    if d.change_request {
        return Err(format!("{repo}#{number} is a {} change request, not an issue", name));
    }
    Ok((d, repo))
}

// ---- routes ----

/// An issue to link: its address or key.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct LinkIssue {
    /// `https://github.com/o/r/issues/12`, `o/r#12`, `ABC-123`.
    pub issue: String,
    /// Done closes (or moves) the issue; off when left out.
    #[serde(default)]
    #[ts(optional)]
    pub close_on_done: Option<bool>,
}

/// One of a ticket's linked issues.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct IssueRef {
    pub system: String,
    pub key: String,
}

/// A linked issue's "Close it when this ticket is done".
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct IssueClose {
    pub system: String,
    pub key: String,
    pub close_on_done: bool,
}

/// A plain link to add.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct NewWebLink {
    pub url: String,
    /// What it shows as; the address's host when left out.
    #[serde(default)]
    #[ts(optional)]
    pub label: Option<String>,
}

async fn reply(st: &AppState, t: Ticket) -> Result<Json<Ticket>, ApiError> {
    Ok(Json(crate::overlap::decorate(st, t).await?))
}

fn no_link(e: TicketError) -> ApiError {
    match e {
        TicketError::NotFound("link") => ApiError::not_found("link"),
        e => e.into(),
    }
}

async fn ticket_project(st: &AppState, id: &str) -> Result<String, ApiError> {
    Ok(st.store.get_ticket_summary(id).await?.ok_or_else(|| ApiError::not_found("ticket"))?.project_id)
}

/// `POST /api/tickets/{id}/issues`: links an issue already in a tracker, read through the project's connection.
/// 422 `unrecognized` when no tracker plugin knows it (the UI offers a plain link).
async fn link(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<LinkIssue>, JsonRejection>,
) -> Result<(StatusCode, Json<Ticket>), ApiError> {
    let Json(b) = body?;
    let project = ticket_project(&st, &id).await?;
    let found = recognize(&st, &project, &b.issue).await?;
    let t = link_found(&st, &id, &found, b.close_on_done, pdt_core::ticket::Actor::User).await?;
    Ok((StatusCode::CREATED, reply(&st, t).await?))
}

/// `DELETE /api/tickets/{id}/issues?system=…&key=…`: unlinks an issue (nothing changes in the tracker).
async fn unlink(
    State(st): State<AppState>,
    Path(id): Path<String>,
    q: Result<Query<IssueRef>, QueryRejection>,
) -> Result<Json<Ticket>, ApiError> {
    let Query(r) = q.map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", e.body_text()))?;
    let t = st.store.unlink_external(&id, &r.system, &r.key, pdt_core::ticket::Actor::User, now_ms()).await;
    reply(&st, t.map_err(no_link)?).await
}

/// `PATCH /api/tickets/{id}/issues`: turns a linked issue's "Close it when this ticket is done" on or off.
async fn set_close(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<IssueClose>, JsonRejection>,
) -> Result<Json<Ticket>, ApiError> {
    let Json(b) = body?;
    let t = st.store.set_close_on_done(&id, &b.system, &b.key, b.close_on_done, now_ms()).await;
    reply(&st, t.map_err(no_link)?).await
}

/// `POST /api/tickets/{id}/issues/refresh`: reads again the linked issues' titles and states that are a few minutes
/// old (the ticket view calls it when it opens); the ticket as it is then.
async fn refresh_route(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Ticket>, ApiError> {
    refresh(&st, &id).await?;
    let t = st.store.get_ticket(&id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
    reply(&st, t).await
}

/// `POST /api/tickets/{id}/web-links`: adds a plain link (an address and its label).
async fn add_web(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<NewWebLink>, JsonRejection>,
) -> Result<(StatusCode, Json<Ticket>), ApiError> {
    let Json(b) = body?;
    let (url, label) = pdt_core::issue_link::web_link(&b.url, b.label.as_deref().unwrap_or_default())
        .map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", m))?;
    let t = st.store.add_web_link(&id, &url, &label, pdt_core::ticket::Actor::User, now_ms()).await?;
    Ok((StatusCode::CREATED, reply(&st, t).await?))
}

/// `DELETE /api/tickets/{id}/web-links/{link}`.
async fn remove_web(
    State(st): State<AppState>,
    Path((id, link)): Path<(String, String)>,
) -> Result<Json<Ticket>, ApiError> {
    let t = st.store.remove_web_link(&id, &link, pdt_core::ticket::Actor::User, now_ms()).await;
    reply(&st, t.map_err(no_link)?).await
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{delete, post};
    axum::Router::new()
        .route("/tickets/{id}/issues", post(link).delete(unlink).patch(set_close))
        .route("/tickets/{id}/issues/refresh", post(refresh_route))
        .route("/tickets/{id}/web-links", post(add_web))
        .route("/tickets/{id}/web-links/{link}", delete(remove_web))
}
