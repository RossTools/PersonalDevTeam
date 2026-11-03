//! GitHub issue actions (PDT-154), modelled on an issue tracker integration's: `github.import` (an issue as a linked Draft ticket),
//! `github.comment` and `github.close`, through the GitHub forge (PDT-176; the run's project's, else GitHub's account, PDT-241; the access the
//! `github.issue.opened` trigger uses). A ticket links an issue as `owner/name#12` ([`pdt_core::github_issue`]); linking is the dedupe.

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, spec};
use pdt_core::forge::ForgeKind;
use pdt_core::github_issue::{CloseReason, GITHUB, issue_key, parse_issue, repo_id, valid_repo};
use pdt_core::issue_link::LinkOrigin;
use pdt_core::ticket::{AGENTIC_LABEL, Actor, NewTicket, Priority};
use pdt_forge::{Forge, ForgeError, IssueDetail};
use pdt_store::StoredExternalLink;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use ts_rs::TS;

/// Runs an issue call off the async threads on GitHub: the forge of `project` when it is on GitHub, else GitHub's
/// account (PDT-241); an error when that forge has no issues.
async fn gh<T: Send + 'static>(
    st: &AppState,
    project: Option<&str>,
    f: impl FnOnce(&dyn Forge) -> Result<T, ForgeError> + Send + 'static,
) -> Result<T, String> {
    let api: Arc<dyn Forge> = crate::forge::forge_for(st, &ForgeKind::GITHUB, project).await?;
    if !api.capabilities().issues {
        return Err(format!("{} has no issues", api.name()));
    }
    tokio::task::spawn_blocking(move || f(api.as_ref()))
        .await
        .map_err(|e| format!("the GitHub task failed: {e}"))?
        .map_err(|e| format!("GitHub: {e}"))
}

fn templated(s: &str) -> bool {
    s.contains("{{")
}

/// An issue number as a config gives it: a number, or text (`12`, `#12`, `owner/name#12`, its URL).
fn number_text(v: &Option<Value>) -> Option<String> {
    match v {
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

/// The issue `repo` and `number` name, or why not (left alone while either is a template).
fn named_issue(repo: &Option<String>, number: &Option<Value>) -> Result<Option<(String, u64)>, String> {
    let Some(n) = number_text(number) else { return Ok(None) };
    let repo = nonblank(repo);
    if templated(&n) || repo.is_some_and(templated) {
        return Ok(None);
    }
    parse_issue(&n, repo).map(Some).ok_or_else(|| match repo {
        Some(r) if valid_repo(r).is_none() => format!("{r:?} is not owner/name"),
        Some(_) => format!("{n:?} is not an issue number"),
        None => format!("{n:?} is not an issue: give repo (owner/name) with its number, or owner/name#12"),
    })
}

/// Where an issue action points: an issue (`repo` + `number`) or a ticket's linked one.
struct IssueTarget {
    repo: Option<String>,
    number: Option<Value>,
    ticket: Option<String>,
}

impl IssueTarget {
    fn check(&self) -> Result<(), String> {
        match (number_text(&self.number), nonblank(&self.ticket)) {
            (Some(_), None) => named_issue(&self.repo, &self.number).map(|_| ()),
            (None, Some(_)) => Ok(()),
            _ => Err("give either number (with repo) or ticket".into()),
        }
    }

    /// The issue: `Ok(None)` when the ticket is linked to none.
    async fn resolve(&self, st: &AppState) -> Result<Option<(String, u64)>, String> {
        if let Some(t) = nonblank(&self.ticket) {
            let id = crate::mcp::tools::resolve_ticket(st, t, None).await?;
            let links = st.store.external_links(&id, GITHUB).await.map_err(crate::workflow::err)?;
            // The URL keeps the repository's own spelling; the key is lower case.
            return Ok(links.iter().find_map(|l| parse_issue(&l.url, None).or_else(|| parse_issue(&l.key, None))));
        }
        let given = number_text(&self.number).unwrap_or_default();
        named_issue(&self.repo, &self.number)?.map(Some).ok_or_else(|| format!("{given:?} is not an issue"))
    }
}

fn unlinked() -> Outcome {
    Outcome::Next {
        port: "unlinked".into(),
        output: json!({"quiet": true, "reason": "the ticket is not linked to a GitHub issue"}),
    }
}

// ---- github.import ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubImportConfig {
    /// The repository (`owner/name`, `{{trigger.repo}}`).
    pub repo: String,
    /// The issue's number (`{{trigger.number}}`).
    #[ts(type = "number | string")]
    pub number: Value,
    /// The PDT project the ticket goes in (`{{trigger.projectId}}`).
    pub project: String,
    /// The ticket's priority (`low`, `medium`, `high`, `urgent`); one PDT doesn't know is left out.
    #[serde(default)]
    #[ts(type = "string | null")]
    pub priority: Option<Value>,
    /// The ticket's labels; ones PDT can't take are left out.
    #[serde(default)]
    #[ts(type = "string[] | string | null")]
    pub labels: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubImportOutput {
    pub ticket_id: String,
    pub ticket_key: String,
    /// `owner/name#12`.
    pub issue: String,
    pub url: String,
}

/// What an import came to: the ticket, and whether it was there already.
pub(crate) struct Imported {
    pub output: GithubImportOutput,
    pub existed: bool,
}

fn priority_of(v: &Option<Value>) -> Option<Priority> {
    v.as_ref().and_then(Value::as_str).and_then(|p| Priority::parse(&p.trim().to_ascii_lowercase()))
}

fn labels_of(v: &Option<Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(|l| l.trim().to_string()).collect(),
        Some(Value::String(s)) => s.split(',').map(|l| l.trim().to_string()).collect(),
        _ => vec![],
    }
    .into_iter()
    .filter(|l| !l.is_empty())
    .collect()
}

/// The ticket's description: the issue's text, then who reported it where.
fn description(d: &IssueDetail) -> String {
    let body = d.issue.body.trim();
    let from = format!("Reported by @{} in {}", d.issue.author, d.issue.url);
    let text = if body.is_empty() { from } else { format!("{body}\n\n---\n{from}") };
    text.chars().take(pdt_core::ticket::MAX_DESCRIPTION).collect()
}

/// Makes a Draft ticket in `project_id` from issue `number` of `repo`, linked to it, unless a ticket already links it.
pub(crate) async fn import_issue(
    st: &AppState,
    project_id: &str,
    repo: &str,
    number: u64,
    priority: Option<Priority>,
    labels: Vec<String>,
) -> Result<Imported, String> {
    let key = issue_key(repo, number);
    let exists = |ticket_id: String| async move {
        let t =
            st.store.get_ticket(&ticket_id).await.map_err(crate::workflow::err)?.ok_or("the linked ticket is gone")?;
        let url = t.summary.external.iter().find(|l| l.system == GITHUB).map(|l| l.url.clone()).unwrap_or_default();
        Ok::<_, String>(Imported {
            output: GithubImportOutput { ticket_id, ticket_key: t.summary.key, issue: issue_key(repo, number), url },
            existed: true,
        })
    };
    if let Some(t) = st.store.ticket_for_external(GITHUB, &repo_id(repo), &key).await.map_err(crate::workflow::err)? {
        return exists(t).await;
    }
    let r = repo.to_string();
    let d = gh(st, Some(project_id), move |g| g.issue(&r, number))
        .await?
        .ok_or_else(|| format!("GitHub has no issue {repo}#{number} (or PDT's GitHub login can't see it)"))?;
    if d.change_request {
        return Err(format!("{repo}#{number} is a pull request, not an issue"));
    }
    let title: String = match d.issue.title.trim() {
        "" => format!("{repo}#{number}"),
        t => t.chars().take(pdt_core::ticket::MAX_TITLE).collect(),
    };
    // An issue's ticket is external (PDT-201): triage can't hand it to agents with the `agentic` label.
    let labels: Vec<String> = labels.into_iter().filter(|l| !l.eq_ignore_ascii_case(AGENTIC_LABEL)).collect();
    let mut new = NewTicket {
        title,
        description: Some(description(&d)),
        priority,
        labels: (!labels.is_empty()).then_some(labels),
        ..Default::default()
    };
    // A suggested priority or label PDT can't take never stops the import.
    let valid = match new.validate() {
        Ok(v) => v,
        Err(_) => {
            (new.priority, new.labels) = (None, None);
            new.validate().map_err(crate::workflow::err)?
        }
    };
    let link = StoredExternalLink::new("", GITHUB, repo_id(repo), &key, &d.issue.url, LinkOrigin::Imported);
    let id = uuid::Uuid::now_v7().to_string();
    match st.store.create_linked_ticket(&id, project_id, &valid, &link, Actor::System, now_ms()).await {
        Ok(Ok(t)) => Ok(Imported {
            output: GithubImportOutput {
                ticket_id: t.summary.id,
                ticket_key: t.summary.key,
                issue: key,
                url: link.url,
            },
            existed: false,
        }),
        Ok(Err(other)) => exists(other).await,
        Err(e) => Err(e.to_string()),
    }
}

pub struct GithubImport;

impl NodeKind for GithubImport {
    fn spec(&self) -> KindSpec {
        spec::<GithubImportConfig, GithubImportOutput>(
            "github.import",
            1,
            Category::Action,
            "Import a GitHub issue",
            "Makes a Draft ticket from a GitHub issue (its title, text, author and link), linked to it. When a ticket \
             already links the issue nothing is made and the step leaves by `exists`: an issue is never imported \
             twice. Output: `ticketId`, `ticketKey`, `issue` (owner/name#12) and `url`.",
            &[("imported", "Imported"), ("exists", "Already a ticket")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubImportConfig = parse(config)?;
        if c.project.trim().is_empty() {
            return Err("project is empty: name the PDT project ({{trigger.projectId}})".into());
        }
        if c.repo.trim().is_empty() {
            return Err("repo is empty: name the repository (owner/name)".into());
        }
        named_issue(&Some(c.repo), &Some(c.number.clone()))?;
        if number_text(&Some(c.number)).is_none() {
            return Err("number is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubImportConfig = parse(&input)?;
            let (repo, number) =
                named_issue(&Some(c.repo.clone()), &Some(c.number.clone()))?.ok_or("number is not an issue number")?;
            if c.project.trim().is_empty() {
                return Err("project is empty: the issue's repository is no PDT project's".into());
            }
            if cx.dry_run {
                return Ok(Outcome::would("imported", format!("import {repo}#{number} as a ticket")));
            }
            let r =
                import_issue(cx.st, c.project.trim(), &repo, number, priority_of(&c.priority), labels_of(&c.labels))
                    .await?;
            Ok(if r.existed {
                let mut out = serde_json::to_value(r.output).unwrap_or_default();
                out["quiet"] = json!(true);
                Outcome::Next { port: "exists".into(), output: out }
            } else {
                // The sessions that triaged the issue read it (PDT-202): on the new ticket's timeline.
                crate::untrusted::record_run_on_ticket(cx.st, cx.run, &r.output.ticket_id).await;
                Outcome::next("imported", r.output)
            })
        })
    }
}

// ---- github.comment ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubCommentConfig {
    /// The repository (`owner/name`, `{{trigger.repo}}`).
    #[serde(default)]
    pub repo: Option<String>,
    /// The issue's number (`{{trigger.number}}`), or `owner/name#12`, or its URL.
    #[serde(default)]
    #[ts(type = "number | string | null")]
    pub number: Option<Value>,
    /// Or a ticket (its key or id, `{{trigger.ticketId}}`): the issue it is linked to.
    #[serde(default)]
    pub ticket: Option<String>,
    /// Markdown.
    #[schemars(extend("x-format" = "markdown"))]
    pub body: String,
}

impl GithubCommentConfig {
    fn target(&self) -> IssueTarget {
        IssueTarget { repo: self.repo.clone(), number: self.number.clone(), ticket: self.ticket.clone() }
    }
}

pub struct GithubComment;

impl NodeKind for GithubComment {
    fn spec(&self) -> KindSpec {
        spec::<GithubCommentConfig, Value>(
            "github.comment",
            1,
            Category::Action,
            "Comment on a GitHub issue",
            "Adds a comment to a GitHub issue: by repo and number, or a ticket's linked issue (`unlinked` when it has \
             none). Its URL is `{{steps.<id>.url}}`.",
            &[("out", "Then"), ("unlinked", "No linked issue")],
            false,
        )
    }

    /// What it does can't be told apart from doing it twice (PDT-131): cut short, it isn't run again on its own.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubCommentConfig = parse(config)?;
        if c.body.trim().is_empty() {
            return Err("body is empty".into());
        }
        c.target().check()
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubCommentConfig = parse(&input)?;
            let Some((repo, number)) = c.target().resolve(cx.st).await? else { return Ok(unlinked()) };
            if cx.dry_run {
                return Ok(Outcome::would("out", format!("comment on {repo}#{number}: {}", c.body)));
            }
            let (r, body) = (repo.clone(), c.body.clone());
            let url = gh(cx.st, cx.run.project_id.as_deref(), move |g| g.comment_issue(&r, number, &body)).await?;
            Ok(Outcome::next("out", json!({"issue": issue_key(&repo, number), "url": url})))
        })
    }
}

// ---- github.close ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubCloseConfig {
    /// The repository (`owner/name`, `{{trigger.repo}}`).
    #[serde(default)]
    pub repo: Option<String>,
    /// The issue's number (`{{trigger.number}}`), or `owner/name#12`, or its URL.
    #[serde(default)]
    #[ts(type = "number | string | null")]
    pub number: Option<Value>,
    /// Or a ticket (its key or id, `{{trigger.ticketId}}`): the issue it is linked to.
    #[serde(default)]
    pub ticket: Option<String>,
    /// `completed`, `not_planned` or `duplicate`.
    pub reason: String,
    /// With `duplicate`: the issue it duplicates (its number in the same repo, `owner/name#12` or its URL).
    #[serde(default)]
    #[ts(type = "number | string | null")]
    pub duplicate_of: Option<Value>,
    /// A comment to add first (Markdown).
    #[serde(default)]
    #[schemars(extend("x-format" = "markdown"))]
    pub comment: Option<String>,
    /// Labels to add (the issue keeps its others).
    #[serde(default)]
    #[ts(type = "string[] | string | null")]
    pub labels: Option<Value>,
}

impl GithubCloseConfig {
    fn target(&self) -> IssueTarget {
        IssueTarget { repo: self.repo.clone(), number: self.number.clone(), ticket: self.ticket.clone() }
    }
}

pub struct GithubClose;

impl NodeKind for GithubClose {
    fn spec(&self) -> KindSpec {
        spec::<GithubCloseConfig, Value>(
            "github.close",
            1,
            Category::Action,
            "Close a GitHub issue",
            "Closes a GitHub issue (by repo and number, or a ticket's linked issue: `unlinked` when it has none) as \
             `completed`, `not_planned` or `duplicate` (GitHub's Close as duplicate, of the issue `duplicateOf`). It \
             adds `labels` (keeping the others) and `comment` first. An issue already closed counts as done: nothing \
             is changed.",
            &[("out", "Then"), ("unlinked", "No linked issue")],
            false,
        )
    }

    /// A comment added before it was cut short would be added again.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubCloseConfig = parse(config)?;
        c.target().check()?;
        if templated(&c.reason) {
            return Ok(());
        }
        let reason = CloseReason::parse(&c.reason)
            .ok_or_else(|| format!("{:?} is not a reason: completed, not_planned or duplicate", c.reason))?;
        if reason == CloseReason::Duplicate && number_text(&c.duplicate_of).is_none() {
            return Err("a duplicate needs duplicateOf: the issue it duplicates".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubCloseConfig = parse(&input)?;
            let reason = CloseReason::parse(&c.reason)
                .ok_or_else(|| format!("{:?} is not a reason: completed, not_planned or duplicate", c.reason))?;
            let Some((repo, number)) = c.target().resolve(cx.st).await? else { return Ok(unlinked()) };
            let canonical = match reason {
                CloseReason::Duplicate => {
                    let given = number_text(&c.duplicate_of).ok_or("a duplicate needs duplicateOf")?;
                    let (r, n) = parse_issue(&given, Some(&repo))
                        .ok_or_else(|| format!("duplicateOf {given:?} is not an issue"))?;
                    if repo_id(&r) == repo_id(&repo) && n == number {
                        return Err(format!("{repo}#{number} can't be a duplicate of itself"));
                    }
                    Some((r, n))
                }
                _ => None,
            };
            let labels = labels_of(&c.labels);
            let comment = nonblank(&c.comment).map(String::from);
            let key = issue_key(&repo, number);
            if cx.dry_run {
                let of = canonical.as_ref().map(|(r, n)| format!(" of {r}#{n}")).unwrap_or_default();
                let mut would = format!("close {repo}#{number} as {}{of}", reason.as_str());
                if let Some(t) = &comment {
                    would.push_str(&format!(", commenting: {t}"));
                }
                return Ok(Outcome::would("out", would));
            }
            let r = repo.clone();
            let d = gh(cx.st, cx.run.project_id.as_deref(), move |g| g.issue(&r, number))
                .await?
                .ok_or_else(|| format!("GitHub has no issue {repo}#{number} (or PDT's GitHub login can't see it)"))?;
            if d.closed {
                return Ok(Outcome::next(
                    "out",
                    json!({"issue": key, "alreadyClosed": true, "stateReason": d.state_reason, "quiet": true}),
                ));
            }
            let duplicate_id = match &canonical {
                Some((r, n)) => {
                    let (r2, n2) = (r.clone(), *n);
                    let c = gh(cx.st, cx.run.project_id.as_deref(), move |g| g.issue(&r2, n2))
                        .await?
                        .ok_or_else(|| format!("GitHub has no issue {r}#{n} to mark this a duplicate of"))?;
                    Some(c.id)
                }
                None => None,
            };
            if !labels.is_empty() {
                let (r, l) = (repo.clone(), labels.clone());
                gh(cx.st, cx.run.project_id.as_deref(), move |g| g.add_labels(&r, number, &l)).await?;
            }
            let mut url = None;
            if let Some(body) = comment {
                let r = repo.clone();
                url = Some(gh(cx.st, cx.run.project_id.as_deref(), move |g| g.comment_issue(&r, number, &body)).await?);
            }
            let r = repo.clone();
            gh(cx.st, cx.run.project_id.as_deref(), move |g| g.close_issue(&r, number, reason, duplicate_id)).await?;
            let of = canonical.map(|(r, n)| issue_key(&r, n));
            Ok(Outcome::next(
                "out",
                json!({"issue": key, "reason": reason.as_str(), "duplicateOf": of, "commentUrl": url, "labels": labels}),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let ok = |kind: &dyn NodeKind, v: Value| kind.check(&v);
        assert!(ok(&GithubComment, json!({"repo": "o/r", "number": 3, "body": "hi"})).is_ok());
        assert!(ok(&GithubComment, json!({"number": "o/r#3", "body": "hi"})).is_ok());
        assert!(ok(&GithubComment, json!({"ticket": "{{trigger.ticketId}}", "body": "hi"})).is_ok());
        assert!(
            ok(&GithubComment, json!({"repo": "{{trigger.repo}}", "number": "{{trigger.number}}", "body": "x"}))
                .is_ok()
        );
        assert!(ok(&GithubComment, json!({"number": 3, "body": "hi"})).unwrap_err().contains("repo"));
        assert!(ok(&GithubComment, json!({"repo": "o/r", "number": 3, "body": " "})).is_err());
        assert!(ok(&GithubComment, json!({"repo": "o/r", "number": 3, "ticket": "PDT-1", "body": "x"})).is_err());
        assert!(
            ok(&GithubComment, json!({"repo": "nope", "number": 3, "body": "x"})).unwrap_err().contains("owner/name")
        );

        let close = |v: Value| GithubClose.check(&v);
        assert!(close(json!({"ticket": "t", "reason": "completed"})).is_ok());
        assert!(close(json!({"ticket": "t", "reason": "wontfix"})).unwrap_err().contains("not a reason"));
        assert!(close(json!({"ticket": "t", "reason": "duplicate"})).unwrap_err().contains("duplicateOf"));
        assert!(close(json!({"repo": "o/r", "number": 4, "reason": "duplicate", "duplicateOf": 2})).is_ok());
        assert!(close(json!({"ticket": "t", "reason": "{{steps.x.reason}}"})).is_ok());

        let import = |v: Value| GithubImport.check(&v);
        assert!(import(json!({"repo": "o/r", "number": 4, "project": "p"})).is_ok());
        assert!(import(json!({"repo": "o/r", "number": 4, "project": ""})).is_err());
        assert!(import(json!({"repo": "o/r", "number": "x", "project": "p"})).is_err());
    }

    #[test]
    fn suggested_priorities_and_labels_are_lenient() {
        assert_eq!(priority_of(&Some(json!("High"))), Some(Priority::High));
        assert_eq!(priority_of(&Some(json!("p0"))), None);
        assert_eq!(priority_of(&Some(Value::Null)), None);
        assert_eq!(labels_of(&Some(json!(["bug", " ", "ui"]))), ["bug", "ui"]);
        assert_eq!(labels_of(&Some(json!("bug, crash"))), ["bug", "crash"]);
        assert!(labels_of(&None).is_empty());
    }
}
