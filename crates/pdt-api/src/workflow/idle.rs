//! Idle tickets (PDT-510): the safety net for the ways a ticket gets stuck that nothing else notices. A ticket in
//! progress is idle while a workflow run on it is unfinished but nothing goes on: no agent step works, asks, waits for
//! a slot or stalled, no session on it is busy, nothing in the Inbox is about it, and its pull request has no CI
//! running or queued and isn't on auto-merge or in a merge queue. Once it has been idle for the Idle tickets input of
//! "When an agent is stuck" (`idleMinutes`; 0 turns it off), the PR poll logs `ticket.idle` once for that spell, and
//! the built-in puts it in the Inbox as stalled (`ticket.stalled`).

use super::builtins::{STUCK_AGENTS, stuck_agents};
use crate::routes::{AppState, now_ms};
use pdt_core::pr::{CiState, PrState};
use pdt_core::session::SessionStatus;
use pdt_core::ticket::TicketStage;
use pdt_core::workflow::agent::AgentStepState;
use pdt_core::workflow::api::NewLogEvent;
use pdt_core::workflow::events::{TICKET_IDLE, TicketIdle};
use pdt_store::{AttemptInfo, RunFilter};
use std::collections::{HashMap, HashSet};

/// What goes on now across tickets, read once a pass.
pub(crate) struct Busy {
    /// Tickets with an agent step that isn't done, or one waiting for a slot.
    steps: HashSet<String>,
    /// Open Inbox items: their ticket and step.
    items: Vec<(String, Option<String>)>,
    /// Attempts with a busy session.
    sessions: HashSet<String>,
}

impl Busy {
    pub(crate) async fn read(st: &AppState) -> Busy {
        let mut steps = HashSet::new();
        for a in st.store.active_agent_steps().await.unwrap_or_default() {
            if a.state != AgentStepState::Done
                && let Some(t) = a.ticket_id
            {
                steps.insert(t);
            }
        }
        for (t, _) in st.store.steps_waiting_for_slot().await.unwrap_or_default() {
            steps.extend(t);
        }
        let items = st.store.open_inbox_items(None).await.unwrap_or_default();
        let items = items.into_iter().filter_map(|i| Some((i.ticket_id?, i.step_id))).collect();
        let busy = crate::scheduler::busy_sessions(st);
        let running = st.store.list_sessions_by_status(SessionStatus::Running).await.unwrap_or_default();
        let sessions = running.into_iter().filter(|s| busy.contains(&s.id)).filter_map(|s| s.attempt_id).collect();
        Busy { steps, items, sessions }
    }
}

/// Whether the attempt's ticket is idle now. `own_step`: the step whose Inbox item says so (it doesn't count).
pub(crate) async fn idle(st: &AppState, info: &AttemptInfo, busy: &Busy, own_step: Option<&str>) -> bool {
    let ticket = &info.ticket_id;
    if busy.steps.contains(ticket)
        || busy.sessions.contains(&info.attempt.id)
        || busy.items.iter().any(|(t, step)| t == ticket && (own_step.is_none() || step.as_deref() != own_step))
    {
        return false;
    }
    if let Some(pr) = crate::github::watched_pr(st, &info.attempt.id) {
        let ci = pr.ci.as_ref().map(|c| c.state);
        if pr.state != PrState::Open
            || pr.auto_merge
            || pr.in_merge_queue
            || matches!(ci, Some(CiState::Running | CiState::Queued))
        {
            return false;
        }
    }
    match st.store.get_ticket(ticket).await {
        Ok(Some(t)) if t.summary.stage == TicketStage::InProgress && !t.summary.waiting => {}
        _ => return false,
    }
    let f = RunFilter { workflow_id: None, ticket_id: Some(ticket.clone()), status: None, limit: 50 };
    st.store.list_workflow_runs(&f).await.unwrap_or_default().iter().any(|r| !r.status.is_finished())
}

/// The Idle tickets input of the enabled "When an agent is stuck": None when it is off, or 0.
async fn idle_minutes(st: &AppState) -> Option<u64> {
    let w = st.store.workflow_by_builtin(STUCK_AGENTS).await.ok().flatten().filter(|w| w.enabled)?;
    let doc = st.store.workflow_doc(&w.id, w.version).await.ok().flatten()?;
    let values = stuck_agents::SETTINGS.values_of(&doc);
    values.get("idleMinutes").and_then(serde_json::Value::as_u64).filter(|m| *m > 0)
}

/// Looks at every active attempt now ([`check_at`]).
pub async fn check(st: &AppState) {
    check_at(st, now_ms()).await
}

/// Looks at every active attempt as of `now`: logs `ticket.idle` for each ticket idle for the set minutes (once per
/// idle spell), and forgets the spells that ended.
pub async fn check_at(st: &AppState, now: i64) {
    let Some(minutes) = idle_minutes(st).await else {
        st.sensors.idle.lock().unwrap_or_else(|e| e.into_inner()).clear();
        return;
    };
    let attempts = match st.store.list_active_attempts().await {
        Ok(v) => v.into_iter().filter(|i| !i.attempt.branch.is_empty()).collect::<Vec<_>>(),
        Err(e) => {
            tracing::warn!(error = %e, "the idle check could not list active attempts");
            return;
        }
    };
    let busy = Busy::read(st).await;
    let mut idle_now: HashMap<String, &AttemptInfo> = HashMap::new();
    for info in &attempts {
        if idle(st, info, &busy, None).await {
            idle_now.insert(info.ticket_id.clone(), info);
        }
    }
    let due: Vec<(&AttemptInfo, i64)> = {
        let mut spells = st.sensors.idle.lock().unwrap_or_else(|e| e.into_inner());
        spells.retain(|t, _| idle_now.contains_key(t));
        let after = i64::try_from(minutes).unwrap_or(i64::MAX / 60_000) * 60_000;
        idle_now
            .values()
            .filter_map(|info| {
                let since = *spells.entry(info.ticket_id.clone()).or_insert(now);
                (now - since >= after).then_some((*info, since))
            })
            .collect()
    };
    for (info, since) in due {
        let pr = crate::github::watched_pr(st, &info.attempt.id);
        let payload = TicketIdle {
            project_id: info.project_id.clone(),
            ticket_id: info.ticket_id.clone(),
            ticket_key: info.ticket_key.clone(),
            attempt_id: info.attempt.id.clone(),
            branch: info.attempt.branch.clone(),
            pr: pr.as_ref().map(|p| p.number),
            url: pr.map(|p| p.url),
            since,
            idle_minutes: u64::try_from((now - since) / 60_000).unwrap_or(0),
        };
        let key = format!("{}:{since}", info.ticket_id);
        if let Err(e) = st.store.emit_event(&NewLogEvent::new(TICKET_IDLE, key, "idle_check", payload), now).await {
            tracing::warn!(ticket = %info.ticket_key, error = %e, "could not log an idle ticket");
        }
    }
}
