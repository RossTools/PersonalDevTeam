//! The durable workflow engine (Phase 7a spec §4): matches logged events to enabled triggers, starts runs, executes
//! their steps one by one (each written before and after it runs, so a restart re-runs at most the step that was
//! running), handles waits, retries, error paths, limits and sub-workflows, and dry-runs documents in memory.

use super::catalog::{Category, Outcome, StepCx, catalog, parse};
use super::nodes::trigger::{MANUAL, SharedTrigger};
use crate::routes::{AppState, now_ms};
use pdt_core::workflow::api::{LogEvent, Run, RunStatus, Step, StepStatus, Workflow};
use pdt_core::workflow::cond::all;
use pdt_core::workflow::doc::{ERROR_PORT, WorkflowDoc};
use pdt_core::workflow::refs::{resolve, resolve_keeping};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use ts_rs::TS;

type R<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The trigger as runs see it (`{{trigger.…}}`): the payload with the event's type and id.
pub(crate) fn trigger_value(e: &LogEvent) -> Value {
    let mut v = match &e.payload {
        Value::Object(_) => e.payload.clone(),
        other => json!({"payload": other}),
    };
    v["type"] = json!(e.kind);
    v["eventId"] = json!(e.id);
    v
}

/// The run's context for references: `trigger`, `steps.<node>` (latest output), `state`, `run`, `workflow`.
fn context(wf: &Workflow, run: &Run, steps: &[Step], state: serde_json::Map<String, Value>, dry_run: bool) -> Value {
    let mut outputs = serde_json::Map::new();
    for s in steps.iter().filter(|s| s.status == StepStatus::Succeeded) {
        outputs.insert(s.node_id.clone(), s.output.clone());
    }
    let state: serde_json::Map<String, Value> =
        state.into_iter().filter(|(k, _)| !k.starts_with(super::nodes::schedule::RESERVED_STATE)).collect();
    json!({
        "trigger": run.trigger,
        "steps": outputs,
        "state": state,
        "run": {"id": run.id, "workflowId": run.workflow_id, "dryRun": dry_run},
        "workflow": {"id": wf.id, "name": wf.name},
    })
}

pub(crate) fn new_step(run_id: &str, seq: i64, node_id: &str, kind: &str) -> Step {
    Step {
        id: uuid::Uuid::now_v7().to_string(),
        run_id: run_id.into(),
        seq,
        node_id: node_id.into(),
        kind: kind.into(),
        status: StepStatus::Pending,
        input: Value::Null,
        output: Value::Null,
        port: None,
        attempts: 0,
        error: None,
        wake_at: None,
        wait: Value::Null,
        started_at: None,
        ended_at: None,
    }
}

/// The steps an edge from `node`'s `port` leads to, numbered from `seq`.
fn successors(doc: &WorkflowDoc, run_id: &str, node: &str, port: &str, seq: i64) -> Vec<Step> {
    doc.next(node, port)
        .into_iter()
        .enumerate()
        .filter_map(|(i, to)| {
            let n = doc.node(to)?;
            Some(new_step(run_id, seq + i64::try_from(i).unwrap_or(0), &n.id, &n.kind))
        })
        .collect()
}

/// Starts a run of `wf` from `trigger_node`. None when its run key already ran.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_run(
    st: &AppState,
    wf: &Workflow,
    doc: &WorkflowDoc,
    trigger_node: &str,
    trigger: Value,
    event_id: Option<String>,
    run_key: String,
    parent: Option<(&str, &str)>,
) -> R<Option<String>> {
    let node = doc.node(trigger_node).ok_or_else(|| format!("no node {trigger_node}"))?;
    let now = now_ms();
    let s = |k: &str| trigger.get(k).and_then(|v| v.as_str()).map(String::from);
    let run = Run {
        id: uuid::Uuid::now_v7().to_string(),
        workflow_id: wf.id.clone(),
        workflow_name: wf.name.clone(),
        version: wf.version,
        status: RunStatus::Running,
        run_key,
        trigger_event_id: event_id,
        trigger_node: trigger_node.into(),
        project_id: s("projectId").or_else(|| wf.project_id.clone()),
        ticket_id: s("ticketId"),
        attempt_id: s("attemptId"),
        trigger: trigger.clone(),
        parent_run_id: parent.map(|p| p.0.to_string()),
        parent_step_id: parent.map(|p| p.1.to_string()),
        error: None,
        output: Value::Null,
        wake_at: None,
        started_at: now,
        ended_at: None,
    };
    let mut first = new_step(&run.id, 1, &node.id, &node.kind);
    first.status = StepStatus::Succeeded;
    first.output = trigger;
    first.port = Some("out".into());
    first.attempts = 1;
    first.started_at = Some(now);
    first.ended_at = Some(now);
    let mut steps = vec![first];
    steps.extend(successors(doc, &run.id, &node.id, "out", 2));
    let started = st.store.start_workflow_run(&run, &steps).await.map_err(err)?;
    if started {
        tracing::info!(workflow = %wf.name, run = %run.id, trigger = %trigger_node, "workflow run started");
    }
    Ok(started.then_some(run.id))
}

async fn load(st: &AppState, run: &Run) -> R<(Workflow, WorkflowDoc)> {
    let wf = st.store.get_workflow(&run.workflow_id).await.map_err(err)?.ok_or("the workflow is gone")?;
    let doc = st.store.workflow_doc(&wf.id, run.version).await.map_err(err)?.ok_or("the workflow version is gone")?;
    Ok((wf, doc))
}

/// Matches new events to enabled triggers (starting runs) and to steps waiting for events.
async fn match_events(st: &AppState) {
    let events = match st.store.unprocessed_events(200).await {
        Ok(e) if !e.is_empty() => e,
        Ok(_) => return,
        Err(e) => return tracing::warn!(error = %e, "could not read new events"),
    };
    let mut enabled: Vec<(Workflow, WorkflowDoc)> = vec![];
    for wf in st.store.enabled_workflows().await.unwrap_or_default() {
        if let Ok(Some(doc)) = st.store.workflow_doc(&wf.id, wf.version).await {
            enabled.push((wf, doc));
        }
    }
    for e in events {
        let trigger = trigger_value(&e);
        let ctx = json!({"trigger": trigger});
        for (wf, doc) in &enabled {
            for node in &doc.nodes {
                let Some(kind) = catalog().get(&node.kind).filter(|k| k.event_type() == Some(e.kind.as_str())) else {
                    continue;
                };
                let Ok(config) = catalog().current_config(node) else { continue };
                if !kind.accepts(&config, wf, node, &e) {
                    continue;
                }
                let Ok(cfg) = parse::<SharedTrigger>(&config) else { continue };
                if !cfg.projects.is_empty() && !e.project_id.as_ref().is_some_and(|p| cfg.projects.contains(p)) {
                    continue;
                }
                if !all(&cfg.conditions, &ctx) {
                    continue;
                }
                let key = cfg
                    .run_key
                    .map(|k| resolve(&Value::String(k), &ctx))
                    .and_then(|v| v.as_str().map(String::from).or_else(|| (!v.is_null()).then(|| v.to_string())))
                    .unwrap_or_else(|| format!("{}:{}", node.id, e.id));
                if let Err(x) = start_run(st, wf, doc, &node.id, trigger.clone(), Some(e.id.clone()), key, None).await {
                    tracing::warn!(workflow = %wf.name, error = %x, "could not start a workflow run");
                }
            }
        }
        // Steps waiting for this event.
        for (_, step) in st.store.steps_waiting_for_events().await.unwrap_or_default() {
            let Some(w) = step.wait.get("event") else { continue };
            if w.get("type").and_then(|t| t.as_str()) != Some(e.kind.as_str()) {
                continue;
            }
            let conds: Vec<pdt_core::workflow::cond::Condition> =
                w.get("where").cloned().and_then(|c| serde_json::from_value(c).ok()).unwrap_or_default();
            if all(&conds, &json!({"event": trigger}))
                && let Err(x) = complete_step(st, &step, "event", trigger.clone()).await
            {
                tracing::warn!(step = %step.id, error = %x, "could not resume a waiting step");
            }
        }
        if let Err(x) = st.store.mark_event_processed(e.seq, now_ms()).await {
            tracing::warn!(error = %x, "could not mark an event processed");
        }
    }
}

/// Finishes a waiting (or running) step by `port` with `output`: queues what it leads to and wakes its run.
pub(crate) async fn complete_step(st: &AppState, step: &Step, port: &str, output: Value) -> R<()> {
    let run = st.store.get_workflow_run(&step.run_id).await.map_err(err)?.ok_or("the run is gone")?;
    if run.status.is_finished() {
        return Ok(());
    }
    let (_, doc) = load(st, &run).await?;
    let steps = st.store.list_steps(&run.id).await.map_err(err)?;
    let next_seq = steps.iter().map(|s| s.seq).max().unwrap_or(0) + 1;
    let mut s = step.clone();
    s.status = StepStatus::Succeeded;
    s.port = Some(port.into());
    s.output = output;
    s.wake_at = None;
    s.ended_at = Some(now_ms());
    st.store.put_step(&s).await.map_err(err)?;
    end_step_extras(st, &s.id).await;
    // An agent step that timed out tells the user, who can resume it (PDT-123).
    if s.kind == super::nodes::agent::KIND && port == pdt_core::workflow::agent::TIMEOUT {
        super::agent::timed_out(st, &s, &run).await;
    }
    for n in successors(&doc, &run.id, &s.node_id, port, next_seq) {
        st.store.put_step(&n).await.map_err(err)?;
    }
    st.store.wake_workflow_run(&run.id, now_ms()).await.map_err(err)
}

/// What waits on a step outside the engine ends with it: its open Inbox items close, its agent binding is done.
async fn end_step_extras(st: &AppState, step_id: &str) {
    if let Err(e) = st.store.close_inbox_items(Some(step_id), None, now_ms()).await {
        tracing::warn!(step = %step_id, error = %e, "could not close a step's Inbox items");
    }
    if let Ok(Some(mut a)) = st.store.get_agent_step(step_id).await
        && a.state != pdt_core::workflow::agent::AgentStepState::Done
    {
        a.state = pdt_core::workflow::agent::AgentStepState::Done;
        a.updated_at = now_ms();
        let _ = st.store.put_agent_step(&a).await;
    }
}

/// Ends a run: leftover steps are skipped, a parent waiting on it resumes.
async fn finish_run(st: &AppState, run: &mut Run, status: RunStatus, output: Value, error: Option<String>) -> R<()> {
    let now = now_ms();
    for mut s in st.store.list_steps(&run.id).await.map_err(err)? {
        if matches!(s.status, StepStatus::Pending | StepStatus::Running | StepStatus::Waiting) {
            s.status = StepStatus::Skipped;
            s.ended_at = Some(now);
            st.store.put_step(&s).await.map_err(err)?;
            end_step_extras(st, &s.id).await;
        }
    }
    let scratch = super::nodes::command::scratch_dir(&st.data_dir, &run.id);
    if tokio::fs::try_exists(&scratch).await.unwrap_or(false) {
        let _ = tokio::fs::remove_dir_all(&scratch).await;
    }
    // A cancelled run waits on nobody: its items close, a timed-out step's too (PDT-123).
    if status == RunStatus::Cancelled
        && let Err(e) = st.store.close_inbox_items(None, Some(&run.id), now).await
    {
        tracing::warn!(run = %run.id, error = %e, "could not close a run's Inbox items");
    }
    run.status = status;
    run.output = output;
    run.error = error;
    run.wake_at = None;
    run.ended_at = Some(now);
    st.store.update_workflow_run(run).await.map_err(err)?;
    tracing::info!(workflow = %run.workflow_name, run = %run.id, status = status.as_str(), error = ?run.error, "workflow run ended");
    if status == RunStatus::Failed {
        crate::notifications::run_failed(st, run, run.error.as_deref()).await;
    }
    if let Some(pid) = &run.parent_step_id
        && let Some(parent) = st.store.get_step(pid).await.map_err(err)?
        && parent.status == StepStatus::Waiting
    {
        let port = if status == RunStatus::Failed || status == RunStatus::Cancelled { "failed" } else { "done" };
        Box::pin(complete_step(st, &parent, port, run.output.clone())).await?;
    }
    Ok(())
}

/// Whether a finished run had nothing to report: no action did anything that wasn't quiet.
fn is_quiet(steps: &[Step]) -> bool {
    !steps.iter().any(|s| {
        s.status == StepStatus::Succeeded
            && catalog().spec(&s.kind).is_some_and(|k| matches!(k.category, Category::Action | Category::Agent))
            && s.output.get("quiet").and_then(|q| q.as_bool()) != Some(true)
    })
}

/// Runs a run's steps until it waits or ends.
async fn advance(st: &AppState, mut run: Run) -> R<()> {
    let (wf, doc) = match load(st, &run).await {
        Ok(x) => x,
        Err(e) => return finish_run(st, &mut run, RunStatus::Failed, Value::Null, Some(e)).await,
    };
    for _ in 0..10_000 {
        let now = now_ms();
        let steps = st.store.list_steps(&run.id).await.map_err(err)?;
        if now - run.started_at > i64::try_from(doc.settings.max_runtime_ms).unwrap_or(i64::MAX) {
            let e = format!("the run took longer than {} ms", doc.settings.max_runtime_ms);
            return finish_run(st, &mut run, RunStatus::Failed, Value::Null, Some(e)).await;
        }
        // Timers that came due: a retry runs again; any other wait takes its timeout port.
        let mut fired = false;
        for s in steps.iter().filter(|s| s.status == StepStatus::Waiting && s.wake_at.is_some_and(|t| t <= now)) {
            fired = true;
            if s.wait.get("retry").is_some() {
                let mut s = s.clone();
                s.status = StepStatus::Pending;
                s.wake_at = None;
                st.store.put_step(&s).await.map_err(err)?;
            } else {
                match s.wait.get("timeoutPort").and_then(|p| p.as_str()) {
                    Some(port) => complete_step(st, s, port, json!({"timedOut": true})).await?,
                    None => {
                        let e = format!("{} timed out", s.node_id);
                        return finish_run(st, &mut run, RunStatus::Failed, Value::Null, Some(e)).await;
                    }
                }
            }
        }
        if fired {
            continue;
        }
        let Some(step) = steps.iter().find(|s| matches!(s.status, StepStatus::Pending | StepStatus::Running)).cloned()
        else {
            let waiting: Vec<&Step> = steps.iter().filter(|s| s.status == StepStatus::Waiting).collect();
            if !waiting.is_empty() {
                run.status = RunStatus::Waiting;
                run.wake_at = waiting.iter().filter_map(|s| s.wake_at).min();
                return st.store.update_workflow_run(&run).await.map_err(err);
            }
            let output = steps.iter().rev().find(|s| s.status == StepStatus::Succeeded).map(|s| s.output.clone());
            let status = if is_quiet(&steps) { RunStatus::Quiet } else { RunStatus::Succeeded };
            return finish_run(st, &mut run, status, output.unwrap_or(Value::Null), None).await;
        };
        let executed = steps.iter().filter(|s| s.attempts > 0).count();
        if executed >= usize::try_from(doc.settings.max_steps).unwrap_or(usize::MAX) {
            let e = format!("the run took more than {} steps", doc.settings.max_steps);
            return finish_run(st, &mut run, RunStatus::Failed, Value::Null, Some(e)).await;
        }
        if execute(st, &wf, &doc, &mut run, step, &steps).await? {
            return Ok(());
        }
    }
    Ok(())
}

/// Executes one step. True when the run ended.
async fn execute(
    st: &AppState,
    wf: &Workflow,
    doc: &WorkflowDoc,
    run: &mut Run,
    mut step: Step,
    steps: &[Step],
) -> R<bool> {
    let fail = |e: String| e;
    let Some(node) = doc.node(&step.node_id) else {
        let e = format!("node {} is not in the workflow", step.node_id);
        finish_run(st, run, RunStatus::Failed, Value::Null, Some(fail(e))).await?;
        return Ok(true);
    };
    let kind = catalog().get(&node.kind);
    let visits = steps
        .iter()
        .filter(|s| {
            s.node_id == node.id && s.id != step.id && matches!(s.status, StepStatus::Succeeded | StepStatus::Failed)
        })
        .count();
    let config = catalog().current_config(node);
    let (kind, config) = match (kind, config) {
        (Some(k), Ok(c)) if visits < usize::try_from(node.max_visits()).unwrap_or(usize::MAX) => (k, c),
        (_, Err(e)) => return fail_step(st, doc, run, step, e).await,
        (None, _) => return fail_step(st, doc, run, step, format!("no node kind is called {}", node.kind)).await,
        _ => {
            let e = format!("loop limit: {} ran {visits} times in this run", node.id);
            step.attempts = u32::MAX / 2;
            return fail_step(st, doc, run, step, e).await;
        }
    };
    let state = st.store.workflow_state(&wf.id).await.map_err(err)?;
    let ctx = context(wf, run, steps, state, false);
    let input = resolve_keeping(&config, &ctx, Some("event"));
    let now = now_ms();
    step.status = StepStatus::Running;
    step.attempts += 1;
    step.input = input.clone();
    step.started_at = Some(now);
    step.error = None;
    st.store.put_step(&step).await.map_err(err)?;
    let limit = std::time::Duration::from_millis(node.timeout_ms.unwrap_or_else(|| kind.default_timeout_ms()));
    let result = {
        let cx = StepCx { st, workflow: wf, run, step: &step, node, ctx: &ctx, dry_run: false };
        match tokio::time::timeout(limit, kind.run(&cx, input)).await {
            Ok(r) => r,
            Err(_) => Err(format!("timed out after {} ms", limit.as_millis())),
        }
    };
    match result {
        Ok(Outcome::Next { port, output }) => {
            complete_step(st, &step, &port, output).await?;
            Ok(false)
        }
        Ok(Outcome::Wait { mut wait, until, timeout_port, output }) => {
            if let (Some(p), Value::Object(m)) = (timeout_port, &mut wait) {
                m.insert("timeoutPort".into(), json!(p));
            }
            step.status = StepStatus::Waiting;
            step.wait = wait;
            step.wake_at = until;
            step.output = output;
            st.store.put_step(&step).await.map_err(err)?;
            Ok(false)
        }
        Ok(Outcome::End { quiet, output }) => {
            step.status = StepStatus::Succeeded;
            step.output = output.clone();
            step.ended_at = Some(now_ms());
            st.store.put_step(&step).await.map_err(err)?;
            // An end node says it: quiet, or not.
            let status = if quiet { RunStatus::Quiet } else { RunStatus::Succeeded };
            finish_run(st, run, status, output, None).await?;
            Ok(true)
        }
        Err(e) => fail_step(st, doc, run, step, e).await,
    }
}

/// A failed execution: retried while the node allows, then its error port, else the run fails. True when it ended.
async fn fail_step(st: &AppState, doc: &WorkflowDoc, run: &mut Run, mut step: Step, e: String) -> R<bool> {
    let node = doc.node(&step.node_id);
    let retry = node.and_then(|n| n.retry);
    tracing::info!(workflow = %run.workflow_name, run = %run.id, node = %step.node_id, error = %e, "a workflow step failed");
    step.error = Some(e.clone());
    if let Some(r) = retry
        && step.attempts <= r.max
    {
        step.status = StepStatus::Waiting;
        step.wait = json!({"retry": true});
        step.wake_at = Some(now_ms() + i64::try_from(r.delay_ms(step.attempts)).unwrap_or(0));
        st.store.put_step(&step).await.map_err(err)?;
        return Ok(false);
    }
    if node.is_some() && doc.has_edge_from(&step.node_id, ERROR_PORT) {
        st.store.put_step(&step).await.map_err(err)?;
        complete_step(st, &step, ERROR_PORT, json!({"error": e})).await?;
        return Ok(false);
    }
    step.status = StepStatus::Failed;
    step.ended_at = Some(now_ms());
    st.store.put_step(&step).await.map_err(err)?;
    let msg = format!("{}: {e}", step.node_id);
    finish_run(st, run, RunStatus::Failed, Value::Null, Some(msg)).await?;
    Ok(true)
}

/// Advances every run that can move.
async fn advance_all(st: &AppState) {
    let runs = match st.store.runnable_workflow_runs(now_ms()).await {
        Ok(r) => r,
        Err(e) => return tracing::warn!(error = %e, "could not list workflow runs"),
    };
    for run in runs {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let id = run.id.clone();
        if let Err(e) = advance(st, run).await {
            tracing::warn!(run = %id, error = %e, "a workflow run could not go on");
        }
    }
}

/// One engine tick: install built-ins (once), match new events, run steps, deliver session messages, run what they
/// woke. pdtd runs it every 2 s; tests call it.
pub async fn run_workflows(st: &AppState) {
    let _engine = st.engine_lock.lock().await;
    super::builtins::ensure_installed(st).await;
    crate::personas::ensure_builtins(st).await;
    super::nodes::schedule::fire_schedules(st).await;
    match_events(st).await;
    crate::scheduler::admit_waiting(st).await;
    advance_all(st).await;
    super::outbox::deliver(st).await;
    super::agent::sweep(st).await;
    advance_all(st).await;
}

// ---- commands used by the API ----

/// "Run now": a run of the workflow's `manual` trigger with `input`.
pub(crate) async fn start_manual(
    st: &AppState,
    wf: &Workflow,
    input: Value,
    run_key: Option<String>,
) -> R<Option<String>> {
    let doc = st.store.workflow_doc(&wf.id, wf.version).await.map_err(err)?.ok_or("the workflow has no document")?;
    let node = doc.nodes.iter().find(|n| n.kind == MANUAL).ok_or("the workflow has no Run now trigger")?;
    let trigger = if input.is_null() { json!({}) } else { input };
    let configured =
        catalog().current_config(node).ok().and_then(|c| c.get("runKey").and_then(|k| k.as_str()).map(String::from));
    let key = run_key
        .or_else(|| {
            configured.and_then(|k| resolve(&Value::String(k), &json!({"trigger": trigger})).as_str().map(String::from))
        })
        .unwrap_or_else(|| format!("manual:{}", uuid::Uuid::now_v7()));
    let _engine = st.engine_lock.lock().await;
    start_run(st, wf, &doc, &node.id, trigger, None, key, None).await
}

/// Starts a sub-workflow for a `workflow.call` step (once per step).
pub(crate) async fn start_child(cx: &StepCx<'_>, workflow_id: &str, input: Value) -> R<String> {
    let st = cx.st;
    let wf =
        st.store.get_workflow(workflow_id).await.map_err(err)?.ok_or_else(|| format!("no workflow {workflow_id}"))?;
    let doc = st.store.workflow_doc(&wf.id, wf.version).await.map_err(err)?.ok_or("the workflow has no document")?;
    let node =
        doc.nodes.iter().find(|n| n.kind == MANUAL).ok_or_else(|| format!("{} has no Run now trigger", wf.name))?;
    let key = format!("call:{}", cx.step.id);
    let trigger = if input.is_null() { json!({}) } else { input };
    match start_run(st, &wf, &doc, &node.id, trigger, None, key.clone(), Some((&cx.run.id, &cx.step.id))).await? {
        Some(id) => Ok(id),
        None => Ok(st.store.workflow_run_by_key(&wf.id, &key).await.map_err(err)?.map(|r| r.id).unwrap_or_default()),
    }
}

/// Retries a failed run from its failed step.
pub(crate) async fn retry_run(st: &AppState, run_id: &str) -> R<()> {
    let _engine = st.engine_lock.lock().await;
    let mut run = st.store.get_workflow_run(run_id).await.map_err(err)?.ok_or("no such run")?;
    if run.status != RunStatus::Failed {
        return Err("only a failed run can be retried".into());
    }
    let steps = st.store.list_steps(run_id).await.map_err(err)?;
    let mut failed =
        steps.into_iter().rev().find(|s| s.status == StepStatus::Failed).ok_or("the run has no failed step")?;
    failed.status = StepStatus::Pending;
    failed.error = None;
    failed.attempts = 0;
    failed.ended_at = None;
    st.store.put_step(&failed).await.map_err(err)?;
    run.status = RunStatus::Running;
    run.error = None;
    run.ended_at = None;
    st.store.update_workflow_run(&run).await.map_err(err)
}

/// Takes a run back to `step`, the agent step it timed out or stopped at (PDT-123), so it can go on from there: what
/// the run still had to do after it (its `timeout` path) is skipped, the step's open Inbox items end with
/// `resolution`, and the run runs again. Callers hold the engine lock and add the step that goes on.
pub(crate) async fn reopen_at(st: &AppState, step: &Step, resolution: Value) -> R<Run> {
    let mut run = st.store.get_workflow_run(&step.run_id).await.map_err(err)?.ok_or("the run is gone")?;
    let steps = st.store.list_steps(&run.id).await.map_err(err)?;
    if pdt_core::workflow::agent::resumable_step(&steps).map(|s| s.id.as_str()) != Some(step.id.as_str()) {
        return Err("the run has moved on from this step".into());
    }
    if run.parent_step_id.is_some() && run.status.is_finished() {
        return Err("this run's caller has gone on without it".into());
    }
    let now = now_ms();
    for mut s in steps.into_iter().filter(|s| s.seq > step.seq) {
        if matches!(s.status, StepStatus::Pending | StepStatus::Running | StepStatus::Waiting) {
            s.status = StepStatus::Skipped;
            s.ended_at = Some(now);
            st.store.put_step(&s).await.map_err(err)?;
            end_step_extras(st, &s.id).await;
        }
    }
    for item in st.store.open_items_for_step(&step.id).await.map_err(err)? {
        st.store
            .end_inbox_item(&item.id, pdt_core::inbox::InboxStatus::Resolved, &resolution, now)
            .await
            .map_err(err)?;
    }
    run.status = RunStatus::Running;
    run.error = None;
    run.output = Value::Null;
    run.wake_at = None;
    run.ended_at = None;
    st.store.update_workflow_run(&run).await.map_err(err)?;
    tracing::info!(workflow = %run.workflow_name, run = %run.id, node = %step.node_id, "a workflow run goes on from a stopped step");
    Ok(run)
}

/// The next step number of a run.
pub(crate) async fn next_seq(st: &AppState, run_id: &str) -> R<i64> {
    Ok(st.store.list_steps(run_id).await.map_err(err)?.iter().map(|s| s.seq).max().unwrap_or(0) + 1)
}

/// Resumes a run from the agent step it timed out or stopped at (PDT-123): the node runs again in the same run, so
/// with the same trigger, ticket and attempt, and in that step's session while it is there. `note` (the user's) goes
/// to the session first.
pub(crate) async fn resume_run(st: &AppState, run_id: &str, note: Option<String>) -> R<()> {
    let _engine = st.engine_lock.lock().await;
    resume_locked(st, run_id, note).await
}

/// `resume_run` for a caller that holds the engine lock.
pub(crate) async fn resume_locked(st: &AppState, run_id: &str, note: Option<String>) -> R<()> {
    let steps = st.store.list_steps(run_id).await.map_err(err)?;
    let step = pdt_core::workflow::agent::resumable_step(&steps)
        .cloned()
        .ok_or("the run has no agent step that timed out or stopped to resume from")?;
    reopen_at(st, &step, json!({"choice": pdt_core::inbox::RESUME})).await?;
    let mut next = new_step(run_id, next_seq(st, run_id).await?, &step.node_id, &step.kind);
    let note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    // A resumed step doesn't count against the node's loop limit: the user asked for it.
    next.wait = json!({"loopAllowed": true, "resumeOf": step.id, "note": note});
    st.store.put_step(&next).await.map_err(err)
}

pub(crate) async fn cancel_run(st: &AppState, run_id: &str) -> R<()> {
    let _engine = st.engine_lock.lock().await;
    cancel_locked(st, run_id).await
}

/// `cancel_run` for a caller that holds the engine lock.
pub(crate) async fn cancel_locked(st: &AppState, run_id: &str) -> R<()> {
    let mut run = st.store.get_workflow_run(run_id).await.map_err(err)?.ok_or("no such run")?;
    if run.status.is_finished() {
        return Err("the run has ended".into());
    }
    finish_run(st, &mut run, RunStatus::Cancelled, Value::Null, Some("cancelled".into())).await
}

/// Answers a waiting gate.
/// Answers a waiting gate with `choice`. The user's `text` with it (why they rejected, what to change) goes on the
/// run's ticket as their comment, so the timeline keeps it and the agent working on the ticket hears it ("Ticket
/// comments to the agent"); the gate's output carries it too.
pub(crate) async fn answer(st: &AppState, run_id: &str, step_id: &str, choice: &str, text: Option<&str>) -> R<()> {
    let _engine = st.engine_lock.lock().await;
    let step = st.store.get_step(step_id).await.map_err(err)?.filter(|s| s.run_id == run_id).ok_or("no such step")?;
    let choices: Vec<String> = step
        .wait
        .pointer("/answer/choices")
        .and_then(|c| serde_json::from_value(c.clone()).ok())
        .filter(|_| step.status == StepStatus::Waiting)
        .ok_or("the step is not waiting for an answer")?;
    if !choices.iter().any(|c| c == choice) {
        return Err(format!("{choice} is not one of {}", choices.join(", ")));
    }
    // A choice that approves records the user's approval of the run's attempt (Phase 7c §5).
    if step.wait.pointer("/answer/approve").and_then(Value::as_str) == Some(choice)
        && let Some(run) = st.store.get_workflow_run(run_id).await.map_err(err)?
        && let Some(attempt) = run.attempt_id.as_deref()
        && let Some(info) = st.store.get_attempt(attempt).await.map_err(err)?
    {
        let extra = json!({"workflowId": run.workflow_id, "workflowName": run.workflow_name, "runId": run.id});
        crate::review::record_approval(st, &info, extra).await.map_err(|e| e.message().to_string())?;
    }
    // Its Inbox item is answered, wherever the answer came from.
    for item in st.store.open_items_for_step(step_id).await.map_err(err)? {
        let _ = st
            .store
            .end_inbox_item(&item.id, pdt_core::inbox::InboxStatus::Resolved, &json!({"choice": choice}), now_ms())
            .await;
    }
    let text = text.map(str::trim).filter(|t| !t.is_empty());
    if let Some(note) = text
        && let Some(run) = st.store.get_workflow_run(run_id).await.map_err(err)?
        && let Some(ticket) = run.ticket_id.as_deref()
    {
        let body = pdt_core::ticket::valid_comment(note).map_err(|e| e.to_string())?;
        let data = json!({"workflowId": run.workflow_id, "workflowName": run.workflow_name, "runId": run.id,
                          "choice": choice});
        st.store
            .add_comment_by(ticket, &body, pdt_core::ticket::Actor::User, Some(data), now_ms())
            .await
            .map_err(err)?;
    }
    complete_step(st, &step, choice, json!({"choice": choice, "answeredBy": "user", "text": text})).await
}

// ---- dry run ----

/// The event a dry run starts from.
#[derive(Debug, Clone, Deserialize, Serialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DryEvent {
    /// An event type, or `manual`.
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub kind: String,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DryTrigger {
    pub node: String,
    pub matched: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DryStep {
    pub node_id: String,
    pub kind: String,
    #[ts(type = "unknown")]
    pub input: Value,
    pub port: Option<String>,
    #[ts(type = "unknown")]
    pub output: Value,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DryRun {
    pub triggers: Vec<DryTrigger>,
    pub steps: Vec<DryStep>,
    /// How the run would end (`succeeded`, `quiet`, `failed`); None when no trigger matched.
    pub status: Option<RunStatus>,
}

/// Runs `doc` against `event` in memory: conditions evaluate, actions say what they would do.
pub(crate) async fn dry_run(st: &AppState, wf: &Workflow, doc: &WorkflowDoc, event: DryEvent) -> DryRun {
    let trigger = match event.payload {
        Value::Object(mut m) => {
            m.insert("type".into(), json!(event.kind));
            Value::Object(m)
        }
        other => json!({"payload": other, "type": event.kind}),
    };
    let ctx0 = json!({"trigger": trigger});
    let mut out = DryRun { triggers: vec![], steps: vec![], status: None };
    let mut start = None;
    for node in &doc.nodes {
        let fires = match catalog().get(&node.kind) {
            Some(k) if k.event_type() == Some(event.kind.as_str()) => true,
            Some(_) if node.kind == MANUAL && event.kind == MANUAL => true,
            _ => false,
        };
        if !fires {
            continue;
        }
        let cfg = catalog().current_config(node).and_then(|c| parse::<SharedTrigger>(&c)).unwrap_or_default();
        let project = trigger.get("projectId").and_then(|p| p.as_str());
        let reason = if !cfg.projects.is_empty() && !project.is_some_and(|p| cfg.projects.iter().any(|x| x == p)) {
            Some("the event's project is not in the trigger's projects".to_string())
        } else if !all(&cfg.conditions, &ctx0) {
            Some("a condition in the trigger's where does not hold".to_string())
        } else {
            None
        };
        if reason.is_none() && start.is_none() {
            start = Some(node.id.clone());
        }
        out.triggers.push(DryTrigger { node: node.id.clone(), matched: reason.is_none(), reason });
    }
    let Some(start) = start else { return out };
    let run = Run {
        id: "dry-run".into(),
        workflow_id: wf.id.clone(),
        workflow_name: wf.name.clone(),
        version: wf.version,
        status: RunStatus::Running,
        run_key: String::new(),
        trigger_event_id: None,
        trigger_node: start.clone(),
        trigger: trigger.clone(),
        project_id: None,
        ticket_id: trigger.get("ticketId").and_then(|t| t.as_str()).map(String::from),
        attempt_id: None,
        parent_run_id: None,
        parent_step_id: None,
        error: None,
        output: Value::Null,
        wake_at: None,
        started_at: now_ms(),
        ended_at: None,
    };
    let state = st.store.workflow_state(&wf.id).await.unwrap_or_default();
    let mut done: Vec<Step> = vec![];
    let mut first = new_step(&run.id, 1, &start, &doc.node(&start).map(|n| n.kind.clone()).unwrap_or_default());
    first.status = StepStatus::Succeeded;
    first.output = trigger.clone();
    first.port = Some("out".into());
    out.steps.push(DryStep {
        node_id: start.clone(),
        kind: first.kind.clone(),
        input: Value::Null,
        port: first.port.clone(),
        output: trigger.clone(),
        error: None,
    });
    let mut queue: std::collections::VecDeque<Step> = successors(doc, &run.id, &start, "out", 2).into();
    done.push(first);
    let mut visits: HashMap<String, u32> = HashMap::new();
    let mut status = RunStatus::Succeeded;
    while let Some(mut step) = queue.pop_front() {
        if done.len() >= usize::try_from(doc.settings.max_steps).unwrap_or(usize::MAX) {
            status = RunStatus::Failed;
            break;
        }
        let Some(node) = doc.node(&step.node_id) else { continue };
        let v = visits.entry(node.id.clone()).or_default();
        *v += 1;
        let mut ds = DryStep {
            node_id: node.id.clone(),
            kind: node.kind.clone(),
            input: Value::Null,
            port: None,
            output: Value::Null,
            error: None,
        };
        if *v > node.max_visits() {
            ds.error = Some(format!("loop limit: {} ran {} times", node.id, node.max_visits()));
            out.steps.push(ds);
            status = RunStatus::Failed;
            break;
        }
        let (Some(kind), Ok(config)) = (catalog().get(&node.kind), catalog().current_config(node)) else {
            ds.error = Some(format!("no node kind is called {}", node.kind));
            out.steps.push(ds);
            status = RunStatus::Failed;
            break;
        };
        let ctx = context(wf, &run, &done, state.clone(), true);
        let input = resolve_keeping(&config, &ctx, Some("event"));
        ds.input = input.clone();
        let cx = StepCx { st, workflow: wf, run: &run, step: &step, node, ctx: &ctx, dry_run: true };
        let result = kind.dry_run(&cx, input).await;
        let port = match result {
            Ok(Outcome::Next { port, output }) => {
                ds.output = output.clone();
                step.output = output;
                Some(port)
            }
            Ok(Outcome::Wait { timeout_port, wait, .. }) => {
                ds.output = json!({"would": "wait", "for": wait});
                timeout_port
            }
            Ok(Outcome::End { quiet, output }) => {
                ds.output = output;
                out.steps.push(ds);
                status = if quiet { RunStatus::Quiet } else { RunStatus::Succeeded };
                queue.clear();
                break;
            }
            Err(e) if doc.has_edge_from(&node.id, ERROR_PORT) => {
                ds.error = Some(e.clone());
                step.output = json!({"error": e});
                Some(ERROR_PORT.to_string())
            }
            Err(e) => {
                ds.error = Some(e);
                out.steps.push(ds);
                status = RunStatus::Failed;
                break;
            }
        };
        ds.port = port.clone();
        out.steps.push(ds);
        step.status = StepStatus::Succeeded;
        if let Some(p) = &port {
            let seq = i64::try_from(done.len()).unwrap_or(0) + 2 + i64::try_from(queue.len()).unwrap_or(0);
            queue.extend(successors(doc, &run.id, &node.id, p, seq));
        }
        step.port = port;
        done.push(step);
    }
    if status == RunStatus::Succeeded && is_quiet(&done) {
        status = RunStatus::Quiet;
    }
    out.status = Some(status);
    out
}
