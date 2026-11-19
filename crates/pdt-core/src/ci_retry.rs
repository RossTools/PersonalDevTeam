//! An agent retrying its own CI jobs (PDT-629, `ci_retry`): which jobs it named, and the guards. Only its attempt's
//! current head is retried (re-running an older commit's jobs cancels the head's run where CI keeps one run per
//! branch, PDT-510), each job at most [`RETRY_CAP`] times per head, and every retry is a `ci_retry` event on the
//! ticket's timeline, which the cap counts. Pure: pdt-api reads CI and records the events.

use crate::pr::{CiJob, CiState};
use serde_json::Value;

/// How many times an agent may retry one job on one head commit; then it fixes the cause or asks the user.
pub const RETRY_CAP: usize = 2;

/// A commit as people read it.
pub fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// Whether `job` is the one `wanted` names: its id, its step key, or its name (any case).
pub fn names(job: &CiJob, wanted: &str) -> bool {
    let w = wanted.trim();
    job.id == w || job.key.as_deref() == Some(w) || job.name.eq_ignore_ascii_case(w)
}

/// The jobs to retry, as indexes into `jobs` (the head's jobs, from every run on it): those `wanted` names (each by id,
/// step key or name), and with `failed` every failed one. Each must have failed or been cancelled, and be retryable.
pub fn pick(jobs: &[CiJob], wanted: &[String], failed: bool) -> Result<Vec<usize>, String> {
    let wanted: Vec<&str> = wanted.iter().map(|w| w.trim()).filter(|w| !w.is_empty()).collect();
    if wanted.is_empty() && !failed {
        return Err("name the jobs to retry (`jobs`), or give `failed: true` for every failed one".into());
    }
    let mut out: Vec<usize> = vec![];
    for w in &wanted {
        let found: Vec<usize> = (0..jobs.len()).filter(|&i| names(&jobs[i], w)).collect();
        match found.as_slice() {
            [] => {
                let all = jobs.iter().map(|j| j.name.as_str()).collect::<Vec<_>>().join(", ");
                return Err(format!("no job on the head is {w:?} (its jobs: {all})"));
            }
            [i] => out.push(*i),
            many => {
                let ids = many.iter().map(|&i| jobs[i].id.as_str()).collect::<Vec<_>>().join(", ");
                return Err(format!("{w:?} names {} jobs (ids {ids}): give the one's id", many.len()));
            }
        }
    }
    if failed {
        out.extend((0..jobs.len()).filter(|&i| jobs[i].state == CiState::Failed));
        if out.is_empty() {
            return Err("no job on the head has failed".into());
        }
    }
    let mut picked: Vec<usize> = vec![];
    for i in out {
        if !picked.contains(&i) {
            picked.push(i);
        }
    }
    for &i in &picked {
        let j = &jobs[i];
        match j.state {
            CiState::Failed | CiState::Cancelled if j.retryable => {}
            CiState::Failed | CiState::Cancelled => {
                return Err(format!(
                    "{} can't be retried on its own (it was retried already, or CI doesn't allow it)",
                    j.name
                ));
            }
            CiState::Queued | CiState::Running => return Err(format!("{} hasn't finished yet", j.name)),
            CiState::Passed => {
                return Err(format!("{} passed (or only soft-failed): there is nothing to retry", j.name));
            }
        }
    }
    Ok(picked)
}

/// Refuses a run on another commit than the attempt's head.
pub fn check_head(run_id: &str, run_sha: &str, head: &str) -> Result<(), String> {
    if run_sha.is_empty() || run_sha == head {
        return Ok(());
    }
    Err(format!(
        "run {run_id} is on commit {}, not your branch's head {}: only the head's CI is retried (re-running an older \
         commit's jobs cancels the head's run, and the pull request then waits on nothing). Retry the head's jobs \
         (`ci_jobs` lists them), or push",
        short_sha(run_sha),
        short_sha(head)
    ))
}

/// The data of a `ci_retry` event: the attempt, its head, the provider and the jobs retried (by name).
pub fn event_data(
    attempt_id: &str,
    head: &str,
    provider: &str,
    provider_label: &str,
    run_id: &str,
    jobs: &[&str],
) -> Value {
    serde_json::json!({
        "attemptId": attempt_id, "head": head, "provider": provider, "providerLabel": provider_label,
        "runId": run_id, "jobs": jobs,
    })
}

/// How many times job `name` of `provider` was retried on `head` of the attempt, from the ticket's `ci_retry` events'
/// data.
pub fn retries(events: &[Value], attempt_id: &str, head: &str, provider: &str, name: &str) -> usize {
    events
        .iter()
        .filter(|d| d["attemptId"] == attempt_id && d["head"] == head && d["provider"] == provider)
        .filter(|d| d["jobs"].as_array().is_some_and(|js| js.iter().any(|j| j.as_str() == Some(name))))
        .count()
}

/// Refuses a job retried [`RETRY_CAP`] times on this head already.
pub fn check_cap(used: usize, name: &str, head: &str) -> Result<(), String> {
    if used < RETRY_CAP {
        return Ok(());
    }
    Err(format!(
        "{name} was retried {used} times on {} already: it isn't a flake. Fix the cause and push, or ask the user \
         (`ask_user`)",
        short_sha(head)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job(id: &str, name: &str, key: Option<&str>, state: CiState, retryable: bool) -> CiJob {
        CiJob { id: id.into(), name: name.into(), state, url: String::new(), key: key.map(String::from), retryable }
    }

    fn jobs() -> Vec<CiJob> {
        vec![
            job("j1", "test (windows)", Some("test-windows"), CiState::Failed, true),
            job("j2", "lint", None, CiState::Passed, true),
            job("j3", "test (linux)", None, CiState::Failed, true),
            job("j4", "deploy", None, CiState::Running, false),
            job("j5", "old", None, CiState::Failed, false),
        ]
    }

    fn w(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn jobs_are_named_by_id_key_or_name() {
        let js = jobs();
        assert_eq!(pick(&js, &w(&["j1"]), false), Ok(vec![0]));
        assert_eq!(pick(&js, &w(&["test-windows"]), false), Ok(vec![0]));
        assert_eq!(pick(&js, &w(&["Test (Linux)"]), false), Ok(vec![2]));
        assert_eq!(pick(&js, &w(&["j1", "test (windows)"]), false), Ok(vec![0]), "once each");
        assert!(pick(&js, &w(&["nope"]), false).unwrap_err().contains("its jobs: test (windows), lint"));
        assert!(pick(&js, &[], false).unwrap_err().contains("failed: true"));
    }

    #[test]
    fn failed_takes_every_failed_job_that_can_be_retried() {
        let mut js = jobs();
        assert!(pick(&js, &[], true).unwrap_err().contains("old can't be retried"));
        js.pop();
        assert_eq!(pick(&js, &[], true), Ok(vec![0, 2]));
        js.retain(|j| j.state != CiState::Failed);
        assert_eq!(pick(&js, &[], true).unwrap_err(), "no job on the head has failed");
    }

    #[test]
    fn only_finished_unsuccessful_jobs_are_retried() {
        let js = jobs();
        assert!(pick(&js, &w(&["lint"]), false).unwrap_err().contains("nothing to retry"));
        assert!(pick(&js, &w(&["deploy"]), false).unwrap_err().contains("hasn't finished"));
        let two = vec![job("a", "test", None, CiState::Failed, true), job("b", "test", None, CiState::Failed, true)];
        assert!(pick(&two, &w(&["test"]), false).unwrap_err().contains("ids a, b"));
        assert_eq!(pick(&two, &w(&["b"]), false), Ok(vec![1]));
    }

    #[test]
    fn only_the_head_is_retried() {
        assert!(check_head("app/3", "abc1234567", "abc1234567").is_ok());
        assert!(check_head("app/3", "", "abc1234567").is_ok(), "a run without its commit is the head's");
        let e = check_head("app/2", "0123456789", "abc1234567").unwrap_err();
        assert!(e.contains("commit 0123456, not your branch's head abc1234"), "{e}");
    }

    #[test]
    fn the_cap_counts_retries_of_a_job_on_one_head() {
        let ev =
            |head: &str, provider: &str, jobs: &[&str]| event_data("a1", head, provider, "Buildkite", "app/1", jobs);
        let events = vec![
            ev("h1", "buildkite", &["test"]),
            ev("h1", "buildkite", &["test", "lint"]),
            ev("h2", "buildkite", &["test"]),
            ev("h1", "github_actions", &["test"]),
            json!({"attemptId": "a2", "head": "h1", "provider": "buildkite", "jobs": ["test"]}),
        ];
        assert_eq!(retries(&events, "a1", "h1", "buildkite", "test"), 2);
        assert_eq!(retries(&events, "a1", "h1", "buildkite", "lint"), 1);
        assert_eq!(retries(&events, "a1", "h3", "buildkite", "test"), 0);
        assert!(check_cap(1, "lint", "h1").is_ok());
        let e = check_cap(RETRY_CAP, "test", "abcdef0123").unwrap_err();
        assert!(e.contains("test was retried 2 times on abcdef0 already") && e.contains("ask_user"), "{e}");
    }
}
