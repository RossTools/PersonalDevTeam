//! A pull request's failures against its base branch's (PDT-641), through the project's CI providers (forge-neutral):
//! `github.pr.base_ci` tells a pull request that fails only what its base fails (`inherited`, with the open ticket
//! that looks like the fix) from one with failures of its own (`own`); `ci.branch_checks` reads how a branch stands
//! on named checks now (`fixed` once they all pass there). The rules are [`pdt_core::base_ci`]'s.

use super::forge_call::{ci_call, forge_call};
use super::repo_of::{RepoOf, check_repo, repo_for_step, templated};
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, spec};
use pdt_core::base_ci::{self, BaseNow};
use pdt_core::pr::{CiRun, CiState, CiStatus, PrState};
use pdt_core::ticket::{TicketFilter, TicketStage};
use pdt_forge::{CiProvider, ForgeError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use ts_rs::TS;

/// The failed jobs of `runs`' failed runs, filled in where the provider didn't name them.
fn fill_jobs(p: &dyn CiProvider, repo: &str, runs: &mut [CiRun]) {
    for r in runs.iter_mut().filter(|r| r.state == CiState::Failed && r.failed_jobs.is_empty()) {
        if let Ok(j) = pdt_forge::ci::failed_job_names(p, repo, r) {
            r.failed_jobs = j;
        }
    }
}

/// A branch's latest finished run of each workflow, from every provider, failed jobs filled in.
async fn branch_runs(ci: &[Arc<dyn CiProvider>], repo: &str, branch: &str) -> Result<Vec<CiRun>, String> {
    let mut all = vec![];
    for p in ci {
        let (r, b) = (repo.to_string(), branch.to_string());
        let runs = ci_call(p, move |p| {
            let runs = p.runs(&r, &b, None)?;
            let mut latest: Vec<CiRun> = base_ci::latest_finished(&runs).into_iter().cloned().collect();
            fill_jobs(p, &r, &mut latest);
            Ok(latest)
        })
        .await?;
        all.extend(runs);
    }
    Ok(all)
}

/// A commit's newest run of each workflow, from every provider, failed jobs filled in.
async fn commit_runs(ci: &[Arc<dyn CiProvider>], repo: &str, branch: &str, sha: &str) -> Result<Vec<CiRun>, String> {
    let mut all = vec![];
    for p in ci {
        let (r, b, s) = (repo.to_string(), branch.to_string(), sha.to_string());
        let runs = ci_call(p, move |p| {
            let mut runs = CiStatus::of(&p.runs(&r, &b, Some(&s))?, &s).map(|c| c.runs).unwrap_or_default();
            fill_jobs(p, &r, &mut runs);
            Ok(runs)
        })
        .await?;
        all.extend(runs);
    }
    Ok(all)
}

/// The advisories the base's failed jobs of `checks` name in their logs (a few jobs, where the provider has logs).
async fn base_advisories(ci: &[Arc<dyn CiProvider>], repo: &str, base: &[CiRun], checks: &[String]) -> Vec<String> {
    let mut out = vec![];
    for p in ci.iter().filter(|p| p.capabilities().logs) {
        let runs: Vec<CiRun> = base
            .iter()
            .filter(|r| r.provider == p.id() && r.state == CiState::Failed)
            .filter(|r| checks.iter().any(|c| c == &r.name || c.starts_with(&format!("{}: ", r.name))))
            .cloned()
            .collect();
        let (r, checks) = (repo.to_string(), checks.to_vec());
        let found = ci_call(p, move |p| {
            let mut found = vec![];
            for run in &runs {
                let jobs = p.jobs(&r, run)?;
                let failed = jobs.iter().filter(|j| j.state == CiState::Failed);
                for job in failed.filter(|j| checks.contains(&format!("{}: {}", run.name, j.name))).take(3) {
                    if let Ok(log) = p.job_log_tail(&r, run, job, pdt_forge::ci::LOG_LINES) {
                        found.extend(base_ci::advisories(&log));
                    }
                }
            }
            Ok::<_, ForgeError>(found)
        })
        .await
        .unwrap_or_default();
        out.extend(found);
    }
    out.dedup();
    out
}

/// The open ticket of `project` that looks like the fix: the first (by rank) that mentions an advisory, else a check's
/// job.
async fn fix_ticket(cx: &StepCx<'_>, project: &str, terms: &[String]) -> Option<(String, String)> {
    for term in terms {
        let f = TicketFilter {
            project: Some(project.to_string()),
            q: Some(term.clone()),
            stages: vec![TicketStage::Ready, TicketStage::Todo, TicketStage::InProgress],
            limit: 5,
            ..Default::default()
        };
        let Ok(page) = cx.st.store.query_tickets(&f).await else { continue };
        if let Some(t) = page.items.into_iter().next() {
            return Some((t.key, t.title));
        }
    }
    None
}

/// Checks as Markdown, a line each.
fn listed(checks: &[String]) -> String {
    checks.iter().map(|c| format!("- {c}")).collect::<Vec<_>>().join("\n")
}

// ---- github.pr.base_ci ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct BaseCiConfig {
    /// The repository (`owner/name`, `{{trigger.repo}}`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The pull request's number (`{{trigger.number}}`).
    #[ts(type = "number | string")]
    pub number: Value,
    /// Its head branch, where its CI runs (`{{trigger.headBranch}}`).
    pub branch: String,
    /// Its base branch; the pull request's when left out.
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BaseCiOutput {
    /// The base branch.
    pub base: String,
    /// The head it read.
    pub head_sha: String,
    /// The failed checks that fail on the base too (`workflow: job`).
    pub checks: Vec<String>,
    /// The failed checks that don't: the pull request's own.
    pub own: Vec<String>,
    /// `checks` (or `own`) as a Markdown list.
    pub listed: String,
    /// The open ticket that looks like the fix (its key), when one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fix_title: Option<String>,
    /// What it waits on, for the run's row: `Waiting on main · PDT-639`.
    pub note: String,
}

pub struct BaseCi;

impl NodeKind for BaseCi {
    fn needs_project(&self, config: &Value) -> bool {
        crate::workflow::catalog::repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<BaseCiConfig, BaseCiOutput>(
            "github.pr.base_ci",
            1,
            Category::Action,
            "Failures from the base branch?",
            "Reads the pull request's failed checks (`workflow: job`) on its head and the base branch's latest \
             finished run of each, through the project's CI providers. Every failure fails on the base too: \
             `inherited`, with `checks`, the open ticket that looks like the fix (`fix`: one that mentions an \
             advisory in the base's failed logs, or a failed job) and `note` (`Waiting on main · PDT-639`). Any that \
             passes there, or never ran there: `own`, with `own`. Nothing failing on its head: `none`. Closed, merged \
             or gone: `gone`.",
            &[
                ("inherited", "Only the base's failures"),
                ("own", "Its own failures"),
                ("none", "Nothing failing"),
                ("gone", "Not open"),
            ],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: BaseCiConfig = parse(config)?;
        check_repo(&c.repo)?;
        if c.branch.trim().is_empty() {
            return Err("branch is empty: name its head branch ({{trigger.headBranch}})".into());
        }
        match &c.number {
            Value::String(s) if templated(s) => Ok(()),
            n => number(n).map(|_| ()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: BaseCiConfig = parse(&input)?;
            let n = number(&c.number)?;
            if cx.dry_run {
                return Ok(Outcome::would("own", format!("compare #{n}'s failed checks with its base branch's")));
            }
            let RepoOf { repo, forge, ci } = repo_for_step(cx, &c.repo).await?;
            let r = repo.clone();
            let pr = forge_call(&forge, move |f| f.change_request_number(&r, n)).await?;
            let Some(pr) = pr.filter(|p| p.state == PrState::Open) else {
                return Ok(Outcome::next("gone", json!({"quiet": true})));
            };
            let base = nonblank(&c.base).map(String::from).unwrap_or(pr.base_branch.clone());
            let head = commit_runs(&ci, &repo, c.branch.trim(), &pr.head_sha).await?;
            let base_runs = branch_runs(&ci, &repo, &base).await?;
            let split = base_ci::split(&head, &base_runs);
            let mut out = BaseCiOutput {
                base: base.clone(),
                head_sha: pr.head_sha.clone(),
                listed: String::new(),
                checks: split.inherited.clone(),
                own: split.own.clone(),
                fix: None,
                fix_title: None,
                note: format!("Waiting on {base}"),
            };
            if split.inherited.is_empty() && split.own.is_empty() {
                return Ok(Outcome::next("none", out));
            }
            if !split.all_inherited() {
                out.listed = listed(&split.own);
                return Ok(Outcome::next("own", out));
            }
            out.listed = listed(&split.inherited);
            if let Some(project) = cx.run.project_id.as_deref() {
                let advisories = base_advisories(&ci, &repo, &base_runs, &split.inherited).await;
                let terms = base_ci::fix_terms(&split.inherited, &advisories);
                if let Some((key, title)) = fix_ticket(cx, project, &terms).await {
                    out.note = format!("Waiting on {base} · {key}");
                    out.fix = Some(key);
                    out.fix_title = Some(title);
                }
            }
            Ok(Outcome::next("inherited", out))
        })
    }

    fn rerun_safe(&self, _input: &Value) -> bool {
        true
    }
}

fn number(v: &Value) -> Result<u32, String> {
    let text = match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.trim().trim_start_matches('#').to_string(),
        _ => String::new(),
    };
    text.parse().map_err(|_| format!("{text:?} is not a pull request number"))
}

// ---- ci.branch_checks ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct BranchChecksConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The branch (`{{steps.base.base}}`).
    pub branch: String,
    /// The checks, `workflow: job` or `workflow` (`{{steps.base.checks}}`): a list, or one a line.
    #[ts(type = "Array<string> | string")]
    pub checks: Value,
}

impl BranchChecksConfig {
    fn checks(&self) -> Vec<String> {
        let items: Vec<String> = match &self.checks {
            Value::Array(a) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
            Value::String(s) => s.lines().map(String::from).collect(),
            _ => vec![],
        };
        items
            .into_iter()
            .map(|c| c.trim().trim_start_matches("- ").trim().to_string())
            .filter(|c| !c.is_empty())
            .collect()
    }
}

pub struct BranchChecks;

impl NodeKind for BranchChecks {
    fn needs_project(&self, config: &Value) -> bool {
        crate::workflow::catalog::repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<BranchChecksConfig, Value>(
            "ci.branch_checks",
            1,
            Category::Action,
            "Checks on a branch",
            "Reads how a branch stands on `checks` (`workflow: job`), from the latest finished run of each workflow \
             there, through the project's CI providers: `fixed` when every one passes, `failing` while one fails, \
             `pending` while one has no finished run yet. Output: `sha`, the commit those runs ran on.",
            &[("fixed", "All pass"), ("failing", "One fails"), ("pending", "Not known yet")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: BranchChecksConfig = parse(config)?;
        check_repo(&c.repo)?;
        if c.branch.trim().is_empty() {
            return Err("branch is empty".into());
        }
        match &c.checks {
            Value::String(s) if templated(s) => Ok(()),
            _ if c.checks().is_empty() => Err("checks is empty: name them (workflow: job)".into()),
            _ => Ok(()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: BranchChecksConfig = parse(&input)?;
            let checks = c.checks();
            let branch = c.branch.trim().to_string();
            if cx.dry_run {
                return Ok(Outcome::would("fixed", format!("read {branch}'s CI on {}", checks.join(", "))));
            }
            let RepoOf { repo, ci, .. } = repo_for_step(cx, &c.repo).await?;
            let runs = branch_runs(&ci, &repo, &branch).await?;
            let port = match base_ci::base_now(&checks, &runs) {
                BaseNow::Fixed => "fixed",
                BaseNow::Failing => "failing",
                BaseNow::Pending => "pending",
            };
            let sha = base_ci::base_sha(&checks, &runs).unwrap_or_default();
            Ok(Outcome::next(port, json!({"sha": sha, "branch": branch, "checks": checks, "quiet": true})))
        })
    }

    fn rerun_safe(&self, _input: &Value) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let base = json!({"number": "{{trigger.number}}", "branch": "{{trigger.headBranch}}"});
        assert!(BaseCi.check(&base).is_ok());
        assert!(BaseCi.check(&json!({"number": 4, "branch": " "})).unwrap_err().contains("branch"));
        assert!(BaseCi.check(&json!({"number": "x", "branch": "b"})).is_err());
        assert!(BranchChecks.check(&json!({"branch": "main", "checks": "{{steps.base.checks}}"})).is_ok());
        assert!(BranchChecks.check(&json!({"branch": "main", "checks": []})).unwrap_err().contains("checks"));
        let c: BranchChecksConfig =
            serde_json::from_value(json!({"branch": "main", "checks": "- CI: audit\n\nDocs"})).unwrap();
        assert_eq!(c.checks(), ["CI: audit", "Docs"]);
    }
}
