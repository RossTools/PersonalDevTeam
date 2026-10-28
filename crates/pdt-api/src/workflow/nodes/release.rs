//! Release actions (PDT-194), through the GitHub forge: `github.workflow.run` (start a CI workflow by hand, e.g. the
//! repository's Release Action), `github.change_request` (read a pull request by branch or number) and
//! `github.release` (read the latest release, or one tag). The built-in "Cut a release" polls the last two between
//! waits. `repo` defaults to the run's project's GitHub repository.

use crate::routes::AppState;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::github_issue::valid_repo;
use pdt_core::pr::PrState;
use pdt_core::release::parse_version;
use pdt_forge::{ChangeRequest, Forge, ForgeError, Release};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use ts_rs::TS;

/// Runs a call on the GitHub forge off the async threads; `releases` when it needs [`pdt_forge::Capabilities::releases`].
async fn gh<T: Send + 'static>(
    st: &AppState,
    releases: bool,
    f: impl FnOnce(&dyn Forge) -> Result<T, ForgeError> + Send + 'static,
) -> Result<T, String> {
    let api: Arc<dyn Forge> = st.github.clone();
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
fn check_repo(repo: &Option<String>) -> Result<(), String> {
    match nonblank(repo) {
        Some(r) if !templated(r) && valid_repo(r).is_none() => Err(format!("{r:?} is not owner/name")),
        _ => Ok(()),
    }
}

/// The repository: the one given, else the run's project's on GitHub.
async fn repo_of(cx: &StepCx<'_>, repo: &Option<String>) -> Result<String, String> {
    if let Some(r) = nonblank(repo) {
        return valid_repo(r).map(|_| r.to_string()).ok_or_else(|| format!("{r:?} is not owner/name"));
    }
    let id = cx.run.project_id.as_deref().ok_or("give repo (owner/name): the run has no project to take it from")?;
    let p = cx.st.store.get_project(id).await.map_err(|e| e.to_string())?.ok_or("the run's project is gone")?;
    match crate::forge::project_forge(cx.st, id, &p.repo_path).await {
        Some(f) if f.kind == pdt_core::forge::ForgeKind::Github => Ok(f.repo),
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
            let repo = repo_of(cx, &c.repo).await?;
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
            let (r, w, g) = (repo.clone(), workflow.clone(), git_ref.clone());
            gh(cx.st, true, move |f| f.run_workflow(&r, &w, &g, &inputs)).await?;
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
    /// Or its number.
    #[serde(default)]
    #[ts(type = "number | string | null")]
    pub number: Option<Value>,
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
    }
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
             Between waits it polls one (a dry run takes `dryRunPort`, else `open` for a branch, `merged` for a \
             number).",
            &[("open", "Open"), ("merged", "Merged"), ("closed", "Closed unmerged"), ("none", "No pull request")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubChangeRequestConfig = parse(config)?;
        check_repo(&c.repo)?;
        if let Some(p) = &c.dry_run_port
            && !["open", "merged", "closed", "none"].contains(&p.as_str())
        {
            return Err(format!("dryRunPort {p:?} is not open, merged, closed or none"));
        }
        match (nonblank(&c.branch), number_of(&c.number)) {
            (Some(_), None) => Ok(()),
            (None, Some(Ok(_))) => Ok(()),
            (None, Some(Err(e))) => match &c.number {
                Some(Value::String(s)) if templated(s) => Ok(()),
                _ => Err(e),
            },
            _ => Err("give either branch or number".into()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubChangeRequestConfig = parse(&input)?;
            let repo = repo_of(cx, &c.repo).await?;
            if cx.dry_run {
                // The number may come from a step a dry run didn't really run.
                let (default, would) = match nonblank(&c.branch) {
                    Some(b) => ("open", format!("read {repo}'s pull request from {b}")),
                    None => ("merged", format!("read a pull request of {repo}")),
                };
                let port = c.dry_run_port.as_deref().unwrap_or(default);
                return Ok(Outcome::next(port, json!({"would": would})));
            }
            let by = match (nonblank(&c.branch), number_of(&c.number)) {
                (Some(b), None) => Err(b.to_string()),
                (None, Some(n)) => Ok(n?),
                _ => return Err("give either branch or number".into()),
            };
            let r = repo.clone();
            let pr = match by {
                Err(b) => gh(cx.st, false, move |f| f.change_request(&r, &b)).await?,
                Ok(n) => gh(cx.st, false, move |f| f.change_request_number(&r, n)).await?,
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
            "Reads the latest published release (drafts and pre-releases don't count), or the one tagged `tag`. \
             `found` with `tag`, `name`, `url` and `publishedAt`; `none` when there is none, or with `newerThan` \
             when the latest is still that one. Between waits it polls for a release to be published.",
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
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubReleaseConfig = parse(&input)?;
            let repo = repo_of(cx, &c.repo).await?;
            let tag = nonblank(&c.tag).map(String::from);
            let than = nonblank(&c.newer_than).map(String::from);
            if cx.dry_run {
                let what = tag.as_ref().map_or("the latest release".to_string(), |t| format!("release {t}"));
                return Ok(Outcome::next("found", json!({"would": format!("read {what} of {repo}")})));
            }
            let (r, t) = (repo.clone(), tag.clone());
            let release: Option<Release> = gh(cx.st, true, move |f| f.release(&r, t.as_deref())).await?;
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

        let rel = |v: Value| GithubRelease.check(&v);
        assert!(rel(json!({})).is_ok());
        assert!(rel(json!({"newerThan": "{{steps.before.tag}}"})).is_ok());
        assert!(rel(json!({"tag": "v1.0.0", "newerThan": "v0.9.0"})).is_err());
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
    }
}
