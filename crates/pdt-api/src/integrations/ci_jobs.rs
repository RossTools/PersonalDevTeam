//! The CI job tools (PDT-629): an agent lists the jobs of its own ticket attempt's head (`ci_jobs`) and retries chosen
//! ones (`ci_retry`) through the project's CI providers, a Buildkite step or a GitHub Actions job on its own, instead
//! of re-running whole builds with a CI's CLI. Only the head's newest runs are retried (an older commit's re-run
//! cancels the head's, PDT-510), each job at most [`ci_retry::RETRY_CAP`] times per head, and each retry is a
//! `ci_retry` event on the ticket's timeline. No confirmation: they only touch the attempt's own CI.

use super::action::{Action, ActionCx, ToolFace};
use super::forge::{Here, RunId, here, providers_of, tool};
use crate::workflow::catalog::BoxFut;
use pdt_core::ci_retry;
use pdt_core::integration::ToolDecl;
use pdt_core::pr::{CiJob, CiRun, CiState, CiStatus, PrState};
use pdt_core::ticket::EventKind;
use pdt_forge::{CiProvider, ForgeError};
use pdt_store::AttemptInfo;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// How many lines of a failed job's log `ci_jobs` gives (`ci_failed_logs` gives more).
const TAIL_LINES: usize = 30;

/// The session's own attempt, its open pull request's number and its head: the pull request's head, else the pushed
/// branch's (CI that builds branches).
async fn own_head(cx: &ActionCx<'_>, h: &Here) -> Result<(AttemptInfo, Option<u32>, String), String> {
    let info = h.attempt.clone().ok_or("only a ticket attempt's own session reads and retries its CI jobs")?;
    crate::github::refresh_attempt(cx.st, &info).await;
    if let Some(pr) = crate::github::watched_pr(cx.st, &info.attempt.id).filter(|p| p.state == PrState::Open) {
        return Ok((info, Some(pr.number), pr.head_sha));
    }
    let head = crate::github::branch_head(&info).await;
    let head = head.ok_or(format!("{} has no commit whose CI to read", info.attempt.branch))?;
    Ok((info, None, head))
}

/// The provider a named run is of: `provider` as given, else the only one, else GitHub Actions.
fn provider_of_run(all: Vec<Arc<dyn CiProvider>>, given: bool) -> Result<Vec<Arc<dyn CiProvider>>, String> {
    if given || all.len() == 1 {
        return Ok(all);
    }
    let gh = all.into_iter().find(|p| p.id() == pdt_core::pr::GITHUB_ACTIONS);
    Ok(vec![gh.ok_or("name the run's provider (`ci_jobs` gives it)")?])
}

/// `p`'s newest run of each workflow (pipeline) on `head`, or only run `id` of them. A run of another commit, or an
/// older run of the head, is refused.
fn head_runs(p: &dyn CiProvider, repo: &str, branch: &str, head: &str, id: Option<&str>) -> Result<Vec<CiRun>, String> {
    let label = |e: ForgeError| format!("{}: {e}", p.label());
    let newest = CiStatus::of(&p.runs(repo, branch, Some(head)).map_err(label)?, head).map(|s| s.runs);
    let newest = newest.unwrap_or_default();
    let Some(id) = id else { return Ok(newest) };
    if let Some(r) = newest.iter().find(|r| r.id == id) {
        return Ok(vec![r.clone()]);
    }
    let all = p.runs(repo, branch, None).map_err(label)?;
    let r = all.iter().find(|r| r.id == id).ok_or(format!("{} has no run {id} on {branch}", p.label()))?;
    ci_retry::check_head(id, &r.head_sha, head)?;
    Err(format!("run {id} is not the newest {} run on the head: `ci_jobs` lists the newest", r.name))
}

/// A run on the head and its jobs.
struct HeadRun {
    provider: Arc<dyn CiProvider>,
    run: CiRun,
    jobs: Vec<CiJob>,
}

/// The runs on `head` of `providers` (only run `id` with one), with their jobs; providers that couldn't be read go in
/// the errors (all but a named run's).
fn read_head(
    providers: &[Arc<dyn CiProvider>],
    repo: &str,
    branch: &str,
    head: &str,
    id: Option<&str>,
) -> Result<(Vec<HeadRun>, Vec<String>), String> {
    let (mut out, mut errors) = (vec![], vec![]);
    for p in providers {
        let runs = match head_runs(p.as_ref(), repo, branch, head, id) {
            Ok(r) => r,
            Err(e) if id.is_some() => return Err(e),
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        for run in runs {
            let jobs = p.jobs(repo, &run).map_err(|e| format!("{}: {e}", p.label()))?;
            out.push(HeadRun { provider: p.clone(), run, jobs });
        }
    }
    Ok((out, errors))
}

/// The data of the ticket's `ci_retry` events, which the cap counts.
async fn retry_events(cx: &ActionCx<'_>, ticket_id: &str) -> Result<Vec<Value>, String> {
    let events = cx.st.store.events_of(ticket_id, &[EventKind::CiRetry], 1000).await.map_err(crate::workflow::err)?;
    Ok(events.into_iter().filter_map(|e| e.data).collect())
}

// ---- ci_jobs

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobsInput {
    /// Only this CI provider's runs (`github_actions`, `buildkite`); every provider of the project's when left out.
    #[serde(default)]
    pub provider: Option<String>,
    /// Only this run of the head (`ci_runs` names it).
    #[serde(default)]
    pub run_id: Option<RunId>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobOut {
    /// What `ci_retry` takes (as do its key and name).
    pub id: String,
    pub name: String,
    /// Its step's key (Buildkite).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub state: CiState,
    /// `ci_retry` can run it again.
    pub retryable: bool,
    pub url: String,
    /// Times it was retried on this head (`ci_retry`), of `retryCap`.
    pub retries: usize,
    /// The end of a failed job's log (`ci_failed_logs` gives more).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_tail: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeadRunOut {
    pub provider: String,
    pub run_id: String,
    /// Its workflow (or pipeline).
    pub name: String,
    pub state: CiState,
    pub url: String,
    pub jobs: Vec<JobOut>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobsOutput {
    pub branch: String,
    /// The commit whose CI this is: the pull request's head, else the pushed branch's.
    pub head: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<u32>,
    /// The newest run of each workflow on the head.
    pub runs: Vec<HeadRunOut>,
    /// How many times `ci_retry` retries one job on one head.
    pub retry_cap: usize,
    /// Providers whose runs couldn't be read, and why.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

pub struct CiJobs;

impl Action for CiJobs {
    type Input = JobsInput;
    type Output = JobsOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("ci_jobs", "List the CI jobs of the session's pull request head").read_only().open_world(),
            "CI jobs",
            "The jobs of the newest CI runs on your ticket attempt's head commit (its pull request's head, else the \
             pushed branch's), from each of the project's CI providers (or one, or one run): each job's id, name, \
             step key, state, whether `ci_retry` can run it again and how often it was, its URL, and the end of a \
             failed job's log.",
        )
    }

    fn approval_title(&self, _: &JobsInput) -> String {
        "read the head's CI jobs".into()
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: JobsInput) -> BoxFut<'a, Result<JobsOutput, String>> {
        Box::pin(async move {
            let h = here(cx).await?;
            let (info, pr, head) = own_head(cx, &h).await?;
            let mut providers = providers_of(&h, i.provider.as_deref())?;
            let id = i.run_id.map(|r| r.text());
            if id.is_some() {
                providers = provider_of_run(providers, i.provider.is_some())?;
            }
            let (repo, branch, at) = (h.repo(), info.attempt.branch.clone(), head.clone());
            let (runs, errors) = tokio::task::spawn_blocking(move || {
                let (runs, errors) = read_head(&providers, &repo, &branch, &at, id.as_deref())?;
                let with_logs = runs
                    .into_iter()
                    .map(|r| {
                        let tails = r
                            .jobs
                            .iter()
                            .map(|j| {
                                let failed = j.state == CiState::Failed && r.provider.capabilities().logs;
                                failed.then(|| r.provider.job_log_tail(&repo, &r.run, j, TAIL_LINES).ok()).flatten()
                            })
                            .collect::<Vec<_>>();
                        (r, tails)
                    })
                    .collect::<Vec<_>>();
                Ok::<_, String>((with_logs, errors))
            })
            .await
            .map_err(crate::workflow::err)??;
            if runs.is_empty() && !errors.is_empty() {
                return Err(errors.join("; "));
            }
            let events = retry_events(cx, &info.ticket_id).await?;
            let runs = runs
                .into_iter()
                .map(|(r, tails)| {
                    let provider = r.provider.id().to_string();
                    let jobs = r
                        .jobs
                        .into_iter()
                        .zip(tails)
                        .map(|(j, log_tail)| JobOut {
                            retries: ci_retry::retries(&events, &info.attempt.id, &head, &provider, &j.name),
                            id: j.id,
                            name: j.name,
                            key: j.key,
                            state: j.state,
                            retryable: j.retryable && r.provider.capabilities().retry,
                            url: j.url,
                            log_tail,
                        })
                        .collect();
                    HeadRunOut {
                        provider,
                        run_id: r.run.id,
                        name: r.run.name,
                        state: r.run.state,
                        url: r.run.url,
                        jobs,
                    }
                })
                .collect();
            let branch = info.attempt.branch;
            Ok(JobsOutput { branch, head, pr, runs, retry_cap: ci_retry::RETRY_CAP, errors })
        })
    }
}

// ---- ci_retry

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryInput {
    /// The jobs to run again: each its id, step key or name (`ci_jobs`).
    #[serde(default)]
    pub jobs: Vec<String>,
    /// Every failed job of the head's runs (with `jobs`, those too).
    #[serde(default)]
    pub failed: bool,
    /// Only this CI provider's jobs; every provider of the project's when left out.
    #[serde(default)]
    pub provider: Option<String>,
    /// Only this run's jobs (the head's newest of its workflow).
    #[serde(default)]
    pub run_id: Option<RunId>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetriedOut {
    pub provider: String,
    pub run_id: String,
    /// The jobs asked for, by name.
    pub jobs: Vec<String>,
    /// The provider can't retry one job on its own: every failed job of the run runs again.
    pub all_failed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetryOutput {
    pub head: String,
    pub retried: Vec<RetriedOut>,
    pub message: String,
}

pub struct CiRetry;

impl Action for CiRetry {
    type Input = RetryInput;
    type Output = RetryOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("ci_retry", "Retry CI jobs (a flaky step) on the session's pull request head").open_world(),
            "Retry CI jobs",
            "Runs chosen jobs of your ticket attempt's head commit again (by id, step key or name from `ci_jobs`; \
             `failed: true` for every failed one): a Buildkite step or a GitHub Actions job on its own, not the whole \
             build. Only for a flake outside your change: fix a real failure and push instead. Only the head's newest \
             runs are retried, never an older commit's (that cancels the head's run), and each job at most twice per \
             head; then fix the cause or ask the user. Each retry shows on the ticket's timeline. It asks no \
             confirmation: it only re-runs your attempt's own CI.",
        )
    }

    fn check(&self, i: &RetryInput) -> Result<(), String> {
        if i.jobs.iter().all(|j| j.trim().is_empty()) && !i.failed {
            return Err("name the jobs to retry (`jobs`), or give `failed: true` for every failed one".into());
        }
        Ok(())
    }

    fn approval_title(&self, i: &RetryInput) -> String {
        if i.jobs.is_empty() {
            "retry the head's failed CI jobs".into()
        } else {
            format!("retry CI jobs {}", i.jobs.join(", "))
        }
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: RetryInput) -> BoxFut<'a, Result<RetryOutput, String>> {
        Box::pin(async move {
            self.check(&i)?;
            let h = here(cx).await?;
            let (info, _, head) = own_head(cx, &h).await?;
            let mut providers = providers_of(&h, i.provider.as_deref())?;
            let id = i.run_id.map(|r| r.text());
            if id.is_some() {
                providers = provider_of_run(providers, i.provider.is_some())?;
            }
            let labels = providers.iter().map(|p| p.label().to_string()).collect::<Vec<_>>().join(", ");
            providers.retain(|p| p.capabilities().retry);
            if providers.is_empty() {
                return Err(format!("{labels} can't retry jobs: push to run CI again"));
            }
            let (repo, branch, at) = (h.repo(), info.attempt.branch.clone(), head.clone());
            let (runs, _) =
                tokio::task::spawn_blocking(move || read_head(&providers, &repo, &branch, &at, id.as_deref()))
                    .await
                    .map_err(crate::workflow::err)??;
            let all: Vec<(usize, CiJob)> =
                runs.iter().enumerate().flat_map(|(r, run)| run.jobs.iter().map(move |j| (r, j.clone()))).collect();
            let jobs: Vec<CiJob> = all.iter().map(|(_, j)| j.clone()).collect();
            let ci_retry::Picked { jobs: picked, skipped } = ci_retry::pick(&jobs, &i.jobs, i.failed)?;
            let events = retry_events(cx, &info.ticket_id).await?;
            for &k in &picked {
                let (r, j) = &all[k];
                let used = ci_retry::retries(&events, &info.attempt.id, &head, runs[*r].provider.id(), &j.name);
                ci_retry::check_cap(used, &j.name, &head)?;
            }
            let by = crate::mcp::tools::agent_by(cx.st, cx.session.ok_or("only an agent session retries CI")?).await;
            let mut retried: Vec<RetriedOut> = vec![];
            for (r, run) in runs.iter().enumerate() {
                let chosen: Vec<&CiJob> = picked.iter().filter(|&&k| all[k].0 == r).map(|&k| &all[k].1).collect();
                if chosen.is_empty() {
                    continue;
                }
                let ids: Vec<String> = chosen.iter().map(|j| j.id.clone()).collect();
                let (p, ci_run, repo) = (run.provider.clone(), run.run.clone(), h.repo());
                let got = tokio::task::spawn_blocking(move || p.retry_jobs(&repo, &ci_run, &ids))
                    .await
                    .map_err(crate::workflow::err)?;
                // Failing partway, the jobs that ran again all the same are recorded (and count toward the cap).
                let (done, failed): (Vec<&CiJob>, _) = match got {
                    Ok(()) => (chosen, None),
                    Err(e) => (chosen.into_iter().filter(|j| e.retried.contains(&j.id)).collect(), Some(e.error)),
                };
                let p = &run.provider;
                if !done.is_empty() {
                    let names: Vec<&str> = done.iter().map(|j| j.name.as_str()).collect();
                    let data = ci_retry::event_data(&info.attempt.id, &head, p.id(), p.label(), &run.run.id, &names);
                    let now = crate::routes::now_ms();
                    if let Err(e) = cx
                        .st
                        .store
                        .add_ticket_event(&info.ticket_id, EventKind::CiRetry, by.clone(), None, Some(data), now)
                        .await
                    {
                        tracing::warn!(ticket = %info.ticket_key, error = %e, "could not record a CI retry");
                    }
                    retried.push(RetriedOut {
                        provider: p.id().to_string(),
                        run_id: run.run.id.clone(),
                        jobs: names.iter().map(|n| n.to_string()).collect(),
                        all_failed: !p.capabilities().retry_jobs,
                    });
                }
                if let Some(e) = failed {
                    let names = retried.iter().flat_map(|r| r.jobs.iter().map(String::as_str)).collect::<Vec<_>>();
                    let ran = if names.is_empty() {
                        String::new()
                    } else {
                        format!(" ({} ran again all the same)", names.join(", "))
                    };
                    return Err(format!("{}: {e}{ran}", p.label()));
                }
            }
            let names = retried.iter().flat_map(|r| r.jobs.iter().map(String::as_str)).collect::<Vec<_>>();
            let mut message = format!(
                "Retried {} on {}. You are told how it ends: no need to watch it",
                names.join(", "),
                ci_retry::short_sha(&head)
            );
            if !skipped.is_empty() {
                message.push_str(&format!(". Skipped {}: CI can't retry them on their own", skipped.join(", ")));
            }
            if retried.iter().any(|r| r.all_failed) {
                message.push_str(" (a provider without single-job retries runs every failed job of its run again)");
            }
            Ok(RetryOutput { head, retried, message })
        })
    }
}
