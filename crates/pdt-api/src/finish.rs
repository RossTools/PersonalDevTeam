//! Finishing a ticket by itself once its pull request merges (Merge…'s "Move to Done and clean up once it merges",
//! PDT-75): the server side of the web's `finishMerged`, without its questions.

use crate::routes::AppState;
use pdt_core::session::SessionStatus;
use pdt_core::ticket::{TicketPatch, TicketStage};
use pdt_store::AttemptInfo;
use std::collections::HashSet;
use std::path::PathBuf;

/// Called once, when merge detection ends an attempt as merged. Does nothing unless Merge… asked for it.
pub(crate) fn on_merged(st: &AppState, info: &AttemptInfo) {
    if !info.finish_on_merge {
        return;
    }
    let (st, info) = (st.clone(), info.clone());
    tokio::spawn(async move { finish(&st, &info).await });
}

/// Moves the ticket to Done, stops and removes the sessions and runs in the worktree and the workspace Start made,
/// removes the worktree (with the branch when asked), deletes that workspace with its tabs and fast-forwards the base
/// branch. Uncommitted changes in the worktree are never discarded: then nothing happens and the merged banner stays
/// for the user. Problems are logged.
pub(crate) async fn finish(st: &AppState, info: &AttemptInfo) {
    let key = &info.ticket_key;
    if let Some(path) = info.worktree_path.clone().map(PathBuf::from) {
        let dirty =
            tokio::task::spawn_blocking(move || path.is_dir() && pdt_git::is_dirty(&path).unwrap_or(true)).await;
        if dirty.unwrap_or(true) {
            tracing::info!(ticket = %key, "merged, but the worktree has uncommitted changes: left for the user to finish");
            return;
        }
    }
    let done = TicketPatch { stage: Some(TicketStage::Done), ..TicketPatch::default() };
    match done.validate() {
        Ok(p) => {
            if let Err(e) = st.store.patch_ticket(&info.ticket_id, &p, crate::routes::now_ms()).await {
                tracing::warn!(ticket = %key, error = %e, "could not move the merged ticket to Done");
                return;
            }
        }
        Err(e) => {
            tracing::warn!(ticket = %key, error = %e, "could not move the merged ticket to Done");
            return;
        }
    }
    let attempt = &info.attempt;
    let worktree_id = attempt.worktree_id.as_deref();
    let workspace = match &attempt.workspace_id {
        Some(id) => st.store.get_workspace(id).await.ok().flatten(),
        None => None,
    };
    let tabs = workspace.as_ref().map(|w| w.tabs.clone()).unwrap_or_default();

    // The attempt's own sessions elsewhere only stop; the worktree's and the workspace's go.
    let sessions = st.store.list_sessions(&info.project_id).await.unwrap_or_default();
    let mut remove: HashSet<String> = tabs.iter().filter_map(|t| t.session_id.clone()).collect();
    for s in &sessions {
        if worktree_id.is_some() && s.worktree_id.as_deref() == worktree_id {
            remove.insert(s.id.clone());
        } else if s.attempt_id.as_deref() == Some(attempt.id.as_str())
            && s.status == SessionStatus::Running
            && !remove.contains(&s.id)
            && let Err(e) = crate::sessions::stop(st, &s.id).await
        {
            tracing::warn!(ticket = %key, session = %s.id, error = %e.message(), "could not stop the session");
        }
    }
    for id in &remove {
        if let Err(e) = crate::sessions::stop_and_remove(st, id).await {
            tracing::warn!(ticket = %key, session = %id, error = %e.message(), "could not remove the session");
        }
    }
    let run_tabs: HashSet<&str> = tabs.iter().filter_map(|t| t.run_id.as_deref()).collect();
    for r in st.store.list_active_runs().await.unwrap_or_default() {
        let ours =
            (worktree_id.is_some() && r.worktree_id.as_deref() == worktree_id) || run_tabs.contains(r.id.as_str());
        if ours && let Err(e) = crate::runs::stop_and_remove(st, &r.id).await {
            tracing::warn!(ticket = %key, run = %r.id, error = %e.message(), "could not remove the run");
        }
    }

    if let Some(id) = worktree_id {
        // A stopped process takes a moment to let go of the worktree.
        let mut tries = 0;
        loop {
            match crate::worktrees::remove_worktree(st, id, false, info.finish_delete_branch).await {
                Err(e) if e.code() == "worktree_busy" && tries < 20 => {
                    tries += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                Err(e) if e.code() != "not_found" => {
                    tracing::warn!(ticket = %key, error = %e.message(), "could not remove the merged worktree");
                    break;
                }
                _ => break,
            }
        }
    }

    if let Some(w) = &workspace {
        // A ticket tab left elsewhere closes too; the workspace goes with every tab in it.
        if let Ok(all) = st.store.list_workspaces().await {
            for tab in all.iter().flat_map(|w| &w.tabs) {
                if tab.workspace_id != w.id && tab.ticket_id.as_deref() == Some(info.ticket_id.as_str()) {
                    let _ = st.store.close_tab(&tab.id).await;
                }
            }
        }
        if let Err(e) = st.store.delete_workspace(&w.id, true).await {
            tracing::warn!(ticket = %key, error = %e, "could not delete the ticket's workspace");
        }
    }

    let (repo, base) = (PathBuf::from(&info.repo_path), attempt.base_branch.clone());
    let pulled =
        tokio::task::spawn_blocking(move || pdt_git::fast_forward(&repo, &base, crate::projects::PULL_FETCH_TIMEOUT))
            .await;
    if let Ok(Err(e)) = pulled {
        tracing::info!(ticket = %key, error = %e, "could not pull the base branch after the merge");
    }
    tracing::info!(ticket = %key, "merged: moved to Done and cleaned up");
}
