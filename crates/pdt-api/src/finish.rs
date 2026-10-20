//! `ticket.finish` (Phase 7a spec §5.1): one server command for the merged banner, Merge…'s "Move to Done and clean
//! up", the API and the `ticket.finish` workflow node. It checks first and changes nothing when something would be
//! lost; otherwise it moves the ticket to Done, stops and removes what ran in its worktree and workspace, removes the
//! worktree (and the branch when asked), deletes the workspace Start made and pulls the base branch.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use pdt_core::project::PullResult;
use pdt_core::session::SessionStatus;
use pdt_core::ticket::{Actor, TicketStage};
use pdt_store::AttemptInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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
pub(crate) struct By {
    pub workflow_id: String,
    pub workflow_name: String,
    pub run_id: String,
}

impl By {
    pub(crate) fn data(&self) -> serde_json::Value {
        serde_json::json!({"workflowId": self.workflow_id, "workflowName": self.workflow_name, "runId": self.run_id})
    }
}

/// `POST /api/tickets/{id}/finish`.
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
    Ok(Json(finish_ticket(&st, &id, &req, None).await?))
}

/// What finishing would stop and remove.
struct Plan {
    running: Vec<String>,
    remove: HashSet<String>,
    runs: Vec<String>,
    workspace: Option<pdt_core::workspace::Workspace>,
    dirty: bool,
}

async fn plan(st: &AppState, info: &AttemptInfo) -> Result<Plan, ApiError> {
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
    let run_tabs: HashSet<&str> = tabs.iter().filter_map(|t| t.run_id.as_deref()).collect();
    let runs = st
        .store
        .list_active_runs()
        .await?
        .into_iter()
        .filter(|r| {
            (worktree_id.is_some() && r.worktree_id.as_deref() == worktree_id) || run_tabs.contains(r.id.as_str())
        })
        .map(|r| r.id)
        .collect();
    let dirty = match info.worktree_path.clone().map(PathBuf::from) {
        Some(path) => tokio::task::spawn_blocking(move || path.is_dir() && pdt_git::is_dirty(&path).unwrap_or(true))
            .await
            .unwrap_or(true),
        None => false,
    };
    Ok(Plan { running, remove, runs, workspace, dirty })
}

async fn move_to_done(st: &AppState, ticket_id: &str, by: Option<&By>) -> Result<(), ApiError> {
    let actor = if by.is_some() { Actor::System } else { Actor::User };
    let data = by.map(By::data);
    st.store.move_ticket_by(ticket_id, TicketStage::Done, actor, data.as_ref(), now_ms()).await?;
    Ok(())
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

/// Finishes a ticket by its latest attempt (see the module docs). Problems after the checks are logged and reported
/// in the result, not failed: the ticket is Done by then.
pub(crate) async fn finish_ticket(
    st: &AppState,
    ticket_id: &str,
    req: &FinishRequest,
    by: Option<&By>,
) -> Result<FinishResult, ApiError> {
    let ticket = st.store.get_ticket_summary(ticket_id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
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
        runs: if req.stop_running { 0 } else { u32::try_from(p.runs.len()).unwrap_or(u32::MAX) },
        dirty: p.dirty && !req.discard_changes,
    };
    if blocked != FinishBlocked::default() {
        result.outcome = FinishOutcome::Blocked;
        result.blocked = Some(blocked);
        return Ok(result);
    }
    move_to_done(st, ticket_id, by).await?;

    // The attempt's own sessions elsewhere only stop; the worktree's and the workspace's go.
    for id in p.running.iter().filter(|id| !p.remove.contains(*id)) {
        if let Err(e) = crate::sessions::stop(st, id).await {
            tracing::warn!(ticket = %key, session = %id, error = %e.message(), "could not stop the session");
        }
    }
    for id in &p.remove {
        if let Err(e) = crate::sessions::stop_and_remove(st, id).await
            && e.code() != "not_found"
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
            match crate::worktrees::remove_worktree(st, id, req.discard_changes, req.delete_branch).await {
                Err(e) if e.code() == "worktree_busy" && tries < 20 => {
                    tries += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                Err(e) if e.code() == "branch_not_deleted" => {
                    result.worktree_removed = true;
                    result.branch_note = Some(e.message().to_string());
                    break;
                }
                Err(e) if e.code() != "not_found" => {
                    tracing::warn!(ticket = %key, error = %e.message(), "could not remove the merged worktree");
                    break;
                }
                _ => {
                    result.worktree_removed = true;
                    break;
                }
            }
        }
    }

    if let Some(w) = &p.workspace {
        // A ticket tab left elsewhere closes too; the workspace goes with every tab in it.
        if let Ok(all) = st.store.list_workspaces().await {
            for tab in all.iter().flat_map(|w| &w.tabs) {
                if tab.workspace_id != w.id && tab.ticket_id.as_deref() == Some(ticket_id) {
                    let _ = st.store.close_tab(&tab.id).await;
                }
            }
        }
        match st.store.delete_workspace(&w.id, true).await {
            Ok(r) => result.workspace_removed = r == pdt_store::DeleteWorkspace::Deleted,
            Err(e) => tracing::warn!(ticket = %key, error = %e, "could not delete the ticket's workspace"),
        }
    }
    result.pull = pull(st, &info.project_id, base).await;
    tracing::info!(ticket = %key, by = ?by.map(|b| &b.workflow_name), "finished: moved to Done and cleaned up");
    Ok(result)
}
