//! `github.change_request` (PDT-194): read a pull request by branch or number, with its CI when asked. The built-in
//! "Cut a release" polls it between waits (PDT-214: the release PR merges itself, so a failed check is the news).
//! `repo` defaults to the run's project's GitHub repository.

use super::forge_call::{ci_call, forge_call};
use super::repo_of::{RepoOf, check_repo, repo_for_step, templated};
use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, repo_missing, spec,
};
use pdt_core::pr::{CiState, CiStatus, PrState, PullRequestStatus, QueueRemovalReason};
use pdt_forge::{ChangeRequest, ForgeError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubChangeRequestConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The pull request whose head is this branch (the newest open one, else the newest).
    #[serde(default)]
    pub branch: Option<String>,
    /// Or its number. With `ci`, give its head branch too.
    #[serde(default)]
    #[ts(type = "number | string | null")]
    pub number: Option<Value>,
    /// Read its CI too: an open pull request whose CI failed on its head, or that the merge queue removed for failed
    /// checks, leaves by `failed`. Needs `branch` (its head, where CI runs).
    #[serde(default)]
    pub ci: bool,
    /// The port a dry run takes, as it reads nothing: `open` for a branch and `merged` for a number when left out.
    #[serde(default)]
    pub dry_run_port: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubChangeRequestOutput {
    pub number: u32,
    pub url: String,
    pub title: String,
    /// `open`, `merged` or `closed`.
    pub state: String,
    pub merged: bool,
    pub head_sha: String,
    /// With `ci`: its CI on the head (`passed`, `running`, `failed`, …), when it has any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ci: Option<CiState>,
    /// With `ci`, when `failed`: what failed, `ci` (a check on the head) or `queue` (the merge queue's run, which took
    /// it out of the queue and turned auto-merge off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub failure: Option<String>,
    /// With `ci`, when `failed`: the failed run to look at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ci_url: Option<String>,
    /// With `ci`, when `failed`: the failed jobs (`workflow: job`), as a Markdown list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub failed_jobs: Option<String>,
}

fn number_of(v: &Option<Value>) -> Option<Result<u32, String>> {
    let text = match v {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().trim_start_matches('#').to_string(),
        _ => return None,
    };
    Some(text.parse().map_err(|_| format!("{text:?} is not a pull request number")))
}

fn state_name(s: PrState) -> &'static str {
    match s {
        PrState::Open => "open",
        PrState::Merged => "merged",
        PrState::Closed => "closed",
    }
}

fn change_output(pr: ChangeRequest) -> GithubChangeRequestOutput {
    GithubChangeRequestOutput {
        number: pr.number,
        url: pr.url,
        title: pr.title,
        state: state_name(pr.state).into(),
        merged: pr.state == PrState::Merged,
        head_sha: pr.head_sha,
        ci: None,
        failure: None,
        ci_url: None,
        failed_jobs: None,
    }
}

/// A pull request read with its CI, and the port it leaves by: `failed` when it is open and a check on its head
/// failed, or the merge queue removed it for failed checks (auto-merge off since).
fn ci_output(pr: PullRequestStatus) -> (&'static str, GithubChangeRequestOutput) {
    let open = pr.state == PrState::Open;
    let head_failed = pr.ci.as_ref().filter(|c| c.state == CiState::Failed);
    let queue_dropped =
        pr.queue_removal.as_ref().is_some_and(|r| r.reason == QueueRemovalReason::FailedChecks) && !pr.auto_merge;
    let failed = match (open, head_failed) {
        (false, _) => None,
        (true, Some(c)) => Some(("ci", c.url.clone(), c.failed_jobs_markdown())),
        (true, None) if queue_dropped => {
            let q = pr.queue_ci.as_ref().filter(|c| c.state == CiState::Failed);
            Some((
                "queue",
                q.map(|c| c.url.clone()).unwrap_or_default(),
                q.map(CiStatus::failed_jobs_markdown).unwrap_or_default(),
            ))
        }
        (true, None) => None,
    };
    let port = if failed.is_some() { "failed" } else { state_name(pr.state) };
    let (failure, ci_url, failed_jobs) = match failed {
        Some((f, url, jobs)) => (Some(f.to_string()), Some(url), Some(jobs)),
        None => (None, None, None),
    };
    let out = GithubChangeRequestOutput {
        number: pr.number,
        url: pr.url,
        title: pr.title,
        state: state_name(pr.state).into(),
        merged: pr.state == PrState::Merged,
        head_sha: pr.head_sha,
        ci: pr.ci.map(|c| c.state),
        failure,
        ci_url,
        failed_jobs,
    };
    (port, out)
}

pub struct GithubChangeRequest;

impl NodeKind for GithubChangeRequest {
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubChangeRequestConfig, GithubChangeRequestOutput>(
            "github.change_request",
            1,
            Category::Action,
            "Read a GitHub pull request",
            "Reads a pull request by its head branch or its number and leaves by its state: `open`, `merged`, \
             `closed`, or `none` when there is none. Output: `number`, `url`, `title`, `state`, `merged`, `headSha`. \
             With `ci` (and `branch`, its head) it reads CI too: an open one whose check failed on its head, or that \
             the merge queue removed for failed checks, leaves by `failed` with `failure` (`ci` or `queue`), `ciUrl` \
             and `failedJobs`. Between waits it polls one (a dry run takes `dryRunPort`, else `open` for a branch, \
             `merged` for a number).",
            &[("open", "Open"), ("merged", "Merged"), ("closed", "Closed unmerged"), ("none", "No pull request")],
            true,
        )
    }

    /// `failed` too, with `ci`.
    fn ports(&self, config: &Value) -> Vec<String> {
        let mut out: Vec<String> = ["open", "merged", "closed", "none"].map(String::from).into();
        if parse::<GithubChangeRequestConfig>(config).is_ok_and(|c| c.ci) {
            out.push("failed".into());
        }
        out
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubChangeRequestConfig = parse(config)?;
        check_repo(&c.repo)?;
        if let Some(p) = &c.dry_run_port
            && !["open", "merged", "closed", "none", "failed"].contains(&p.as_str())
        {
            return Err(format!("dryRunPort {p:?} is not open, merged, closed, none or failed"));
        }
        if c.ci && nonblank(&c.branch).is_none() {
            return Err("ci reads CI on the pull request's head: give branch".into());
        }
        let number = match number_of(&c.number) {
            Some(Err(_)) if matches!(&c.number, Some(Value::String(s)) if templated(s)) => Some(Ok(0)),
            n => n,
        };
        match (nonblank(&c.branch), number) {
            (Some(_), None) => Ok(()),
            (None, Some(Ok(_))) => Ok(()),
            (Some(_), Some(Ok(_))) if c.ci => Ok(()),
            (_, Some(Err(e))) => Err(e),
            _ => Err("give either branch or number (both only with ci)".into()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubChangeRequestConfig = parse(&input)?;
            let RepoOf { repo, forge, ci } = repo_for_step(cx, &c.repo).await?;
            if cx.dry_run {
                // The number may come from a step a dry run didn't really run.
                let (default, would) = match nonblank(&c.branch) {
                    Some(b) => ("open", format!("read {repo}'s pull request from {b}")),
                    None => ("merged", format!("read a pull request of {repo}")),
                };
                let port = c.dry_run_port.as_deref().unwrap_or(default);
                return Ok(Outcome::would(port, would));
            }
            if c.ci {
                let branch = nonblank(&c.branch).ok_or("ci needs branch")?.to_string();
                let number = number_of(&c.number).transpose()?;
                let (r, b) = (repo.clone(), branch.clone());
                let args = crate::github::ReadArgs {
                    forge: Some(forge.clone()),
                    ci: ci.clone(),
                    repo: r,
                    branch: b,
                    since: 0,
                    was_queued: false,
                    queue_cache: None,
                    known: Default::default(),
                    branch_head: None,
                    last_pr_head: None,
                };
                let read = forge_call(&forge, move |_| {
                    let r = crate::github::read_attempt(&args)?;
                    match r.forge_error.clone() {
                        Some(e) => Err(ForgeError::Failed(e)),
                        None => Ok(r.pr_with_ci()),
                    }
                })
                .await?;
                if let Some(pr) = read.filter(|pr| number.is_none_or(|n| n == pr.number)) {
                    let (port, out) = ci_output(pr);
                    return Ok(Outcome::next(port, out));
                }
                // The branch's newest is another pull request (a later release's, or PDT-269 another fork's branch of
                // the same name): this one by number, with the CI on its own head when it is open.
                let found = match number {
                    Some(n) => {
                        let r = repo.clone();
                        forge_call(&forge, move |f| f.change_request_number(&r, n)).await?
                    }
                    None => None,
                };
                let Some(pr) = found else {
                    return Ok(Outcome::Next { port: "none".into(), output: json!({"repo": repo, "quiet": true}) });
                };
                if pr.state != PrState::Open {
                    return Ok(Outcome::next(state_name(pr.state), change_output(pr)));
                }
                let mut all = vec![];
                for p in &ci {
                    let (r, b, sha) = (repo.clone(), branch.clone(), pr.head_sha.clone());
                    all.extend(CiStatus::of(&ci_call(p, move |p| p.runs(&r, &b, Some(&sha))).await?, &pr.head_sha));
                }
                let status = CiStatus::combine(all);
                let mut out = change_output(pr);
                out.ci = status.as_ref().map(|c| c.state);
                let port = match status.filter(|c| c.state == CiState::Failed) {
                    Some(c) => {
                        (out.failure, out.ci_url, out.failed_jobs) =
                            (Some("ci".into()), Some(c.url.clone()), Some(c.failed_jobs_markdown()));
                        "failed"
                    }
                    None => "open",
                };
                return Ok(Outcome::next(port, out));
            }
            let by = match (nonblank(&c.branch), number_of(&c.number)) {
                (Some(b), None) => Err(b.to_string()),
                (None, Some(n)) => Ok(n?),
                _ => return Err("give either branch or number".into()),
            };
            let r = repo.clone();
            let pr = match by {
                Err(b) => forge_call(&forge, move |f| f.change_request(&r, &b)).await?,
                Ok(n) => forge_call(&forge, move |f| f.change_request_number(&r, n)).await?,
            };
            let Some(pr) = pr else {
                return Ok(Outcome::Next { port: "none".into(), output: json!({"repo": repo, "quiet": true}) });
            };
            let port = state_name(pr.state);
            Ok(Outcome::next(port, change_output(pr)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let pr = |v: Value| GithubChangeRequest.check(&v);
        assert!(pr(json!({"branch": "release"})).is_ok());
        assert!(pr(json!({"number": "{{steps.pr.number}}"})).is_ok());
        assert!(pr(json!({"number": "#12"})).is_ok());
        assert!(pr(json!({"number": "x"})).is_err());
        assert!(pr(json!({})).is_err());
        assert!(pr(json!({"branch": "b", "number": 3})).is_err());
        assert!(pr(json!({"branch": "b", "number": "{{steps.pr.number}}", "ci": true})).is_ok());
        assert!(pr(json!({"number": 3, "ci": true})).unwrap_err().contains("branch"));
        assert!(pr(json!({"branch": "b", "dryRunPort": "failed"})).is_ok());
        assert!(!GithubChangeRequest.ports(&json!({"branch": "b"})).contains(&"failed".to_string()));
        assert!(GithubChangeRequest.ports(&json!({"branch": "b", "ci": true})).contains(&"failed".to_string()));
    }
}
