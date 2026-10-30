//! Pull requests PDT didn't open (PDT-269), through the GitHub forge (the run's project's, else GitHub's account, as
//! the issue actions): `github.pr.ticket` (the external ticket that is the pull request's home in PDT),
//! `github.pr.merge` (set it to merge: auto-merge or the merge queue), `github.pr.review` (submit a review) and
//! `github.pr.close`. None of them touches the contributor's branch. A ticket links a pull request as `owner/name#12`,
//! as an issue ([`pdt_core::github_issue`]: they share numbers); linking is the dedupe.

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::forge::ForgeKind;
use pdt_core::github_issue::{GITHUB, issue_key, repo_id, valid_repo};
use pdt_core::pr::{Mergeable, PrState};
use pdt_core::ticket::{Actor, NewTicket};
use pdt_forge::{Forge, ForgeError, ReviewVerdict};
use pdt_store::StoredExternalLink;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use ts_rs::TS;

/// Runs a call on GitHub off the async threads: the run's project's forge when it is on GitHub, else GitHub's account.
async fn gh<T: Send + 'static>(
    cx: &StepCx<'_>,
    f: impl FnOnce(&dyn Forge) -> Result<T, ForgeError> + Send + 'static,
) -> Result<T, String> {
    let api: Arc<dyn Forge> = crate::forge::forge_for(cx.st, &ForgeKind::GITHUB, cx.run.project_id.as_deref()).await?;
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

/// A pull request's repository and number as a config names them (`{{trigger.repo}}`, `{{trigger.number}}`).
struct PrRef<'a> {
    repo: &'a str,
    number: &'a Value,
}

impl PrRef<'_> {
    /// Checks what isn't a reference yet.
    fn check(&self) -> Result<(), String> {
        let repo = self.repo.trim();
        if repo.is_empty() {
            return Err("repo is empty: name the repository (owner/name)".into());
        }
        if !templated(repo) && valid_repo(repo).is_none() {
            return Err(format!("{repo:?} is not owner/name"));
        }
        match self.number {
            Value::String(s) if templated(s) => Ok(()),
            _ => self.number().map(|_| ()),
        }
    }

    fn number(&self) -> Result<u32, String> {
        let text = match self.number {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.trim().trim_start_matches('#').to_string(),
            _ => String::new(),
        };
        text.parse().map_err(|_| format!("{text:?} is not a pull request number"))
    }

    /// The repository and number, filled in.
    fn resolved(&self) -> Result<(String, u32), String> {
        let repo = self.repo.trim();
        valid_repo(repo).ok_or_else(|| format!("{repo:?} is not owner/name"))?;
        Ok((repo.to_string(), self.number()?))
    }
}

/// The configs that name a pull request.
macro_rules! names_a_pr {
    ($($t:ty),*) => {$(
        impl $t {
            fn pr(&self) -> PrRef<'_> {
                PrRef { repo: &self.repo, number: &self.number }
            }
        }
    )*};
}
names_a_pr!(PrTicketConfig, PrMergeConfig, PrReviewConfig, PrCloseConfig);

// ---- github.pr.ticket ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PrTicketConfig {
    /// The repository (`owner/name`, `{{trigger.repo}}`).
    pub repo: String,
    /// The pull request's number (`{{trigger.number}}`).
    #[ts(type = "number | string")]
    pub number: Value,
    /// The PDT project the ticket goes in (`{{trigger.projectId}}`).
    pub project: String,
    /// The pull request's title (`{{trigger.title}}`), its link and author, and its text: the new ticket's.
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub body: String,
    /// Make the ticket when there is none (else leave by `none`).
    #[serde(default = "yes")]
    pub create: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrTicketOutput {
    pub ticket_id: String,
    pub ticket_key: String,
    /// `owner/name#12`.
    pub pr: String,
    /// The ticket's stage.
    pub stage: pdt_core::ticket::TicketStage,
    /// It is Done or Won't do.
    pub closed: bool,
}

/// The new ticket's description: the pull request's text, then who opened it where.
fn pr_description(c: &PrTicketConfig, key: &str) -> String {
    let at = if c.url.trim().is_empty() { key } else { c.url.trim() };
    let from = match c.author.trim() {
        "" => format!("Pull request {at}"),
        a => format!("Pull request by @{a}: {at}"),
    };
    let body = c.body.trim();
    let text = if body.is_empty() { from } else { format!("{body}\n\n---\n{from}") };
    text.chars().take(pdt_core::ticket::MAX_DESCRIPTION).collect()
}

pub struct PrTicket;

impl NodeKind for PrTicket {
    fn spec(&self) -> KindSpec {
        spec::<PrTicketConfig, PrTicketOutput>(
            "github.pr.ticket",
            1,
            Category::Action,
            "The pull request's ticket",
            "Finds the ticket linked to a GitHub pull request (`exists`), or makes one (`created`): an external \
             Draft in `project` with the pull request's title, text, author and link, linked to it, so it is never \
             made twice. Without `create`, a pull request with no ticket leaves by `none`. Output: `ticketId`, \
             `ticketKey`, `pr` (owner/name#12), `stage` and `closed` (Done or Won't do).",
            &[("created", "Made"), ("exists", "Already a ticket"), ("none", "No ticket")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: PrTicketConfig = parse(config)?;
        if c.project.trim().is_empty() {
            return Err("project is empty: name the PDT project ({{trigger.projectId}})".into());
        }
        c.pr().check()
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PrTicketConfig = parse(&input)?;
            let (repo, number) = c.pr().resolved()?;
            let key = issue_key(&repo, u64::from(number));
            if cx.dry_run {
                // Later steps name the ticket: a dry run's is blank.
                let would = format!("find or make the ticket of {key}");
                return Ok(Outcome::next(
                    "created",
                    json!({"would": would, "ticketId": "", "ticketKey": "", "pr": key, "closed": false}),
                ));
            }
            let store = &cx.st.store;
            let output = |t: &pdt_core::ticket::TicketSummary| PrTicketOutput {
                ticket_id: t.id.clone(),
                ticket_key: t.key.clone(),
                pr: key.clone(),
                stage: t.stage,
                closed: t.stage.is_closed(),
            };
            let linked = store.ticket_for_external(GITHUB, &repo_id(&repo), &key).await.map_err(|e| e.to_string())?;
            if let Some(id) = linked {
                let t = store.get_ticket_summary(&id).await.map_err(|e| e.to_string())?.ok_or("the ticket is gone")?;
                return Ok(Outcome::next("exists", output(&t)));
            }
            if !c.create {
                return Ok(Outcome::Next { port: "none".into(), output: json!({"pr": key, "quiet": true}) });
            }
            let project = c.project.trim();
            if project.is_empty() {
                return Err("project is empty: the pull request's repository is no PDT project's".into());
            }
            let title: String = match c.title.trim() {
                "" => key.clone(),
                t => t.chars().take(pdt_core::ticket::MAX_TITLE).collect(),
            };
            let new = NewTicket { title, description: Some(pr_description(&c, &key)), ..Default::default() };
            let valid = new.validate().map_err(|e| e.to_string())?;
            let url = match c.url.trim() {
                "" => format!("https://github.com/{repo}/pull/{number}"),
                u => u.to_string(),
            };
            let link = StoredExternalLink {
                ticket_id: String::new(),
                system: GITHUB.into(),
                connection_id: repo_id(&repo),
                key: key.clone(),
                url,
            };
            let id = uuid::Uuid::now_v7().to_string();
            match store.create_linked_ticket(&id, project, &valid, &link, Actor::System, now_ms()).await {
                Ok(Ok(t)) => {
                    crate::untrusted::record_run_on_ticket(cx.st, cx.run, &t.summary.id).await;
                    Ok(Outcome::next("created", output(&t.summary)))
                }
                Ok(Err(other)) => {
                    let t = store.get_ticket_summary(&other).await.map_err(|e| e.to_string())?.ok_or("gone")?;
                    Ok(Outcome::next("exists", output(&t)))
                }
                Err(e) => Err(e.to_string()),
            }
        })
    }

    /// A second run would find the first one's ticket.
    fn rerun_safe(&self, _input: &Value) -> bool {
        true
    }
}

// ---- github.pr.merge ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PrMergeConfig {
    /// The repository (`owner/name`, `{{trigger.repo}}`).
    pub repo: String,
    /// The pull request's number (`{{trigger.number}}`).
    #[ts(type = "number | string")]
    pub number: Value,
    /// Merge only while its head is this commit (`{{trigger.headSha}}`: the one reviewed and approved). A push since
    /// leaves by `refused` with `code` `head_moved`, and GitHub is told the commit too, so one in between is refused.
    #[serde(default)]
    pub head_sha: Option<String>,
}

pub struct PrMerge;

impl NodeKind for PrMerge {
    fn spec(&self) -> KindSpec {
        spec::<PrMergeConfig, Value>(
            "github.pr.merge",
            1,
            Category::Action,
            "Merge a GitHub pull request",
            "Sets a pull request PDT didn't open to merge, as Merge does for an attempt's: auto-merge (GitHub holds it \
             until the base branch's required checks pass) or the merge queue, with the strategy the repository \
             allows (`queued`), or merged at once when GitHub can (`merged`). With `headSha`, only while its head is \
             that commit (the one approved): `refused` with `code` `head_moved` otherwise. It never pushes to the \
             pull request's branch: one that is closed, a draft or in conflict with its base leaves by `refused` with \
             `code` and `reason`, as does a merge GitHub refuses.",
            &[("queued", "Set to merge"), ("merged", "Merged"), ("refused", "Refused")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<PrMergeConfig>(config)?.pr().check()
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PrMergeConfig = parse(&input)?;
            let (repo, number) = c.pr().resolved()?;
            if cx.dry_run {
                return Ok(Outcome::next("queued", json!({"would": format!("merge {repo}#{number}")})));
            }
            let refused = |code: &str, reason: String| {
                Ok(Outcome::next("refused", json!({"code": code, "reason": reason, "repo": repo, "number": number})))
            };
            let r = repo.clone();
            let Some(pr) = gh(cx, move |f| f.change_request_number(&r, number)).await? else {
                return refused("missing", format!("GitHub has no pull request {repo}#{number}"));
            };
            match pr.state {
                PrState::Merged => {
                    return Ok(Outcome::next("merged", json!({"already": true, "url": pr.url})));
                }
                PrState::Closed => return refused("closed", format!("{repo}#{number} is closed")),
                PrState::Open => {}
            }
            if pr.draft {
                return refused("draft", format!("{repo}#{number} is a draft"));
            }
            if pr.mergeable == Mergeable::Conflicting {
                return refused(
                    "conflict",
                    format!("{repo}#{number} conflicts with {}: its author has to resolve it", pr.base_branch),
                );
            }
            let head = nonblank(&c.head_sha).map(String::from);
            if let Some(h) = head.as_deref().filter(|h| *h != pr.head_sha) {
                return refused(
                    "head_moved",
                    format!("{repo}#{number} was pushed to since {h} was approved: its head is {}", pr.head_sha),
                );
            }
            if pr.auto_merge || pr.in_merge_queue {
                return Ok(Outcome::next("queued", json!({"already": true, "url": pr.url})));
            }
            let (r, base) = (repo.clone(), pr.base_branch.clone());
            let merged = gh(cx, move |f| {
                let strategy = f.merge_settings(&r, &base)?.strategy();
                f.merge_head(&r, number, strategy, head.as_deref())?;
                Ok(f.change_request_number(&r, number)?.is_some_and(|p| p.state == PrState::Merged))
            })
            .await;
            match merged {
                Ok(true) => Ok(Outcome::next("merged", json!({"url": pr.url}))),
                Ok(false) => Ok(Outcome::next("queued", json!({"url": pr.url}))),
                Err(e) => refused("forge", e),
            }
        })
    }
}

// ---- github.pr.review ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PrReviewConfig {
    /// The repository (`owner/name`, `{{trigger.repo}}`).
    pub repo: String,
    /// The pull request's number (`{{trigger.number}}`).
    #[ts(type = "number | string")]
    pub number: Value,
    /// `request_changes` (the default), `comment` or `approve`.
    #[serde(default)]
    pub verdict: Option<String>,
    /// Markdown (e.g. the user's note with their choice: `{{steps.ask.text}}`).
    #[serde(default)]
    #[schemars(extend("x-format" = "markdown"))]
    pub body: Option<String>,
    /// The text when `body` comes to nothing (e.g. the user's note, else the review: `{{steps.review.summary}}`).
    #[serde(default)]
    #[schemars(extend("x-format" = "markdown"))]
    pub fallback: Option<String>,
}

fn verdict_of(v: &Option<String>) -> Result<ReviewVerdict, String> {
    match nonblank(v) {
        None => Ok(ReviewVerdict::RequestChanges),
        Some(t) => ReviewVerdict::parse(t)
            .ok_or_else(|| format!("{t:?} is not a verdict: request_changes, comment or approve")),
    }
}

pub struct PrReview;

impl NodeKind for PrReview {
    fn spec(&self) -> KindSpec {
        spec::<PrReviewConfig, Value>(
            "github.pr.review",
            1,
            Category::Action,
            "Review a GitHub pull request",
            "Submits a review of a pull request as PDT's GitHub login: `request_changes` (the default), `comment` or \
             `approve`, with `body` (or `fallback` when it comes to nothing). Its URL is `{{steps.<id>.url}}`.",
            &[("out", "Then")],
            false,
        )
    }

    /// Submitted twice, it shows twice.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: PrReviewConfig = parse(config)?;
        if c.verdict.as_deref().is_some_and(|v| !templated(v)) {
            verdict_of(&c.verdict)?;
        }
        if nonblank(&c.body).is_none() && nonblank(&c.fallback).is_none() {
            return Err("body is empty".into());
        }
        c.pr().check()
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PrReviewConfig = parse(&input)?;
            let (repo, number) = c.pr().resolved()?;
            let verdict = verdict_of(&c.verdict)?;
            let body = nonblank(&c.body).or(nonblank(&c.fallback)).ok_or("the review has no text")?.to_string();
            if cx.dry_run {
                return Ok(Outcome::next(
                    "out",
                    json!({"would": format!("review {repo}#{number} ({}): {body}", verdict.as_str())}),
                ));
            }
            let (r, text) = (repo.clone(), body.clone());
            let url = gh(cx, move |f| f.review_change_request(&r, number, verdict, &text)).await?;
            Ok(Outcome::next("out", json!({"url": url, "verdict": verdict.as_str(), "body": body})))
        })
    }
}

// ---- github.pr.close ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PrCloseConfig {
    /// The repository (`owner/name`, `{{trigger.repo}}`).
    pub repo: String,
    /// The pull request's number (`{{trigger.number}}`).
    #[ts(type = "number | string")]
    pub number: Value,
    /// A comment to add first (Markdown).
    #[serde(default)]
    #[schemars(extend("x-format" = "markdown"))]
    pub comment: Option<String>,
}

pub struct PrClose;

impl NodeKind for PrClose {
    fn spec(&self) -> KindSpec {
        spec::<PrCloseConfig, Value>(
            "github.pr.close",
            1,
            Category::Action,
            "Close a GitHub pull request",
            "Closes a pull request without merging it, adding `comment` first. One already closed or merged counts as \
             done: nothing is changed.",
            &[("out", "Then")],
            false,
        )
    }

    /// A comment added before it was cut short would be added again.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<PrCloseConfig>(config)?.pr().check()
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PrCloseConfig = parse(&input)?;
            let (repo, number) = c.pr().resolved()?;
            let comment = nonblank(&c.comment).map(String::from);
            if cx.dry_run {
                let mut would = format!("close {repo}#{number}");
                if let Some(t) = &comment {
                    would.push_str(&format!(", commenting: {t}"));
                }
                return Ok(Outcome::next("out", json!({"would": would})));
            }
            let r = repo.clone();
            let open = gh(cx, move |f| f.change_request_number(&r, number)).await?.filter(|p| p.state == PrState::Open);
            if open.is_none() {
                return Ok(Outcome::next("out", json!({"alreadyClosed": true, "quiet": true})));
            }
            let r = repo.clone();
            let url = gh(cx, move |f| {
                let url = match comment {
                    Some(body) => Some(f.comment_change_request(&r, number, &body)?),
                    None => None,
                };
                f.close_change_request(&r, number)?;
                Ok(url)
            })
            .await?;
            Ok(Outcome::next("out", json!({"pr": issue_key(&repo, u64::from(number)), "commentUrl": url})))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let pr = json!({"repo": "{{trigger.repo}}", "number": "{{trigger.number}}"});
        let with = |extra: Value| {
            let mut v = pr.clone();
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            v
        };
        assert!(PrMerge.check(&pr).is_ok());
        assert!(PrMerge.check(&json!({"repo": "o/r", "number": 4})).is_ok());
        assert!(PrMerge.check(&json!({"repo": "nope", "number": 4})).unwrap_err().contains("owner/name"));
        assert!(PrMerge.check(&json!({"repo": "o/r", "number": "x"})).is_err());
        assert!(PrClose.check(&with(json!({"comment": "Thanks"}))).is_ok());
        assert!(PrReview.check(&with(json!({"body": "Add tests"}))).is_ok());
        assert!(PrReview.check(&with(json!({"body": "", "fallback": "{{steps.review.summary}}"}))).is_ok());
        assert!(PrReview.check(&with(json!({"body": " "}))).unwrap_err().contains("empty"));
        assert!(PrReview.check(&with(json!({"body": "x", "verdict": "lgtm"}))).unwrap_err().contains("verdict"));
        assert!(PrTicket.check(&with(json!({"project": "{{trigger.projectId}}"}))).is_ok());
        assert!(PrTicket.check(&with(json!({"project": " "}))).unwrap_err().contains("project"));
    }

    #[test]
    fn a_ticket_describes_the_pull_request() {
        let c: PrTicketConfig = serde_json::from_value(json!({
            "repo": "o/r", "number": 7, "project": "p", "title": "Fix it", "author": "ann",
            "url": "https://github.com/o/r/pull/7", "body": "It was broken."}))
        .unwrap();
        assert!(c.create, "makes one by default");
        assert_eq!(
            pr_description(&c, "o/r#7"),
            "It was broken.\n\n---\nPull request by @ann: https://github.com/o/r/pull/7"
        );
    }
}
