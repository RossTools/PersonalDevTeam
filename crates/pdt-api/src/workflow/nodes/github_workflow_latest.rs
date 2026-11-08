//! `github.workflow.latest` (PDT-283): read a workflow's newest run on a branch, e.g. a scheduled release's, which no
//! PDT run starts. `repo` defaults to the run's project's GitHub repository.

use super::forge_call::ci_call;
use super::repo_of::{check_repo, repo_for_step, templated};
use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, repo_missing, spec,
};
use pdt_core::pr::{CiRun, CiState};
use pdt_core::workflow_runs::WorkflowRun;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubWorkflowLatestConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The workflow's name as Actions shows it (its `name:`, e.g. `release`), or its file (`release.yml`).
    pub workflow: String,
    /// The branch its runs are on (default `main`): for a pull request's run, its head branch.
    #[serde(default)]
    pub branch: Option<String>,
    /// Only a run started by this event (`workflow_dispatch`, `pull_request`, `push`, `schedule`).
    #[serde(default)]
    pub event: Option<String>,
    /// Only a run of this commit (for a pull request's run, its head commit).
    #[serde(default)]
    pub head_sha: Option<String>,
    /// Only a run started at or after this time (RFC 3339, e.g. `github.workflow.run`'s `dispatchedAt`), less
    /// [`SINCE_SLACK`] for the clocks' difference.
    #[serde(default)]
    pub since: Option<String>,
}

/// How much earlier than `since` a run may start and still count: this host's clock and GitHub's differ a little.
const SINCE_SLACK: jiff::SignedDuration = jiff::SignedDuration::from_mins(2);

impl GithubWorkflowLatestConfig {
    /// Whether it names the workflow by its file, or filters by event, commit or time: then it reads the workflow's
    /// own runs (they say their file and event), not the branch's.
    fn by_workflow(&self) -> bool {
        let w = self.workflow.trim();
        w.ends_with(".yml")
            || w.ends_with(".yaml")
            || [&self.event, &self.head_sha, &self.since].iter().any(|f| nonblank(f).is_some())
    }

    /// `since` as a time, less [`SINCE_SLACK`].
    fn since(&self) -> Result<Option<jiff::Timestamp>, String> {
        let Some(t) = nonblank(&self.since) else { return Ok(None) };
        let t: jiff::Timestamp = t.parse().map_err(|e| format!("since is not a time (RFC 3339): {t}: {e}"))?;
        Ok(Some(t.checked_sub(SINCE_SLACK).unwrap_or(t)))
    }

    /// Whether `run` passes its event, commit and time filters.
    fn matches(&self, run: &WorkflowRun, since: Option<jiff::Timestamp>) -> bool {
        let at = |s: &str| s.parse::<jiff::Timestamp>().ok();
        nonblank(&self.event).is_none_or(|e| run.event == e)
            && nonblank(&self.head_sha).is_none_or(|sha| run.run.head_sha == sha)
            && since.is_none_or(|t| at(&run.run.created_at).is_some_and(|c| c >= t))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubWorkflowLatestOutput {
    /// The run's id (with `attempt`, which a re-run bumps: a key for "once per run").
    pub id: String,
    pub attempt: u32,
    pub url: String,
    pub head_sha: String,
    /// When it started (RFC 3339).
    pub created_at: String,
    /// When `failed`: the failed jobs (`workflow: job`), as a Markdown list.
    pub failed_jobs: Option<String>,
}

pub struct GithubWorkflowLatest;

impl NodeKind for GithubWorkflowLatest {
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubWorkflowLatestConfig, GithubWorkflowLatestOutput>(
            "github.workflow.latest",
            1,
            Category::Action,
            "Read a workflow's latest run",
            "Reads the newest GitHub Actions run of a workflow (by its name, `release`, or file, `release.yml`) on a \
             branch (default `main`), however it was started (a schedule, by hand), and leaves by its state: \
             `passed`, `failed` (with `failedJobs`), `running` (queued or running), `cancelled`, or `none` when there \
             is no run. `event`, `headSha` and `since` pick one run: the one a pull request's merge started (its head \
             branch and commit), or the one `github.workflow.run` started (`since` its `dispatchedAt`).",
            &[
                ("passed", "Passed"),
                ("failed", "Failed"),
                ("running", "Running"),
                ("cancelled", "Cancelled"),
                ("none", "None"),
            ],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubWorkflowLatestConfig = parse(config)?;
        if c.workflow.trim().is_empty() {
            return Err("workflow is empty: give its name (release)".into());
        }
        if nonblank(&c.since).is_some_and(|t| !templated(t)) {
            c.since()?;
        }
        check_repo(&c.repo)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubWorkflowLatestConfig = parse(&input)?;
            let of = repo_for_step(cx, &c.repo).await?;
            let repo = of.repo.clone();
            let workflow = c.workflow.trim().to_string();
            let branch = nonblank(&c.branch).unwrap_or("main").to_string();
            if cx.dry_run {
                return Ok(Outcome::would("passed", format!("read the latest {workflow} run on {branch} of {repo}")));
            }
            let actions = of.actions(&format!("read {workflow} from"))?;
            let since = c.since()?;
            let (r, w, b) = (repo.clone(), workflow, branch);
            let latest = ci_call(&actions, move |actions| {
                let newest = |x: &CiRun, y: &CiRun| x.created_at.cmp(&y.created_at);
                let run = if c.by_workflow() {
                    let runs = actions.workflow_runs(&r, &w, Some(&b), false)?;
                    let runs = runs.into_iter().filter(|x| x.run.branch == b && c.matches(x, since));
                    runs.map(|x| x.run).max_by(newest)
                } else {
                    let runs = actions.runs(&r, &b, None)?;
                    runs.into_iter().filter(|x| x.name == w && x.branch == b).max_by(newest)
                };
                let Some(mut run) = run else {
                    return Ok(None);
                };
                // Which jobs failed is only for the message: a failed lookup leaves them out.
                if run.state == CiState::Failed && run.failed_jobs.is_empty() {
                    run.failed_jobs = pdt_forge::ci::failed_job_names(actions, &r, &run).unwrap_or_default();
                }
                Ok(Some(run))
            })
            .await?;
            let Some(run) = latest else {
                return Ok(Outcome::Next { port: "none".into(), output: json!({"repo": repo, "quiet": true}) });
            };
            let port = match run.state {
                CiState::Queued => "running",
                s => s.name(),
            };
            let failed_jobs = (run.state == CiState::Failed).then(|| run.failed_jobs_markdown());
            Ok(Outcome::next(
                port,
                GithubWorkflowLatestOutput {
                    id: run.id,
                    attempt: run.attempt,
                    url: run.url,
                    head_sha: run.head_sha,
                    created_at: run.created_at,
                    failed_jobs,
                },
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let latest = |v: Value| GithubWorkflowLatest.check(&v);
        assert!(latest(json!({"workflow": "release"})).is_ok());
        assert!(latest(json!({"workflow": "release", "branch": "main", "repo": "a/b"})).is_ok());
        assert!(latest(json!({"workflow": ""})).is_err());
        assert!(latest(json!({"workflow": "release", "repo": "nope"})).is_err());
        let since = json!({"workflow": "release.yml", "since": "{{steps.dispatch.dispatchedAt}}"});
        assert!(latest(since).is_ok());
        assert!(latest(json!({"workflow": "r.yml", "since": "2026-11-03T08:00:00Z", "event": "push"})).is_ok());
        assert!(latest(json!({"workflow": "r.yml", "since": "yesterday"})).unwrap_err().contains("RFC 3339"));
    }
}
