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
use std::path::PathBuf;
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

/// What the wind-down after Won't do, Done or Abandon (PDT-339) is to do once the attempt's work is stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindDown {
    /// Done: only stop what still runs; the automatic cleanup takes the worktree later.
    Stop,
    /// Found merged outside PDT: stop the sessions and runs, but leave the workflow runs to take the merge (the
    /// Pipeline's Accept ends on it).
    Merged,
    /// Won't do or Abandon: also close its pull request and remove the worktree, branch and workspace as the project
    /// says, unless something would be lost.
    Close,
}

/// What became of a ticket's work after it closed or its attempt was abandoned, for the timeline.
#[derive(Debug, Default)]
struct WoundDown {
    runs_cancelled: u32,
    sessions_stopped: u32,
    runs_stopped: u32,
    notes: Vec<String>,
}

/// After a ticket closed or its attempt was abandoned (`attempt`, which was active until then): cancels the ticket's
/// open workflow runs (the attempt's, and those started before `since`, but `keep_run` and the runs that called it),
/// stops the attempt's sessions and runs, and with [`WindDown::Close`] closes its open pull request and cleans up
/// ([`clean_up_closed`]). Records what it did on the ticket's timeline. Problems are logged and noted, not failed.
pub(crate) async fn wind_down(st: &AppState, attempt: &str, since: i64, how: WindDown, keep_run: Option<&str>) {
    let Ok(Some(info)) = st.store.get_attempt(attempt).await else { return };
    let mut done = WoundDown::default();
    let keep = keep_chain(st, keep_run).await;
    if how == WindDown::Merged {
        // No stage of the Pipeline is left to wait in (the board showed it at Accept).
        if let Err(e) = st.store.set_pipeline_stage(&info.ticket_id, None).await {
            tracing::warn!(ticket = %info.ticket_key, error = %e, "could not clear the merged ticket's pipeline stage");
        }
    } else {
        done.runs_cancelled = cancel_runs_but(st, &info, since, &keep).await;
    }
    match crate::finish::plan(st, &info).await {
        Ok(p) => {
            for id in &p.running {
                match crate::sessions::stop(st, id).await {
                    Ok(()) => done.sessions_stopped += 1,
                    Err(e) => tracing::warn!(ticket = %info.ticket_key, session = %id, error = %e.message(),
                        "could not stop the session"),
                }
            }
            for id in &p.runs {
                let active = matches!(st.store.get_run(id).await, Ok(Some(r)) if r.state.is_active());
                if active && crate::run_manager::stop(st, id, None).await {
                    done.runs_stopped += 1;
                }
            }
        }
        Err(e) => tracing::warn!(ticket = %info.ticket_key, error = %e.message(), "could not find the attempt's work"),
    }
    // A merged attempt's pull request and branch are the merge's; nothing more to close.
    let merged =
        st.store.get_attempt(attempt).await.ok().flatten().is_some_and(|a| a.attempt.state == AttemptState::Merged);
    if how == WindDown::Close && !merged && !info.attempt.branch.is_empty() {
        if let Some(note) = close_pr(st, &info).await {
            done.notes.push(note);
        }
        done.notes.extend(clean_up_closed(st, &info, false).await);
    }
    note_on_timeline(st, &info, &done).await;
}

/// The run `id` and the runs that called it: a workflow's own step closing the ticket goes on.
async fn keep_chain(st: &AppState, id: Option<&str>) -> std::collections::HashSet<String> {
    let mut keep = std::collections::HashSet::new();
    let mut next = id.map(String::from);
    while let Some(id) = next.filter(|id| keep.insert(id.clone())) {
        next = st.store.get_workflow_run(&id).await.ok().flatten().and_then(|r| r.parent_run_id);
    }
    keep
}

/// [`cancel_runs`] but the runs in `keep`.
async fn cancel_runs_but(
    st: &AppState,
    info: &AttemptInfo,
    since: i64,
    keep: &std::collections::HashSet<String>,
) -> u32 {
    if keep.is_empty() {
        return cancel_runs(st, info, since).await;
    }
    let filter = pdt_store::RunFilter { ticket_id: Some(info.ticket_id.clone()), limit: 500, ..Default::default() };
    let Ok(runs) = st.store.list_workflow_runs(&filter).await else { return 0 };
    let mut cancelled = 0;
    for run in runs.iter().filter(|r| {
        !r.status.is_finished()
            && !keep.contains(&r.id)
            && (r.attempt_id.as_deref() == Some(info.attempt.id.as_str()) || r.started_at <= since)
    }) {
        let _engine = st.engine_lock.lock().await;
        if crate::workflow::engine::cancel_because(st, &run.id, "cancelled: the ticket's work was stopped")
            .await
            .is_ok()
        {
            cancelled += 1;
        }
    }
    cancelled
}

/// Closes the attempt's open pull request with a comment saying why. A note for the timeline: closed, or why not
/// (None: there was none, or it merged).
async fn close_pr(st: &AppState, info: &AttemptInfo) -> Option<String> {
    let forge = crate::forge::project_forge(st, &info.project_id, &info.repo_path).await?;
    let (api, repo, branch) = (forge.api.clone(), forge.repo.clone(), info.attempt.branch.clone());
    let body = format!("Closed: the ticket was set to Won't do in {}.", pdt_core::brand::SHORT_NAME);
    let closed = tokio::task::spawn_blocking(move || -> Result<Option<u32>, pdt_forge::ForgeError> {
        let Some(pr) = api.change_request(&repo, &branch)?.filter(|p| p.state == pdt_core::pr::PrState::Open) else {
            return Ok(None);
        };
        // The comment is a courtesy: the close is what matters.
        if let Err(e) = api.comment_change_request(&repo, pr.number, &body) {
            tracing::info!(pr = pr.number, error = %e, "could not comment on the pull request before closing it");
        }
        api.close_change_request(&repo, pr.number).map(|()| Some(pr.number))
    })
    .await;
    match closed {
        Ok(Ok(Some(n))) => {
            crate::github::refresh_attempt(st, info).await;
            Some(format!("Closed pull request #{n}."))
        }
        Ok(Ok(None)) => None,
        Ok(Err(e)) => {
            tracing::warn!(ticket = %info.ticket_key, error = %e, "could not close the pull request");
            Some(format!("Could not close the pull request: {e}. Close it on the forge."))
        }
        Err(e) => Some(format!("Could not close the pull request: {e}.")),
    }
}

/// What a closed attempt's worktree holds that removing it would lose, if anything: a reason for the timeline.
fn would_lose(info: &AttemptInfo) -> Option<String> {
    let path = PathBuf::from(info.worktree_path.as_deref()?);
    if !path.is_dir() {
        return None;
    }
    let repo = PathBuf::from(&info.repo_path);
    if pdt_git::is_dirty(&path).unwrap_or(true) {
        return Some("it has uncommitted changes".into());
    }
    let Ok(u) = pdt_git::unpushed(&path) else { return Some("its changes could not be read".into()) };
    if u.ahead > 0 {
        return Some(format!("it has {} commit{} not pushed", u.ahead, if u.ahead == 1 { "" } else { "s" }));
    }
    // Never pushed (or no remote): its own commits are only here.
    if u.upstream.is_none() {
        let head = pdt_git::rev_parse(&repo, &format!("refs/heads/{}", info.attempt.branch))?;
        let own = pdt_git::commits_between(&repo, &info.base_commit, &head).map_or(1, |c| c.len());
        if own > 0 {
            return Some("its commits are not pushed".into());
        }
    }
    None
}

/// Removes a closed attempt's worktree, its branch (and the remote branch, when PDT named it for the ticket and the
/// remote holds nothing more) and the workspace Start made, as the project's Won't do setting says; with `now`, at
/// once whatever the setting (the Clean up action). Keeps it all when something would be lost, unless `now`. Notes
/// for the timeline.
pub(crate) async fn clean_up_closed(st: &AppState, info: &AttemptInfo, now: bool) -> Vec<String> {
    use pdt_core::storage::WontDoCleanup;
    let mut notes = vec![];
    if info.attempt.worktree_id.is_none() {
        return notes;
    }
    if !now {
        let setting = st.store.get_project_storage(&info.project_id).await.unwrap_or_default();
        match setting.wont_do {
            WontDoCleanup::Now => {}
            WontDoCleanup::Later => {
                let days = setting.wont_do_after_days.max(1);
                notes.push(format!("Kept the worktree for {days} day{}, as the project says.", plural_s(days)));
                return notes;
            }
            WontDoCleanup::Keep => {
                notes.push("Kept the worktree, as the project says.".into());
                return notes;
            }
        }
        let i = info.clone();
        let lose =
            tokio::task::spawn_blocking(move || would_lose(&i)).await.unwrap_or(Some("it could not be read".into()));
        if let Some(why) = lose {
            notes.push(format!(
                "Kept the worktree and branch {}: {why}. Clean up removes them anyway.",
                info.attempt.branch
            ));
            return notes;
        }
    }
    // Whether to delete the remote branch, decided before the local one goes.
    let i = info.clone();
    let remote = tokio::task::spawn_blocking(move || remote_branch_to_delete(&i)).await.ok().flatten();
    let p = match crate::finish::plan(st, info).await {
        Ok(p) => p,
        Err(e) => {
            notes.push(format!("Could not clean up the worktree: {}.", e.message()));
            return notes;
        }
    };
    let done = crate::finish::clean_up(st, info, &p, true, true).await;
    if done.worktree_removed {
        let mut what = vec!["the worktree".to_string()];
        if done.branch_note.is_none() {
            what.push(format!("the branch {}", info.attempt.branch));
        }
        if let Some(r) = remote {
            let (repo, branch, to) = (PathBuf::from(&info.repo_path), info.attempt.branch.clone(), r.clone());
            let refspec = format!(":refs/heads/{branch}");
            let pushed = tokio::task::spawn_blocking(move || {
                pdt_git::push(&repo, &to, &[refspec.as_str()], crate::start::FETCH_TIMEOUT)
            })
            .await;
            match pushed {
                Ok(Ok(pdt_git::PushOutcome::Pushed)) => what.push(format!("its copy on {r}")),
                Ok(Ok(pdt_git::PushOutcome::Rejected(e))) => {
                    notes.push(format!("Could not delete the branch on {r}: {}.", e.trim()))
                }
                Ok(Err(e)) => notes.push(format!("Could not delete the branch on {r}: {e}.")),
                Err(e) => notes.push(format!("Could not delete the branch on {r}: {e}.")),
            }
        }
        if done.workspace_removed {
            what.push("the ticket's workspace".into());
        }
        notes.insert(0, format!("Removed {}.", join_and(&what)));
        if let Some(n) = done.branch_note {
            notes.push(format!("Kept the branch: {n}."));
        }
        // The ticket's tabs elsewhere close too.
        if let Ok(all) = st.store.list_workspaces().await {
            for tab in all.iter().flat_map(|w| &w.tabs) {
                if tab.ticket_id.as_deref() == Some(info.ticket_id.as_str()) {
                    let _ = st.store.close_tab(&tab.id).await;
                }
            }
        }
    } else {
        notes.push("Could not remove the worktree; it is kept. Clean up tries again.".into());
    }
    notes
}

/// The remote to delete the attempt's branch from: `origin`, when PDT named the branch for the ticket and origin's
/// copy is the local branch's head (so nothing pushed from elsewhere is lost).
fn remote_branch_to_delete(info: &AttemptInfo) -> Option<String> {
    if !pdt_core::start::is_attempt_branch(&info.attempt.branch, &info.ticket_key) {
        return None;
    }
    let repo = PathBuf::from(&info.repo_path);
    pdt_git::remote_url(&repo, "origin")?;
    let local = pdt_git::rev_parse(&repo, &format!("refs/heads/{}", info.attempt.branch))?;
    let remote = pdt_git::rev_parse(&repo, &format!("refs/remotes/origin/{}", info.attempt.branch))?;
    (local == remote).then(|| "origin".to_string())
}

fn plural_s(n: u32) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// One timeline comment for what [`wind_down`] did; none when it did nothing.
async fn note_on_timeline(st: &AppState, info: &AttemptInfo, done: &WoundDown) {
    let mut stopped = vec![];
    if done.sessions_stopped > 0 {
        stopped.push(format!("{} session{}", done.sessions_stopped, plural_s(done.sessions_stopped)));
    }
    if done.runs_stopped > 0 {
        stopped.push(format!("{} run{}", done.runs_stopped, plural_s(done.runs_stopped)));
    }
    if done.runs_cancelled > 0 {
        let n = done.runs_cancelled;
        stopped.push(format!("{n} workflow run{}", plural_s(n)));
    }
    let mut lines = vec![];
    if !stopped.is_empty() {
        lines.push(format!("Stopped {}.", join_and(&stopped)));
    }
    lines.extend(done.notes.iter().cloned());
    if lines.is_empty() {
        return;
    }
    let body = lines.join(" ");
    if let Err(e) = st.store.add_comment_by(&info.ticket_id, &body, Actor::System, None, now_ms()).await {
        tracing::warn!(ticket = %info.ticket_key, error = %e, "could not note the cleanup on the timeline");
    }
}
