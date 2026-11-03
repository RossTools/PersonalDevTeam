//! PDT's own requests in a conversation (PDT-490): an asking tool's call from an agent that can't ask in its own
//! prompt (Codex, Devin, ACP agents, Claude in Bypass) shows as an approval card in the conversation, and the call
//! waits for the answer. The card is also the Inbox's approval item (`inbox::approvals`), so answering either answers
//! both.

use super::Conv;
use super::live::append;
use crate::error::{ApiError, codes};
use crate::routes::AppState;
use pdt_agents::adapter::AdapterError;
use pdt_core::conversation::{AgentEvent, Answer, RequestKind, RequestState, SessionMode};
use pdt_core::session::Session;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// The ids of PDT's own requests start with it: the agent never sees them, and PDT never answers them for the user.
const PREFIX: &str = "pdt-ask-";

/// The answer, and whether the user gave it (PDT's web client, or their own account elsewhere).
type Reply = (Answer, bool);

/// PDT's requests waiting for an answer, by `(session, request)`, and the tools the user always allowed in a session.
#[derive(Default)]
pub(crate) struct Held {
    waiting: Mutex<HashMap<(String, String), oneshot::Sender<Reply>>>,
    always: Mutex<HashMap<String, HashSet<String>>>,
}

/// How the user answered one of PDT's requests.
#[derive(Debug, PartialEq)]
pub(crate) enum Asked {
    Allowed,
    /// With the user's reason, if they gave one.
    Denied(Option<String>),
    /// The turn or the agent ended before the user answered.
    Ended,
}

/// Whether a request is one of PDT's own (not the agent's).
pub(crate) fn is_held(request_id: &str) -> bool {
    request_id.starts_with(PREFIX)
}

/// Whether the user chose "Always allow in this session" for `tool` in this session.
pub(crate) fn allowed_always(st: &AppState, session_id: &str, tool: &str) -> bool {
    let always = st.conversations.held.always.lock().unwrap_or_else(|e| e.into_inner());
    always.get(session_id).is_some_and(|t| t.contains(tool))
}

/// Asks the user about an asking tool's call (`tool`, PDT's own name for it) in the session's conversation and waits
/// for the answer. None when the session isn't a conversation loaded in this pdtd (a terminal): it can't ask there.
/// `can_always`: whether "Always allow in this session" is offered.
pub(crate) async fn ask(st: &AppState, s: &Session, tool: &str, input: &Value, can_always: bool) -> Option<Asked> {
    if s.mode != SessionMode::Conversation {
        return None;
    }
    let c = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(&s.id).cloned()?;
    let id = format!("{PREFIX}{}", uuid::Uuid::now_v7());
    let (tx, rx) = oneshot::channel();
    st.conversations.held.waiting.lock().unwrap_or_else(|e| e.into_inner()).insert((s.id.clone(), id.clone()), tx);
    let title = st
        .registry()
        .tool_infos()
        .into_iter()
        .find(|t| t.name == tool)
        .map(|t| t.title)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| tool.to_string());
    // In the turn that made the call, so the card shows with its tools.
    let turn =
        st.store.session_events_of_kind(&s.id, "turn").await.ok().and_then(|t| {
            t.last().filter(|e| matches!(e.event, AgentEvent::Turn { .. })).and_then(|e| e.turn_id.clone())
        });
    let open = Card {
        st: st.clone(),
        c,
        session_id: s.id.clone(),
        id: id.clone(),
        turn,
        event: AgentEvent::Request {
            id: id.clone(),
            request_kind: RequestKind::Approval,
            tool: format!("mcp__{}__{tool}", pdt_core::mcp::SERVER_NAME),
            title,
            description: None,
            input: st.store.redactor().json(input),
            tool_call_id: None,
            can_always,
            diff: None,
            state: RequestState::Open,
            answer: None,
        },
        done: false,
    };
    open.show(RequestState::Open, None).await;
    crate::notifications::kick(st);
    // Dropped unanswered (the agent gave up on the call), the card expires ([`Card`]'s drop).
    let mut open = open;
    let reply = rx.await;
    open.done = true;
    let asked = match reply {
        Ok((answer, by_user)) => {
            open.show(RequestState::Answered, Some(answer.clone())).await;
            match answer {
                Answer::Approve { always } => {
                    if always && can_always {
                        let mut a = st.conversations.held.always.lock().unwrap_or_else(|e| e.into_inner());
                        a.entry(s.id.clone()).or_default().insert(tool.to_string());
                    }
                    // The user's own yes is good for this very call (PDT-201), as for an agent's prompt.
                    if by_user {
                        st.grants.hold(&s.id, &id, tool, input);
                        st.grants.answered(&s.id, &id, true);
                    }
                    Asked::Allowed
                }
                Answer::Deny { message } => Asked::Denied(message),
                _ => Asked::Denied(None),
            }
        }
        Err(_) => {
            open.show(RequestState::Expired, None).await;
            Asked::Ended
        }
    };
    crate::notifications::kick(st);
    Some(asked)
}

/// Answers one of PDT's requests in a session. `by_user`: the user gave the answer (not an agent's curl).
pub(crate) fn answer(
    st: &AppState,
    session_id: &str,
    request_id: &str,
    answer: &Answer,
    by_user: bool,
) -> Result<(), ApiError> {
    if !matches!(answer, Answer::Approve { .. } | Answer::Deny { .. }) {
        return Err(super::adapter_error(AdapterError::WrongAnswer));
    }
    // An agent's curl can't say yes for the user (PDT-204), as with the Inbox's Allow.
    if matches!(answer, Answer::Approve { .. }) && !by_user {
        return Err(ApiError::conflict(codes::CANNOT_ANSWER, "only the user can allow this call"));
    }
    let tx = st
        .conversations
        .held
        .waiting
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(session_id.to_string(), request_id.to_string()))
        .ok_or_else(|| super::adapter_error(AdapterError::RequestClosed))?;
    tx.send((answer.clone(), by_user)).map_err(|_| super::adapter_error(AdapterError::RequestClosed))
}

/// A session's agent ended: its open requests of PDT's expire, and their calls go on without running.
pub(crate) fn end_session(st: &AppState, session_id: &str) {
    st.conversations.held.waiting.lock().unwrap_or_else(|e| e.into_inner()).retain(|(s, _), _| s != session_id);
}

/// A session's turn ended: these requests of PDT's (those open at its end) expire, and their calls go on without
/// running. One opened since stays open.
pub(crate) fn expire(st: &AppState, session_id: &str, request_ids: &[String]) {
    if request_ids.is_empty() {
        return;
    }
    let mut waiting = st.conversations.held.waiting.lock().unwrap_or_else(|e| e.into_inner());
    for id in request_ids {
        waiting.remove(&(session_id.to_string(), id.clone()));
    }
}

/// An open request's card in the conversation; one dropped before its answer (the call was given up) expires.
struct Card {
    st: AppState,
    c: Arc<Conv>,
    session_id: String,
    id: String,
    turn: Option<String>,
    event: AgentEvent,
    done: bool,
}

impl Card {
    async fn show(&self, state: RequestState, answer: Option<Answer>) {
        show(&self.st, &self.c, &self.id, self.turn.as_deref(), &self.event, state, answer).await;
    }
}

impl Drop for Card {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let key = (self.session_id.clone(), self.id.clone());
        self.st.conversations.held.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
        let (st, c, id, turn, event) =
            (self.st.clone(), self.c.clone(), self.id.clone(), self.turn.clone(), self.event.clone());
        tokio::spawn(async move { show(&st, &c, &id, turn.as_deref(), &event, RequestState::Expired, None).await });
    }
}

async fn show(
    st: &AppState,
    c: &Arc<Conv>,
    id: &str,
    turn: Option<&str>,
    event: &AgentEvent,
    state: RequestState,
    answer: Option<Answer>,
) {
    let mut e = event.clone();
    if let AgentEvent::Request { state: s, answer: a, .. } = &mut e {
        *s = state;
        *a = answer;
    }
    let item = pdt_core::conversation::claude::request_item(id);
    append(st, c, Some(&item), turn, e, None).await;
}
