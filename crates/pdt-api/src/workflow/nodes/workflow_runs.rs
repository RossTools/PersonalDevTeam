//! A failed CI workflow run in the Inbox (PDT-292), through GitHub Actions (the run's project's, else GitHub's
//! account, PDT-241): `github.workflow.watch` keeps one Inbox item about a failed run, with Re-run failed jobs, and
//! closes it by itself once a later attempt of the run passes; `github.workflow.rerun` re-runs a run's failed jobs.
//! The template "Report failed workflow runs" puts them after the `github.workflow.failed` trigger.

use super::release::{RepoOf, check_repo, repo_for_step};
use crate::log_err::LogErr;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, repo_missing, spec};
use pdt_core::inbox::{InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
use pdt_core::pr::CiState;
use pdt_core::workflow::schedule::parse_every;
use pdt_core::workflow_runs::WorkflowRun;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use ts_rs::TS;

/// How often the watch looks at its run again.
const LOOK_MS: u64 = 10 * 60_000;
/// How long it looks when `watchFor` is left out: after that its item stays until the user answers it.
const DEFAULT_WATCH: &str = "7d";
/// Its choices (ports).
pub const RERUN: &str = "rerun";
pub const DISMISS: &str = "dismiss";

fn templated(s: &str) -> bool {
    s.contains("{{")
}

fn nonblank(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// A run id or attempt as a config gives it: a number or text. None when left out (or empty).
fn text_of(v: &Option<Value>) -> Option<String> {
    match v {
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

/// A run id is GitHub's: digits (left alone while it is a template).
fn check_run_id(v: &Option<Value>) -> Result<(), String> {
    match text_of(v) {
        None => Err("runId is empty: give the run's id ({{trigger.runId}})".into()),
        Some(t) if templated(&t) || t.chars().all(|c| c.is_ascii_digit()) => Ok(()),
        Some(t) => Err(format!("runId {t:?} is not a run's id")),
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, pdt_forge::ForgeError> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("the GitHub task failed: {e}"))?
        .map_err(|e| format!("GitHub: {e}"))
}

/// The run as it stands now, through `actions`.
async fn read_run(
    actions: &Arc<dyn pdt_forge::CiProvider>,
    repo: &str,
    id: &str,
) -> Result<Option<WorkflowRun>, String> {
    let (a, r, i) = (actions.clone(), repo.to_string(), id.to_string());
    blocking(move || a.workflow_run(&r, &i)).await
}

// ---- github.workflow.watch ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubWorkflowWatchConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The failed run's id (`{{trigger.runId}}`).
    #[ts(type = "number | string")]
    pub run_id: Option<Value>,
    /// Report only attempts after this one (`{{steps.rerun.attempt}}`, the attempt re-run before the watch came back);
    /// empty: any failed attempt.
    #[serde(default)]
    #[ts(type = "number | string | null")]
    pub after: Option<Value>,
    /// The Inbox item's title (default "<workflow> failed on <branch>").
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
    /// How long it looks for a later attempt (default `"7d"`); after that the item stays until you answer it.
    #[serde(default)]
    #[ts(optional)]
    pub watch_for: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubWorkflowWatchOutput {
    pub run_id: String,
    /// The attempt it last read.
    pub attempt: u32,
    pub workflow: String,
    pub title: String,
    pub branch: String,
    pub event: String,
    pub url: String,
    pub version: Option<String>,
    /// The failed jobs of the attempt it reported, as a Markdown list.
    pub failed_jobs: Option<String>,
}

fn output_of(run: &WorkflowRun, failed_jobs: Option<String>) -> GithubWorkflowWatchOutput {
    GithubWorkflowWatchOutput {
        run_id: run.run.id.clone(),
        attempt: run.run.attempt,
        workflow: run.run.name.clone(),
        title: run.title.clone(),
        branch: run.run.branch.clone(),
        event: run.event.clone(),
        url: run.run.url.clone(),
        version: run.version(),
        failed_jobs,
    }
}

/// What started a run, for people.
fn started_by(event: &str) -> String {
    match event {
        "schedule" => "its schedule".into(),
        "workflow_dispatch" => "hand (Run workflow)".into(),
        "push" => "a push".into(),
        "pull_request" | "pull_request_target" => "a pull request".into(),
        "merge_group" => "the merge queue".into(),
        "release" => "a release".into(),
        "workflow_call" | "workflow_run" => "another workflow".into(),
        "" => "GitHub".into(),
        other => format!("`{other}`"),
    }
}

/// The failed jobs as a Markdown list (the workflow alone when it names none).
fn job_list(run: &WorkflowRun, jobs: &[String]) -> String {
    if jobs.is_empty() {
        return format!("- {}", run.run.name);
    }
    jobs.iter().map(|j| format!("- {j}")).collect::<Vec<_>>().join("\n")
}

/// Text for inside a Markdown link's brackets: a run's title is its commit's or pull request's (outside input), and a
/// bracket or backslash in it would break the link.
fn link_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(if c == '\n' { ' ' } else { c });
    }
    out
}

/// The item's text: what failed, where, and what Re-run does.
fn item_text(run: &WorkflowRun, jobs: &str, can_rerun: bool) -> String {
    let attempt = if run.run.attempt > 1 { format!(" (attempt {})", run.run.attempt) } else { String::new() };
    let version = run.version().map(|v| format!("\n- Version: `{v}`")).unwrap_or_default();
    let rerun = if can_rerun {
        format!(
            "**Re-run failed jobs** runs them again on GitHub as your GitHub login; if your login can't, re-run them \
             from [the run]({}).",
            run.run.url
        )
    } else {
        format!("Re-run its failed jobs from [the run]({}).", run.run.url)
    };
    format!(
        "[{title}]({url}){attempt} of the **{workflow}** workflow failed.\n\n- Branch: `{branch}`\n- Started by \
         {by}{version}\n\nFailed jobs:\n\n{jobs}\n\n{rerun} This item goes away by itself once a later attempt of the \
         run passes.",
        title = link_text(&run.title),
        url = run.run.url,
        workflow = run.run.name,
        branch = run.run.branch,
        by = started_by(&run.event),
    )
}

pub struct GithubWorkflowWatch;

impl NodeKind for GithubWorkflowWatch {
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubWorkflowWatchConfig, GithubWorkflowWatchOutput>(
            "github.workflow.watch",
            1,
            Category::Action,
            "Report a failed workflow run",
            "Puts one Inbox item about a failed GitHub Actions run (`runId`) in the Inbox: its workflow, title, \
             branch, what started it, its version or tag when it has one, its failed jobs and a link, with Re-run \
             failed jobs (`rerun`) and Dismiss (`dismiss`). It looks at the run again every 10 minutes: the item \
             closes by itself when a later attempt passes (`passed`), and a later failed attempt replaces it. After \
             `watchFor` (default 7 days) it stops looking and the item waits for you. `cancelled` when the run was \
             cancelled before it was reported, `none` when there is no such run, `already` when its attempt is in the \
             Inbox already.",
            &[
                ("passed", "A later attempt passed"),
                (RERUN, "Re-run failed jobs"),
                (DISMISS, "Dismiss"),
                ("cancelled", "Cancelled"),
                ("none", "No such run"),
                ("already", "Already in the Inbox"),
                ("expired", "Stopped looking"),
            ],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubWorkflowWatchConfig = parse(config)?;
        check_run_id(&c.run_id)?;
        if let Some(w) = nonblank(&c.watch_for).filter(|w| !templated(w)) {
            parse_every(w)?;
        }
        check_repo(&c.repo)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubWorkflowWatchConfig = parse(&input)?;
            let id = text_of(&c.run_id).ok_or("runId is empty")?;
            let after: u32 = text_of(&c.after).and_then(|a| a.parse().ok()).unwrap_or(0);
            let of: RepoOf = repo_for_step(cx, &c.repo).await?;
            if cx.dry_run {
                return Ok(Outcome::next(
                    "passed",
                    json!({"would": format!("report run {id} of {} in the Inbox until it passes", of.repo)}),
                ));
            }
            let actions = of.actions("read the run from")?;
            let now = crate::routes::now_ms();
            // A look after the first keeps the first's start (its saved output).
            let since = cx.step.output.get("since").and_then(Value::as_i64).unwrap_or(now);
            let watch_ms = parse_every(nonblank(&c.watch_for).unwrap_or(DEFAULT_WATCH))?;
            let watching = now - since < i64::try_from(watch_ms).unwrap_or(i64::MAX);
            let can_rerun = actions.capabilities().retry;
            let choices: Vec<&str> = if can_rerun { vec![RERUN, DISMISS] } else { vec![DISMISS] };
            let store = &cx.st.store;
            let open = store.open_items_for_step(&cx.step.id).await.map_err(crate::workflow::err)?;
            // It waits for the user's choice, and (`retry`) looks again until it stops watching: the item stays open
            // while the step does, and closes when it leaves by any port.
            let wait = |title: &str, progress: Value| {
                let mut progress = progress;
                progress["since"] = json!(since);
                let until = watching.then(|| {
                    let next = now + i64::try_from(LOOK_MS).unwrap_or(i64::MAX / 2);
                    next.min(since + i64::try_from(watch_ms).unwrap_or(i64::MAX / 2))
                });
                Outcome::Wait {
                    wait: json!({"retry": true, "answer": {"question": title, "choices": choices}}),
                    until,
                    timeout_port: None,
                    output: progress,
                }
            };
            let saved_title = cx.step.output.get("question").and_then(Value::as_str).unwrap_or("").to_string();
            let read = match read_run(&actions, &of.repo, &id).await {
                Ok(r) => r,
                // While it watches, a failed look waits for the next one: each look is one of the step's attempts,
                // so the node's retries would soon be used up.
                Err(e) if cx.step.output.get("since").is_some() => {
                    tracing::warn!(error = %e, run = %id, "could not read a workflow run; looking again later");
                    let mut keep = cx.step.output.clone();
                    keep["quiet"] = json!(true);
                    return Ok(wait(&saved_title, keep));
                }
                Err(e) => return Err(e),
            };
            let quiet = |port: &str, extra: Value| {
                let mut o = json!({"runId": id, "repo": of.repo, "quiet": true});
                if let (Some(o), Some(x)) = (o.as_object_mut(), extra.as_object()) {
                    o.extend(x.clone());
                }
                Outcome::Next { port: port.into(), output: o }
            };
            let Some(run) = read else { return Ok(quiet("none", json!({}))) };
            let title = c
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| format!("{} failed on {}", run.run.name, run.run.branch));
            let progress = |jobs: Option<String>| {
                let mut v = serde_json::to_value(output_of(&run, jobs)).unwrap_or_default();
                v["question"] = json!(title);
                v
            };
            match run.run.state {
                CiState::Passed => return Ok(Outcome::next("passed", output_of(&run, None))),
                CiState::Cancelled if open.is_empty() => return Ok(quiet("cancelled", json!({"url": run.run.url}))),
                _ if !watching && open.is_empty() => return Ok(quiet("expired", json!({"url": run.run.url}))),
                // The attempt re-run before the watch came back, not yet replaced by GitHub's next one; a run going.
                CiState::Failed if run.run.attempt <= after => return Ok(wait(&title, progress(None))),
                CiState::Failed => {}
                _ => return Ok(wait(&title, progress(None))),
            }
            // Its item already shows this attempt.
            let attempt = run.run.attempt;
            if open.iter().any(|i| i.body.get("attempt").and_then(Value::as_u64) == Some(u64::from(attempt))) {
                let jobs = cx.step.output.get("failedJobs").and_then(Value::as_str).map(String::from);
                return Ok(wait(&title, progress(jobs)));
            }
            let (a, r, ci) = (actions.clone(), of.repo.clone(), run.run.clone());
            // Which jobs failed is only for the message: a failed lookup leaves them out.
            let jobs = blocking(move || pdt_forge::ci::failed_job_names(a.as_ref(), &r, &ci)).await.unwrap_or_default();
            let jobs = job_list(&run, &jobs);
            let text = item_text(&run, &jobs, can_rerun);
            let mut choice_list = vec![];
            if can_rerun {
                choice_list.push(InboxChoice::new(RERUN, "Re-run failed jobs", true));
            }
            choice_list.push(InboxChoice::new(DISMISS, "Dismiss", !can_rerun));
            let body = json!({"question": title, "text": text, "url": run.run.url, "workflow": cx.workflow.name,
                              "node": cx.node_title(), "runId": id, "attempt": attempt});
            let item = InboxItem {
                created_at: now,
                ..cx.inbox_item(InboxKind::Gate, InboxGroup::Gate, &title, body).choices(choice_list)
            };
            // Once per run attempt: the trigger starts a run for each failed attempt, and the watch that reported an
            // earlier one may see this one too; whichever is first reports it.
            let key = format!("workflow-run:{}:{}:{id}:{attempt}", cx.workflow.id, of.repo.to_ascii_lowercase());
            let added = store.add_inbox_item_once(&item, &key).await.map_err(crate::workflow::err)?;
            // A later failed attempt replaces the earlier one's item, here or in the run that reported it.
            for old in &open {
                let gone = json!({"replacedBy": attempt});
                store
                    .end_inbox_item(&old.id, InboxStatus::Closed, &gone, now)
                    .await
                    .log_warn("close a replaced Inbox item");
            }
            if added.is_none() {
                return Ok(quiet("already", json!({"attempt": attempt})));
            }
            crate::notifications::kick(cx.st);
            Ok(wait(&title, progress(Some(jobs))))
        })
    }
}

// ---- github.workflow.rerun ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubWorkflowRerunConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The run's id (`{{trigger.runId}}`).
    #[ts(type = "number | string")]
    pub run_id: Option<Value>,
    /// Re-run every job, not only the failed ones.
    #[serde(default)]
    pub all_jobs: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubWorkflowRerunOutput {
    pub run_id: String,
    /// The attempt that was re-run (the next one is GitHub's).
    pub attempt: u32,
    pub url: String,
}

pub struct GithubWorkflowRerun;

impl NodeKind for GithubWorkflowRerun {
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubWorkflowRerunConfig, GithubWorkflowRerunOutput>(
            "github.workflow.rerun",
            1,
            Category::Action,
            "Re-run a workflow run's failed jobs",
            "Re-runs the failed jobs of a GitHub Actions run (`runId`; every job with `allJobs`) as PDT's GitHub \
             login, which needs the Actions write permission (else it fails: take its `error` port). Output: `runId`, \
             the `attempt` it re-ran and the run's `url`. `none` when there is no such run.",
            &[("out", "Then"), ("none", "No such run")],
            false,
        )
    }

    /// Asked twice, GitHub re-runs twice (or refuses while the first is going).
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubWorkflowRerunConfig = parse(config)?;
        check_run_id(&c.run_id)?;
        check_repo(&c.repo)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubWorkflowRerunConfig = parse(&input)?;
            let id = text_of(&c.run_id).ok_or("runId is empty")?;
            let of = repo_for_step(cx, &c.repo).await?;
            let which = if c.all_jobs { "every job" } else { "the failed jobs" };
            if cx.dry_run {
                return Ok(Outcome::next(
                    "out",
                    json!({"would": format!("re-run {which} of run {id} in {}", of.repo)}),
                ));
            }
            let actions = of.actions("re-run the run with")?;
            let Some(run) = read_run(&actions, &of.repo, &id).await? else {
                return Ok(Outcome::Next { port: "none".into(), output: json!({"runId": id, "quiet": true}) });
            };
            let (a, r, ci, failed_only) = (actions.clone(), of.repo.clone(), run.run.clone(), !c.all_jobs);
            blocking(move || a.retry(&r, &ci, failed_only)).await?;
            Ok(Outcome::next(
                "out",
                GithubWorkflowRerunOutput { run_id: run.run.id, attempt: run.run.attempt, url: run.run.url },
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let watch = |v: Value| GithubWorkflowWatch.check(&v);
        assert!(watch(json!({"runId": "{{trigger.runId}}", "after": "{{steps.rerun.attempt}}"})).is_ok());
        assert!(watch(json!({"runId": 77, "watchFor": "2d", "repo": "o/r"})).is_ok());
        assert!(watch(json!({"runId": "x"})).unwrap_err().contains("not a run"));
        assert!(watch(json!({})).unwrap_err().contains("runId"));
        assert!(watch(json!({"runId": 7, "watchFor": "soon"})).is_err());
        assert!(watch(json!({"runId": 7, "repo": "nope"})).is_err());

        let rerun = |v: Value| GithubWorkflowRerun.check(&v);
        assert!(rerun(json!({"runId": "{{trigger.runId}}"})).is_ok());
        assert!(rerun(json!({"runId": "77", "allJobs": true})).is_ok());
        assert!(rerun(json!({"runId": ""})).is_err());
    }

    #[test]
    fn the_item_says_what_failed_where_and_how_to_re_run_it() {
        let ci = pdt_forge::ci::testing::FakeCi::actions();
        let mut run =
            ci.workflow_run("77", 2, ".github/workflows/release.yml", "v1.2.0", "schedule", CiState::Failed, "t");
        run.title = "Release v1.2.0".into();
        let jobs = job_list(&run, &["build (macos)".into(), "publish".into()]);
        let text = item_text(&run, &jobs, true);
        for part in [
            "[Release v1.2.0](https://ci.example/github_actions/runs/77) (attempt 2)",
            "**release** workflow",
            "Branch: `v1.2.0`",
            "Started by its schedule",
            "Version: `v1.2.0`",
            "- build (macos)\n- publish",
            "**Re-run failed jobs**",
            "goes away by itself",
        ] {
            assert!(text.contains(part), "{part:?} in {text}");
        }
        assert!(!item_text(&run, &jobs, false).contains("**Re-run failed jobs**"));
        assert_eq!(job_list(&run, &[]), "- release");
        run.title = "Fix [ci] (again)\nmore".into();
        assert!(item_text(&run, &jobs, true).starts_with(r"[Fix \[ci\] (again) more](https://ci.example/"));
        assert_eq!(started_by("workflow_dispatch"), "hand (Run workflow)");
    }
}
