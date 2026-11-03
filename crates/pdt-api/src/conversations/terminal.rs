//! A conversation's terminal: Codex's live shared TUI, and Claude or Devin handed between the conversation and its own
//! TUI.

use super::live::{append, conv, ensure_live, park, refresh_activity};
use super::{codex_server, conversation_session, login_path_env, scrubbed};
use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_agents::adapter::codex::TOKEN_ENV;
use pdt_core::conversation::{AgentEvent, NoticeLevel, SessionMode};
use pdt_core::session::{AgentKind, Session, SessionKind};
use pdt_store::NewEvent;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// ---- the live shared terminal (Codex) ----

#[derive(serde::Deserialize)]
pub(crate) struct TerminalSizeBody {
    cols: u16,
    rows: u16,
}

/// `POST /api/sessions/{id}/terminal`: opens the real Codex TUI on this conversation's thread, attached live to PDT's
/// app-server, in a terminal keyed by the session id (`/api/sessions/{id}/pty`). Nothing happens when it is open.
pub(crate) async fn open_terminal(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TerminalSizeBody>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(size) = payload?;
    let size = pdt_core::session::TerminalSize { cols: size.cols, rows: size.rows }.validate()?;
    let session = conversation_session(&st, &id).await?;
    if session.agent != Some(AgentKind::Codex) {
        return Err(ApiError::conflict(
            codes::TERMINAL_UNSUPPORTED,
            "only Codex conversations open in a live terminal so far",
        ));
    }
    let _guard = st.lock_session(&id).await;
    if st.ptys.is_running(&id) {
        return Ok(StatusCode::NO_CONTENT);
    }
    // Codex saves a thread only once it has taken a turn; the TUI cannot resume one before that.
    if st.store.session_turn_ids(&id).await?.is_empty() {
        return Err(ApiError::conflict(codes::NO_TURN_YET, "send a first message, then open the terminal"));
    }
    let c = conv(&st, &session).await?;
    ensure_live(&st, &c, &session).await?;
    // The pump records a new thread's id just after it opens.
    let mut thread = None;
    for _ in 0..100 {
        thread = st.store.get_session(&id).await?.and_then(|s| s.external_session_id);
        if thread.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let thread = thread.ok_or_else(|| {
        ApiError::conflict(codes::AGENT_NOT_RUNNING, "Codex has not started this conversation's thread")
    })?;
    let server = codex_server(&st).await?;
    let endpoint =
        server.endpoint().await.ok_or_else(|| ApiError::conflict(codes::AGENT_NOT_RUNNING, "Codex is not running"))?;
    let args = vec![
        "resume".to_string(),
        thread,
        "--remote".into(),
        endpoint.url,
        "--remote-auth-token-env".into(),
        TOKEN_ENV.into(),
    ];
    let resolved =
        pdt_agents::launch::resolve_launch(&st.platform.codex_program, &args, st.platform.login_path.as_deref())?;
    let mut env = pdt_core::session::terminal_env();
    env.extend(login_path_env(&st));
    env.push((TOKEN_ENV.to_string(), endpoint.token));
    let creds = st.store.get_host_settings().await?.agent_credentials;
    st.remove_pty(&id).await;
    let exit = st.ptys.spawn(pdt_agents::SpawnSpec {
        id: id.clone(),
        program: resolved.program,
        args: resolved.args,
        cwd: PathBuf::from(&session.cwd),
        env_remove: scrubbed(creds),
        env,
        cols: size.cols,
        rows: size.rows,
        output_file: None,
    })?;
    c.state().terminal_open = true;
    c.push_state();
    let c2 = c.clone();
    tokio::spawn(async move {
        let _ = exit.await;
        // Closing the TUI leaves the conversation as it was.
        c2.state().terminal_open = false;
        c2.push_state();
    });
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/sessions/{id}/terminal`: closes the live TUI (the conversation goes on).
pub(crate) async fn close_terminal(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let session = conversation_session(&st, &id).await?;
    let _guard = st.lock_session(&id).await;
    if st.ptys.is_running(&id) {
        st.kill_pty(&id).await?;
    }
    let c = conv(&st, &session).await?;
    c.state().terminal_open = false;
    c.push_state();
    Ok(StatusCode::NO_CONTENT)
}

// ---- Claude: one at a time, conversation or terminal ----

/// How long the TUI gets to leave after `/exit` before it is killed.
const TUI_EXIT_GRACE: Duration = Duration::from_secs(5);

/// `POST /api/sessions/{id}/mode`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SwitchMode {
    mode: SessionMode,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
    /// Interrupt a running turn instead of refusing (`turn_running`).
    #[serde(default)]
    interrupt: bool,
}

/// `POST /api/sessions/{id}/mode`: hands a Claude session between the conversation and Claude's own TUI. Codex
/// conversations keep their live shared terminal (`…/terminal`) instead.
pub(crate) async fn switch_mode(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<SwitchMode>, JsonRejection>,
) -> Result<Json<Session>, ApiError> {
    let Json(body) = payload?;
    let _guard = st.lock_session(&id).await;
    let session = st.session(&id).await?;
    if session.kind != SessionKind::AgentTui || !matches!(session.agent, Some(AgentKind::Claude | AgentKind::Devin)) {
        return Err(ApiError::conflict(
            codes::SWITCH_UNSUPPORTED,
            "only Claude and Devin sessions switch between conversation and terminal (Codex conversations open a live \
             terminal)",
        ));
    }
    if session.mode != body.mode {
        match body.mode {
            SessionMode::Terminal => {
                let size =
                    pdt_core::session::TerminalSize { cols: body.cols.unwrap_or(120), rows: body.rows.unwrap_or(30) }
                        .validate()?;
                to_terminal(&st, &session, size, body.interrupt).await?;
            }
            SessionMode::Conversation => to_conversation(&st, &session).await?,
        }
    }
    let session = st.session(&id).await?;
    Ok(Json(session))
}

/// A Claude conversation's transcript, read off the async runtime (None: Claude has not saved one, or no config dir
/// known).
pub(super) async fn read_transcript(st: &AppState, conversation_id: &str) -> Option<String> {
    let id = conversation_id;
    let dir = st.platform.claude_config_dir.clone()?;
    let id = id.to_string();
    tokio::task::spawn_blocking(move || {
        pdt_agents::launch::claude_transcript(&dir, &id).and_then(|p| std::fs::read_to_string(p).ok())
    })
    .await
    .ok()
    .flatten()
}

/// Conversation → terminal: closes the `-p` process (interrupting a turn only when asked), remembers where PDT's log
/// ends in Claude's transcript, and starts the TUI on the same conversation as a restart would (hooks, `--resume`).
async fn to_terminal(
    st: &AppState,
    session: &Session,
    size: pdt_core::session::TerminalSize,
    interrupt: bool,
) -> Result<(), ApiError> {
    let c = conv(st, session).await?;
    let busy = {
        let s = c.state();
        s.turn_running || !s.pending.is_empty() || !s.open_requests.is_empty()
    };
    if busy && !interrupt {
        let agent = session.agent.unwrap_or(AgentKind::Claude).label();
        return Err(ApiError::conflict(
            codes::TURN_RUNNING,
            format!("{agent} is working; interrupt it to open the terminal"),
        ));
    }
    park(st, &c).await;
    let notice =
        AgentEvent::Notice { level: NoticeLevel::Info, text: "Continued in the terminal.".into(), until: None };
    append(st, &c, None, None, notice, None).await;
    // "" means "from the start": Claude had not saved this conversation yet. The row is read again: the process
    // may have moved to a new conversation (`/clear`) since `session` was read.
    let row = st.store.get_session(&session.id).await?.unwrap_or_else(|| session.clone());
    let anchor = if session.agent == Some(AgentKind::Devin) {
        // Devin stamps each prompt: the terminal's turns are the ones after now.
        now_ms().to_string()
    } else {
        read_transcript(st, row.claude_conversation_id())
            .await
            .and_then(|t| pdt_core::conversation::claude::last_uuid(&t))
            .unwrap_or_default()
    };
    st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).remove(&session.id);
    st.store.switch_session_mode(&session.id, SessionMode::Terminal, Some(&anchor)).await?;
    if let Err(e) = crate::sessions::restart_locked(st, &session.id, size, None).await {
        st.store.switch_session_mode(&session.id, SessionMode::Conversation, None).await?;
        return Err(e);
    }
    Ok(())
}

/// Terminal → conversation: asks the TUI to `/exit` (killing it after a grace period), imports the turns it took
/// (all of them for a session that was always a terminal) and makes the session a conversation again. The
/// next message resumes it headless.
async fn to_conversation(st: &AppState, session: &Session) -> Result<(), ApiError> {
    let id = session.id.as_str();
    if st.ptys.is_running(id) {
        // Typed, then Enter on its own: a single write would look like a paste to the TUI.
        let _ = st.ptys.write(id, b"/exit");
        tokio::time::sleep(Duration::from_millis(150)).await;
        let _ = st.ptys.write(id, b"\r");
        let deadline = Instant::now() + TUI_EXIT_GRACE;
        while st.ptys.is_running(id) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if st.ptys.is_running(id) {
            st.kill_pty(id).await?;
        }
    }
    crate::sessions::settle_exit(st, id).await;
    st.remove_pty(id).await;
    if session.agent == Some(AgentKind::Devin) {
        return devin_to_conversation(st, id).await;
    }
    let after = import_anchor(st, id).await?;
    st.store.switch_session_mode(id, SessionMode::Conversation, None).await?;
    st.store.set_session_running(id).await?;
    let session = st.session(id).await?;
    let c = conv(st, &session).await?;
    for (item_id, turn_id, event) in terminal_turns(st, &session, after).await {
        append(st, &c, item_id.as_deref(), turn_id.as_deref(), event, None).await;
    }
    refresh_activity(st, &c).await;
    c.push_state();
    Ok(())
}

/// Devin, terminal → conversation: its terminal has quit, so the ACP server can take the session (Devin locks a
/// session to one process). The turns the terminal took arrive with `session/load`'s replay when the conversation
/// opens, which happens now; `history_from` stays until that import (a failed open retries it on the next message).
async fn devin_to_conversation(st: &AppState, id: &str) -> Result<(), ApiError> {
    let anchor = match st.store.history_from(id).await? {
        Some(a) => Some(a),
        // Always a terminal: import every turn.
        None if st.store.last_session_event_seq(id).await? == 0 => Some(String::new()),
        None => None,
    };
    st.store.switch_session_mode(id, SessionMode::Conversation, anchor.as_deref()).await?;
    st.store.set_session_running(id).await?;
    let session = st.session(id).await?;
    let c = conv(st, &session).await?;
    if let Err(e) = ensure_live(st, &c, &session).await {
        tracing::warn!(session = %id, error = %e.message(), "could not reopen the Devin conversation after its terminal");
    }
    refresh_activity(st, &c).await;
    c.push_state();
    Ok(())
}

/// Where to import a terminal session's transcript from: `Some(None)` from its start, `Some(Some(uuid))` after
/// the entry PDT's log ends at, None not at all.
pub(super) async fn import_anchor(st: &AppState, id: &str) -> Result<Option<Option<String>>, ApiError> {
    let anchor = st.store.history_from(id).await?;
    let logged = st.store.last_session_event_seq(id).await? > 0;
    Ok(match anchor.as_deref() {
        Some("") => Some(None),
        Some(uuid) => Some(Some(uuid.to_string())),
        // Never handed over by PDT: import everything, unless PDT already has a log it cannot line up.
        None if logged => None,
        None => Some(None),
    })
}

type Imported = (Option<String>, Option<String>, AgentEvent);

/// The turns the TUI took in the session's current Claude conversation, after `after` (see
/// `import_anchor`); a warning instead when the transcript no longer has that entry.
async fn terminal_turns(st: &AppState, session: &Session, after: Option<Option<String>>) -> Vec<Imported> {
    let Some(after) = after else { return Vec::new() };
    let Some(transcript) = read_transcript(st, session.claude_conversation_id()).await else { return Vec::new() };
    match pdt_core::conversation::claude::history_events(&transcript, after.as_deref(), Some(&session.cwd)) {
        Ok(events) => events
            .into_iter()
            .filter_map(|out| match out {
                pdt_core::conversation::claude::MapOut::Event { item_id, turn_id, event } => {
                    Some((item_id, turn_id, event))
                }
                _ => None,
            })
            .collect(),
        Err(e) => {
            let text = format!("{e}: turns taken in the terminal are not shown here.");
            vec![(None, None, AgentEvent::Notice { level: NoticeLevel::Warning, text, until: None })]
        }
    }
}

/// A terminal Claude session moved to another conversation (`/clear`, or `/resume` in the TUI), as its SessionStart
/// hook reported. The turns it took in the old one are logged now, while that transcript is still the one PDT's log
/// lines up with, then the divider; from here on the session resumes `new_id`, and switching to a conversation
/// imports the new transcript from its start. Written straight to the store: the session's activity is the
/// terminal's, not derived from these events.
pub(crate) async fn terminal_conversation_moved(
    st: &AppState,
    session: &Session,
    new_id: &str,
    trigger: &str,
) -> Result<(), ApiError> {
    let id = session.id.as_str();
    let after = import_anchor(st, id).await?;
    let mut events = terminal_turns(st, session, after).await;
    events.push((None, None, AgentEvent::ConversationReset { trigger: trigger.to_string() }));
    for (item_id, turn_id, event) in &events {
        let ne = NewEvent { ts: now_ms(), item_id: item_id.as_deref(), turn_id: turn_id.as_deref(), event, raw: None };
        st.store.upsert_session_event(id, ne).await?;
    }
    st.store.set_external_session_id(id, new_id).await?;
    st.store.switch_session_mode(id, session.mode, Some("")).await?;
    tracing::info!(session = %id, conversation = %new_id, trigger, "the terminal moved to another Claude conversation");
    Ok(())
}
