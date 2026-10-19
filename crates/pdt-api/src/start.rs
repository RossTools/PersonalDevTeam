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
use pdt_core::conversation::{PermissionMode, SessionMode};
use pdt_core::session::{SessionKind, ValidSession};
use pdt_core::start::{StartMode, StartResult, StartTicket, ValidStart, started_session_title, ticket_prompt};
use pdt_core::ticket::{Actor, AttemptState, EventKind, LinkDirection, LinkKind, Ticket, TicketStage};
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

/// The ticket with its overlaps.
async fn load_ticket(st: &AppState, id: &str) -> Result<Ticket, ApiError> {
    let t = st.store.get_ticket(id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
    crate::overlap::decorate(st, t).await
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
    // Overlap with the project's active tickets: what this one would share once started.
    let overlaps = ticket.overlaps.clone();
    if (!blockers.is_empty() || !overlaps.is_empty()) && v.mode == StartMode::Check {
        let mut why = vec![];
        if !blockers.is_empty() {
            why.push(format!("is blocked by {}, which {} not done yet", blockers.join(", "), plural(blockers.len())));
        }
        if !overlaps.is_empty() {
            why.push(crate::overlap::describe(&overlaps));
        }
        return Err(conflict("start_blocked", format!("{key} {}", why.join("; it "))));
    }
    if !v.new_workspace {
        crate::workspaces::check_placement(&st, v.tab.as_ref()).await?;
    }
    let project =
        st.store.get_project(&ticket.summary.project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let base = v.base.clone().unwrap_or_else(|| project.base_branch.clone());
    // Start from a chat: its worktree and branch, and the chat itself instead of a new session.
    let chat = match &v.from_session {
        Some(sid) => Some(startable_chat(&st, sid, &project).await?),
        None => None,
    };
    let (wt, branch) = match &chat {
        Some((_, wt)) => (wt.clone(), wt.branch.clone()),
        None => {
            let branch = v.branch.clone().unwrap_or_else(|| ticket.default_branch.clone());
            let existing = st.store.list_worktrees(&project.id).await?.into_iter().find(|w| w.branch == branch);
            let wt = match existing {
                Some(w) => w,
                None => crate::worktrees::make_worktree(&st, &project, &branch, &base, true).await?,
            };
            (wt, branch)
        }
    };
    let extra = if blockers.is_empty() && overlaps.is_empty() {
        json!({})
    } else {
        let overlaps: Vec<_> =
            overlaps.iter().map(|o| json!({"key": o.key, "level": o.level, "areas": o.areas})).collect();
        json!({"forced": true, "blockers": blockers, "overlaps": overlaps,
               "level": crate::overlap::worst(&ticket.overlaps)})
    };
    let attempt_id = uuid::Uuid::now_v7().to_string();
    let started = st
        .store
        .start_attempt(&attempt_id, &id, Some(&wt.id), &branch, &base, &wt.base_commit, extra, now_ms())
        .await?;
    drop(guard);

    if let Some((chat, _)) = chat {
        let linked = st.store.set_session_attempt(&chat.id, &attempt_id).await;
        if !matches!(linked, Ok(true)) {
            if let Err(u) = st.store.undo_start(&started).await {
                tracing::error!(ticket = %id, error = %u, "could not undo a failed start");
            }
            return Err(match linked {
                Err(e) => e.into(),
                _ => conflict("chat_not_startable", "that chat already works on a ticket".into()),
            });
        }
        let data = json!({"sessionId": chat.id, "agent": chat.agent, "attemptId": attempt_id, "fromChat": true});
        if let Err(e) =
            st.store.add_ticket_event(&id, EventKind::Session, Actor::User, None, Some(data), now_ms()).await
        {
            tracing::warn!(ticket = %id, error = %e, "could not record the chat on the timeline");
        }
        open_tab_before_chat(&st, &project.id, &key, &id, &attempt_id, &chat.id, &v).await;
        let session = st.store.get_session(&chat.id).await?.ok_or_else(|| ApiError::not_found("session"))?;
        let ticket = load_ticket(&st, &id).await?;
        let attempt =
            ticket.attempts.iter().find(|a| a.id == attempt_id).cloned().unwrap_or_else(|| started.attempt.clone());
        return Ok((StatusCode::CREATED, Json(StartResult { ticket, attempt, session })));
    }

    let conversation = v.session_mode == SessionMode::Conversation;
    let permission_mode =
        conversation.then_some(if v.bypass_permissions { PermissionMode::Bypass } else { PermissionMode::Ask });
    let req = CreateSession {
        valid: ValidSession {
            project_id: project.id.clone(),
            worktree_id: Some(wt.id.clone()),
            kind: SessionKind::AgentTui,
            agent: Some(v.agent),
            bypass_permissions: v.bypass_permissions,
            size: v.size,
            // A conversation gets it as its first message, a terminal as the agent's last argument.
            initial_prompt: Some(ticket_prompt(&key, &ticket.summary.title, &ticket.description, &branch)),
            mode: v.session_mode,
            model: None,
            permission_mode,
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

/// The chat a Start from a chat adopts, with its worktree: a conversation session of this project with no ticket,
/// in a worktree off the base branch that no active attempt uses.
async fn startable_chat(
    st: &AppState,
    session_id: &str,
    project: &pdt_core::project::Project,
) -> Result<(pdt_core::session::Session, pdt_core::worktree::Worktree), ApiError> {
    let no = |m: &str| conflict("chat_not_startable", m.to_string());
    let s = st.store.get_session(session_id).await?.ok_or_else(|| ApiError::not_found("session"))?;
    if s.project_id != project.id {
        return Err(no("that chat belongs to another project"));
    }
    if s.mode != SessionMode::Conversation || s.agent.is_none() {
        return Err(no("only a conversation can be started from"));
    }
    if let Some(t) = &s.ticket {
        return Err(no(&format!("that chat already works on {}", t.key)));
    }
    let wid = s
        .worktree_id
        .clone()
        .ok_or_else(|| no("that chat runs in the main checkout; start the ticket in its own worktree instead"))?;
    let wt = st.store.get_worktree(&wid).await?.ok_or_else(|| ApiError::not_found("worktree"))?;
    if wt.branch == project.base_branch {
        return Err(no("that chat's worktree is on the base branch"));
    }
    if st.store.list_active_attempts().await?.iter().any(|a| a.attempt.worktree_id.as_deref() == Some(wid.as_str())) {
        return Err(no("another ticket's attempt already uses that chat's worktree"));
    }
    Ok((s, wt))
}

/// Start from a chat: the ticket's tab goes just before the chat's (which stays where it is); a chat with no tab gets
/// both, as a new start would.
async fn open_tab_before_chat(
    st: &AppState,
    project_id: &str,
    key: &str,
    ticket_id: &str,
    attempt_id: &str,
    session_id: &str,
    v: &ValidStart,
) {
    match st.store.tab_for(&TabItem::Session(session_id.to_string())).await {
        Ok(Some(tab)) => {
            let item = TabItem::Ticket(ticket_id.to_string());
            let new_id = uuid::Uuid::now_v7().to_string();
            if let Err(e) = st.store.open_tab(&new_id, &tab.workspace_id, &item, Some(tab.position), now_ms()).await {
                tracing::warn!(error = %e, "could not open the ticket's tab");
            }
        }
        _ => open_tabs(st, project_id, key, ticket_id, attempt_id, session_id, v).await,
    }
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
            ticket: None,
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
    if let Some(info) = st.store.active_attempt(&id).await? {
        crate::github::refresh_attempt(&st, &info).await;
        if is_merged(&st, &info, true).await
            && st.store.end_attempt(&info.attempt.id, AttemptState::Merged, Actor::System, now_ms()).await?.is_some()
        {
            crate::github::notify_merged(&st, &info).await;
        }
    }
    Ok(Json(load_ticket(&st, &id).await?))
}

/// Ends the ticket's active attempt because the ticket is being closed. Done says the work landed, so the attempt
/// is `merged`; Won't do ends it `merged` if its branch is (git or GitHub), else `abandoned`.
pub(crate) async fn end_for_close(st: &AppState, ticket_id: &str, stage: TicketStage) -> Result<(), ApiError> {
    if let Some(info) = st.store.active_attempt(ticket_id).await? {
        let merged = stage == TicketStage::Done || is_merged(st, &info, false).await;
        let state = if merged { AttemptState::Merged } else { AttemptState::Abandoned };
        st.store.end_attempt(&info.attempt.id, state, Actor::User, now_ms()).await?;
    }
    Ok(())
}

/// Whether the attempt's branch is merged into its base, and remembers the branch head it saw. A deleted branch is
/// judged by its last known head. Merged means GitHub merged the attempt's PR at that head (the merge queue squashes
/// onto a newer base, which git's patch check can miss), or git finds the head or its whole patch in the base (locally
/// or on `origin`, fetched first with `fetch`; for projects not on GitHub). Git runs on a blocking thread; any
/// failure counts as not merged.
async fn is_merged(st: &AppState, info: &AttemptInfo, fetch: bool) -> bool {
    let repo = PathBuf::from(&info.repo_path);
    let (branch, base) = (info.attempt.branch.clone(), info.attempt.base_branch.clone());
    let (base_commit, last_head) = (info.base_commit.clone(), info.attempt.last_head.clone());
    let attempt = info.attempt.id.clone();
    let pr = crate::github::watched_pr(st, &info.attempt.id);
    let found = tokio::task::spawn_blocking(move || {
        if fetch
            && pdt_git::remote_url(&repo, "origin").is_some()
            && let Err(e) = pdt_git::fetch(&repo, "origin", &base, FETCH_TIMEOUT)
        {
            tracing::info!(attempt = %attempt, error = %e, "could not fetch the base branch; checking the local refs");
        }
        let head = pdt_git::rev_parse(&repo, &format!("refs/heads/{branch}")).or(last_head)?;
        if pr.is_some_and(|p| p.merged_at(&head)) {
            return Some((head, true));
        }
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
                Ok(Some(_)) => {
                    tracing::info!(ticket = %info.ticket_key, branch = %info.attempt.branch,
                        "the attempt's branch was merged");
                    crate::github::notify_merged(st, &info).await;
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(attempt = %info.attempt.id, error = %e, "could not end the merged attempt"),
            }
        }
    }
}
