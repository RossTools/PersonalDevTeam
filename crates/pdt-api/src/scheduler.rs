//! Scheduling (Phase 7c spec §6): the pickup watch that feeds auto-start's workflow ("Pick up tickets", PDT-327), the
//! host's cap on agents running a turn (slots for PDT's own work), and who owns a ticket's workflow step now (the
//! Board's cards, §7).

use crate::routes::AppState;
use pdt_core::inbox::InboxKind;
use pdt_core::policy::{AutoStart, auto_start_at, settles_at};
use pdt_core::ticket::{TicketAgent, TicketAgentState, TicketQuery, TicketSummary};
use pdt_core::workflow::agent::AgentStepState;
use pdt_core::workflow::api::NewLogEvent;
use pdt_core::workflow::api::{Scope, StepStatus};
use pdt_core::workflow::events::{SLOT_FREED, SlotFreed, StartableTicket, TICKET_STARTABLE, TicketStartable};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// How often the pickup watch looks for tickets that can start and room to start them.
const WATCH_EVERY: Duration = Duration::from_secs(10);

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
        let keep = |k: &str| step.wait.get(k).cloned().unwrap_or(Value::Null);
        let (loop_allowed, resume_of, note) = (keep("loopAllowed"), keep("resumeOf"), keep("note"));
        step.status = StepStatus::Pending;
        step.wait = json!({"admitted": true, "loopAllowed": loop_allowed.as_bool() == Some(true),
                           "resumeOf": resume_of, "note": note});
        step.wake_at = None;
        // The step and its run's wake are one write (PDT-131): an admitted step is never left in a run that sleeps.
        if let Err(e) = st.store.put_step_and_wake(&step).await {
            tracing::warn!(step = %step.id, error = %e, "could not admit a step waiting for a slot");
            continue;
        }
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

/// What the pickup watch saw on its last pass.
#[derive(Default)]
pub(crate) struct PickupWatch {
    /// Startable tickets told as `ticket.startable`, with their `updated_at` then.
    startable: HashMap<String, i64>,
    /// Tickets in flight per project, and its auto-start cap.
    active: HashMap<String, (u32, u32)>,
    /// Free agent slots on the host (None before the first pass).
    free: Option<usize>,
}

/// How long a start may take between its attempt and its announcement before the watch calls it half-done.
const HALF_DONE_AFTER_MS: i64 = 2 * 60 * 1000;

/// One pass of the pickup watch (PDT-327), which feeds the built-in "Pick up tickets": starts left half-done are
/// announced; in each project with auto-start on, `ticket.startable` for each ticket that became one auto-start could
/// take (its start delay over, its blocker or conflict gone) and `slot.freed` when fewer tickets are in flight or its cap
/// rose; and
/// `slot.freed` when an agent slot freed on the host. Nothing is told while auto-start waits for disk space.
pub async fn watch_pickup(st: &AppState) {
    repair_half_done_starts(st).await;
    // A disk almost full: nothing new starts (PDT-324), so nothing is told; once there is space, every ticket that can
    // start is.
    if crate::storage::auto_start_paused(st).await {
        return;
    }
    let now = crate::routes::now_ms();
    let Ok(projects) = st.store.list_projects().await else { return };
    let mut startable: HashMap<String, i64> = HashMap::new();
    let mut active: HashMap<String, (u32, u32)> = HashMap::new();
    let mut events: Vec<NewLogEvent> = vec![];
    let (seen, was_active) = {
        let w = st.pickup_watch.lock().unwrap_or_else(|e| e.into_inner());
        (w.startable.clone(), w.active.clone())
    };
    for p in projects {
        let Ok(settings) = st.store.get_project_settings(&p.id).await else { continue };
        let policy = settings.policy;
        if policy.auto_start == AutoStart::Off {
            continue;
        }
        let Ok(n) = st.store.count_active_attempts(&p.id).await else { continue };
        let cap = policy.auto_start_cap();
        active.insert(p.id.clone(), (n, cap));
        let freed = match was_active.get(&p.id) {
            Some((before, _)) if n < *before => Some("attempt"),
            Some((_, was_cap)) if cap > *was_cap => Some("cap"),
            _ => None,
        };
        if let Some(reason) = freed {
            let payload = SlotFreed { project_id: Some(p.id.clone()), reason: reason.into() };
            let mut e = NewLogEvent::new(SLOT_FREED, format!("{reason}:{}:{now}", p.id), "pickup", payload);
            e.project_id = Some(p.id.clone());
            events.push(e);
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
            if !(pdt_core::policy::startable(t) && settles_at(&policy, t) <= now) {
                continue;
            }
            startable.insert(t.id.clone(), t.updated_at);
            if seen.get(&t.id) == Some(&t.updated_at) {
                continue;
            }
            let ticket = StartableTicket {
                id: t.id.clone(),
                key: t.key.clone(),
                title: t.title.clone(),
                stage: t.stage.as_str().into(),
                priority: t.priority.as_str().into(),
                labels: t.labels.clone(),
            };
            let payload = TicketStartable { project_id: p.id.clone(), ticket };
            let mut e =
                NewLogEvent::new(TICKET_STARTABLE, format!("{}:{}:{now}", t.id, t.updated_at), "pickup", payload);
            e.project_id = Some(p.id.clone());
            events.push(e);
        }
    }
    let free = free_slots(st).await;
    let was_free = st.pickup_watch.lock().unwrap_or_else(|e| e.into_inner()).free;
    if was_free == Some(0) && free > 0 {
        let payload = SlotFreed { project_id: None, reason: "agent".into() };
        events.push(NewLogEvent::new(SLOT_FREED, format!("agent:{now}"), "pickup", payload));
    }
    for e in &events {
        if let Err(x) = st.store.emit_event(e, now).await {
            // Logged once writes work again (PDT-323); the watch moves on.
            tracing::warn!(event = %e.kind, error = %x, "could not log a pickup event");
        }
    }
    let mut w = st.pickup_watch.lock().unwrap_or_else(|e| e.into_inner());
    (w.startable, w.active, w.free) = (startable, active, Some(free));
}

/// Announces starts left half-done (PDT-327): an attempt and worktree made, but its `attempt.started` never logged
/// (pdtd stopped, or the write failed, before Start announced it), so no workflow took it over. Only attempts at least
/// [`HALF_DONE_AFTER_MS`] old (a Start still making its session announces it itself) and younger than the event log
/// keeps events.
pub(crate) async fn repair_half_done_starts(st: &AppState) {
    let now = crate::routes::now_ms();
    let since = now - pdt_store::EVENT_RETENTION_MS / 2;
    let Ok(half_done) = st.store.unannounced_starts(since, now - HALF_DONE_AFTER_MS).await else { return };
    for (ticket, events) in half_done {
        match st.store.announce_events(&events, now).await {
            Ok(()) => tracing::warn!(ticket = %ticket, "announced a start that was left half-done"),
            Err(e) => tracing::warn!(ticket = %ticket, error = %e, "could not announce a start left half-done"),
        }
    }
}

/// pdtd's pickup watch loop.
pub async fn run_pickup_watch(st: AppState) {
    crate::loops::run_every(st, "pickup", WATCH_EVERY, crate::loops::Start::AfterOnePeriod, |st| async move {
        watch_pickup(&st).await
    })
    .await
}

/// Who owns each ticket's current workflow step, by ticket id (Phase 7c spec §7): a persona's session at work, a
/// question or stall waiting on the user, a manual step or gate (theirs), or a step waiting for a slot.
pub(crate) async fn ticket_agents(st: &AppState) -> HashMap<String, TicketAgent> {
    let rank = |s: TicketAgentState| match s {
        TicketAgentState::Asking => 0,
        TicketAgentState::Stalled => 1,
        TicketAgentState::Yours => 2,
        TicketAgentState::Working | TicketAgentState::Background => 3,
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
        // Its turn ended while background tasks run (PDT-142): what they are now, else as of the turn end.
        let (state, background) = match a.background_since {
            Some(_) if state == TicketAgentState::Working && !session_busy => {
                let now: Vec<String> = a
                    .session_id
                    .as_deref()
                    .and_then(|sid| crate::conversations::background(st, sid))
                    .map(|b| b.tasks.into_iter().map(|t| t.description).collect())
                    .unwrap_or_default();
                (TicketAgentState::Background, if now.is_empty() { a.background.clone() } else { now })
            }
            _ => (state, vec![]),
        };
        put(
            ticket,
            TicketAgent {
                persona: Some(a.persona_name.clone()),
                state,
                session_id: a.session_id.clone(),
                busy: session_busy,
                run_id: Some(a.run_id.clone()),
                background,
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
                    background: vec![],
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
                background: vec![],
            },
        );
    }
    out
}

/// Fills in `auto_start_at` on tickets of a list (overlaps first: they decide whether a ticket can start), with each
/// project's policy.
pub(crate) async fn apply_auto_start(st: &AppState, items: &mut [TicketSummary]) {
    let now = crate::routes::now_ms();
    // No countdown while auto-start waits for disk space.
    let paused = !items.is_empty() && crate::storage::auto_start_paused(st).await;
    let mut policies: HashMap<String, Option<pdt_core::policy::ProjectPolicy>> = HashMap::new();
    for t in items {
        if !policies.contains_key(&t.project_id) {
            let p = st.store.get_project_settings(&t.project_id).await.ok().map(|s| s.policy);
            policies.insert(t.project_id.clone(), p);
        }
        t.auto_start_at = policies[&t.project_id].as_ref().filter(|_| !paused).and_then(|p| auto_start_at(p, t, now));
    }
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
