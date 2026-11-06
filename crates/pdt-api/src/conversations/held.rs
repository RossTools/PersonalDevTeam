//! PDT's own requests in a conversation (PDT-490): an asking tool's call from an agent that can't ask in its own
//! prompt (Codex, Devin, ACP agents, Claude in Bypass) shows as an approval card in the conversation, and the call
//! waits for the answer. The card is also the Inbox's approval item (`inbox::approvals`), so answering either answers
//! both. A batch call's card (the PM's `inbox_decide`, PDT-502) carries its rows: the user unticks the ones they don't
//! want, and their Allow says which they kept.

use super::Conv;
use super::live::append;
use crate::error::{ApiError, codes};
use crate::mcp::tools::Answered;
use crate::routes::AppState;
use pdt_agents::adapter::AdapterError;
use pdt_core::conversation::{AgentEvent, Answer, RequestKind, RequestState, SessionMode};
use pdt_core::inbox_decide::DecideRow;
use pdt_core::session::Session;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// The ids of PDT's own requests start with it: the agent never sees them, and PDT never answers them for the user.
const PREFIX: &str = "pdt-ask-";

/// The answer, and the rows an Allow kept (a card with rows; every row when None).
type Reply = (Answer, Option<Vec<u32>>);

/// Counts the requests PDT opened, so a turn's end can tell the ones opened before it from the next turn's ([`mark`]).
static OPENED: AtomicU64 = AtomicU64::new(0);

/// A request waiting for its answer.
struct Waiting {
    /// Its place in [`OPENED`]'s count.
    n: u64,
    /// How many rows its card has (None: no rows).
    rows: Option<usize>,
    /// Where the answer goes.
    tx: oneshot::Sender<Reply>,
    /// Its call returned "still waiting" (PDT-589): the card outlives the turn, and the answer comes as a message.
    detached: bool,
}

/// PDT's requests waiting for an answer, by `(session, request)`, and the tools the user always allowed in a session.
#[derive(Default)]
pub(crate) struct Held {
    waiting: Mutex<HashMap<(String, String), Waiting>>,
    always: Mutex<HashMap<String, HashSet<String>>>,
}

impl Held {
    /// Drops the session's waiting requests opened before `before` ([`mark`]), or all of them: their calls go on
    /// without running. A turn's end keeps the ones whose call no longer waits: the user still answers them.
    fn expire(&self, session_id: &str, before: Option<u64>) {
        let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
        waiting.retain(|(s, _), w| s != session_id || before.is_some_and(|b| w.n >= b || w.detached));
    }

    /// Marks a request whose call stops waiting (PDT-589): false when it is no longer waiting (just answered or
    /// expired), and its reply is in.
    fn detach(&self, session_id: &str, request_id: &str) -> bool {
        let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
        let key = (session_id.to_string(), request_id.to_string());
        waiting.get_mut(&key).map(|w| w.detached = true).is_some()
    }
}

/// Where the count of PDT's requests is now: taken as a turn's end is read, it marks the requests that turn left. Its
/// expiry runs only once the end is stored, and by then the agent's next call may have opened its own (PDT-496).
pub(crate) fn mark() -> u64 {
    OPENED.load(Ordering::SeqCst)
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
/// for the answer ([`crate::mcp::tools::Answered`]; `Ended` when the turn or the agent ended first). None when the
/// session isn't a conversation loaded in this pdtd (a terminal): it can't ask there. `can_always`: whether "Always
/// allow in this session" is offered. `rows`: a batch call's rows the user can untick.
pub(crate) async fn ask(
    st: &AppState,
    s: &Session,
    tool: &str,
    input: &Value,
    can_always: bool,
    rows: Option<Vec<DecideRow>>,
) -> Option<Answered> {
    if s.mode != SessionMode::Conversation {
        return None;
    }
    let c = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(&s.id).cloned()?;
    let id = format!("{PREFIX}{}", uuid::Uuid::now_v7());
    let (tx, rx) = oneshot::channel();
    let n = OPENED.fetch_add(1, Ordering::SeqCst);
    let count = rows.as_ref().map(Vec::len);
    st.conversations
        .held
        .waiting
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert((s.id.clone(), id.clone()), Waiting { n, rows: count, tx, detached: false });
    let keys: Vec<String> = rows.iter().flatten().map(|r| r.ticket_key.clone()).collect();
    let title = crate::mcp::tools::tool_title(st, tool);
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
            title: title.clone(),
            description: None,
            input: st.store.redactor().json(input),
            tool_call_id: None,
            can_always: can_always && rows.is_none(),
            diff: None,
            rows,
            state: RequestState::Open,
            answer: None,
        },
        done: false,
    };
    open.show(RequestState::Open, None).await;
    crate::notifications::kick(st);
    // Dropped unanswered (the agent gave up on the call), the card expires ([`Card`]'s drop).
    let mut open = open;
    let mut rx = rx;
    let reply = match tokio::time::timeout(st.ask_wait, &mut rx).await {
        Ok(reply) => reply,
        Err(_) if st.conversations.held.detach(&s.id, &id) => {
            // Back to the agent before its own timeout; the card stays for the user (PDT-589).
            let (st, s, tool, input) = (st.clone(), s.clone(), tool.to_string(), input.clone());
            tokio::spawn(async move {
                let reply = rx.await;
                let answered = settle(&st, &s, &mut open, reply, &tool, can_always).await;
                answered_late(&st, &s, &tool, &title, input, &keys, answered).await;
            });
            return Some(Answered::Waiting);
        }
        Err(_) => rx.await,
    };
    Some(settle(st, s, &mut open, reply, tool, can_always).await)
}

/// Shows the card answered (or expired) and reads the answer. Only the user's own click allows ([`answer`]).
async fn settle(
    st: &AppState,
    s: &Session,
    open: &mut Card,
    reply: Result<Reply, oneshot::error::RecvError>,
    tool: &str,
    can_always: bool,
) -> Answered {
    open.done = true;
    let answered = match reply {
        Ok((answer, kept)) => {
            open.show(RequestState::Answered, Some(answer.clone())).await;
            match answer {
                Answer::Approve { always } => {
                    if always && can_always {
                        let mut a = st.conversations.held.always.lock().unwrap_or_else(|e| e.into_inner());
                        a.entry(s.id.clone()).or_default().insert(tool.to_string());
                    }
                    Answered::Allowed(kept)
                }
                Answer::Deny { message } => Answered::Denied(message),
                _ => Answered::Denied(None),
            }
        }
        Err(_) => {
            open.show(RequestState::Expired, None).await;
            Answered::Ended
        }
    };
    crate::notifications::kick(st);
    answered
}

/// The user's answer to a card whose call stopped waiting (PDT-589): an Allow runs the call with the rows they kept
/// (`keys`: its rows' tickets), and the session hears the result or the Deny as a message.
async fn answered_late(
    st: &AppState,
    s: &Session,
    tool: &str,
    title: &str,
    input: Value,
    keys: &[String],
    answered: Answered,
) {
    use crate::workflow::agent as steps;
    let text = match answered {
        Answered::Allowed(kept) => {
            let narrowed =
                if keys.is_empty() { Ok((input, vec![])) } else { steps::keep_rows(input, keys, kept.as_deref()) };
            match narrowed {
                Ok((input, dropped)) => steps::run_allowed(st, s, tool, title, input, &dropped).await.0,
                Err(e) => format!("[PDT] The user allowed your {tool} call ({title}), but it failed: {e}"),
            }
        }
        Answered::Denied(why) => steps::denied_text(tool, title, why),
        // The agent ended: nothing to run, and no one to tell.
        Answered::Ended | Answered::Waiting => return,
    };
    let ticket = crate::mcp::tools::session_ticket(st, s).await;
    if let Err(e) = steps::queue_message(st, s, &text, None, ticket, steps::USER).await {
        tracing::warn!(session = %s.id, error = %e, "could not tell the session the user's late answer");
    }
}

/// Answers one of PDT's requests in a session. `by_user`: the user gave the answer (not an agent's curl). `rows`: the
/// rows an Allow keeps, for a card with rows (every row when None).
pub(crate) fn answer(
    st: &AppState,
    session_id: &str,
    request_id: &str,
    answer: &Answer,
    by_user: bool,
    rows: Option<&[u32]>,
) -> Result<(), ApiError> {
    if !matches!(answer, Answer::Approve { .. } | Answer::Deny { .. }) {
        return Err(super::adapter_error(AdapterError::WrongAnswer));
    }
    // An agent's curl can't say yes for the user (PDT-204), as with the Inbox's Allow.
    if matches!(answer, Answer::Approve { .. }) && !by_user {
        return Err(ApiError::conflict(codes::CANNOT_ANSWER, "only the user can allow this call"));
    }
    let key = (session_id.to_string(), request_id.to_string());
    let mut waiting = st.conversations.held.waiting.lock().unwrap_or_else(|e| e.into_inner());
    let count = waiting.get(&key).ok_or_else(|| super::adapter_error(AdapterError::RequestClosed))?.rows;
    // The kept rows are checked before the request is answered, so a wrong pick leaves the card open.
    let kept = match (answer, count) {
        (Answer::Approve { .. }, Some(n)) => {
            let kept = pdt_core::inbox_decide::kept(n, rows).map_err(ApiError::invalid)?;
            Some(kept.into_iter().map(|r| r as u32).collect())
        }
        _ if rows.is_some() => return Err(ApiError::invalid("only an Allow of a request with rows keeps rows")),
        _ => None,
    };
    let tx = waiting.remove(&key).ok_or_else(|| super::adapter_error(AdapterError::RequestClosed))?.tx;
    drop(waiting);
    tx.send((answer.clone(), kept)).map_err(|_| super::adapter_error(AdapterError::RequestClosed))
}

/// A session's agent ended: its open requests of PDT's expire, and their calls go on without running.
pub(crate) fn end_session(st: &AppState, session_id: &str) {
    st.conversations.held.expire(session_id, None);
}

/// A session's turn ended: PDT's requests opened before its end was read (`before`, [`mark`]) expire.
pub(crate) fn end_turn(st: &AppState, session_id: &str, before: u64) {
    st.conversations.held.expire(session_id, Some(before));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(held: &Held, session: &str, request: &str) -> oneshot::Receiver<Reply> {
        let (tx, rx) = oneshot::channel();
        let n = OPENED.fetch_add(1, Ordering::SeqCst);
        held.waiting
            .lock()
            .unwrap()
            .insert((session.into(), request.into()), Waiting { n, rows: None, tx, detached: false });
        rx
    }

    /// A turn's end expires only what was open when it was read: a request the next turn opened while the end was
    /// still being stored stays open (PDT-496). The agent's exit expires everything.
    #[test]
    fn a_turn_end_expires_only_the_requests_opened_before_it() {
        let held = Held::default();
        let mut left = wait(&held, "s", "pdt-ask-1");
        let mut other = wait(&held, "t", "pdt-ask-2");
        let before = mark();
        let mut next = wait(&held, "s", "pdt-ask-3");
        held.expire("s", Some(before));
        assert!(left.try_recv().is_err_and(|e| e == oneshot::error::TryRecvError::Closed));
        assert!(next.try_recv().is_err_and(|e| e == oneshot::error::TryRecvError::Empty));
        assert!(other.try_recv().is_err_and(|e| e == oneshot::error::TryRecvError::Empty));
        held.expire("s", None);
        assert!(next.try_recv().is_err_and(|e| e == oneshot::error::TryRecvError::Closed));
        assert!(other.try_recv().is_err_and(|e| e == oneshot::error::TryRecvError::Empty));
    }
}
