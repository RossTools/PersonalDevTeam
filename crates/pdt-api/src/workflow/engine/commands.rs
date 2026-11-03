//! The engine's commands used by the API: Run now, sub-workflows, retry, resume, cancel, delete and gate answers.

use super::{CUT_SHORT, R, complete_step, end_step_extras, err, finish_run, lock_run, new_step, start_run};
use crate::log_err::LogErr;
use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{StepCx, parse};
use crate::workflow::nodes::trigger::{MANUAL, ManualConfig};
use pdt_core::workflow::api::{Run, RunStatus, Step, StepStatus, Workflow};
use pdt_core::workflow::refs::resolve;
use serde_json::{Value, json};

/// "Run now": a run of the workflow's `manual` trigger with `input`.
pub(crate) async fn start_manual(
    st: &AppState,
    wf: &Workflow,
    input: Value,
    run_key: Option<String>,
) -> R<Option<String>> {
    let doc = st.store.workflow_doc(&wf.id, wf.version).await.map_err(err)?.ok_or("the workflow has no document")?;
    let node = doc.nodes.iter().find(|n| n.kind == MANUAL).ok_or("the workflow has no Run now trigger")?;
    let manual = manual_config(st, node)?;
    let trigger = manual.input(&input).map_err(|e| format!("{} can't run with that input: {e}", wf.name))?;
    if wf.project_id.is_none()
        && trigger.get("projectId").and_then(Value::as_str).is_none_or(str::is_empty)
        && crate::workflow::catalog::doc_needs_project(st.registry().catalog(), &doc)
    {
        return Err(format!(
            "{} needs a project to run in (its steps work on the run's project): give projectId",
            wf.name
        ));
    }
    let configured = manual.run_key;
    let key = run_key
        .or_else(|| {
            configured.and_then(|k| resolve(&Value::String(k), &json!({"trigger": trigger})).as_str().map(String::from))
        })
        .unwrap_or_else(|| format!("manual:{}", uuid::Uuid::now_v7()));
    let _engine = st.engine_lock.lock().await;
    start_run(st, wf, &doc, &node.id, trigger, None, key, None).await
}

/// A Run now trigger's config at its kind's current version.
pub(super) fn manual_config(st: &AppState, node: &pdt_core::workflow::doc::Node) -> R<ManualConfig> {
    let config = st.registry().catalog().current_config(node)?;
    parse(&config)
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
    let trigger =
        manual_config(st, node)?.input(&input).map_err(|e| format!("{} can't run with that input: {e}", wf.name))?;
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
    // A choice that approves records the user's approval of the run's attempt.
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
        st.store
            .end_inbox_item(&item.id, pdt_core::inbox::InboxStatus::Resolved, &json!({"choice": choice}), now_ms())
            .await
            .log_warn("close an answered Inbox item");
    }
    let text = text.map(str::trim).filter(|t| !t.is_empty());
    // The gate's own ticket when the run has none (PDT-269).
    let gate_ticket = step.wait.pointer("/answer/ticket").and_then(Value::as_str).map(String::from);
    if let Some(note) = text
        && let Some(run) = st.store.get_workflow_run(run_id).await.map_err(err)?
        && let Some(ticket) = run.ticket_id.as_deref().or(gate_ticket.as_deref())
    {
        let body = pdt_core::ticket::valid_comment(note).map_err(crate::workflow::err)?;
        let data = json!({"workflowId": run.workflow_id, "workflowName": run.workflow_name, "runId": run.id,
                          "choice": choice});
        st.store
            .add_comment_by(ticket, &body, pdt_core::ticket::Actor::User, Some(data), now_ms())
            .await
            .map_err(err)?;
    }
    complete_step(st, &step, choice, json!({"choice": choice, "answeredBy": "user", "text": text})).await
}
