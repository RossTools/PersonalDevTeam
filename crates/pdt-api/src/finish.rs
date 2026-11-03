//! `ticket.finish`: one server command for the merged banner, Merge…'s "Move to Done and clean
//! up", the API and the `ticket.finish` workflow node. It checks first and changes nothing when something would be
//! lost; otherwise it moves the ticket to Done, stops and removes what ran in its worktree and workspace, removes the
//! worktree (and the branch when asked), deletes the workspace Start made and pulls the base branch.

use crate::error::{ApiError, OrNotFound, codes};
use crate::log_err::LogErr;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use pdt_core::project::PullResult;
use pdt_core::session::SessionStatus;
use pdt_core::ticket::{Actor, AttemptState, TicketStage};
use pdt_core::workflow::api::{Run, RunStatus, StepStatus};
use pdt_store::AttemptInfo;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct FinishRequest {
    /// Stop the sessions and runs still going (else they block).
    pub stop_running: bool,
    /// Remove the worktree even with uncommitted changes (else they block).
    pub discard_changes: bool,
    /// Delete the branch with the worktree.
    pub delete_branch: bool,
    /// Only move to Done and pull: the worktree, workspace, sessions and runs stay.
    pub keep_worktree: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FinishOutcome {
    /// Done, and everything Start made is gone.
    Finished,
    /// Nothing changed: see `blocked`.
    Blocked,
    /// Done; the worktree and the rest were kept (`keepWorktree`).
    Kept,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FinishBlocked {
    /// Sessions still running.
    pub sessions: u32,
    /// Runs still going.
    pub runs: u32,
    /// The worktree has uncommitted changes.
    pub dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FinishResult {
    pub outcome: FinishOutcome,
    pub blocked: Option<FinishBlocked>,
    pub worktree_removed: bool,
    pub workspace_removed: bool,
    /// Why the branch stayed when it was to be deleted.
    pub branch_note: Option<String>,
    /// The base branch's pull (None when the project is gone).
    pub pull: Option<PullResult>,
}

/// Who finished it, when not the user: the workflow and run (recorded on the `moved` event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct By {
    pub workflow_id: String,
    pub workflow_name: String,
    pub run_id: String,
}

impl By {
    pub(crate) fn data(&self) -> serde_json::Value {
        serde_json::json!({"workflowId": self.workflow_id, "workflowName": self.workflow_name, "runId": self.run_id})
    }
}

/// One lock per ticket a finish is going on for (PDT-182): two finishes at once (the Pipeline's `merged → finish` and
/// Finish merged tickets) take turns, so only the first moves the ticket to Done and cleans up and the second sees it
/// finished already.
#[derive(Default)]
pub(crate) struct Finishes(std::sync::Mutex<HashMap<String, (TicketLock, usize)>>);

type TicketLock = std::sync::Arc<tokio::sync::Mutex<()>>;

/// A finish of a ticket, from before it waits for the ticket's lock until it ends; the last one removes the lock.
struct Entered<'a> {
    finishes: &'a Finishes,
    ticket_id: &'a str,
}

impl Finishes {
    fn enter<'a>(&'a self, ticket_id: &'a str) -> (Entered<'a>, TicketLock) {
        let mut all = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let (lock, count) = all.entry(ticket_id.to_string()).or_default();
        *count += 1;
        (Entered { finishes: self, ticket_id }, lock.clone())
    }
}

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        let mut all = self.finishes.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, count)) = all.get_mut(self.ticket_id) {
            *count -= 1;
            if *count == 0 {
                all.remove(self.ticket_id);
            }
        }
    }
}

/// `POST /api/tickets/{id}/finish`. The work runs in a task of its own, so it completes when the caller goes away
/// first: an agent finishing its own ticket is stopped by it (PDT-97), which used to leave the ticket in Done with its
/// worktree and workspace still there.
pub(crate) async fn finish_route(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<FinishRequest>, JsonRejection>,
) -> Result<Json<FinishResult>, ApiError> {
    let req = match payload {
        Ok(Json(r)) => r,
        Err(JsonRejection::MissingJsonContentType(_)) => FinishRequest::default(),
        Err(e) => return Err(e.into()),
    };
    let task = tokio::spawn(async move { finish_ticket(&st, &id, &req, None).await });
    Ok(Json(task.await.map_err(ApiError::internal)??))
}

/// What finishing (or stopping, PDT-159) would stop and remove: the attempt's running sessions, the sessions in its
/// worktree and workspace, the attempt's runs, the workspace Start made and whether the worktree has uncommitted changes.
pub(crate) struct Plan {
    pub running: Vec<String>,
    pub remove: HashSet<String>,
    /// Every run of the attempt, in any state.
    pub runs: Vec<String>,
    /// How many of `runs` are still going.
    pub active_runs: usize,
    pub workspace: Option<pdt_core::workspace::Workspace>,
    pub dirty: bool,
}

pub(crate) async fn plan(st: &AppState, info: &AttemptInfo) -> Result<Plan, ApiError> {
    let attempt = &info.attempt;
    let worktree_id = attempt.worktree_id.as_deref();
    let workspace = match &attempt.workspace_id {
        Some(id) => st.store.get_workspace(id).await?,
        None => None,
    };
    let tabs = workspace.as_ref().map(|w| w.tabs.clone()).unwrap_or_default();
    let tab_sessions: HashSet<String> = tabs.iter().filter_map(|t| t.session_id.clone()).collect();
    let sessions = st.store.list_sessions(&info.project_id).await?;
    let mut remove: HashSet<String> = tab_sessions.clone();
    let mut running = vec![];
    for s in &sessions {
        let in_worktree = worktree_id.is_some() && s.worktree_id.as_deref() == worktree_id;
        if in_worktree {
            remove.insert(s.id.clone());
        }
        let ours = in_worktree || tab_sessions.contains(&s.id) || s.attempt_id.as_deref() == Some(attempt.id.as_str());
        if ours && s.status == SessionStatus::Running {
            running.push(s.id.clone());
        }
    }
    // Sessions in the workspace that belong to another project.
    for id in tab_sessions.iter().filter(|id| !sessions.iter().any(|s| &s.id == *id)) {
        if let Some(s) = st.store.get_session(id).await?
            && s.status == SessionStatus::Running
        {
            running.push(s.id);
        }
    }
    // Every run of the attempt, in any state (PDT-160): in its worktree, on its branch or in its workspace.
    let run_tabs: HashSet<&str> = tabs.iter().filter_map(|t| t.run_id.as_deref()).collect();
    let mut runs: Vec<pdt_core::run::Run> = st
        .store
        .list_runs(&info.project_id)
        .await?
        .into_iter()
        .filter(|r| {
            (worktree_id.is_some() && r.worktree_id.as_deref() == worktree_id)
                || r.branch == attempt.branch
                || run_tabs.contains(r.id.as_str())
        })
        .collect();
    // Runs in the workspace that belong to another project.
    let elsewhere: Vec<&str> = run_tabs.into_iter().filter(|id| !runs.iter().any(|r| r.id == *id)).collect();
    for id in elsewhere {
        if let Some(r) = st.store.get_run(id).await? {
            runs.push(r);
        }
    }
    let active_runs = runs.iter().filter(|r| r.state.is_active()).count();
    let runs = runs.into_iter().map(|r| r.id).collect();
    let dirty = match info.worktree_path.clone().map(PathBuf::from) {
        Some(path) => tokio::task::spawn_blocking(move || path.is_dir() && pdt_git::is_dirty(&path).unwrap_or(true))
            .await
            .unwrap_or(true),
        None => false,
    };
    Ok(Plan { running, remove, runs, active_runs, workspace, dirty })
}

/// Moves the ticket to Done, then ends its active attempt (as merged: Done says the work landed) and cancels its open
/// workflow runs but `by`'s own: nothing works on a Done ticket, holds a slot under the caps or keeps an agent step
/// waiting on a session finishing removes.
async fn move_to_done(st: &AppState, ticket_id: &str, by: Option<&By>) -> Result<(), ApiError> {
    let actor = if by.is_some() { Actor::System } else { Actor::User };
    let data = by.map(By::data);
    st.store.move_ticket_by(ticket_id, TicketStage::Done, actor, data.as_ref(), now_ms()).await?;
    if let Err(e) = crate::start::end_for_close(st, ticket_id, TicketStage::Done, actor).await {
        tracing::warn!(ticket = %ticket_id, error = %e.message(), "could not end the finished ticket's attempt");
    }
    cancel_runs(st, ticket_id, by).await;
    Ok(())
}

/// Cancels the ticket's runs still going, except `by`'s run and the runs that called it, and the runs about to
/// finish the ticket themselves (PDT-182), which then find it finished already. Each cancel takes
/// the engine lock: a `ticket.finish` step's body runs outside it (PDT-128).
async fn cancel_runs(st: &AppState, ticket_id: &str, by: Option<&By>) {
    let filter = pdt_store::RunFilter { ticket_id: Some(ticket_id.to_string()), limit: 500, ..Default::default() };
    let runs = match st.store.list_workflow_runs(&filter).await {
        Ok(r) => r,
        Err(e) => return tracing::warn!(ticket = %ticket_id, error = %e, "could not list the ticket's workflow runs"),
    };
    let mut finishing: Vec<String> = by.map(|b| b.run_id.clone()).into_iter().collect();
    for run in runs.iter().filter(|r| r.status == RunStatus::Running) {
        if heading_to_finish(st, run).await {
            finishing.push(run.id.clone());
        }
    }
    let mut keep = HashSet::new();
    for run in finishing {
        let mut next = Some(run);
        while let Some(id) = next.filter(|id| keep.insert(id.clone())) {
            next = st.store.get_workflow_run(&id).await.ok().flatten().and_then(|r| r.parent_run_id);
        }
    }
    for run in runs.iter().filter(|r| !r.status.is_finished() && !keep.contains(&r.id)) {
        let why = "cancelled: the ticket was finished";
        let done = {
            let _engine = st.engine_lock.lock().await;
            crate::workflow::engine::cancel_because(st, &run.id, why).await
        };
        if let Err(e) = done {
            tracing::warn!(ticket = %ticket_id, run = %run.id, error = %e, "could not cancel the finished ticket's run");
        }
    }
}

/// Whether a run advancing (not waiting) is in a `ticket.finish` step, or has a step to run with an edge straight to
/// one (Finish merged tickets' gate that answers itself, the Pipeline's merge). Only one step away: in the Pipeline
/// nearly every step leads to its finish some time, and a run further back works on a Done ticket, so it is cancelled.
async fn heading_to_finish(st: &AppState, run: &Run) -> bool {
    let Ok((_, doc)) = crate::workflow::engine::load(st, run).await else { return false };
    let Ok(steps) = st.store.list_steps(&run.id).await else { return false };
    let finish = |id: &str| doc.node(id).is_some_and(|n| n.kind == "ticket.finish");
    steps
        .iter()
        .filter(|s| matches!(s.status, StepStatus::Pending | StepStatus::Running))
        .any(|s| finish(&s.node_id) || doc.edges.iter().any(|e| e.from == s.node_id && finish(&e.to)))
}

async fn pull(st: &AppState, project_id: &str, branch: Option<String>) -> Option<PullResult> {
    let project = st.store.get_project(project_id).await.ok().flatten()?;
    let branch = branch.unwrap_or_else(|| project.base_branch.clone());
    match crate::projects::pull_branch(&project, branch).await {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::info!(project = %project_id, error = %e.message(), "could not pull the base branch after finishing");
            None
        }
    }
}

/// Finishes a ticket by its latest attempt (see the module docs), after any other finish of it going on. Problems
/// after the checks are logged and reported in the result, not failed: the ticket is Done by then.
pub(crate) async fn finish_ticket(
    st: &AppState,
    ticket_id: &str,
    req: &FinishRequest,
    by: Option<&By>,
) -> Result<FinishResult, ApiError> {
    let (_entered, lock) = st.finishes.enter(ticket_id);
    let _one_at_a_time = lock.lock().await;
    finish_alone(st, ticket_id, req, by).await
}

async fn finish_alone(
    st: &AppState,
    ticket_id: &str,
    req: &FinishRequest,
    by: Option<&By>,
) -> Result<FinishResult, ApiError> {
    let ticket = st.store.get_ticket_summary(ticket_id).await?.or_404("ticket")?;
    let info = match &ticket.attempt {
        Some(a) => st.store.get_attempt(&a.id).await?,
        None => None,
    };
    let mut result = FinishResult {
        outcome: FinishOutcome::Finished,
        blocked: None,
        worktree_removed: false,
        workspace_removed: false,
        branch_note: None,
        pull: None,
    };
    let Some(info) = info else {
        move_to_done(st, ticket_id, by).await?;
        result.pull = pull(st, &ticket.project_id, None).await;
        return Ok(result);
    };
    // Finished already (a workflow's merge is finished by the workflow and by Finish merged tickets, PDT-181): nothing
    // is left to clean up, so a finish that comes after another one's cleanup cancels no runs. Two finishes at the
    // same moment take turns (`finish_ticket`), so the second sees the first's.
    if ticket.stage.is_closed() && info.attempt.state != AttemptState::Active && info.attempt.worktree_id.is_none() {
        return Ok(result);
    }
    let key = &info.ticket_key;
    let base = Some(info.attempt.base_branch.clone());
    if req.keep_worktree {
        move_to_done(st, ticket_id, by).await?;
        result.outcome = FinishOutcome::Kept;
        result.pull = pull(st, &info.project_id, base).await;
        return Ok(result);
    }
    let p = plan(st, &info).await?;
    let blocked = FinishBlocked {
        sessions: if req.stop_running { 0 } else { u32::try_from(p.running.len()).unwrap_or(u32::MAX) },
        runs: if req.stop_running { 0 } else { u32::try_from(p.active_runs).unwrap_or(u32::MAX) },
        dirty: p.dirty && !req.discard_changes,
    };
    if blocked != FinishBlocked::default() {
        result.outcome = FinishOutcome::Blocked;
        result.blocked = Some(blocked);
        return Ok(result);
    }
    move_to_done(st, ticket_id, by).await?;
    let done = clean_up(st, &info, &p, req.discard_changes, req.delete_branch).await;
    (result.worktree_removed, result.workspace_removed, result.branch_note) =
        (done.worktree_removed, done.workspace_removed, done.branch_note);
    // The ticket's tabs elsewhere close too.
    match st.store.list_workspaces().await {
        Err(e) => tracing::warn!(error = %e, "could not read the workspaces to close the ticket's tabs"),
        Ok(all) => {
            for tab in all.iter().flat_map(|w| &w.tabs) {
                if tab.ticket_id.as_deref() == Some(ticket_id) {
                    st.store.close_tab(&tab.id).await.log_warn("close a finished ticket's tab");
                }
            }
        }
    }
    result.pull = pull(st, &info.project_id, base).await;
    tracing::info!(ticket = %key, by = ?by.map(|b| &b.workflow_name), "finished: moved to Done and cleaned up");
    Ok(result)
}

/// What [`clean_up`] removed.
pub(crate) struct Cleanup {
    pub worktree_removed: bool,
    pub workspace_removed: bool,
    /// Why the branch stayed when it was to be deleted.
    pub branch_note: Option<String>,
}

/// Stops and removes what `p` found running in the attempt's worktree and workspace, removes the worktree (with
/// `discard`, uncommitted changes too; with `delete_branch`, the branch) and deletes the workspace Start made. Problems
/// are logged, not failed. Finishing and stopping a ticket with its worktree (PDT-159) share it.
pub(crate) async fn clean_up(
    st: &AppState,
    info: &AttemptInfo,
    p: &Plan,
    discard: bool,
    delete_branch: bool,
) -> Cleanup {
    let key = &info.ticket_key;
    let mut out = Cleanup { worktree_removed: false, workspace_removed: false, branch_note: None };
    // The attempt's own sessions elsewhere only stop; the worktree's and the workspace's go.
    for id in p.running.iter().filter(|id| !p.remove.contains(*id)) {
        if let Err(e) = crate::sessions::stop(st, id).await {
            tracing::warn!(ticket = %key, session = %id, error = %e.message(), "could not stop the session");
        }
    }
    for id in &p.remove {
        if let Err(e) = crate::sessions::stop_and_remove(st, id).await
            && e.code() != codes::NOT_FOUND
        {
            tracing::warn!(ticket = %key, session = %id, error = %e.message(), "could not remove the session");
        }
    }
    for id in &p.runs {
        if let Err(e) = crate::runs::stop_and_remove(st, id).await {
            tracing::warn!(ticket = %key, run = %id, error = %e.message(), "could not remove the run");
        }
    }
    if let Some(id) = info.attempt.worktree_id.as_deref() {
        // A stopped process takes a moment to let go of the worktree.
        let mut tries = 0;
        loop {
            match crate::worktrees::remove_worktree(st, id, discard, delete_branch).await {
                Err(e) if e.code() == codes::WORKTREE_BUSY && tries < 20 => {
                    tries += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                Err(e) if e.code() == codes::BRANCH_NOT_DELETED => {
                    out.worktree_removed = true;
                    out.branch_note = Some(e.message().to_string());
                    break;
                }
                Err(e) if e.code() != codes::NOT_FOUND => {
                    tracing::warn!(ticket = %key, error = %e.message(), "could not remove the worktree");
                    break;
                }
                _ => {
                    out.worktree_removed = true;
                    break;
                }
            }
        }
    }
    if let Some(w) = &p.workspace {
        match st.store.delete_workspace(&w.id, true).await {
            Ok(r) => out.workspace_removed = r == pdt_store::DeleteWorkspace::Deleted,
            Err(e) => tracing::warn!(ticket = %key, error = %e, "could not delete the ticket's workspace"),
        }
    }
    out
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::post;
    axum::Router::new().route("/tickets/{id}/finish", post(finish_route))
}
