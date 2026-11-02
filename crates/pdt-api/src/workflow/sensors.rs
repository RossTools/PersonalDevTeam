//! The Inbox, session and run events (PDT-364): `inbox.created` and `inbox.answered`, `session.needs_you`,
//! `session.turn_ended` and `session.exited`, `run.failed` and `run.finished`. Each is logged where it happens (the
//! Notifier's pass for new Inbox items, the answer routes, activity changes, a terminal's exit, a run's end); none of
//! this ever fails what reports it: problems are logged.

use crate::routes::{AppState, now_ms};
use pdt_core::activity::Activity;
use pdt_core::inbox::{InboxItem, InboxKind};
use pdt_core::session::{Session, SessionKind};
use pdt_core::workflow::api::{NewLogEvent, Run, RunStatus, StepStatus};
use pdt_core::workflow::events::{
    self as ev, InboxAnswered, InboxCreated, InboxRef, MAX_RUN_CHAIN, RunEnded, SessionEvent, SessionExited,
};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The longest `lastMessage`, in characters.
const LAST_MESSAGE_CHARS: usize = 400;
/// How much of a terminal's output is read for its `lastMessage`.
const TERMINAL_TAIL_BYTES: usize = 8 * 1024;
/// Items already open this long before pdtd's first look are taken as told by the pdtd before (or by a version
/// without these events): only newer ones are logged.
const STARTUP_GRACE_MS: i64 = 60_000;

/// What the sensors remember between passes.
#[derive(Default)]
pub(crate) struct Sensors {
    inbox: Mutex<InboxSeen>,
    /// Tells apart two turn ends of one session in the same millisecond.
    turns: AtomicU64,
}

#[derive(Default)]
struct InboxSeen {
    /// When the first pass looked.
    since: Option<i64>,
    /// The open items already logged (or left alone at startup), by id.
    ids: HashSet<String>,
}

async fn emit(st: &AppState, e: NewLogEvent) {
    if let Err(x) = st.store.emit_event(&e, now_ms()).await {
        tracing::warn!(kind = %e.kind, key = %e.key, error = %x, "could not log an event");
    }
}

/// The workflow a run belongs to.
async fn workflow_of(st: &AppState, run_id: Option<&str>) -> Option<String> {
    let run = st.store.get_workflow_run(run_id?).await.ok().flatten()?;
    Some(run.workflow_id)
}

async fn inbox_ref(st: &AppState, i: &InboxItem, now: i64) -> InboxRef {
    InboxRef {
        item_id: i.id.clone(),
        kind: i.kind.as_str().into(),
        group: i.group.as_str().into(),
        title: i.title.clone(),
        project_id: i.project_id.clone(),
        ticket_id: i.ticket_id.clone(),
        ticket_key: i.ticket_key.clone(),
        session_id: i.session_id.clone(),
        persona: i.persona_name.clone(),
        run_id: i.run_id.clone(),
        workflow_id: workflow_of(st, i.run_id.as_deref()).await,
        created_at: i.created_at,
        age_ms: (now - i.created_at).max(0),
    }
}

/// The Notifier's pass saw these open Inbox items (stored and listed): logs `inbox.created` for each one not seen
/// before. An item is keyed by its id and when it opened, so one that goes and comes back (low disk space) is new
/// again, and a restart logs nothing twice.
pub(crate) async fn inbox_open(st: &AppState, items: &[InboxItem]) {
    let now = now_ms();
    let fresh: Vec<InboxItem> = {
        let mut seen = st.sensors.inbox.lock().unwrap_or_else(|e| e.into_inner());
        let since = *seen.since.get_or_insert(now);
        let open: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        seen.ids.retain(|id| open.contains(id.as_str()));
        let mut fresh = vec![];
        for i in items {
            // A session waiting is `session.needs_you`.
            if i.kind == InboxKind::Session || !seen.ids.insert(i.id.clone()) {
                continue;
            }
            if i.created_at >= since - STARTUP_GRACE_MS {
                fresh.push(i.clone());
            }
        }
        fresh
    };
    if fresh.is_empty() {
        return;
    }
    let mut fresh = fresh;
    if let Err(e) = crate::inbox::decorate(st, &mut fresh).await {
        tracing::debug!(error = ?e, "could not fill in new Inbox items");
    }
    for i in &fresh {
        let item = inbox_ref(st, i, now).await;
        let key = format!("{}:{}", i.id, i.created_at);
        emit(st, NewLogEvent::new(ev::INBOX_CREATED, key, "inbox", InboxCreated { item })).await;
    }
}

/// Someone answered `item` (`by`: `user` or `agent`): `inbox.answered`, once per item.
pub(crate) async fn inbox_answered(
    st: &AppState,
    item: &InboxItem,
    choice: Option<&str>,
    answer: Option<&str>,
    by: &str,
) {
    let mut items = [item.clone()];
    if let Err(e) = crate::inbox::decorate(st, &mut items).await {
        tracing::debug!(error = ?e, "could not fill in an answered Inbox item");
    }
    let item = inbox_ref(st, &items[0], now_ms()).await;
    let key = format!("{}:answered", item.item_id);
    let payload = InboxAnswered {
        item,
        choice: choice.map(String::from),
        answer: answer.filter(|a| !a.trim().is_empty()).map(String::from),
        by: by.into(),
    };
    emit(st, NewLogEvent::new(ev::INBOX_ANSWERED, key, "inbox", payload)).await;
}

/// A session's payload: its workflow step (if one runs it) and the end of what the agent last said.
async fn session_event(st: &AppState, s: &Session) -> SessionEvent {
    let step = match st.store.active_agent_step(&s.id).await {
        Ok(Some(a)) => Some(a),
        _ => st.store.latest_agent_step(&s.id).await.ok().flatten(),
    };
    let run_id = step.as_ref().map(|a| a.run_id.clone());
    let ticket_id = s.ticket.as_ref().map(|t| t.id.clone()).or_else(|| step.as_ref().and_then(|a| a.ticket_id.clone()));
    let ticket_key = match (&s.ticket, &ticket_id) {
        (Some(t), _) => Some(t.key.clone()),
        (None, Some(id)) => st.store.get_ticket_summary(id).await.ok().flatten().map(|t| t.key),
        _ => None,
    };
    SessionEvent {
        session_id: s.id.clone(),
        title: s.title.clone(),
        project_id: s.project_id.clone(),
        ticket_id,
        ticket_key,
        agent: s.agent.and_then(|a| serde_json::to_value(a).ok()).and_then(|v| v.as_str().map(String::from)),
        mode: s.mode.as_str().into(),
        persona: s.persona.as_ref().map(|p| p.name.clone()).or_else(|| step.as_ref().map(|a| a.persona_name.clone())),
        workflow_step: step.is_some(),
        workflow_id: workflow_of(st, run_id.as_deref()).await,
        run_id,
        last_message: last_message(st, s).await,
    }
}

/// The end of what the agent last said: a conversation's last message, else the end of its terminal's text.
async fn last_message(st: &AppState, s: &Session) -> String {
    if s.mode == pdt_core::conversation::SessionMode::Conversation {
        let events = st.store.recent_session_events(&s.id, 40).await.unwrap_or_default();
        return events
            .iter()
            .rev()
            .find_map(|e| match &e.event {
                pdt_core::conversation::AgentEvent::AgentMessage { markdown, .. } if !markdown.trim().is_empty() => {
                    Some(ev::message_tail(markdown, LAST_MESSAGE_CHARS))
                }
                _ => None,
            })
            .unwrap_or_default();
    }
    let Ok(att) = st.ptys.attach(&s.id) else { return String::new() };
    let bytes = &att.scrollback[att.scrollback.len().saturating_sub(TERMINAL_TAIL_BYTES)..];
    let text = pdt_core::activity::strip_ansi(bytes);
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(8)..].join("\n");
    ev::message_tail(&tail, LAST_MESSAGE_CHARS)
}

/// An agent session's activity went from `prev` to `next`: `session.needs_you` when it now waits on the user,
/// `session.turn_ended` when a turn (or a wait on the user) gave way to idle. Logged off the caller's task.
pub(crate) fn activity_changed(st: &AppState, id: &str, prev: Option<Activity>, next: Activity) {
    let kind = match (prev, next) {
        (Some(p), Activity::NeedsYou) if p != Activity::NeedsYou => ev::SESSION_NEEDS_YOU,
        (Some(Activity::Working | Activity::NeedsYou), Activity::Idle) => ev::SESSION_TURN_ENDED,
        _ => return,
    };
    let n = st.sensors.turns.fetch_add(1, Ordering::Relaxed);
    let key = format!("{id}:{}:{}:{n}", kind.rsplit('.').next().unwrap_or(kind), now_ms());
    let (st, id) = (st.clone(), id.to_string());
    tokio::spawn(async move {
        let Ok(Some(s)) = st.store.get_session(&id).await else { return };
        if s.kind != SessionKind::AgentTui {
            return;
        }
        let payload = session_event(&st, &s).await;
        emit(&st, NewLogEvent::new(kind, key, "sessions", payload)).await;
    });
}

/// An agent session's process ended (`code` when it has one; `stopped`: PDT stopped it). Not while PDT shuts down:
/// every session ends then, and none of them is news.
pub(crate) async fn session_exited(st: &AppState, id: &str, code: Option<i32>, stopped: bool) {
    if st.shutting_down.load(Ordering::SeqCst) {
        return;
    }
    let Ok(Some(s)) = st.store.get_session(id).await else { return };
    if s.kind != SessionKind::AgentTui {
        return;
    }
    let n = st.sensors.turns.fetch_add(1, Ordering::Relaxed);
    let key = format!("{id}:exited:{}:{n}", now_ms());
    let payload = SessionExited { session: session_event(st, &s).await, exit_code: code, stopped_by_pdt: stopped };
    emit(st, NewLogEvent::new(ev::SESSION_EXITED, key, "sessions", payload)).await;
}

/// A run ended: `run.failed`, or `run.finished` for any other end. A run far down a chain of runs started by these
/// events logs neither ([`MAX_RUN_CHAIN`]).
pub(crate) async fn run_ended(st: &AppState, run: &Run) {
    let chain = ev::run_chain(&run.trigger);
    if chain >= MAX_RUN_CHAIN {
        tracing::info!(workflow = %run.workflow_name, run = %run.id, "a chain of runs reporting runs stops here");
        return;
    }
    let steps = st.store.list_steps(&run.id).await.unwrap_or_default();
    let failed = steps.iter().filter(|s| s.status == StepStatus::Failed).max_by_key(|s| (s.ended_at, s.seq));
    let last = steps
        .iter()
        .filter(|s| s.ended_at.is_some() && s.status != StepStatus::Skipped)
        .max_by_key(|s| (s.ended_at, s.seq));
    let ticket_key = match &run.ticket_id {
        Some(t) => st.store.get_ticket_summary(t).await.ok().flatten().map(|t| t.key),
        None => None,
    };
    let ended_at = run.ended_at.unwrap_or_else(now_ms);
    let kind = if run.status == RunStatus::Failed { ev::RUN_FAILED } else { ev::RUN_FINISHED };
    let payload = RunEnded {
        workflow_id: run.workflow_id.clone(),
        workflow_name: run.workflow_name.clone(),
        run_id: run.id.clone(),
        status: run.status.as_str().into(),
        project_id: run.project_id.clone(),
        ticket_id: run.ticket_id.clone(),
        ticket_key,
        error: run.error.clone(),
        failed_node: failed.map(|s| s.node_id.clone()),
        failed_kind: failed.map(|s| s.kind.clone()),
        failed_error: failed.and_then(|s| s.error.clone()),
        end_node: last.map(|s| s.node_id.clone()),
        end_kind: last.map(|s| s.kind.clone()),
        duration_ms: (ended_at - run.started_at).max(0),
        trigger_type: run.trigger.get("type").and_then(|t| t.as_str()).unwrap_or("manual").into(),
        parent_run_id: run.parent_run_id.clone(),
        chain,
    };
    // A retried run can end again: each end is its own event.
    let key = format!("{}:{}:{ended_at}", run.id, run.status.as_str());
    emit(st, NewLogEvent::new(kind, key, "workflows", payload)).await;
}
