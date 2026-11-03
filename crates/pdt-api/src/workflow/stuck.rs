//! "When an agent is stuck" (PDT-367): an agent step that stalls, reaches its loop limit or times out is handed to the
//! built-in workflow of that name (`agent.stalled`, `agent.loop_limit`, `agent.timed_out`), whose nodes nudge it,
//! restart it in a fresh session, stop it or put it in the Inbox. While the workflow is off, or when its run fails or
//! ends without acting on the step, the step gets PDT's own handling: one nudge, then the Inbox. A step handed over
//! waits in `agent_escalations` until one of those happens, so nothing is ever left stuck without anyone told.

use super::agent::{
    PDT, PDT_REOPENS, StepAt, Target, loop_limit_item, new_session, queue_message, stall_item, step_persona,
    timed_out_item,
};
use super::catalog::{Outcome, StepCx};
use super::nodes::agent::{AgentConfig, SessionChoice};
use crate::routes::{AppState, now_ms};
use pdt_core::conversation::SessionMode;
use pdt_core::session::{Session, SessionKind, SessionStatus};
use pdt_core::workflow::agent::{
    AgentStep, AgentStepState, BranchState, Escalation, LOOP_LIMIT, STALLED, STOPPED, StepTicket, TIMED_OUT,
    first_message, nudge_message, restart_message, resumable_step, stuck_nudge_message,
};
use pdt_core::workflow::api::{NewLogEvent, Run, Step, StepStatus, Workflow};
use pdt_core::workflow::events::{self as ev, AgentStuck};
use serde_json::{Value, json};

type R<T> = Result<T, String>;

use super::err;

/// How long a step handed over waits for the workflow to start a run for it before PDT handles it itself.
pub(crate) const START_GRACE_MS: i64 = 60_000;

/// The built-in "When an agent is stuck", while it is on.
pub(crate) async fn handler(st: &AppState) -> Option<Workflow> {
    let w = st.store.workflow_by_builtin(super::builtins::STUCK_AGENTS).await.ok().flatten()?;
    (w.enabled && !super::builtins::hidden(&st.registry(), &w)).then_some(w)
}

/// Whether a stuck step of run `run_id` is handed over: the workflow is on, and the run is not one of its own.
pub(crate) async fn on_for(st: &AppState, run_id: &str) -> bool {
    let Some(w) = handler(st).await else { return false };
    matches!(st.store.get_workflow_run(run_id).await, Ok(Some(r)) if r.workflow_id != w.id)
}

/// A loop-limited step's wait: for the workflow, or the user's answer in the Inbox.
pub(crate) fn loop_limit_wait() -> Outcome {
    Outcome::Wait { wait: json!({"loopLimit": true}), until: None, timeout_port: None, output: Value::Null }
}

/// The event's payload.
#[allow(clippy::too_many_arguments)]
async fn payload(
    st: &AppState,
    kind: &str,
    why: &str,
    step: &Step,
    run: &Run,
    a: Option<&AgentStep>,
    c: Option<&AgentConfig>,
    passes: usize,
) -> AgentStuck {
    let session = match a.and_then(|a| a.session_id.as_deref()) {
        Some(sid) => st.store.get_session(sid).await.ok().flatten(),
        None => None,
    };
    let last_message = match &session {
        Some(s) => super::sensors::last_message(st, s).await,
        None => String::new(),
    };
    let ticket_id = a.and_then(|a| a.ticket_id.clone()).or_else(|| run.ticket_id.clone());
    let ticket_key = match &ticket_id {
        Some(t) => st.store.get_ticket_summary(t).await.ok().flatten().map(|t| t.key),
        None => None,
    };
    AgentStuck {
        kind: kind.into(),
        why: why.into(),
        step_id: step.id.clone(),
        node_id: step.node_id.clone(),
        run_id: run.id.clone(),
        workflow_id: run.workflow_id.clone(),
        workflow_name: run.workflow_name.clone(),
        persona: a.map(|a| a.persona_name.clone()).or_else(|| c.map(|c| c.persona.clone())).unwrap_or_default(),
        session_id: session.map(|s| s.id),
        project_id: a.and_then(|a| a.project_id.clone()).or_else(|| run.project_id.clone()),
        ticket_id,
        ticket_key,
        attempt_id: a.and_then(|a| a.attempt_id.clone()).or_else(|| run.attempt_id.clone()),
        goal: c.map(|c| c.prompt.trim().to_string()).unwrap_or_default(),
        last_message,
        nudges: a.map_or(0, |a| a.nudges),
        restarts: a.map_or(0, |a| a.restarts),
        turns: a.map_or(0, |a| a.turns),
        passes: u32::try_from(passes).unwrap_or(u32::MAX),
    }
}

/// Records the step as handed over and logs its event.
#[allow(clippy::too_many_arguments)]
async fn hand_over(
    st: &AppState,
    kind: &str,
    why: &str,
    step: &Step,
    run: &Run,
    a: Option<&AgentStep>,
    c: Option<&AgentConfig>,
    passes: usize,
) -> bool {
    let at = now_ms();
    let e = Escalation { step_id: step.id.clone(), run_id: run.id.clone(), kind: kind.into(), why: why.into(), at };
    if let Err(x) = st.store.put_escalation(&e).await {
        tracing::warn!(step = %step.id, error = %x, "could not hand a stuck step over");
        return false;
    }
    let p = payload(st, kind, why, step, run, a, c, passes).await;
    let event = match kind {
        LOOP_LIMIT => ev::AGENT_LOOP_LIMIT,
        TIMED_OUT => ev::AGENT_TIMED_OUT,
        _ => ev::AGENT_STALLED,
    };
    let key = format!("{}:{kind}:{at}", step.id);
    if let Err(x) = st.store.emit_event(&NewLogEvent::new(event, key, "agents", p), now_ms()).await {
        // No run will come: the sweep falls back once its grace is over.
        tracing::warn!(step = %step.id, error = %x, "could not log a stuck step's event");
    }
    tracing::info!(step = %step.id, kind, "a stuck agent step was handed to its workflow");
    true
}

/// A working step stalls (`why`): handed over while the workflow is on, marked stalled with no Inbox item yet.
pub(crate) async fn escalate_stall(st: &AppState, a: &AgentStep, why: &str) -> bool {
    if !on_for(st, &a.run_id).await {
        return false;
    }
    let (Ok(Some(step)), Ok(Some(run))) =
        (st.store.get_step(&a.step_id).await, st.store.get_workflow_run(&a.run_id).await)
    else {
        return false;
    };
    let mut b = a.clone();
    b.state = AgentStepState::Stalled;
    b.updated_at = now_ms();
    if let Err(e) = st.store.put_agent_step(&b).await {
        tracing::warn!(step = %b.step_id, error = %e, "could not mark an agent step stalled");
        return false;
    }
    let c = serde_json::from_value::<AgentConfig>(step.input.clone()).ok();
    hand_over(st, STALLED, why, &step, &run, Some(&b), c.as_ref(), 0).await
}

/// A step is at its loop limit: handed over while the workflow is on (the step then waits, as for the Inbox).
pub(crate) async fn escalate_loop_limit(cx: &StepCx<'_>, c: &AgentConfig, passes: &[AgentStep]) -> bool {
    if !on_for(cx.st, &cx.run.id).await {
        return false;
    }
    let why = format!(
        "{} has run {} times in this run (its loop limit is {}).",
        cx.node_title(),
        passes.len(),
        c.loop_limit()
    );
    hand_over(cx.st, LOOP_LIMIT, &why, cx.step, cx.run, passes.last(), Some(c), passes.len()).await
}

/// A step timed out (it has ended by `timeout`): handed over while the workflow is on.
pub(crate) async fn escalate_timed_out(st: &AppState, step: &Step, run: &Run) -> bool {
    if !on_for(st, &run.id).await {
        return false;
    }
    let a = st.store.get_agent_step(&step.id).await.ok().flatten();
    let why = match step.output.get("reason").and_then(Value::as_str) {
        Some(r) => format!("{r}."),
        None => "It had no outcome within its time limit.".into(),
    };
    let c = serde_json::from_value::<AgentConfig>(step.input.clone()).ok();
    hand_over(st, TIMED_OUT, &why, step, run, a.as_ref(), c.as_ref(), 0).await
}

/// How a step is stuck now.
enum Stuck {
    /// Stalled, its step waiting.
    Stalled(AgentStep, Step),
    /// Waiting at its loop limit.
    LoopLimit(Step, Run),
    /// Ended by `timeout`, and its run hasn't moved on from it.
    TimedOut(Step, Run),
    /// Not (any more): it has its outcome, the user acted, or its run moved on.
    No,
}

async fn stuck(st: &AppState, step_id: &str) -> R<Stuck> {
    let Some(step) = st.store.get_step(step_id).await.map_err(err)? else { return Ok(Stuck::No) };
    let Some(run) = st.store.get_workflow_run(&step.run_id).await.map_err(err)? else { return Ok(Stuck::No) };
    if run.status.is_finished() && step.status == StepStatus::Waiting {
        return Ok(Stuck::No);
    }
    if step.status == StepStatus::Waiting {
        if step.wait.get("loopLimit").and_then(Value::as_bool) == Some(true) {
            return Ok(Stuck::LoopLimit(step, run));
        }
        return Ok(match st.store.get_agent_step(step_id).await.map_err(err)? {
            Some(a) if a.state == AgentStepState::Stalled => Stuck::Stalled(a, step),
            _ => Stuck::No,
        });
    }
    let steps = st.store.list_steps(&run.id).await.map_err(err)?;
    let resumable = resumable_step(&steps).is_some_and(|s| s.id == step.id);
    Ok(if resumable && step.port.as_deref() == Some(pdt_core::workflow::agent::TIMEOUT) {
        Stuck::TimedOut(step, run)
    } else {
        Stuck::No
    })
}

/// The run a step belongs to.
async fn run_of(st: &AppState, step_id: &str) -> R<Option<String>> {
    Ok(st.store.get_step(step_id).await.map_err(err)?.map(|s| s.run_id))
}

/// Why the step was handed over (for the Inbox and a fresh session).
async fn handed_why(st: &AppState, step_id: &str, default: &str) -> String {
    match st.store.get_escalation(step_id).await {
        Ok(Some(e)) if !e.why.trim().is_empty() => e.why,
        _ => default.into(),
    }
}

/// Ends the step's escalation: something acted on it.
async fn handled(st: &AppState, step_id: &str) {
    if let Err(e) = st.store.take_escalation(step_id).await {
        tracing::warn!(step = %step_id, error = %e, "could not end a stuck step's escalation");
    }
}

/// Whether a session can take a nudge: a conversation (a stopped one reopens), or a running terminal.
fn takes_messages(st: &AppState, s: &Session) -> bool {
    s.kind == SessionKind::AgentTui
        && (s.mode == SessionMode::Conversation || (s.status == SessionStatus::Running && st.ptys.is_running(&s.id)))
}

fn skipped(why: &str) -> Outcome {
    Outcome::next("skipped", json!({"why": why}))
}

const NOT_STUCK: &str = "the step is no longer stuck";

/// `agent.nudge`: a message to a stalled step's session; it works again.
pub(crate) async fn nudge(cx: &StepCx<'_>, step_id: &str, message: &str) -> R<Outcome> {
    let st = cx.st;
    let Some(run_id) = run_of(st, step_id).await? else { return Ok(skipped(NOT_STUCK)) };
    let _locked = super::engine::lock_other_run(st, cx.run, &run_id).await;
    let Stuck::Stalled(mut a, _) = stuck(st, step_id).await? else { return Ok(skipped(NOT_STUCK)) };
    let session = match &a.session_id {
        Some(sid) => st.store.get_session(sid).await.map_err(err)?.filter(|s| takes_messages(st, s)),
        None => None,
    };
    let Some(s) = session else {
        return Ok(Outcome::next("no_session", json!({"why": "the step's session can't take a message"})));
    };
    let text = stuck_nudge_message(message, &a.outcomes);
    queue_message(st, &s, &text, None, a.ticket_id.clone(), PDT_REOPENS).await?;
    a.nudges += 1;
    a.state = AgentStepState::Working;
    a.background_since = None;
    a.background.clear();
    a.updated_at = now_ms();
    st.store.put_agent_step(&a).await.map_err(err)?;
    handled(st, step_id).await;
    tracing::info!(step = %step_id, nudges = a.nudges, "nudged a stalled agent step");
    Ok(Outcome::next("nudged", json!({"sessionId": s.id, "nudges": a.nudges})))
}

/// `agent.restart`: the step again in a fresh session, with its task, what the earlier session last said and the
/// branch's state. A stalled step goes on in it; a timed-out one is resumed (its run goes back to it).
pub(crate) async fn restart(cx: &StepCx<'_>, step_id: &str) -> R<Outcome> {
    let st = cx.st;
    let Some(run_id) = run_of(st, step_id).await? else { return Ok(skipped(NOT_STUCK)) };
    let _locked = super::engine::lock_other_run(st, cx.run, &run_id).await;
    match stuck(st, step_id).await? {
        Stuck::Stalled(a, step) => {
            let why = handed_why(st, step_id, "It stopped without an outcome.").await;
            let session = restart_stalled(st, a, &step, &why).await?;
            handled(st, step_id).await;
            Ok(Outcome::next("restarted", json!({"sessionId": session})))
        }
        Stuck::TimedOut(step, _) => {
            let why = handed_why(st, step_id, "It had no outcome within its time limit.").await;
            let restarts = st.store.get_agent_step(step_id).await.map_err(err)?.map_or(0, |a| a.restarts);
            super::engine::reopen_at(st, &step, json!({"restarted": true})).await?;
            let mut next = super::engine::new_step(
                &run_id,
                super::engine::next_seq(st, &run_id).await?,
                &step.node_id,
                &step.kind,
            );
            next.wait = json!({"loopAllowed": true, "resumeOf": step.id, "fresh": true, "restarts": restarts + 1,
                               "why": why});
            st.store.put_step(&next).await.map_err(err)?;
            handled(st, step_id).await;
            tracing::info!(step = %step_id, "restarted a timed-out agent step in a fresh session");
            Ok(Outcome::next("restarted", json!({"resumed": true})))
        }
        Stuck::LoopLimit(..) => Ok(skipped("a step at its loop limit isn't restarted: ask, or stop it")),
        Stuck::No => Ok(skipped(NOT_STUCK)),
    }
}

/// A stalled step in a fresh session: the earlier one is stopped, the new one (the attempt's, for a `reuse` step) gets
/// the task again. The step keeps waiting, now on it.
async fn restart_stalled(st: &AppState, mut a: AgentStep, step: &Step, why: &str) -> R<String> {
    let c: AgentConfig = serde_json::from_value(step.input.clone()).map_err(err)?;
    let run = st.store.get_workflow_run(&a.run_id).await.map_err(err)?.ok_or("the run is gone")?;
    let persona = step_persona(st, &c.persona).await?;
    super::agent::agent_on(st, &persona).await?;
    let t = target_of(st, &a, &run).await?;
    let ticket = t.ticket.as_ref().map(|(_, key, title)| StepTicket { key, title });
    let first = first_message(&c.prompt, ticket, &c.outcomes);
    let message = fresh_session_message(st, &first, why, Some(&a), &t).await;
    let old = a.session_id.clone();
    let attempt_for_session = (c.session == SessionChoice::Reuse).then(|| a.attempt_id.clone()).flatten();
    let at =
        StepAt { step_id: &a.step_id, run: &run, workflow_id: &run.workflow_id, workflow_name: &run.workflow_name };
    // The binding names its persona before the session starts: the new session's instructions read it.
    (a.persona_id, a.persona_name, a.persona_version) =
        (Some(persona.id.clone()), persona.name.clone(), persona.version);
    a.session_id = None;
    a.state = AgentStepState::Working;
    a.updated_at = now_ms();
    st.store.put_agent_step(&a).await.map_err(err)?;
    let s = new_session(st, at, &persona, &t, attempt_for_session, message).await?;
    a.session_id = Some(s.id.clone());
    a.restarts += 1;
    a.background_since = None;
    a.background.clear();
    a.updated_at = now_ms();
    st.store.put_agent_step(&a).await.map_err(err)?;
    let mut waiting = step.clone();
    waiting.wait = json!({"agent": {"session": s.id, "persona": a.persona_name}});
    st.store.put_step(&waiting).await.map_err(err)?;
    if let Some(old) = old {
        stop_stuck_session(st, &old).await;
    }
    tracing::info!(step = %a.step_id, session = %s.id, restarts = a.restarts, "restarted a stalled agent step in a fresh session");
    Ok(s.id)
}

/// Where a stalled step runs, from its binding.
async fn target_of(st: &AppState, a: &AgentStep, run: &Run) -> R<Target> {
    let ticket = match &a.ticket_id {
        Some(t) => st.store.get_ticket_summary(t).await.map_err(err)?.map(|t| (t.id, t.key, t.title)),
        None => None,
    };
    let attempt = match &a.attempt_id {
        Some(x) => st.store.get_attempt(x).await.map_err(err)?,
        None => None,
    };
    let project_id = match a.project_id.clone().or_else(|| run.project_id.clone()) {
        Some(p) => p,
        None => crate::pm::host_project(st).await.map_err(|e| e.message().to_string())?.id,
    };
    let host = ticket.is_none() && attempt.is_none() && run.project_id.is_none();
    Ok(Target { project_id, host, ticket, attempt })
}

/// A fresh session's first message: the step's (`first`), why it restarted, the earlier session's last message and
/// the branch's state.
pub(crate) async fn fresh_session_message(
    st: &AppState,
    first: &str,
    why: &str,
    earlier: Option<&AgentStep>,
    t: &Target,
) -> String {
    let last = match earlier.and_then(|a| a.session_id.as_deref()) {
        Some(sid) => match st.store.get_session(sid).await {
            Ok(Some(s)) => super::sensors::last_message(st, &s).await,
            _ => String::new(),
        },
        None => String::new(),
    };
    let branch = branch_state(t).await;
    restart_message(first, why, &last, branch.as_ref())
}

/// The attempt's branch: what is uncommitted or unpushed, and its latest commits.
async fn branch_state(t: &Target) -> Option<BranchState> {
    let a = t.attempt.as_ref()?;
    let path = std::path::PathBuf::from(a.worktree_path.as_deref()?);
    if !path.is_dir() {
        return None;
    }
    let (branch, base) = (a.attempt.branch.clone(), a.base_commit.clone());
    tokio::task::spawn_blocking(move || {
        let up = pdt_git::unpushed(&path).ok()?;
        let commits = pdt_git::commits_between(&path, &base, "HEAD").unwrap_or_default();
        Some(BranchState {
            branch,
            changed: up.changed,
            ahead: up.ahead,
            unpushed: up.unpushed_branch,
            commits: commits
                .into_iter()
                .take(5)
                .map(|(sha, subject)| format!("{} {subject}", sha.get(..8).unwrap_or(&sha)))
                .collect(),
        })
    })
    .await
    .ok()
    .flatten()
}

/// Stops a stuck step's earlier session once a fresh one took over (off the caller's task: it holds the run's lock).
pub(crate) async fn stop_stuck_session(st: &AppState, session_id: &str) {
    match st.store.get_session(session_id).await {
        Ok(Some(s)) if s.status != SessionStatus::Finished => {}
        _ => return,
    }
    let (st, id) = (st.clone(), session_id.to_string());
    tokio::spawn(async move {
        if let Err(e) = crate::sessions::stop(&st, &id).await {
            tracing::warn!(session = %id, error = %e.message(), "could not stop a stuck step's earlier session");
        }
    });
}

/// `agent.inbox`: the step in the Inbox, as PDT puts it there itself (Stalled, Loop limit or Timed out).
pub(crate) async fn ask(cx: &StepCx<'_>, step_id: &str) -> R<Outcome> {
    let st = cx.st;
    let Some(run_id) = run_of(st, step_id).await? else { return Ok(skipped(NOT_STUCK)) };
    let _locked = super::engine::lock_other_run(st, cx.run, &run_id).await;
    let state = stuck(st, step_id).await?;
    if matches!(state, Stuck::No) {
        return Ok(skipped(NOT_STUCK));
    }
    let why = handed_why(st, step_id, "It stopped without an outcome.").await;
    handled(st, step_id).await;
    to_inbox(st, state, &why).await?;
    Ok(Outcome::next("asked", json!({})))
}

/// Puts a stuck step in the Inbox.
async fn to_inbox(st: &AppState, state: Stuck, why: &str) -> R<()> {
    match state {
        Stuck::Stalled(a, _) => stall_item(st, a, why, None).await,
        Stuck::LoopLimit(step, run) => {
            let c: AgentConfig = serde_json::from_value(step.input.clone()).map_err(err)?;
            let passes = st.store.agent_steps_of_node(&run.id, &step.node_id).await.map_err(err)?;
            let title = match super::engine::load(st, &run).await {
                Ok((_, doc)) => doc.node(&step.node_id).map(|n| n.title.clone()).filter(|t| !t.is_empty()),
                Err(_) => None,
            };
            let title = title.unwrap_or_else(|| step.node_id.clone());
            loop_limit_item(st, &run, &step.id, &step.node_id, &title, &c, &passes).await?;
        }
        Stuck::TimedOut(step, run) => timed_out_item(st, &step, &run).await,
        Stuck::No => {}
    }
    Ok(())
}

/// `agent.stop`: the step ends by `stopped` (a timed-out one has ended already: it is left as it is).
pub(crate) async fn stop(cx: &StepCx<'_>, step_id: &str) -> R<Outcome> {
    let st = cx.st;
    let Some(run_id) = run_of(st, step_id).await? else { return Ok(skipped(NOT_STUCK)) };
    let _locked = super::engine::lock_other_run(st, cx.run, &run_id).await;
    let out = json!({"stopped": true, "by": "workflow", "workflow": cx.workflow.name});
    match stuck(st, step_id).await? {
        Stuck::Stalled(mut a, step) => {
            handled(st, step_id).await;
            a.state = AgentStepState::Done;
            a.updated_at = now_ms();
            st.store.put_agent_step(&a).await.map_err(err)?;
            super::engine::complete_step(st, &step, STOPPED, out).await?;
        }
        Stuck::LoopLimit(step, _) => {
            handled(st, step_id).await;
            super::engine::complete_step(st, &step, STOPPED, out).await?;
        }
        Stuck::TimedOut(..) => handled(st, step_id).await,
        Stuck::No => return Ok(skipped(NOT_STUCK)),
    }
    tracing::info!(step = %step_id, "stopped a stuck agent step");
    Ok(Outcome::next("stopped", json!({})))
}

/// Part of the engine tick: each step handed over that is still stuck, whose workflow is off, or whose run for it
/// ended without acting on it (or never started), gets PDT's own handling: one nudge, then the Inbox.
pub(crate) async fn sweep(st: &AppState) {
    let Ok(handed) = st.store.escalations().await else { return };
    if handed.is_empty() {
        return;
    }
    let handler = handler(st).await;
    let now = now_ms();
    for e in handed {
        let state = match stuck(st, &e.step_id).await {
            Ok(s) => s,
            Err(x) => {
                tracing::warn!(step = %e.step_id, error = %x, "could not look at a stuck step");
                continue;
            }
        };
        if matches!(state, Stuck::No) {
            handled(st, &e.step_id).await;
            continue;
        }
        if !due(st, &e, handler.as_ref(), now).await {
            continue;
        }
        handled(st, &e.step_id).await;
        tracing::info!(step = %e.step_id, kind = %e.kind, "a stuck agent step falls back to PDT's own handling");
        if let Err(x) = fall_back(st, state, &e.why).await {
            tracing::warn!(step = %e.step_id, error = %x, "could not handle a stuck step");
        }
    }
}

/// Whether PDT handles a step handed over itself: the workflow is off (or the step is its own run's), its run for it
/// ended without acting on it, or none started within [`START_GRACE_MS`].
async fn due(st: &AppState, e: &Escalation, handler: Option<&Workflow>, now: i64) -> bool {
    let Some(w) = handler else { return true };
    if matches!(st.store.get_workflow_run(&e.run_id).await, Ok(Some(r)) if r.workflow_id == w.id) {
        return true;
    }
    let f = pdt_store::RunFilter { workflow_id: Some(w.id.clone()), limit: 100, ..Default::default() };
    let runs = st.store.list_workflow_runs(&f).await.unwrap_or_default();
    let mine: Vec<&Run> = runs
        .iter()
        .filter(|r| r.started_at >= e.at && r.trigger.get("stepId").and_then(Value::as_str) == Some(e.step_id.as_str()))
        .collect();
    if mine.iter().any(|r| !r.status.is_finished()) {
        return false;
    }
    !mine.is_empty() || now - e.at >= START_GRACE_MS
}

/// PDT's own handling of a stuck step: a stalled one is nudged once (if it wasn't yet), else it and the others go to
/// the Inbox.
async fn fall_back(st: &AppState, state: Stuck, why: &str) -> R<()> {
    if let Stuck::Stalled(a, _) = &state
        && a.nudges < a.auto_nudges
        && let Some(sid) = a.session_id.as_deref()
        && let Some(s) = st.store.get_session(sid).await.map_err(err)?.filter(|s| takes_messages(st, s))
    {
        let mut a = a.clone();
        queue_message(st, &s, &nudge_message(&a.outcomes), None, a.ticket_id.clone(), PDT).await?;
        a.nudges += 1;
        a.state = AgentStepState::Working;
        a.updated_at = now_ms();
        return st.store.put_agent_step(&a).await.map_err(err);
    }
    to_inbox(st, state, why).await
}
