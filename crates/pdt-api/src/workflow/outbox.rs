//! Delivers `session.send` messages (Phase 7a spec §5.2) with the rule PR notices always had: a terminal must be idle
//! with nobody typing for 20 s; a conversation idle with no open request, no queued message, no rate limit and its
//! terminal closed (a parked one reopens to take it). All of a session's waiting messages go in one message. A
//! message whose attempt has no running agent session is recorded as not told. Delivering resumes the step. PDT's own
//! messages wait while the host's cap on agents running a turn is reached (Phase 7c §6.2); the user's never do.

use crate::routes::{AppState, now_ms};
use pdt_core::conversation::SessionMode;
use pdt_core::session::{Session, SessionKind, SessionStatus};
use pdt_core::ticket::{Actor, EventKind};
use pdt_core::workflow::api::StepStatus;
use pdt_store::SessionMessage;
use serde_json::{Value, json};
use std::collections::HashMap;

/// How long nobody may have typed into a terminal before a message is typed into it.
const QUIET_MS: i64 = 20_000;

pub(crate) async fn deliver(st: &AppState) {
    let waiting = match st.store.waiting_session_messages().await {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => return,
        Err(e) => return tracing::warn!(error = %e, "could not read the session messages waiting"),
    };
    let running = match st.store.list_sessions_by_status(SessionStatus::Running).await {
        Ok(v) => v,
        Err(e) => return tracing::warn!(error = %e, "could not list sessions to deliver messages"),
    };
    // The newest running agent session of each attempt (the list is oldest first): a terminal with its PTY, or a
    // conversation.
    let live = |s: &&Session| {
        s.kind == SessionKind::AgentTui && (s.mode == SessionMode::Conversation || st.ptys.is_running(&s.id))
    };
    let mut by_attempt: HashMap<&str, &Session> = HashMap::new();
    for s in running.iter().filter(live) {
        if let Some(a) = s.attempt_id.as_deref() {
            by_attempt.insert(a, s);
        }
    }
    let by_id: HashMap<&str, &Session> = running.iter().filter(live).map(|s| (s.id.as_str(), s)).collect();
    let mut batches: HashMap<String, (&Session, Vec<SessionMessage>)> = HashMap::new();
    for m in waiting {
        // A message whose step no longer waits (its run was cancelled) is dropped.
        if let Some(step) = &m.step_id
            && !matches!(st.store.get_step(step).await, Ok(Some(s)) if s.status == StepStatus::Waiting)
        {
            let _ = st.store.resolve_session_message(&m.id, None, now_ms()).await;
            continue;
        }
        let target = match (&m.session_id, &m.attempt_id) {
            (Some(s), _) => by_id.get(s.as_str()).copied(),
            (None, Some(a)) => by_attempt.get(a.as_str()).copied(),
            _ => None,
        };
        match target {
            Some(s) => batches.entry(s.id.clone()).or_insert_with(|| (s, vec![])).1.push(m),
            None => resolve(st, &m, None).await,
        }
    }
    // The host's cap on agents running a turn holds PDT's own messages, never what the user sent (Phase 7c §6.2).
    let mut free: Option<usize> = None;
    for (sid, (session, list)) in batches {
        let conversation = session.mode == SessionMode::Conversation;
        let ready = if conversation {
            crate::conversations::ready_for_notice(st, &sid)
        } else {
            crate::activity::ready_for_message(st, &sid, QUIET_MS)
        };
        if !ready {
            continue;
        }
        if !list.iter().any(|m| m.by_user) {
            let slots = match free {
                Some(n) => n,
                None => crate::scheduler::free_slots(st).await,
            };
            if slots == 0 {
                free = Some(0);
                continue;
            }
            free = Some(slots.saturating_sub(1));
        }
        let text = list.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n\n");
        let sent = if conversation {
            crate::conversations::send_notice(st, session, &text).await.map_err(|e| e.message().to_string())
        } else {
            crate::activity::type_message(st, &sid, &text).await.map_err(|e| e.to_string())
        };
        match sent {
            Ok(()) => {
                tracing::info!(session = %sid, messages = list.len(), "told the agent");
                for m in &list {
                    resolve(st, m, Some(&sid)).await;
                }
            }
            Err(e) => tracing::warn!(session = %sid, error = %e, "could not type a message into the session"),
        }
    }
}

/// Records a message as told to `session` (or to nobody): its timeline record, the outbox row, and its step.
async fn resolve(st: &AppState, m: &SessionMessage, session: Option<&str>) {
    let now = now_ms();
    if let (Some(ticket), Some(kind)) = (&m.ticket_id, m.timeline.get("kind").and_then(|k| k.as_str()))
        && kind == EventKind::Pr.as_str()
    {
        let mut data = m.timeline.get("data").cloned().unwrap_or(json!({}));
        data["sessionId"] = json!(session);
        data["told"] = json!(session.is_some());
        if let Err(e) = st.store.add_ticket_event(ticket, EventKind::Pr, Actor::System, None, Some(data), now).await {
            return tracing::warn!(ticket = %ticket, error = %e, "could not record a message on the timeline");
        }
    }
    match st.store.resolve_session_message(&m.id, session, now).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => return tracing::warn!(error = %e, "could not resolve a session message"),
    }
    if let Some(step_id) = &m.step_id
        && let Ok(Some(step)) = st.store.get_step(step_id).await
        && step.status == StepStatus::Waiting
    {
        let port = if session.is_some() { "sent" } else { "untold" };
        let mut output = json!({"told": session.is_some(), "sessionId": session});
        if session.is_none() {
            output["quiet"] = Value::Bool(true);
        }
        if let Err(e) = super::engine::complete_step(st, &step, port, output).await {
            tracing::warn!(step = %step_id, error = %e, "could not resume the step after delivery");
        }
    }
}
