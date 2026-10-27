//! Buildkite CI (Phase 7e spec §6.4, PDT-85). A project names a `buildkite` connection and its pipelines in its host
//! settings. Every 60 s, after the PR check, pdtd reads the builds of each active attempt's head (its PR's head, else
//! the pushed branch, else the local one) and keeps them for ticket responses (merged with the PR's GitHub CI for
//! the badge, or `attempt.ci` without a PR). It logs `ci.run_failed` / `ci.run_passed` with `provider: buildkite`,
//! each once (its key names the builds and their retries), so the built-in PR notices tell the agent. A failing
//! connection is reported once ([`pdt_api::sdk::connections`]).

use crate::core::{API_URL, BUILDKITE, failed_notice, run_ids, runs_of};
use pdt_api::sdk::http::{client, read_body, send_error};
use pdt_api::sdk::{AppState, now_ms};
use pdt_core::pr::{CiRun, CiState, CiStatus, PrState, PullRequestStatus};
use pdt_core::workflow::api::NewLogEvent;
use pdt_core::workflow::events::{self as ev, CiEvent};
use pdt_store::AttemptInfo;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

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

    /// The builds of `pipeline` for `branch` at `commit`, newest first.
    pub(crate) async fn builds(&self, pipeline: &str, branch: &str, commit: &str) -> Result<Vec<Value>, String> {
        let url = format!("{}/builds", self.pipeline_url(pipeline));
        let v = self
            .call(reqwest::Method::GET, &url, &[("branch", branch), ("commit", commit), ("per_page", "30")], None)
            .await?;
        Ok(v.as_array().cloned().unwrap_or_default())
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

/// The commit Buildkite would build for an attempt: its open PR's head, else the pushed branch, else the local one.
async fn head_of(st: &AppState, info: &AttemptInfo) -> Option<String> {
    if let Some(pr) = pdt_api::sdk::ci::watched_pr(st, &info.attempt.id).filter(|p| p.state == PrState::Open) {
        return Some(pr.head_sha);
    }
    let repo = std::path::PathBuf::from(&info.repo_path);
    let branch = info.attempt.branch.clone();
    tokio::task::spawn_blocking(move || {
        pdt_git::rev_parse(&repo, &format!("refs/remotes/origin/{branch}"))
            .or_else(|| pdt_git::rev_parse(&repo, &format!("refs/heads/{branch}")))
    })
    .await
    .ok()
    .flatten()
}

/// The `ci.*` events of an attempt's Buildkite CI (each once: its key names the builds and their retries).
/// `pr`: the attempt's open pull request; `agent_working`: an agent step works on the attempt now; `merge_waiting`: a
/// workflow's merge waits for this CI.
pub(crate) fn ci_events(
    info: &AttemptInfo,
    ci: &CiStatus,
    pr: Option<&PullRequestStatus>,
    head: &str,
    agent_working: bool,
    merge_waiting: bool,
) -> Vec<NewLogEvent> {
    let a = pdt_api::sdk::ci::attempt_ref(info);
    let merge_requested = pr.is_some_and(|p| pdt_api::sdk::ci::merge_requested(info, p));
    let merge_refused = pr.is_some_and(|p| pdt_api::sdk::ci::merge_refused(info, p));
    let (auto_merge, in_merge_queue) = pr.map(|p| (p.auto_merge, p.in_merge_queue)).unwrap_or_default();
    let pr = pr.map(|p| p.number);
    let event = |notice, jobs| CiEvent {
        attempt: a.clone(),
        provider: BUILDKITE.into(),
        pr,
        head_sha: head.to_string(),
        url: ci.url.clone(),
        runs: pdt_api::sdk::ci::run_refs(ci),
        jobs,
        notice,
        auto_merge,
        in_merge_queue,
        merge_requested,
        merge_refused,
        agent_working,
        merge_waiting,
    };
    match ci.state {
        CiState::Failed => failed_notice(ci, &info.attempt.branch, pr, head)
            .map(|n| {
                let key = format!("{}:{}", a.attempt_id, n.key);
                NewLogEvent::new(ev::CI_RUN_FAILED, key, "buildkite", event(Some(n.clone()), n.jobs))
            })
            .into_iter()
            .collect(),
        CiState::Passed => {
            let all: Vec<&CiRun> = ci.runs.iter().collect();
            let key = format!("{}:bk_passed:{}", a.attempt_id, run_ids(&all));
            vec![NewLogEvent::new(ev::CI_RUN_PASSED, key, "buildkite", event(None, vec![]))]
        }
        _ => vec![],
    }
}

/// One look at the Buildkite CI of every active attempt of a project with Buildkite pipelines. pdtd runs it every
/// 60 s, after the PR check. Never fails: a connection that fails is reported on it, other problems are logged.
pub async fn refresh_buildkite(st: &AppState) {
    let active = match st.store.list_active_attempts().await {
        Ok(v) => v.into_iter().filter(|i| !i.attempt.branch.is_empty()).collect::<Vec<_>>(),
        Err(e) => return tracing::warn!(error = %e, "the Buildkite check could not list active attempts"),
    };
    let ids: HashSet<String> = active.iter().map(|i| i.attempt.id.clone()).collect();
    pdt_api::sdk::ci::keep_other_ci(st, BUILDKITE, &ids);
    let mut settings: HashMap<String, Option<crate::core::BuildkiteSettings>> = HashMap::new();
    let mut uses = pdt_api::sdk::connections::Uses::default();
    for info in active {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let bk = match settings.get(&info.project_id) {
            Some(s) => s.clone(),
            None => {
                let s = st
                    .store
                    .get_project_settings(&info.project_id)
                    .await
                    .ok()
                    .and_then(|s| s.integration::<crate::core::BuildkiteSettings>(BUILDKITE));
                settings.insert(info.project_id.clone(), s.clone());
                s
            }
        };
        let Some(bk) = bk.filter(|b| !b.pipelines.is_empty()) else {
            pdt_api::sdk::ci::set_other_ci(st, BUILDKITE, &info.attempt.id, None);
            continue;
        };
        match refresh_attempt(st, &info, &bk).await {
            Ok(true) => uses.worked(&bk.connection),
            Ok(false) => {}
            Err(e) => uses.failed(&bk.connection, &e),
        }
    }
    uses.report(st).await;
}

/// Reads one attempt's builds; true when it used the connection (false: no head to ask about), the connection's
/// error when that failed.
async fn refresh_attempt(
    st: &AppState,
    info: &AttemptInfo,
    bk: &crate::core::BuildkiteSettings,
) -> Result<bool, String> {
    let Some(head) = head_of(st, info).await else {
        pdt_api::sdk::ci::set_other_ci(st, BUILDKITE, &info.attempt.id, None);
        return Ok(false);
    };
    let c = BuildkiteClient::named(st, &bk.connection).await?;
    let mut runs = vec![];
    for p in &bk.pipelines {
        runs.extend(runs_of(p, &c.builds(p, &info.attempt.branch, &head).await?));
    }
    let ci = CiStatus::of(&runs, &head);
    pdt_api::sdk::ci::set_other_ci(st, BUILDKITE, &info.attempt.id, ci.clone());
    let Some(ci) = ci else { return Ok(true) };
    let pr = pdt_api::sdk::ci::watched_pr(st, &info.attempt.id).filter(|p| p.state == PrState::Open);
    let agent_working = st.store.attempt_has_open_agent_step(&info.attempt.id).await.unwrap_or(false);
    let merge_waiting = st.store.attempt_merge_waits_for_ci(&info.attempt.id).await.unwrap_or(false);
    for e in ci_events(info, &ci, pr.as_ref(), &head, agent_working, merge_waiting) {
        match st.store.emit_event(&e, now_ms()).await {
            Ok(Some(logged)) => pdt_api::sdk::on_event(st, &logged).await,
            Ok(None) => {}
            Err(x) => tracing::warn!(ticket = %info.ticket_key, error = %x, "could not log a Buildkite event"),
        }
    }
    Ok(true)
}
