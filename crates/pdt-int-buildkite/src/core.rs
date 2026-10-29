//! Buildkite CI (Phase 7e spec §6.4, PDT-85): builds as the REST API returns them, as provider-neutral CI runs
//! ([`CiRun`]) and jobs ([`CiJob`]). Pure: [`crate::ci::BuildkiteCi`] reads Buildkite.

use pdt_core::pr::{CiJob, CiRun, CiState};
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

/// A build's job, if it is one that runs (a script step).
pub fn job_of(j: &Value) -> Option<CiJob> {
    if j["type"].as_str() != Some("script") || j["retried"].as_bool() == Some(true) {
        return None;
    }
    let state = match j["state"].as_str().unwrap_or_default() {
        _ if failed_job(j) => CiState::Failed,
        "passed" => CiState::Passed,
        // Soft failures don't fail the build.
        "failed" | "broken" | "timed_out" => CiState::Passed,
        "running" | "assigned" | "accepted" | "canceling" | "timing_out" => CiState::Running,
        "canceled" | "skipped" | "not_run" | "expired" => CiState::Cancelled,
        _ => CiState::Queued,
    };
    let name = j["name"].as_str().or(j["label"].as_str()).or(j["step_key"].as_str()).unwrap_or("job").to_string();
    Some(CiJob {
        id: j["id"].as_str()?.to_string(),
        name,
        state,
        url: j["web_url"].as_str().unwrap_or_default().into(),
    })
}

/// The builds of `pipeline` as CI runs: `id` `pipeline/number`, `attempt` 1 plus its jobs' retries (so a retried build
/// is news again), `name` the pipeline.
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
                provider: BUILDKITE.into(),
                id: format!("{pipeline}/{number}"),
                attempt: u32::try_from(1 + retries).unwrap_or(u32::MAX),
                name: pipeline.to_string(),
                state,
                url: b["web_url"].as_str().unwrap_or_default().to_string(),
                head_sha: b["commit"].as_str().unwrap_or_default().to_string(),
                branch: b["branch"].as_str().unwrap_or_default().to_string(),
                created_at: b["created_at"].as_str().unwrap_or_default().to_string(),
                failed_jobs,
            })
        })
        .collect()
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
        assert_eq!((runs[0].id.as_str(), runs[0].attempt, runs[0].name.as_str()), ("app/3", 2, "app"));
        assert_eq!((runs[0].provider.as_str(), runs[0].branch.as_str()), ("buildkite", "b"));
        assert_eq!(runs[0].failed_jobs, ["test"]);
        assert_eq!(runs[1].state, CiState::Passed);
        let ci = pdt_core::pr::CiStatus::of(&runs, "h").unwrap();
        assert_eq!((ci.state, ci.runs.len()), (CiState::Failed, 1), "the newest build of the pipeline");
    }

    #[test]
    fn a_builds_jobs_are_its_script_steps() {
        let jobs = [
            json!({"id": "j1", "type": "script", "name": "test", "state": "failed", "web_url": "u1"}),
            json!({"id": "j2", "type": "script", "label": "lint", "state": "failed", "soft_failed": true}),
            json!({"id": "j3", "type": "script", "name": "old", "state": "failed", "retried": true}),
            json!({"id": "j4", "type": "waiter"}),
            json!({"id": "j5", "type": "script", "name": "deploy", "state": "scheduled"}),
        ];
        let got: Vec<CiJob> = jobs.iter().filter_map(job_of).collect();
        assert_eq!(got.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["j1", "j2", "j5"]);
        assert_eq!((got[0].state, got[0].url.as_str()), (CiState::Failed, "u1"));
        assert_eq!((got[1].name.as_str(), got[1].state), ("lint", CiState::Passed), "a soft failure passes");
        assert_eq!(got[2].state, CiState::Queued);
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
