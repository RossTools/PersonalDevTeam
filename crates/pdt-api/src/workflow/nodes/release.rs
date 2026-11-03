//! Release actions (PDT-194), through the GitHub forge (the run's project's, else GitHub's account, PDT-241): `github.workflow.run` (start a CI workflow by hand, e.g. the
//! repository's Release Action), `github.change_request` (read a pull request by branch or number) and
//! `github.release` (read the latest release, or one tag). The built-in "Cut a release" polls the last two between
//! waits, the pull request with its CI (PDT-214: the release PR merges itself, so a failed check is the news).
//! `github.workflow.latest` reads a workflow's newest run on a branch, e.g. a scheduled release's
//! (PDT-283), which no PDT run starts. `github.nightly` watches for a day's scheduled release (any tag
//! pattern, PDT-294; PDT's nightlies in version 1, PDT-288) and puts a missing one in the Inbox until it is published.
//! `repo` defaults to the run's project's GitHub repository.

use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, repo_missing, setup_error, spec,
};
use pdt_core::forge::ForgeKind;
use pdt_core::github_issue::valid_repo;
use pdt_core::pr::{CiRun, CiState, CiStatus, PrState, PullRequestStatus, QueueRemovalReason};
use pdt_core::release::parse_version;
use pdt_core::workflow_runs::WorkflowRun;
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
/// that is GitHub, else GitHub's account; without one, the run's project's on GitHub. For a workflow step, so a
/// missing project or repository is a setup error ([`setup_error`]): the run fails at once (PDT-301).
pub(crate) async fn repo_for_step(cx: &StepCx<'_>, repo: &Option<String>) -> Result<RepoOf, String> {
    resolve_repo(cx.st, cx.run.project_id.as_deref(), repo)
        .await
        .map_err(|(setup, e)| if setup { setup_error(e) } else { e })
}

pub(crate) async fn repo_for(
    st: &crate::routes::AppState,
    project: Option<&str>,
    repo: &Option<String>,
) -> Result<RepoOf, String> {
    resolve_repo(st, project, repo).await.map_err(|(_, e)| e)
}

/// The repository, or why not: `(true, …)` when it is the setup's fault (none given and no project to take it from,
/// the project gone or not on GitHub, a repo that isn't owner/name).
async fn resolve_repo(
    st: &crate::routes::AppState,
    project: Option<&str>,
    repo: &Option<String>,
) -> Result<RepoOf, (bool, String)> {
    let passing = |e: String| (false, e);
    if let Some(r) = nonblank(repo) {
        valid_repo(r).ok_or_else(|| (true, format!("{r:?} is not owner/name")))?;
        let (forge, ci) = crate::forge::forge_and_ci_for(st, &ForgeKind::GITHUB, project).await.map_err(passing)?;
        return Ok(RepoOf { repo: r.to_string(), forge, ci });
    }
    let id = project.ok_or((
        true,
        "give repo (owner/name): the run has no project to take it from (start it in a project)".to_string(),
    ))?;
    let p = st
        .store
        .get_project(id)
        .await
        .map_err(|e| passing(e.to_string()))?
        .ok_or((true, "the run's project is gone".to_string()))?;
    match crate::forge::project_forge(st, id, &p.repo_path).await {
        Some(f) if f.kind == ForgeKind::GITHUB => {
            let ci = crate::ci::project_ci(st, id, &p.repo_path, Some(&f)).await;
            Ok(RepoOf { repo: f.repo, forge: f.api, ci })
        }
        _ => Err((true, format!("{} is not on GitHub: give repo (owner/name)", p.name))),
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
    /// The workflow's file name (`publish.yml`) or id. It must take `workflow_dispatch`.
    pub workflow: String,
    /// The branch or tag it runs on (default `main`).
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    /// Its `workflow_dispatch` inputs (an object, or text holding one as JSON); empty and null values are left out.
    #[serde(default)]
    #[ts(type = "Record<string, unknown> | null")]
    pub inputs: Option<Value>,
}

/// Inputs given as text: the JSON object it holds (a template's text param, PDT-294); blank is none.
fn inputs_text(s: &str) -> Result<Value, String> {
    match s.trim() {
        "" => Ok(Value::Null),
        t => match serde_json::from_str::<Value>(t) {
            Ok(v @ Value::Object(_)) => Ok(v),
            _ => Err("inputs is not an object (or JSON text holding one)".into()),
        },
    }
}

/// The inputs as GitHub takes them: text, without the empty ones.
fn inputs_of(v: &Option<Value>) -> Result<BTreeMap<String, String>, String> {
    let Some(v) = v else { return Ok(BTreeMap::new()) };
    let parsed;
    let v = match v {
        Value::String(s) => {
            parsed = inputs_text(s)?;
            &parsed
        }
        v => v,
    };
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
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubWorkflowRunConfig, Value>(
            "github.workflow.run",
            1,
            Category::Action,
            "Run a GitHub Actions workflow",
            "Starts a GitHub Actions workflow by hand (`workflow_dispatch`) on a branch (default `main`) with inputs, \
             as PDT's GitHub login. The repository is the run's project's unless `repo` names one. GitHub doesn't \
             say which run it started: read what it makes (a pull request, a release) with the other GitHub nodes, \
             or the run with `github.workflow.latest` and `since` set to its `dispatchedAt`.",
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
            return Err("workflow is empty: name its file (publish.yml)".into());
        }
        check_repo(&c.repo)?;
        match &c.inputs {
            None | Some(Value::Null | Value::Object(_)) => Ok(()),
            Some(Value::String(s)) if templated(s) => Ok(()),
            Some(Value::String(s)) => inputs_text(s).map(|_| ()),
            Some(_) => Err("inputs is not an object".into()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubWorkflowRunConfig = parse(&input)?;
            let RepoOf { repo, ci, .. } = repo_for_step(cx, &c.repo).await?;
            let workflow = c.workflow.trim().to_string();
            if workflow.is_empty() {
                return Err("workflow is empty: name its file (publish.yml)".into());
            }
            let git_ref = nonblank(&c.git_ref).unwrap_or("main").to_string();
            let inputs = inputs_of(&c.inputs)?;
            // When it was started: `github.workflow.latest`'s `since` finds the run it started (PDT-488).
            let at = jiff::Timestamp::now().round(jiff::Unit::Second).unwrap_or_else(|_| jiff::Timestamp::now());
            let out = json!({"repo": repo, "workflow": workflow, "ref": git_ref, "inputs": inputs, "dispatchedAt": at.to_string()});
            if cx.dry_run {
                return Ok(Outcome::would(
                    "out",
                    format!("run {workflow} on {repo} at {git_ref} with {}", json!(inputs)),
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
                        all.extend(CiStatus::of(&p.runs(&r, &b, Some(&sha)).map_err(|forge| forge.to_string())?, &sha));
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
    /// Instead of the latest release, the newest published one (pre-releases included) whose tag matches this glob
    /// (PDT-294): `*` any text, `?` one character, `{date}` a date (`v*`, `v*-beta.*`, `snapshot-{date}`). Blank: the
    /// latest release.
    #[serde(default)]
    pub tag_pattern: Option<String>,
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
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubReleaseConfig, GithubReleaseOutput>(
            "github.release",
            1,
            Category::Action,
            "Read a GitHub release",
            "Reads the latest published release (drafts and pre-releases don't count; with `prereleases`, the newest \
             one, pre-releases included; with `tagPattern`, the newest whose tag matches it, `*` any text, e.g. \
             `v*-beta.*`), or the one tagged `tag`. `found` with `tag`, `name`, `url` and \
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
        match nonblank(&c.tag_pattern) {
            Some(_) if nonblank(&c.tag).is_some() => {
                Err("tagPattern reads the newest matching release: leave tag out".into())
            }
            Some(p) if !templated(p) => pdt_core::release::TagPattern::parse(p).map(|_| ()),
            _ => Ok(()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubReleaseConfig = parse(&input)?;
            let RepoOf { repo, forge, .. } = repo_for_step(cx, &c.repo).await?;
            let tag = nonblank(&c.tag).map(String::from);
            let than = nonblank(&c.newer_than).map(String::from);
            let pattern = match nonblank(&c.tag_pattern) {
                Some(p) => Some((p.to_string(), pdt_core::release::TagPattern::parse(p)?)),
                None => None,
            };
            if cx.dry_run {
                let latest = match (&pattern, c.prereleases) {
                    (Some((p, _)), _) => format!("the newest release matching {p}"),
                    (None, true) => "the newest release or pre-release".into(),
                    (None, false) => "the latest release".into(),
                };
                let what = tag.as_ref().map_or(latest, |t| format!("release {t}"));
                return Ok(Outcome::would("found", format!("read {what} of {repo}")));
            }
            let (r, t, pre) = (repo.clone(), tag.clone(), c.prereleases);
            let release: Option<Release> = gh(&forge, true, move |f| match (t, pattern, pre) {
                (None, Some((_, p)), _) => Ok(f.recent_releases(&r)?.into_iter().find(|x| p.matches(&x.tag).is_some())),
                (None, None, true) => f.newest_release(&r),
                (t, _, _) => f.release(&r, t.as_deref()),
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
    /// The workflow's name as Actions shows it (its `name:`, e.g. `release`), or its file (`release.yml`).
    pub workflow: String,
    /// The branch its runs are on (default `main`): for a pull request's run, its head branch.
    #[serde(default)]
    pub branch: Option<String>,
    /// Only a run started by this event (`workflow_dispatch`, `pull_request`, `push`, `schedule`).
    #[serde(default)]
    pub event: Option<String>,
    /// Only a run of this commit (for a pull request's run, its head commit).
    #[serde(default)]
    pub head_sha: Option<String>,
    /// Only a run started at or after this time (RFC 3339, e.g. `github.workflow.run`'s `dispatchedAt`), less
    /// [`SINCE_SLACK`] for the clocks' difference.
    #[serde(default)]
    pub since: Option<String>,
}

/// How much earlier than `since` a run may start and still count: this host's clock and GitHub's differ a little.
const SINCE_SLACK: jiff::SignedDuration = jiff::SignedDuration::from_mins(2);

impl GithubWorkflowLatestConfig {
    /// Whether it names the workflow by its file, or filters by event, commit or time: then it reads the workflow's
    /// own runs (they say their file and event), not the branch's.
    fn by_workflow(&self) -> bool {
        let w = self.workflow.trim();
        w.ends_with(".yml")
            || w.ends_with(".yaml")
            || [&self.event, &self.head_sha, &self.since].iter().any(|f| nonblank(f).is_some())
    }

    /// `since` as a time, less [`SINCE_SLACK`].
    fn since(&self) -> Result<Option<jiff::Timestamp>, String> {
        let Some(t) = nonblank(&self.since) else { return Ok(None) };
        let t: jiff::Timestamp = t.parse().map_err(|e| format!("since is not a time (RFC 3339): {t}: {e}"))?;
        Ok(Some(t.checked_sub(SINCE_SLACK).unwrap_or(t)))
    }

    /// Whether `run` passes its event, commit and time filters.
    fn matches(&self, run: &WorkflowRun, since: Option<jiff::Timestamp>) -> bool {
        let at = |s: &str| s.parse::<jiff::Timestamp>().ok();
        nonblank(&self.event).is_none_or(|e| run.event == e)
            && nonblank(&self.head_sha).is_none_or(|sha| run.run.head_sha == sha)
            && since.is_none_or(|t| at(&run.run.created_at).is_some_and(|c| c >= t))
    }
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
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubWorkflowLatestConfig, GithubWorkflowLatestOutput>(
            "github.workflow.latest",
            1,
            Category::Action,
            "Read a workflow's latest run",
            "Reads the newest GitHub Actions run of a workflow (by its name, `release`, or file, `release.yml`) on a \
             branch (default `main`), however it was started (a schedule, by hand), and leaves by its state: \
             `passed`, `failed` (with `failedJobs`), `running` (queued or running), `cancelled`, or `none` when there \
             is no run. `event`, `headSha` and `since` pick one run: the one a pull request's merge started (its head \
             branch and commit), or the one `github.workflow.run` started (`since` its `dispatchedAt`).",
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
        if nonblank(&c.since).is_some_and(|t| !templated(t)) {
            c.since()?;
        }
        check_repo(&c.repo)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubWorkflowLatestConfig = parse(&input)?;
            let RepoOf { repo, ci, .. } = repo_for_step(cx, &c.repo).await?;
            let workflow = c.workflow.trim().to_string();
            let branch = nonblank(&c.branch).unwrap_or("main").to_string();
            if cx.dry_run {
                return Ok(Outcome::would("passed", format!("read the latest {workflow} run on {branch} of {repo}")));
            }
            let actions = ci.into_iter().find(|p| p.id() == pdt_core::pr::GITHUB_ACTIONS);
            let actions = actions.ok_or_else(|| format!("{repo} has no GitHub Actions to read {workflow} from"))?;
            let since = c.since()?;
            let (r, w, b) = (repo.clone(), workflow.clone(), branch.clone());
            let latest = tokio::task::spawn_blocking(move || {
                let newest = |x: &CiRun, y: &CiRun| x.created_at.cmp(&y.created_at);
                let run = if c.by_workflow() {
                    let runs = actions.workflow_runs(&r, &w, Some(&b), false)?;
                    let runs = runs.into_iter().filter(|x| x.run.branch == b && c.matches(x, since));
                    runs.map(|x| x.run).max_by(newest)
                } else {
                    let runs = actions.runs(&r, &b, None)?;
                    runs.into_iter().filter(|x| x.name == w && x.branch == b).max_by(newest)
                };
                let Some(mut run) = run else {
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

// ---- github.nightly: a missing scheduled release ----

/// How often [`GithubNightly`] looks again while a release is missing.
const NIGHTLY_LOOK_MS: u64 = 30 * 60_000;
/// How long it watches a missing release at most: until shortly before the next day's check takes over.
const NIGHTLY_WATCH_MS: i64 = 23 * 60 * 60_000 + 30 * 60_000;
/// Its choices (ports): start the release workflow, or leave it.
const START: &str = "start";
const IGNORE: &str = "ignore";
const MISSING_TITLE: &str = "A scheduled release is missing";

fn yes() -> bool {
    true
}

/// Version 2 (PDT-294): any tag scheme, due time and branch; version 1 knew only PDT's nightlies ([`migrate_nightly_v1`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubNightlyConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The release workflow's file (`publish.yml`): its runs are linked, and Start it runs it.
    pub workflow: String,
    /// The branch the release is built from (default `main`).
    #[serde(default)]
    pub branch: Option<String>,
    /// The scheduled release's tag: a glob where `*` is any text, `?` one character and `{date}` the day
    /// (`YYYYMMDD` or `YYYY-MM-DD`), e.g. `v*-beta.{date}*` or `snapshot-{date}`. Without `{date}`, a matching
    /// release published on the day counts.
    pub tag_pattern: String,
    /// The day whose release to look for (`2026-10-30`, e.g. `{{trigger.date}}`); today (UTC) when left out.
    #[serde(default)]
    pub day: Option<String>,
    /// When (UTC, `HH:MM`) the day's release should be out by; before it, it says nothing (default `00:00`).
    #[serde(default)]
    #[ts(optional)]
    pub due_by: Option<String>,
    /// Report it missing only if the branch has commits the newest matching release (or the latest release) lacks
    /// (default true): a scheduled workflow that publishes nothing new on a quiet day isn't missing.
    #[serde(default = "yes")]
    pub only_if_changed: bool,
    /// The Inbox item's title (default "A scheduled release is missing").
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
}

/// A version 1 config (PDT-288: PDT's own nightlies, `X.Y.Z-nightly.YYYYMMDD`, due at `dueHour`, default 10) as version
/// 2, so a workflow written for it runs as before.
fn migrate_nightly_v1(config: Value) -> Result<Value, String> {
    let mut m = match config {
        Value::Object(m) => m,
        Value::Null => Default::default(),
        _ => return Err("the config is not an object".into()),
    };
    let hour = match m.remove("dueHour") {
        None | Some(Value::Null) => 10,
        Some(v) => v.as_u64().filter(|h| *h < 24).ok_or("dueHour is an hour, 0 to 23")?,
    };
    m.insert("dueBy".into(), json!(format!("{hour:02}:00")));
    m.insert("tagPattern".into(), json!("*-nightly.{date}*"));
    m.entry("title").or_insert_with(|| json!("Tonight's nightly didn't run"));
    m.insert("onlyIfChanged".into(), json!(true));
    Ok(Value::Object(m))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubNightlyOutput {
    /// The day looked at (`20261030`).
    pub day: u64,
    /// With `published`: the day's release.
    pub tag: Option<String>,
    pub url: Option<String>,
    /// The release workflow's runs on GitHub.
    pub runs_url: String,
}

/// What the watch read: the day's release, if published, and whether the branch has commits the newest matching
/// release and the latest release lack (asked only when it matters).
struct ScheduledRead {
    published: Option<Release>,
    changed: bool,
    /// What the branch was compared with, for the item's text.
    base: Option<String>,
    ahead: u64,
}

struct Look {
    repo: String,
    branch: String,
    pattern: pdt_core::release::TagPattern,
    day: u64,
    /// The due time has passed: whether the branch changed matters.
    due: bool,
    only_if_changed: bool,
}

async fn read_scheduled(forge: &Arc<dyn Forge>, look: Look) -> Result<ScheduledRead, String> {
    use pdt_core::release::rfc3339_day;
    gh(forge, true, move |f| {
        let Look { repo: r, branch: b, pattern, day, due, only_if_changed } = look;
        let releases = f.recent_releases(&r)?;
        // Each matching release with its day: `{date}`'s, else the day it was published.
        let matching: Vec<(&Release, Option<u64>)> = releases
            .iter()
            .filter_map(|x| {
                let date = pattern.matches(&x.tag)?;
                Some((x, date.or_else(|| x.published_at.as_deref().and_then(rfc3339_day))))
            })
            .collect();
        let published = matching.iter().find(|(_, d)| d.is_some_and(|d| d >= day)).map(|(x, _)| (*x).clone());
        let mut read = ScheduledRead { published, changed: true, base: None, ahead: 0 };
        if read.published.is_some() || !due || !only_if_changed {
            return Ok(read);
        }
        let mut bases: Vec<String> = matching.first().map(|(x, _)| x.tag.clone()).into_iter().collect();
        if let Some(latest) = f.release(&r, None)?.filter(|l| !bases.contains(&l.tag)) {
            bases.push(latest.tag);
        }
        // Nothing released yet: anything on the branch is new.
        for base in &bases {
            match f.commits_ahead(&r, base, &b)? {
                Some(0) => {
                    read.changed = false;
                    break;
                }
                Some(n) if read.base.is_none() => (read.base, read.ahead) = (Some(base.clone()), n),
                _ => {}
            }
        }
        Ok(read)
    })
    .await
}

/// `20261030` as `2026-10-30`.
fn iso_day(day: u64) -> String {
    format!("{}-{:02}-{:02}", day / 10_000, day / 100 % 100, day % 100)
}

pub struct GithubNightly;

impl NodeKind for GithubNightly {
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubNightlyConfig, GithubNightlyOutput>(
            "github.nightly",
            2,
            Category::Action,
            "Watch for a scheduled release",
            "Looks for a day's scheduled release: a published release (pre-releases included) whose tag matches \
             `tagPattern` (`*` any text, `?` one character, `{date}` the day as `YYYYMMDD` or `YYYY-MM-DD`; without \
             `{date}`, one published that day), the day's or a later one. `published` when there is one; before \
             `dueBy` (UTC) `early`; with `onlyIfChanged` (the default), `unchanged` when the branch has nothing the \
             newest matching release or the latest release lacks. Otherwise it is missing: one Inbox item says so \
             (once per workflow and day, else `already`), with Start it (`start`, e.g. to a step that runs the \
             workflow) and Ignore (`ignore`), and it looks again every half hour: the item closes by itself once the \
             release is published (`published`), or after about a day (`expired`).",
            &[
                ("published", "Published"),
                ("early", "Not due yet"),
                ("unchanged", "Branch unchanged"),
                (START, "Start it"),
                (IGNORE, "Ignore"),
                ("already", "Already in the Inbox"),
                ("expired", "Never came"),
            ],
            false,
        )
    }

    fn migrate(&self, from: u32, config: Value) -> Result<Value, String> {
        match from {
            1 => migrate_nightly_v1(config),
            2 => Ok(config),
            v => Err(format!("no migration from version {v} to 2")),
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubNightlyConfig = parse(config)?;
        if c.workflow.trim().is_empty() {
            return Err("workflow is empty: name its file (publish.yml)".into());
        }
        if !templated(&c.tag_pattern) {
            pdt_core::release::TagPattern::parse(&c.tag_pattern)?;
        }
        if let Some(d) = nonblank(&c.day).filter(|d| !templated(d))
            && pdt_core::release::parse_day(d).is_none()
        {
            return Err(format!("day {d:?} is not a date (2026-10-30)"));
        }
        if let Some(t) = nonblank(&c.due_by).filter(|t| !templated(t))
            && pdt_core::release::parse_time_of_day(t).is_none()
        {
            return Err(format!("dueBy {t:?} is not a time of day (HH:MM, UTC)"));
        }
        check_repo(&c.repo)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            use pdt_core::inbox::{InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
            use pdt_core::release::{
                ScheduledWatch, TagPattern, parse_day, parse_time_of_day, scheduled_watch, utc_day,
            };
            let c: GithubNightlyConfig = parse(&input)?;
            let RepoOf { repo, forge, .. } = repo_for_step(cx, &c.repo).await?;
            let workflow = c.workflow.trim().to_string();
            if workflow.is_empty() {
                return Err("workflow is empty: name its file (publish.yml)".into());
            }
            let pattern_text = c.tag_pattern.trim().to_string();
            let pattern = TagPattern::parse(&pattern_text)?;
            let branch = nonblank(&c.branch).unwrap_or("main").to_string();
            let now = crate::routes::now_ms();
            // A look after the first keeps the first's day and start (its saved output).
            let saved = |k: &str| cx.step.output.get(k).and_then(Value::as_i64);
            let day = match saved("day") {
                Some(d) => d as u64,
                None => match nonblank(&c.day) {
                    Some(d) => parse_day(d).ok_or_else(|| format!("day {d:?} is not a date"))?,
                    None => utc_day(now),
                },
            };
            let since = saved("since").unwrap_or(now);
            let due_by = match nonblank(&c.due_by) {
                Some(t) => parse_time_of_day(t).ok_or_else(|| format!("dueBy {t:?} is not a time of day (HH:MM)"))?,
                None => 0,
            };
            let due_text = format!("{:02}:{:02}", due_by / 60, due_by % 60);
            let runs_url = format!("https://github.com/{repo}/actions/workflows/{workflow}");
            let out = |tag: Option<&Release>| GithubNightlyOutput {
                day,
                tag: tag.map(|r| r.tag.clone()),
                url: tag.map(|r| r.url.clone()),
                runs_url: runs_url.clone(),
            };
            if cx.dry_run {
                return Ok(Outcome::would(
                    "unchanged",
                    format!(
                        "look for {repo}'s release matching {pattern_text} for {day}, due at \
                                             {due_text} UTC"
                    ),
                ));
            }
            let title = c.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| MISSING_TITLE.into());
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
            let due = now >= pdt_core::release::day_time_ms(day, due_by);
            let look = Look {
                repo: repo.clone(),
                branch: branch.clone(),
                pattern,
                day,
                due,
                only_if_changed: c.only_if_changed,
            };
            let read = match read_scheduled(&forge, look).await {
                Ok(r) => r,
                // While it watches (its item open), a failed look waits for the next one: each look is one of the
                // step's attempts, so the node's retries would soon be used up.
                Err(e) if saved("since").is_some() => {
                    tracing::warn!(error = %e, "could not look for the day's scheduled release; looking again later");
                    return Ok(look_again());
                }
                Err(e) => return Err(e),
            };
            let quiet = |port: &str| {
                let mut o = serde_json::to_value(out(None)).unwrap_or_default();
                o["quiet"] = json!(true);
                Outcome::Next { port: port.into(), output: o }
            };
            match scheduled_watch(now, day, due_by, read.published.is_some(), read.changed) {
                ScheduledWatch::Published => return Ok(Outcome::next("published", out(read.published.as_ref()))),
                ScheduledWatch::Early => return Ok(quiet("early")),
                ScheduledWatch::Unchanged => return Ok(quiet("unchanged")),
                ScheduledWatch::Missing => {}
            }
            if now - since >= NIGHTLY_WATCH_MS {
                return Ok(quiet("expired"));
            }
            let store = &cx.st.store;
            let open = store.open_items_for_step(&cx.step.id).await.map_err(crate::workflow::err)?;
            if open.is_empty() {
                let changed = match (&read.base, read.ahead) {
                    _ if !c.only_if_changed => String::new(),
                    (Some(b), n) if n > 0 => {
                        format!(", though {branch} has {n} commit{} since {b}", if n == 1 { "" } else { "s" })
                    }
                    _ => format!(", though {branch} has changed since the last release"),
                };
                let text = format!(
                    "No release matching `{pattern_text}` for {} was published by {due_text} UTC{changed}. GitHub's \
                     scheduled runs are best-effort and sometimes never start.\n\nSee [the release workflow's \
                     runs]({runs_url}). **Start it** goes on with the workflow's next step (usually running \
                     `{workflow}` on {branch} as your GitHub login); if your login can't start workflows, start it \
                     there (Run workflow). This item goes away by itself once a release for the day is published.",
                    iso_day(day)
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
                    change_request: None,
                    title: title.clone(),
                    body: json!({"question": title, "text": text, "url": runs_url, "workflow": cx.workflow.name,
                                 "node": cx.node_title()}),
                    choices: vec![InboxChoice::new(START, "Start it", true), InboxChoice::new(IGNORE, "Ignore", false)],
                    resolution: Value::Null,
                    created_at: now,
                    resolved_at: None,
                };
                // Once per workflow and day: a second run the same day (a catch-up) leaves it to the first. (The key
                // keeps version 1's name, so an upgrade on the day adds no second item.)
                let key = format!("nightly-missing:{}:{day}", cx.workflow.id);
                match store.add_inbox_item_once(&item, &key).await.map_err(crate::workflow::err)? {
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
        assert!(run(json!({"workflow": "r.yml", "inputs": "{\"channel\": \"beta\"}"})).is_ok());
        assert!(run(json!({"workflow": "r.yml", "inputs": "  "})).is_ok());
        assert!(run(json!({"workflow": "r.yml", "inputs": "channel=beta"})).is_err());

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
        let since = json!({"workflow": "release.yml", "since": "{{steps.dispatch.dispatchedAt}}"});
        assert!(latest(since).is_ok());
        assert!(latest(json!({"workflow": "r.yml", "since": "2026-11-03T08:00:00Z", "event": "push"})).is_ok());
        assert!(latest(json!({"workflow": "r.yml", "since": "yesterday"})).unwrap_err().contains("RFC 3339"));

        let nightly = |v: Value| GithubNightly.check(&v);
        let w = |extra: Value| {
            let mut c = json!({"workflow": "publish.yml", "tagPattern": "v*-beta.{date}"});
            c.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            c
        };
        assert!(nightly(w(json!({"day": "{{trigger.date}}", "dueBy": "10:30"}))).is_ok());
        assert!(nightly(w(json!({"day": "2026-10-30", "onlyIfChanged": false}))).is_ok());
        assert!(nightly(w(json!({"tagPattern": "{{params.tag}}", "dueBy": "{{params.hour}}:00"}))).is_ok());
        assert!(nightly(w(json!({"day": "Friday"}))).unwrap_err().contains("not a date"));
        assert!(nightly(w(json!({"dueBy": "24:00"}))).unwrap_err().contains("time of day"));
        assert!(nightly(w(json!({"tagPattern": " "}))).unwrap_err().contains("empty"));
        assert!(nightly(w(json!({"tagPattern": "{date}{date}"}))).unwrap_err().contains("twice"));
        assert!(nightly(json!({"workflow": "publish.yml"})).is_err(), "tagPattern is needed");
        assert!(nightly(w(json!({"workflow": " "}))).is_err());
        assert!(nightly(w(json!({"dueHour": 10}))).is_err(), "version 1's field");

        let rel = |v: Value| GithubRelease.check(&v);
        assert!(rel(json!({})).is_ok());
        assert!(rel(json!({"newerThan": "{{steps.before.tag}}"})).is_ok());
        assert!(rel(json!({"tag": "v1.0.0", "newerThan": "v0.9.0"})).is_err());
        assert!(rel(json!({"prereleases": true, "newerThan": "{{steps.before.tag}}"})).is_ok());
        assert!(rel(json!({"tag": "v1.0.0", "prereleases": true})).is_err());
        assert!(rel(json!({"tagPattern": "v*-beta.*", "newerThan": "{{steps.before.tag}}"})).is_ok());
        assert!(rel(json!({"tagPattern": "{{trigger.tag}}"})).is_ok());
        assert!(rel(json!({"tagPattern": "{date}{date}"})).is_err());
        assert!(rel(json!({"tagPattern": "v*", "tag": "v1.0.0"})).is_err());
    }

    /// PDT-294: a version 1 config (PDT's nightlies) reads as version 2 and finds the same releases.
    #[test]
    fn a_version_1_nightly_config_migrates() {
        let old = json!({"repo": "a/b", "workflow": "release.yml", "branch": "main", "day": "{{trigger.date}}",
                         "dueHour": 7});
        let new = GithubNightly.migrate(1, old).unwrap();
        assert_eq!(
            new,
            json!({"repo": "a/b", "workflow": "release.yml", "branch": "main", "day": "{{trigger.date}}",
                   "dueBy": "07:00", "tagPattern": "*-nightly.{date}*", "onlyIfChanged": true,
                   "title": "Tonight's nightly didn't run"})
        );
        GithubNightly.check(&new).unwrap();
        let pattern = pdt_core::release::TagPattern::parse(new["tagPattern"].as_str().unwrap()).unwrap();
        for (tag, day) in [("v0.2.1-nightly.20261030", Some(20261030)), ("0.2.1-nightly.20261030.2", Some(20261030))] {
            assert_eq!(pattern.matches(tag), Some(day), "{tag}");
        }
        assert_eq!(pattern.matches("v0.2.0"), None);
        assert_eq!(pattern.matches("v0.2.1-exp.3"), None);

        // Its defaults: due at 10:00, PDT's title; one the user gave stays.
        let new = GithubNightly.migrate(1, json!({"workflow": "release.yml", "title": "Mine"})).unwrap();
        assert_eq!((new["dueBy"].as_str(), new["title"].as_str()), (Some("10:00"), Some("Mine")));
        assert!(GithubNightly.migrate(1, json!({"workflow": "r.yml", "dueHour": 30})).is_err());
        assert!(GithubNightly.migrate(3, json!({})).is_err());
        let current = json!({"workflow": "p.yml", "tagPattern": "v*"});
        assert_eq!(GithubNightly.migrate(2, current.clone()).unwrap(), current);
    }

    #[test]
    fn inputs_are_text_without_empty_ones() {
        let i = inputs_of(&Some(json!({"bump": "minor", "version": "", "dry": true, "n": null}))).unwrap();
        assert_eq!(i, BTreeMap::from([("bump".into(), "minor".into()), ("dry".into(), "true".into())]));
        assert!(inputs_of(&None).unwrap().is_empty());
        assert!(inputs_of(&Some(json!("x"))).is_err());
        let text = inputs_of(&Some(json!(r#"{"channel": "beta", "n": 2}"#))).unwrap();
        assert_eq!(text, BTreeMap::from([("channel".into(), "beta".into()), ("n".into(), "2".into())]));
        assert!(inputs_of(&Some(json!(""))).unwrap().is_empty());
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
