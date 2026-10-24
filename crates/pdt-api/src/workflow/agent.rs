//! Agent steps (Phase 7b spec §4–§5): starting a persona's session for an `agent` node, the outcome it submits through
//! pdt-mcp, its questions, turns that end without an outcome (nudges, stalls, max turns), loop limits, and the user
//! taking a step over from the Inbox, and steps that timed out: the Inbox, Resume and a late outcome (PDT-123).

use super::catalog::{Outcome, StepCx};
use super::nodes::agent::{AgentConfig, Owner, SessionChoice};
use crate::routes::{AppState, now_ms};
use pdt_core::conversation::SessionMode;
use pdt_core::inbox::{self, InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus, Question};
use pdt_core::persona::{Persona, effective_permission_mode};
use pdt_core::session::{Session, SessionKind, SessionPersona, SessionStatus, TerminalSize, ValidSession};
use pdt_core::ticket::{Actor, AttemptState, EventKind};
use pdt_core::workflow::agent::{
    AgentStep, AgentStepState, OutcomeSpec, STOPPED, StepTicket, TIMEOUT, back_because, first_message,
    next_pass_message, nudge_message, resumable_step, resume_message, step_section,
};
use pdt_core::workflow::api::{Step, StepStatus};
use pdt_core::workspace::TabPlacement;
use pdt_store::SessionMessage;
use serde_json::{Value, json};

type R<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// How long a step may wait for a session that ended mid-step before it counts as stalled.
const GONE_GRACE_MS: i64 = 30_000;

/// The dedupe key of a step's first message to a reused session: until it is delivered, the session's turns are
/// its earlier work, not the step's.
fn prompt_key(step_id: &str) -> String {
    format!("agent:{step_id}")
}

/// What an agent node's step resolves to before it starts.
struct Target {
    project_id: String,
    ticket: Option<(String, String, String)>,
    attempt: Option<pdt_store::AttemptInfo>,
}

async fn target(cx: &StepCx<'_>, c: &AgentConfig) -> R<Target> {
    let st = cx.st;
    let pick = |given: &Option<String>, key: &str| {
        given
            .clone()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| cx.ctx.pointer(&format!("/trigger/{key}")).and_then(Value::as_str).map(String::from))
            .filter(|v| !v.trim().is_empty())
    };
    let mut attempt = match pick(&c.attempt, "attemptId") {
        Some(a) => Some(st.store.get_attempt(&a).await.map_err(err)?.ok_or_else(|| format!("no attempt {a}"))?),
        None => None,
    };
    let ticket_id = pick(&c.ticket, "ticketId").or_else(|| attempt.as_ref().map(|a| a.ticket_id.clone()));
    let ticket = match &ticket_id {
        Some(t) => Some(st.store.get_ticket(t).await.map_err(err)?.ok_or_else(|| format!("no ticket {t}"))?),
        None => None,
    };
    if attempt.is_none()
        && let Some(t) = &ticket
        && let Some(a) = t.attempts.iter().find(|a| a.state == AttemptState::Active)
    {
        attempt = st.store.get_attempt(&a.id).await.map_err(err)?;
    }
    let project_id = ticket
        .as_ref()
        .map(|t| t.summary.project_id.clone())
        .or_else(|| cx.run.project_id.clone())
        .or_else(|| cx.workflow.project_id.clone())
        .ok_or("the step has no project to run in: give it a ticket, or use it in a project's workflow")?;
    Ok(Target {
        project_id,
        ticket: ticket.map(|t| (t.summary.id.clone(), t.summary.key.clone(), t.summary.title.clone())),
        attempt,
    })
}

/// Runs an `agent` node: the loop limit, then the persona's session and its first message, then a wait for the
/// outcome. A step re-run after a restart finds its binding and waits again.
pub(crate) async fn start_step(cx: &StepCx<'_>, c: AgentConfig) -> R<Outcome> {
    let st = cx.st;
    let now = now_ms();
    // A manual step waits for the user as long as it takes; an agent's has its max runtime.
    let wait = |a: &AgentStep| Outcome::Wait {
        wait: json!({"agent": {"session": a.session_id, "persona": a.persona_name}}),
        until: (a.state != AgentStepState::Manual)
            .then(|| a.created_at + i64::try_from(c.max_runtime_ms()).unwrap_or(i64::MAX / 4)),
        timeout_port: Some(TIMEOUT.into()),
        output: Value::Null,
    };
    if let Some(a) = st.store.get_agent_step(&cx.step.id).await.map_err(err)? {
        match (&a.state, &a.outcome, &a.session_id) {
            (AgentStepState::Done, Some(o), _) => return Ok(Outcome::next(o, outcome_output(&a))),
            // Its session never started (`new_session` failed or the step's time limit cut it), or the step ended
            // without an outcome: this run of the step (a Retry, a node retry) starts over (PDT-129).
            (AgentStepState::Working, _, None) | (AgentStepState::Done, None, _) => {
                st.store.delete_agent_step(&cx.step.id).await.map_err(err)?;
            }
            _ => return Ok(wait(&a)),
        }
    }
    // Loop limit (spec §5.2): earlier passes of this node in the run.
    let passes = st.store.agent_steps_of_node(&cx.run.id, &cx.node.id).await.map_err(err)?;
    let allowed = cx.step.wait.get("loopAllowed").and_then(Value::as_bool) == Some(true);
    let limit = c.loop_limit();
    if limit > 0 && passes.len() > usize::try_from(limit).unwrap_or(usize::MAX) && !allowed {
        return loop_limit(cx, &c, &passes).await;
    }
    let persona = st
        .store
        .find_persona(&c.persona)
        .await
        .map_err(err)?
        .ok_or_else(|| format!("no persona is called {}", c.persona))?;
    if persona.archived {
        return Err(format!("the persona {} is archived", persona.name));
    }
    let t = target(cx, &c).await?;
    if let (Some(stage), Some((ticket_id, _, _))) = (c.pipeline_stage, &t.ticket)
        && let Err(e) = st.store.set_pipeline_stage(ticket_id, Some(stage)).await
    {
        tracing::warn!(ticket = %ticket_id, error = %e, "could not set the ticket's pipeline stage");
    }
    // A resumed step (PDT-123) goes on in the session of the step it resumes, while that is there.
    let resume_of = cx.step.wait.get("resumeOf").and_then(Value::as_str).map(String::from);
    let note = cx.step.wait.get("note").and_then(Value::as_str).map(String::from);
    let resumed = match (&resume_of, c.owner) {
        (Some(id), Owner::Agent) => resumed_session(st, id).await,
        _ => None,
    };
    let reused = match (c.owner, c.session) {
        _ if resumed.is_some() => resumed.clone(),
        (Owner::Agent, SessionChoice::Reuse) => reusable_session(st, cx, &t).await,
        _ => None,
    };
    // A new session runs a turn at once: under the host's cap it waits for a slot (Phase 7c §6.2). A reused one gets
    // its message through the outbox, which keeps to the cap itself.
    let admitted = cx.step.wait.get("admitted").and_then(Value::as_bool) == Some(true);
    // Steps of different runs start side by side (PDT-128): one at a time here, so two can't both take the last slot.
    let _starting = st.agent_step_start.lock().await;
    if c.owner == Owner::Agent && reused.is_none() && !admitted && !crate::scheduler::slot_free(st).await {
        tracing::info!(run = %cx.run.id, node = %cx.node.id, "an agent step waits for a slot");
        return Ok(Outcome::Wait {
            wait: json!({"slot": true, "persona": persona.name, "loopAllowed": allowed, "resumeOf": resume_of,
                         "note": note}),
            until: None,
            timeout_port: None,
            output: Value::Null,
        });
    }
    let mut binding = AgentStep {
        step_id: cx.step.id.clone(),
        run_id: cx.run.id.clone(),
        node_id: cx.node.id.clone(),
        session_id: None,
        persona_id: Some(persona.id.clone()),
        persona_name: persona.name.clone(),
        persona_version: persona.version,
        project_id: Some(t.project_id.clone()),
        ticket_id: t.ticket.as_ref().map(|x| x.0.clone()),
        attempt_id: t.attempt.as_ref().map(|a| a.attempt.id.clone()),
        outcomes: c.outcomes.clone(),
        state: if c.owner == Owner::User { AgentStepState::Manual } else { AgentStepState::Working },
        turns: 0,
        nudges: 0,
        max_turns: c.max_turns(),
        auto_nudges: c.nudges(),
        outcome: None,
        summary: None,
        comments: vec![],
        by_user: false,
        created_at: now,
        updated_at: now,
    };
    // Written first: the new session's instructions read it (its step section).
    st.store.put_agent_step(&binding).await.map_err(err)?;
    if c.owner == Owner::User {
        manual_item(cx, &c, &binding).await?;
        tracing::info!(run = %cx.run.id, node = %cx.node.id, "a manual step waits for the user");
        return Ok(wait(&binding));
    }
    let ticket = t.ticket.as_ref().map(|(_, key, title)| StepTicket { key, title });
    let mut message = first_message(&c.prompt, ticket, &c.outcomes);
    if resume_of.is_some()
        && let Some(n) = note.as_deref().map(str::trim).filter(|n| !n.is_empty())
    {
        message.push_str(&format!("\n\nThe user's note: {n}"));
    }
    let session_id = match reused {
        Some(s) => {
            // A later pass in the same session: it has the step's role and the ticket already, so only why it is back.
            // Otherwise its instructions were fixed when it started: the step's rules go with the message.
            let again = passes.iter().any(|a| a.session_id.as_deref() == Some(s.id.as_str()));
            let by_user = resumed.is_some();
            let text = if by_user {
                let why = match &resume_of {
                    Some(id) => st.store.get_step(id).await.ok().flatten().map(|x| stop_reason(&x)),
                    None => None,
                };
                resume_message(why.as_deref(), note.as_deref(), &c.outcomes)
            } else if again {
                next_pass_message(why_back(st, cx).await.as_deref(), &c.outcomes)
            } else {
                format!("{}\n\n{message}", step_section(&persona.name, &persona.setup.instructions, &c.outcomes))
            };
            // Resume is the user's click: it reaches a stopped conversation too, like a Nudge.
            queue_message(st, &s, &text, Some(prompt_key(&cx.step.id)), binding.ticket_id.clone(), by_user).await?;
            s.id
        }
        None => {
            let attempt_for_session = (c.session == SessionChoice::Reuse).then(|| binding.attempt_id.clone()).flatten();
            new_session(st, cx, &persona, &t, attempt_for_session, message).await?.id
        }
    };
    binding.session_id = Some(session_id.clone());
    binding.updated_at = now_ms();
    st.store.put_agent_step(&binding).await.map_err(err)?;
    tracing::info!(run = %cx.run.id, node = %cx.node.id, persona = %persona.name, session = %session_id,
        "an agent step started");
    Ok(wait(&binding))
}

/// The session of the step a resumed step resumes (PDT-123), while it is there and can take a message: a
/// conversation (a stopped one reopens), or a running terminal.
async fn resumed_session(st: &AppState, step_id: &str) -> Option<Session> {
    let a = st.store.get_agent_step(step_id).await.ok()??;
    let s = st.store.get_session(a.session_id.as_deref()?).await.ok()??;
    (s.kind == SessionKind::AgentTui && (s.mode == SessionMode::Conversation || s.status == SessionStatus::Running))
        .then_some(s)
}

/// Why an agent step ended without an outcome, for the user and the resumed session.
fn stop_reason(step: &Step) -> String {
    if let Some(r) = step.output.get("reason").and_then(Value::as_str) {
        return r.to_string();
    }
    match step.port.as_deref() {
        Some(STOPPED) => "it was stopped at its loop limit".into(),
        _ => "it had no outcome within its time limit".into(),
    }
}

/// An agent step took its `timeout` port (its max runtime, or its max turns): a Needs-you item in the Inbox naming
/// the ticket, step and session, with Resume and Stop (PDT-123). The step has ended, so the item stays open until the
/// user answers it, the run is resumed or cancelled.
pub(crate) async fn timed_out(st: &AppState, step: &Step, run: &pdt_core::workflow::api::Run) {
    let Ok(Some(a)) = st.store.get_agent_step(&step.id).await else { return };
    // A removed session is not offered (PDT-93).
    let session_id = match &a.session_id {
        Some(sid) if matches!(st.store.get_session(sid).await, Ok(Some(_))) => Some(sid.clone()),
        _ => None,
    };
    let why = stop_reason(step);
    let item = InboxItem {
        id: uuid::Uuid::now_v7().to_string(),
        kind: InboxKind::TimedOut,
        group: InboxGroup::Stalled,
        status: InboxStatus::Open,
        project_id: a.project_id.clone().or_else(|| run.project_id.clone()),
        ticket_id: a.ticket_id.clone(),
        ticket_key: None,
        ticket_title: None,
        attempt_id: a.attempt_id.clone(),
        session_id,
        run_id: Some(a.run_id.clone()),
        step_id: Some(a.step_id.clone()),
        persona_id: a.persona_id.clone(),
        persona_name: None,
        title: format!("{} timed out at {}", a.persona_name, step.node_id),
        body: json!({"why": why, "node": step.node_id, "workflow": run.workflow_name, "outcomes": a.outcomes,
                     "turns": a.turns}),
        choices: vec![InboxChoice::new(inbox::RESUME, "Resume", true), InboxChoice::new(inbox::STOP, "Stop", false)],
        resolution: Value::Null,
        created_at: now_ms(),
        resolved_at: None,
    };
    if let Err(e) = st.store.open_inbox_item(&item).await {
        tracing::warn!(step = %step.id, error = %e, "could not put a timed-out step in the Inbox");
    }
    tracing::info!(step = %step.id, persona = %a.persona_name, "an agent step timed out");
}

/// Why a step runs again: the outcome or gate answer of the latest agent step or gate before it in the run.
async fn why_back(st: &AppState, cx: &StepCx<'_>) -> Option<String> {
    let steps = st.store.list_steps(&cx.run.id).await.ok()?;
    steps.iter().rev().filter(|x| x.seq < cx.step.seq && x.status == StepStatus::Succeeded).find_map(|x| {
        let port = x.port.as_deref()?;
        back_because(&x.kind, port, &x.output)
    })
}

/// A manual step's Inbox item: the step's task, and one choice per outcome (Phase 7c §3.4).
async fn manual_item(cx: &StepCx<'_>, c: &AgentConfig, a: &AgentStep) -> R<()> {
    let choices = c
        .outcomes
        .iter()
        .enumerate()
        .map(|(i, o)| InboxChoice::new(inbox::outcome_choice(&o.id), &o.id, i == 0))
        .collect();
    let item = InboxItem {
        id: uuid::Uuid::now_v7().to_string(),
        kind: InboxKind::Manual,
        group: InboxGroup::Gate,
        status: InboxStatus::Open,
        project_id: a.project_id.clone(),
        ticket_id: a.ticket_id.clone(),
        ticket_key: None,
        ticket_title: None,
        attempt_id: a.attempt_id.clone(),
        session_id: None,
        run_id: Some(a.run_id.clone()),
        step_id: Some(a.step_id.clone()),
        persona_id: a.persona_id.clone(),
        persona_name: None,
        title: format!("Your turn: {}", cx.node_title()),
        body: json!({"prompt": c.prompt, "outcomes": c.outcomes, "node": cx.node.id, "persona": a.persona_name,
                     "workflow": cx.workflow.name}),
        choices,
        resolution: Value::Null,
        created_at: now_ms(),
        resolved_at: None,
    };
    cx.st.store.open_inbox_item(&item).await.map_err(err)?;
    Ok(())
}

/// A session `reuse` can take: this node's earlier session in the run, else the attempt's running agent session.
async fn reusable_session(st: &AppState, cx: &StepCx<'_>, t: &Target) -> Option<Session> {
    let alive = |s: &Session| s.kind == SessionKind::AgentTui && !matches!(s.status, SessionStatus::Finished);
    let earlier = st.store.agent_steps_of_node(&cx.run.id, &cx.node.id).await.ok()?;
    for a in earlier.iter().rev() {
        if let Some(id) = &a.session_id
            && let Ok(Some(s)) = st.store.get_session(id).await
            && alive(&s)
        {
            return Some(s);
        }
    }
    let attempt = t.attempt.as_ref()?;
    let sessions = st.store.list_sessions(&t.project_id).await.ok()?;
    sessions.into_iter().rev().find(|s| s.attempt_id.as_deref() == Some(attempt.attempt.id.as_str()) && alive(s))
}

/// Where a step's new session on a ticket opens its tab (Coder, Reviewer, QA, Refiner alike): at the end of the
/// workspace the attempt's Start made, so after the ticket's tab and the sessions before it; the project's workspace
/// when there is no attempt (Refine) or its workspace is gone. None (no tab) for a step with no ticket.
async fn step_tab(st: &AppState, t: &Target) -> Option<Option<TabPlacement>> {
    t.ticket.as_ref()?;
    let workspace = match t.attempt.as_ref().and_then(|a| a.attempt.workspace_id.as_ref()) {
        Some(id) => st.store.get_workspace(id).await.ok().flatten(),
        None => None,
    };
    Some(workspace.map(|w| TabPlacement { workspace_id: w.id, position: None }))
}

/// A new conversation for the persona (spec §4.2), its first message the step's.
async fn new_session(
    st: &AppState,
    cx: &StepCx<'_>,
    persona: &Persona,
    t: &Target,
    attempt_id: Option<String>,
    message: String,
) -> R<Session> {
    let project = st.store.get_project(&t.project_id).await.map_err(err)?.ok_or("the project is gone")?;
    let settings = st.store.get_project_settings(&project.id).await.map_err(err)?;
    let host = st.store.get_host_settings().await.map_err(err)?;
    let host_default = if t.ticket.is_some() { host.ticket_permission_mode } else { host.session_permission_mode };
    let mode = effective_permission_mode(settings.agent_permission_mode, persona.setup.permission_mode, host_default);
    let worktree_id = t.attempt.as_ref().and_then(|a| a.attempt.worktree_id.clone());
    let cwd = match (&t.ticket, project.is_personal()) {
        (Some((_, key, _)), true) => Some(crate::start::personal_folder(&project, key).map_err(err)?),
        _ => None,
    };
    let title = match &t.ticket {
        Some((_, key, _)) => format!("{} · {key}", persona.name),
        None => persona.name.clone(),
    };
    let tab = step_tab(st, t).await;
    let req = crate::sessions::CreateSession {
        valid: ValidSession {
            project_id: project.id.clone(),
            worktree_id,
            kind: SessionKind::AgentTui,
            agent: Some(persona.setup.agent),
            bypass_permissions: mode == pdt_core::conversation::PermissionMode::Bypass,
            size: TerminalSize { cols: 120, rows: 30 },
            initial_prompt: Some(message),
            mode: SessionMode::Conversation,
            model: persona.setup.model.clone(),
            permission_mode: Some(mode),
        },
        title: Some(title),
        attempt_id,
        ticket: t
            .ticket
            .as_ref()
            .map(|(id, key, _)| pdt_core::ticket::SessionTicket { id: id.clone(), key: key.clone() }),
        tab,
        persona: Some(SessionPersona {
            id: persona.id.clone(),
            name: persona.name.clone(),
            version: persona.version,
            step_id: Some(cx.step.id.clone()),
        }),
        cwd,
    };
    let s = crate::sessions::create_session(st, req).await.map_err(|e| e.message().to_string())?;
    if let Some((ticket_id, _, _)) = &t.ticket {
        let data = json!({"sessionId": s.id, "agent": persona.setup.agent, "persona": persona.name,
                          "personaVersion": persona.version, "attemptId": s.attempt_id, "runId": cx.run.id,
                          "workflowId": cx.workflow.id, "workflowName": cx.workflow.name});
        if let Err(e) =
            st.store.add_ticket_event(ticket_id, EventKind::Session, Actor::System, None, Some(data), now_ms()).await
        {
            tracing::warn!(ticket = %ticket_id, error = %e, "could not record the step's session on the timeline");
        }
    }
    Ok(s)
}

/// Queues a message to a session through the outbox (7a's delivery rule: never into a running turn).
async fn queue_message(
    st: &AppState,
    s: &Session,
    text: &str,
    dedupe: Option<String>,
    ticket_id: Option<String>,
    by_user: bool,
) -> R<()> {
    let m = SessionMessage {
        id: uuid::Uuid::now_v7().to_string(),
        attempt_id: None,
        session_id: Some(s.id.clone()),
        ticket_id,
        dedupe,
        text: text.to_string(),
        timeline: Value::Null,
        run_id: None,
        step_id: None,
        status: "waiting".into(),
        told_session: None,
        created_at: now_ms(),
        resolved_at: None,
        by_user,
        pending_id: None,
    };
    st.store.queue_session_message(&m).await.map_err(err)?;
    Ok(())
}

fn outcome_output(a: &AgentStep) -> Value {
    json!({
        "outcome": a.outcome, "summary": a.summary, "comments": a.comments, "sessionId": a.session_id,
        "persona": a.persona_name, "personaVersion": a.persona_version, "turns": a.turns,
        "by": if a.by_user { "user" } else { "agent" },
    })
}

/// The loop limit's Inbox item, and a wait for its answer.
async fn loop_limit(cx: &StepCx<'_>, c: &AgentConfig, passes: &[AgentStep]) -> R<Outcome> {
    let st = cx.st;
    let first = passes.first();
    let mut choices = vec![InboxChoice::new(inbox::AGAIN, "Once more", true)];
    choices.extend(
        c.outcomes
            .iter()
            .map(|o| InboxChoice::new(inbox::outcome_choice(&o.id), format!("Take over: {}", o.id), false)),
    );
    choices.push(InboxChoice::new(inbox::STOP, "Stop", false));
    let history: Vec<Value> = passes
        .iter()
        .map(|a| json!({"persona": a.persona_name, "outcome": a.outcome, "summary": a.summary, "at": a.updated_at}))
        .collect();
    let item = InboxItem {
        id: uuid::Uuid::now_v7().to_string(),
        kind: InboxKind::LoopLimit,
        group: InboxGroup::LoopLimit,
        status: InboxStatus::Open,
        project_id: first.and_then(|a| a.project_id.clone()).or_else(|| cx.run.project_id.clone()),
        ticket_id: first.and_then(|a| a.ticket_id.clone()).or_else(|| cx.run.ticket_id.clone()),
        ticket_key: None,
        ticket_title: None,
        attempt_id: first.and_then(|a| a.attempt_id.clone()).or_else(|| cx.run.attempt_id.clone()),
        session_id: None,
        run_id: Some(cx.run.id.clone()),
        step_id: Some(cx.step.id.clone()),
        persona_id: first.and_then(|a| a.persona_id.clone()),
        persona_name: None,
        title: format!("{} has run {} times in this run", cx.node_title(), passes.len()),
        body: json!({"passes": history, "outcomes": c.outcomes, "limit": c.loop_limit(), "node": cx.node.id}),
        choices,
        resolution: Value::Null,
        created_at: now_ms(),
        resolved_at: None,
    };
    st.store.open_inbox_item(&item).await.map_err(err)?;
    Ok(Outcome::Wait { wait: json!({"loopLimit": true}), until: None, timeout_port: None, output: Value::Null })
}

/// The step section a persona's session gets in its instructions, when it runs a step.
pub(crate) async fn step_section_for(st: &AppState, s: &Session) -> Option<String> {
    let p = s.persona.as_ref()?;
    let step = p.step_id.as_ref()?;
    let binding = st.store.get_agent_step(step).await.ok()??;
    let version = st.store.get_persona_version(&p.id, p.version).await.ok()??;
    Some(step_section(&p.name, &version.setup.instructions, &binding.outcomes))
}

/// The waiting step of a binding, or why the binding is over.
async fn waiting_step(st: &AppState, a: &AgentStep) -> R<Step> {
    let step = st.store.get_step(&a.step_id).await.map_err(err)?.ok_or("the workflow step is gone")?;
    if step.status != StepStatus::Waiting {
        let mut done = a.clone();
        done.state = AgentStepState::Done;
        done.updated_at = now_ms();
        let _ = st.store.put_agent_step(&done).await;
        return Err("this workflow step has already ended".into());
    }
    Ok(step)
}

/// Ends a binding with `outcome` (the agent's, or the user's on Take over) and moves the workflow on.
async fn finish_with(
    st: &AppState,
    mut a: AgentStep,
    step: &Step,
    outcome: &str,
    summary: String,
    by_user: bool,
) -> R<()> {
    a.state = AgentStepState::Done;
    a.outcome = Some(outcome.to_string());
    a.summary = Some(summary.clone());
    a.by_user = by_user;
    a.updated_at = now_ms();
    st.store.put_agent_step(&a).await.map_err(err)?;
    let run = st.store.get_workflow_run(&a.run_id).await.map_err(err)?;
    if let Some(ticket) = &a.ticket_id {
        let data = json!({
            "outcome": outcome, "summary": summary, "persona": a.persona_name, "personaVersion": a.persona_version,
            "by": if by_user { "user" } else { "agent" }, "sessionId": a.session_id, "attemptId": a.attempt_id,
            "comments": a.comments.len(),
            "workflowId": run.as_ref().map(|r| r.workflow_id.clone()),
            "workflowName": run.as_ref().map(|r| r.workflow_name.clone()),
            "runId": a.run_id,
        });
        let actor = if by_user { Actor::User } else { Actor::Agent };
        if let Err(e) = st.store.add_ticket_event(ticket, EventKind::Outcome, actor, None, Some(data), now_ms()).await {
            tracing::warn!(ticket = %ticket, error = %e, "could not record an outcome on the timeline");
        }
    }
    super::engine::complete_step(st, step, outcome, outcome_output(&a)).await
}

/// The run a session's step belongs to, for its lock: its active binding's, else the step the session was started
/// for (whose binding doesn't name the session yet while the step is starting it), else its latest binding's.
async fn session_run(st: &AppState, session_id: &str) -> Option<String> {
    if let Ok(Some(a)) = st.store.active_agent_step(session_id).await {
        return Some(a.run_id);
    }
    if let Ok(Some(s)) = st.store.get_session(session_id).await
        && let Some(step) = s.persona.and_then(|p| p.step_id)
        && let Ok(Some(a)) = st.store.get_agent_step(&step).await
    {
        return Some(a.run_id);
    }
    st.store.latest_agent_step(session_id).await.ok().flatten().map(|a| a.run_id)
}

/// `submit_outcome` (pdt-mcp): ends the session's step with an outcome it allows. It waits only for its own run (a
/// step of it being started), never for other runs' steps.
pub(crate) async fn submit_outcome(st: &AppState, s: &Session, input: crate::mcp::tools::SubmitOutcome) -> R<Value> {
    let run = session_run(st, &s.id).await;
    let _locked = super::engine::lock_run(st, run.as_deref()).await;
    let Some(mut a) = st.store.active_agent_step(&s.id).await.map_err(err)? else {
        return late_outcome(st, s, input).await;
    };
    let summary = checked_outcome(&a, &input)?;
    let step = waiting_step(st, &a).await?;
    if !input.comments.is_empty() {
        let cx = crate::mcp::tools::Cx { st, session: s };
        for c in &input.comments {
            crate::mcp::tools::add_review_comment(&cx, c).await?;
        }
    }
    a.comments = input.comments;
    finish_with(st, a, &step, &input.outcome, summary, false).await?;
    tracing::info!(session = %s.id, outcome = %input.outcome, "an agent submitted an outcome");
    Ok(json!({"accepted": true, "outcome": input.outcome,
              "message": "PDT has your outcome and moves the workflow on. End your turn now."}))
}

/// The outcome is one of the step's and the summary says something: the summary, trimmed.
fn checked_outcome(a: &AgentStep, input: &crate::mcp::tools::SubmitOutcome) -> R<String> {
    if !a.outcomes.iter().any(|o| o.id == input.outcome) {
        let ids: Vec<&str> = a.outcomes.iter().map(|o| o.id.as_str()).collect();
        return Err(format!("{} is not an outcome of this step; use one of: {}", input.outcome, ids.join(", ")));
    }
    let summary = input.summary.trim().to_string();
    if summary.is_empty() {
        return Err("give a short summary of what you did and found".into());
    }
    Ok(summary)
}

/// A `submit_outcome` from the session of a step that timed out, while its run hasn't moved on from it (PDT-123): the
/// run goes on from that step's outcome port, as if the outcome had come in time.
async fn late_outcome(st: &AppState, s: &Session, input: crate::mcp::tools::SubmitOutcome) -> R<Value> {
    const NONE: &str = "no workflow step is waiting for an outcome from this session";
    let old = st.store.latest_agent_step(&s.id).await.map_err(err)?.ok_or(NONE)?;
    let steps = st.store.list_steps(&old.run_id).await.map_err(err)?;
    let timed_out = resumable_step(&steps)
        .filter(|x| x.id == old.step_id && x.port.as_deref() == Some(TIMEOUT))
        .cloned()
        .ok_or(NONE)?;
    let summary = checked_outcome(&old, &input)?;
    let run = super::engine::reopen_at(st, &timed_out, json!({"lateOutcome": input.outcome})).await?;
    // The node's step again, waiting as if it had never timed out, and ended at once by the outcome.
    let now = now_ms();
    let mut step = super::engine::new_step(
        &run.id,
        super::engine::next_seq(st, &run.id).await?,
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
        by_user: false,
        created_at: now,
        updated_at: now,
        ..old
    };
    st.store.put_agent_step(&a).await.map_err(err)?;
    if !input.comments.is_empty() {
        let cx = crate::mcp::tools::Cx { st, session: s };
        for c in &input.comments {
            crate::mcp::tools::add_review_comment(&cx, c).await?;
        }
    }
    a.comments = input.comments;
    finish_with(st, a, &step, &input.outcome, summary, false).await?;
    tracing::info!(session = %s.id, outcome = %input.outcome, run = %run.id, "a late outcome resumed a workflow run");
    Ok(json!({"accepted": true, "outcome": input.outcome,
              "message": "Your step had timed out, but its run hadn't moved on: PDT has your outcome and moves the \
                          workflow on. End your turn now."}))
}

/// `ask_user` (pdt-mcp): an Inbox question; a step waits for the answers instead of stalling.
pub(crate) async fn ask_user(st: &AppState, s: &Session, questions: Vec<Question>) -> R<Value> {
    let questions = inbox::valid_questions(questions)?;
    let run = session_run(st, &s.id).await;
    let _locked = super::engine::lock_run(st, run.as_deref()).await;
    let binding = st.store.active_agent_step(&s.id).await.map_err(err)?;
    if let Some(a) = &binding {
        // A new question replaces the step's open one.
        for old in st.store.open_items_for_step(&a.step_id).await.map_err(err)? {
            if old.kind == InboxKind::Question {
                st.store
                    .end_inbox_item(&old.id, InboxStatus::Closed, &json!({"replaced": true}), now_ms())
                    .await
                    .map_err(err)?;
            }
        }
    }
    let ticket_id = crate::mcp::tools::session_ticket(st, s).await;
    let first = questions[0].question.clone();
    let item = InboxItem {
        id: uuid::Uuid::now_v7().to_string(),
        kind: InboxKind::Question,
        group: InboxGroup::Question,
        status: InboxStatus::Open,
        project_id: Some(s.project_id.clone()),
        ticket_id,
        ticket_key: None,
        ticket_title: None,
        attempt_id: s.attempt_id.clone().or_else(|| binding.as_ref().and_then(|a| a.attempt_id.clone())),
        session_id: Some(s.id.clone()),
        run_id: binding.as_ref().map(|a| a.run_id.clone()),
        step_id: binding.as_ref().map(|a| a.step_id.clone()),
        persona_id: s.persona.as_ref().map(|p| p.id.clone()),
        persona_name: None,
        title: first,
        body: json!({"questions": questions}),
        choices: vec![],
        resolution: Value::Null,
        created_at: now_ms(),
        resolved_at: None,
    };
    let item = st.store.open_inbox_item(&item).await.map_err(err)?;
    if let Some(mut a) = binding {
        a.state = AgentStepState::Asking;
        a.updated_at = now_ms();
        st.store.put_agent_step(&a).await.map_err(err)?;
    }
    Ok(json!({"asked": true, "inboxItemId": item.id,
              "message": "Your questions are in the user's Inbox. Their answers will come as a message; end your turn now."}))
}

/// A session's turn ended (a conversation's `Turn` end, a terminal going idle): a working step with no outcome is
/// nudged, then stalled; at `maxTurns` it times out (spec §4.4).
pub(crate) async fn turn_ended(st: &AppState, session_id: &str) {
    let run = session_run(st, session_id).await;
    let _locked = super::engine::lock_run(st, run.as_deref()).await;
    let Ok(Some(mut a)) = st.store.active_agent_step(session_id).await else { return };
    if a.state != AgentStepState::Working {
        return;
    }
    // A reused session's turn before the step's message reached it is its earlier work.
    if let Ok(waiting) = st.store.waiting_session_messages().await
        && waiting.iter().any(|m| m.dedupe.as_deref() == Some(prompt_key(&a.step_id).as_str()))
    {
        return;
    }
    let Ok(step) = waiting_step(st, &a).await else { return };
    a.turns += 1;
    a.updated_at = now_ms();
    if a.turns >= a.max_turns {
        a.state = AgentStepState::Done;
        let _ = st.store.put_agent_step(&a).await;
        let output = json!({"timedOut": true, "reason": format!("{} turns ended without an outcome", a.turns),
                            "sessionId": a.session_id, "persona": a.persona_name});
        if let Err(e) = super::engine::complete_step(st, &step, TIMEOUT, output).await {
            tracing::warn!(step = %a.step_id, error = %e, "could not time out an agent step");
        }
        return;
    }
    if a.nudges < a.auto_nudges {
        a.nudges += 1;
        let _ = st.store.put_agent_step(&a).await;
        if let Ok(Some(s)) = st.store.get_session(session_id).await
            && let Err(e) = queue_message(st, &s, &nudge_message(&a.outcomes), None, a.ticket_id.clone(), false).await
        {
            tracing::warn!(session = %session_id, error = %e, "could not nudge an agent step's session");
        }
        return;
    }
    stall(st, a, "Its turn ended without an outcome.").await;
}

/// Marks a binding stalled and puts it in the Inbox (Nudge / Take over).
async fn stall(st: &AppState, mut a: AgentStep, why: &str) {
    a.state = AgentStepState::Stalled;
    a.updated_at = now_ms();
    if let Err(e) = st.store.put_agent_step(&a).await {
        return tracing::warn!(step = %a.step_id, error = %e, "could not mark an agent step stalled");
    }
    // A removed session is not offered (PDT-93).
    let session_id = match &a.session_id {
        Some(sid) if matches!(st.store.get_session(sid).await, Ok(Some(_))) => Some(sid.clone()),
        _ => None,
    };
    let mut choices = vec![InboxChoice::new(inbox::NUDGE, "Nudge", true)];
    choices.extend(
        a.outcomes
            .iter()
            .map(|o| InboxChoice::new(inbox::outcome_choice(&o.id), format!("Take over: {}", o.id), false)),
    );
    let item = InboxItem {
        id: uuid::Uuid::now_v7().to_string(),
        kind: InboxKind::Stalled,
        group: InboxGroup::Stalled,
        status: InboxStatus::Open,
        project_id: a.project_id.clone(),
        ticket_id: a.ticket_id.clone(),
        ticket_key: None,
        ticket_title: None,
        attempt_id: a.attempt_id.clone(),
        session_id,
        run_id: Some(a.run_id.clone()),
        step_id: Some(a.step_id.clone()),
        persona_id: a.persona_id.clone(),
        persona_name: None,
        title: format!("{} stalled", a.persona_name),
        body: json!({"why": why, "outcomes": a.outcomes, "turns": a.turns, "nudges": a.nudges}),
        choices,
        resolution: Value::Null,
        created_at: now_ms(),
        resolved_at: None,
    };
    if let Err(e) = st.store.open_inbox_item(&item).await {
        tracing::warn!(step = %a.step_id, error = %e, "could not put a stalled step in the Inbox");
    }
    tracing::info!(step = %a.step_id, persona = %a.persona_name, "an agent step stalled");
}

/// Part of the engine tick: a working step whose session ended (stopped, crashed, or pdtd restarted mid-turn) is
/// stalled, and so is an asking one whose session was removed (its question closed with it: nothing can answer it).
pub(crate) async fn sweep(st: &AppState) {
    let Ok(active) = st.store.active_agent_steps().await else { return };
    let now = now_ms();
    for a in active {
        let waited = now - a.updated_at >= GONE_GRACE_MS;
        if !(a.state == AgentStepState::Working && waited || a.state == AgentStepState::Asking) {
            continue;
        }
        if !matches!(st.store.get_step(&a.step_id).await, Ok(Some(s)) if s.status == StepStatus::Waiting) {
            let mut done = a.clone();
            done.state = AgentStepState::Done;
            let _ = st.store.put_agent_step(&done).await;
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

/// Answers a stalled or loop-limit item, or a question (from the Inbox).
pub(crate) async fn answer_item(st: &AppState, item: &InboxItem, answer: &inbox::InboxAnswer) -> R<Value> {
    match item.kind {
        InboxKind::Question => answer_question(st, item, answer).await,
        InboxKind::Stalled => {
            let choice = answer.choice.as_deref().ok_or("choose Nudge or an outcome")?;
            let _locked = super::engine::lock_run(st, item.run_id.as_deref()).await;
            let step_id = item.step_id.as_deref().ok_or("the item has no step")?;
            let mut a = st.store.get_agent_step(step_id).await.map_err(err)?.ok_or("the step is gone")?;
            let step = waiting_step(st, &a).await?;
            if choice == inbox::NUDGE {
                let s = st
                    .store
                    .get_session(a.session_id.as_deref().unwrap_or_default())
                    .await
                    .map_err(err)?
                    .ok_or("the step's session is gone: take it over instead")?;
                let text = answer
                    .text
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| nudge_message(&a.outcomes));
                if s.status != SessionStatus::Running && s.mode != SessionMode::Conversation {
                    return Err("the step's terminal session has stopped: restart it, or take the step over".into());
                }
                queue_message(st, &s, &text, None, a.ticket_id.clone(), true).await?;
                a.nudges += 1;
                a.state = AgentStepState::Working;
                a.updated_at = now_ms();
                st.store.put_agent_step(&a).await.map_err(err)?;
                end_item(st, item, json!({"choice": choice})).await?;
                return Ok(json!({"nudged": true}));
            }
            let outcome = inbox::choice_outcome(choice).ok_or_else(|| format!("{choice} is not a choice here"))?;
            if !a.outcomes.iter().any(|o| o.id == outcome) {
                return Err(format!("{outcome} is not an outcome of this step"));
            }
            let summary = answer
                .text
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "Taken over by the user.".into());
            end_item(st, item, json!({"choice": choice})).await?;
            finish_with(st, a, &step, outcome, summary, true).await?;
            Ok(json!({"outcome": outcome}))
        }
        InboxKind::LoopLimit => {
            let choice = answer.choice.as_deref().ok_or("choose Once more, an outcome or Stop")?;
            let _locked = super::engine::lock_run(st, item.run_id.as_deref()).await;
            let step_id = item.step_id.as_deref().ok_or("the item has no step")?;
            let mut step = st.store.get_step(step_id).await.map_err(err)?.ok_or("the step is gone")?;
            if step.status != StepStatus::Waiting {
                return Err("this workflow step has already ended".into());
            }
            end_item(st, item, json!({"choice": choice})).await?;
            if choice == inbox::AGAIN {
                step.status = StepStatus::Pending;
                step.wait = json!({"loopAllowed": true});
                step.wake_at = None;
                st.store.put_step(&step).await.map_err(err)?;
                st.store.wake_workflow_run(&step.run_id, now_ms()).await.map_err(err)?;
                return Ok(json!({"again": true}));
            }
            if choice == inbox::STOP {
                super::engine::complete_step(st, &step, STOPPED, json!({"stopped": true, "by": "user"})).await?;
                return Ok(json!({"stopped": true}));
            }
            let outcome = inbox::choice_outcome(choice).ok_or_else(|| format!("{choice} is not a choice here"))?;
            let outcomes: Vec<OutcomeSpec> =
                serde_json::from_value(item.body.get("outcomes").cloned().unwrap_or_default()).unwrap_or_default();
            if !outcomes.iter().any(|o| o.id == outcome) {
                return Err(format!("{outcome} is not an outcome of this step"));
            }
            super::engine::complete_step(st, &step, outcome, json!({"outcome": outcome, "by": "user"})).await?;
            Ok(json!({"outcome": outcome}))
        }
        InboxKind::Gate => {
            let choice = answer.choice.as_deref().ok_or("choose one of the gate's choices")?;
            let (run, step) = (item.run_id.as_deref().unwrap_or_default(), item.step_id.as_deref().unwrap_or_default());
            super::engine::answer(st, run, step, choice, answer.text.as_deref()).await?;
            Ok(json!({"choice": choice}))
        }
        InboxKind::Manual => {
            let choice = answer.choice.as_deref().ok_or("choose the step's outcome")?;
            let _locked = super::engine::lock_run(st, item.run_id.as_deref()).await;
            let step_id = item.step_id.as_deref().ok_or("the item has no step")?;
            let a = st.store.get_agent_step(step_id).await.map_err(err)?.ok_or("the step is gone")?;
            let step = waiting_step(st, &a).await?;
            let outcome = inbox::choice_outcome(choice).ok_or_else(|| format!("{choice} is not a choice here"))?;
            if !a.outcomes.iter().any(|o| o.id == outcome) {
                return Err(format!("{outcome} is not an outcome of this step"));
            }
            let summary =
                answer.text.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "Done by the user.".into());
            end_item(st, item, json!({"choice": choice})).await?;
            finish_with(st, a, &step, outcome, summary, true).await?;
            Ok(json!({"outcome": outcome}))
        }
        InboxKind::TimedOut => {
            let choice = answer.choice.as_deref().ok_or("choose Resume or Stop")?;
            let _locked = super::engine::lock_run(st, item.run_id.as_deref()).await;
            let run_id = item.run_id.as_deref().ok_or("the item has no run")?;
            let steps = st.store.list_steps(run_id).await.map_err(err)?;
            if resumable_step(&steps).map(|s| s.id.as_str()) != item.step_id.as_deref() {
                // Resumed from the run, or it went on otherwise: nothing left to decide.
                end_item(st, item, json!({"choice": choice, "movedOn": true})).await?;
                return Err("the run has already moved on from this step".into());
            }
            if choice == inbox::RESUME {
                super::engine::resume_locked(st, run_id, answer.text.clone()).await?;
                return Ok(json!({"resumed": true}));
            }
            if choice == inbox::STOP {
                end_item(st, item, json!({"choice": choice})).await?;
                let run = st.store.get_workflow_run(run_id).await.map_err(err)?;
                if run.is_some_and(|r| !r.status.is_finished()) {
                    super::engine::cancel_locked(st, run_id).await?;
                }
                return Ok(json!({"stopped": true}));
            }
            Err(format!("{choice} is not a choice here"))
        }
        InboxKind::Approval => Err("answer an approval in its session".into()),
        InboxKind::Session => Err("answer the session in its tab".into()),
        InboxKind::Notice => Err("a notice waits on no answer: dismiss it".into()),
    }
}

async fn end_item(st: &AppState, item: &InboxItem, resolution: Value) -> R<()> {
    if !st.store.end_inbox_item(&item.id, InboxStatus::Resolved, &resolution, now_ms()).await.map_err(err)? {
        return Err("this item was already answered".into());
    }
    Ok(())
}

/// Sends the user's answers to the asking session; its step works again.
async fn answer_question(st: &AppState, item: &InboxItem, answer: &inbox::InboxAnswer) -> R<Value> {
    let questions: Vec<Question> =
        serde_json::from_value(item.body.get("questions").cloned().unwrap_or_default()).unwrap_or_default();
    let text = inbox::answers_message(&questions, answer)?;
    let sid = item.session_id.as_deref().ok_or("the question has no session to answer")?;
    let s = st.store.get_session(sid).await.map_err(err)?.ok_or("the asking session is gone")?;
    if s.mode != SessionMode::Conversation && s.status != SessionStatus::Running {
        return Err("the asking terminal session has stopped; restart it first".into());
    }
    let _locked = super::engine::lock_run(st, item.run_id.as_deref()).await;
    end_item(st, item, json!({"answers": answer.answers, "text": answer.text})).await?;
    queue_message(st, &s, &text, None, item.ticket_id.clone(), true).await?;
    if let Some(step) = &item.step_id
        && let Ok(Some(mut a)) = st.store.get_agent_step(step).await
        && a.state == AgentStepState::Asking
    {
        a.state = AgentStepState::Working;
        a.updated_at = now_ms();
        st.store.put_agent_step(&a).await.map_err(err)?;
    }
    Ok(json!({"answered": true}))
}
