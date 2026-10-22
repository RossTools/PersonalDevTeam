//! Workflow templates (Phase 7d spec §5): the ones PDT ships (`builtin:<id>`, read-only) and the user's own, with
//! parameters. Instantiating one makes an ordinary workflow, disabled until the user enables it.

use super::builtins::BUILTINS;
use super::routes::{body, checked, conflict, create_workflow, invalid};
use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::workflow::api::{Scope, WorkflowDetail};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::template::{Template, TemplateParam, check_params, fill, param_values, placeholder_values};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ts_rs::TS;

pub const BUILTIN_PREFIX: &str = "builtin:";

fn doc(v: Value) -> WorkflowDoc {
    serde_json::from_value(v).expect("a built-in template is a valid document")
}

fn params(v: Value) -> Vec<TemplateParam> {
    serde_json::from_value(v).expect("built-in template params")
}

fn builtin(id: &str, params: Vec<TemplateParam>, doc: WorkflowDoc) -> Template {
    Template {
        id: format!("{BUILTIN_PREFIX}{id}"),
        name: doc.name.clone(),
        description: doc.description.clone(),
        builtin: true,
        params,
        doc,
        created_at: 0,
        updated_at: 0,
    }
}

/// The templates PDT ships: a blank one, a few starters, and the built-in workflows (to copy, e.g. per project).
pub fn builtin_templates() -> Vec<Template> {
    let mut out = vec![
        Template {
            name: "Blank".into(),
            description: "A Run now trigger and an end: build the rest yourself.".into(),
            ..builtin(
                "blank",
                vec![],
                doc(json!({
                    "name": "New workflow",
                    "description": "",
                    "nodes": [
                        {"id": "start", "kind": "manual", "version": 1, "position": {"x": 0, "y": 0}},
                        {"id": "end", "kind": "end", "version": 1, "position": {"x": 300, "y": 0}},
                    ],
                    "edges": [{"from": "start", "to": "end"}],
                })),
            )
        },
        builtin(
            "ci-failed-tell",
            params(json!([{"id": "message", "title": "Message", "type": "text",
                "description": "What the agent is told. {{trigger.notice.text}} is PDT's own notice (the failed jobs and \
                                how to see their logs).",
                "default": "{{trigger.notice.text}}"}])),
            doc(json!({
                "name": "Tell the agent when CI fails",
                "description": "When CI fails on a ticket's pull request, tells the attempt's agent once it is idle.",
                "nodes": [
                    {"id": "failed", "kind": "ci.run_failed", "version": 1, "position": {"x": 0, "y": 0}},
                    {"id": "tell", "kind": "session.send", "version": 1, "position": {"x": 320, "y": 0}, "config": {
                        "attempt": "{{trigger.attemptId}}", "text": "{{params.message}}",
                        "dedupe": "{{trigger.notice.key}}"}},
                ],
                "edges": [{"from": "failed", "to": "tell"}],
            })),
        ),
        builtin(
            "ask-finish",
            params(
                json!([{"id": "deleteBranch", "title": "Delete the branch too", "type": "boolean", "default": false}]),
            ),
            doc(json!({
                "name": "Ask before finishing a merged ticket",
                "description": "When a ticket's branch is found merged, asks whether to move it to Done and clean up.",
                "nodes": [
                    {"id": "merged", "kind": "attempt.merged", "version": 1, "position": {"x": 0, "y": 0},
                     "config": {"where": [{"left": "{{trigger.via}}", "op": "eq", "right": "detected"}]}},
                    {"id": "ask", "kind": "gate", "version": 1, "position": {"x": 300, "y": 0}, "config": {
                        "question": "{{trigger.ticketKey}} merged. Move it to Done and remove its worktree?",
                        "choices": ["finish", "keep"]}},
                    {"id": "finish", "kind": "ticket.finish", "version": 1, "position": {"x": 600, "y": 0}, "config": {
                        "ticket": "{{trigger.ticketId}}", "stopRunning": true, "deleteBranch": "{{params.deleteBranch}}"}},
                    {"id": "done", "kind": "end", "version": 1, "position": {"x": 600, "y": 160}, "title": "Leave it",
                     "config": {"quiet": true}},
                ],
                "edges": [
                    {"from": "merged", "to": "ask"},
                    {"from": "ask", "port": "finish", "to": "finish"},
                    {"from": "ask", "port": "keep", "to": "done"},
                    {"from": "ask", "port": "unanswered", "to": "done"},
                    {"from": "ask", "port": "timeout", "to": "done"},
                    {"from": "finish", "port": "blocked", "to": "done"},
                ],
            })),
        ),
        builtin(
            "comment-on-merge",
            params(json!([{"id": "body", "title": "Comment", "type": "text", "default": "Merged."}])),
            doc(json!({
                "name": "Comment when a pull request merges",
                "description": "Adds a comment to the ticket when its attempt merges.",
                "nodes": [
                    {"id": "merged", "kind": "attempt.merged", "version": 1, "position": {"x": 0, "y": 0}},
                    {"id": "comment", "kind": "ticket.comment", "version": 1, "position": {"x": 320, "y": 0},
                     "config": {"ticket": "{{trigger.ticketId}}", "body": "{{params.body}}"}},
                ],
                "edges": [{"from": "merged", "to": "comment"}],
            })),
        ),
    ];
    out.extend(integration_templates());
    out.extend(BUILTINS.iter().map(|b| builtin(b.id, vec![], (b.doc)())));
    out
}

/// The param of a Discord webhook connection.
fn discord_param() -> Value {
    json!({"id": "discord", "title": "Discord channel", "type": "connection", "connectionKind": "discord_webhook",
        "required": true, "description": "Where it posts: a Discord webhook connection (Settings → This host → Connections)."})
}

/// A schedule's `every`.
fn every_param(default: &str) -> Value {
    json!({"id": "every", "title": "How often", "type": "string", "default": default,
        "description": "An interval: 1m, 15m, 1h, 1d (from midnight UTC). For set times, change the schedule to a \
                        cron expression after creating it."})
}

/// Templates for the Hermes jobs that need no agent (Phase 7e spec §5).
fn integration_templates() -> Vec<Template> {
    let mut out = vec![
        builtin(
            "github-issues-discord",
            params(json!([
                {"id": "repo", "title": "Repository", "type": "string", "required": true,
                 "description": "owner/name on GitHub (read through gh)."},
                discord_param(),
            ])),
            doc(json!({
                "name": "New GitHub issues to Discord",
                "description": "Posts each new issue of a GitHub repository to a Discord channel. The first look at \
                                the repository only records where it is.",
                "nodes": [
                    {"id": "opened", "kind": "github.issue.opened", "version": 1, "position": {"x": 0, "y": 0},
                     "config": {"repos": ["{{params.repo}}"]}},
                    {"id": "post", "kind": "discord.post", "version": 1, "position": {"x": 320, "y": 0},
                     "retry": {"max": 3, "backoffMs": 30000}, "config": {"connection": "{{params.discord}}",
                        "content": "New issue in {{trigger.repo}} #{{trigger.number}}: **{{trigger.title}}** by \
                                    {{trigger.author}}\n{{trigger.url}}"}},
                ],
                "edges": [{"from": "opened", "to": "post"}],
            })),
        ),
        builtin(
            "health-watchdog",
            params(json!([
                {"id": "name", "title": "Service name", "type": "string", "default": "The service"},
                {"id": "url", "title": "Health URL", "type": "string", "required": true,
                 "description": "Healthy when it answers with a status below 400."},
                every_param("1m"),
                discord_param(),
            ])),
            doc(json!({
                "name": "Health watchdog",
                "description": "Checks a URL on a schedule and posts to Discord only when it goes down or comes back. \
                                The first check only records the state.",
                "nodes": [
                    {"id": "tick", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 160},
                     "config": {"every": "{{params.every}}", "catchUp": false}},
                    {"id": "check", "kind": "http.request", "version": 1, "position": {"x": 260, "y": 160},
                     "timeoutMs": 30000, "config": {"url": "{{params.url}}", "expect": "any"}},
                    {"id": "healthy", "kind": "condition", "version": 1, "position": {"x": 520, "y": 160},
                     "title": "Healthy?", "config": {"conditions": [
                        {"left": "{{steps.check.status}}", "op": "lt", "right": 400}]}},
                    {"id": "was_down", "kind": "condition", "version": 1, "position": {"x": 780, "y": 0},
                     "title": "Was it down?", "config": {"conditions": [
                        {"left": "{{state.healthy}}", "op": "eq", "right": false}]}},
                    {"id": "was_up", "kind": "condition", "version": 1, "position": {"x": 780, "y": 320},
                     "title": "Was it up?", "config": {"conditions": [
                        {"left": "{{state.healthy}}", "op": "eq", "right": true}]}},
                    {"id": "set_up", "kind": "state.set", "version": 1, "position": {"x": 1040, "y": 0},
                     "config": {"key": "healthy", "value": true}},
                    {"id": "set_down", "kind": "state.set", "version": 1, "position": {"x": 1040, "y": 320},
                     "config": {"key": "healthy", "value": false}},
                    {"id": "recovered", "kind": "discord.post", "version": 1, "position": {"x": 1300, "y": 0},
                     "retry": {"max": 3, "backoffMs": 30000}, "config": {"connection": "{{params.discord}}",
                        "content": "✅ {{params.name}} recovered."}},
                    {"id": "down", "kind": "discord.post", "version": 1, "position": {"x": 1300, "y": 320},
                     "retry": {"max": 3, "backoffMs": 30000}, "config": {"connection": "{{params.discord}}",
                        "content": "🔴 {{params.name}} is down: {{steps.check.status}}{{steps.check.error}} \
                                    ({{params.url}})"}},
                    {"id": "same_up", "kind": "state.set", "version": 1, "position": {"x": 1040, "y": 160},
                     "title": "Still up (or first check)", "config": {"key": "healthy", "value": true}},
                    {"id": "same_down", "kind": "state.set", "version": 1, "position": {"x": 1040, "y": 480},
                     "title": "Still down (or first check)", "config": {"key": "healthy", "value": false}},
                    {"id": "quiet_up", "kind": "end", "version": 1, "position": {"x": 1300, "y": 160},
                     "config": {"quiet": true}},
                    {"id": "quiet_down", "kind": "end", "version": 1, "position": {"x": 1300, "y": 480},
                     "config": {"quiet": true}},
                ],
                "edges": [
                    {"from": "tick", "to": "check"},
                    {"from": "check", "to": "healthy"},
                    {"from": "check", "port": "error", "to": "was_up"},
                    {"from": "healthy", "port": "true", "to": "was_down"},
                    {"from": "healthy", "port": "false", "to": "was_up"},
                    {"from": "was_down", "port": "true", "to": "set_up"},
                    {"from": "was_down", "port": "false", "to": "same_up"},
                    {"from": "was_up", "port": "true", "to": "set_down"},
                    {"from": "was_up", "port": "false", "to": "same_down"},
                    {"from": "set_up", "to": "recovered"},
                    {"from": "set_down", "to": "down"},
                    {"from": "same_up", "to": "quiet_up"},
                    {"from": "same_down", "to": "quiet_down"},
                ],
            })),
        ),
    ];
    for (id, name, what, every) in [
        (
            "brief-follow-ups",
            "Brief follow-ups",
            "Runs a brief's follow-up script (say, matching its items against a watchlist) once a day",
            "1d",
        ),
        ("email-processing", "Email processing", "Runs an email-processing script every 15 minutes", "15m"),
    ] {
        out.push(scheduled_script(id, name, what, every));
    }
    out
}

/// A script on a schedule that posts to Discord only when it has news: `{"message": "…"}` on stdout; `{}` or
/// `{"quiet": true}` when not.
fn scheduled_script(id: &str, name: &str, what: &str, every: &str) -> Template {
    builtin(
        id,
        params(json!([
            {"id": "program", "title": "Program", "type": "string", "default": "python3",
             "description": "What runs the script: python3, node, bash, or the script itself."},
            {"id": "script", "title": "Script", "type": "string", "required": true,
             "description": "The script's full path, given to the program. It prints {\"message\": \"…\"} when there \
                             is news and {} when not."},
            {"id": "cwd", "title": "Run in", "type": "string", "default": "scratch",
             "description": "scratch (a fresh folder), or a full path."},
            every_param(every),
            discord_param(),
        ])),
        doc(json!({
            "name": name,
            "description": format!("{what} and posts its message to Discord when it has one. The script prints \
                                    {{\"message\": \"…\"}} when there is news, {{}} when not."),
            "nodes": [
                {"id": "tick", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 0},
                 "config": {"every": "{{params.every}}"}},
                {"id": "run", "kind": "command", "version": 1, "position": {"x": 260, "y": 0}, "timeoutMs": 600000,
                 "config": {"program": "{{params.program}}", "args": ["{{params.script}}"], "cwd": "{{params.cwd}}",
                    "input": {"slot": "{{trigger.slotIso}}", "date": "{{trigger.date}}"}}},
                {"id": "news", "kind": "condition", "version": 1, "position": {"x": 520, "y": 0}, "title": "Any news?",
                 "config": {"conditions": [{"left": "{{steps.run.json.message}}", "op": "truthy"}]}},
                {"id": "post", "kind": "discord.post", "version": 1, "position": {"x": 780, "y": 0},
                 "retry": {"max": 3, "backoffMs": 30000},
                 "config": {"connection": "{{params.discord}}", "content": "{{steps.run.json.message}}"}},
                {"id": "nothing", "kind": "end", "version": 1, "position": {"x": 780, "y": 160},
                 "config": {"quiet": true}},
            ],
            "edges": [
                {"from": "tick", "to": "run"},
                {"from": "run", "to": "news"},
                {"from": "news", "port": "true", "to": "post"},
                {"from": "news", "port": "false", "to": "nothing"},
            ],
        })),
    )
}

async fn find(st: &AppState, id: &str) -> Result<Template, ApiError> {
    if id.starts_with(BUILTIN_PREFIX) {
        return builtin_templates().into_iter().find(|t| t.id == id).ok_or_else(|| ApiError::not_found("template"));
    }
    st.store.get_template(id).await?.ok_or_else(|| ApiError::not_found("template"))
}

pub(crate) async fn list(State(st): State<AppState>) -> Result<Json<Vec<Template>>, ApiError> {
    let mut out = builtin_templates();
    out.extend(st.store.list_templates().await?);
    Ok(Json(out))
}

pub(crate) async fn get_one(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Template>, ApiError> {
    Ok(Json(find(&st, &id).await?))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TemplateInput {
    pub name: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub params: Vec<TemplateParam>,
    pub doc: Option<WorkflowDoc>,
}

/// A template's name, params and document, checked: the params make sense and the document validates with their
/// defaults (or placeholders) filled in.
fn checked_input(t: TemplateInput) -> Result<(String, String, Vec<TemplateParam>, WorkflowDoc), ApiError> {
    let name = t.name.as_deref().map(str::trim).unwrap_or_default().to_string();
    if name.is_empty() || name.len() > 120 {
        return Err(invalid("a template needs a name of at most 120 characters"));
    }
    let doc = t.doc.ok_or_else(|| invalid("doc is required"))?;
    check_params(&t.params).map_err(invalid)?;
    let filled = fill(&doc, &placeholder_values(&t.params)).map_err(invalid)?;
    checked(&filled)?;
    Ok((name, t.description.trim().to_string(), t.params, doc))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<TemplateInput>, JsonRejection>,
) -> Result<(StatusCode, Json<Template>), ApiError> {
    let (name, description, params, doc) = checked_input(body(payload)?)?;
    let now = now_ms();
    let t = Template {
        id: uuid::Uuid::now_v7().to_string(),
        name,
        description,
        builtin: false,
        params,
        doc,
        created_at: now,
        updated_at: now,
    };
    st.store.insert_template(&t).await?;
    Ok((StatusCode::CREATED, Json(t)))
}

fn read_only(t: &Template) -> ApiError {
    conflict("builtin_template", format!("{} ships with PDT: save a copy instead", t.name))
}

pub(crate) async fn save(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TemplateInput>, JsonRejection>,
) -> Result<Json<Template>, ApiError> {
    let mut t = find(&st, &id).await?;
    if t.builtin {
        return Err(read_only(&t));
    }
    let (name, description, params, doc) = checked_input(body(payload)?)?;
    (t.name, t.description, t.params, t.doc, t.updated_at) = (name, description, params, doc, now_ms());
    st.store.update_template(&t).await?;
    Ok(Json(t))
}

pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let t = find(&st, &id).await?;
    if t.builtin {
        return Err(read_only(&t));
    }
    st.store.delete_template(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Instantiate {
    /// By param id; missing ones take their defaults.
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub params: Map<String, Value>,
    /// The new workflow's name (default: the template's document's).
    pub name: Option<String>,
    pub scope: Option<Scope>,
    pub project_id: Option<String>,
}

/// `POST /api/workflow-templates/{id}/instantiate`: a new workflow from the template, disabled.
pub(crate) async fn instantiate(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<Instantiate>, JsonRejection>,
) -> Result<(StatusCode, Json<WorkflowDetail>), ApiError> {
    let i = body(payload)?;
    let t = find(&st, &id).await?;
    let values = param_values(&t.params, &i.params).map_err(invalid)?;
    let mut doc = fill(&t.doc, &values).map_err(invalid)?;
    if let Some(n) = i.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        doc.name = n.to_string();
    }
    let note = format!("from the template {}", t.name);
    let w =
        create_workflow(&st, &doc, i.scope, i.project_id, false, &note, pdt_core::workflow::api::Author::User).await?;
    Ok((StatusCode::CREATED, Json(w)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::validate::{Severity, validate};

    #[test]
    fn every_builtin_template_is_valid_with_its_defaults() {
        let all = builtin_templates();
        let mut ids = std::collections::HashSet::new();
        for t in &all {
            assert!(ids.insert(t.id.clone()), "{} twice", t.id);
            check_params(&t.params).unwrap();
            let d = fill(&t.doc, &placeholder_values(&t.params)).unwrap();
            let issues = validate(&d, super::super::catalog::catalog());
            let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
            assert!(errors.is_empty(), "{}: {errors:?}", t.id);
        }
        assert!(all.iter().any(|t| t.id == "builtin:pr-notices"));
        for id in ["github-issues-discord", "health-watchdog", "brief-follow-ups", "email-processing"] {
            assert!(all.iter().any(|t| t.id == format!("builtin:{id}")), "{id}");
        }
    }
}
