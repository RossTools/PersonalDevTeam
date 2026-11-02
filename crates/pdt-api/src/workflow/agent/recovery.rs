//! Agent steps that lost their session or their wait: the engine tick's sweep (sessions that ended, background tasks
//! that finished or that a restart stopped, undelivered messages) and an outcome that comes after its step timed out.

use super::{
    GONE_GRACE_MS, PDT_REOPENS, R, checked_outcome, err, finish_with, handed_in, outcome_less_turn, prompt_key,
    queue_message, restart_key, session_run, stall, stall_undelivered, waiting_step,
};
use crate::routes::{AppState, now_ms};
use pdt_core::conversation::SessionMode;
use pdt_core::session::{Session, SessionStatus};
use pdt_core::workflow::agent::{AgentStep, AgentStepState, TIMEOUT, resumable_step};
use pdt_core::workflow::api::StepStatus;
use serde_json::{Value, json};

/// A `submit_outcome` from the session of a step that timed out, while its run hasn't moved on from it (PDT-123): the
/// run goes on from that step's outcome port, as if the outcome had come in time.
pub(super) async fn late_outcome(st: &AppState, s: &Session, input: crate::mcp::tools::SubmitOutcome) -> R<Value> {
    const NONE: &str = "no workflow step is waiting for an outcome from this session";
    let old = st.store.latest_agent_step(&s.id).await.map_err(err)?.ok_or(NONE)?;
    let steps = st.store.list_steps(&old.run_id).await.map_err(err)?;
    let timed_out = resumable_step(&steps)
        .filter(|x| x.id == old.step_id && x.port.as_deref() == Some(TIMEOUT))
        .cloned()
        .ok_or(NONE)?;
    let summary = checked_outcome(&old, &input)?;
    handed_in(st, &old, &input.outcome).await?;
    let run = crate::workflow::engine::reopen_at(st, &timed_out, json!({"lateOutcome": input.outcome})).await?;
    // The node's step again, waiting as if it had never timed out, and ended at once by the outcome.
    let now = now_ms();
    let mut step = crate::workflow::engine::new_step(
        &run.id,
        crate::workflow::engine::next_seq(st, &run.id).await?,
        &timed_out.node_id,
        &timed_out.kind,
    );
    step.status = StepStatus::Waiting;
    step.attempts = 1;
    step.input = timed_out.input.clone();
    step.wait = json!({"agent": {"session": s.id, "persona": old.persona_name}, "lateOutcomeOf": timed_out.id});
    step.started_at = Some(now);
    st.store.put_step(&step).await.map_err(err)?;
    let mut a = AgentStep {
        step_id: step.id.clone(),
        state: AgentStepState::Working,
        outcome: None,
        summary: None,
        comments: vec![],
        data: None,
        by_user: false,
        created_at: now,
        updated_at: now,
        ..old
    };
    st.store.put_agent_step(&a).await.map_err(err)?;
    if !input.comments.is_empty() {
        let cx = crate::mcp::tools::Cx { st, session: s, input: &crate::mcp::tools::NO_INPUT, confirmed: false };
        for c in &input.comments {
            crate::mcp::tools::add_review_comment(&cx, c).await?;
        }
    }
    a.comments = input.comments;
    a.data = input.data.map(Value::Object);
    finish_with(st, a, &step, &input.outcome, summary, false).await?;
    tracing::info!(session = %s.id, outcome = %input.outcome, run = %run.id, "a late outcome resumed a workflow run");
    Ok(json!({"accepted": true, "outcome": input.outcome,
              "message": "Your step had timed out, but its run hadn't moved on: PDT has your outcome and moves the \
                          workflow on. End your turn now."}))
}

/// The background tasks a step waited on all finished and its session stayed quiet (no turn took them up): its last
/// turn ended without an outcome after all (PDT-142).
async fn background_settled(st: &AppState, session_id: &str, step_id: &str) {
    let run = session_run(st, session_id).await;
    let _locked = crate::workflow::engine::lock_run(st, run.as_deref()).await;
    let Ok(Some(a)) = st.store.active_agent_step(session_id).await else { return };
    if a.step_id != step_id || a.state != AgentStepState::Working || a.background_since.is_none() {
        return;
    }
    if !crate::conversations::background(st, session_id).is_some_and(|b| b.settled) {
        return;
    }
    let Ok(step) = waiting_step(st, &a).await else { return };
    tracing::info!(step = %a.step_id, "an agent step's background tasks ended without a turn");
    outcome_less_turn(st, a, &step, session_id).await;
}

/// PDT restarted while the step waited on background tasks (PDT-311): they died with the agent's process and will
/// never report back. The agent is told so, its conversation reopening to take it (an automatic nudge); with none
/// left, the step stalls saying why.
async fn restarted_while_waiting(st: &AppState, mut a: AgentStep, session: &Session) {
    if a.nudges >= a.auto_nudges {
        let why = format!("PDT restarted while it waited on background tasks: {}.", a.background.join(", "));
        return stall(st, a, &why).await;
    }
    let stopped = crate::conversations::take_stopped_background(st, &session.id).await;
    let tasks: Vec<String> =
        if stopped.is_empty() { a.background.clone() } else { stopped.into_iter().map(|t| t.description).collect() };
    a.nudges += 1;
    a.background_since = None;
    a.background.clear();
    a.updated_at = now_ms();
    if let Err(e) = st.store.put_agent_step(&a).await {
        return tracing::warn!(step = %a.step_id, error = %e, "could not write an agent step's binding");
    }
    let text = pdt_core::conversation::restart_notice(&tasks);
    let key = Some(restart_key(&a.step_id));
    if let Err(e) = queue_message(st, session, &text, key, a.ticket_id.clone(), PDT_REOPENS).await {
        tracing::warn!(session = %session.id, error = %e, "could not tell a step its background tasks stopped");
    }
    tracing::info!(step = %a.step_id, tasks = tasks.len(), "a restart stopped the background tasks a step waited on");
}

/// Why a step that waited on background tasks too long stalls.
fn background_why(a: &AgentStep, now: i64) -> String {
    let mins = (now - a.background_since.unwrap_or(now)).max(0) / 60_000;
    format!("Waiting on background tasks for {mins} min: {}.", a.background.join(", "))
}

/// Part of the engine tick: a working step whose session ended (stopped, crashed, or pdtd restarted mid-turn) is
/// stalled, and so is an asking one whose session was removed (its question closed with it: nothing can answer it).
pub(crate) async fn sweep(st: &AppState) {
    crate::workflow::stuck::sweep(st).await;
    let Ok(active) = st.store.active_agent_steps().await else { return };
    let now = now_ms();
    for mut a in active {
        // Waiting on background tasks (PDT-142): while they run, only the cap stalls it; once they ended with no turn
        // taking them up, its last turn counts as ended. A conversation whose process ended is gone (below).
        if a.state == AgentStepState::Working
            && a.background_since.is_some()
            && let Some(sid) = a.session_id.clone()
            && let Some(b) = crate::conversations::background(st, &sid).filter(|b| b.live)
        {
            if !b.tasks.is_empty() {
                // A turn at work meanwhile (a notice, the user's message) isn't stalled under it.
                let capped =
                    now - a.background_since.unwrap_or(now) >= i64::try_from(a.background_wait_ms).unwrap_or(i64::MAX);
                if capped && !b.busy && waiting_step(st, &a).await.is_ok() {
                    a.background = b.tasks.into_iter().map(|t| t.description).collect();
                    let why = background_why(&a, now);
                    stall(st, a, &why).await;
                }
            } else if b.settled {
                // Not here: it takes the run's lock, and the tick holds the engine's.
                let (st2, step) = (st.clone(), a.step_id.clone());
                tokio::spawn(async move { background_settled(&st2, &sid, &step).await });
            }
            continue;
        }
        // Waiting on background tasks when PDT restarted (PDT-311): the agent is told they stopped, not left waiting.
        if a.state == AgentStepState::Working
            && a.background_since.is_some()
            && let Some(sid) = a.session_id.as_deref()
            && let Ok(Some(s)) = st.store.get_session(sid).await
            && s.mode == SessionMode::Conversation
            && s.status == SessionStatus::Interrupted
            && waiting_step(st, &a).await.is_ok()
        {
            restarted_while_waiting(st, a, &s).await;
            continue;
        }
        // The step's message to a reused session (PDT-144): while it waits to go in (a conversation a restart
        // interrupted reopens to take it, a terminal about to resume waits for it), the step waits too; one that never
        // reached the session since the step last went on (a Nudge) stalls it at once, saying so.
        let prompt = match (&a.state, &a.session_id) {
            (AgentStepState::Working, Some(sid)) => {
                st.store.latest_session_message(sid, &prompt_key(&a.step_id)).await.ok().flatten()
            }
            _ => None,
        };
        // So is the notice that a restart stopped its background tasks, on its way to reopen it (PDT-311).
        let notice = match (&a.state, &a.session_id) {
            (AgentStepState::Working, Some(sid)) => {
                st.store.latest_session_message(sid, &restart_key(&a.step_id)).await.ok().flatten()
            }
            _ => None,
        };
        let on_its_way = [&prompt, &notice]
            .iter()
            .any(|m| m.as_ref().is_some_and(|m| m.status == "waiting" || m.status == "sending"));
        let untold = prompt
            .as_ref()
            .filter(|m| m.status == "untold" && m.resolved_at.unwrap_or(0) >= a.updated_at)
            .map(|m| m.text.clone());
        let waited = now - a.updated_at >= GONE_GRACE_MS || untold.is_some();
        if !(a.state == AgentStepState::Working && waited || a.state == AgentStepState::Asking) {
            continue;
        }
        if !matches!(st.store.get_step(&a.step_id).await, Ok(Some(s)) if s.status == StepStatus::Waiting) {
            let mut done = a.clone();
            done.state = AgentStepState::Done;
            if let Err(e) = st.store.put_agent_step(&done).await {
                tracing::warn!(step = %done.step_id, error = %e, "could not write an agent step's binding");
            }
            continue;
        }
        // Removing a session clears its steps' `session_id` (a working step's binding is written with the session
        // once it started, so a waiting step without one lost it).
        let Some(sid) = a.session_id.clone() else {
            let why = if a.state == AgentStepState::Asking {
                "Its session was removed while it waited for your answer."
            } else {
                "Its session was removed before it submitted an outcome."
            };
            stall(st, a, why).await;
            continue;
        };
        if on_its_way {
            continue;
        }
        if let Some(text) = untold {
            stall_undelivered(st, a, &text).await;
            continue;
        }
        let gone = match st.store.get_session(&sid).await {
            // An asking step waits for the user, however its session is.
            Ok(Some(_)) if a.state == AgentStepState::Asking => false,
            Ok(Some(s)) => match (s.status, s.mode) {
                (SessionStatus::Running, SessionMode::Conversation) => {
                    !crate::conversations::is_live_or_waiting(st, &s)
                }
                (SessionStatus::Running, _) => !st.ptys.is_running(&sid),
                _ => true,
            },
            Ok(None) => true,
            Err(_) => false,
        };
        if gone {
            stall(st, a, "Its session stopped before it submitted an outcome.").await;
        }
    }
}
