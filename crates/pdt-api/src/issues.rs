//! Filing issues on a project's forge (PDT-221): from a ticket (the ticket links the new issue as `owner/name#12`,
//! [`pdt_core::github_issue`], and stays the user's own: the issue came from it, not from outside), or new. The ticket
//! drawer's "Create GitHub issue", the command palette's "New issue", the `github.create_issue` node and the
//! `issue_create` tool all come here; the issue is opened as the forge's login (the user's).

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use pdt_core::forge::ForgeKind;
use pdt_core::github_issue::{GITHUB, issue_key, repo_id};
use pdt_core::ticket::{Actor, By};
use pdt_forge::{ForgeError, NewIssue};
use pdt_store::StoredExternalLink;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// GitHub's longest issue title.
pub const MAX_ISSUE_TITLE: usize = 256;

/// A project's forge, as the UI needs it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectForgeInfo {
    pub kind: ForgeKind,
    /// The forge's name for people (`GitHub`).
    pub name: String,
    /// What it calls a base branch's merge queue (`merge queue`), when it has them.
    pub queue_name: Option<String>,
    /// The forge's name for the repository (GitHub's `owner/name`).
    pub repo: String,
    /// Issues can be filed there (`POST /api/projects/{id}/issues`).
    pub issues: bool,
}

/// An issue to file.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct NewForgeIssue {
    pub title: String,
    /// Markdown.
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub labels: Vec<String>,
    /// The ticket it is filed from (in the project): it links the new issue.
    #[serde(default)]
    #[ts(optional)]
    pub ticket_id: Option<String>,
}

/// An issue just filed.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FiledIssue {
    /// `owner/name`.
    pub repo: String,
    #[ts(type = "number")]
    pub number: u64,
    /// `owner/name#12`: the ticket's external key.
    pub issue: String,
    pub url: String,
    /// The ticket that links it, when it was filed from one.
    pub ticket_key: Option<String>,
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", message)
}

fn forge_error(e: ForgeError) -> ApiError {
    match e {
        ForgeError::Unavailable(m) => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "forge_unavailable", m),
        ForgeError::Failed(m) | ForgeError::NotReady(m) => ApiError::new(StatusCode::BAD_GATEWAY, "forge_error", m),
        ForgeError::Unsupported(m) => ApiError::new(StatusCode::BAD_REQUEST, "unsupported", m),
    }
}

/// The project's forge, if it has one.
async fn info(
    st: &AppState,
    project_id: &str,
) -> Result<Option<(ProjectForgeInfo, crate::forge::ProjectForge)>, ApiError> {
    let project = st.store.get_project(project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let Some(f) = crate::forge::project_forge(st, &project.id, &project.repo_path).await else { return Ok(None) };
    // Only GitHub's issues can be linked to tickets (`owner/name#12`).
    let issues = f.kind == ForgeKind::GITHUB && f.caps().issues;
    let name = f.api.name().to_string();
    let queue_name = f.caps().merge_queue.then(|| f.api.queue_name().to_string());
    Ok(Some((ProjectForgeInfo { kind: f.kind.clone(), name, queue_name, repo: f.repo.clone(), issues }, f)))
}

/// `GET /api/projects/{id}/forge`: the project's forge; null when it has none.
pub async fn forge(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<ProjectForgeInfo>>, ApiError> {
    Ok(Json(info(&st, &id).await?.map(|(i, _)| i)))
}

/// `POST /api/projects/{id}/issues`: files an issue in the project's repository as the user. Only from PDT's web client
/// (its UI token, PDT-204): it publishes as the user's login, so an agent files through `issue_create`, which asks.
pub async fn create(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<NewForgeIssue>, JsonRejection>,
) -> Result<(StatusCode, Json<FiledIssue>), ApiError> {
    if !crate::untrusted::from_ui(&st, &headers)? {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "not_the_ui",
            "only the user files issues over the HTTP API (PDT's web app); an agent uses the issue_create tool",
        ));
    }
    let Json(new) = body?;
    Ok((StatusCode::CREATED, Json(file(&st, &id, new, Actor::User.into()).await?)))
}

/// Files `new` in project `project_id`'s repository and links it to `new.ticket_id`, on behalf of `by`.
pub(crate) async fn file(st: &AppState, project_id: &str, new: NewForgeIssue, by: By) -> Result<FiledIssue, ApiError> {
    let title = new.title.trim().to_string();
    if title.is_empty() {
        return Err(bad("the issue has no title"));
    }
    if title.chars().count() > MAX_ISSUE_TITLE {
        return Err(bad(format!("an issue title must be at most {MAX_ISSUE_TITLE} characters")));
    }
    let labels: Vec<String> = new.labels.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    let ticket = match new.ticket_id.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => {
            let t = st.store.get_ticket(t).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
            if t.summary.project_id != project_id {
                return Err(bad(format!("{} is not this project's ticket", t.summary.key)));
            }
            if let Some(l) = t.summary.external.iter().find(|l| l.system == GITHUB) {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "already_linked",
                    format!("{} is already linked to {}", t.summary.key, l.key),
                ));
            }
            Some(t)
        }
        None => None,
    };
    let (i, f) = info(st, project_id).await?.ok_or_else(|| {
        bad("this project has no forge: its .pdt/project.toml says `forge = \"none\"`, or its origin is on no forge PDT knows")
    })?;
    if !i.issues {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "unsupported", format!("{} has no issues", f.api.name())));
    }
    let (api, repo) = (f.api.clone(), f.repo.clone());
    let ask = NewIssue { title, body: new.body, labels };
    let issue = tokio::task::spawn_blocking(move || api.create_issue(&repo, &ask))
        .await
        .map_err(ApiError::internal)?
        .map_err(forge_error)?;
    let key = issue_key(&f.repo, issue.number);
    let mut ticket_key = None;
    if let Some(t) = ticket {
        let link = StoredExternalLink {
            ticket_id: t.summary.id.clone(),
            system: GITHUB.into(),
            connection_id: repo_id(&f.repo),
            key: key.clone(),
            url: issue.url.clone(),
        };
        // The issue is filed whatever happens to the link: say so rather than lose it.
        match st.store.link_external(&link, by, now_ms()).await {
            Ok(Ok(_)) => ticket_key = Some(t.summary.key),
            Ok(Err(_)) | Err(_) => {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "link_failed",
                    format!("filed {} ({}), but could not link it to {}", key, issue.url, t.summary.key),
                ));
            }
        }
    }
    Ok(FiledIssue { repo: f.repo, number: issue.number, issue: key, url: issue.url, ticket_key })
}
