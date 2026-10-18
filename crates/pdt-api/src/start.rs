//! Ticket Start, attempts and merge detection (tickets spec §5, §6): Start makes the branch, worktree, attempt and an
//! agent session whose first prompt is the ticket, and opens the ticket and the session as tabs; an attempt ends when
//! its branch is merged into the base (checked every 60 s and on demand), when it is abandoned, or when the ticket is
//! closed.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use crate::sessions::{CreateSession, create_session};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::session::{SessionKind, ValidSession};
use pdt_core::start::{StartMode, StartResult, StartTicket, ValidStart, started_session_title, ticket_prompt};
use pdt_core::ticket::{Actor, AttemptState, EventKind, LinkDirection, LinkKind, Ticket};
use pdt_core::workspace::{TabItem, Workspace, valid_workspace_name};
use pdt_store::{AttemptInfo, TicketError};
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

/// The longest a `git fetch` of a base branch may take.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

fn conflict(code: &'static str, message: String) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, message)
}

async fn load_ticket(st: &AppState, id: &str) -> Result<Ticket, ApiError> {
    st.store.get_ticket(id).await?.ok_or_else(|| ApiError::not_found("ticket"))
}

/// `POST /api/tickets/{id}/start`.
pub(crate) async fn start(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<StartTicket>, JsonRejection>,
) -> Result<(StatusCode, Json<StartResult>), ApiError> {
    let Json(input) = payload?;
    let v = input.validate()?;
    let guard = st.ticket_start_lock.lock().await;
    let ticket = load_ticket(&st, &id).await?;
    let key = ticket.summary.key.clone();
    if ticket.summary.stage.is_closed() {
        return Err(conflict("ticket_closed", format!("{key} is closed; move it back to Ready to start it")));
    }
    if ticket.attempts.iter().any(|a| a.state == AttemptState::Active) {
        return Err(TicketError::Active(key).into());
    }
    let blockers: Vec<String> = ticket
        .links
        .iter()
        .filter(|l| l.kind == LinkKind::Blocks && l.direction == LinkDirection::Incoming && !l.stage.is_closed())
        .map(|l| l.key.clone())
        .collect();
    if !blockers.is_empty() && v.mode == StartMode::Check {
        return Err(conflict(
            "start_blocked",
            format!("{key} is blocked by {}, which {} not done yet", blockers.join(", "), plural(blockers.len())),
        ));
    }
    if !v.new_workspace {
        crate::workspaces::check_placement(&st, v.tab.as_ref()).await?;
    }
    let project =
        st.store.get_project(&ticket.summary.project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let branch = v.branch.clone().unwrap_or_else(|| ticket.default_branch.clone());
    let base = v.base.clone().unwrap_or_else(|| project.base_branch.clone());
    let existing = st.store.list_worktrees(&project.id).await?.into_iter().find(|w| w.branch == branch);
    let wt = match existing {
        Some(w) => w,
        None => crate::worktrees::make_worktree(&st, &project, &branch, &base, true).await?,
    };
    let extra = if blockers.is_empty() { json!({}) } else { json!({"forced": true, "blockers": blockers}) };
    let attempt_id = uuid::Uuid::now_v7().to_string();
    let started = st
        .store
        .start_attempt(&attempt_id, &id, Some(&wt.id), &branch, &base, &wt.base_commit, extra, now_ms())
        .await?;
    drop(guard);

    let req = CreateSession {
        valid: ValidSession {
            project_id: project.id.clone(),
            worktree_id: Some(wt.id.clone()),
            kind: SessionKind::AgentTui,
            agent: Some(v.agent),
            bypass_permissions: v.bypass_permissions,
            size: v.size,
            initial_prompt: Some(ticket_prompt(&key, &ticket.summary.title, &ticket.description, &branch)),
        },
        title: Some(started_session_title(&ticket.summary.title)),
        attempt_id: Some(attempt_id.clone()),
        ticket_key: Some(key.clone()),
        tab: None,
    };
    let session = match create_session(&st, req).await {
        Ok(s) => s,
        Err(e) => {
            // The worktree stays (the next Start adopts it); the ticket goes back to how it was so Start can be retried.
            if let Err(u) = st.store.undo_start(&started).await {
                tracing::error!(ticket = %id, error = %u, "could not undo a failed start");
            }
            return Err(e);
        }
    };
    let data = json!({"sessionId": session.id, "agent": v.agent, "attemptId": attempt_id});
    if let Err(e) = st.store.add_ticket_event(&id, EventKind::Session, Actor::User, None, Some(data), now_ms()).await {
        tracing::warn!(ticket = %id, error = %e, "could not record the started session on the timeline");
    }
    open_tabs(&st, &project.id, &key, &id, &attempt_id, &session.id, &v).await;
    let ticket = load_ticket(&st, &id).await?;
    let attempt =
        ticket.attempts.iter().find(|a| a.id == attempt_id).cloned().unwrap_or_else(|| started.attempt.clone());
    Ok((StatusCode::CREATED, Json(StartResult { ticket, attempt, session })))
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "is" } else { "are" }
}

/// The ticket's tab, then the session's right after it: in a new workspace named after the ticket, where asked, or
/// at the end of the project's workspace (the attempt remembers a new workspace). A failure is logged: the session runs and can be opened from the sidebar.
async fn open_tabs(
    st: &AppState,
    project_id: &str,
    key: &str,
    ticket_id: &str,
    attempt_id: &str,
    session_id: &str,
    v: &ValidStart,
) {
    let target = if v.new_workspace {
        let w = Workspace {
            id: uuid::Uuid::now_v7().to_string(),
            name: valid_workspace_name(key).unwrap_or_else(|_| key.chars().take(60).collect()),
            project_id: None,
            position: 0,
            created_at: now_ms(),
            tabs: vec![],
        };
        match st.store.insert_workspace(&w).await {
            Ok(()) => {
                // Finishing the ticket removes the workspace with the worktree.
                if let Err(e) = st.store.set_attempt_workspace(attempt_id, &w.id).await {
                    tracing::warn!(error = %e, "could not remember the ticket's workspace");
                }
                Some((w.id, Some(0)))
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not make the ticket's workspace");
                None
            }
        }
    } else if let Some(p) = &v.tab {
        Some((p.workspace_id.clone(), p.position))
    } else {
        st.store.project_workspace_id(project_id).await.ok().flatten().map(|w| (w, None))
    };
    let Some((workspace_id, position)) = target else {
        tracing::warn!(project = %project_id, "no workspace to open the started ticket in");
        return;
    };
    let new_id = || uuid::Uuid::now_v7().to_string();
    let ticket_item = TabItem::Ticket(ticket_id.to_string());
    let after = match st.store.open_tab(&new_id(), &workspace_id, &ticket_item, position, now_ms()).await {
        Ok(Some(tab)) => Some(tab.position + 1),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "could not open the ticket's tab");
            None
        }
    };
    let session_item = TabItem::Session(session_id.to_string());
    if let Err(e) = st.store.open_tab(&new_id(), &workspace_id, &session_item, after, now_ms()).await {
        tracing::warn!(error = %e, "could not open the started session's tab");
    }
}

/// `POST /api/attempts/{id}/abandon`: ends the attempt (the worktree stays). Abandoning an ended attempt changes
/// nothing.
pub(crate) async fn abandon(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Ticket>, ApiError> {
    let info = st.store.get_attempt(&id).await?.ok_or_else(|| ApiError::not_found("attempt"))?;
    st.store.end_attempt(&id, AttemptState::Abandoned, Actor::User, now_ms()).await?;
    Ok(Json(load_ticket(&st, &info.ticket_id).await?))
}

/// `POST /api/tickets/{id}/check-merge`: merge detection for the ticket's active attempt, now.
pub(crate) async fn check_merge(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Ticket>, ApiError> {
    st.store.get_ticket_summary(&id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
    if let Some(info) = st.store.active_attempt(&id).await?
        && is_merged(&st, &info, true).await
    {
        st.store.end_attempt(&info.attempt.id, AttemptState::Merged, Actor::System, now_ms()).await?;
    }
    Ok(Json(load_ticket(&st, &id).await?))
}

/// Ends the ticket's active attempt because the ticket is being closed: `merged` if its branch is, else `abandoned`.
pub(crate) async fn end_for_close(st: &AppState, ticket_id: &str) -> Result<(), ApiError> {
    if let Some(info) = st.store.active_attempt(ticket_id).await? {
        let state = if is_merged(st, &info, false).await { AttemptState::Merged } else { AttemptState::Abandoned };
        st.store.end_attempt(&info.attempt.id, state, Actor::User, now_ms()).await?;
    }
    Ok(())
}

/// Whether the attempt's branch is merged into its base (locally or on `origin`, fetched first with `fetch`), and
/// remembers the branch head it saw. A deleted branch is judged by its last known head. Git runs on a blocking
/// thread; any failure counts as not merged.
async fn is_merged(st: &AppState, info: &AttemptInfo, fetch: bool) -> bool {
    let repo = PathBuf::from(&info.repo_path);
    let (branch, base) = (info.attempt.branch.clone(), info.attempt.base_branch.clone());
    let (base_commit, last_head) = (info.base_commit.clone(), info.attempt.last_head.clone());
    let attempt = info.attempt.id.clone();
    let found = tokio::task::spawn_blocking(move || {
        if fetch
            && pdt_git::remote_url(&repo, "origin").is_some()
            && let Err(e) = pdt_git::fetch(&repo, "origin", &base, FETCH_TIMEOUT)
        {
            tracing::info!(attempt = %attempt, error = %e, "could not fetch the base branch; checking the local refs");
        }
        let head = pdt_git::rev_parse(&repo, &format!("refs/heads/{branch}")).or(last_head)?;
        let origin = format!("origin/{base}");
        let merged = pdt_git::is_merged(&repo, &head, &base_commit, &[&base, &origin]);
        Some((head, merged))
    })
    .await
    .ok()
    .flatten();
    let Some((head, merged)) = found else { return false };
    if info.attempt.last_head.as_deref() != Some(head.as_str())
        && let Err(e) = st.store.set_attempt_head(&info.attempt.id, &head).await
    {
        tracing::warn!(attempt = %info.attempt.id, error = %e, "could not record the branch head");
    }
    merged
}

/// Merge detection for every active attempt (pdtd runs it every 60 s): each project's base branch is fetched once,
/// and attempts whose branch is merged end as `merged`. Never fails; problems are logged.
pub async fn check_merges(st: &AppState) {
    let active = match st.store.list_active_attempts().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "the merge check could not list active attempts");
            return;
        }
    };
    let mut fetched: HashSet<(String, String)> = HashSet::new();
    for info in active {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let fetch = fetched.insert((info.project_id.clone(), info.attempt.base_branch.clone()));
        if is_merged(st, &info, fetch).await {
            match st.store.end_attempt(&info.attempt.id, AttemptState::Merged, Actor::System, now_ms()).await {
                Ok(Some(_)) => tracing::info!(ticket = %info.ticket_key, branch = %info.attempt.branch,
                    "the attempt's branch was merged"),
                Ok(None) => {}
                Err(e) => tracing::warn!(attempt = %info.attempt.id, error = %e, "could not end the merged attempt"),
            }
        }
    }
}
