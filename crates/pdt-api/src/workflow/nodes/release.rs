//! Release actions (PDT-194), through the GitHub forge (the run's project's, else GitHub's account, PDT-241): `github.workflow.run` (start a CI workflow by hand, e.g. the
//! repository's Release Action), `github.change_request` (read a pull request by branch or number) and
//! `github.release` (read the latest release, or one tag). The built-in "Cut a release" polls the last two between
//! waits, the pull request with its CI (PDT-214: the release PR merges itself, so a failed check is the news).
//! `github.workflow.latest` reads a workflow's newest run on a branch: PDT nightly release reads the scheduled
//! nightly's (PDT-283), which no PDT run starts. `github.nightly` watches for a day's nightly and puts a missing one
//! in the Inbox until it is published (PDT-288).
//! `repo` defaults to the run's project's GitHub repository.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::forge::ForgeKind;
use pdt_core::github_issue::valid_repo;
use pdt_core::pr::{CiState, CiStatus, PrState, PullRequestStatus, QueueRemovalReason};
use pdt_core::release::parse_version;
use pdt_forge::{ChangeRequest, CiProvider, Forge, ForgeError, Release};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use ts_rs::TS;

/// Runs a call on `api` (from [`repo_of`]) off the async threads; `releases` when it needs
/// [`pdt_forge::Capabilities::releases`] (an error when the forge lacks them).
async fn gh<T: Send + 'static>(
    api: &Arc<dyn Forge>,
    releases: bool,
    f: impl FnOnce(&dyn Forge) -> Result<T, ForgeError> + Send + 'static,
) -> Result<T, String> {
    let api = api.clone();
    if releases && !api.capabilities().releases {
        return Err(format!("{} has no releases", api.name()));
    }
    tokio::task::spawn_blocking(move || f(api.as_ref()))
        .await
        .map_err(|e| format!("the GitHub task failed: {e}"))?
        .map_err(|e| format!("GitHub: {e}"))
}

fn templated(s: &str) -> bool {
    s.contains("{{")
}

fn nonblank(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// A given `repo` must be owner/name (left alone while it is a template).
pub(crate) fn check_repo(repo: &Option<String>) -> Result<(), String> {
    match nonblank(repo) {
        Some(r) if !templated(r) && valid_repo(r).is_none() => Err(format!("{r:?} is not owner/name")),
        _ => Ok(()),
    }
}

/// A repository, the forge it is read through and the CI it has ([`repo_of`]).
pub(crate) struct RepoOf {
    pub repo: String,
    pub forge: Arc<dyn Forge>,
    /// The project's CI providers (PDT-242), else GitHub's own CI.
    pub ci: Vec<Arc<dyn CiProvider>>,
}

impl RepoOf {
    /// Its GitHub Actions, or why it has none (`what` it was wanted for).
    pub fn actions(&self, what: &str) -> Result<Arc<dyn CiProvider>, String> {
        self.ci
            .iter()
            .find(|p| p.id() == pdt_core::pr::GITHUB_ACTIONS)
            .cloned()
            .ok_or_else(|| format!("{} has no GitHub Actions to {what}", self.repo))
    }
}

/// The repository and the forge it is read through (PDT-241): the one given, through the run's project's forge when
/// that is GitHub, else GitHub's account; without one, the run's project's on GitHub.
async fn repo_of(cx: &StepCx<'_>, repo: &Option<String>) -> Result<RepoOf, String> {
    repo_for(cx.st, cx.run.project_id.as_deref(), repo).await
}

/// [`repo_of`] for `project` (a workflow's, when no run asks).
pub(crate) async fn repo_for(
    st: &crate::routes::AppState,
    project: Option<&str>,
    repo: &Option<String>,
) -> Result<RepoOf, String> {
    if let Some(r) = nonblank(repo) {
        valid_repo(r).ok_or_else(|| format!("{r:?} is not owner/name"))?;
        let (forge, ci) = crate::forge::forge_and_ci_for(st, &ForgeKind::GITHUB, project).await?;
        return Ok(RepoOf { repo: r.to_string(), forge, ci });
    }
    let id = project.ok_or("give repo (owner/name): the run has no project to take it from")?;
    let p = st.store.get_project(id).await.map_err(|e| e.to_string())?.ok_or("the run's project is gone")?;
    match crate::forge::project_forge(st, id, &p.repo_path).await {
        Some(f) if f.kind == ForgeKind::GITHUB => {
            let ci = crate::ci::project_ci(st, id, &p.repo_path, Some(&f)).await;
            Ok(RepoOf { repo: f.repo, forge: f.api, ci })
        }
        _ => Err(format!("{} is not on GitHub: give repo (owner/name)", p.name)),
    }
}

// ---- github.workflow.run ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubWorkflowRunConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The workflow's file name (`release.yml`) or id. It must take `workflow_dispatch`.
    pub workflow: String,
    /// The branch or tag it runs on (default `main`).
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    /// Its `workflow_dispatch` inputs; empty and null values are left out.
    #[serde(default)]
    #[ts(type = "Record<string, unknown> | null")]
    pub inputs: Option<Value>,
}

/// The inputs as GitHub takes them: text, without the empty ones.
fn inputs_of(v: &Option<Value>) -> Result<BTreeMap<String, String>, String> {
    let Some(v) = v else { return Ok(BTreeMap::new()) };
    let Some(m) = v.as_object() else {
        return match v {
            Value::Null => Ok(BTreeMap::new()),
            _ => Err("inputs is not an object".into()),
        };
    };
    Ok(m.iter()
        .filter_map(|(k, v)| {
            let s = match v {
                Value::Null => return None,
                Value::String(s) => s.trim().to_string(),
                other => other.to_string(),
            };
            (!s.is_empty()).then(|| (k.clone(), s))
        })
        .collect())
}

pub struct GithubWorkflowRun;

impl NodeKind for GithubWorkflowRun {
    fn spec(&self) -> KindSpec {
        spec::<GithubWorkflowRunConfig, Value>(
            "github.workflow.run",
            1,
            Category::Action,
            "Run a GitHub Actions workflow",
            "Starts a GitHub Actions workflow by hand (`workflow_dispatch`) on a branch (default `main`) with inputs, \
             as PDT's GitHub login. The repository is the run's project's unless `repo` names one. GitHub doesn't \
             say which run it started: read what it makes (a pull request, a release) with the other GitHub nodes.",
            &[("out", "Then")],
            false,
        )
    }

    /// Started twice, the workflow runs twice.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubWorkflowRunConfig = parse(config)?;
        if c.workflow.trim().is_empty() {
            return Err("workflow is empty: name its file (release.yml)".into());
        }
        check_repo(&c.repo)?;
        if c.inputs.as_ref().is_some_and(|v| !v.is_object() && !v.is_null() && !v.as_str().is_some_and(templated)) {
            return Err("inputs is not an object".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubWorkflowRunConfig = parse(&input)?;
            let RepoOf { repo, ci, .. } = repo_of(cx, &c.repo).await?;
            let workflow = c.workflow.trim().to_string();
            let git_ref = nonblank(&c.git_ref).unwrap_or("main").to_string();
            let inputs = inputs_of(&c.inputs)?;
            let out = json!({"repo": repo, "workflow": workflow, "ref": git_ref, "inputs": inputs});
            if cx.dry_run {
                return Ok(Outcome::next(
                    "out",
                    json!({"would": format!("run {workflow} on {repo} at {git_ref} with {}", json!(inputs))}),
                ));
            }
            let actions = ci.into_iter().find(|p| p.id() == pdt_core::pr::GITHUB_ACTIONS);
            let actions = actions.ok_or_else(|| format!("{repo} has no GitHub Actions to run {workflow} with"))?;
            let (r, w, g) = (repo.clone(), workflow.clone(), git_ref.clone());
            tokio::task::spawn_blocking(move || actions.trigger(&r, &g, &w, &inputs))
                .await
                .map_err(|e| format!("the GitHub task failed: {e}"))?
                .map_err(|e| format!("GitHub: {e}"))?;
            Ok(Outcome::next("out", out))
        })
    }
}

// ---- github.change_request ----

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

/// The failed jobs of a CI status, one Markdown list item each (the workflow alone when it names none).
fn failed_list(ci: &CiStatus) -> String {
    let mut lines = vec![];
    for r in ci.failed() {
        if r.failed_jobs.is_empty() {
            lines.push(format!("- {}", r.name));
        }
        lines.extend(r.failed_jobs.iter().map(|j| format!("- {}: {j}", r.name)));
    }
    lines.join("\n")
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
        (true, Some(c)) => Some(("ci", c.url.clone(), failed_list(c))),
        (true, None) if queue_dropped => {
            let q = pr.queue_ci.as_ref().filter(|c| c.state == CiState::Failed);
            Some(("queue", q.map(|c| c.url.clone()).unwrap_or_default(), q.map(failed_list).unwrap_or_default()))
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
            let RepoOf { repo, forge, ci } = repo_of(cx, &c.repo).await?;
            if cx.dry_run {
                // The number may come from a step a dry run didn't really run.
                let (default, would) = match nonblank(&c.branch) {
                    Some(b) => ("open", format!("read {repo}'s pull request from {b}")),
                    None => ("merged", format!("read a pull request of {repo}")),
                };
                let port = c.dry_run_port.as_deref().unwrap_or(default);
                return Ok(Outcome::next(port, json!({"would": would})));
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
                let read = gh(&forge, false, move |_| {
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
                        gh(&forge, false, move |f| f.change_request_number(&r, n)).await?
                    }
                    None => None,
                };
                let Some(pr) = found else {
                    return Ok(Outcome::Next { port: "none".into(), output: json!({"repo": repo, "quiet": true}) });
                };
                if pr.state != PrState::Open {
                    return Ok(Outcome::next(state_name(pr.state), change_output(pr)));
                }
                let (r, b, sha) = (repo.clone(), branch.clone(), pr.head_sha.clone());
                let status = tokio::task::spawn_blocking(move || {
                    let mut all = vec![];
                    for p in &ci {
                        all.extend(CiStatus::of(&p.runs(&r, &b, Some(&sha)).map_err(|e| e.to_string())?, &sha));
                    }
                    Ok::<_, String>(CiStatus::combine(all))
                })
                .await
                .map_err(|e| format!("the CI task failed: {e}"))??;
                let mut out = change_output(pr);
                out.ci = status.as_ref().map(|c| c.state);
                let port = match status.filter(|c| c.state == CiState::Failed) {
                    Some(c) => {
                        (out.failure, out.ci_url, out.failed_jobs) =
                            (Some("ci".into()), Some(c.url.clone()), Some(failed_list(&c)));
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
                Err(b) => gh(&forge, false, move |f| f.change_request(&r, &b)).await?,
                Ok(n) => gh(&forge, false, move |f| f.change_request_number(&r, n)).await?,
            };
            let Some(pr) = pr else {
                return Ok(Outcome::Next { port: "none".into(), output: json!({"repo": repo, "quiet": true}) });
            };
            let port = state_name(pr.state);
            Ok(Outcome::next(port, change_output(pr)))
        })
    }
}

// ---- github.release ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubReleaseConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The release tagged this (`v0.2.0`); the latest release when left out.
    #[serde(default)]
    pub tag: Option<String>,
    /// With the latest: count it only when its tag is not this one (nor an older version), e.g. the latest before a
    /// release was started (`{{steps.before.tag}}`). Empty: any release counts.
    #[serde(default)]
    pub newer_than: Option<String>,
    /// Instead of the latest release, the newest published one, pre-releases included (a nightly release, PDT-210,
    /// PDT-283).
    #[serde(default)]
    pub prereleases: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubReleaseOutput {
    pub tag: String,
    pub name: String,
    pub url: String,
    pub published_at: Option<String>,
}

/// Whether release `tag` counts as newer than `than` (any other tag when either isn't a version).
fn newer(tag: &str, than: Option<&str>) -> bool {
    let Some(than) = than else { return true };
    if tag == than {
        return false;
    }
    match (parse_version(tag), parse_version(than)) {
        (Some(a), Some(b)) => a.is_newer_than(&b),
        _ => true,
    }
}

pub struct GithubRelease;

impl NodeKind for GithubRelease {
    fn spec(&self) -> KindSpec {
        spec::<GithubReleaseConfig, GithubReleaseOutput>(
            "github.release",
            1,
            Category::Action,
            "Read a GitHub release",
            "Reads the latest published release (drafts and pre-releases don't count; with `prereleases`, the newest \
             one, pre-releases included), or the one tagged `tag`. `found` with `tag`, `name`, `url` and \
             `publishedAt`; `none` when there is none, or with `newerThan` when the latest is still that one. \
             Between waits it polls for a release to be published.",
            &[("found", "Found"), ("none", "None")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubReleaseConfig = parse(config)?;
        check_repo(&c.repo)?;
        if nonblank(&c.tag).is_some() && nonblank(&c.newer_than).is_some() {
            return Err("newerThan reads the latest release: leave tag out".into());
        }
        if nonblank(&c.tag).is_some() && c.prereleases {
            return Err("prereleases reads the newest release: leave tag out".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubReleaseConfig = parse(&input)?;
            let RepoOf { repo, forge, .. } = repo_of(cx, &c.repo).await?;
            let tag = nonblank(&c.tag).map(String::from);
            let than = nonblank(&c.newer_than).map(String::from);
            if cx.dry_run {
                let latest = if c.prereleases { "the newest release or pre-release" } else { "the latest release" };
                let what = tag.as_ref().map_or(latest.to_string(), |t| format!("release {t}"));
                return Ok(Outcome::next("found", json!({"would": format!("read {what} of {repo}")})));
            }
            let (r, t, pre) = (repo.clone(), tag.clone(), c.prereleases);
            let release: Option<Release> = gh(&forge, true, move |f| match (t, pre) {
                (None, true) => f.newest_release(&r),
                (t, _) => f.release(&r, t.as_deref()),
            })
            .await?;
            match release {
                Some(rel) if newer(&rel.tag, than.as_deref()) => Ok(Outcome::next(
                    "found",
                    GithubReleaseOutput { tag: rel.tag, name: rel.name, url: rel.url, published_at: rel.published_at },
                )),
                other => Ok(Outcome::Next {
                    port: "none".into(),
                    output: json!({"repo": repo, "latest": other.map(|r| r.tag), "quiet": true}),
                }),
            }
        })
    }
}

// ---- github.workflow.latest ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubWorkflowLatestConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The workflow's name as Actions shows it (its `name:`, e.g. `release`).
    pub workflow: String,
    /// The branch its runs are on (default `main`).
    #[serde(default)]
    pub branch: Option<String>,
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
    fn spec(&self) -> KindSpec {
        spec::<GithubWorkflowLatestConfig, GithubWorkflowLatestOutput>(
            "github.workflow.latest",
            1,
            Category::Action,
            "Read a workflow's latest run",
            "Reads the newest GitHub Actions run of a workflow (by its name, `release`) on a branch (default `main`), \
             however it was started (a schedule, by hand), and leaves by its state: `passed`, `failed` (with \
             `failedJobs`), `running` (queued or running), `cancelled`, or `none` when there is no run.",
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
        check_repo(&c.repo)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubWorkflowLatestConfig = parse(&input)?;
            let RepoOf { repo, ci, .. } = repo_of(cx, &c.repo).await?;
            let workflow = c.workflow.trim().to_string();
            let branch = nonblank(&c.branch).unwrap_or("main").to_string();
            if cx.dry_run {
                return Ok(Outcome::next(
                    "passed",
                    json!({"would": format!("read the latest {workflow} run on {branch} of {repo}")}),
                ));
            }
            let actions = ci.into_iter().find(|p| p.id() == pdt_core::pr::GITHUB_ACTIONS);
            let actions = actions.ok_or_else(|| format!("{repo} has no GitHub Actions to read {workflow} from"))?;
            let (r, w, b) = (repo.clone(), workflow.clone(), branch.clone());
            let latest = tokio::task::spawn_blocking(move || {
                let runs = actions.runs(&r, &b, None)?;
                let Some(mut run) = runs
                    .into_iter()
                    .filter(|x| x.name == w && x.branch == b)
                    .max_by(|x, y| x.created_at.cmp(&y.created_at))
                else {
                    return Ok(None);
                };
                // Which jobs failed is only for the message: a failed lookup leaves them out.
                if run.state == CiState::Failed && run.failed_jobs.is_empty() {
                    run.failed_jobs = pdt_forge::ci::failed_job_names(actions.as_ref(), &r, &run).unwrap_or_default();
                }
                Ok::<_, ForgeError>(Some(run))
            })
            .await
            .map_err(|e| format!("the GitHub task failed: {e}"))?
            .map_err(|e| format!("GitHub: {e}"))?;
            let Some(run) = latest else {
                return Ok(Outcome::Next { port: "none".into(), output: json!({"repo": repo, "quiet": true}) });
            };
            let port = match run.state {
                CiState::Passed => "passed",
                CiState::Failed => "failed",
                CiState::Cancelled => "cancelled",
                CiState::Queued | CiState::Running => "running",
            };
            let failed_jobs = (run.state == CiState::Failed).then(|| {
                let jobs: Vec<String> = run.failed_jobs.iter().map(|j| format!("- {}: {j}", run.name)).collect();
                if jobs.is_empty() { format!("- {}", run.name) } else { jobs.join("\n") }
            });
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

// ---- github.nightly ----

/// How often [`GithubNightly`] looks again while a nightly is missing.
const NIGHTLY_LOOK_MS: u64 = 30 * 60_000;
/// How long it watches a missing nightly at most: until shortly before the next day's check takes over.
const NIGHTLY_WATCH_MS: i64 = 23 * 60 * 60_000 + 30 * 60_000;
/// Its choices (ports): start the nightly, or leave it.
const START: &str = "start";
const IGNORE: &str = "ignore";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubNightlyConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The release workflow's file (`release.yml`): its runs are linked, and Start it runs it.
    pub workflow: String,
    /// The branch the nightly is built from (default `main`).
    #[serde(default)]
    pub branch: Option<String>,
    /// The day whose nightly to look for (`2026-10-30`, e.g. `{{trigger.date}}`); today (UTC) when left out.
    #[serde(default)]
    pub day: Option<String>,
    /// The hour (UTC) the day's nightly should be out by (default 10).
    #[serde(default)]
    #[ts(optional)]
    pub due_hour: Option<u32>,
    /// The Inbox item's title (default "Tonight's nightly didn't run").
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubNightlyOutput {
    /// The day looked at (`20261030`).
    pub day: u64,
    /// With `published`: the day's nightly.
    pub tag: Option<String>,
    pub url: Option<String>,
    /// The release workflow's runs on GitHub.
    pub runs_url: String,
}

/// What the watch read: the day's nightly, if published, and whether main has commits its newest nightly and stable
/// release lack (asked only when it matters).
struct NightlyRead {
    published: Option<Release>,
    changed: bool,
    /// What main was compared with, for the item's text.
    base: Option<String>,
    ahead: u64,
}

async fn read_nightly(
    forge: &Arc<dyn Forge>,
    repo: &str,
    branch: &str,
    day: u64,
    due: bool,
) -> Result<NightlyRead, String> {
    let (r, b) = (repo.to_string(), branch.to_string());
    gh(forge, true, move |f| {
        let releases = f.recent_releases(&r)?;
        let tags: Vec<&str> = releases.iter().map(|x| x.tag.as_str()).collect();
        let published =
            pdt_core::release::nightly_for_day(&tags, day).and_then(|t| releases.iter().find(|x| x.tag == t));
        let mut read = NightlyRead { published: published.cloned(), changed: false, base: None, ahead: 0 };
        if read.published.is_some() || !due {
            return Ok(read);
        }
        let (nightly, stable) = pdt_core::release::nightly_bases(&tags);
        // Nothing released yet: anything on main is new.
        read.changed = true;
        for base in [nightly, stable].into_iter().flatten() {
            match f.commits_ahead(&r, base, &b)? {
                Some(0) => {
                    read.changed = false;
                    break;
                }
                Some(n) if read.base.is_none() => (read.base, read.ahead) = (Some(base.to_string()), n),
                _ => {}
            }
        }
        Ok(read)
    })
    .await
}

pub struct GithubNightly;

impl NodeKind for GithubNightly {
    fn spec(&self) -> KindSpec {
        spec::<GithubNightlyConfig, GithubNightlyOutput>(
            "github.nightly",
            1,
            Category::Action,
            "Watch for the day's nightly",
            "Looks for a day's nightly release (a published `X.Y.Z-nightly.YYYYMMDD` pre-release, PDT-288). \
             `published` when there is one; before `dueHour` UTC `early`; `unchanged` when main has nothing its \
             newest nightly or stable release lacks. Otherwise the nightly is missing: one Inbox item says so (once \
             per workflow and day, else `already`), with Start it (`start`) and Ignore (`ignore`), and it looks \
             again every half hour: the item closes by itself once the nightly is published (`published`), or \
             after about a day (`expired`).",
            &[
                ("published", "Published"),
                ("early", "Not due yet"),
                ("unchanged", "main unchanged"),
                (START, "Start it"),
                (IGNORE, "Ignore"),
                ("already", "Already in the Inbox"),
                ("expired", "Never came"),
            ],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubNightlyConfig = parse(config)?;
        if c.workflow.trim().is_empty() {
            return Err("workflow is empty: name its file (release.yml)".into());
        }
        if let Some(d) = nonblank(&c.day).filter(|d| !templated(d))
            && pdt_core::release::nightly_day(d).is_none()
        {
            return Err(format!("day {d:?} is not a date (2026-10-30)"));
        }
        if c.due_hour.is_some_and(|h| h > 23) {
            return Err("dueHour is an hour, 0 to 23".into());
        }
        check_repo(&c.repo)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            use pdt_core::inbox::{InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
            use pdt_core::release::{NightlyWatch, nightly_day, nightly_watch, utc_day};
            let c: GithubNightlyConfig = parse(&input)?;
            let RepoOf { repo, forge, .. } = repo_of(cx, &c.repo).await?;
            let workflow = c.workflow.trim().to_string();
            let branch = nonblank(&c.branch).unwrap_or("main").to_string();
            let now = crate::routes::now_ms();
            // A look after the first keeps the first's day and start (its saved output).
            let saved = |k: &str| cx.step.output.get(k).and_then(Value::as_i64);
            let day = match saved("day") {
                Some(d) => d as u64,
                None => match nonblank(&c.day) {
                    Some(d) => nightly_day(d).ok_or_else(|| format!("day {d:?} is not a date"))?,
                    None => utc_day(now),
                },
            };
            let since = saved("since").unwrap_or(now);
            let due_hour = c.due_hour.unwrap_or(10);
            let runs_url = format!("https://github.com/{repo}/actions/workflows/{workflow}");
            let out = |tag: Option<&Release>| GithubNightlyOutput {
                day,
                tag: tag.map(|r| r.tag.clone()),
                url: tag.map(|r| r.url.clone()),
                runs_url: runs_url.clone(),
            };
            if cx.dry_run {
                return Ok(Outcome::next(
                    "unchanged",
                    json!({"would": format!("look for {repo}'s nightly of {day}, due at {due_hour}:00 UTC")}),
                ));
            }
            let title =
                c.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "Tonight's nightly didn't run".into());
            // It waits for the user's choice, and (`retry`) runs again at the next look: the item stays open while
            // the step does, and closes when it leaves by any port.
            let look_again = || {
                let next = now + i64::try_from(NIGHTLY_LOOK_MS).unwrap_or(i64::MAX / 2);
                let mut progress = serde_json::to_value(out(None)).unwrap_or_default();
                progress["since"] = json!(since);
                Outcome::Wait {
                    wait: json!({"retry": true, "answer": {"question": title, "choices": [START, IGNORE]}}),
                    until: Some(next.min(since + NIGHTLY_WATCH_MS)),
                    timeout_port: None,
                    output: progress,
                }
            };
            let due = now >= pdt_core::release::day_hour_ms(day, due_hour);
            let read = match read_nightly(&forge, &repo, &branch, day, due).await {
                Ok(r) => r,
                // While it watches (its item open), a failed look waits for the next one: each look is one of the
                // step's attempts, so the node's retries would soon be used up.
                Err(e) if saved("since").is_some() => {
                    tracing::warn!(error = %e, "could not look for the day's nightly; looking again later");
                    return Ok(look_again());
                }
                Err(e) => return Err(e),
            };
            let quiet = |port: &str| {
                let mut o = serde_json::to_value(out(None)).unwrap_or_default();
                o["quiet"] = json!(true);
                Outcome::Next { port: port.into(), output: o }
            };
            match nightly_watch(now, day, due_hour, read.published.is_some(), read.changed) {
                NightlyWatch::Published => return Ok(Outcome::next("published", out(read.published.as_ref()))),
                NightlyWatch::Early => return Ok(quiet("early")),
                NightlyWatch::Unchanged => return Ok(quiet("unchanged")),
                NightlyWatch::Missing => {}
            }
            if now - since >= NIGHTLY_WATCH_MS {
                return Ok(quiet("expired"));
            }
            let store = &cx.st.store;
            let open = store.open_items_for_step(&cx.step.id).await.map_err(|e| e.to_string())?;
            if open.is_empty() {
                let date = format!("{}-{:02}-{:02}", day / 10_000, day / 100 % 100, day % 100);
                let changed = match (&read.base, read.ahead) {
                    (Some(b), n) if n > 0 => {
                        format!("{branch} has {n} commit{} since {b}", if n == 1 { "" } else { "s" })
                    }
                    _ => format!("{branch} has changed since the last release"),
                };
                let text = format!(
                    "No nightly for {date} was published by {due_hour:02}:00 UTC, though {changed}. GitHub's \
                     scheduled runs are best-effort and sometimes never start.\n\nSee [the release workflow's \
                     runs]({runs_url}). **Start it** runs `{workflow}` on {branch} with channel `nightly` as your \
                     GitHub login; if your login can't start workflows, start it there (Run workflow, channel \
                     `nightly`). This item goes away by itself once a nightly for the day is published."
                );
                let item = InboxItem {
                    id: uuid::Uuid::now_v7().to_string(),
                    kind: InboxKind::Gate,
                    group: InboxGroup::Gate,
                    status: InboxStatus::Open,
                    project_id: cx.run.project_id.clone(),
                    ticket_id: None,
                    ticket_key: None,
                    ticket_title: None,
                    attempt_id: None,
                    session_id: None,
                    run_id: Some(cx.run.id.clone()),
                    step_id: Some(cx.step.id.clone()),
                    persona_id: None,
                    persona_name: None,
                    title: title.clone(),
                    body: json!({"question": title, "text": text, "url": runs_url, "workflow": cx.workflow.name,
                                 "node": cx.node_title()}),
                    choices: vec![InboxChoice::new(START, "Start it", true), InboxChoice::new(IGNORE, "Ignore", false)],
                    resolution: Value::Null,
                    created_at: now,
                    resolved_at: None,
                };
                // Once per workflow and day: a second run the same day (a catch-up) leaves it to the first.
                let key = format!("nightly-missing:{}:{day}", cx.workflow.id);
                match store.add_inbox_item_once(&item, &key).await.map_err(|e| e.to_string())? {
                    Some(_) => crate::notifications::kick(cx.st),
                    None => return Ok(quiet("already")),
                }
            }
            Ok(look_again())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let run = |v: Value| GithubWorkflowRun.check(&v);
        assert!(run(json!({"workflow": "release.yml", "inputs": {"bump": "{{trigger.bump}}"}})).is_ok());
        assert!(run(json!({"workflow": " "})).is_err());
        assert!(run(json!({"workflow": "r.yml", "repo": "nope"})).unwrap_err().contains("owner/name"));
        assert!(run(json!({"workflow": "r.yml", "inputs": [1]})).is_err());

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

        let latest = |v: Value| GithubWorkflowLatest.check(&v);
        assert!(latest(json!({"workflow": "release"})).is_ok());
        assert!(latest(json!({"workflow": "release", "branch": "main", "repo": "a/b"})).is_ok());
        assert!(latest(json!({"workflow": ""})).is_err());
        assert!(latest(json!({"workflow": "release", "repo": "nope"})).is_err());

        let nightly = |v: Value| GithubNightly.check(&v);
        assert!(nightly(json!({"workflow": "release.yml", "day": "{{trigger.date}}", "dueHour": 10})).is_ok());
        assert!(nightly(json!({"workflow": "release.yml", "day": "2026-10-30"})).is_ok());
        assert!(nightly(json!({"workflow": "release.yml", "day": "Friday"})).unwrap_err().contains("not a date"));
        assert!(nightly(json!({"workflow": "release.yml", "dueHour": 24})).is_err());
        assert!(nightly(json!({"workflow": " "})).is_err());

        let rel = |v: Value| GithubRelease.check(&v);
        assert!(rel(json!({})).is_ok());
        assert!(rel(json!({"newerThan": "{{steps.before.tag}}"})).is_ok());
        assert!(rel(json!({"tag": "v1.0.0", "newerThan": "v0.9.0"})).is_err());
        assert!(rel(json!({"prereleases": true, "newerThan": "{{steps.before.tag}}"})).is_ok());
        assert!(rel(json!({"tag": "v1.0.0", "prereleases": true})).is_err());
    }

    #[test]
    fn inputs_are_text_without_empty_ones() {
        let i = inputs_of(&Some(json!({"bump": "minor", "version": "", "dry": true, "n": null}))).unwrap();
        assert_eq!(i, BTreeMap::from([("bump".into(), "minor".into()), ("dry".into(), "true".into())]));
        assert!(inputs_of(&None).unwrap().is_empty());
        assert!(inputs_of(&Some(json!("x"))).is_err());
    }

    #[test]
    fn a_release_is_newer_when_its_version_is() {
        assert!(newer("v0.2.0", None));
        assert!(newer("v0.2.0", Some("v0.1.840")));
        assert!(!newer("v0.2.0", Some("v0.2.0")));
        assert!(!newer("v0.1.9", Some("v0.2.0")));
        assert!(newer("nightly", Some("v0.2.0")));
        assert!(newer("v0.2.1-exp.1", Some("v0.2.0")));
        assert!(newer("v0.2.1-exp.2", Some("v0.2.1-exp.1")));
        assert!(!newer("v0.2.0", Some("v0.2.1-exp.1")));
    }
}
