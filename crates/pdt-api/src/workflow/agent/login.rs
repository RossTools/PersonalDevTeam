//! Agent steps whose agent isn't logged in (PDT-604): a step whose turn failed for it waits (`waiting_login`) instead
//! of being nudged or stalled, a new one waits before its session starts, and both go on once the agent is logged in
//! again: the first in its session with its task again (a fresh session when that can't take a message), the second
//! by starting over. `crate::agent_login` decides when.

use super::{PDT_REOPENS, R, err, prompt_key, queue_message, session_run, waiting_step};
use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::Outcome;
use crate::workflow::nodes::agent::AgentConfig;
use pdt_core::session::AgentKind;
use pdt_core::workflow::agent::{AgentStep, AgentStepState, StepTicket, first_message};
use pdt_core::workflow::api::{Step, StepStatus};
use serde_json::{Value, json};

/// What a step keeps of its wait over a hold: what a slot's wait keeps, and a restart's fresh session.
const KEPT: [&str; 6] = ["loopAllowed", "resumeOf", "note", "fresh", "restarts", "why"];

/// A new step's wait while its agent isn't logged in: it starts over once the agent is.
pub(crate) fn held(agent: AgentKind, persona: &str, wait: &Value) -> Outcome {
    let mut w = json!({"login": agent.as_str(), "persona": persona});
    for k in KEPT {
        if let Some(v) = wait.get(k) {
            w[k] = v.clone();
        }
    }
    Outcome::Wait { wait: w, until: None, timeout_port: None, output: Value::Null }
}

/// The session's working step waits for its agent's login: its turn failed because the agent isn't logged in.
/// Whether the session has a step that now waits so.
pub(crate) async fn hold(st: &AppState, session_id: &str) -> bool {
    let run = session_run(st, session_id).await;
    let _locked = crate::workflow::engine::lock_run(st, run.as_deref()).await;
    let Ok(Some(mut a)) = st.store.active_agent_step(session_id).await else { return false };
    match a.state {
        AgentStepState::WaitingLogin => return true,
        AgentStepState::Working => {}
        _ => return false,
    }
    let Ok(mut step) = waiting_step(st, &a).await else { return false };
    // Its max runtime stops while it waits: the time waited is added back when it goes on.
    let now = now_ms();
    step.wait["heldAt"] = json!(now);
    step.wait["heldUntil"] = json!(step.wake_at);
    step.wake_at = None;
    if let Err(e) = st.store.put_step(&step).await {
        tracing::warn!(step = %step.id, error = %e, "could not stop a held step's time limit");
    }
    a.state = AgentStepState::WaitingLogin;
    a.background_since = None;
    a.background.clear();
    a.updated_at = now;
    if let Err(e) = st.store.put_agent_step(&a).await {
        tracing::warn!(step = %a.step_id, error = %e, "could not hold an agent step for its agent's login");
        return false;
    }
    // Not stuck: "When an agent is stuck" leaves it alone.
    if let Err(e) = st.store.take_escalation(&a.step_id).await {
        tracing::warn!(step = %a.step_id, error = %e, "could not end a held step's escalation");
    }
    tracing::info!(step = %a.step_id, persona = %a.persona_name, "an agent step waits for its agent's login");
    true
}

/// A step waiting for an agent's login.
pub(crate) struct Waiting {
    pub step_id: String,
    pub persona: String,
    pub ticket_id: Option<String>,
    /// Its turn failed (it has a binding and a session); else it waits to start.
    pub stopped: Option<AgentStep>,
    /// A step that waits to start.
    pub held: Option<Step>,
}

/// The steps waiting for `agent`'s login: those whose turn failed, then those waiting to start, oldest first.
pub(crate) async fn waiting(st: &AppState, agent: AgentKind) -> Vec<Waiting> {
    let mut out = vec![];
    for a in st.store.active_agent_steps().await.unwrap_or_default() {
        if a.state != AgentStepState::WaitingLogin {
            continue;
        }
        let Some(sid) = &a.session_id else { continue };
        let of_agent =
            matches!(st.store.get_session(sid).await, Ok(Some(s)) if s.agent.unwrap_or(AgentKind::Claude) == agent);
        if !of_agent {
            continue;
        }
        // A step that ended meanwhile (its run cancelled) waits for nothing.
        if !matches!(st.store.get_step(&a.step_id).await, Ok(Some(s)) if s.status == StepStatus::Waiting) {
            let mut done = a.clone();
            done.state = AgentStepState::Done;
            if let Err(e) = st.store.put_agent_step(&done).await {
                tracing::warn!(step = %done.step_id, error = %e, "could not write an agent step's binding");
            }
            continue;
        }
        out.push(Waiting {
            step_id: a.step_id.clone(),
            persona: a.persona_name.clone(),
            ticket_id: a.ticket_id.clone(),
            stopped: Some(a),
            held: None,
        });
    }
    for (ticket, step) in st.store.steps_waiting_for_login().await.unwrap_or_default() {
        if step.wait.get("login").and_then(Value::as_str) != Some(agent.as_str()) {
            continue;
        }
        out.push(Waiting {
            step_id: step.id.clone(),
            persona: step.wait.get("persona").and_then(Value::as_str).unwrap_or("An agent").to_string(),
            ticket_id: ticket,
            stopped: None,
            held: Some(step),
        });
    }
    out
}

/// The agents some step waits for.
pub(crate) async fn waiting_agents(st: &AppState) -> std::collections::BTreeSet<AgentKind> {
    let mut out = std::collections::BTreeSet::new();
    for a in st.store.active_agent_steps().await.unwrap_or_default() {
        if a.state == AgentStepState::WaitingLogin
            && let Some(sid) = &a.session_id
            && let Ok(Some(s)) = st.store.get_session(sid).await
        {
            out.insert(s.agent.unwrap_or(AgentKind::Claude));
        }
    }
    for (_, step) in st.store.steps_waiting_for_login().await.unwrap_or_default() {
        out.extend(step.wait.get("login").and_then(Value::as_str).and_then(AgentKind::parse_stored));
    }
    out
}

/// A waiting step goes on now its agent (`label`) is logged in again. Whether it did.
pub(crate) async fn go_on(st: &AppState, w: Waiting, label: &str) -> bool {
    let r = match (w.stopped, w.held) {
        (Some(a), _) => retry(st, a, label).await,
        (None, Some(step)) => wake(st, step).await,
        (None, None) => return false,
    };
    match r {
        Ok(went) => went,
        Err(e) => {
            tracing::warn!(step = %w.step_id, error = %e, "could not take up a step that waited for its agent's login");
            false
        }
    }
}

/// A step whose turn failed: its session gets its task again in a fresh turn, or, when the session can't take a
/// message, the step goes on in a fresh session.
async fn retry(st: &AppState, a: AgentStep, label: &str) -> R<bool> {
    let _locked = crate::workflow::engine::lock_run(st, Some(&a.run_id)).await;
    let Some(mut a) = st.store.get_agent_step(&a.step_id).await.map_err(err)? else { return Ok(false) };
    if a.state != AgentStepState::WaitingLogin {
        return Ok(false);
    }
    let mut step = waiting_step(st, &a).await?;
    resume_time_limit(st, &mut step).await?;
    let session = match &a.session_id {
        Some(sid) => {
            st.store.get_session(sid).await.map_err(err)?.filter(|s| super::super::stuck::takes_messages(st, s))
        }
        None => None,
    };
    let Some(s) = session else {
        super::super::stuck::restart_stalled(st, a, &step, &pdt_core::agent_login::restart_why(label)).await?;
        return Ok(true);
    };
    let c: AgentConfig = serde_json::from_value(step.input.clone()).map_err(err)?;
    let ticket = match &a.ticket_id {
        Some(t) => st.store.get_ticket_summary(t).await.map_err(err)?,
        None => None,
    };
    let ticket = ticket.as_ref().map(|t| StepTicket { key: &t.key, title: &t.title });
    let task = first_message(&c.prompt, ticket, &c.outcomes);
    a.state = AgentStepState::Working;
    a.updated_at = now_ms();
    st.store.put_agent_step(&a).await.map_err(err)?;
    let text = pdt_core::agent_login::retry_message(label, &task);
    // Under the step's own key: the turns before it goes in are not the step's.
    queue_message(st, &s, &text, Some(prompt_key(&a.step_id)), a.ticket_id.clone(), PDT_REOPENS).await?;
    tracing::info!(step = %a.step_id, session = %s.id, "an agent step goes on now its agent is logged in");
    Ok(true)
}

/// A held step's max runtime goes on, moved later by the time it waited.
async fn resume_time_limit(st: &AppState, step: &mut Step) -> R<()> {
    let Some(at) = step.wait.get("heldAt").and_then(Value::as_i64) else { return Ok(()) };
    let until = step.wait.get("heldUntil").and_then(Value::as_i64);
    step.wake_at = until.map(|u| u + (now_ms() - at).max(0));
    if let Value::Object(m) = &mut step.wait {
        m.remove("heldAt");
        m.remove("heldUntil");
    }
    st.store.put_step_and_wake(step).await.map_err(err)
}

/// A step that waited to start starts over.
async fn wake(st: &AppState, mut step: Step) -> R<bool> {
    if step.status != StepStatus::Waiting {
        return Ok(false);
    }
    let mut wait = json!({});
    for k in KEPT {
        if let Some(v) = step.wait.get(k) {
            wait[k] = v.clone();
        }
    }
    step.status = StepStatus::Pending;
    step.wait = wait;
    step.wake_at = None;
    st.store.put_step_and_wake(&step).await.map_err(err)?;
    tracing::info!(step = %step.id, "an agent step starts now its agent is logged in");
    Ok(true)
}
