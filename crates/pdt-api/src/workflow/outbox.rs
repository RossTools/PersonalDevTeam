//! Delivers `session.send` messages (Phase 7a spec §5.2) with the rule PR notices always had: a terminal must be idle
//! with nobody typing for 20 s; a conversation idle with no open request, no queued message, no rate limit and its
//! terminal closed (a parked one reopens to take it). All of a session's waiting messages go in one message. A
//! message whose attempt has no running agent session is recorded as not told; what the user sent to a conversation
//! (an Inbox answer, a nudge) reopens it even when it is stopped or interrupted, and so does a message that `reopens`
//! (an agent step's next pass, the user's ticket comment: PDT-144), which may also reopen its attempt's newest
//! conversation that was not stopped. Delivering resumes the step. PDT's own
//! messages wait while the host's cap on agents running a turn is reached (Phase 7c §6.2); the user's never do.
//!
//! Honest delivery (PDT-129): a message typed into a terminal is told once the keys went in and the terminal is still
//! there; one handed to a conversation is `sending` until the agent echoes it, and every other conversation message
//! (the user's own, review feedback) gets a `sending` row too. A message that is lost on the way (the agent stops,
//! pdtd restarts, the agent never takes it) or that waits [`DEADLINE`] on a session that is neither idle nor working
//! is resolved untold, and the user gets a `session.undelivered` notification naming the session and the text.
//! Messages for a session a restart is about to resume wait for it.

use crate::routes::{AppState, now_ms};
use pdt_core::activity::Activity;
use pdt_core::conversation::SessionMode;
use pdt_core::notification::{NewNotification, NotificationKind, NotificationTarget, Severity};
use pdt_core::session::{Session, SessionKind, SessionStatus};
use pdt_core::ticket::{Actor, EventKind};
use pdt_core::workflow::agent::AgentStepState;
use pdt_core::workflow::api::StepStatus;
use pdt_store::SessionMessage;
use serde_json::{Value, json};
use std::collections::HashMap;

/// How long messages may wait on a session that cannot take them though it is not working (`AppState::outbox_deadline`).
pub(crate) const DEADLINE: std::time::Duration = std::time::Duration::from_secs(30 * 60);
/// How much of a message a notification quotes.
const QUOTE_CHARS: usize = 300;

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
    // What the user sent to a conversation that is not running (parked, stopped, or interrupted by a pdtd restart)
    // reopens it, as typing into it would (PDT-95), and so does a workflow message that reopens (an agent step's next
    // pass, the user's ticket comment: PDT-144). PDT's other messages still go only to running sessions.
    let mut stopped: Vec<Session> = vec![];
    // An attempt with no running agent session: its newest conversation that was not stopped, for messages that reopen.
    let mut reopen_by_attempt: HashMap<String, String> = HashMap::new();
    for m in waiting.iter().filter(|m| m.by_user || m.reopens) {
        if let Some(sid) = m.session_id.as_deref()
            && !running.iter().any(|s| s.id == sid)
            && !stopped.iter().any(|s| s.id == sid)
            && let Ok(Some(s)) = st.store.get_session(sid).await
            && s.kind == SessionKind::AgentTui
            && s.mode == SessionMode::Conversation
        {
            stopped.push(s);
        }
        if let (None, Some(a)) = (&m.session_id, &m.attempt_id)
            && !by_attempt.contains_key(a.as_str())
            && !reopen_by_attempt.contains_key(a)
            && let Ok(sessions) = st.store.list_attempt_sessions(a).await
            && let Some(s) = sessions.into_iter().rev().find(|s| {
                s.kind == SessionKind::AgentTui
                    && s.mode == SessionMode::Conversation
                    && s.status != SessionStatus::Finished
                    && !running.iter().any(|r| r.id == s.id)
            })
        {
            reopen_by_attempt.insert(a.clone(), s.id.clone());
            if !stopped.iter().any(|x| x.id == s.id) {
                stopped.push(s);
            }
        }
    }
    // Sessions the previous pdtd left that are about to resume: their messages wait for them (PDT-129).
    let resuming: Vec<Session> = if st.pending_resume.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
        vec![]
    } else {
        let interrupted = st.store.list_sessions_by_status(SessionStatus::Interrupted).await.unwrap_or_default();
        interrupted.into_iter().filter(|s| st.resume_pending(&s.id)).collect()
    };
    let by_id: HashMap<&str, &Session> =
        running.iter().filter(live).chain(&stopped).map(|s| (s.id.as_str(), s)).collect();
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
            (None, Some(a)) => by_attempt.get(a.as_str()).copied().or_else(|| {
                m.reopens.then(|| reopen_by_attempt.get(a).and_then(|id| by_id.get(id.as_str()).copied())).flatten()
            }),
            _ => None,
        };
        let resumes = |s: &Session| match (&m.session_id, &m.attempt_id) {
            (Some(id), _) => s.id == *id,
            (None, Some(a)) => s.attempt_id.as_deref() == Some(a.as_str()),
            _ => false,
        };
        match target {
            Some(s) => batches.entry(s.id.clone()).or_insert_with(|| (s, vec![])).1.push(m),
            None if resuming.iter().any(resumes) => {}
            None => resolve(st, &m, None).await,
        }
    }
    let now = now_ms();
    let deadline = i64::try_from(st.outbox_deadline.as_millis()).unwrap_or(i64::MAX);
    // Sessions that no longer wait on anything are forgotten.
    st.outbox_stuck.lock().unwrap_or_else(|e| e.into_inner()).retain(|sid, _| batches.contains_key(sid));
    // The host's cap on agents running a turn holds PDT's own messages, never what the user sent (Phase 7c §6.2).
    let mut free: Option<usize> = None;
    for (sid, (session, list)) in batches {
        let conversation = session.mode == SessionMode::Conversation;
        let ready = if conversation {
            crate::conversations::ready_for_notice(st, &sid)
        } else {
            crate::activity::ready_for_message(st, &sid, i64::try_from(st.typing_quiet.as_millis()).unwrap_or(i64::MAX))
        };
        if !ready {
            // Working is worth waiting for, however long it takes; anything else only until the deadline.
            let (working, why) = not_ready(st, &sid, conversation);
            let since = {
                let mut stuck = st.outbox_stuck.lock().unwrap_or_else(|e| e.into_inner());
                if working {
                    stuck.remove(&sid);
                    continue;
                }
                *stuck.entry(sid.clone()).or_insert(now)
            };
            if now - since >= deadline {
                st.outbox_stuck.lock().unwrap_or_else(|e| e.into_inner()).remove(&sid);
                tracing::warn!(session = %sid, messages = list.len(), why, "giving up on messages the session never took");
                let mins = st.outbox_deadline.as_secs().div_ceil(60);
                let why = format!("{why} for {mins} min, so PDT stopped waiting to tell it.");
                for m in &list {
                    resolve(st, m, None).await;
                }
                report(st, &sid, &list, &why).await;
            }
            continue;
        }
        st.outbox_stuck.lock().unwrap_or_else(|e| e.into_inner()).remove(&sid);
        // A conversation stopped since it was listed (its step's run was cancelled, PDT-203) takes only what reopens
        // it: looked at under its lock, so a stop can't land between the look and the send. One that is busy (a stop,
        // a restart) waits for a later tick, so the engine never waits on it.
        let _session = if conversation {
            let Some(g) = st.try_lock_session(&sid) else { continue };
            Some(g)
        } else {
            None
        };
        if conversation
            && !list.iter().any(|m| m.by_user || m.reopens)
            && !matches!(st.store.get_session(&sid).await, Ok(Some(s)) if s.status == SessionStatus::Running)
        {
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
        let text = batch_text(st, &sid, &list).await;
        if conversation {
            // Told when the agent echoes it (`echoed`); until then the messages are on their way.
            let pending_id = uuid::Uuid::now_v7().to_string();
            let ids: Vec<String> = list.iter().map(|m| m.id.clone()).collect();
            if let Err(e) = st.store.send_session_messages(&ids, &sid, &pending_id).await {
                tracing::warn!(session = %sid, error = %e, "could not mark messages on their way");
                continue;
            }
            // PDT's own messages to a PM conversation show collapsed (PDT-211): a batch of nothing else is marked.
            let pdt = list
                .iter()
                .map(|m| m.dedupe.as_deref().and_then(pdt_core::pm::pdt_message))
                .collect::<Option<Vec<_>>>()
                .and_then(|kinds| kinds.first().copied());
            match crate::conversations::send_notice(st, session, &text, &pending_id, pdt).await {
                Ok(()) => tracing::info!(session = %sid, messages = list.len(), "gave the agent its messages"),
                Err(e) => {
                    tracing::warn!(session = %sid, error = %e.message(), "could not send messages to the conversation");
                    unsend(st, &sid, &pending_id).await;
                }
            }
            continue;
        }
        match crate::activity::type_message(st, &sid, &text).await {
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

/// Puts a batch that never went in back to waiting, for the next pass. Tried twice (a busy database); failing that,
/// the batch would stay marked on its way with nothing to tell the agent, so it is given up and the user told.
async fn unsend(st: &AppState, sid: &str, pending_id: &str) {
    let mut last = None;
    for _ in 0..2 {
        match st.store.unsend_session_messages(pending_id).await {
            Ok(_) => return,
            Err(e) => last = Some(e),
        }
    }
    if let Some(e) = last {
        tracing::warn!(session = %sid, error = %e, "could not put unsent messages back in the queue");
    }
    lost(st, &[pending_id.to_string()], Some("PDT could not send it, nor queue it again."));
}

/// One message for a session's batch. Workflows' messages (PR notices) meeting a step that hands in work (a Coder's,
/// `submitted`) are folded into it (PDT-161): after the step's message when that goes in the same batch, else marked
/// as part of the step it is working on, so a notice never displaces the review and QA findings.
async fn batch_text(st: &AppState, sid: &str, list: &[SessionMessage]) -> String {
    let prompt = |m: &&SessionMessage| m.dedupe.as_deref().is_some_and(|d| d.starts_with(super::agent::PROMPT_KEY));
    let workflow = |m: &&SessionMessage| m.step_id.is_some() && !m.by_user;
    let hands_in = |a: &pdt_core::workflow::agent::AgentStep| {
        a.outcomes.iter().any(|o| o.id == pdt_core::workflow::agent::SUBMITTED)
    };
    let step = match list.iter().find(prompt).and_then(|m| m.dedupe.as_deref()) {
        Some(d) => st.store.get_agent_step(&d[super::agent::PROMPT_KEY.len()..]).await.ok().flatten(),
        None => st.store.active_agent_step(sid).await.ok().flatten().filter(|a| a.state == AgentStepState::Working),
    };
    if !step.as_ref().is_some_and(hands_in) {
        return list.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n\n");
    }
    let own: Vec<&str> = list
        .iter()
        .filter(prompt)
        .chain(list.iter().filter(|m| !prompt(m) && !workflow(m)))
        .map(|m| m.text.as_str())
        .collect();
    let notices: Vec<&str> = list.iter().filter(|m| !prompt(m) && workflow(m)).map(|m| m.text.as_str()).collect();
    pdt_core::workflow::agent::fold_messages(&own, &notices, true)
}

/// Whether a session that cannot take messages now is working (worth waiting for), and else why it cannot.
fn not_ready(st: &AppState, sid: &str, conversation: bool) -> (bool, &'static str) {
    if conversation {
        return match crate::conversations::busy_with(st, sid) {
            // A rate limit ends by itself.
            Some(crate::conversations::Busy::Turn | crate::conversations::Busy::RateLimit) => (true, ""),
            Some(crate::conversations::Busy::Request) => (false, "It waited on you"),
            Some(crate::conversations::Busy::Message) => (false, "It had another message it never took"),
            Some(crate::conversations::Busy::Terminal) => (false, "Its terminal was open"),
            None => (false, "It was not ready"),
        };
    }
    match crate::activity::activity_of(st, sid) {
        Some(Activity::Working) => (true, ""),
        Some(Activity::NeedsYou) => (false, "It waited on you"),
        Some(Activity::Unknown) => (false, "PDT could not tell whether it was idle (no activity reports)"),
        // Idle, with someone typing into it.
        Some(Activity::Idle) => (false, "Someone was typing in it"),
        None => (false, "PDT was not watching its terminal"),
    }
}

/// Records a conversation message that is not the outbox's (the user's own, review feedback, a step's first message)
/// as on its way, so losing it is reported (PDT-129).
pub(crate) async fn track(
    st: &AppState,
    session: &Session,
    pending_id: &str,
    text: &str,
    by_user: bool,
) -> Result<(), crate::error::ApiError> {
    let m = SessionMessage {
        id: uuid::Uuid::now_v7().to_string(),
        attempt_id: None,
        session_id: Some(session.id.clone()),
        ticket_id: None,
        dedupe: None,
        text: text.to_string(),
        timeline: Value::Null,
        run_id: None,
        step_id: None,
        status: "sending".into(),
        told_session: Some(session.id.clone()),
        created_at: now_ms(),
        resolved_at: None,
        by_user,
        pending_id: Some(pending_id.to_string()),
        reopens: false,
    };
    st.store.queue_session_message(&m).await?;
    Ok(())
}

/// The agent echoed a conversation's pending message: what it carried was told.
pub(crate) async fn echoed(st: &AppState, pending_id: &str) {
    let rows = match st.store.sending_session_messages(Some(pending_id)).await {
        Ok(v) => v,
        Err(e) => return tracing::warn!(error = %e, "could not read the messages on their way"),
    };
    for m in rows {
        let _locked = super::engine::lock_run(st, m.run_id.as_deref()).await;
        resolve(st, &m, m.told_session.as_deref()).await;
    }
}

/// Conversation messages that will never go in: resolved untold and, given `why`, reported to the user. In the
/// background: callers may hold the engine lock.
pub(crate) fn lost(st: &AppState, pending_ids: &[String], why: Option<&str>) {
    if pending_ids.is_empty() {
        return;
    }
    let (st, ids, why) = (st.clone(), pending_ids.to_vec(), why.map(String::from));
    tokio::spawn(async move {
        for id in ids {
            match st.store.sending_session_messages(Some(&id)).await {
                Ok(rows) => give_up(&st, rows, why.as_deref()).await,
                Err(e) => tracing::warn!(error = %e, "could not read the messages on their way"),
            }
        }
    });
}

/// After a restart (before serving): conversation messages the previous pdtd had not seen echoed never went in.
pub(crate) async fn recover(st: &AppState) {
    let rows = match st.store.sending_session_messages(None).await {
        Ok(v) => v,
        Err(e) => return tracing::warn!(error = %e, "could not read the messages on their way"),
    };
    let mut by_pending: HashMap<Option<String>, Vec<SessionMessage>> = HashMap::new();
    for m in rows {
        by_pending.entry(m.pending_id.clone()).or_default().push(m);
    }
    for (_, rows) in by_pending {
        tracing::warn!(messages = rows.len(), "a restart lost messages on their way to a conversation");
        give_up(st, rows, Some("PDT restarted before the agent took it.")).await;
    }
}

/// Resolves messages untold and reports them (one notification for the lot).
async fn give_up(st: &AppState, rows: Vec<SessionMessage>, why: Option<&str>) {
    for m in &rows {
        let _locked = super::engine::lock_run(st, m.run_id.as_deref()).await;
        resolve(st, m, None).await;
    }
    if let (Some(why), Some(sid)) = (why, rows.first().and_then(|m| m.told_session.clone().or(m.session_id.clone()))) {
        report(st, &sid, &rows, why).await;
    }
}

/// A `session.undelivered` notification: the session, why, and what it was not told.
async fn report(st: &AppState, session_id: &str, rows: &[SessionMessage], why: &str) {
    let Some(first) = rows.first() else { return };
    let session = st.store.get_session(session_id).await.ok().flatten();
    let name = session.as_ref().map_or("an agent session".to_string(), |s| format!("“{}”", s.title));
    let text = rows.iter().map(|m| m.text.trim()).collect::<Vec<_>>().join("\n\n");
    let quote: String = text.chars().take(QUOTE_CHARS).collect();
    let ellipsis = if text.chars().count() > QUOTE_CHARS { "…" } else { "" };
    let target = match &session {
        Some(s) => NotificationTarget::Session { id: s.id.clone(), project_id: s.project_id.clone() },
        None => NotificationTarget::None,
    };
    let mut n = NewNotification::new(
        NotificationKind::SessionUndelivered,
        format!("undelivered:{}", first.id),
        format!("A message did not reach {name}"),
        target,
    );
    n.severity = Severity::Warning;
    n.body = format!("{why}\n\n{quote}{ellipsis}");
    n.session_id = session.as_ref().map(|s| s.id.clone());
    n.project_id = session.as_ref().map(|s| s.project_id.clone());
    n.ticket_id = first.ticket_id.clone();
    n.run_id = first.run_id.clone();
    crate::notifications::notify(st, n).await;
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
    // A workflow's message with outside text in it (PDT-202; an agent step's own prompt is recorded by the step).
    let step_prompt = m.dedupe.as_deref().is_some_and(|d| d.starts_with(super::agent::PROMPT_KEY));
    if let (Some(sid), Some(run_id), false) = (session, &m.run_id, step_prompt)
        && pdt_core::fence::fenced(&m.text)
        && let Ok(Some(run)) = st.store.get_workflow_run(run_id).await
        && let Some((source, url)) = crate::untrusted::run_source(st, &run).await
    {
        let e = pdt_core::workflow::events::OutsideInput {
            project_id: run.project_id.clone(),
            ticket_id: m.ticket_id.clone(),
            session_id: sid.to_string(),
            source,
            url,
            via: "message".into(),
            run_id: Some(run.id.clone()),
            workflow_name: Some(run.workflow_name.clone()),
        };
        crate::untrusted::record_input(st, e).await;
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
