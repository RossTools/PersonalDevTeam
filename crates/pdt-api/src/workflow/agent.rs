//! Agent steps (Phase 7b spec §4–§5): starting a persona's session for an `agent` node, the outcome it submits through
//! pdt-mcp, its questions, turns that end without an outcome (nudges, stalls, max turns), loop limits, and the user
//! taking a step over from the Inbox, and steps that timed out: the Inbox, Resume and a late outcome (PDT-123).

use super::catalog::{Outcome, StepCx};
use super::nodes::agent::{AgentConfig, Owner, SessionChoice};
use crate::routes::{AppState, now_ms};
use pdt_core::conversation::{PermissionMode, SessionMode};
use pdt_core::inbox::{self, InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus, Question};
use pdt_core::persona::{Persona, effective_permission_mode};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionPersona, SessionStatus, TerminalSize, ValidSession};
use pdt_core::ticket::{Actor, AttemptState, EventKind};
use pdt_core::workflow::agent::{
    AgentStep, AgentStepState, OutcomeSpec, STOPPED, SUBMITTED, StepTicket, TIMEOUT, back_because, first_message,
    next_pass_message, not_handed_in, nudge_message, resumable_step, resume_message, step_section,
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
/// How much of an undelivered message a stall's reason quotes.
const QUOTE_CHARS: usize = 300;

/// The dedupe key of a step's first message to a reused session: until it is delivered, the session's turns are
/// its earlier work, not the step's.
pub(crate) const PROMPT_KEY: &str = "agent:";

/// [`PROMPT_KEY`] and the step.
fn prompt_key(step_id: &str) -> String {
    format!("{PROMPT_KEY}{step_id}")
}

/// The dedupe key of the notice a step gets when a restart stopped the background tasks it waited on (PDT-311).
fn restart_key(step_id: &str) -> String {
    format!("restarted:{step_id}")
}

/// What an agent node's step resolves to before it starts.
pub(crate) struct Target {
    project_id: String,
    /// No ticket and no project (a personal workflow, PDT-152): the step runs in the host project, as the PM does.
    pub(crate) host: bool,
    ticket: Option<(String, String, String)>,
    attempt: Option<pdt_store::AttemptInfo>,
}

/// Where a step runs: its ticket's project, else the run's or the workflow's, else the host project. A dry run
/// (`dry`) never makes the host project, and takes a ticket or attempt it can't find (a hand-written event's) as one
/// that would be there: the step runs on it, not in the host project.
pub(crate) async fn target(cx: &StepCx<'_>, c: &AgentConfig, dry: bool) -> R<Target> {
    let st = cx.st;
    let pick = |given: &Option<String>, key: &str| {
        given
            .clone()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| cx.ctx.pointer(&format!("/trigger/{key}")).and_then(Value::as_str).map(String::from))
            .filter(|v| !v.trim().is_empty())
    };
    let attempt_id = pick(&c.attempt, "attemptId");
    let mut attempt = match &attempt_id {
        Some(a) => match st.store.get_attempt(a).await.map_err(err)? {
            Some(x) => Some(x),
            None => missing(dry, "attempt", a)?,
        },
        None => None,
    };
    let ticket_id = pick(&c.ticket, "ticketId").or_else(|| attempt.as_ref().map(|a| a.ticket_id.clone()));
    let ticket = match &ticket_id {
        Some(t) => match st.store.get_ticket(t).await.map_err(err)? {
            Some(x) => Some(x),
            None => missing(dry, "ticket", t)?,
        },
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
        .or_else(|| cx.workflow.project_id.clone());
    let host = project_id.is_none() && attempt_id.is_none() && ticket_id.is_none();
    let project_id = match project_id {
        Some(p) => p,
        None if dry => st.store.host_project().await.map_err(err)?.map(|p| p.id).unwrap_or_default(),
        None => crate::pm::host_project(st).await.map_err(|e| e.message().to_string())?.id,
    };
    Ok(Target {
        project_id,
        host,
        ticket: ticket.map(|t| (t.summary.id.clone(), t.summary.key.clone(), t.summary.title.clone())),
        attempt,
    })
}

/// A ticket or attempt a step names that isn't there: an error, or nothing in a dry run.
fn missing<T>(dry: bool, what: &str, id: &str) -> R<Option<T>> {
    if dry { Ok(None) } else { Err(format!("no {what} {id}")) }
}

/// The persona a step names, while it is there and not archived.
pub(crate) async fn step_persona(st: &AppState, name: &str) -> R<Persona> {
    // A dry run may come before the engine's first tick installs the built-ins.
    crate::personas::ensure_builtins(st).await;
    let persona =
        st.store.find_persona(name).await.map_err(err)?.ok_or_else(|| format!("no persona is called {name}"))?;
    if persona.archived {
        return Err(format!("the persona {} is archived", persona.name));
    }
    Ok(persona)
}

/// Err "Codex is switched off in Settings → Agents" when the persona's agent is.
pub(crate) async fn agent_on(st: &AppState, persona: &Persona) -> R<()> {
    match st.store.get_host_settings().await.map_err(err)?.agent_off(persona.setup.agent) {
        Some(why) => Err(format!("{}: {why}", persona.name)),
        None => Ok(()),
    }
}

/// Runs an `agent` node: the loop limit, then the persona's session and its first message, then a wait for the
/// outcome. A step re-run after a restart finds its binding and waits again.
pub(crate) async fn start_step(cx: &StepCx<'_>, c: AgentConfig) -> R<Outcome> {
    let st = cx.st;
    // What an outsider wrote never goes into a session (PDT-202).
    if c.owner == Owner::Agent
        && let Some(why) = crate::untrusted::outsider_refusal(cx.ctx)
    {
        return Err(why);
    }
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
    let persona = step_persona(st, &c.persona).await?;
    // A switched-off agent fails the step (PDT-270): it never falls back to another agent.
    if c.owner == Owner::Agent {
        agent_on(st, &persona).await?;
    }
    let t = target(cx, &c, false).await?;
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
        (Owner::Agent, SessionChoice::Reuse) => reusable_session(st, cx, Some(&t)).await,
        (Owner::Agent, SessionChoice::Own) => reusable_session(st, cx, None).await,
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
        background_since: None,
        background: vec![],
        background_wait_ms: c.background_wait_ms(),
        outcome: None,
        summary: None,
        comments: vec![],
        data: None,
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
    // A later pass in a new session (the earlier one's terminal stopped, PDT-144): the whole task, and why it is back.
    if reused.is_none()
        && c.session != SessionChoice::New
        && resume_of.is_none()
        && !passes.is_empty()
        && let Some(why) = why_back(st, cx).await
    {
        message.push_str(&format!("\n\nThis step is back for another pass. {why}"));
    }
    let fresh = reused.is_none();
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
            // Resume is the user's click: it reaches a stopped conversation too, like a Nudge. The step's own message
            // reopens one a restart interrupted (PDT-144): the user enabled the workflow, and the cap still holds it.
            let to = Delivery { by_user, reopens: true };
            queue_message(st, &s, &text, Some(prompt_key(&cx.step.id)), binding.ticket_id.clone(), to).await?;
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
    record_outside_input(cx, &c, &binding, &session_id, fresh).await;
    tracing::info!(run = %cx.run.id, node = %cx.node.id, persona = %persona.name, session = %session_id,
        "an agent step started");
    Ok(wait(&binding))
}

/// Records the outside input an agent step put into its session (PDT-202): its prompt's fenced outside text, and an
/// external ticket's description a new session reads.
async fn record_outside_input(cx: &StepCx<'_>, c: &AgentConfig, a: &AgentStep, session_id: &str, fresh: bool) {
    use pdt_core::workflow::events::OutsideInput;
    let st = cx.st;
    let record = |source: String, url: Option<String>, via: &str| OutsideInput {
        project_id: a.project_id.clone(),
        ticket_id: a.ticket_id.clone(),
        session_id: session_id.to_string(),
        source,
        url,
        via: via.into(),
        run_id: Some(cx.run.id.clone()),
        workflow_name: Some(cx.workflow.name.clone()),
    };
    if pdt_core::fence::fenced(&c.prompt)
        && let Some((source, url)) = crate::untrusted::trigger_source(&st.registry(), &cx.run.trigger)
    {
        crate::untrusted::record_input(st, record(source, url, "prompt")).await;
    }
    if fresh
        && let Some(t) = &a.ticket_id
        && let Ok(Some(t)) = st.store.get_ticket_summary(t).await
        && let Some((source, url)) = crate::untrusted::ticket_source(&t)
    {
        crate::untrusted::record_input(st, record(source, url, "ticket")).await;
    }
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
        change_request: None,
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
        change_request: None,
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

/// A session `reuse` can take: this node's earlier session in the run, else the attempt's agent session (`t`; `own`
/// passes none, so it never lands in the Coder's conversation, PDT-183). A conversation that was not stopped (a parked or interrupted one reopens to take the message, PDT-144), or a terminal
/// that runs or is about to resume after a restart; a stopped terminal can't take a message, so the step gets a new
/// session.
async fn reusable_session(st: &AppState, cx: &StepCx<'_>, t: Option<&Target>) -> Option<Session> {
    let alive = |s: &Session| {
        s.kind == SessionKind::AgentTui
            && match (s.mode, s.status) {
                (_, SessionStatus::Finished) => false,
                (SessionMode::Conversation, _) => true,
                (_, SessionStatus::Running) => st.ptys.is_running(&s.id),
                (_, SessionStatus::Interrupted) => st.resume_pending(&s.id),
            }
    };
    let earlier = st.store.agent_steps_of_node(&cx.run.id, &cx.node.id).await.ok()?;
    for a in earlier.iter().rev() {
        if let Some(id) = &a.session_id
            && let Ok(Some(s)) = st.store.get_session(id).await
            && alive(&s)
        {
            return Some(s);
        }
    }
    let t = t?;
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
    let mut mode =
        effective_permission_mode(settings.agent_permission_mode, persona.setup.permission_mode, host_default);
    // A locked-down persona (PDT-201) reads untrusted input: Codex runs read-only, Claude asks (its tools are cut down
    // at launch), and Devin or an ACP agent, which PDT can't sandbox, doesn't run it.
    if pdt_core::persona::locked_down(&persona.role) {
        mode = match persona.setup.agent {
            AgentKind::Codex => PermissionMode::Plan,
            AgentKind::Claude => PermissionMode::Ask,
            agent @ (AgentKind::Devin | AgentKind::Acp(_)) => {
                return Err(format!(
                    "{} reads untrusted input, so it runs on Claude or Codex, not {}",
                    persona.name,
                    agent.label()
                ));
            }
        };
    }
    let worktree_id = t.attempt.as_ref().and_then(|a| a.attempt.worktree_id.clone());
    let cwd = match (&t.ticket, project.is_personal()) {
        (Some((_, key, _)), true) => Some(crate::start::personal_folder(&project, key).map_err(err)?),
        // In the host project (PDT-152): the PM's work folder.
        _ if t.host => Some(crate::pm::work_folder(st, &project.repo_path).await.map_err(|e| e.message().to_string())?),
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
        id: None,
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

/// Who a message to a step's session comes from: the user (their click: the cap never holds it, and it reopens a
/// stopped conversation), or PDT, whose message `reopens` a conversation that is not running only when it says so.
#[derive(Clone, Copy)]
struct Delivery {
    by_user: bool,
    reopens: bool,
}

const USER: Delivery = Delivery { by_user: true, reopens: true };
const PDT: Delivery = Delivery { by_user: false, reopens: false };
/// PDT's own message that reopens the conversation a restart interrupted (PDT-311).
const PDT_REOPENS: Delivery = Delivery { by_user: false, reopens: true };

/// Queues a message to a session through the outbox (7a's delivery rule: never into a running turn).
async fn queue_message(
    st: &AppState,
    s: &Session,
    text: &str,
    dedupe: Option<String>,
    ticket_id: Option<String>,
    to: Delivery,
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
        by_user: to.by_user,
        pending_id: None,
        reopens: to.reopens,
    };
    st.store.queue_session_message(&m).await.map_err(err)?;
    Ok(())
}

fn outcome_output(a: &AgentStep) -> Value {
    json!({
        "outcome": a.outcome, "summary": a.summary, "comments": a.comments, "data": a.data.clone().unwrap_or_else(|| json!({})),
        "sessionId": a.session_id,
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
        change_request: None,
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

/// The branch of the attempt a session's workflow step is about (PDT-165): a new Reviewer or QA session has no attempt
/// of its own but reads that attempt's CI.
pub(crate) async fn step_attempt_branch(st: &AppState, s: &Session) -> Option<String> {
    let step = s.persona.as_ref()?.step_id.as_ref()?;
    let binding = st.store.get_agent_step(step).await.ok()??;
    let attempt = st.store.get_attempt(binding.attempt_id.as_deref()?).await.ok()??;
    Some(attempt.attempt.branch)
}

/// The waiting step of a binding, or why the binding is over.
async fn waiting_step(st: &AppState, a: &AgentStep) -> R<Step> {
    let step = st.store.get_step(&a.step_id).await.map_err(err)?.ok_or("the workflow step is gone")?;
    if step.status != StepStatus::Waiting {
        let mut done = a.clone();
        done.state = AgentStepState::Done;
        done.updated_at = now_ms();
        if let Err(e) = st.store.put_agent_step(&done).await {
            tracing::warn!(step = %done.step_id, error = %e, "could not write an agent step's binding");
        }
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
    a.background_since = None;
    a.background.clear();
    a.updated_at = now_ms();
    st.store.put_agent_step(&a).await.map_err(err)?;
    // A stalled step that gets its outcome after all (the agent went on by itself) needs nothing from the user now.
    for item in st.store.open_items_for_step(&a.step_id).await.map_err(err)? {
        if item.kind == InboxKind::Stalled {
            let resolution = json!({"outcome": outcome, "by": if by_user { "user" } else { "agent" }});
            st.store.end_inbox_item(&item.id, InboxStatus::Closed, &resolution, now_ms()).await.map_err(err)?;
        }
    }
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
    super::engine::complete_step(st, step, outcome, outcome_output(&a)).await?;
    close_step_session(st, &a, step);
    Ok(())
}

/// What becomes of a step's session once its outcome is in (PDT-183): a `new` one is stopped (it is never used again)
/// and an `own` one parked until its node's next pass, each once its turn has ended. A `reuse` one (the Coder's) goes on.
fn close_step_session(st: &AppState, a: &AgentStep, step: &Step) {
    let Some(sid) = a.session_id.clone() else { return };
    let Ok(c) = serde_json::from_value::<AgentConfig>(step.input.clone()) else { return };
    let close = match c.session {
        SessionChoice::New => Close::Stop,
        SessionChoice::Own => Close::Park,
        SessionChoice::Reuse => return,
    };
    let st = st.clone();
    tokio::spawn(async move { close_after_turn(&st, &sid, close, true).await });
}

/// A run ended (PDT-183): its steps' sessions that are still open are stopped, once their turn ends (at once when it
/// was cancelled). Kept: the attempt's own session (the Coder's), and the session of a timed-out step the run can still
/// be resumed from (its agent may yet hand in a late outcome, PDT-123).
pub(crate) async fn run_ended(st: &AppState, run: &pdt_core::workflow::api::Run) {
    let Ok(bindings) = st.store.agent_steps_of_run(&run.id).await else { return };
    let cancelled = run.status == pdt_core::workflow::api::RunStatus::Cancelled;
    let resumable = match st.store.list_steps(&run.id).await {
        Ok(steps) if !cancelled => resumable_step(&steps).map(|x| x.id.clone()),
        _ => None,
    };
    let mut seen = std::collections::HashSet::new();
    for a in bindings {
        let Some(sid) = a.session_id.clone() else { continue };
        if resumable.as_deref() == Some(a.step_id.as_str()) {
            seen.insert(sid);
            continue;
        }
        if !seen.insert(sid.clone()) {
            continue;
        }
        match st.store.get_session(&sid).await {
            Ok(Some(s)) if s.attempt_id.is_none() && s.status != SessionStatus::Finished => {}
            _ => continue,
        }
        let st = st.clone();
        tokio::spawn(async move { close_after_turn(&st, &sid, Close::Stop, !cancelled).await });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Close {
    /// Stopped and marked Finished.
    Stop,
    /// Its process closed; the next message reopens it.
    Park,
}

/// How long a session's turn may go on after its step ended before it is closed anyway.
const CLOSE_WAIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Closes a step's conversation, unless a step took it up meanwhile; once it is not at work (the turn that handed in
/// the outcome ended) when `after_turn`. A message that goes in first keeps it open until the turn it starts ends; one
/// sent after reopens it (PDT-188). A terminal is left alone.
async fn close_after_turn(st: &AppState, session_id: &str, close: Close, after_turn: bool) {
    let deadline = tokio::time::Instant::now() + CLOSE_WAIT;
    loop {
        while after_turn
            && crate::conversations::background(st, session_id).is_some_and(|b| b.busy)
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let Ok(Some(s)) = st.store.get_session(session_id).await else { return };
        if s.kind != SessionKind::AgentTui || s.mode != SessionMode::Conversation || s.status == SessionStatus::Finished
        {
            return;
        }
        if matches!(st.store.active_agent_step(session_id).await, Ok(Some(_))) {
            return;
        }
        tokio::time::sleep(st.conversations.close_pause).await;
        // Past the wait (or cancelled), it is closed whatever it is doing.
        let wait = after_turn && tokio::time::Instant::now() < deadline;
        let closed = match close {
            Close::Stop if wait => crate::sessions::stop_if_idle(st, session_id).await,
            Close::Stop => crate::sessions::stop(st, session_id).await.map(|()| true),
            Close::Park => Ok(crate::conversations::park_idle(st, session_id).await || !wait),
        };
        match closed {
            Ok(true) => {
                if close == Close::Stop {
                    tracing::info!(session = %session_id, "stopped a step's session after its outcome");
                }
                return;
            }
            // A message went in meanwhile: wait for its turn.
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(session = %session_id, error = %e.message(), "could not stop a step's session");
                return;
            }
        }
    }
}

/// The run a session's step belongs to, for its lock: its active binding's, else the step the session was started
/// for (whose binding doesn't name the session yet while the step is starting it), else its latest binding's.
pub(crate) async fn session_run(st: &AppState, session_id: &str) -> Option<String> {
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
    handed_in(st, &a, &input.outcome).await?;
    let step = waiting_step(st, &a).await?;
    if !input.comments.is_empty() {
        let cx = crate::mcp::tools::Cx { st, session: s, input: &crate::mcp::tools::NO_INPUT, confirmed: false };
        for c in &input.comments {
            crate::mcp::tools::add_review_comment(&cx, c).await?;
        }
    }
    a.comments = input.comments;
    a.data = input.data.map(Value::Object);
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

/// `submitted` hands the work on (PDT-161): refused while the attempt's worktree has uncommitted changes to tracked
/// files or commits its upstream doesn't have, since the next step (and CI) sees only what is pushed. A worktree git
/// can't read is let through.
async fn handed_in(st: &AppState, a: &AgentStep, outcome: &str) -> R<()> {
    if outcome != SUBMITTED {
        return Ok(());
    }
    let Some(attempt) = &a.attempt_id else { return Ok(()) };
    let Some(path) = st.store.get_attempt(attempt).await.map_err(err)?.and_then(|i| i.worktree_path) else {
        return Ok(());
    };
    let path = std::path::PathBuf::from(path);
    if !path.is_dir() {
        return Ok(());
    }
    let up = match tokio::task::spawn_blocking(move || pdt_git::unpushed(&path)).await.map_err(err)? {
        Ok(up) => up,
        Err(e) => {
            tracing::warn!(attempt = %attempt, error = %e, "could not read whether the worktree is pushed");
            return Ok(());
        }
    };
    match not_handed_in(&up.changed, up.ahead, up.upstream.as_deref(), up.unpushed_branch) {
        Some(why) => Err(why),
        None => Ok(()),
    }
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
    handed_in(st, &old, &input.outcome).await?;
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

/// `ask_user` (pdt-mcp): an Inbox question; a step waits for the answers instead of stalling.
pub(crate) async fn ask_user(st: &AppState, s: &Session, questions: Vec<Question>) -> R<Value> {
    let questions = inbox::valid_questions(questions)?;
    let first = questions[0].question.clone();
    let item = ask(st, s, InboxKind::Question, first, json!({"questions": questions})).await?;
    Ok(json!({"asked": true, "inboxItemId": item.id,
              "message": "Your questions are in the user's Inbox. Their answers will come as a message; end your turn now."}))
}

/// An asking tool's call from an agent that can't ask in its own prompt (PDT-256): an Inbox item with Allow and Deny,
/// which [`answer_confirm`] answers.
pub(crate) async fn ask_confirm(st: &AppState, s: &Session, tool: &str, input: &Value) -> R<InboxItem> {
    let info = st.registry().tool_infos().into_iter().find(|t| t.name == tool);
    let title = info.map(|t| t.title).filter(|t| !t.is_empty()).unwrap_or_else(|| tool.to_string());
    let agent = s.agent.map(|a| a.label());
    let body = json!({"tool": tool, "toolTitle": title, "input": st.store.redactor().json(input), "agent": agent,
                      "sessionTitle": s.title});
    ask(st, s, InboxKind::Confirm, title, body).await
}

/// The user's Allow or Deny of an asking tool's call (PDT-256): Allow runs it as the session's call with the user's
/// yes; the session hears the result either way.
pub(crate) async fn answer_confirm(st: &AppState, item: &InboxItem, choice: &str) -> R<Value> {
    let s = asker(st, item).await?;
    let tool = item.body.get("tool").and_then(Value::as_str).ok_or("the item names no tool")?;
    let title = item.body.get("toolTitle").and_then(Value::as_str).unwrap_or(tool);
    let allowed = match choice {
        inbox::ALLOW => true,
        inbox::DENY => false,
        other => return Err(format!("{other} is not a choice here")),
    };
    // Ended first, so a second click runs nothing.
    end_item(st, item, json!({"choice": choice})).await?;
    let (text, out) = if allowed {
        let input = item.body.get("input").cloned().unwrap_or_else(|| json!({}));
        let cx = crate::mcp::tools::Cx { st, session: &s, input: &input, confirmed: true };
        let r = st.store.redactor();
        match st.registry().call_tool(&cx, tool, input.clone()).await {
            Ok(v) => {
                let v = r.json(&v);
                (
                    format!("[PDT] The user allowed your {tool} call ({title}). PDT ran it; its result:\n{v}"),
                    json!({"allowed": true, "result": v}),
                )
            }
            Err(e) => {
                let e = r.string(&e);
                (
                    format!("[PDT] The user allowed your {tool} call ({title}), but it failed: {e}"),
                    json!({"allowed": true, "error": e}),
                )
            }
        }
    } else {
        (format!("[PDT] The user denied your {tool} call ({title}): don't make it."), json!({"allowed": false}))
    };
    let _locked = super::engine::lock_run(st, item.run_id.as_deref()).await;
    tell(st, item, &s, &text).await?;
    Ok(out)
}

/// Opens an item a session waits on the user for (a question, a secret request, an asking tool's call), replacing its
/// step's open one of that kind; the step is asking until it is answered.
pub(crate) async fn ask(st: &AppState, s: &Session, kind: InboxKind, title: String, body: Value) -> R<InboxItem> {
    let run = session_run(st, &s.id).await;
    let _locked = super::engine::lock_run(st, run.as_deref()).await;
    let binding = st.store.active_agent_step(&s.id).await.map_err(err)?;
    if let Some(a) = &binding {
        // A new question replaces the step's open one.
        for old in st.store.open_items_for_step(&a.step_id).await.map_err(err)? {
            if old.kind == kind {
                st.store
                    .end_inbox_item(&old.id, InboxStatus::Closed, &json!({"replaced": true}), now_ms())
                    .await
                    .map_err(err)?;
            }
        }
    }
    let ticket_id = crate::mcp::tools::session_ticket(st, s).await;
    let item = InboxItem {
        id: uuid::Uuid::now_v7().to_string(),
        kind,
        group: if kind == InboxKind::Confirm { InboxGroup::Approval } else { InboxGroup::Question },
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
        change_request: None,
        title,
        body,
        choices: match kind {
            InboxKind::Confirm => {
                vec![InboxChoice::new(inbox::ALLOW, "Allow", true), InboxChoice::new(inbox::DENY, "Deny", false)]
            }
            _ => vec![],
        },
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
    Ok(item)
}

/// A session's turn ended (a conversation's `Turn` end, a terminal going idle): a working step with no outcome is
/// nudged, then stalled; at `maxTurns` it times out (spec §4.4). A conversation still running background tasks it
/// started waits on them instead (PDT-142): Claude takes each one up when it finishes, in a turn of its own.
pub(crate) async fn turn_ended(st: &AppState, session_id: &str) {
    let run = session_run(st, session_id).await;
    let _locked = super::engine::lock_run(st, run.as_deref()).await;
    // A session being removed ends its turn as it closes: not a turn without an outcome (the sweep stalls its step as
    // removed).
    match st.store.get_session(session_id).await {
        Ok(Some(s)) if s.mode != SessionMode::Conversation || crate::conversations::is_loaded(st, session_id) => {}
        Ok(_) => return,
        Err(_) => {}
    }
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
    let tasks = crate::conversations::background(st, session_id).map(|b| b.tasks).unwrap_or_default();
    if !tasks.is_empty() {
        let now = now_ms();
        a.background_since.get_or_insert(now);
        a.background = tasks.into_iter().map(|t| t.description).collect();
        a.updated_at = now;
        if let Err(e) = st.store.put_agent_step(&a).await {
            tracing::warn!(step = %a.step_id, error = %e, "could not write an agent step's binding");
        }
        tracing::info!(step = %a.step_id, tasks = a.background.len(), "an agent step waits on background tasks");
        return;
    }
    outcome_less_turn(st, a, &step, session_id).await;
}

/// A turn ended without an outcome, with no background task left to wait on: it counts, then a nudge, then a stall.
async fn outcome_less_turn(st: &AppState, mut a: AgentStep, step: &Step, session_id: &str) {
    a.background_since = None;
    a.background.clear();
    a.turns += 1;
    a.updated_at = now_ms();
    if a.turns >= a.max_turns {
        a.state = AgentStepState::Done;
        if let Err(e) = st.store.put_agent_step(&a).await {
            tracing::warn!(step = %a.step_id, error = %e, "could not write an agent step's binding");
        }
        let output = json!({"timedOut": true, "reason": format!("{} turns ended without an outcome", a.turns),
                            "sessionId": a.session_id, "persona": a.persona_name});
        if let Err(e) = super::engine::complete_step(st, step, TIMEOUT, output).await {
            tracing::warn!(step = %a.step_id, error = %e, "could not time out an agent step");
        }
        return;
    }
    if a.nudges < a.auto_nudges {
        a.nudges += 1;
        if let Err(e) = st.store.put_agent_step(&a).await {
            tracing::warn!(step = %a.step_id, error = %e, "could not write an agent step's binding");
        }
        if let Ok(Some(s)) = st.store.get_session(session_id).await
            && let Err(e) = queue_message(st, &s, &nudge_message(&a.outcomes), None, a.ticket_id.clone(), PDT).await
        {
            tracing::warn!(session = %session_id, error = %e, "could not nudge an agent step's session");
        }
        return;
    }
    stall(st, a, "Its turn ended without an outcome.").await;
}

/// The background tasks a step waited on all finished and its session stayed quiet (no turn took them up): its last
/// turn ended without an outcome after all (PDT-142).
async fn background_settled(st: &AppState, session_id: &str, step_id: &str) {
    let run = session_run(st, session_id).await;
    let _locked = super::engine::lock_run(st, run.as_deref()).await;
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

/// Marks a binding stalled and puts it in the Inbox (Nudge / Take over).
async fn stall(st: &AppState, a: AgentStep, why: &str) {
    stall_item(st, a, why, None).await;
}

/// The step's first message never reached its session (PDT-144): stalled, quoting it, with Resume with this message.
async fn stall_undelivered(st: &AppState, a: AgentStep, text: &str) {
    let quote: String = text.trim().chars().take(QUOTE_CHARS).collect();
    let ellipsis = if text.trim().chars().count() > QUOTE_CHARS { "…" } else { "" };
    stall_item(st, a, &format!("Its message was never delivered: {quote}{ellipsis}"), Some(text)).await;
}

async fn stall_item(st: &AppState, mut a: AgentStep, why: &str, undelivered: Option<&str>) {
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
    let mut choices = vec![];
    if undelivered.is_some() {
        choices.push(InboxChoice::new(inbox::RESEND, "Resume with this message", true));
    }
    choices.push(InboxChoice::new(inbox::NUDGE, "Nudge", undelivered.is_none()));
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
        change_request: None,
        title: format!("{} stalled", a.persona_name),
        body: json!({"why": why, "outcomes": a.outcomes, "turns": a.turns, "nudges": a.nudges, "message": undelivered}),
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

/// Answers a stalled or loop-limit item, or a question (from the Inbox).
pub(crate) async fn answer_item(st: &AppState, item: &InboxItem, answer: &inbox::InboxAnswer) -> R<Value> {
    match item.kind {
        InboxKind::Question => answer_question(st, item, answer).await,
        InboxKind::Secret => crate::vault::answer_request(st, item, answer).await,
        InboxKind::Stalled => {
            let choice = answer.choice.as_deref().ok_or("choose Nudge or an outcome")?;
            let _locked = super::engine::lock_run(st, item.run_id.as_deref()).await;
            let step_id = item.step_id.as_deref().ok_or("the item has no step")?;
            let mut a = st.store.get_agent_step(step_id).await.map_err(err)?.ok_or("the step is gone")?;
            let step = waiting_step(st, &a).await?;
            if choice == inbox::NUDGE || choice == inbox::RESEND {
                let s = st
                    .store
                    .get_session(a.session_id.as_deref().unwrap_or_default())
                    .await
                    .map_err(err)?
                    .ok_or("the step's session is gone: take it over instead")?;
                if s.status != SessionStatus::Running && s.mode != SessionMode::Conversation {
                    return Err("the step's terminal session has stopped: restart it, or take the step over".into());
                }
                if choice == inbox::RESEND {
                    // The step's undelivered first message, again, as the user's (PDT-144).
                    let text = item.body.get("message").and_then(Value::as_str).ok_or("the item has no message")?;
                    queue_message(st, &s, text, Some(prompt_key(&a.step_id)), a.ticket_id.clone(), USER).await?;
                } else {
                    let text = answer
                        .text
                        .as_deref()
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(String::from)
                        .unwrap_or_else(|| nudge_message(&a.outcomes));
                    queue_message(st, &s, &text, None, a.ticket_id.clone(), USER).await?;
                    a.nudges += 1;
                }
                a.state = AgentStepState::Working;
                // Its wait on background tasks starts over if the nudged turn ends with some still running.
                a.background_since = None;
                a.background.clear();
                a.updated_at = now_ms();
                st.store.put_agent_step(&a).await.map_err(err)?;
                end_item(st, item, json!({"choice": choice})).await?;
                return Ok(if choice == inbox::RESEND { json!({"resent": true}) } else { json!({"nudged": true}) });
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
        InboxKind::Connect => Err("connect GitHub in Settings › Plugins › GitHub".into()),
        InboxKind::McpSignIn => Err("sign in to the MCP server again in Settings › MCP servers".into()),
        // Answered by `inbox::answer`, which knows whether the user's own click sent it.
        InboxKind::Admit => Err("approve or reject a triaged ticket from PDT's Inbox".into()),
        InboxKind::Confirm => Err("allow or deny an agent's call from PDT's Inbox".into()),
        InboxKind::Proposal => Err("accept or discard a workflow proposal from PDT's Inbox".into()),
        InboxKind::Storage => Err("PDT can't save changes: it waits on no answer, and goes once saving works".into()),
        InboxKind::LowDisk => Err("low disk space waits on no answer: it goes once there is space again".into()),
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
    reply(st, item, &text, json!({"answers": answer.answers, "text": answer.text})).await?;
    Ok(json!({"answered": true}))
}

/// Ends an item the asking session waits on with `resolution`, and sends it `text`; its step works again.
pub(crate) async fn reply(st: &AppState, item: &InboxItem, text: &str, resolution: Value) -> R<()> {
    let s = asker(st, item).await?;
    let _locked = super::engine::lock_run(st, item.run_id.as_deref()).await;
    end_item(st, item, resolution).await?;
    tell(st, item, &s, text).await
}

/// Sends the asking session `text` about its ended item; its step works again. The caller holds the run's lock.
async fn tell(st: &AppState, item: &InboxItem, s: &Session, text: &str) -> R<()> {
    queue_message(st, s, text, None, item.ticket_id.clone(), USER).await?;
    if let Some(step) = &item.step_id
        && let Ok(Some(mut a)) = st.store.get_agent_step(step).await
        && a.state == AgentStepState::Asking
    {
        a.state = AgentStepState::Working;
        a.updated_at = now_ms();
        st.store.put_agent_step(&a).await.map_err(err)?;
    }
    Ok(())
}

/// The session an item asks for, while it can still be answered.
pub(crate) async fn asker(st: &AppState, item: &InboxItem) -> R<Session> {
    let sid = item.session_id.as_deref().ok_or("the question has no session to answer")?;
    let s = st.store.get_session(sid).await.map_err(err)?.ok_or("the asking session is gone")?;
    if s.mode != SessionMode::Conversation && s.status != SessionStatus::Running {
        return Err("the asking terminal session has stopped; restart it first".into());
    }
    Ok(s)
}
