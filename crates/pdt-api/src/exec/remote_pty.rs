//! The remote instance's end of remote terminals (PDT-633): it runs an owner's terminal session as a PTY of its own,
//! under the owner's session id, only in the remote project's checkout or one of its worktrees. The PTY keeps its
//! scrollback while nobody watches; the owner attaches with a one-time ticket (`/link/exec/pty/attach`, signed) and
//! the WebSocket it opens (`/link/exec/pty/ws/{ticket}`) speaks the local terminal protocol (`pty_ws`). Its exit goes
//! to the owner with the events, and its agent's hook reports are passed on (`relay_activity`).

use super::remote::{checked, ensure_checkout, mcp_server, write_instructions};
use super::*;
use crate::error::{ApiError, codes};
use crate::extract::ApiQuery;
use crate::link::{self, PeerError};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, Method};
use axum::response::Response;
use base64::Engine;
use pdt_core::link::OwnerLink;
use pdt_core::session::{Resume, SessionKind, TerminalSize};
use pdt_core::worktree::worktree_path;
use std::path::PathBuf;
use std::time::Duration;

/// How long a terminal's WebSocket ticket is good for.
const TICKET_TTL_MS: i64 = 60_000;
/// How long an ended terminal is kept before it is forgotten.
const FORGET_AFTER: Duration = Duration::from_secs(30);
/// How long passing a hook report on may take.
const RELAY_TIMEOUT: Duration = Duration::from_secs(10);

/// An owner's terminal running here.
pub(super) struct PtyRun {
    pub(super) proc: String,
    pub(super) link_id: String,
    /// A Codex terminal launched without a conversation: when, its folder and the folder's key, so its conversation
    /// can be found by start time once it ends.
    codex_fresh: Option<(i64, String, String)>,
}

pub(super) struct Ticket {
    session_id: String,
    link_id: String,
    expires_at: i64,
}

/// The sessions whose PTY runs here for the owner on `link_id`.
pub(super) fn alive(st: &AppState, link_id: &str) -> Vec<String> {
    let ptys = st.exec.remote.ptys.lock().unwrap_or_else(|e| e.into_inner());
    ptys.iter().filter(|(id, r)| r.link_id == link_id && st.ptys.is_running(id)).map(|(id, _)| id.clone()).collect()
}

/// Whether `session` is a PTY this instance runs for the owner on `link_id` (running or ended, with its scrollback).
fn ours(st: &AppState, link_id: &str, session: &str) -> bool {
    st.exec.remote.ptys.lock().unwrap_or_else(|e| e.into_inner()).get(session).is_some_and(|r| r.link_id == link_id)
}

/// Stops an owner's terminal here (its exit goes to the owner with the events).
pub(super) async fn kill(st: &AppState, link_id: &str, session: &str) {
    if ours(st, link_id, session)
        && st.ptys.is_running(session)
        && let Err(e) = st.kill_pty(session).await
    {
        tracing::warn!(session, error = %e, "could not stop an owner's terminal");
    }
}

/// Where an owner's terminal may run: `branch`'s worktree in the remote project's checkout, or (None) the checkout
/// itself, cloned first when it isn't there. Nowhere else: the branch must be a plain branch name, and the folder
/// must be there.
async fn folder(st: &AppState, link: &OwnerLink, branch: Option<&str>, base: &str) -> Result<PathBuf, ApiError> {
    let Some(branch) = branch else { return ensure_checkout(st, link, base).await };
    pdt_core::worktree::NewWorktree { branch: branch.to_string(), base: None }
        .validate()
        .map_err(|e| ApiError::invalid(e.to_string()))?;
    let path = worktree_path(&super::checkout_dir(st, link), branch);
    if !path.is_dir() {
        return Err(ApiError::conflict(
            codes::REMOTE_PROJECT,
            format!("{} has no worktree for {branch}", link::instance_name(st).await?),
        ));
    }
    Ok(path)
}

/// `/link/exec/pty/open`: starts the owner's terminal session here (or, running already, says which process it is).
pub(crate) async fn open_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Opened>, ApiError> {
    let (link, req): (OwnerLink, PtyOpenReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let mut s = req.session;
    if s.mode == pdt_core::conversation::SessionMode::Conversation {
        return Err(ApiError::invalid("a conversation isn't a terminal"));
    }
    let size = TerminalSize { cols: req.cols, rows: req.rows }.validate()?;
    let cwd = folder(&st, &link, req.branch.as_deref(), &req.base).await?;
    let cwd_text = cwd.to_string_lossy().into_owned();
    let _one = st.exec.remote.opening.lock().await;
    let running = st.exec.remote.ptys.lock().unwrap_or_else(|e| e.into_inner()).get(&s.id).map(|r| {
        let same = r.link_id == link.link_id;
        (same, r.proc.clone())
    });
    match running {
        Some((true, proc)) if st.ptys.is_running(&s.id) => return Ok(Json(Opened { proc, cwd: cwd_text })),
        Some((false, _)) => return Err(ApiError::forbidden(codes::LINK_REFUSED, "that session isn't this owner's")),
        _ => {}
    }
    if st.ptys.is_running(&s.id) {
        return Err(ApiError::conflict(codes::ALREADY_RUNNING, "a terminal by that id runs here"));
    }
    if link.paused {
        let me = link::instance_name(&st).await?;
        return Err(ApiError::conflict(codes::REMOTE_PROJECT, format!("this project is paused on {me}")));
    }
    // A previous run's terminal (kept for its scrollback) makes way.
    st.remove_pty(&s.id).await;
    s.cwd = cwd_text.clone();
    let dir = st.data_dir.join("sessions").join(&s.id);
    let instructions = match &req.instructions {
        Some(i) => {
            let file = write_instructions(&dir, &i.file_name, &i.text)
                .map_err(|e| ApiError::io("writing the session's instructions", e))?;
            Some(pdt_agents::adapter::SessionInstructions { text: i.text.clone(), file })
        }
        None => None,
    };
    let mcp = match (&req.mcp, s.agent) {
        (Some(m), Some(agent)) if s.kind == SessionKind::AgentTui => Some(mcp_server(&st, &dir, agent, m)?),
        _ => None,
    };
    let resume = if req.fresh || s.kind != SessionKind::AgentTui {
        Resume::Fresh
    } else {
        match s.agent {
            Some(AgentKind::Claude) => {
                let saved = pdt_agents::launch::claude_conversation_saved(
                    st.platform.claude_config_dir.as_deref(),
                    &s.cwd,
                    s.claude_conversation_id(),
                )
                .await;
                if saved { Resume::Claude } else { Resume::Fresh }
            }
            Some(AgentKind::Codex) => s.external_session_id.clone().map(Resume::Codex).unwrap_or(Resume::Fresh),
            Some(AgentKind::Devin) => s.external_session_id.clone().map(Resume::Devin).unwrap_or(Resume::Fresh),
            _ => Resume::Fresh,
        }
    };
    // Codex, launched fresh: spaced from other fresh launches in the folder (on this instance), so its conversation can
    // be told apart by start time when it ends.
    let codex_fresh = match (s.agent, &resume) {
        (Some(AgentKind::Codex), Resume::Fresh) if s.kind == SessionKind::AgentTui => {
            let at = crate::sessions::wait_for_fresh_codex_slot(&st, &s.cwd).await;
            Some((at, s.cwd.clone(), pdt_agents::launch::canonical_folder_key(&s.cwd)))
        }
        _ => None,
    };
    let prompt = req
        .prompt
        .clone()
        .filter(|_| req.fresh)
        .map(|text| crate::sessions::InitialPrompt { text, ticket_key: req.ticket_key.clone() });
    let platform = st.agent_platform().await;
    let exit = {
        let (st, s, model) = (st.clone(), s.clone(), req.model.clone());
        tokio::task::spawn_blocking(move || {
            let spec = crate::sessions::pty_spec(
                &st,
                &platform,
                &s,
                size,
                &resume,
                req.credentials,
                prompt.as_ref(),
                instructions.as_ref(),
                mcp.as_ref(),
                model.as_deref(),
            )?;
            st.ptys.spawn(spec)
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::from)?
    };
    let proc = uuid::Uuid::now_v7().to_string();
    st.exec
        .remote
        .ptys
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(s.id.clone(), PtyRun { proc: proc.clone(), link_id: link.link_id.clone(), codex_fresh });
    let row = pdt_store::ExecSession {
        session_id: s.id.clone(),
        link_id: link.link_id.clone(),
        agent: s.agent.map(|a| a.as_str().to_string()).unwrap_or_else(|| "shell".into()),
        title: s.title.clone(),
        ticket_key: req.ticket_key.clone(),
        cwd: cwd_text.clone(),
        token: req.mcp.as_ref().map(|m| m.token.clone()).unwrap_or_default(),
        created_at: now_ms(),
        ended_at: None,
        terminal: true,
    };
    st.store.put_exec_session(&row).await?;
    let (st2, id, p, link_id) = (st.clone(), s.id.clone(), proc.clone(), link.link_id.clone());
    tokio::spawn(async move {
        let Ok(code) = exit.await else { return };
        // The Codex conversation it ran, found here (its files are this instance's): the owner's row resumes it.
        if let Some(found) = codex_conversation(&st2, &id, &p).await {
            st2.exec.remote.push(&link_id, &id, &p, WireOut::ExternalId { id: found });
        }
        let out = WireOut::Exited { code: Some(code), stderr_tail: String::new(), expected: false };
        st2.exec.remote.push(&link_id, &id, &p, out);
        let mine = {
            let ptys = st2.exec.remote.ptys.lock().unwrap_or_else(|e| e.into_inner());
            ptys.get(&id).is_some_and(|r| r.proc == p)
        };
        if !mine {
            return;
        }
        if let Err(e) = st2.store.end_exec_session(&id, now_ms()).await {
            tracing::warn!(session = %id, error = %e, "could not record a remote terminal's end");
        }
        // The owner never attaches to an ended terminal (a restart opens a new one): once anyone watching has had the
        // end, it goes, with its scrollback.
        tokio::time::sleep(FORGET_AFTER).await;
        let still = {
            let mut ptys = st2.exec.remote.ptys.lock().unwrap_or_else(|e| e.into_inner());
            let still = ptys.get(&id).is_some_and(|r| r.proc == p);
            if still {
                ptys.remove(&id);
            }
            still
        };
        if still && !st2.ptys.is_running(&id) {
            st2.remove_pty(&id).await;
        }
    });
    tracing::info!(session = %s.id, owner = %link.owner_name, "started a terminal for the owner");
    Ok(Json(Opened { proc, cwd: cwd_text }))
}

/// The Codex conversation a fresh Codex terminal (`session`'s run `proc`) created, as a local pdtd finds it
/// (`sessions::bind_codex`): in this instance's Codex home, in its folder, started at its launch time. None when it
/// took no turn, or another fresh launch in the folder was too close in time to tell them apart.
async fn codex_conversation(st: &AppState, session: &str, proc: &str) -> Option<String> {
    let home = st.platform.codex_home.clone()?;
    let (fresh, folder, peers, claimed) = {
        let ptys = st.exec.remote.ptys.lock().unwrap_or_else(|e| e.into_inner());
        let run = ptys.get(session).filter(|r| r.proc == proc)?;
        let (fresh, folder, key) = run.codex_fresh.clone()?;
        let peers: Vec<i64> = ptys
            .iter()
            .filter(|(id, _)| id.as_str() != session)
            .filter_map(|(_, r)| r.codex_fresh.as_ref().filter(|(_, _, k)| *k == key).map(|(at, _, _)| *at))
            .collect();
        (fresh, folder, peers, st.exec.remote.codex_bound.lock().unwrap_or_else(|e| e.into_inner()).clone())
    };
    let gap = i64::try_from(st.codex_spawn_spacing.as_millis()).unwrap_or(i64::MAX);
    if peers.iter().any(|p| (p - fresh).abs() < gap) {
        tracing::warn!(session, "another Codex terminal in this folder was launched too close in time; not binding");
        return None;
    }
    let cwd = PathBuf::from(&folder);
    let found = tokio::task::spawn_blocking(move || {
        pdt_agents::launch::find_codex_conversation(&home, &cwd, fresh, &peers, &claimed)
    })
    .await
    .ok()??;
    st.exec.remote.codex_bound.lock().unwrap_or_else(|e| e.into_inner()).insert(found.clone());
    Some(found)
}

/// `/link/exec/pty/attach`: a one-time ticket for the session's terminal WebSocket.
pub(crate) async fn attach_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<PtyTicket>, ApiError> {
    let (link, req): (OwnerLink, SessionReq) = checked(&st, &method, &uri, &headers, &body).await?;
    if !ours(&st, &link.link_id, &req.session_id) || st.ptys.attach(&req.session_id).is_err() {
        return Err(ApiError::conflict(codes::AGENT_NOT_RUNNING, "the terminal is not running"));
    }
    let ticket = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let now = now_ms();
    let mut tickets = st.exec.remote.tickets.lock().unwrap_or_else(|e| e.into_inner());
    tickets.retain(|_, t| t.expires_at > now);
    tickets.insert(
        ticket.clone(),
        Ticket { session_id: req.session_id, link_id: link.link_id, expires_at: now + TICKET_TTL_MS },
    );
    Ok(Json(PtyTicket { ticket }))
}

/// `/link/exec/pty/ws/{ticket}`: the owner's WebSocket to a terminal, with a ticket it got just now (once).
pub(crate) async fn ws_route(
    State(st): State<AppState>,
    Path(ticket): Path<String>,
    ApiQuery(params): ApiQuery<crate::pty_ws::PtyParams>,
    ws: Result<axum::extract::WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let t = st.exec.remote.tickets.lock().unwrap_or_else(|e| e.into_inner()).remove(&ticket);
    let Some(t) = t.filter(|t| t.expires_at > now_ms()) else {
        return Err(ApiError::forbidden(codes::LINK_REFUSED, "that terminal ticket is wrong or used"));
    };
    if !ours(&st, &t.link_id, &t.session_id) {
        return Err(ApiError::forbidden(codes::LINK_REFUSED, "that terminal isn't the owner's"));
    }
    crate::pty_ws::upgrade(st, t.session_id, params.gzip_replay(), ws)
}

/// `/link/exec/pty/write`: bytes the owner types into a terminal here.
pub(crate) async fn write_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let (link, req): (OwnerLink, PtyWriteReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(req.data.as_bytes())
        .map_err(|e| ApiError::invalid(format!("input that can't be read: {e}")))?;
    if !ours(&st, &link.link_id, &req.session_id) || !st.ptys.is_running(&req.session_id) {
        return Err(ApiError::conflict(codes::AGENT_NOT_RUNNING, "the terminal is not running"));
    }
    st.ptys
        .write(&req.session_id, &data)
        .map_err(|_| ApiError::conflict(codes::AGENT_NOT_RUNNING, "the terminal is not running"))?;
    Ok(Json(Value::Null))
}

/// A hook report for a session this instance runs for an owner: passed on to the owner (Ok), or None when `session`
/// isn't one (it is this instance's own).
pub(crate) async fn relay_activity(
    st: &AppState,
    session: &str,
    report: &pdt_core::activity::HookReport,
) -> Option<Result<(), ApiError>> {
    let e = st.store.get_exec_session(session).await.ok()??;
    Some(relay(st, &e, report).await)
}

async fn relay(
    st: &AppState,
    e: &pdt_store::ExecSession,
    report: &pdt_core::activity::HookReport,
) -> Result<(), ApiError> {
    let link = st
        .store
        .owner_link_by_id(&e.link_id)
        .await?
        .ok_or_else(|| ApiError::conflict(codes::REMOTE_PROJECT, "this instance left that project"))?;
    let key = link::load_key(st, &link.link_id).await?;
    let req = ActivityReq { session_id: e.session_id.clone(), report: report.clone() };
    let signed = Some((&key, link.link_id.as_str(), link.owner_project_id.as_str()));
    let _: Value = link::post_within(st, RELAY_TIMEOUT, &link.owner_url, ACTIVITY_PATH, signed, &req)
        .await
        .map_err(|err: PeerError| err.into_api(&link.owner_name))?;
    Ok(())
}
