//! New GitHub issues. pdtd polls, every 60 s, each repo an enabled workflow's
//! `github.issue.opened` trigger watches (its `repos`, else the GitHub repos of the projects in its scope) and logs
//! `github.issue.opened` for each issue newer than the last one seen. The first look at a repo only records its newest
//! issue (`sensor.github_issues.<repo>`), so turning a watcher on never replays a repo's history. Later looks read
//! back page by page to the last issue seen, and the mark moves only past issues that were logged: a failed insert is
//! tried again on the next look. A project's repo is read through the project's forge, any other through GitHub's
//! account (PDT-241).

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, in_scope, parse, ports, schema_of,
};
use pdt_core::forge::ForgeKind;
use pdt_core::workflow::api::{LogEvent, NewLogEvent, Scope, Workflow};
use pdt_core::workflow::cond::Condition;
use pdt_core::workflow::doc::Node;
use pdt_core::workflow::events::{GITHUB_ISSUE_OPENED, GithubIssueOpened, event_type};
use pdt_forge::Forge;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct IssueTriggerConfig {
    /// The repositories to watch (`owner/name`); empty: the GitHub repos of the projects the workflow applies to.
    pub repos: Vec<String>,
    /// Only these projects' repos (a host-wide workflow without `repos`); empty: every project.
    pub projects: Vec<String>,
    /// Only issues for which every condition holds (`{{trigger.labels}} contains bug`).
    #[serde(rename = "where")]
    #[ts(rename = "where")]
    pub conditions: Vec<Condition>,
    /// The run's idempotency key; default: the issue.
    pub run_key: Option<String>,
}

fn same_repo(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

pub struct IssueOpened;

impl NodeKind for IssueOpened {
    fn spec(&self) -> KindSpec {
        let t = event_type(GITHUB_ISSUE_OPENED).expect("github.issue.opened is an event type");
        KindSpec {
            id: GITHUB_ISSUE_OPENED.into(),
            version: 1,
            category: Category::Trigger,
            title: t.title.into(),
            description: "A new issue in a GitHub repository: the ones in `repos` (any repo PDT's GitHub login can read), else the \
                          projects' own. Checked every minute; the first look at a repository only records where it \
                          is."
            .into(),
            config_schema: schema_of::<IssueTriggerConfig>(),
            output_schema: serde_json::to_value((t.schema)()).unwrap_or(Value::Null),
            ports: ports(&[("out", "Then")]),
            dynamic_ports: false,
            preset: None,
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: IssueTriggerConfig = parse(config)?;
        let named = |r: &&String| !r.contains("{{") && r.split('/').filter(|p| !p.trim().is_empty()).count() != 2;
        if let Some(bad) = c.repos.iter().find(named) {
            return Err(format!("{bad:?} is not owner/name"));
        }
        Ok(())
    }

    fn event_type(&self) -> Option<&str> {
        Some(GITHUB_ISSUE_OPENED)
    }

    fn accepts(&self, config: &Value, wf: &Workflow, _node: &Node, event: &LogEvent) -> bool {
        let repos = parse::<IssueTriggerConfig>(config).map(|c| c.repos).unwrap_or_default();
        if repos.is_empty() {
            return in_scope(wf, event);
        }
        let repo = event.payload.get("repo").and_then(|r| r.as_str()).unwrap_or_default();
        repos.iter().any(|r| same_repo(r, repo))
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move { Ok(Outcome::Next { port: "out".into(), output: cx.run.trigger.clone() }) })
    }
}

fn baseline_key(repo: &str) -> String {
    format!("sensor.github_issues.{}", repo.to_ascii_lowercase())
}

/// A repository a workflow watches: as written, the project it belongs to, and the forge its issues are read from
/// (the project's, else GitHub's account).
struct Watched {
    repo: String,
    project: Option<String>,
    forge: Arc<dyn Forge>,
}

/// The repos enabled workflows watch (by lower-case name).
async fn watched_repos(st: &AppState) -> BTreeMap<String, Watched> {
    let mut project_repos: Vec<(String, String, Arc<dyn Forge>)> = vec![];
    for p in st.store.list_projects().await.unwrap_or_default() {
        // Projects whose forge is GitHub (the trigger's issues are GitHub's).
        if let Some(f) = crate::forge::project_forge(st, &p.id, &p.repo_path).await
            && f.kind == ForgeKind::GITHUB
        {
            project_repos.push((p.id.clone(), f.repo, f.api));
        }
    }
    let account = st.forge_account(&ForgeKind::GITHUB).ok();
    let mut out: BTreeMap<String, Watched> = BTreeMap::new();
    for wf in st.store.enabled_workflows().await.unwrap_or_default() {
        let Ok(Some(doc)) = st.store.workflow_doc(&wf.id, wf.version).await else { continue };
        for node in doc.nodes.iter().filter(|n| n.kind == GITHUB_ISSUE_OPENED) {
            let Ok(c) = st.registry().catalog().current_config(node).and_then(|c| parse::<IssueTriggerConfig>(&c))
            else {
                continue;
            };
            let repos: Vec<String> = if !c.repos.is_empty() {
                c.repos.clone()
            } else {
                project_repos
                    .iter()
                    .filter(|(p, _, _)| match wf.scope {
                        Scope::Project => wf.project_id.as_deref() == Some(p.as_str()),
                        Scope::Host => true,
                        Scope::Personal => false,
                    })
                    .filter(|(p, _, _)| c.projects.is_empty() || c.projects.contains(p))
                    .map(|(_, r, _)| r.clone())
                    .collect()
            };
            for r in repos {
                let key = r.to_ascii_lowercase();
                if out.contains_key(&key) {
                    continue;
                }
                let owner = project_repos.iter().find(|(_, pr, _)| same_repo(pr, &r));
                let (project, forge) = match owner {
                    Some((p, _, f)) => (Some(p.clone()), f.clone()),
                    None => match &account {
                        Some(a) => (None, a.clone()),
                        None => continue,
                    },
                };
                out.insert(key, Watched { repo: r, project, forge });
            }
        }
    }
    out
}

/// One look at every watched repo's newest issues. pdtd runs it every 60 s. Never fails; problems are logged (a
/// failing read once per repo until it works again).
pub async fn refresh_issues(st: &AppState) {
    for Watched { repo, project, forge } in watched_repos(st).await.into_values() {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        if !forge.capabilities().issues {
            continue;
        }
        let key = baseline_key(&repo);
        let seen = match st.store.get_host_value(&key).await {
            Ok(v) => v.and_then(|v| v["newest"].as_u64()),
            Err(e) => {
                tracing::warn!(repo, error = %e, "could not read where a repository's issues were read to");
                continue;
            }
        };
        let (gh, r) = (forge, repo.clone());
        let read = tokio::task::spawn_blocking(move || gh.recent_issues(&r, seen)).await;
        let issues = match read {
            Ok(Ok(i)) => {
                crate::github::gh_worked(st, &format!("issues:{repo}"));
                i
            }
            Ok(Err(e)) => {
                crate::github::gh_failed(st, &format!("issues:{repo}"), &e.to_string());
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "the issues task failed");
                continue;
            }
        };
        let newest = issues.iter().map(|i| i.number).max().unwrap_or(0);
        // The newest issue logged (or, the first time, seen).
        let mut mark = seen.unwrap_or(newest);
        if let Some(seen) = seen {
            let mut new: Vec<_> = issues.iter().filter(|i| i.number > seen).collect();
            new.sort_by_key(|i| i.number);
            for i in new {
                let payload = GithubIssueOpened {
                    project_id: project.clone(),
                    repo: repo.clone(),
                    number: i.number,
                    title: i.title.clone(),
                    body: i.body.clone(),
                    url: i.url.clone(),
                    author: i.author.clone(),
                    labels: i.labels.clone(),
                    created_at: i.created_at.clone(),
                };
                let e = NewLogEvent::new(
                    GITHUB_ISSUE_OPENED,
                    format!("github:{repo}:issue:{}", i.number),
                    "github_issues",
                    payload,
                );
                if let Err(x) = st.store.emit_event(&e, now_ms()).await {
                    // The mark stays before it: the next look tries it again.
                    tracing::warn!(repo, issue = i.number, error = %x, "could not log a new issue");
                    break;
                }
                mark = i.number;
            }
        }
        if seen != Some(mark)
            && let Err(e) = st.store.put_host_value(&key, &json!({"newest": mark})).await
        {
            tracing::warn!(repo, error = %e, "could not record where a repository's issues were read to");
        }
    }
}
