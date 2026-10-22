//! New GitHub issues (Phase 7e spec §2.3). pdtd polls, every 60 s, each repo an enabled workflow's
//! `github.issue.opened` trigger watches (its `repos`, else the GitHub repos of the projects in its scope) and logs
//! `github.issue.opened` for each issue newer than the last one seen. The first look at a repo only records its newest
//! issue (`sensor.github_issues.<repo>`), so turning a watcher on never replays a repo's history.

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, catalog, in_scope, parse, ports, schema_of,
};
use pdt_core::workflow::api::{LogEvent, NewLogEvent, Scope, Workflow};
use pdt_core::workflow::cond::Condition;
use pdt_core::workflow::doc::Node;
use pdt_core::workflow::events::{GITHUB_ISSUE_OPENED, GithubIssueOpened, event_type};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
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
            description: "A new issue in a GitHub repository: the ones in `repos` (any repo, through gh), else the \
                          projects' own. Checked every minute; the first look at a repository only records where it \
                          is."
            .into(),
            config_schema: schema_of::<IssueTriggerConfig>(),
            output_schema: serde_json::to_value((t.schema)()).unwrap_or(Value::Null),
            ports: ports(&[("out", "Then")]),
            dynamic_ports: false,
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

/// The repos enabled workflows watch (by lower-case name), each as written and with the project it belongs to.
async fn watched_repos(st: &AppState) -> BTreeMap<String, (String, Option<String>)> {
    let mut project_repos: Vec<(String, String)> = vec![];
    for p in st.store.list_projects().await.unwrap_or_default() {
        if let Some(r) = crate::github::github_repo(&p.repo_path).await {
            project_repos.push((p.id.clone(), r));
        }
    }
    let project_of = |repo: &str| project_repos.iter().find(|(_, r)| same_repo(r, repo)).map(|(p, _)| p.clone());
    let mut out: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    for wf in st.store.enabled_workflows().await.unwrap_or_default() {
        let Ok(Some(doc)) = st.store.workflow_doc(&wf.id, wf.version).await else { continue };
        for node in doc.nodes.iter().filter(|n| n.kind == GITHUB_ISSUE_OPENED) {
            let Ok(c) = catalog().current_config(node).and_then(|c| parse::<IssueTriggerConfig>(&c)) else { continue };
            let repos: Vec<String> = if !c.repos.is_empty() {
                c.repos.clone()
            } else {
                project_repos
                    .iter()
                    .filter(|(p, _)| match wf.scope {
                        Scope::Project => wf.project_id.as_deref() == Some(p.as_str()),
                        Scope::Host => true,
                        Scope::Personal => false,
                    })
                    .filter(|(p, _)| c.projects.is_empty() || c.projects.contains(p))
                    .map(|(_, r)| r.clone())
                    .collect()
            };
            for r in repos {
                let key = r.to_ascii_lowercase();
                let project = project_of(&r);
                out.entry(key).or_insert((r, project));
            }
        }
    }
    out
}

/// One look at every watched repo's newest issues. pdtd runs it every 60 s. Never fails; problems are logged.
pub async fn refresh_issues(st: &AppState) {
    for (repo, project) in watched_repos(st).await.into_values() {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let (gh, r) = (st.github.clone(), repo.clone());
        let issues = match tokio::task::spawn_blocking(move || gh.recent_issues(&r)).await {
            Ok(Ok(i)) => i,
            Ok(Err(e)) => {
                tracing::debug!(repo, error = %e, "could not read a repository's issues");
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "the issues task failed");
                continue;
            }
        };
        let key = baseline_key(&repo);
        let newest = issues.iter().map(|i| i.number).max().unwrap_or(0);
        let seen = st.store.get_host_value(&key).await.ok().flatten().and_then(|v| v["newest"].as_u64());
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
                    tracing::warn!(repo, error = %x, "could not log a new issue");
                }
            }
        }
        if seen.is_none_or(|s| newest > s) {
            let _ = st.store.put_host_value(&key, &json!({"newest": newest.max(seen.unwrap_or(0))})).await;
        }
    }
}
