//! Failed runs of a repository's CI workflows (PDT-292). The `github.workflow.failed` trigger watches one GitHub
//! Actions workflow (in `repo`, else the workflow's project's repository): every `every` (default 30 minutes) pdtd
//! reads its completed runs, however they were started, and logs `github.workflow.failed` for each failed attempt
//! since the last look. Where it read to is a [`RunCursor`] in the workflow's state, so neither a restart nor a
//! catch-up misses or repeats a run; the first look only records where the runs are.

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, ports, schema_of,
};
use crate::workflow::nodes::schedule::RESERVED_STATE;
use pdt_core::workflow::api::{LogEvent, NewLogEvent, Workflow};
use pdt_core::workflow::cond::Condition;
use pdt_core::workflow::doc::Node;
use pdt_core::workflow::events::{GITHUB_WORKFLOW_FAILED, GithubWorkflowFailed, event_type};
use pdt_core::workflow::schedule::parse_every;
use pdt_core::workflow_runs::{RunCursor, WorkflowRun, event_matches, reportable};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// How often a watch looks when `every` is left out.
pub const DEFAULT_EVERY: &str = "30m";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct WorkflowFailedConfig {
    /// The repository (`owner/name`); the workflow's project's when left out.
    pub repo: Option<String>,
    /// The workflow: its file name (`ci.yml`) or its name as Actions shows it (`ci`).
    pub workflow: String,
    /// Only its runs on this branch (or tag); empty: every branch.
    pub branch: Option<String>,
    /// Only runs these started (`schedule`, `workflow_dispatch`, `push`, `pull_request`…); empty: any.
    pub events: Vec<String>,
    /// How often to look: `"30m"` (the default), `"1h"`.
    pub every: Option<String>,
    /// Only runs for which every condition holds.
    #[serde(rename = "where")]
    #[ts(rename = "where")]
    pub conditions: Vec<Condition>,
    /// The run's idempotency key; default: the failed attempt (a re-run that fails again starts another run).
    pub run_key: Option<String>,
}

fn templated(s: &str) -> bool {
    s.contains("{{")
}

impl WorkflowFailedConfig {
    /// How often it looks, in ms.
    fn every_ms(&self) -> Result<u64, String> {
        parse_every(nonblank(&self.every).unwrap_or(DEFAULT_EVERY))
    }
}

pub struct WorkflowFailed;

impl NodeKind for WorkflowFailed {
    fn spec(&self) -> KindSpec {
        let t = event_type(GITHUB_WORKFLOW_FAILED).expect("github.workflow.failed is an event type");
        KindSpec {
            id: GITHUB_WORKFLOW_FAILED.into(),
            version: 1,
            category: Category::Trigger,
            title: t.title.into(),
            description: "A failed run of one GitHub Actions workflow (`workflow`, its file or name) in `repo` (else \
                          the project's), however it was started: on a schedule, by hand, by a push. Optionally only \
                          on `branch` and for some `events`. Checked every `every` (default 30 minutes); each failed \
                          attempt starts one run, and the first look only records where the runs are."
                .into(),
            config_schema: schema_of::<WorkflowFailedConfig>(),
            output_schema: serde_json::to_value((t.schema)()).unwrap_or(Value::Null),
            ports: ports(&[("out", "Then")]),
            dynamic_ports: false,
            preset: None,
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: WorkflowFailedConfig = parse(config)?;
        if c.workflow.trim().is_empty() {
            return Err("workflow is empty: give its file (ci.yml) or name".into());
        }
        crate::workflow::nodes::release::check_repo(&c.repo)?;
        if !nonblank(&c.every).is_some_and(templated) {
            c.every_ms()?;
        }
        Ok(())
    }

    fn event_type(&self) -> Option<&str> {
        Some(GITHUB_WORKFLOW_FAILED)
    }

    /// Its own looks' events only (each node keeps its own cursor).
    fn accepts(&self, _config: &Value, wf: &Workflow, node: &Node, event: &LogEvent) -> bool {
        let s = |k: &str| event.payload.get(k).and_then(|v| v.as_str());
        s("workflowId") == Some(wf.id.as_str()) && s("nodeId") == Some(node.id.as_str())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move { Ok(Outcome::Next { port: "out".into(), output: cx.run.trigger.clone() }) })
    }
}

/// The state key of a node's cursor.
pub fn cursor_key(node: &str) -> String {
    format!("{RESERVED_STATE}workflow_runs.{node}")
}

/// One look at every enabled watch that is due. pdtd runs it with the 60 s poll. Never fails; problems are logged
/// (a failing read once per watch until it works again) and the look is tried again on the next poll.
pub async fn refresh_workflow_runs(st: &AppState) {
    // The GitHub plugin's trigger: nothing to read while it is off.
    if !st.integrations.is_enabled("github") {
        return;
    }
    let enabled = match crate::workflow::engine::enabled_with_docs(st).await {
        Ok(w) => w,
        Err(e) => return tracing::warn!(error = %e, "could not read the enabled workflows"),
    };
    for (wf, doc) in enabled {
        let nodes: Vec<&Node> = doc.nodes.iter().filter(|n| n.kind == GITHUB_WORKFLOW_FAILED).collect();
        if nodes.is_empty() {
            continue;
        }
        // Without its cursors a watch would read from scratch (reporting runs twice): it waits for the next poll.
        let state = match st.store.workflow_state(&wf.id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(workflow = %wf.id, error = %e, "could not read a workflow's watch cursors");
                continue;
            }
        };
        for node in nodes {
            if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let Ok(c) = st.registry().catalog().current_config(node).and_then(|c| parse::<WorkflowFailedConfig>(&c))
            else {
                continue;
            };
            let Ok(every) = c.every_ms() else { continue };
            let key = cursor_key(&node.id);
            let cursor: Option<RunCursor> = state.get(&key).and_then(|v| serde_json::from_value(v.clone()).ok());
            let now = now_ms();
            if cursor.as_ref().is_some_and(|c| !c.due(now, every)) {
                continue;
            }
            look(st, &wf, node, &c, cursor, now).await;
        }
    }
}

/// One watch's look: its fresh failed attempts logged, its cursor moved past what it read.
async fn look(
    st: &AppState,
    wf: &Workflow,
    node: &Node,
    c: &WorkflowFailedConfig,
    cursor: Option<RunCursor>,
    now: i64,
) {
    let what = format!("workflow runs:{}:{}", wf.id, node.id);
    let read = async {
        let of = crate::workflow::nodes::release::repo_for(st, wf.project_id.as_deref(), &c.repo).await?;
        let actions = of.actions("read workflow runs from")?;
        let (r, w, b, a) =
            (of.repo.clone(), c.workflow.trim().to_string(), nonblank(&c.branch).map(String::from), actions.clone());
        let runs = tokio::task::spawn_blocking(move || a.workflow_runs(&r, &w, b.as_deref(), true))
            .await
            .map_err(|e| format!("the GitHub task failed: {e}"))?
            .map_err(|e| format!("GitHub: {e}"))?;
        Ok::<_, String>((of.repo, actions, runs))
    };
    let (repo, actions, runs) = match read.await {
        Ok(x) => {
            crate::github::gh_worked(st, &what);
            x
        }
        Err(e) => return crate::github::gh_failed(st, &what, &e),
    };
    let runs: Vec<WorkflowRun> = runs.into_iter().filter(|r| event_matches(&c.events, &r.event)).collect();
    let mut cursor = match cursor {
        Some(c) => c,
        None => {
            // The first look: nothing that already happened is news.
            return save(st, wf, node, &RunCursor::baseline(&runs, now)).await;
        }
    };
    for run in cursor.fresh(&runs).into_iter().cloned().collect::<Vec<_>>() {
        if reportable(&run) {
            let (a, r, ci) = (actions.clone(), repo.clone(), run.run.clone());
            // Which jobs failed is only for the message: a failed lookup leaves them out.
            let failed_jobs = tokio::task::spawn_blocking(move || pdt_forge::ci::failed_job_names(a.as_ref(), &r, &ci))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            let payload = GithubWorkflowFailed {
                workflow_id: wf.id.clone(),
                node_id: node.id.clone(),
                project_id: wf.project_id.clone(),
                repo: repo.clone(),
                workflow: run.run.name.clone(),
                path: run.path.clone(),
                run_id: run.run.id.clone(),
                attempt: run.run.attempt,
                title: run.title.clone(),
                branch: run.run.branch.clone(),
                event: run.event.clone(),
                head_sha: run.run.head_sha.clone(),
                url: run.run.url.clone(),
                version: run.version(),
                failed_jobs,
                created_at: run.run.created_at.clone(),
                updated_at: run.updated_at.clone(),
            };
            let mut e = NewLogEvent::new(
                GITHUB_WORKFLOW_FAILED,
                format!("github:{}:{}:{repo}:run:{}", wf.id, node.id, run.key()),
                "github_workflow_runs",
                payload,
            );
            e.project_id = wf.project_id.clone();
            if let Err(x) = st.store.emit_event(&e, now).await {
                // The cursor stays before it: the next look tries it again.
                tracing::warn!(workflow = %wf.name, run = %run.key(), error = %x, "could not log a failed workflow run");
                break;
            }
            tracing::info!(workflow = %wf.name, repo, run = %run.key(), "a watched workflow run failed");
        }
        cursor.advance(&run);
    }
    cursor.looked_at = now;
    save(st, wf, node, &cursor).await;
}

async fn save(st: &AppState, wf: &Workflow, node: &Node, cursor: &RunCursor) {
    let v = serde_json::to_value(cursor).unwrap_or(Value::Null);
    if let Err(e) = st.store.set_workflow_state(&wf.id, &cursor_key(&node.id), &v, false, now_ms()).await {
        tracing::warn!(workflow = %wf.name, error = %e, "could not record where a workflow's runs were read to");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn configs_are_checked() {
        let check = |v: Value| WorkflowFailed.check(&v);
        assert!(check(json!({"workflow": "release.yml"})).is_ok());
        assert!(
            check(json!({"workflow": "ci", "repo": "o/r", "branch": "main", "events": ["schedule"], "every": "1h"}))
                .is_ok()
        );
        assert!(check(json!({"workflow": "ci", "every": "{{params.every}}"})).is_ok());
        assert!(check(json!({"workflow": " "})).unwrap_err().contains("workflow"));
        assert!(check(json!({"workflow": "ci", "repo": "nope"})).unwrap_err().contains("owner/name"));
        assert!(check(json!({"workflow": "ci", "every": "10"})).is_err());
        assert!(check(json!({"workflow": "ci", "every": "5s"})).is_err());
        assert!(check(json!({"workflow": "ci", "nope": 1})).is_err());
    }
}
