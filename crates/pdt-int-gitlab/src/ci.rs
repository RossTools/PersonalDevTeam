//! GitLab CI as a CI provider (PDT-246): a branch's pipelines (pushed branches' and merge requests' alike), their jobs
//! and logs (`/jobs/:id/trace`), retrying a pipeline's failed jobs, starting a pipeline, and a merge train's pipelines,
//! through the same signed-in [`GitlabApi`] as the forge. The GitLab plugin registers it as GitLab's own CI.

use crate::api::{GitlabApi, GlError, project_path, s};
use pdt_core::pr::{CiJob, CiRun, CiState};
use pdt_forge::transport::encode;
use pdt_forge::{CiCapabilities, CiConfig, CiProvider, ForgeError};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

/// GitLab CI's provider id, which `[ci] providers` names.
pub const GITLAB_CI: &str = "gitlab_ci";

/// What GitLab CI can do: everything. Every pushed branch gets a pipeline, so its runs are read before the branch has
/// a merge request.
pub const CAPABILITIES: CiCapabilities =
    CiCapabilities { branches: true, logs: true, retry: true, trigger: true, merge_queue: true };

/// Where a project keeps its pipeline configuration.
pub const CI_FILES: &[&str] = &[".gitlab-ci.yml"];

/// How many recent pipelines are read per branch (and of merge requests' pipelines per project).
const RUN_LIMIT: u32 = 30;

/// The most lines of a failed job's log kept, and of characters.
pub const LOG_CHARS: usize = 30_000;

pub struct GitlabCi {
    api: Arc<GitlabApi>,
}

impl GitlabCi {
    pub fn new(api: Arc<GitlabApi>) -> Self {
        Self { api }
    }
}

/// A pipeline's or a job's `status`.
pub fn state_of(status: &str) -> CiState {
    match status {
        "success" | "skipped" => CiState::Passed,
        "failed" => CiState::Failed,
        "canceled" | "canceling" => CiState::Cancelled,
        "running" => CiState::Running,
        // created, waiting_for_resource, preparing, pending, scheduled, and manual (waiting for someone to start a
        // job it needs, as GitLab's "pipeline must succeed" waits).
        _ => CiState::Queued,
    }
}

/// One pipeline of a pipelines answer.
pub fn parse_pipeline(v: &Value) -> Option<CiRun> {
    Some(CiRun {
        provider: GITLAB_CI.into(),
        id: v["id"].as_u64()?.to_string(),
        attempt: 1,
        name: Some(s(v, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| "pipeline".into()),
        state: state_of(v["status"].as_str().unwrap_or_default()),
        url: s(v, "web_url"),
        head_sha: s(v, "sha"),
        branch: s(v, "ref"),
        created_at: s(v, "created_at"),
        failed_jobs: vec![],
    })
}

pub fn parse_pipelines(v: &Value) -> Result<Vec<CiRun>, GlError> {
    let list = v.as_array().ok_or_else(|| GlError::BadAnswer("pipelines are not an array".into()))?;
    Ok(list.iter().filter_map(parse_pipeline).collect())
}

/// One job of a pipeline's jobs. A failure the pipeline allows (`allow_failure`) is not one to fix.
pub fn parse_job(v: &Value) -> Option<CiJob> {
    let state = match state_of(v["status"].as_str().unwrap_or_default()) {
        CiState::Failed if v["allow_failure"].as_bool() == Some(true) => CiState::Passed,
        st => st,
    };
    Some(CiJob { id: v["id"].as_u64()?.to_string(), name: s(v, "name"), state, url: s(v, "web_url") })
}

/// The end of a job's log: its last `keep` lines (at most [`LOG_CHARS`] characters), without GitLab's section markers
/// (`section_start:…\r\x1b[0K`), overwritten progress and terminal colours.
pub fn log_tail(raw: &str, keep: usize) -> String {
    let lines: Vec<String> = raw.lines().map(clean_line).collect();
    let skipped = lines.len().saturating_sub(keep);
    let mut out: String = lines[skipped..].iter().map(|l| format!("{l}\n")).collect();
    let chars = out.chars().count();
    if chars > LOG_CHARS {
        out = out.chars().skip(chars - LOG_CHARS).collect();
    }
    if skipped > 0 {
        out = format!("… ({skipped} earlier lines)\n{out}");
    }
    out
}

fn clean_line(l: &str) -> String {
    // A `\r` overwrites the line: what follows the last one shows.
    let l = l.trim_end_matches('\r');
    let l = l.rsplit('\r').next().unwrap_or(l);
    let mut out = String::with_capacity(l.len());
    let mut chars = l.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl GitlabCi {
    /// The pipeline a merge request on `branch` last ran on its merged result (merged-results pipelines run on a merge
    /// commit, not the head), as a run of the head `sha` it was for: GitLab's `head_pipeline` is the current head's.
    fn merged_result_run(&self, repo: &str, branch: &str, sha: &str) -> Result<Option<CiRun>, GlError> {
        let list = self.api.get(&format!(
            "{}/merge_requests?source_branch={}&state=opened&per_page=1",
            project_path(repo),
            encode(branch)
        ))?;
        let Some(iid) = list.as_array().and_then(|l| l.first()).and_then(|m| m["iid"].as_u64()) else {
            return Ok(None);
        };
        let mr = self.api.get(&format!("{}/merge_requests/{iid}", project_path(repo)))?;
        if s(&mr, "sha") != sha {
            return Ok(None);
        }
        Ok(parse_pipeline(&mr["head_pipeline"])
            .filter(|p| p.head_sha != sha)
            .map(|p| CiRun { head_sha: sha.into(), ..p }))
    }
}

impl CiProvider for GitlabCi {
    fn id(&self) -> &str {
        GITLAB_CI
    }
    fn label(&self) -> &str {
        "GitLab CI"
    }
    fn capabilities(&self) -> CiCapabilities {
        CAPABILITIES
    }
    fn config_paths(&self) -> CiConfig {
        CiConfig::Paths(CI_FILES)
    }
    /// The commit's pipelines (the branch's and its merge request's), with a merged-results pipeline counted as the
    /// head's; without `sha`, the branch's recent ones.
    fn runs(&self, repo: &str, branch: &str, sha: Option<&str>) -> Result<Vec<CiRun>, ForgeError> {
        let filter = match sha {
            Some(h) => format!("sha={}", encode(h)),
            None => format!("ref={}", encode(branch)),
        };
        let path = format!("{}/pipelines?{filter}&order_by=id&sort=desc&per_page={RUN_LIMIT}", project_path(repo));
        let mut runs = parse_pipelines(&self.api.get(&path)?)?;
        if let Some(h) = sha
            && self.api.project(repo).is_ok_and(|p| p.merged_results)
            && let Some(r) = self.merged_result_run(repo, branch, h)?
            && !runs.iter().any(|x| x.id == r.id)
        {
            runs.push(r);
        }
        Ok(runs)
    }
    fn jobs(&self, repo: &str, run: &CiRun) -> Result<Vec<CiJob>, ForgeError> {
        let jobs = self.api.get_pages(&format!("{}/pipelines/{}/jobs?per_page=100", project_path(repo), run.id))?;
        Ok(jobs.iter().filter_map(parse_job).collect())
    }
    fn job_log_tail(&self, repo: &str, _run: &CiRun, job: &CiJob, lines: usize) -> Result<String, ForgeError> {
        match self.api.call("GET", &format!("{}/jobs/{}/trace", project_path(repo), job.id), None) {
            Ok(res) => Ok(log_tail(&res.body, lines)),
            // Logs expire (and a job that never started has none): say so instead of failing.
            Err(GlError::Status(404 | 410, _)) => Ok("(no log: GitLab has none for this job)".into()),
            Err(e) => Err(e.into()),
        }
    }
    /// GitLab retries a pipeline's failed and cancelled jobs; all of a branch's jobs again is a new pipeline on it (a
    /// merge request's pipeline can only be retried).
    fn retry(&self, repo: &str, run: &CiRun, failed_only: bool) -> Result<(), ForgeError> {
        if !failed_only && !run.branch.is_empty() && !run.branch.starts_with("refs/") {
            self.api.send_json("POST", &format!("{}/pipeline", project_path(repo)), json!({"ref": run.branch}))?;
            return Ok(());
        }
        self.api.send_json("POST", &format!("{}/pipelines/{}/retry", project_path(repo), run.id), json!({}))?;
        Ok(())
    }
    /// A new pipeline on `git_ref`, with `inputs` as its CI/CD variables. GitLab has one pipeline configuration per
    /// project, so `workflow` only names it.
    fn trigger(
        &self,
        repo: &str,
        git_ref: &str,
        workflow: &str,
        inputs: &BTreeMap<String, String>,
    ) -> Result<(), ForgeError> {
        let variables: Vec<Value> = inputs.iter().map(|(k, v)| json!({"key": k, "value": v})).collect();
        let path = format!("{}/pipeline", project_path(repo));
        match self.api.send_json("POST", &path, json!({"ref": git_ref, "variables": variables})) {
            Ok(_) => Ok(()),
            Err(GlError::Status(code @ (400 | 404), m)) => Err(ForgeError::Failed(format!(
                "GitLab started no pipeline ({workflow}) on {git_ref} in {repo} ({code}: {m})"
            ))),
            Err(e) => Err(e.into()),
        }
    }
    /// Merge requests' recent pipelines: a merge train's run on `refs/merge-requests/:iid/train`.
    fn queue_runs(&self, repo: &str) -> Result<Vec<CiRun>, ForgeError> {
        let path = format!(
            "{}/pipelines?source=merge_request_event&order_by=id&sort=desc&per_page={RUN_LIMIT}",
            project_path(repo)
        );
        Ok(parse_pipelines(&self.api.get(&path)?)?)
    }
}
