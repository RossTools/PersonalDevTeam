//! Buildkite CI (PDT-85) as a CI provider (PDT-242). A project names a `buildkite` connection and
//! its pipelines in its host settings; then [`BuildkiteCi`] is one of its CI providers, which the PR poller reads every
//! 60 s on each active attempt's head (its PR's head, else the pushed branch, else the local one, as Buildkite builds
//! branches): rolled up with the PR's other CI for the badge (or `attempt.ci` without a PR), and logged as
//! `ci.run_failed` / `ci.run_passed` with `provider: buildkite`. A failing connection is reported once
//! ([`pdt_api::sdk::connections`]).

use crate::core::{API_URL, BUILDKITE, BuildkiteSettings, job_of, retryable_jobs, runs_of};
use pdt_api::sdk::AppState;
use pdt_api::sdk::http::{client, read_body, send_error};
use pdt_core::pr::{CiJob, CiRun};
use pdt_forge::{CiCapabilities, CiConfig, CiProvider, ForgeError};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// A `buildkite` connection ready to call: the API, its organization and token.
pub(crate) struct BuildkiteClient {
    base: String,
    org: String,
    token: String,
    /// The longest one call may take (`AppState::sensor_timeout`).
    timeout: std::time::Duration,
}

impl BuildkiteClient {
    pub(crate) async fn named(st: &AppState, name: &str) -> Result<BuildkiteClient, String> {
        let (conn, secret) = pdt_api::sdk::connections::connection_with_secret(st, name).await?;
        if conn.kind != BUILDKITE {
            return Err(format!("{} is a {} connection, not {BUILDKITE}", conn.name, conn.kind));
        }
        let org = conn.config.get("org").and_then(|o| o.as_str()).unwrap_or_default().trim().to_string();
        if org.is_empty() {
            return Err(format!("{} names no Buildkite organization", conn.name));
        }
        let token = secret.filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("{} has no API token", conn.name))?;
        let base = match conn.url.trim().trim_end_matches('/') {
            "" => API_URL.to_string(),
            u => u.to_string(),
        };
        Ok(BuildkiteClient { base, org, token, timeout: st.sensor_timeout })
    }

    fn pipeline_url(&self, pipeline: &str) -> String {
        format!("{}/v2/organizations/{}/pipelines/{}", self.base, self.org, pipeline.trim())
    }

    async fn call(
        &self,
        method: reqwest::Method,
        url: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Value, String> {
        let mut req = client().request(method.clone(), url).bearer_auth(&self.token).timeout(self.timeout);
        if !query.is_empty() {
            req = req.query(query);
        }
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| format!("Buildkite {method} {url}: {}", send_error(e)))?;
        let status = resp.status().as_u16();
        let bytes = read_body(resp, 8 * 1024 * 1024).await?;
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            let msg = v["message"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| String::from_utf8_lossy(&bytes).chars().take(300).collect());
            return Err(format!("Buildkite answered {status} to {method} {url}: {msg}"));
        }
        Ok(v)
    }

    /// The builds of `pipeline` for `branch` (at `commit`), newest first.
    pub(crate) async fn builds(
        &self,
        pipeline: &str,
        branch: &str,
        commit: Option<&str>,
    ) -> Result<Vec<Value>, String> {
        let url = format!("{}/builds", self.pipeline_url(pipeline));
        let mut query = vec![("branch", branch), ("per_page", "30")];
        if let Some(c) = commit {
            query.insert(1, ("commit", c));
        }
        let v = self.call(reqwest::Method::GET, &url, &query, None).await?;
        Ok(v.as_array().cloned().unwrap_or_default())
    }

    /// A job's log (its text).
    pub(crate) async fn job_log(&self, pipeline: &str, number: u64, job: &str) -> Result<String, String> {
        let url = format!("{}/builds/{number}/jobs/{job}/log", self.pipeline_url(pipeline));
        let v = self.call(reqwest::Method::GET, &url, &[], None).await?;
        Ok(v["content"].as_str().unwrap_or_default().to_string())
    }

    pub(crate) async fn build(&self, pipeline: &str, number: u64) -> Result<Value, String> {
        let url = format!("{}/builds/{number}", self.pipeline_url(pipeline));
        self.call(reqwest::Method::GET, &url, &[], None).await
    }

    pub(crate) async fn retry_job(&self, pipeline: &str, number: u64, job: &str) -> Result<Value, String> {
        let url = format!("{}/builds/{number}/jobs/{job}/retry", self.pipeline_url(pipeline));
        self.call(reqwest::Method::PUT, &url, &[], None).await
    }

    pub(crate) async fn rebuild(&self, pipeline: &str, number: u64) -> Result<Value, String> {
        let url = format!("{}/builds/{number}/rebuild", self.pipeline_url(pipeline));
        self.call(reqwest::Method::PUT, &url, &[], None).await
    }

    pub(crate) async fn create_build(&self, pipeline: &str, body: &Value) -> Result<Value, String> {
        let url = format!("{}/builds", self.pipeline_url(pipeline));
        self.call(reqwest::Method::POST, &url, &[], Some(body)).await
    }
}

/// A project's Buildkite pipelines as a CI provider (PDT-242).
pub struct BuildkiteCi {
    st: AppState,
    rt: tokio::runtime::Handle,
    settings: BuildkiteSettings,
}

impl BuildkiteCi {
    /// The provider of a project with these settings; None without pipelines. Made on the async runtime.
    pub fn new(st: &AppState, settings: BuildkiteSettings) -> Option<Self> {
        if settings.pipelines.is_empty() {
            return None;
        }
        Some(BuildkiteCi { st: st.clone(), rt: tokio::runtime::Handle::current(), settings })
    }

    /// Runs `f` with the connection's client from the blocking thread a provider is called on, and reports how the
    /// connection did.
    fn with_client<T>(&self, f: impl AsyncFnOnce(&BuildkiteClient) -> Result<T, String>) -> Result<T, ForgeError> {
        let name = self.settings.connection.clone();
        self.rt.block_on(async {
            let got = match BuildkiteClient::named(&self.st, &name).await {
                Ok(c) => f(&c).await,
                Err(e) => Err(e),
            };
            pdt_api::sdk::connections::report(&self.st, &name, got.as_ref().err().map(String::as_str)).await;
            got.map_err(ForgeError::Failed)
        })
    }
}

/// A run's pipeline and build number (its id is `pipeline/number`).
fn build_of(run: &CiRun) -> Result<(String, u64), ForgeError> {
    run.id
        .split_once('/')
        .and_then(|(p, n)| Some((p.to_string(), n.parse().ok()?)))
        .ok_or_else(|| ForgeError::Failed(format!("{} is not a Buildkite build", run.id)))
}

impl CiProvider for BuildkiteCi {
    fn id(&self) -> &str {
        BUILDKITE
    }

    fn label(&self) -> &str {
        "Buildkite"
    }

    fn capabilities(&self) -> CiCapabilities {
        CiCapabilities { branches: true, logs: true, retry: true, trigger: true, merge_queue: false }
    }

    /// Set up for the project: its pipelines build every branch.
    fn config_paths(&self) -> CiConfig {
        CiConfig::Known(true)
    }

    fn runs(&self, _repo: &str, branch: &str, sha: Option<&str>) -> Result<Vec<CiRun>, ForgeError> {
        let pipelines = self.settings.pipelines.clone();
        self.with_client(async |c| {
            let mut runs = vec![];
            for p in &pipelines {
                runs.extend(runs_of(p, &c.builds(p, branch, sha).await?));
            }
            Ok(runs)
        })
    }

    fn jobs(&self, _repo: &str, run: &CiRun) -> Result<Vec<CiJob>, ForgeError> {
        let (pipeline, number) = build_of(run)?;
        let build = self.with_client(async |c| c.build(&pipeline, number).await)?;
        Ok(build["jobs"].as_array().into_iter().flatten().filter_map(job_of).collect())
    }

    fn job_log_tail(&self, _repo: &str, run: &CiRun, job: &CiJob, lines: usize) -> Result<String, ForgeError> {
        let (pipeline, number) = build_of(run)?;
        let log = self.with_client(async |c| c.job_log(&pipeline, number, &job.id).await)?;
        let all: Vec<&str> = log.lines().collect();
        let skipped = all.len().saturating_sub(lines);
        let mut out: String = all[skipped..].iter().map(|l| format!("{l}\n")).collect();
        if skipped > 0 {
            out = format!("… ({skipped} earlier lines)\n{out}");
        }
        Ok(out)
    }

    /// The failed jobs again (`failed_only`, when some can be), else the whole build.
    fn retry(&self, _repo: &str, run: &CiRun, failed_only: bool) -> Result<(), ForgeError> {
        let (pipeline, number) = build_of(run)?;
        self.with_client(async |c| {
            let jobs = if failed_only { retryable_jobs(&c.build(&pipeline, number).await?) } else { vec![] };
            if jobs.is_empty() {
                c.rebuild(&pipeline, number).await?;
            }
            for (id, _) in &jobs {
                c.retry_job(&pipeline, number, id).await?;
            }
            Ok(())
        })
    }

    /// A build of pipeline `workflow` on branch `git_ref`'s newest commit, `inputs` as its environment.
    fn trigger(
        &self,
        _repo: &str,
        git_ref: &str,
        workflow: &str,
        inputs: &BTreeMap<String, String>,
    ) -> Result<(), ForgeError> {
        let body = json!({"commit": "HEAD", "branch": git_ref, "message": "Started by PDT", "env": inputs});
        self.with_client(async |c| c.create_build(workflow, &body).await.map(|_| ()))
    }
}
