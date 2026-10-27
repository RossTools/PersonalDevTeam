//! Buildkite CI (Phase 7e spec §6.4, PDT-85): builds as the REST API returns them, as provider-neutral CI runs
//! ([`CiRun`]), and the notice PDT tells the agent when they fail. Pure: pdt-api reads Buildkite and logs the events.

use pdt_core::pr::{CiRun, CiState, CiStatus, Notice, NoticeKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// The connection kind, and the CI provider of the events.
pub const BUILDKITE: &str = "buildkite";

/// Buildkite's REST API, unless a connection names another address.
pub const API_URL: &str = "https://api.buildkite.com";

/// A project's Buildkite pipelines on this host: its `buildkite` integration settings, read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BuildkiteSettings {
    /// A `buildkite` connection's name.
    pub connection: String,
    /// The pipelines (slugs) that build the project's branches.
    #[serde(default)]
    pub pipelines: Vec<String>,
}

/// A build's `state`.
pub fn build_state(state: &str) -> CiState {
    match state {
        "passed" => CiState::Passed,
        "failed" => CiState::Failed,
        // Still running: failed once it finishes.
        "running" | "failing" | "canceling" => CiState::Running,
        "canceled" | "skipped" | "not_run" => CiState::Cancelled,
        // `scheduled`, `creating`, and `blocked` (waiting for someone to unblock it).
        _ => CiState::Queued,
    }
}

/// A job that failed the build (not a soft failure).
fn failed_job(j: &Value) -> bool {
    j["type"].as_str() == Some("script")
        && matches!(j["state"].as_str(), Some("failed" | "broken" | "timed_out"))
        && !j["soft_failed"].as_bool().unwrap_or(false)
        && j["retried"].as_bool() != Some(true)
}

/// The failed jobs of a build that Buildkite can retry: `(id, name)`.
pub fn retryable_jobs(build: &Value) -> Vec<(String, String)> {
    build["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|j| failed_job(j) && j["retryable"].as_bool() != Some(false))
        .filter_map(|j| Some((j["id"].as_str()?.to_string(), j["name"].as_str().unwrap_or("job").to_string())))
        .collect()
}

/// A build's pipeline and number from its web URL (`https://buildkite.com/<org>/<pipeline>/builds/<n>`).
pub fn parse_build_url(url: &str) -> Option<(String, u64)> {
    let path = url.split("://").nth(1)?.split(['?', '#']).next()?;
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    match parts.as_slice() {
        [_, _org, pipeline, "builds", n, ..] => Some(((*pipeline).to_string(), n.parse().ok()?)),
        _ => None,
    }
}

/// The builds of `pipeline` as CI runs: `id` the build number, `attempt` 1 plus its jobs' retries (so a retried build
/// is news again), `workflow` the pipeline.
pub fn runs_of(pipeline: &str, builds: &[Value]) -> Vec<CiRun> {
    builds
        .iter()
        .filter_map(|b| {
            let number = b["number"].as_u64()?;
            let jobs = b["jobs"].as_array().map(Vec::as_slice).unwrap_or_default();
            let retries: u64 = jobs.iter().filter_map(|j| j["retries_count"].as_u64()).sum();
            let state = build_state(b["state"].as_str().unwrap_or_default());
            let failed_jobs = if state == CiState::Failed {
                jobs.iter()
                    .filter(|j| failed_job(j))
                    .map(|j| j["name"].as_str().or(j["label"].as_str()).or(j["step_key"].as_str()).unwrap_or("job"))
                    .map(String::from)
                    .collect()
            } else {
                vec![]
            };
            Some(CiRun {
                id: number,
                attempt: u32::try_from(1 + retries).unwrap_or(u32::MAX),
                workflow: pipeline.to_string(),
                state,
                url: b["web_url"].as_str().unwrap_or_default().to_string(),
                head_sha: b["commit"].as_str().unwrap_or_default().to_string(),
                head_branch: b["branch"].as_str().unwrap_or_default().to_string(),
                created_at: b["created_at"].as_str().unwrap_or_default().to_string(),
                failed_jobs,
            })
        })
        .collect()
}

/// The ids in a Buildkite event's key: `<pipeline>#<number>.<attempt>` per run.
pub fn run_ids(runs: &[&CiRun]) -> String {
    runs.iter().map(|r| format!("{}#{}.{}", r.workflow, r.id, r.attempt)).collect::<Vec<_>>().join(",")
}

/// What to tell the agent when Buildkite CI failed on `head` of `branch` (and its PR, if it has one).
pub fn failed_notice(ci: &CiStatus, branch: &str, pr: Option<u32>, head: &str) -> Option<Notice> {
    if ci.state != CiState::Failed {
        return None;
    }
    let failed: Vec<&CiRun> = ci.failed().collect();
    let jobs: Vec<String> = failed.iter().flat_map(|r| r.failed_jobs.iter().cloned()).collect();
    let short = &head[..head.len().min(7)];
    let on = match pr {
        Some(n) => format!("PR #{n} ({branch})"),
        None => branch.to_string(),
    };
    let mut text = format!("[PDT] Buildkite CI failed on {on} at {short}");
    if !jobs.is_empty() {
        text.push_str(&format!(": {}", jobs.join(", ")));
    }
    text.push_str(&format!(". Build: {}. Read the failed jobs' logs there, fix it, run the checks and push.", ci.url));
    Some(Notice {
        kind: NoticeKind::CiFailed,
        key: format!("bk_failed:{}", run_ids(&failed)),
        pr,
        url: Some(ci.url.clone()),
        jobs,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn build(number: u64, state: &str, commit: &str, jobs: Value) -> Value {
        json!({"number": number, "state": state, "commit": commit, "branch": "b",
            "web_url": format!("https://buildkite.com/acme/app/builds/{number}"),
            "created_at": format!("2026-10-22T10:00:{number:02}Z"), "jobs": jobs})
    }

    #[test]
    fn builds_become_runs_with_their_failed_jobs() {
        let jobs = json!([
            {"type": "script", "name": "test", "state": "failed", "retries_count": 1},
            {"type": "script", "name": "lint", "state": "failed", "soft_failed": true},
            {"type": "script", "name": "old try", "state": "failed", "retried": true},
            {"type": "waiter"},
            {"type": "script", "name": "build", "state": "passed"}]);
        let runs = runs_of("app", &[build(3, "failed", "h", jobs), build(2, "passed", "h", json!([]))]);
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].id, runs[0].attempt, runs[0].workflow.as_str()), (3, 2, "app"));
        assert_eq!(runs[0].failed_jobs, ["test"]);
        assert_eq!(runs[1].state, CiState::Passed);
        let ci = CiStatus::of(&runs, "h").unwrap();
        assert_eq!((ci.state, ci.runs.len()), (CiState::Failed, 1), "the newest build of the pipeline");
        let n = failed_notice(&ci, "b", Some(4), "abcdef1234").unwrap();
        assert_eq!(n.key, "bk_failed:app#3.2");
        assert!(
            n.text.contains("PR #4 (b) at abcdef1: test. Build: https://buildkite.com/acme/app/builds/3"),
            "{}",
            n.text
        );
        assert!(failed_notice(&ci, "b", None, "h").unwrap().text.contains("failed on b at h"));
    }

    #[test]
    fn providers_merge_into_one_status() {
        let bk = CiStatus::of(&runs_of("app", &[build(1, "failed", "h", json!([]))]), "h");
        let gh = CiStatus::of(&runs_of("gh", &[build(9, "passed", "h", json!([]))]), "h");
        let both = CiStatus::merge(gh.clone(), bk.clone()).unwrap();
        assert_eq!((both.state, both.runs.len()), (CiState::Failed, 2));
        assert!(both.url.ends_with("/builds/1"));
        assert_eq!(CiStatus::merge(None, bk.clone()), bk);
        assert_eq!(CiStatus::merge(gh.clone(), None), gh);
    }

    #[test]
    fn retryable_jobs_and_build_urls() {
        let b = json!({"jobs": [
            {"id": "j1", "type": "script", "name": "test", "state": "failed", "retryable": true},
            {"id": "j2", "type": "script", "name": "lint", "state": "failed", "retryable": false},
            {"id": "j3", "type": "script", "name": "build", "state": "passed"}]});
        assert_eq!(retryable_jobs(&b), [("j1".to_string(), "test".to_string())]);
        assert_eq!(parse_build_url("https://buildkite.com/acme/app/builds/42"), Some(("app".into(), 42)));
        assert_eq!(parse_build_url("https://buildkite.com/acme/app/builds/42#job"), Some(("app".into(), 42)));
        assert_eq!(parse_build_url("https://buildkite.com/acme/app"), None);
        assert_eq!(parse_build_url("42"), None);
    }

    #[test]
    fn states_map_to_ci_states() {
        assert_eq!(build_state("passed"), CiState::Passed);
        assert_eq!(build_state("failing"), CiState::Running);
        assert_eq!(build_state("blocked"), CiState::Queued);
        assert_eq!(build_state("canceled"), CiState::Cancelled);
        assert_eq!(build_state("failed"), CiState::Failed);
    }
}
