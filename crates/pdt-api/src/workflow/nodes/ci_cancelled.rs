//! CI cancelled on a pull request's head with nothing else running (PDT-510): nothing would ever move the ticket on.
//! `ci.rerun_head` re-runs the cancelled runs of the head, through each run's CI provider, and only while that head
//! is still the pull request's and its CI is still cancelled; `ci.stalled` puts one stalled item in the Inbox (Re-run
//! CI, Dismiss) that closes by itself once the head moves on or its CI runs again. The built-in "When CI is cancelled"
//! puts them after `ci.run_cancelled`.

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, parses, spec};
use pdt_core::inbox::{InboxChoice, InboxGroup, InboxKind};
use pdt_core::pr::{CiRun, CiState, PullRequestStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const RERUN_KIND: &str = "ci.rerun_head";
pub const STALLED_KIND: &str = "ci.stalled";
/// The stalled item's choices (ports), and the port of a head that moved on.
pub const RERUN: &str = "rerun";
pub const DISMISS: &str = "dismiss";
pub const MOVED_ON: &str = "moved_on";
/// How often the stalled item looks at the head again.
const LOOK_MS: i64 = 60_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CiHeadConfig {
    /// The attempt (`{{trigger.attemptId}}`).
    pub attempt: String,
    /// The head commit whose CI was cancelled (`{{trigger.headSha}}`): a pull request that moved on to another head is
    /// left alone.
    pub head: String,
}

/// The attempt's open pull request as last seen, against `head`: it and the head's cancelled runs while its CI is
/// still cancelled, else (`Err`) why it moved on.
fn head_now(cx: &StepCx<'_>, attempt: &str, head: &str) -> Result<(PullRequestStatus, Vec<CiRun>), String> {
    let Some((pr, _)) = crate::github::head_ci(cx.st, attempt) else {
        return Err("the pull request is not open".into());
    };
    if pr.head_sha != head {
        return Err(format!("the pull request moved on to {}", short(&pr.head_sha)));
    }
    match pr.ci.as_ref().map(|c| c.state) {
        Some(CiState::Cancelled) => {
            let runs = pr.ci.iter().flat_map(|c| c.runs.iter()).filter(|r| r.state == CiState::Cancelled).cloned();
            Ok((pr.clone(), runs.collect()))
        }
        Some(s) => Err(format!("CI on {} is {} now", short(head), state_word(s))),
        None => Err(format!("{} has no CI runs now", short(head))),
    }
}

fn state_word(s: CiState) -> &'static str {
    match s {
        CiState::Passed => "passed",
        CiState::Cancelled => "cancelled",
        CiState::Queued => "queued",
        CiState::Running => "running",
        CiState::Failed => "failed",
    }
}

/// A commit as people read it: its first 8 characters.
pub(crate) fn short(sha: &str) -> &str {
    sha.get(..8).unwrap_or(sha)
}

fn moved_on(why: String) -> Outcome {
    Outcome::Next { port: MOVED_ON.into(), output: json!({"quiet": true, "reason": why}) }
}

/// Re-runs every cancelled run of the head through its provider.
async fn rerun(cx: &StepCx<'_>, attempt: &str, runs: &[CiRun]) -> Result<Vec<String>, String> {
    let info = cx.st.store.get_attempt(attempt).await.map_err(crate::workflow::err)?.ok_or("no such attempt")?;
    let forge = crate::forge::project_forge(cx.st, &info.project_id, &info.repo_path).await;
    let providers = crate::ci::project_ci(cx.st, &info.project_id, &info.repo_path, forge.as_ref()).await;
    let repo = forge.map(|f| f.repo).unwrap_or_default();
    let mut done = vec![];
    for run in runs {
        let p = providers
            .iter()
            .find(|p| p.id() == run.provider)
            .ok_or_else(|| format!("the project no longer has the CI provider {}", run.provider))?
            .clone();
        if !p.capabilities().retry {
            return Err(format!("{} can't re-run runs: re-run {} there", p.label(), run.url));
        }
        let (r, ci) = (repo.clone(), run.clone());
        tokio::task::spawn_blocking(move || p.retry(&r, &ci, false))
            .await
            .map_err(|e| format!("the CI task failed: {e}"))?
            .map_err(|e| format!("could not re-run {}: {e}", run.name))?;
        done.push(run.name.clone());
    }
    Ok(done)
}

pub struct RerunHead;

impl NodeKind for RerunHead {
    fn spec(&self) -> KindSpec {
        spec::<CiHeadConfig, Value>(
            RERUN_KIND,
            1,
            Category::Action,
            "Re-run the pull request head's CI",
            "Reads the attempt's pull request again and re-runs the cancelled CI runs of `head` through their CI \
             provider (every job). Only the pull request's current head counts: `moved_on` when it moved to another \
             commit, isn't open, or its CI is no longer cancelled (running, queued, passed or failed). Fails when a \
             provider can't re-run runs or refuses (take its `error` port). Output: `head`, `short` (its first 8 \
             characters), `pr`, `url` and the re-run `runs`.",
            &[("rerun", "Re-ran it"), (MOVED_ON, "Moved on")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<CiHeadConfig>(config)
    }

    /// Asked twice, the provider re-runs twice (or refuses while the first is going).
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    /// It reads the pull request and calls the CI provider, which may be slow.
    fn in_engine_lock(&self, _category: Category) -> bool {
        false
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CiHeadConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::would("rerun", format!("re-run the cancelled CI on {}", short(&c.head))));
            }
            let info =
                cx.st.store.get_attempt(&c.attempt).await.map_err(crate::workflow::err)?.ok_or("no such attempt")?;
            // A fresh look: the head may have moved on, or CI started again, since the event.
            crate::github::refresh_attempt(cx.st, &info).await;
            let (pr, runs) = match head_now(cx, &c.attempt, &c.head) {
                Ok(found) => found,
                Err(why) => return Ok(moved_on(why)),
            };
            let names = rerun(cx, &c.attempt, &runs).await?;
            Ok(Outcome::next(
                "rerun",
                json!({"head": c.head, "short": short(&c.head), "pr": pr.number, "url": pr.url, "runs": names}),
            ))
        })
    }
}

pub struct Stalled;

/// The stalled item's text.
fn stalled_text(pr: &PullRequestStatus, head: &str, runs: &[CiRun]) -> String {
    let list = runs.iter().map(|r| format!("- [{}]({})", r.name, r.url)).collect::<Vec<_>>().join("\n");
    format!(
        "CI on [{name}]({url})'s head `{head}` was cancelled, and nothing is running, queued or failed there, so \
         nothing moves the ticket on.\n\n{list}\n\nA run is often cancelled by a newer one in the same concurrency \
         group (re-running an older commit's jobs cancels the head's). **Re-run CI** runs the head's cancelled runs \
         again. This item goes away by itself once the pull request moves on to another commit or CI runs on it \
         again.",
        name = pr.name(),
        url = pr.url,
        head = short(head),
    )
}

impl NodeKind for Stalled {
    fn spec(&self) -> KindSpec {
        spec::<CiHeadConfig, Value>(
            STALLED_KIND,
            1,
            Category::Action,
            "Report the stalled pull request",
            "Puts one stalled item in the Inbox about the cancelled CI on `head` of the attempt's pull request, with \
             Re-run CI (`rerun`) and Dismiss (`dismiss`), and links the pull request; the ticket shows as stalled \
             meanwhile. It looks at the pull request again every minute: once the head moved on, isn't open, or its \
             CI runs again, passes or fails, the item closes by itself (`moved_on`).",
            &[(RERUN, "Re-run CI"), (DISMISS, "Dismiss"), (MOVED_ON, "Moved on")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<CiHeadConfig>(config)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CiHeadConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::would(
                    RERUN,
                    format!("report the cancelled CI on {} in the Inbox", short(&c.head)),
                ));
            }
            let (pr, runs) = match head_now(cx, &c.attempt, &c.head) {
                Ok(found) => found,
                Err(why) => return Ok(moved_on(why)),
            };
            let title = format!("CI on {} was cancelled; nothing is running", pr.name());
            let store = &cx.st.store;
            if store.open_items_for_step(&cx.step.id).await.map_err(crate::workflow::err)?.is_empty() {
                let info =
                    store.get_attempt(&c.attempt).await.map_err(crate::workflow::err)?.ok_or("no such attempt")?;
                let body = json!({
                    "question": title, "text": stalled_text(&pr, &c.head, &runs), "url": pr.url, "pr": pr.number,
                    "head": c.head, "reason": "ci_cancelled", "who": "CI", "workflow": cx.workflow.name, "node": cx.node_title(),
                });
                let choices =
                    vec![InboxChoice::new(RERUN, "Re-run CI", true), InboxChoice::new(DISMISS, "Dismiss", false)];
                let item = cx
                    .inbox_item(InboxKind::Gate, InboxGroup::Stalled, &title, body)
                    .choices(choices)
                    .ticket(Some(info.ticket_id.clone()))
                    .attempt(Some(info.attempt.id.clone()));
                let key = format!("ci-stalled:{}", cx.step.id);
                if store.add_inbox_item_once(&item, &key).await.map_err(crate::workflow::err)?.is_some() {
                    crate::notifications::kick(cx.st);
                }
            }
            // It waits for the choice, and looks again every minute (the same attempt each time).
            Ok(Outcome::Wait {
                wait: json!({"retry": true, (crate::workflow::engine::POLL): true,
                             "answer": {"question": title, "choices": [RERUN, DISMISS]}}),
                until: Some(now_ms() + LOOK_MS),
                timeout_port: None,
                output: json!({"head": c.head, "short": short(&c.head), "pr": pr.number, "url": pr.url}),
            })
        })
    }
}
