//! The durable workflow engine (Phase 7a spec §4): matches logged events to enabled triggers, starts runs, executes
//! their steps one by one (each written before and after it runs, so a restart re-runs at most the step that was
//! running), handles waits, retries, error paths, limits and sub-workflows, and dry-runs documents in memory.
//!
//! Locks (PDT-128): `engine_lock` is held only around state changes (claiming a step, writing its result, the tick's
//! passes, the commands); a node's body runs outside it, holding only its run's lock, and runs advance side by side.

use super::catalog::{Category, Outcome, StepCx, catalog, parse};
use super::nodes::trigger::{MANUAL, SharedTrigger};
use crate::routes::{AppState, now_ms};
use pdt_core::workflow::api::{LogEvent, Run, RunStatus, Step, StepStatus, Workflow};
use pdt_core::workflow::cond::all;
use pdt_core::workflow::doc::{ERROR_PORT, WorkflowDoc};
use pdt_core::workflow::refs::{resolve, resolve_keeping};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use ts_rs::TS;

/// How many runs advance at once.
pub(crate) const CONCURRENT_RUNS: usize = 16;

/// One lock per run (`AppState::run_locks`), shared by the runs of one ticket (`lock_key`). An entry nobody holds
/// or waits for is dropped on the next lookup.
#[derive(Default)]
pub(crate) struct RunLocks(std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl RunLocks {
    fn get(&self, run_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.retain(|_, l| Arc::strong_count(l) > 1);
        m.entry(run_id.to_string()).or_default().clone()
    }

    pub(crate) async fn lock(&self, run_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.get(run_id).lock_owned().await
    }

    fn try_lock(&self, run_id: &str) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.get(run_id).try_lock_owned().ok()
    }
}

/// Runs of one ticket (or attempt) go one at a time, as before PDT-128: one run's checks (is an agent on it?) see what
/// the other's steps did. Runs of different tickets go side by side.
fn lock_key(run: &Run) -> String {
    match (&run.ticket_id, &run.attempt_id) {
        (Some(t), _) => format!("ticket:{t}"),
        (None, Some(a)) => format!("attempt:{a}"),
        _ => format!("run:{}", run.id),
    }
}

/// What a command acting on a run's steps holds: the run's lock (so no step of it is half-run), then the engine's.
pub(crate) struct Locked<'a> {
    _engine: tokio::sync::MutexGuard<'a, ()>,
    _run: Option<tokio::sync::OwnedMutexGuard<()>>,
}

/// Takes `run_id`'s lock (when there is a run), then the engine lock.
pub(crate) async fn lock_run<'a>(st: &'a AppState, run_id: Option<&str>) -> Locked<'a> {
    let run = match run_id {
        Some(id) => {
            let key = match st.store.get_workflow_run(id).await {
                Ok(Some(r)) => lock_key(&r),
                _ => format!("run:{id}"),
            };
            Some(st.run_locks.lock(&key).await)
        }
        None => None,
    };
    Locked { _run: run, _engine: st.engine_lock.lock().await }
}

/// Parsed documents of workflows as they were at `updated_at` (`AppState::workflow_docs`), so the 2 s passes over
/// enabled workflows don't parse every document each tick.
#[derive(Default)]
pub(crate) struct DocCache(std::sync::Mutex<HashMap<(String, u32, i64), Arc<WorkflowDoc>>>);

const DOC_CACHE_MAX: usize = 256;

/// The enabled workflows with their current documents.
pub(crate) async fn enabled_with_docs(st: &AppState) -> Vec<(Workflow, Arc<WorkflowDoc>)> {
    let mut out = vec![];
    for wf in st.store.enabled_workflows().await.unwrap_or_default() {
        let key = (wf.id.clone(), wf.version, wf.updated_at);
        let cached = st.workflow_docs.0.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
        let doc = match cached {
            Some(d) => d,
            None => {
                let Ok(Some(doc)) = st.store.workflow_doc(&wf.id, wf.version).await else { continue };
                let doc = Arc::new(doc);
                let mut m = st.workflow_docs.0.lock().unwrap_or_else(|e| e.into_inner());
                if m.len() >= DOC_CACHE_MAX {
                    m.clear();
                }
                m.insert(key, doc.clone());
                doc
            }
        };
        out.push((wf, doc));
    }
    out
}

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
        resumed_at: None,
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
    let enabled = enabled_with_docs(st).await;
    // Read once for the batch; a step an earlier event resumed is not resumed again.
    let waiting = st.store.steps_waiting_for_events().await.unwrap_or_default();
    let mut resumed: HashSet<String> = HashSet::new();
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
        for (_, step) in &waiting {
            if resumed.contains(&step.id) {
                continue;
            }
            let Some(w) = step.wait.get("event") else { continue };
            if w.get("type").and_then(|t| t.as_str()) != Some(e.kind.as_str()) {
                continue;
            }
            let conds: Vec<pdt_core::workflow::cond::Condition> =
                w.get("where").cloned().and_then(|c| serde_json::from_value(c).ok()).unwrap_or_default();
            if all(&conds, &json!({"event": trigger})) {
                resumed.insert(step.id.clone());
                if let Err(x) = complete_step(st, step, "event", trigger.clone()).await {
                    tracing::warn!(step = %step.id, error = %x, "could not resume a waiting step");
                }
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
    let next_seq = next_seq(st, &run.id).await?;
    let now = now_ms();
    let mut s = step.clone();
    s.status = StepStatus::Succeeded;
    s.port = Some(port.into());
    s.output = output;
    s.wake_at = None;
    s.ended_at = Some(now);
    // The step, what it leads to, its Inbox items, its agent binding and the run's wake are one write (PDT-131).
    let next = successors(&doc, &run.id, &s.node_id, port, next_seq);
    st.store.complete_workflow_step(&s, &next, now).await.map_err(err)?;
    // An agent step that timed out tells the user, who can resume it (PDT-123).
    if s.kind == super::nodes::agent::KIND && port == pdt_core::workflow::agent::TIMEOUT {
        super::agent::timed_out(st, &s, &run).await;
    }
    Ok(())
}

/// The steps a crash left out (PDT-131): a run with nothing left to run or wait for whose last step to end left by a
/// port that leads somewhere, with nothing queued after it. Before step completion was one write, a crash between
/// ending a step and queueing its successors left a run so; they are queued now instead of the run ending early.
fn lost_successors(doc: &WorkflowDoc, run_id: &str, steps: &[Step]) -> Vec<Step> {
    let max_seq = steps.iter().map(|s| s.seq).max().unwrap_or(0);
    let Some(last) = steps.iter().filter(|s| s.ended_at.is_some()).max_by_key(|s| (s.ended_at, s.seq)) else {
        return vec![];
    };
    match (&last.status, &last.port) {
        (StepStatus::Succeeded, Some(port)) if last.seq == max_seq => {
            successors(doc, run_id, &last.node_id, port, max_seq + 1)
        }
        _ => vec![],
    }
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
        if let Err(e) = st.store.put_agent_step(&a).await {
            tracing::warn!(step = %step_id, error = %e, "could not end a step's agent binding");
        }
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

/// Runs a run's steps until it waits or ends. The caller holds the run's lock; the engine lock is taken for each state
/// change and let go while a node's body runs.
async fn advance(st: &AppState, run_id: &str) -> R<()> {
    let mut loaded: Option<(Workflow, WorkflowDoc)> = None;
    for _ in 0..10_000 {
        let engine = st.engine_lock.lock().await;
        // Read again each time: a command may have ended the run while a step ran.
        let Some(mut run) = st.store.get_workflow_run(run_id).await.map_err(err)? else { return Ok(()) };
        if run.status.is_finished() {
            return Ok(());
        }
        if loaded.is_none() {
            match load(st, &run).await {
                Ok(x) => loaded = Some(x),
                Err(e) => return finish_run(st, &mut run, RunStatus::Failed, Value::Null, Some(e)).await,
            }
        }
        let Some((wf, doc)) = loaded.as_ref() else { return Ok(()) };
        let now = now_ms();
        let steps = st.store.list_steps(&run.id).await.map_err(err)?;
        let deadline =
            run.runtime_from().saturating_add(i64::try_from(doc.settings.max_runtime_ms).unwrap_or(i64::MAX));
        if now > deadline {
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
                // It wakes for its first timer, or at its runtime limit at the latest (PDT-131): a wait with no timeout
                // (an answer, a sub-run, a session that never comes) can't keep it going past `maxRuntimeMs`.
                let first = waiting.iter().filter_map(|s| s.wake_at).min();
                let limit = deadline.saturating_add(1);
                run.wake_at = Some(first.map_or(limit, |t| t.min(limit)));
                return st.store.update_workflow_run(&run).await.map_err(err);
            }
            let lost = lost_successors(doc, &run.id, &steps);
            if !lost.is_empty() {
                tracing::warn!(run = %run.id, "a step ended without its next steps queued: queueing them");
                for n in &lost {
                    st.store.put_step(n).await.map_err(err)?;
                }
                continue;
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
        let job = match claim(st, wf, doc, &mut run, step, &steps).await? {
            Claim::Ended => return Ok(()),
            Claim::Moved => continue,
            Claim::Run(job) => job,
        };
        // A node's body runs outside the engine lock (PDT-128), unless it is a quick one the engine's passes depend on.
        let engine = if job.locked {
            Some(engine)
        } else {
            drop(engine);
            None
        };
        let (result, cut_short) = {
            let cx = StepCx {
                st,
                workflow: wf,
                run: &run,
                step: &job.step,
                node: job.node,
                ctx: &job.ctx,
                dry_run: false,
                rerun: job.rerun,
            };
            // A timeout drops the body wherever it was: it may have done its work (PDT-131).
            match tokio::time::timeout(job.limit, job.kind.run(&cx, job.input)).await {
                Ok(r) => (r, false),
                Err(_) => (Err(format!("timed out after {} ms", job.limit.as_millis())), true),
            }
        };
        let _engine = match engine {
            Some(g) => g,
            None => st.engine_lock.lock().await,
        };
        // The step may have ended while it ran (its run was cancelled): then its result goes nowhere.
        let current = st.store.get_step(&job.step.id).await.map_err(err)?;
        let Some(current) = current.filter(|s| s.status == StepStatus::Running && s.attempts == job.step.attempts)
        else {
            tracing::info!(run = %run_id, node = %job.step.node_id, "a step's run moved on while it ran: its result is dropped");
            continue;
        };
        let Some(mut run) = st.store.get_workflow_run(run_id).await.map_err(err)? else { return Ok(()) };
        // What the body recorded as it went (`StepCx::save_progress`) stays with the step for its next execution.
        let mut step = job.step;
        step.output = current.output;
        if apply(st, doc, &mut run, step, result, cut_short).await? {
            return Ok(());
        }
    }
    Ok(())
}

/// A step claimed to run: what its body needs.
struct Job<'a> {
    step: Step,
    node: &'a pdt_core::workflow::doc::Node,
    kind: &'static dyn super::catalog::NodeKind,
    input: Value,
    ctx: Value,
    limit: std::time::Duration,
    /// The body runs under the engine lock.
    locked: bool,
    /// The step ran before and was cut short (`StepCx::rerun`).
    rerun: bool,
}

enum Claim<'a> {
    /// The run ended.
    Ended,
    /// The step ended without running (an error path or a retry): look again.
    Moved,
    Run(Box<Job<'a>>),
}

/// Marks one step running (or fails it before it runs), under the engine lock.
async fn claim<'a>(
    st: &AppState,
    wf: &Workflow,
    doc: &'a WorkflowDoc,
    run: &mut Run,
    mut step: Step,
    steps: &[Step],
) -> R<Claim<'a>> {
    let ended = |e: bool| if e { Claim::Ended } else { Claim::Moved };
    let Some(node) = doc.node(&step.node_id) else {
        let e = format!("node {} is not in the workflow", step.node_id);
        finish_run(st, run, RunStatus::Failed, Value::Null, Some(e)).await?;
        return Ok(Claim::Ended);
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
        (_, Err(e)) => return fail_step(st, doc, run, step, e, true).await.map(ended),
        (None, _) => {
            let e = format!("no node kind is called {}", node.kind);
            return fail_step(st, doc, run, step, e, true).await.map(ended);
        }
        _ => {
            let e = format!("loop limit: {} ran {visits} times in this run", node.id);
            step.attempts = u32::MAX / 2;
            return fail_step(st, doc, run, step, e, true).await.map(ended);
        }
    };
    let state = st.store.workflow_state(&wf.id).await.map_err(err)?;
    let ctx = context(wf, run, steps, state, false);
    let input = resolve_keeping(&config, &ctx, Some("event"));
    // Found running, it was cut short by a restart (PDT-131); a retry after a timeout says so in its wait. A restart
    // doesn't use up one of the node's retries.
    let restarted = step.status == StepStatus::Running;
    let rerun = restarted || step.wait.get(CUT_SHORT).and_then(Value::as_bool) == Some(true);
    if !restarted {
        step.attempts += 1;
    }
    step.status = StepStatus::Running;
    step.input = input.clone();
    step.started_at = Some(now_ms());
    step.error = None;
    if rerun && !kind.rerun_safe(&input) {
        let e = "PDT stopped while this step ran, so it may have done its work already: it does not run again on its \
                 own. Retry the run to run it once more.";
        tracing::warn!(workflow = %run.workflow_name, run = %run.id, node = %step.node_id, "a step cut short by a restart is not run again");
        return fail_step(st, doc, run, step, e.into(), false).await.map(ended);
    }
    st.store.put_step(&step).await.map_err(err)?;
    let limit = std::time::Duration::from_millis(node.timeout_ms.unwrap_or_else(|| kind.default_timeout_ms()));
    let outside = matches!(catalog().spec(&node.kind).map(|s| s.category), Some(Category::Action | Category::Agent));
    let locked = !outside || kind.in_engine_lock();
    Ok(Claim::Run(Box::new(Job { step, node, kind, input, ctx, limit, locked, rerun })))
}

/// Writes what a step's body came to, under the engine lock. True when the run ended.
async fn apply(
    st: &AppState,
    doc: &WorkflowDoc,
    run: &mut Run,
    mut step: Step,
    result: Result<Outcome, String>,
    cut_short: bool,
) -> R<bool> {
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
            // The event may have come already, while the run was on an earlier step (PDT-131).
            if let Some(e) = look_back(st, run, &step).await? {
                complete_step(st, &step, "event", trigger_value(&e)).await?;
            }
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
        Err(e) => fail_step(st, doc, run, step, e, !cut_short).await,
    }
}

/// An event a step that starts waiting for one would have taken had it been waiting already: one of its type and
/// conditions logged since the run's start, that is not the run's trigger and that no step of the run took already
/// (a loop waits for the next one, not the last). Events are matched to waits only as they are processed, so without
/// this a wait reached after its event (a pull request merged by hand during QA) would wait for one that never comes.
async fn look_back(st: &AppState, run: &Run, step: &Step) -> R<Option<LogEvent>> {
    let Some(w) = step.wait.get("event") else { return Ok(None) };
    let Some(kind) = w.get("type").and_then(Value::as_str) else { return Ok(None) };
    let conds: Vec<pdt_core::workflow::cond::Condition> =
        w.get("where").cloned().and_then(|c| serde_json::from_value(c).ok()).unwrap_or_default();
    let steps = st.store.list_steps(&run.id).await.map_err(err)?;
    let taken: HashSet<&str> = steps
        .iter()
        .filter(|s| s.status == StepStatus::Succeeded && s.port.as_deref() == Some("event"))
        .filter_map(|s| s.output.get("eventId").and_then(Value::as_str))
        .chain(run.trigger_event_id.as_deref())
        .collect();
    let filter = pdt_store::EventFilter {
        kind: Some(kind.into()),
        since: Some(run.started_at),
        limit: 1000,
        ..Default::default()
    };
    let events = st.store.list_log_events(&filter).await.map_err(err)?;
    Ok(events.into_iter().find(|e| !taken.contains(e.id.as_str()) && all(&conds, &json!({"event": trigger_value(e)}))))
}

/// A step's `wait` while it waits to be retried after a timeout: it was cut short (`StepCx::rerun`).
const CUT_SHORT: &str = "cutShort";

/// A failed execution: retried while the node allows, then its error port, else the run fails. True when it ended.
/// `definite`: it failed and did nothing (else it was cut short, by a timeout or a restart, and may have done its work:
/// a kind that isn't safe to run again is not retried, PDT-131).
async fn fail_step(
    st: &AppState,
    doc: &WorkflowDoc,
    run: &mut Run,
    mut step: Step,
    e: String,
    definite: bool,
) -> R<bool> {
    // An agent step whose session would not start no longer works on its attempt (PDT-129). One cut short after its
    // session started keeps its binding, so running it again goes on in that session instead of starting another.
    if step.kind == super::nodes::agent::KIND
        && let Ok(Some(mut a)) = st.store.get_agent_step(&step.id).await
        && a.state != pdt_core::workflow::agent::AgentStepState::Done
        && a.session_id.is_none()
    {
        a.state = pdt_core::workflow::agent::AgentStepState::Done;
        a.updated_at = now_ms();
        if let Err(e) = st.store.put_agent_step(&a).await {
            tracing::warn!(step = %a.step_id, error = %e, "could not write an agent step's binding");
        }
    }
    let node = doc.node(&step.node_id);
    let retry = node.and_then(|n| n.retry);
    tracing::info!(workflow = %run.workflow_name, run = %run.id, node = %step.node_id, error = %e, "a workflow step failed");
    step.error = Some(e.clone());
    let safe = definite || catalog().get(&step.kind).is_none_or(|k| k.rerun_safe(&step.input));
    if let Some(r) = retry.filter(|_| safe)
        && step.attempts <= r.max
    {
        step.status = StepStatus::Waiting;
        step.wait = json!({"retry": true, CUT_SHORT: !definite});
        step.wake_at = Some(now_ms() + i64::try_from(r.delay_ms(step.attempts)).unwrap_or(0));
        st.store.put_step(&step).await.map_err(err)?;
        return Ok(false);
    }
    if node.is_some() && doc.has_edge_from(&step.node_id, ERROR_PORT) {
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

/// Advances every run that can move, each in its own task (at most `CONCURRENT_RUNS` at once), and waits for them
/// up to `engine_tick_wait`; a run still going then (a slow step) goes on in the background, and later ticks leave it
/// alone while its lock is held.
async fn advance_all(st: &AppState) {
    let runs = match st.store.runnable_workflow_runs(now_ms()).await {
        Ok(r) => r,
        Err(e) => return tracing::warn!(error = %e, "could not list workflow runs"),
    };
    // A ticket's runs go one after another in one task (`lock_key`), in the order they were listed.
    let mut groups: Vec<(String, Vec<Run>)> = vec![];
    for run in runs {
        let key = lock_key(&run);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, g)) => g.push(run),
            None => groups.push((key, vec![run])),
        }
    }
    let mut tasks = tokio::task::JoinSet::new();
    for (key, group) in groups {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        // Being advanced already (a slow step), or a command is acting on it: a later tick picks it up.
        let Some(guard) = st.run_locks.try_lock(&key) else { continue };
        let Ok(permit) = st.engine_runs.clone().try_acquire_owned() else { break };
        let st = st.clone();
        tasks.spawn(async move {
            let _held = (guard, permit);
            for run in group {
                if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                if let Err(e) = advance(&st, &run.id).await {
                    tracing::warn!(run = %run.id, error = %e, "a workflow run could not go on");
                }
            }
        });
    }
    let all = async { while tasks.join_next().await.is_some() {} };
    let _ = tokio::time::timeout(st.engine_tick_wait, all).await;
    tasks.detach_all();
}

/// One engine tick: install built-ins (once), match new events, run steps, deliver session messages, run what they
/// woke. pdtd runs it every 2 s; tests call it.
pub async fn run_workflows(st: &AppState) {
    {
        let _engine = st.engine_lock.lock().await;
        super::builtins::ensure_installed(st).await;
        crate::personas::ensure_builtins(st).await;
        super::nodes::schedule::fire_schedules(st).await;
        if let Err(e) = st.store.wake_untimed_workflow_runs(now_ms()).await {
            tracing::warn!(error = %e, "could not wake waiting runs with no wake time");
        }
        match_events(st).await;
        crate::scheduler::admit_waiting(st).await;
    }
    advance_all(st).await;
    {
        let _engine = st.engine_lock.lock().await;
        super::outbox::deliver(st).await;
        super::agent::sweep(st).await;
    }
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
    let _locked = lock_run(st, Some(run_id)).await;
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
    // The user's Retry runs it once more, whatever cut it short (PDT-131).
    if let Value::Object(w) = &mut failed.wait {
        w.remove(CUT_SHORT);
    }
    st.store.put_step(&failed).await.map_err(err)?;
    run.status = RunStatus::Running;
    run.error = None;
    run.ended_at = None;
    // Its runtime limit counts from now, as its waits' timeouts do.
    run.resumed_at = Some(now_ms());
    st.store.update_workflow_run(&run).await.map_err(err)
}

/// Takes a run back to `step`, the agent step it timed out or stopped at (PDT-123), so it can go on from there: what
/// the run still had to do after it (its `timeout` path) is skipped, the step's open Inbox items end with
/// `resolution`, and the run runs again. Callers hold the run's and the engine's locks and add the step that goes on.
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
    run.resumed_at = Some(now);
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
    let _locked = lock_run(st, Some(run_id)).await;
    resume_locked(st, run_id, note).await
}

/// `resume_run` for a caller that holds the run's and the engine's locks.
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

/// Cancels a run. It takes only the engine lock, so it never waits for a step that is running (a hung request): that
/// step's result is dropped when it comes.
pub(crate) async fn cancel_run(st: &AppState, run_id: &str) -> R<()> {
    let _engine = st.engine_lock.lock().await;
    cancel_locked(st, run_id).await
}

/// `cancel_run` for a caller that holds the engine lock.
pub(crate) async fn cancel_locked(st: &AppState, run_id: &str) -> R<()> {
    cancel_because(st, run_id, "cancelled").await
}

/// `cancel_locked` with why (the run's error).
pub(crate) async fn cancel_because(st: &AppState, run_id: &str, why: &str) -> R<()> {
    let mut run = st.store.get_workflow_run(run_id).await.map_err(err)?.ok_or("no such run")?;
    if run.status.is_finished() {
        return Err("the run has ended".into());
    }
    finish_run(st, &mut run, RunStatus::Cancelled, Value::Null, Some(why.into())).await
}

/// Deletes a workflow (PDT-131): its runs still going are cancelled first (a run of another workflow waiting on one,
/// through `workflow.call`, goes on by `failed`), and every open Inbox item of its runs closes, since nothing could
/// answer them once it is gone. Under the engine lock, so no run of it starts meanwhile. False for an unknown one.
pub(crate) async fn delete_workflow(st: &AppState, id: &str) -> R<bool> {
    let _engine = st.engine_lock.lock().await;
    for status in [RunStatus::Running, RunStatus::Waiting] {
        let f = pdt_store::RunFilter {
            workflow_id: Some(id.into()),
            status: Some(status),
            limit: 500,
            ..Default::default()
        };
        for run in st.store.list_workflow_runs(&f).await.map_err(err)? {
            if let Err(e) = cancel_because(st, &run.id, "its workflow was deleted").await {
                tracing::warn!(run = %run.id, error = %e, "could not cancel a deleted workflow's run");
            }
        }
    }
    st.store.close_workflow_inbox_items(id, now_ms()).await.map_err(err)?;
    st.store.delete_workflow(id).await.map_err(err)
}

/// Answers a waiting gate with `choice`. The user's `text` with it (why they rejected, what to change) goes on the
/// run's ticket as their comment, so the timeline keeps it and the agent working on the ticket hears it ("Ticket
/// comments to the agent"); the gate's output carries it too.
pub(crate) async fn answer(st: &AppState, run_id: &str, step_id: &str, choice: &str, text: Option<&str>) -> R<()> {
    let _locked = lock_run(st, Some(run_id)).await;
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
        resumed_at: None,
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
        let cx = StepCx { st, workflow: wf, run: &run, step: &step, node, ctx: &ctx, dry_run: true, rerun: false };
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
