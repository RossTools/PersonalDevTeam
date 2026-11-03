//! GitHub Actions as a CI provider (PDT-242): a branch's workflow runs, their jobs and logs, re-runs, starting a
//! workflow by hand (`workflow_dispatch`), and the merge queue's runs, through the same signed-in [`GithubApi`] as
//! the forge. The GitHub plugin registers it as GitHub's own CI.

use crate::transport::encode;
use crate::{
    CI_DIRS, GhError, GithubApi, RUN_LIMIT, failed, job_list, log_tail_lines, parse_runs, parse_workflow_run, s,
};
use pdt_core::pr::{CiJob, CiRun, CiState, GITHUB_ACTIONS};
use pdt_core::workflow_runs::WorkflowRun;
use pdt_forge::{CiCapabilities, CiConfig, CiProvider, ForgeError};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

/// What Actions can do: everything but report on branches without a pull request (PDT reads it for one's head).
pub const CAPABILITIES: CiCapabilities =
    CiCapabilities { branches: false, logs: true, retry: true, trigger: true, merge_queue: true };

pub struct GithubActions {
    api: Arc<GithubApi>,
}

/// How many of a workflow's completed runs one look reads (PDT-292): a check every half hour sees far fewer.
const WORKFLOW_RUN_LIMIT: u32 = 50;

impl GithubActions {
    pub fn new(api: Arc<GithubApi>) -> Self {
        Self { api }
    }

    /// The workflow's id for the API: its file name (`release.yml`) or id as given, else the one whose name
    /// (`release`) it is.
    fn workflow_id(&self, repo: &str, workflow: &str) -> Result<String, ForgeError> {
        let w = workflow.trim();
        let file = w.rsplit('/').next().unwrap_or(w);
        if file.ends_with(".yml") || file.ends_with(".yaml") || w.chars().all(|c| c.is_ascii_digit()) {
            return Ok(file.to_string());
        }
        let v = self.api.get(&format!("repos/{repo}/actions/workflows?per_page=100"))?;
        let list = v["workflows"].as_array().ok_or_else(|| GhError::BadAnswer("no workflows".into()))?;
        list.iter()
            .find(|x| x["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(w)))
            .and_then(|x| x["id"].as_u64())
            .map(|id| id.to_string())
            .ok_or_else(|| ForgeError::Failed(format!("{repo} has no workflow named {w:?}")))
    }
}

/// A job of a jobs answer.
pub fn parse_job(j: &Value) -> Option<CiJob> {
    let state = match CiRun::state_of(j["status"].as_str().unwrap_or(""), j["conclusion"].as_str().unwrap_or("")) {
        // Only these fail a run; another conclusion (`action_required`) is not a failure to fix.
        CiState::Failed if !failed(j) => CiState::Cancelled,
        st => st,
    };
    Some(CiJob { id: j["id"].as_u64()?.to_string(), name: s(j, "name"), state, url: s(j, "html_url") })
}

impl CiProvider for GithubActions {
    fn id(&self) -> &str {
        GITHUB_ACTIONS
    }
    fn label(&self) -> &str {
        "GitHub Actions"
    }
    fn capabilities(&self) -> CiCapabilities {
        CAPABILITIES
    }
    fn config_paths(&self) -> CiConfig {
        CiConfig::Paths(CI_DIRS)
    }
    /// The branch's recent runs, whatever their commit (one conditional read, unchanged while no run is).
    fn runs(&self, repo: &str, branch: &str, _sha: Option<&str>) -> Result<Vec<CiRun>, ForgeError> {
        let path = format!("repos/{repo}/actions/runs?branch={}&per_page={RUN_LIMIT}", encode(branch));
        Ok(parse_runs(&self.api.get(&path)?)?)
    }
    fn jobs(&self, repo: &str, run: &CiRun) -> Result<Vec<CiJob>, ForgeError> {
        let (id, attempt) = (&run.id, run.attempt);
        let v = self.api.get(&format!("repos/{repo}/actions/runs/{id}/attempts/{attempt}/jobs?per_page=100"))?;
        Ok(job_list(&v)?.iter().filter_map(parse_job).collect())
    }
    fn job_log_tail(&self, repo: &str, _run: &CiRun, job: &CiJob, lines: usize) -> Result<String, ForgeError> {
        match self.api.call("GET", &format!("repos/{repo}/actions/jobs/{}/logs", job.id), None, false) {
            Ok(res) => Ok(log_tail_lines(&res.body, lines)),
            // Logs expire (and a job that never started has none): say so instead of failing.
            Err(GhError::Status(404 | 410, _)) => Ok("(no log: GitHub has none for this job)".into()),
            Err(e) => Err(e.into()),
        }
    }
    fn retry(&self, repo: &str, run: &CiRun, failed_only: bool) -> Result<(), ForgeError> {
        let what = if failed_only { "rerun-failed-jobs" } else { "rerun" };
        self.api.send_json("POST", &format!("repos/{repo}/actions/runs/{}/{what}", run.id), json!({}))?;
        Ok(())
    }
    /// `workflow_dispatch`: the workflow must declare it, and the token needs the Actions write permission.
    fn trigger(
        &self,
        repo: &str,
        git_ref: &str,
        workflow: &str,
        inputs: &BTreeMap<String, String>,
    ) -> Result<(), ForgeError> {
        let path = format!("repos/{repo}/actions/workflows/{}/dispatches", encode(workflow));
        match self.api.send_json("POST", &path, json!({"ref": git_ref, "inputs": inputs})) {
            Ok(_) => Ok(()),
            Err(GhError::Status(404, m)) => Err(ForgeError::Failed(format!(
                "GitHub has no workflow {workflow} in {repo} (or PDT's GitHub login can't start it): {m}"
            ))),
            Err(e) => Err(e.into()),
        }
    }
    fn workflow_runs(
        &self,
        repo: &str,
        workflow: &str,
        branch: Option<&str>,
        completed: bool,
    ) -> Result<Vec<WorkflowRun>, ForgeError> {
        let id = self.workflow_id(repo, workflow)?;
        let status = if completed { "status=completed&" } else { "" };
        let mut path =
            format!("repos/{repo}/actions/workflows/{}/runs?{status}per_page={WORKFLOW_RUN_LIMIT}", encode(&id));
        if let Some(b) = branch.map(str::trim).filter(|b| !b.is_empty()) {
            path.push_str(&format!("&branch={}", encode(b)));
        }
        let v = match self.api.get(&path) {
            Ok(v) => v,
            Err(GhError::Status(404, m)) => {
                return Err(ForgeError::Failed(format!("GitHub has no workflow {workflow} in {repo}: {m}")));
            }
            Err(e) => return Err(e.into()),
        };
        let list = v["workflow_runs"].as_array().ok_or_else(|| GhError::BadAnswer("no workflow_runs".into()))?;
        Ok(list.iter().filter_map(parse_workflow_run).collect())
    }
    fn workflow_run(&self, repo: &str, id: &str) -> Result<Option<WorkflowRun>, ForgeError> {
        match self.api.get(&format!("repos/{repo}/actions/runs/{}", encode(id))) {
            Ok(v) => Ok(parse_workflow_run(&v)),
            Err(GhError::Status(404, _)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn queue_runs(&self, repo: &str) -> Result<Vec<CiRun>, ForgeError> {
        Ok(parse_runs(&self.api.get(&format!("repos/{repo}/actions/runs?event=merge_group&per_page={RUN_LIMIT}"))?)?)
    }
}
