//! Dry runs: a workflow document run against an event in memory, where conditions evaluate and actions say what they
//! would do.

use super::commands::manual_config;
use super::{context, fence_input, is_quiet, new_step, successors};
use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{Outcome, StepCx, parse};
use crate::workflow::nodes::trigger::{MANUAL, SharedTrigger};
use pdt_core::workflow::api::{Run, RunStatus, Step, StepStatus, Workflow};
use pdt_core::workflow::cond::all;
use pdt_core::workflow::doc::{ERROR_PORT, WorkflowDoc};
use pdt_core::workflow::dry::with_stand_ins;
use pdt_core::workflow::refs::resolve_keeping;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use ts_rs::TS;

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
    // Run now's input as a real run gets it: checked against the declared inputs, defaults filled in (PDT-301).
    let manual = doc.nodes.iter().find(|n| n.kind == MANUAL).filter(|_| event.kind == MANUAL);
    let trigger = match (manual, event.payload) {
        (Some(node), payload) => match manual_config(st, node).and_then(|c| c.input(&payload)) {
            Ok(t) => t,
            Err(e) => {
                let refused = DryTrigger { node: node.id.clone(), matched: false, reason: Some(e) };
                return DryRun { triggers: vec![refused], steps: vec![], status: None };
            }
        },
        (None, Value::Object(mut m)) => {
            m.insert("type".into(), json!(event.kind));
            Value::Object(m)
        }
        (None, other) => json!({"payload": other, "type": event.kind}),
    };
    let ctx0 = json!({"trigger": trigger});
    let mut out = DryRun { triggers: vec![], steps: vec![], status: None };
    let mut start = None;
    for node in &doc.nodes {
        let fires = match st.registry().catalog().get(&node.kind) {
            Some(k) if k.event_type() == Some(event.kind.as_str()) => true,
            Some(_) if node.kind == MANUAL && event.kind == MANUAL => true,
            _ => false,
        };
        if !fires {
            continue;
        }
        let cfg =
            st.registry().catalog().current_config(node).and_then(|c| parse::<SharedTrigger>(&c)).unwrap_or_default();
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
        // As a real run's: the event's project, else the workflow's.
        project_id: trigger
            .get("projectId")
            .and_then(|p| p.as_str())
            .map(String::from)
            .or_else(|| wf.project_id.clone()),
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
        let reg = st.registry();
        let (Some(kind), Ok(config)) = (reg.catalog().get(&node.kind), reg.catalog().current_config(node)) else {
            ds.error = Some(reg.catalog().missing(&node.kind).1);
            out.steps.push(ds);
            status = RunStatus::Failed;
            break;
        };
        let ctx = context(wf, &run, &done, state.clone(), true);
        let mut input = resolve_keeping(&config, &ctx, &["event", "secret"]);
        fence_input(&reg, kind.agent_text(), &config, &ctx, None, &mut input);
        ds.input = input.clone();
        let cx = StepCx {
            st,
            workflow: wf,
            run: &run,
            step: &step,
            node,
            ctx: &ctx,
            origin: None,
            dry_run: true,
            rerun: false,
        };
        let result =
            kind.dry_run(&cx, input).await.map_err(|e| crate::workflow::catalog::setup_split(&e).1.to_string());
        let port = match result {
            Ok(Outcome::Next { port, output }) => {
                // An action that only says what it would do still gives later steps the fields they read (PDT-189).
                let output = if output.get("would").is_some() {
                    with_stand_ins(output, &kind.spec().output_schema)
                } else {
                    output
                };
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
    if status == RunStatus::Succeeded && is_quiet(st, &done) {
        status = RunStatus::Quiet;
    }
    out.status = Some(status);
    out
}
