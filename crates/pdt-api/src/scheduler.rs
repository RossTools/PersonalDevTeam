//! Scheduling (Phase 7c spec §6): auto-start of To do / Ready tickets up to a project's cap, the host's cap on agents
//! running a turn (slots for PDT's own work), and who owns a ticket's workflow step now (the Board's cards, §7).

use crate::routes::{AppState, now_ms};
use pdt_core::inbox::InboxKind;
use pdt_core::policy::{AutoStart, pick_next};
use pdt_core::start::{StartMode, StartTicket};
use pdt_core::ticket::{TicketAgent, TicketAgentState, TicketQuery, TicketSummary};
use pdt_core::workflow::agent::AgentStepState;
use pdt_core::workflow::api::{Scope, StepStatus};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// How often the scheduler looks for tickets to start.
const SCHEDULE_EVERY: Duration = Duration::from_secs(10);

/// The agent sessions running a turn now: conversations with a turn or a message on its way, and agent terminals
/// whose activity is `working`. Idle ones don't count.
pub(crate) fn busy_sessions(st: &AppState) -> HashSet<String> {
    let mut out: HashSet<String> = crate::conversations::busy_ids(st).into_iter().collect();
    out.extend(crate::activity::working_ids(st));
    out
}

/// How many more agents may start a turn for PDT's own work (`usize::MAX` with no cap).
pub(crate) async fn free_slots(st: &AppState) -> usize {
    let cap = st.store.get_host_settings().await.map(|h| h.max_agent_turns).unwrap_or(0);
    if cap == 0 {
        return usize::MAX;
    }
    usize::try_from(cap).unwrap_or(usize::MAX).saturating_sub(busy_sessions(st).len())
}

/// Whether PDT may start another agent turn of its own now.
pub(crate) async fn slot_free(st: &AppState) -> bool {
    free_slots(st).await > 0
}

/// Part of the engine tick: agent steps waiting for a slot run again, oldest first, as many as there are free slots.
pub(crate) async fn admit_waiting(st: &AppState) {
    let Ok(waiting) = st.store.steps_waiting_for_slot().await else { return };
    if waiting.is_empty() {
        return;
    }
    let free = free_slots(st).await;
    for (_, mut step) in waiting.into_iter().take(free) {
        if step.status != StepStatus::Waiting {
            continue;
        }
        let loop_allowed = step.wait.get("loopAllowed").cloned().unwrap_or(Value::Bool(false));
        step.status = StepStatus::Pending;
        step.wait = json!({"admitted": true, "loopAllowed": loop_allowed});
        step.wake_at = None;
        if let Err(e) = st.store.put_step(&step).await {
            tracing::warn!(step = %step.id, error = %e, "could not admit a step waiting for a slot");
            continue;
        }
        let _ = st.store.wake_workflow_run(&step.run_id, now_ms()).await;
        tracing::info!(step = %step.id, "a step waiting for a slot got one");
    }
}

/// Whether an enabled workflow for the project has a trigger on `event_type` (e.g. `attempt.started`: the pipeline's
/// Coder step starts its own agent when a ticket starts).
pub(crate) async fn workflow_listens(st: &AppState, project_id: &str, event_type: &str) -> bool {
    for wf in st.store.enabled_workflows().await.unwrap_or_default() {
        let applies = match wf.scope {
            Scope::Project => wf.project_id.as_deref() == Some(project_id),
            Scope::Host => true,
            Scope::Personal => false,
        };
        if !applies {
            continue;
        }
        if let Ok(Some(doc)) = st.store.workflow_doc(&wf.id, wf.version).await
            && doc.nodes.iter().any(|n| n.kind == event_type)
        {
            return true;
        }
    }
    false
}

/// One pass of auto-start: in each project that asks for it, the next candidate while it is under its cap. A ticket
/// that fails to start is skipped until it changes.
pub async fn schedule_tickets(st: &AppState) {
    let Ok(projects) = st.store.list_projects().await else { return };
    for p in projects {
        let Ok(settings) = st.store.get_project_settings(&p.id).await else { continue };
        let policy = settings.policy;
        if policy.auto_start == AutoStart::Off {
            continue;
        }
        let Ok(active) = st.store.count_active_attempts(&p.id).await else { continue };
        if active >= policy.auto_start_cap() {
            continue;
        }
        let q = TicketQuery {
            project: Some(p.id.clone()),
            stage: Some("todo,ready".into()),
            has_parts: Some(false),
            limit: Some(200),
            ..Default::default()
        };
        let Ok(filter) = q.validate() else { continue };
        let Ok(idx) = crate::overlap::index(st, Some(&p.id)).await else { continue };
        let Ok(mut page) = st.store.query_tickets(&filter).await else { continue };
        for t in page.items.iter_mut() {
            idx.apply_summary(t);
        }
        let skip: Vec<String> = {
            let failed = st.auto_start_failed.lock().unwrap_or_else(|e| e.into_inner());
            page.items
                .iter()
                .filter(|t| failed.get(&t.id).is_some_and(|at| *at == t.updated_at))
                .map(|t| t.id.clone())
                .collect()
        };
        let Some(next) = pick_next(&policy, active, &page.items, &skip) else { continue };
        let (id, updated_at, key) = (next.id.clone(), next.updated_at, next.key.clone());
        let no_session = workflow_listens(st, &p.id, pdt_core::workflow::events::ATTEMPT_STARTED).await;
        // A start with its own session runs a turn at once: it waits for a slot like an agent step.
        if !no_session && !slot_free(st).await {
            continue;
        }
        let Ok(host) = st.store.get_host_settings().await else { continue };
        let input = StartTicket {
            agent: host.default_agent,
            branch: None,
            base: None,
            bypass_permissions: None,
            permission_mode: Some(host.ticket_permission_mode),
            tab: None,
            new_workspace: Some(true),
            mode: Some(StartMode::Check),
            session_mode: Some(host.default_session_mode),
            from_session: None,
            no_session: Some(no_session),
            next: None,
            cols: None,
            rows: None,
        };
        let started = match input.validate() {
            Ok(v) => crate::start::start_ticket(st, id.clone(), v, true).await.map_err(|e| e.message().to_string()),
            Err(e) => Err(e.to_string()),
        };
        match started {
            Ok(_) => tracing::info!(ticket = %key, project = %p.name, "auto-start started a ticket"),
            Err(e) => {
                tracing::warn!(ticket = %key, error = %e, "auto-start could not start a ticket; skipping it until it changes");
                st.auto_start_failed.lock().unwrap_or_else(|e| e.into_inner()).insert(id, updated_at);
            }
        }
    }
}

/// pdtd's scheduler loop.
pub async fn run_scheduler(st: AppState) {
    loop {
        tokio::time::sleep(SCHEDULE_EVERY).await;
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        schedule_tickets(&st).await;
    }
}

/// Who owns each ticket's current workflow step, by ticket id (Phase 7c spec §7): a persona's session at work, a
/// question or stall waiting on the user, a manual step or gate (theirs), or a step waiting for a slot.
pub(crate) async fn ticket_agents(st: &AppState) -> HashMap<String, TicketAgent> {
    let rank = |s: TicketAgentState| match s {
        TicketAgentState::Asking => 0,
        TicketAgentState::Stalled => 1,
        TicketAgentState::Yours => 2,
        TicketAgentState::Working => 3,
        TicketAgentState::Queued => 4,
    };
    let mut out: HashMap<String, TicketAgent> = HashMap::new();
    let mut put = |ticket: String, a: TicketAgent| {
        if out.get(&ticket).is_none_or(|old| rank(a.state) < rank(old.state)) {
            out.insert(ticket, a);
        }
    };
    let busy = busy_sessions(st);
    for a in st.store.active_agent_steps().await.unwrap_or_default() {
        let Some(ticket) = a.ticket_id.clone() else { continue };
        let state = match a.state {
            AgentStepState::Working => TicketAgentState::Working,
            AgentStepState::Asking => TicketAgentState::Asking,
            AgentStepState::Stalled => TicketAgentState::Stalled,
            AgentStepState::Manual => TicketAgentState::Yours,
            AgentStepState::Done => continue,
        };
        let session_busy = a.session_id.as_ref().is_some_and(|s| busy.contains(s));
        put(
            ticket,
            TicketAgent {
                persona: Some(a.persona_name.clone()),
                state,
                session_id: a.session_id.clone(),
                busy: session_busy,
                run_id: Some(a.run_id.clone()),
            },
        );
    }
    for item in st.store.open_inbox_items(None).await.unwrap_or_default() {
        let Some(ticket) = item.ticket_id.clone() else { continue };
        if item.kind == InboxKind::Gate {
            put(
                ticket,
                TicketAgent {
                    persona: None,
                    state: TicketAgentState::Yours,
                    session_id: None,
                    busy: false,
                    run_id: item.run_id.clone(),
                },
            );
        }
    }
    for (ticket, step) in st.store.steps_waiting_for_slot().await.unwrap_or_default() {
        let Some(ticket) = ticket else { continue };
        let persona = step.wait.get("persona").and_then(Value::as_str).map(String::from);
        put(
            ticket,
            TicketAgent {
                persona,
                state: TicketAgentState::Queued,
                session_id: None,
                busy: false,
                run_id: Some(step.run_id.clone()),
            },
        );
    }
    out
}

/// Fills in `agent` on tickets of a list.
pub(crate) async fn apply_agents(st: &AppState, items: &mut [TicketSummary]) {
    if items.is_empty() {
        return;
    }
    let agents = ticket_agents(st).await;
    for t in items {
        t.agent = agents.get(&t.id).cloned();
    }
}
