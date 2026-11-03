//! What PDT and integrations read of a loaded conversation's runtime state.

use super::conversation_session;
use super::routes::answer_request;
use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use pdt_core::conversation::{BackgroundTask, ConversationEvent};
use pdt_core::session::Session;
use tokio::sync::broadcast;

/// Whether PDT may send a message of its own now (a PR notice): no turn running, nothing asked or queued,
/// no rate limit hit, its terminal not open beside it, and nothing happening for `notice_quiet`. A conversation not
/// loaded since pdtd started is idle.
pub(crate) fn ready_for_notice(st: &AppState, id: &str) -> bool {
    let Some(c) = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned() else {
        return true;
    };
    let s = c.state();
    !s.turn_running
        && s.open_requests.is_empty()
        && s.pending.is_empty()
        && !s.terminal_open
        && s.limited_until.is_none_or(|t| t <= now_ms())
        && s.last_active.elapsed() >= st.conversations.notice_quiet
}

/// What keeps a conversation from taking PDT's message now (`busy_with`).
pub(crate) enum Busy {
    Turn,
    Request,
    Message,
    Terminal,
    RateLimit,
}

/// Why a loaded conversation is not ready for a notice, if one of these is it (a quiet spell is not).
pub(crate) fn busy_with(st: &AppState, id: &str) -> Option<Busy> {
    let c = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()?;
    let s = c.state();
    if s.turn_running {
        Some(Busy::Turn)
    } else if !s.open_requests.is_empty() {
        Some(Busy::Request)
    } else if !s.pending.is_empty() {
        Some(Busy::Message)
    } else if s.terminal_open {
        Some(Busy::Terminal)
    } else if s.limited_until.is_some_and(|t| t > now_ms()) {
        Some(Busy::RateLimit)
    } else {
        None
    }
}

/// The conversations running a turn, or with a message on its way to one (the host's cap counts them).
pub(crate) fn busy_ids(st: &AppState) -> Vec<String> {
    let map = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner());
    map.iter()
        .filter_map(|(id, c)| {
            let s = c.state();
            (s.turn_running || !s.pending.is_empty()).then(|| id.clone())
        })
        .collect()
}

/// The open permission requests of live conversations, by session (the Inbox's approvals).
pub(crate) fn open_requests(st: &AppState) -> Vec<(String, Vec<String>)> {
    let map = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner());
    map.iter()
        .filter_map(|(id, c)| {
            let s = c.state();
            (!s.open_requests.is_empty()).then(|| (id.clone(), s.open_requests.iter().cloned().collect()))
        })
        .collect()
}

/// Whether a conversation can still finish what it was doing: its process is live, a turn runs, or a message waits to
/// go in. False after its process died or pdtd restarted (an agent step then counts as stalled).
pub(crate) fn is_live_or_waiting(st: &AppState, s: &Session) -> bool {
    let Some(c) = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(&s.id).cloned() else {
        return false;
    };
    let state = c.state();
    state.live || state.turn_running || !state.pending.is_empty()
}

/// Every conversation's stored events from now on, each with its session's id (an integration that holds PM
/// conversations elsewhere follows them, PDT-304). A receiver that falls behind skips what it missed.
pub fn subscribe_all(st: &AppState) -> broadcast::Receiver<(String, ConversationEvent)> {
    st.conversations.all.subscribe()
}

/// Whether a loaded conversation is running a turn or has a message on its way to one.
pub fn is_working(st: &AppState, id: &str) -> bool {
    let Some(c) = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned() else {
        return false;
    };
    let s = c.state();
    s.turn_running || !s.pending.is_empty()
}

/// The user's answer to one of a conversation's open requests, given elsewhere than PDT's web (PDT-304: a button in
/// Discord that only the user's own account can press): it counts as theirs, the same as an answer from the web.
pub async fn answer_as_user(
    st: &AppState,
    session_id: &str,
    request_id: &str,
    answer: &pdt_core::conversation::Answer,
) -> Result<(), ApiError> {
    let session = conversation_session(st, session_id).await?;
    answer_request(st, &session, request_id, answer, None, true).await?;
    let approved = matches!(answer, pdt_core::conversation::Answer::Approve { .. });
    st.grants.answered(&session.id, request_id, approved);
    Ok(())
}

/// Whether a conversation's agent process is running (false: parked, or not loaded).
pub fn conversation_live(st: &AppState, id: &str) -> bool {
    st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|c| c.state().live)
}

/// Whether a conversation is loaded in this pdtd. Removing a session forgets it first, so a turn its closing ended
/// finds it gone.
pub(crate) fn is_loaded(st: &AppState, id: &str) -> bool {
    st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).contains_key(id)
}

/// A loaded conversation's background tasks (PDT-142).
pub(crate) struct Background {
    pub tasks: Vec<BackgroundTask>,
    /// Its agent process is running.
    pub live: bool,
    /// A turn runs, something is asked, or a message is on its way: it is at work, not waiting.
    pub busy: bool,
    /// None run, and nothing has happened for `background_grace`: no turn, request or message on its way. A task that
    /// finished without the agent taking it up leaves it settled.
    pub settled: bool,
}

/// The background tasks of a conversation loaded in this pdtd; None for one that isn't (or a terminal).
pub(crate) fn background(st: &AppState, id: &str) -> Option<Background> {
    let c = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()?;
    let s = c.state();
    let settled = s.background_tasks.is_empty()
        && !s.turn_running
        && s.open_requests.is_empty()
        && s.pending.is_empty()
        && s.last_active.elapsed() >= st.conversations.background_grace;
    let busy = s.turn_running || !s.open_requests.is_empty() || !s.pending.is_empty();
    Some(Background { tasks: s.background_tasks.clone(), live: s.live, busy, settled })
}
