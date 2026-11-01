//! Stopping a ticket's work (PDT-159): its open workflow runs are cancelled, its attempt's sessions stop and the
//! attempt is abandoned; with `removeWorktree`, its worktree (uncommitted changes and all), the sessions and runs in it
//! and the workspace Start made go too, so a new start begins clean. Moving a ticket with an active attempt back to
//! Draft, Ready or To do does it (worktree kept), as do Abandon, `POST /api/tickets/{id}/stop` and the PM's
//! `stop_ticket`.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::ticket::{Actor, AttemptState, By, TicketStage};
use pdt_store::AttemptInfo;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct StopRequest {
    /// Where the ticket goes: draft, ready or todo. Left out: an in-progress ticket goes to todo, any other stays.
    #[ts(optional)]
    pub stage: Option<TicketStage>,
    /// Also remove the worktree, its uncommitted changes and the workspace Start made, so the next start is clean.
    pub remove_worktree: bool,
    /// With `removeWorktree`: delete the branch too.
    pub delete_branch: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StopResult {
    /// The ticket's key.
    pub key: String,
    /// Its stage now.
    pub stage: Option<TicketStage>,
    /// The attempt abandoned (None: none was active).
    pub abandoned: Option<String>,
    pub runs_cancelled: u32,
    pub sessions_stopped: u32,
    pub worktree_removed: bool,
    pub workspace_removed: bool,
    /// Why the branch stayed when it was to be deleted.
    pub branch_note: Option<String>,
}

/// `POST /api/tickets/{id}/stop`. In a task of its own, so it completes when the caller goes away.
pub(crate) async fn stop_route(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    payload: Result<Json<StopRequest>, JsonRejection>,
) -> Result<Json<StopResult>, ApiError> {
    let req = match payload {
        Ok(Json(r)) => r,
        Err(JsonRejection::MissingJsonContentType(_)) => StopRequest::default(),
        Err(e) => return Err(e.into()),
    };
    // Moving it back to To do lets an external ticket in: only from PDT's web client (PDT-204).
    if let Some(to) = req.stage
        && !crate::untrusted::from_ui(&st, &headers)?
        && let Some(t) = st.store.get_ticket_summary(&id).await?
        && crate::untrusted::held_back(&t)
        && pdt_core::ticket::boards(Some(t.stage), Some(to), &[], None)
    {
        return Err(crate::untrusted::refused(&t.key));
    }
    let task = tokio::spawn(async move { stop_ticket(&st, &id, &req, Actor::User).await });
    Ok(Json(task.await.map_err(ApiError::internal)??))
}

/// Moves the ticket where `req` says, then stops its work (see the module docs).
pub(crate) async fn stop_ticket(
    st: &AppState,
    ticket_id: &str,
    req: &StopRequest,
    by: impl Into<By>,
) -> Result<StopResult, ApiError> {
    let by = by.into();
    if let Some(stage) = req.stage.filter(|s| !s.is_before_work()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_stage",
            format!("a stopped ticket goes to draft, ready or todo, not {}", stage.as_str()),
        ));
    }
    let since = now_ms();
    let ticket = st.store.get_ticket_summary(ticket_id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
    let to = req.stage.or((ticket.stage == TicketStage::InProgress).then_some(TicketStage::Todo));
    if let Some(stage) = to.filter(|s| *s != ticket.stage) {
        st.store.move_ticket_by(ticket_id, stage, &by, None, now_ms()).await?;
    }
    let mut result = stop_work(st, ticket_id, since, &by).await?;
    if req.remove_worktree {
        // The latest attempt's worktree, also when an earlier stop left it.
        let latest = st.store.get_ticket_summary(ticket_id).await?.and_then(|t| t.attempt);
        if let Some(info) = match latest {
            Some(a) => st.store.get_attempt(&a.id).await?,
            None => None,
        } {
            let p = crate::finish::plan(st, &info).await?;
            let done = crate::finish::clean_up(st, &info, &p, true, req.delete_branch).await;
            result.worktree_removed = done.worktree_removed;
            result.workspace_removed = done.workspace_removed;
            result.branch_note = done.branch_note;
        }
    }
    result.stage = st.store.get_ticket_summary(ticket_id).await?.map(|t| t.stage);
    tracing::info!(ticket = %result.key, abandoned = ?result.abandoned, runs = result.runs_cancelled,
        sessions = result.sessions_stopped, worktree_removed = result.worktree_removed, "stopped a ticket's work");
    Ok(result)
}

/// Stops the work on a ticket with an active attempt, if it has one: cancels its open workflow runs (the attempt's,
/// and the ticket's that started before `since`, so a run the move itself triggers goes on), stops the attempt's
/// running sessions (they stay, with their tabs) and abandons the attempt. Runs first, so no step starts a new session
/// meanwhile. Problems stopping one thing are logged and the rest goes on.
pub(crate) async fn stop_work(
    st: &AppState,
    ticket_id: &str,
    since: i64,
    by: impl Into<By>,
) -> Result<StopResult, ApiError> {
    let key = st.store.get_ticket_summary(ticket_id).await?.map(|t| t.key).unwrap_or_default();
    let mut result = StopResult { key, ..Default::default() };
    let Some(info) = st.store.active_attempt(ticket_id).await? else { return Ok(result) };
    result.runs_cancelled = cancel_runs(st, &info, since).await;
    let p = crate::finish::plan(st, &info).await?;
    for id in &p.running {
        match crate::sessions::stop(st, id).await {
            Ok(()) => result.sessions_stopped += 1,
            Err(e) => {
                tracing::warn!(ticket = %info.ticket_key, session = %id, error = %e.message(), "could not stop the session")
            }
        }
    }
    let attempt = &info.attempt.id;
    if st.store.end_attempt(attempt, AttemptState::Abandoned, by, None, None, now_ms()).await?.is_some() {
        result.abandoned = Some(attempt.clone());
    }
    Ok(result)
}

/// Cancels the ticket's open runs of the attempt, or started before `since`. Each cancel takes the engine lock.
async fn cancel_runs(st: &AppState, info: &AttemptInfo, since: i64) -> u32 {
    let ticket_id = &info.ticket_id;
    let filter = pdt_store::RunFilter { ticket_id: Some(ticket_id.clone()), limit: 500, ..Default::default() };
    let runs = match st.store.list_workflow_runs(&filter).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(ticket = %ticket_id, error = %e, "could not list the ticket's workflow runs");
            return 0;
        }
    };
    let ours = |r: &&pdt_core::workflow::api::Run| {
        !r.status.is_finished() && (r.attempt_id.as_deref() == Some(info.attempt.id.as_str()) || r.started_at <= since)
    };
    let mut cancelled = 0;
    for run in runs.iter().filter(ours) {
        let done = {
            let _engine = st.engine_lock.lock().await;
            crate::workflow::engine::cancel_because(st, &run.id, "cancelled: the ticket's work was stopped").await
        };
        match done {
            Ok(()) => cancelled += 1,
            Err(e) => {
                tracing::warn!(ticket = %ticket_id, run = %run.id, error = %e, "could not cancel the ticket's run")
            }
        }
    }
    cancelled
}

/// After a stage change made at `since`: a ticket moved to Draft, Ready or To do with an active attempt stops its
/// work. Failures are logged: the move itself went through.
pub(crate) async fn after_move(st: &AppState, ticket_id: &str, stage: TicketStage, since: i64, by: impl Into<By>) {
    if !stage.is_before_work() {
        return;
    }
    if let Err(e) = stop_work(st, ticket_id, since, by).await {
        tracing::warn!(ticket = %ticket_id, error = %e.message(), "could not stop the moved ticket's work");
    }
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::post;
    axum::Router::new().route("/tickets/{id}/stop", post(stop_route))
}
