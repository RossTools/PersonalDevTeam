//! Ticket Start, attempts and merge detection: Start makes the branch, worktree, attempt and an
//! agent session whose first prompt is the ticket (unless `noSession`), and opens the ticket and the session as tabs; an attempt ends when
//! its branch is merged into the base (checked every 60 s and on demand), when it is abandoned, or when the ticket is
//! closed.

use crate::error::{ApiError, OrNotFound, codes};
use crate::log_err::LogErr;
use crate::routes::{AppState, now_ms};
use crate::sessions::{CreateSession, create_session};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::conversation::{PermissionMode, SessionMode};
use pdt_core::parts::TicketParent;
use pdt_core::session::{SessionKind, ValidSession};
use pdt_core::start::{StartMode, StartResult, StartTicket, ValidStart, start_prompt, started_session_title};
use pdt_core::ticket::{Actor, AttemptState, EventKind, LinkDirection, LinkKind, Ticket, TicketStage};
use pdt_core::workflow::events::MergeVia;
use pdt_core::workspace::{TabItem, Workspace, valid_workspace_name};
use pdt_store::{AttemptInfo, StartedAttempt, TicketError};
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;

/// `POST /api/tickets/{id}/start`.
pub(crate) async fn start(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    payload: Result<Json<StartTicket>, JsonRejection>,
) -> Result<(StatusCode, Json<StartResult>), ApiError> {
    let Json(input) = payload?;
    let v = input.validate()?;
    let by = if crate::untrusted::from_ui(&st, &headers)? { Starter::User } else { Starter::Api };
    let r = start_ticket(&st, id, v, by).await?;
    Ok((StatusCode::CREATED, Json(r)))
}

/// Who asked for a Start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Starter {
    /// The user, from PDT's web client.
    User,
    /// A request to the HTTP API without the UI token: an agent's curl, as far as PDT can tell (PDT-204).
    Api,
    /// The scheduler's auto-start.
    Auto,
}

/// The Start command (the route, and the scheduler with [`Starter::Auto`]): checks, then the branch, worktree, attempt, session
/// and tabs.
pub(crate) async fn start_ticket(
    st: &AppState,
    id: String,
    v: ValidStart,
    by: Starter,
) -> Result<StartResult, ApiError> {
    let st = st.clone();
    // The fetch goes before the lock (PDT-440): it may take seconds, and the lock is only for the checks and rows.
    let (project_id, fetched) = fetch_before_start(&st, &id, &v).await?;
    let guard = st.ticket_start_locks.lock(&project_id).await;
    let mut ticket = crate::overlap::load_ticket(&st, &id).await?;
    // A parent is never started itself (PDT-38): `next` starts its next part instead.
    let mut id = id;
    if !ticket.part_list.is_empty() {
        let key = ticket.summary.key.clone();
        if !v.next {
            return Err(ApiError::conflict(
                codes::TICKET_HAS_PARTS,
                format!("{key} has parts; start one of them (or its next part) instead"),
            ));
        }
        match crate::parts::next(&st, &id).await? {
            Ok(part) => {
                id = part;
                ticket = crate::overlap::load_ticket(&st, &id).await?;
            }
            Err(why) => {
                return Err(ApiError::conflict(codes::NO_NEXT_PART, format!("{key} has no part to start: {why}")));
            }
        }
    }
    let key = ticket.summary.key.clone();
    let parent = ticket.summary.parent.clone();
    // Only the user starts an external ticket they haven't let in (PDT-201); starting it lets it in.
    if by == Starter::Auto && crate::untrusted::held_back(&ticket.summary) {
        return Err(ApiError::forbidden(
            codes::UNTRUSTED_TICKET,
            format!("{key} came from outside PDT: only you can start it, or move it to To do"),
        ));
    }
    if by == Starter::Api && crate::untrusted::held_back(&ticket.summary) {
        return Err(crate::untrusted::refused(&key));
    }
    // A ticket proposal waits for the user's decision in the Inbox (PDT-366): neither auto-start nor an agent's curl
    // starts it.
    if by != Starter::User && ticket.summary.proposed {
        return Err(ApiError::forbidden(
            codes::TICKET_PROPOSAL,
            format!("{key} is a ticket proposal waiting for the user in PDT's Inbox: only they can start it"),
        ));
    }
    if ticket.summary.stage.is_closed() {
        return Err(ApiError::conflict(
            codes::TICKET_CLOSED,
            format!("{key} is closed; move it back to Ready to start it"),
        ));
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
    // Overlap with the project's active tickets: what this one would share once started. A pickup's start checked its
    // own overlap rule, the one choose applied (PDT-381): only a ApiError::conflict refuses it.
    let overlaps: Vec<_> =
        pdt_core::start::refusing_overlaps(&ticket.overlaps, v.pickup_checked).into_iter().cloned().collect();
    // The project's cap on running attempts.
    let policy = st.store.get_project_settings(&ticket.summary.project_id).await?.policy;
    let active = st.store.count_active_attempts(&ticket.summary.project_id).await?;
    // A workflow's start that checked its own cap (PDT-327) may go over the project's: the step's cap is the one.
    let over_cap = !v.pickup_checked && policy.over_cap(active);
    if (!blockers.is_empty() || !overlaps.is_empty() || over_cap) && v.mode == StartMode::Check {
        let mut why = vec![];
        if !blockers.is_empty() {
            why.push(format!("is blocked by {}, which {} not done yet", blockers.join(", "), plural(blockers.len())));
        }
        if !overlaps.is_empty() {
            why.push(crate::overlap::describe(&overlaps));
        }
        if over_cap {
            why.push(format!(
                "would go over the project's cap of {} running tickets ({active} in flight)",
                policy.max_running_attempts.unwrap_or_default()
            ));
        }
        return Err(ApiError::conflict(codes::START_BLOCKED, format!("{key} {}", why.join("; it "))));
    }
    if !v.new_workspace {
        crate::workspaces::check_placement(&st, v.tab.as_ref()).await?;
    }
    let project = st.project(&ticket.summary.project_id).await?;
    let base = v.base.clone().unwrap_or_else(|| project.base_branch.clone());
    if v.base.is_some() && !project.is_personal() {
        known_base(&project.repo_path, &base).await?;
    }
    // A Personal project has no repository: the ticket runs in its folder, with no branch or worktree.
    let personal = match project.is_personal() {
        true if v.from_session.is_some() => {
            return Err(ApiError::conflict(
                codes::PERSONAL_PROJECT,
                "a Personal project's tickets start on their own, not from a chat",
            ));
        }
        true => Some(personal_folder(&project, &key)?),
        false => None,
    };
    // Start from a chat: its worktree and branch, and the chat itself instead of a new session.
    let chat = match &v.from_session {
        Some(sid) => Some(startable_chat(&st, sid, &project).await?),
        None => None,
    };
    // Whether PDT named the branch from the project's template (PDT-594): Won't do may delete such a branch from the
    // remote, whatever its name.
    let named = chat.is_none() && personal.is_none() && v.branch.as_deref().is_none_or(|b| b == ticket.default_branch);
    let (wt, branch) = match (&chat, &personal) {
        (_, Some(_)) => (None, String::new()),
        (Some((_, wt)), None) => (Some(wt.clone()), wt.branch.clone()),
        (None, None) => {
            let branch = match &v.branch {
                Some(b) => b.clone(),
                None => crate::branch_names::start_branch(&st, &project, &ticket, &ticket.default_branch).await?,
            };
            let existing = st.store.list_worktrees(&project.id).await?.into_iter().find(|w| w.branch == branch);
            let wt = match existing {
                Some(w) => w,
                None => {
                    let from = match fetched {
                        Some((b, from)) if b == base => from,
                        _ => newest_base(&project.repo_path, &base).await,
                    };
                    crate::worktrees::make_worktree(&st, &project, &branch, &from, true).await?
                }
            };
            (Some(wt), branch)
        }
    };
    let base = if personal.is_some() { String::new() } else { base };
    let mut extra = if blockers.is_empty() && overlaps.is_empty() && !over_cap {
        json!({})
    } else {
        let overlaps: Vec<_> =
            overlaps.iter().map(|o| json!({"key": o.key, "level": o.level, "areas": o.areas})).collect();
        json!({"forced": true, "blockers": blockers, "overlaps": overlaps,
               "level": crate::overlap::worst(&ticket.overlaps), "overCap": over_cap})
    };
    if by == Starter::Auto {
        extra["auto"] = json!(true);
    }
    let attempt_id = uuid::Uuid::now_v7().to_string();
    let started = st
        .store
        .start_attempt(
            &attempt_id,
            &id,
            wt.as_ref().map(|w| w.id.as_str()),
            &branch,
            &base,
            wt.as_ref().map(|w| w.base_commit.as_str()).unwrap_or_default(),
            extra,
            now_ms(),
        )
        .await?;
    st.store.set_attempt_branch_named(&attempt_id, named).await.log_warn("record who named the attempt's branch");
    drop(guard);

    if let Some((chat, _)) = chat {
        let linked = st.store.set_session_attempt(&chat.id, &attempt_id).await;
        if !matches!(linked, Ok(true)) {
            if let Err(u) = st.store.undo_start(&started).await {
                tracing::error!(ticket = %id, error = %u, "could not undo a failed start");
            }
            return Err(match linked {
                Err(e) => e.into(),
                _ => ApiError::conflict(codes::CHAT_NOT_STARTABLE, "that chat already works on a ticket"),
            });
        }
        announce(&st, &started).await;
        let data = json!({"sessionId": chat.id, "agent": chat.agent, "attemptId": attempt_id, "fromChat": true});
        if let Err(e) =
            st.store.add_ticket_event(&id, EventKind::Session, Actor::User, None, Some(data), now_ms()).await
        {
            tracing::warn!(ticket = %id, error = %e, "could not record the chat on the timeline");
        }
        outside_ticket(&st, &ticket.summary, &chat.id).await;
        open_tab_before_chat(&st, &project.id, &key, &id, &attempt_id, &chat.id, &v).await;
        if let Some(p) = &parent {
            part_started(&st, p).await;
        }
        let session = st.session(&chat.id).await?;
        let ticket = crate::overlap::load_ticket(&st, &id).await?;
        let attempt =
            ticket.attempts.iter().find(|a| a.id == attempt_id).cloned().unwrap_or_else(|| started.attempt.clone());
        return Ok(StartResult { ticket, attempt, session: Some(session) });
    }

    if v.no_session {
        announce(&st, &started).await;
        open_tabs(&st, &project.id, &key, &id, &attempt_id, None, &v).await;
        if let Some(p) = &parent {
            part_started(&st, p).await;
        }
        let ticket = crate::overlap::load_ticket(&st, &id).await?;
        let attempt =
            ticket.attempts.iter().find(|a| a.id == attempt_id).cloned().unwrap_or_else(|| started.attempt.clone());
        return Ok(StartResult { ticket, attempt, session: None });
    }

    let part_context = match crate::parts::context(&st, &ticket).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(ticket = %id, error = ?e, "could not read the part's parent for its prompt");
            None
        }
    };
    let comments = match st.store.comment_bodies(&id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(ticket = %id, error = ?e, "could not read the ticket's comments for its prompt");
            Vec::new()
        }
    };
    let conversation = v.session_mode == SessionMode::Conversation;
    let permission_mode = conversation.then_some(v.permission_mode.unwrap_or(if v.bypass_permissions {
        PermissionMode::Bypass
    } else {
        v.agent.default_permission_mode()
    }));
    let req = CreateSession {
        valid: ValidSession {
            project_id: project.id.clone(),
            worktree_id: wt.as_ref().map(|w| w.id.clone()),
            kind: SessionKind::AgentTui,
            agent: Some(v.agent),
            bypass_permissions: v.bypass_permissions,
            size: v.size,
            // A conversation gets it as its first message, a terminal as the agent's last argument.
            initial_prompt: Some(start_prompt(
                &key,
                &ticket.summary.title,
                &ticket.description,
                &branch,
                part_context.as_ref(),
                &comments,
            )),
            mode: v.session_mode,
            model: None,
            permission_mode,
        },
        title: Some(started_session_title(&ticket.summary.title)),
        attempt_id: Some(attempt_id.clone()),
        ticket: Some(pdt_core::ticket::SessionTicket { id: ticket.summary.id.clone(), key: key.clone() }),
        tab: None,
        persona: None,
        cwd: personal.clone(),
        id: None,
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
    announce(&st, &started).await;
    let data = json!({"sessionId": session.id, "agent": v.agent, "attemptId": attempt_id});
    if let Err(e) = st.store.add_ticket_event(&id, EventKind::Session, Actor::User, None, Some(data), now_ms()).await {
        tracing::warn!(ticket = %id, error = %e, "could not record the started session on the timeline");
    }
    outside_ticket(&st, &ticket.summary, &session.id).await;
    open_tabs(&st, &project.id, &key, &id, &attempt_id, Some(&session.id), &v).await;
    if let Some(p) = &parent {
        part_started(&st, p).await;
    }
    let ticket = crate::overlap::load_ticket(&st, &id).await?;
    let attempt =
        ticket.attempts.iter().find(|a| a.id == attempt_id).cloned().unwrap_or_else(|| started.attempt.clone());
    Ok(StartResult { ticket, attempt, session: Some(session) })
}

/// Logs the start for workflows (`attempt.started`), now that it can no longer be undone. Logged, never fails Start.
async fn announce(st: &AppState, started: &StartedAttempt) {
    if let Err(e) = st.store.announce_start(started, now_ms()).await {
        tracing::error!(ticket = %started.ticket_id, error = %e, "could not log the start for workflows");
    }
}

/// A Personal project's folder for a ticket's work, `<project folder>/<ticket key>` (made if missing; kept across
/// attempts: it is the work's output).
pub(crate) fn personal_folder(project: &pdt_core::project::Project, key: &str) -> std::io::Result<String> {
    let dir = PathBuf::from(&project.repo_path).join(key);
    std::fs::create_dir_all(&dir)?;
    Ok(dir.to_string_lossy().into_owned())
}

/// The chat a Start from a chat adopts, with its worktree: a conversation session of this project with no ticket,
/// in a worktree off the base branch that no active attempt uses.
async fn startable_chat(
    st: &AppState,
    session_id: &str,
    project: &pdt_core::project::Project,
) -> Result<(pdt_core::session::Session, pdt_core::worktree::Worktree), ApiError> {
    let no = |m: &str| ApiError::conflict(codes::CHAT_NOT_STARTABLE, m.to_string());
    let s = st.session(session_id).await?;
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
    let wt = st.store.get_worktree(&wid).await?.or_404("worktree")?;
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
        _ => open_tabs(st, project_id, key, ticket_id, attempt_id, Some(session_id), v).await,
    }
}

/// A part started: its parent moves to In progress (if it had not yet). Logged, never fails Start.
async fn part_started(st: &AppState, parent: &TicketParent) {
    if let Err(e) = st.store.part_started(&parent.id, now_ms()).await {
        tracing::warn!(ticket = %parent.key, error = %e, "could not move the parent to In progress");
    }
}

/// A requested base must name a commit of the repository, here or as `origin/<base>` (the attempt keeps it and
/// later hands it to `git merge` and `git merge-base`).
async fn known_base(repo_path: &str, base: &str) -> Result<(), ApiError> {
    let (repo, b) = (PathBuf::from(repo_path), base.to_string());
    let known = tokio::task::spawn_blocking(move || {
        pdt_git::rev_parse(&repo, &b).is_some()
            || pdt_git::rev_parse(&repo, &format!("refs/remotes/origin/{b}")).is_some()
    })
    .await
    .map_err(ApiError::internal)?;
    match known {
        true => Ok(()),
        false => Err(pdt_git::GitError::UnknownRef(base.to_string()).into()),
    }
}

/// The ticket's project, and the base branch with where to branch from it when Start will likely make a worktree
/// (fetched first: [`newest_base`]). A guess made before the lock: Start fetches again under it if the guess missed.
async fn fetch_before_start(
    st: &AppState,
    id: &str,
    v: &ValidStart,
) -> Result<(String, Option<(String, String)>), ApiError> {
    let mut ticket = st.store.get_ticket(id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
    crate::branch_names::apply(st, &mut ticket).await;
    // A part is always in its parent's project.
    let project_id = ticket.summary.project_id.clone();
    let Some(project) = st.store.get_project(&project_id).await? else { return Ok((project_id, None)) };
    if project.is_personal() || v.from_session.is_some() || ticket.summary.stage.is_closed() {
        return Ok((project_id, None));
    }
    // A parent starts its next part, on that part's branch: fetch for it anyway.
    if ticket.part_list.is_empty() {
        let branch = v.branch.clone().unwrap_or_else(|| ticket.default_branch.clone());
        if st.store.list_worktrees(&project.id).await?.iter().any(|w| w.branch == branch) {
            return Ok((project_id, None));
        }
    }
    let base = v.base.clone().unwrap_or_else(|| project.base_branch.clone());
    let from = newest_base(&project.repo_path, &base).await;
    Ok((project_id, Some((base, from))))
}

/// Where Start's new branch begins: the project's `base`, or `origin/<base>` (fetched first) when that is ahead, so
/// the branch and its `base_commit` start from the newest base (PDT-45). A failed fetch keeps the refs there are.
async fn newest_base(repo_path: &str, base: &str) -> String {
    let (repo, base) = (PathBuf::from(repo_path), base.to_string());
    let fallback = base.clone();
    tokio::task::spawn_blocking(move || {
        if pdt_git::remote_url(&repo, "origin").is_none() {
            return base;
        }
        if let Err(e) = pdt_git::fetch(&repo, "origin", &base, pdt_git::QUICK_FETCH_TIMEOUT) {
            tracing::info!(error = %e, "could not fetch the base branch before Start; using the local refs");
        }
        pdt_git::start_point(&repo, &base)
    })
    .await
    .unwrap_or(fallback)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "is" } else { "are" }
}

/// The ticket's tab, then the session's (if any) right after it: in a new workspace named after the ticket, where asked, or
/// at the end of the project's workspace (the attempt remembers a new workspace). A failure is logged: the session runs and can be opened from the sidebar.
async fn open_tabs(
    st: &AppState,
    project_id: &str,
    key: &str,
    ticket_id: &str,
    attempt_id: &str,
    session_id: Option<&str>,
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
    let Some(session_id) = session_id else { return };
    let session_item = TabItem::Session(session_id.to_string());
    if let Err(e) = st.store.open_tab(&new_id(), &workspace_id, &session_item, after, now_ms()).await {
        tracing::warn!(error = %e, "could not open the started session's tab");
    }
}

/// `POST /api/attempts/{id}/abandon`: stops the ticket's work and ends the attempt (PDT-159: its runs are cancelled
/// and its sessions stop), then winds it down as Won't do does (PDT-339): its pull request closes and the worktree,
/// branch and workspace go as the project says, when nothing would be lost. Abandoning an ended attempt changes nothing.
pub(crate) async fn abandon(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Ticket>, ApiError> {
    let info = st.store.get_attempt(&id).await?.or_404("attempt")?;
    if info.attempt.state == AttemptState::Active {
        let (st, ticket, attempt) = (st.clone(), info.ticket_id.clone(), info.attempt.id.clone());
        let task = tokio::spawn(async move {
            let since = now_ms();
            let r = crate::stop::stop_work(&st, &ticket, since, Actor::User).await?;
            if r.abandoned.is_some() {
                let how = crate::stop::WindDown::Abandon {
                    sessions_stopped: r.sessions_stopped,
                    runs_cancelled: r.runs_cancelled,
                };
                crate::stop::wind_down(&st, &attempt, since, how, None).await;
            }
            Ok::<_, ApiError>(())
        });
        task.await.map_err(ApiError::internal)??;
    }
    Ok(Json(crate::overlap::load_ticket(&st, &info.ticket_id).await?))
}

/// `POST /api/attempts/{id}/clean-up`: the Clean up action on an ended attempt whose worktree was kept (PDT-339):
/// removes the worktree (uncommitted changes and all), the branch (and PDT's copy on the remote) and the workspace.
pub(crate) async fn clean_up(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Ticket>, ApiError> {
    let info = st.store.get_attempt(&id).await?.or_404("attempt")?;
    if info.attempt.state == AttemptState::Active {
        return Err(ApiError::conflict(codes::ATTEMPT_ACTIVE, "the attempt is still active; abandon it first"));
    }
    let ticket = info.ticket_id.clone();
    let task = tokio::spawn({
        let st = st.clone();
        async move {
            let notes = crate::stop::clean_up_closed(&st, &info, true).await;
            if !notes.is_empty()
                && let Err(e) =
                    st.store.add_comment_by(&info.ticket_id, &notes.join(" "), Actor::User, None, now_ms()).await
            {
                tracing::warn!(ticket = %info.ticket_key, error = %e, "could not note the cleanup on the timeline");
            }
        }
    });
    task.await.map_err(ApiError::internal)?;
    Ok(Json(crate::overlap::load_ticket(&st, &ticket).await?))
}

/// Ends the ticket's active attempt because the ticket closed ([`end_for_close`]), then winds its work down
/// (PDT-339): Won't do stops its sessions and runs, cancels its workflow runs, closes its pull request and cleans up
/// as the project says; Done only stops what still runs. `keep_run`: the workflow run closing it, which goes on.
pub(crate) async fn close_attempt(
    st: &AppState,
    ticket_id: &str,
    stage: TicketStage,
    since: i64,
    by: impl Into<pdt_core::ticket::By>,
    keep_run: Option<&str>,
) -> Result<(), ApiError> {
    let active = st.store.active_attempt(ticket_id).await?;
    end_for_close(st, ticket_id, stage, by).await?;
    let Some(info) = active else { return Ok(()) };
    let how = if stage == TicketStage::WontDo { crate::stop::WindDown::Close } else { crate::stop::WindDown::Stop };
    let (st, attempt, keep) = (st.clone(), info.attempt.id, keep_run.map(String::from));
    // In a task of its own, so it completes when the caller goes away (an agent closing its own ticket is stopped).
    let task = tokio::spawn(async move { crate::stop::wind_down(&st, &attempt, since, how, keep.as_deref()).await });
    task.await.map_err(ApiError::internal)
}

/// `POST /api/tickets/{id}/check-merge`: merge detection for the ticket's active attempt, now.
pub(crate) async fn check_merge(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Ticket>, ApiError> {
    st.store.get_ticket_summary(&id).await?.or_404("ticket")?;
    if let Some(info) = st.store.active_attempt(&id).await? {
        crate::github::refresh_attempt(&st, &info).await;
        if is_merged(&st, &info, true, true).await {
            let pr = crate::github::watched_pr(&st, &info.attempt.id).map(|p| p.number);
            let ended = st
                .store
                .end_attempt(
                    &info.attempt.id,
                    AttemptState::Merged,
                    Actor::System,
                    Some(MergeVia::Detected),
                    pr,
                    now_ms(),
                )
                .await?;
            if ended.is_some() {
                // Nothing works on a merged attempt (PDT-339).
                let (st, attempt) = (st.clone(), info.attempt.id.clone());
                let task = tokio::spawn(async move {
                    crate::stop::wind_down(&st, &attempt, now_ms(), crate::stop::WindDown::Merged, None).await
                });
                task.await.map_err(ApiError::internal)?;
            }
        }
    }
    Ok(Json(crate::overlap::load_ticket(&st, &id).await?))
}

/// Merge detection for one active attempt now, without fetching (the local forge merged it here, PDT-178): ends it
/// as `merged` when its branch is. Whether it ended.
pub(crate) async fn end_if_merged(st: &AppState, info: &AttemptInfo) -> bool {
    if !is_merged(st, info, false, true).await {
        return false;
    }
    let pr = crate::github::watched_pr(st, &info.attempt.id).map(|p| p.number);
    let via = Some(MergeVia::Detected);
    match st.store.end_attempt(&info.attempt.id, AttemptState::Merged, Actor::System, via, pr, now_ms()).await {
        Ok(ended) => ended.is_some(),
        Err(e) => {
            tracing::warn!(attempt = %info.attempt.id, error = %e, "could not end the merged attempt");
            false
        }
    }
}

/// Ends the ticket's active attempt because the ticket was closed (callers close it first, so a refused close ends
/// nothing): `merged` if its branch is (git or the forge), else `abandoned`. Done says the work landed, so it ends
/// `merged` even when git cannot see the merge, but not while its PR is unmerged (PDT-257: open or closed, read again
/// first in case it merged since the last look).
pub(crate) async fn end_for_close(
    st: &AppState,
    ticket_id: &str,
    stage: TicketStage,
    by: impl Into<pdt_core::ticket::By>,
) -> Result<(), ApiError> {
    if let Some(info) = st.store.active_attempt(ticket_id).await? {
        let unmerged_pr = || {
            crate::github::watched_pr(st, &info.attempt.id).is_some_and(|p| p.state != pdt_core::pr::PrState::Merged)
        };
        if stage == TicketStage::Done && unmerged_pr() {
            crate::github::refresh_attempt(st, &info).await;
        }
        let merged = is_merged(st, &info, false, false).await || (stage == TicketStage::Done && !unmerged_pr());
        let state = if merged { AttemptState::Merged } else { AttemptState::Abandoned };
        let via = merged.then_some(MergeVia::Closed);
        let pr = crate::github::watched_pr(st, &info.attempt.id).map(|p| p.number);
        st.store.end_attempt(&info.attempt.id, state, by, via, pr, now_ms()).await?;
    }
    Ok(())
}

/// Whether the attempt's branch is merged into its base, and remembers the branch head it saw. A deleted branch is
/// judged by its last known head. Merged means GitHub merged the attempt's PR at that head (the merge queue squashes
/// onto a newer base, which git's patch check can miss), or git finds the head or its whole patch in the base (locally
/// or on `origin`, fetched first with `fetch`; for projects not on GitHub) after the branch has been seen with commits
/// of its own. Git runs on a blocking thread; any failure counts as not merged. With `news`, a PR that merged at an
/// older head than the branch's logs `attempt.late_commits` (PDT-151): the commits pushed after the merge keep the
/// attempt open, and the user decides what becomes of them.
async fn is_merged(st: &AppState, info: &AttemptInfo, fetch: bool, news: bool) -> bool {
    let repo = PathBuf::from(&info.repo_path);
    let (branch, base) = (info.attempt.branch.clone(), info.attempt.base_branch.clone());
    let (base_commit, last_head) = (info.base_commit.clone(), info.attempt.last_head.clone());
    let seen_own = info.own_commits;
    let attempt = info.attempt.id.clone();
    let pr = crate::github::watched_pr(st, &info.attempt.id);
    let found = tokio::task::spawn_blocking(move || {
        if fetch
            && pdt_git::remote_url(&repo, "origin").is_some()
            && let Err(e) = pdt_git::fetch(&repo, "origin", &base, pdt_git::REMOTE_TIMEOUT)
        {
            tracing::info!(attempt = %attempt, error = %e, "could not fetch the base branch; checking the local refs");
        }
        let head = pdt_git::rev_parse(&repo, &format!("refs/heads/{branch}")).or(last_head)?;
        if pr.as_ref().is_some_and(|p| p.merged_at(&head)) {
            return Some((head, seen_own, true, vec![]));
        }
        // Merged on GitHub at an older head: the commits since aren't in the base.
        let late = match pr
            .as_ref()
            .filter(|p| news && p.state == pdt_core::pr::PrState::Merged && pdt_git::is_commit_id(&p.head_sha))
        {
            Some(p) => pdt_git::commits_between(&repo, &p.head_sha, &head)
                .unwrap_or_else(|| vec![(head.clone(), String::new())]),
            None => vec![],
        };
        let origin = format!("origin/{base}");
        let bases = [base.as_str(), origin.as_str()];
        // A head already in the base when first seen only caught up with it (PDT-45): not merged until the branch
        // has held commits of its own.
        let own = seen_own || pdt_git::has_own_commits(&repo, &head, &base_commit, &bases);
        let merged = own && pdt_git::is_merged(&repo, &head, &base_commit, &bases);
        Some((head, own, merged, late))
    })
    .await
    .ok()
    .flatten();
    let Some((head, own, merged, late)) = found else { return false };
    if !merged
        && !late.is_empty()
        && let Some(pr) = crate::github::watched_pr(st, &info.attempt.id)
    {
        crate::github::log_late_commits(st, info, &pr, &head, late).await;
    }
    if (info.attempt.last_head.as_deref() != Some(head.as_str()) || own != info.own_commits)
        && let Err(e) = st.store.set_attempt_head(&info.attempt.id, &head, own).await
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
        // A Personal project's ticket has no branch to merge.
        if info.attempt.branch.is_empty() {
            continue;
        }
        let fetch = fetched.insert((info.project_id.clone(), info.attempt.base_branch.clone()));
        if is_merged(st, &info, fetch, true).await {
            let pr = crate::github::watched_pr(st, &info.attempt.id).map(|p| p.number);
            let via = Some(MergeVia::Detected);
            match st.store.end_attempt(&info.attempt.id, AttemptState::Merged, Actor::System, via, pr, now_ms()).await {
                Ok(Some(_)) => {
                    tracing::info!(ticket = %info.ticket_key, branch = %info.attempt.branch,
                        "the attempt's branch was merged");
                    // Nothing works on a merged attempt (PDT-339).
                    crate::stop::wind_down(st, &info.attempt.id, now_ms(), crate::stop::WindDown::Merged, None).await;
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(attempt = %info.attempt.id, error = %e, "could not end the merged attempt"),
            }
        }
    }
}

/// An external ticket's description is outside input (PDT-202): its new session reading it is recorded.
async fn outside_ticket(st: &AppState, t: &pdt_core::ticket::TicketSummary, session_id: &str) {
    if let Some((source, url)) = crate::untrusted::ticket_source(t) {
        let e = pdt_core::workflow::events::OutsideInput {
            project_id: Some(t.project_id.clone()),
            ticket_id: Some(t.id.clone()),
            session_id: session_id.to_string(),
            source,
            url,
            via: "ticket".into(),
            run_id: None,
            workflow_name: None,
        };
        crate::untrusted::record_input(st, e).await;
    }
}

/// `POST /api/tickets/{id}/retry-auto-start`: auto-start forgets that it failed to start the ticket (PDT-381), and
/// its next run tries it again.
pub(crate) async fn retry_auto_start(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Ticket>, ApiError> {
    crate::overlap::load_ticket(&st, &id).await?;
    crate::scheduler::forget_failed_start(&st, &id);
    Ok(Json(crate::overlap::load_ticket(&st, &id).await?))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::post;
    axum::Router::new()
        .route("/tickets/{id}/start", post(start))
        .route("/tickets/{id}/check-merge", post(check_merge))
        .route("/tickets/{id}/retry-auto-start", post(retry_auto_start))
        .route("/attempts/{id}/abandon", post(abandon))
        .route("/attempts/{id}/clean-up", post(clean_up))
}
