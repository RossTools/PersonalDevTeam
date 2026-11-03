//! Pull requests PDT didn't open (PDT-268). pdtd looks, every 60 s, at the pull requests of each project's GitHub
//! repository an enabled workflow's `github.pull_request.*` trigger watches, and logs `github.pull_request.opened` for
//! a new one, `github.pull_request.pushed` when an open one's head moves, and `github.pull_request.closed` when one it
//! saw open is closed or merged. A pull request from one of PDT's branches (an attempt's or a worktree's, in the
//! repository itself) is left out. Every author is logged, with `kind` (`outside`, `bot`, `collaborator`); the trigger
//! starts runs only for outsiders and bots unless its `kinds` say otherwise.
//!
//! The first look at a repository records the ones open (`sensor.github_pulls.<repo>`), so turning a watcher on never
//! replays its history: only pull requests opened after it count as new. Later looks read back to the last update
//! seen; the mark moves only past what was logged, so a failed insert is tried again on the next look.

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, in_scope, parse, parses, ports, schema_of,
};
use pdt_core::forge::ForgeKind;
use pdt_core::outside_pr::{AuthorKind, dependabot_update};
use pdt_core::pr::PrState;
use pdt_core::workflow::api::{LogEvent, NewLogEvent, Scope, Workflow};
use pdt_core::workflow::cond::Condition;
use pdt_core::workflow::doc::Node;
use pdt_core::workflow::events::{
    GITHUB_PR_CLOSED, GITHUB_PR_EVENTS, GITHUB_PR_OPENED, GITHUB_PR_PUSHED, GithubPullRequest, event_type,
};
use pdt_forge::{ChangeStats, Forge, RepoChangeRequest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct PullTriggerConfig {
    /// Whose pull requests start runs: `outside`, `bot`, `collaborator`. Empty: outsiders' and bots'.
    #[ts(type = "Array<\"outside\" | \"bot\" | \"collaborator\">")]
    pub kinds: Vec<AuthorKind>,
    /// Only these projects' repos (a host-wide workflow); empty: every project.
    pub projects: Vec<String>,
    /// Only these repositories (`owner/name`, any case; PDT-269: a host-wide built-in for one repository); empty:
    /// every watched one.
    pub repos: Vec<String>,
    /// Only pull requests for which every condition holds (`{{trigger.updateType}} != major`).
    #[serde(rename = "where")]
    #[ts(rename = "where")]
    pub conditions: Vec<Condition>,
    /// The run's idempotency key; default: the event.
    pub run_key: Option<String>,
}

impl PullTriggerConfig {
    fn wants_repo(&self, repo: &str) -> bool {
        self.repos.is_empty() || self.repos.iter().any(|r| r.trim().eq_ignore_ascii_case(repo))
    }

    fn wants(&self, kind: &str) -> bool {
        if self.kinds.is_empty() {
            return kind == AuthorKind::Outside.as_str() || kind == AuthorKind::Bot.as_str();
        }
        self.kinds.iter().any(|k| k.as_str() == kind)
    }
}

/// The trigger of one of the pull request events.
pub struct PullRequestTrigger(pub &'static str);

impl NodeKind for PullRequestTrigger {
    fn spec(&self) -> KindSpec {
        let t = event_type(self.0).expect("a pull request event type");
        KindSpec {
            id: self.0.into(),
            version: 1,
            category: Category::Trigger,
            title: t.title.into(),
            description: format!(
                "{} Checked every minute on the projects' GitHub repositories; outsiders' and bots' unless `kinds` \
                 says otherwise. Its title and body are outside input.",
                t.description
            ),
            config_schema: schema_of::<PullTriggerConfig>(),
            output_schema: serde_json::to_value((t.schema)()).unwrap_or(Value::Null),
            ports: ports(&[("out", "Then")]),
            dynamic_ports: false,
            preset: None,
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<PullTriggerConfig>(config)
    }

    fn event_type(&self) -> Option<&str> {
        Some(self.0)
    }

    fn accepts(&self, config: &Value, wf: &Workflow, _node: &Node, event: &LogEvent) -> bool {
        let Ok(c) = parse::<PullTriggerConfig>(config) else { return false };
        let kind = event.payload.get("kind").and_then(Value::as_str).unwrap_or_default();
        let repo = event.payload.get("repo").and_then(Value::as_str).unwrap_or_default();
        in_scope(wf, event) && c.wants(kind) && c.wants_repo(repo)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move { Ok(Outcome::Next { port: "out".into(), output: cx.run.trigger.clone() }) })
    }
}

fn state_key(repo: &str) -> String {
    format!("sensor.github_pulls.{}", repo.to_ascii_lowercase())
}

/// What the sensor remembers of a repository.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Seen {
    /// The newest update read (the forge's timestamp): the next look reads back to it.
    since: String,
    /// The newest update at the first look: a pull request created before it is not new.
    baseline: String,
    /// The open pull requests PDT didn't open, by number: their head.
    open: BTreeMap<u32, String>,
}

/// What one pull request in a look comes to.
#[derive(Debug, PartialEq)]
enum News {
    Opened,
    Pushed {
        previous: String,
    },
    Closed,
    /// Remember it, log nothing (one open before the first look, or reopened).
    Track,
    Nothing,
}

/// What `pr` is to a repository last seen as `seen`. Pure.
fn news(seen: &Seen, pr: &RepoChangeRequest) -> News {
    match (pr.state, seen.open.get(&pr.number)) {
        (PrState::Open, Some(head)) if *head != pr.head_sha => News::Pushed { previous: head.clone() },
        (PrState::Open, Some(_)) => News::Nothing,
        (PrState::Open, None) if pr.created_at > seen.baseline => News::Opened,
        (PrState::Open, None) => News::Track,
        (_, Some(_)) => News::Closed,
        (_, None) => News::Nothing,
    }
}

/// A repository a workflow watches and the project it is.
struct Watched {
    repo: String,
    project: String,
    forge: Arc<dyn Forge>,
}

/// The projects' GitHub repos enabled workflows watch with a pull request trigger (by lower-case name).
async fn watched_repos(st: &AppState) -> BTreeMap<String, Watched> {
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    // Host-wide triggers that name their repositories (lower case).
    let mut wanted_repos: BTreeSet<String> = BTreeSet::new();
    let mut every_project = false;
    for wf in st.store.enabled_workflows().await.unwrap_or_default() {
        let Ok(Some(doc)) = st.store.workflow_doc(&wf.id, wf.version).await else { continue };
        for node in doc.nodes.iter().filter(|n| GITHUB_PR_EVENTS.contains(&n.kind.as_str())) {
            let Ok(c) = st.registry().catalog().current_config(node).and_then(|c| parse::<PullTriggerConfig>(&c))
            else {
                continue;
            };
            match (wf.scope, &wf.project_id) {
                (Scope::Project, Some(p)) => {
                    wanted.insert(p.clone());
                }
                (Scope::Host, _) if c.projects.is_empty() && !c.repos.is_empty() => {
                    wanted_repos.extend(c.repos.iter().map(|r| r.trim().to_ascii_lowercase()));
                }
                (Scope::Host, _) if c.projects.is_empty() => every_project = true,
                (Scope::Host, _) => wanted.extend(c.projects.iter().cloned()),
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    for p in st.store.list_projects().await.unwrap_or_default() {
        if !every_project && !wanted.contains(&p.id) && wanted_repos.is_empty() {
            continue;
        }
        if let Some(f) = crate::forge::project_forge(st, &p.id, &p.repo_path).await
            && f.kind == ForgeKind::GITHUB
            && (every_project || wanted.contains(&p.id) || wanted_repos.contains(&f.repo.to_ascii_lowercase()))
        {
            out.entry(f.repo.to_ascii_lowercase()).or_insert(Watched {
                repo: f.repo,
                project: p.id.clone(),
                forge: f.api,
            });
        }
    }
    out
}

/// Whether the pull request is from one of PDT's own branches in the project's repository.
async fn pdts_own(st: &AppState, project: &str, repo: &str, pr: &RepoChangeRequest) -> bool {
    let same_repo = pr.head_repo.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(repo));
    same_repo && st.store.is_pdt_branch(project, &pr.head_branch).await.unwrap_or(false)
}

fn payload(project: &str, repo: &str, pr: &RepoChangeRequest, stats: ChangeStats) -> GithubPullRequest {
    GithubPullRequest {
        project_id: project.into(),
        repo: repo.into(),
        number: u64::from(pr.number),
        title: pr.title.clone(),
        body: pr.body.clone(),
        url: pr.url.clone(),
        author: pr.author.clone(),
        kind: AuthorKind::of(pr.is_bot, pr.collaborator),
        head_repo: pr.head_repo.clone(),
        head_branch: pr.head_branch.clone(),
        head_sha: pr.head_sha.clone(),
        fork: !pr.head_repo.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(repo)),
        base: pr.base_branch.clone(),
        labels: pr.labels.clone(),
        draft: pr.draft,
        changed_files: stats.changed_files,
        additions: stats.additions,
        deletions: stats.deletions,
        update_type: dependabot_update(&pr.author, &pr.title),
        created_at: pr.created_at.clone(),
        previous_head: None,
        merged: pr.state == PrState::Merged,
    }
}

/// One look at every watched repo's pull requests. pdtd runs it every 60 s. Never fails; problems are logged (a
/// failing read once per repo until it works again).
pub async fn refresh_pulls(st: &AppState) {
    for Watched { repo, project, forge } in watched_repos(st).await.into_values() {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let key = state_key(&repo);
        let stored = match st.store.get_host_value(&key).await {
            Ok(v) => v.and_then(|v| serde_json::from_value::<Seen>(v).ok()),
            Err(e) => {
                tracing::warn!(repo, error = %e, "could not read where a repository's pull requests were read to");
                continue;
            }
        };
        let since = stored.as_ref().map(|s| s.since.clone()).filter(|s| !s.is_empty());
        let (gh, r) = (forge.clone(), repo.clone());
        let read = tokio::task::spawn_blocking(move || gh.recent_change_requests(&r, since.as_deref())).await;
        let mut prs = match read {
            Ok(Ok(p)) => {
                crate::github::gh_worked(st, &format!("pulls:{repo}"));
                p
            }
            Ok(Err(pdt_forge::ForgeError::Unsupported(_))) => continue,
            Ok(Err(e)) => {
                crate::github::gh_failed(st, &format!("pulls:{repo}"), &e.to_string());
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "the pull requests task failed");
                continue;
            }
        };
        prs.sort_by(|a, b| a.updated_at.cmp(&b.updated_at).then(a.number.cmp(&b.number)));
        let newest = prs.iter().map(|p| p.updated_at.clone()).max().unwrap_or_default();
        let mut seen = match stored {
            Some(s) => s,
            None => {
                // The first look: what is open now is the baseline.
                let mut s = Seen { since: newest.clone(), baseline: newest, open: BTreeMap::new() };
                for p in prs.iter().filter(|p| p.state == PrState::Open) {
                    if !pdts_own(st, &project, &repo, p).await {
                        s.open.insert(p.number, p.head_sha.clone());
                    }
                }
                save(st, &key, &repo, &s).await;
                continue;
            }
        };
        let before = seen.clone();
        for pr in &prs {
            if pdts_own(st, &project, &repo, pr).await {
                continue;
            }
            let (kind, event_key, previous) = match news(&seen, pr) {
                News::Opened => (GITHUB_PR_OPENED, format!("github:{repo}:pr:{}:opened", pr.number), None),
                News::Pushed { previous } => {
                    (GITHUB_PR_PUSHED, format!("github:{repo}:pr:{}:head:{}", pr.number, pr.head_sha), Some(previous))
                }
                News::Closed => {
                    (GITHUB_PR_CLOSED, format!("github:{repo}:pr:{}:closed:{}", pr.number, pr.updated_at), None)
                }
                News::Track => {
                    seen.open.insert(pr.number, pr.head_sha.clone());
                    seen.since = seen.since.clone().max(pr.updated_at.clone());
                    continue;
                }
                News::Nothing => {
                    seen.since = seen.since.clone().max(pr.updated_at.clone());
                    continue;
                }
            };
            let stats = if kind == GITHUB_PR_CLOSED {
                ChangeStats::default()
            } else {
                let (gh, r, n) = (forge.clone(), repo.clone(), pr.number);
                match tokio::task::spawn_blocking(move || gh.change_request_stats(&r, n)).await {
                    Ok(Ok(s)) => s,
                    Ok(Err(e)) => {
                        tracing::warn!(repo, pr = pr.number, error = %e, "could not read a pull request's size");
                        break;
                    }
                    Err(_) => break,
                }
            };
            let mut p = payload(&project, &repo, pr, stats);
            p.previous_head = previous;
            let e = NewLogEvent::new(kind, event_key, "github_pulls", &p);
            if let Err(x) = st.store.emit_event(&e, now_ms()).await {
                // The mark stays before it: the next look tries it again.
                tracing::warn!(repo, pr = pr.number, error = %x, "could not log a pull request");
                break;
            }
            if pr.state == PrState::Open {
                seen.open.insert(pr.number, pr.head_sha.clone());
            } else {
                seen.open.remove(&pr.number);
            }
            seen.since = seen.since.clone().max(pr.updated_at.clone());
        }
        if seen != before {
            save(st, &key, &repo, &seen).await;
        }
    }
}

async fn save(st: &AppState, key: &str, repo: &str, seen: &Seen) {
    let v = serde_json::to_value(seen).unwrap_or_else(|_| json!({}));
    if let Err(e) = st.store.put_host_value(key, &v).await {
        tracing::warn!(repo, error = %e, "could not record where a repository's pull requests were read to");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: u32, state: PrState, head: &str, created: &str) -> RepoChangeRequest {
        RepoChangeRequest {
            number,
            title: String::new(),
            body: String::new(),
            url: String::new(),
            author: "al".into(),
            is_bot: false,
            collaborator: false,
            head_repo: Some("al/r".into()),
            head_branch: "patch".into(),
            head_sha: head.into(),
            base_branch: "main".into(),
            labels: vec![],
            state,
            draft: false,
            created_at: created.into(),
            updated_at: created.into(),
        }
    }

    #[test]
    fn a_look_tells_new_pushed_and_closed_pull_requests_apart() {
        let seen =
            Seen { since: "2026-10-20".into(), baseline: "2026-10-10".into(), open: BTreeMap::from([(1, "a".into())]) };
        assert_eq!(news(&seen, &pr(1, PrState::Open, "a", "2026-10-01")), News::Nothing);
        assert_eq!(news(&seen, &pr(1, PrState::Open, "b", "2026-10-01")), News::Pushed { previous: "a".into() });
        assert_eq!(news(&seen, &pr(1, PrState::Merged, "b", "2026-10-01")), News::Closed);
        assert_eq!(news(&seen, &pr(2, PrState::Open, "c", "2026-10-21")), News::Opened);
        assert_eq!(news(&seen, &pr(3, PrState::Open, "c", "2026-10-05")), News::Track, "older than the first look");
        assert_eq!(news(&seen, &pr(4, PrState::Closed, "c", "2026-10-21")), News::Nothing);
    }

    #[test]
    fn the_trigger_wants_outsiders_and_bots_unless_told() {
        let c = PullTriggerConfig::default();
        assert!(c.wants("outside") && c.wants("bot") && !c.wants("collaborator"));
        let c: PullTriggerConfig = parse(&json!({"kinds": ["collaborator"]})).unwrap();
        assert!(c.wants("collaborator") && !c.wants("bot"));
        assert!(parse::<PullTriggerConfig>(&json!({"kinds": ["someone"]})).is_err());
    }
}
