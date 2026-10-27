//! Jira actions (Phase 7e spec §6.3): `jira.import` (an issue as a linked ticket), `jira.create`, `jira.transition`
//! and `jira.comment`, through a `jira` connection (pdt-api `jira.rs`). The triggers are the plain `jira.issue.*`
//! event triggers.

use crate::jira::{Import, JiraClient, import_issue};
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::jira::{DEFAULT_DONE, JIRA, issue_key, text_to_adf};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ts_rs::TS;

fn key_ok(key: &str) -> Result<(), String> {
    if key.contains("{{") || issue_key(key).is_some() { Ok(()) } else { Err(format!("{key:?} is not an issue key")) }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraImportConfig {
    /// The issue (`ABC-123`, `{{trigger.key}}`).
    pub key: String,
    /// The PDT project the ticket goes in (`{{trigger.projectId}}`).
    pub project: String,
    /// A `jira` connection; default the project's.
    #[serde(default)]
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraImportOutput {
    pub ticket_id: String,
    pub key: String,
}

pub struct JiraImport;

impl NodeKind for JiraImport {
    fn spec(&self) -> KindSpec {
        spec::<JiraImportConfig, JiraImportOutput>(
            "jira.import",
            1,
            Category::Action,
            "Import a Jira issue",
            "Makes a Draft ticket from a Jira issue (its summary and description), linked to it. When a ticket already \
             links the issue nothing is made and the step leaves by `exists`: an issue is never imported twice.",
            &[("imported", "Imported"), ("exists", "Already a ticket")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: JiraImportConfig = parse(config)?;
        if c.project.trim().is_empty() {
            return Err("project is empty: name the PDT project ({{trigger.projectId}})".into());
        }
        key_ok(&c.key)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: JiraImportConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next("imported", json!({"would": format!("import {} as a ticket", c.key)})));
            }
            let r = import_issue(cx.st, &c.project, &c.key, c.connection.as_deref()).await.map_err(|e| e.message())?;
            let key = issue_key(&c.key).unwrap_or(c.key);
            Ok(match r {
                Import::Imported(ticket_id) => Outcome::next("imported", JiraImportOutput { ticket_id, key }),
                Import::Exists(ticket_id) => {
                    let mut out = serde_json::to_value(JiraImportOutput { ticket_id, key }).unwrap_or_default();
                    out["quiet"] = json!(true);
                    Outcome::Next { port: "exists".into(), output: out }
                }
            })
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraCreateConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The Jira project's key (`ABC`).
    pub project: String,
    /// Default `Task`.
    #[serde(default = "task")]
    pub issue_type: String,
    pub summary: String,
    /// Text (sent as Jira rich text).
    #[serde(default)]
    pub description: String,
    /// More fields, as Jira's REST API names them (`{"labels": ["pdt"]}`).
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub fields: Map<String, Value>,
}

fn task() -> String {
    "Task".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueOutput {
    pub key: String,
    pub url: String,
}

pub struct JiraCreate;

impl NodeKind for JiraCreate {
    fn spec(&self) -> KindSpec {
        spec::<JiraCreateConfig, JiraIssueOutput>(
            "jira.create",
            1,
            Category::Action,
            "Create a Jira issue",
            "Creates an issue in a Jira project; its key is `{{steps.<id>.key}}`.",
            &[("out", "Then")],
            false,
        )
    }

    /// What it does can't be told apart from doing it twice (PDT-131): cut short, it isn't run again on its own.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: JiraCreateConfig = parse(config)?;
        for (field, v) in [("connection", &c.connection), ("project", &c.project), ("summary", &c.summary)] {
            if v.trim().is_empty() {
                return Err(format!("{field} is empty"));
            }
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: JiraCreateConfig = parse(&input)?;
            if cx.dry_run {
                let what = format!("create a {} in {} via {}: {}", c.issue_type, c.project, c.connection, c.summary);
                return Ok(Outcome::next("out", json!({"would": what})));
            }
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let mut fields = c.fields.clone();
            fields.insert("project".into(), json!({"key": c.project.trim()}));
            fields.insert("issuetype".into(), json!({"name": c.issue_type}));
            fields.insert("summary".into(), json!(c.summary));
            if !c.description.trim().is_empty() {
                fields.insert("description".into(), text_to_adf(&c.description));
            }
            let key = client.create(Value::Object(fields)).await?;
            let url = format!("{}/browse/{key}", client.site());
            Ok(Outcome::next("out", JiraIssueOutput { key, url }))
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraTransitionConfig {
    /// A `jira` connection (with `key`).
    #[serde(default)]
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: Option<String>,
    /// The issue (`ABC-123`).
    #[serde(default)]
    pub key: Option<String>,
    /// Or a ticket (`{{trigger.ticketId}}`): its Jira issue, through the connection it was linked with.
    #[serde(default)]
    pub ticket: Option<String>,
    /// A transition's name or the status it leads to; with `ticket`, default its project's done mapping.
    #[serde(default)]
    pub to: Option<String>,
}

pub struct JiraTransition;

fn nonblank(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

impl NodeKind for JiraTransition {
    fn spec(&self) -> KindSpec {
        spec::<JiraTransitionConfig, JiraIssueOutput>(
            "jira.transition",
            1,
            Category::Action,
            "Move a Jira issue",
            "Moves a Jira issue (by key, or a ticket's linked issue) through the transition named `to`, or the one \
             leading to that status. An issue already there counts as done; no such transition from its status leaves \
             by `unavailable`.",
            &[("out", "Then"), ("unavailable", "Can't move there")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: JiraTransitionConfig = parse(config)?;
        match (nonblank(&c.key), nonblank(&c.ticket)) {
            (Some(k), None) => {
                if nonblank(&c.connection).is_none() {
                    return Err("with key, name the jira connection".into());
                }
                if nonblank(&c.to).is_none() {
                    return Err("to is empty: name a transition or status".into());
                }
                key_ok(k)
            }
            (None, Some(_)) => Ok(()),
            _ => Err("give either key (with connection) or ticket".into()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: JiraTransitionConfig = parse(&input)?;
            let (connection, key, to) = match nonblank(&c.ticket) {
                Some(ticket) => {
                    let links = cx.st.store.external_links(ticket, JIRA).await.map_err(|e| e.to_string())?;
                    let Some(link) = links.into_iter().next() else {
                        let out = json!({"quiet": true, "reason": "the ticket is not linked to a Jira issue"});
                        return Ok(Outcome::Next { port: "unavailable".into(), output: out });
                    };
                    let conn = cx
                        .st
                        .store
                        .get_connection(&link.connection_id)
                        .await
                        .map_err(|e| e.to_string())?
                        .ok_or("the Jira connection the ticket was linked with is gone")?;
                    let to = match nonblank(&c.to) {
                        Some(t) => t.to_string(),
                        None => {
                            let summary = cx.st.store.get_ticket_summary(ticket).await.map_err(|e| e.to_string())?;
                            let project = summary.map(|t| t.project_id).unwrap_or_default();
                            let s = cx.st.store.get_project_settings(&project).await.map_err(|e| e.to_string())?;
                            s.jira().map(|j| j.done).unwrap_or_else(|| DEFAULT_DONE.into())
                        }
                    };
                    (conn.name, link.key, to)
                }
                None => (
                    nonblank(&c.connection).unwrap_or_default().to_string(),
                    issue_key(nonblank(&c.key).unwrap_or_default()).ok_or("key is not an issue key")?,
                    nonblank(&c.to).unwrap_or_default().to_string(),
                ),
            };
            if cx.dry_run {
                return Ok(Outcome::next("out", json!({"would": format!("move {key} to {to} via {connection}")})));
            }
            let client = JiraClient::named(cx.st, &connection).await?;
            let url = format!("{}/browse/{key}", client.site());
            let issue = client.issue(&key).await?.ok_or_else(|| format!("Jira has no issue {key}"))?;
            if issue.status.eq_ignore_ascii_case(&to) {
                return Ok(Outcome::next(
                    "out",
                    json!({"key": key, "url": url, "status": issue.status, "quiet": true}),
                ));
            }
            let list = client.transitions(&key).await?;
            let pick = list
                .iter()
                .find(|(_, name, _)| name.eq_ignore_ascii_case(&to))
                .or_else(|| list.iter().find(|(_, _, status)| status.eq_ignore_ascii_case(&to)));
            let Some((id, name, status)) = pick else {
                let available: Vec<&str> = list.iter().map(|(_, n, _)| n.as_str()).collect();
                let out = json!({"key": key, "url": url, "status": issue.status, "to": to, "available": available,
                    "quiet": true});
                return Ok(Outcome::Next { port: "unavailable".into(), output: out });
            };
            client.do_transition(&key, id).await?;
            Ok(Outcome::next("out", json!({"key": key, "url": url, "transition": name, "status": status})))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraCommentConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    pub key: String,
    /// Text (sent as Jira rich text: a paragraph per blank-line-separated block).
    pub body: String,
}

pub struct JiraComment;

impl NodeKind for JiraComment {
    fn spec(&self) -> KindSpec {
        spec::<JiraCommentConfig, Value>(
            "jira.comment",
            1,
            Category::Action,
            "Comment on a Jira issue",
            "Adds a comment to a Jira issue; its id is `{{steps.<id>.commentId}}`.",
            &[("out", "Then")],
            false,
        )
    }

    /// What it does can't be told apart from doing it twice (PDT-131): cut short, it isn't run again on its own.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: JiraCommentConfig = parse(config)?;
        if c.connection.trim().is_empty() {
            return Err("connection is empty: name a jira connection".into());
        }
        if c.body.trim().is_empty() {
            return Err("body is empty".into());
        }
        key_ok(&c.key)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: JiraCommentConfig = parse(&input)?;
            let key = issue_key(&c.key).ok_or("key is not an issue key")?;
            if cx.dry_run {
                return Ok(Outcome::next("out", json!({"would": format!("comment on {key}: {}", c.body)})));
            }
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let id = client.comment(&key, &c.body).await?;
            Ok(Outcome::next("out", json!({"key": key, "commentId": id})))
        })
    }
}
