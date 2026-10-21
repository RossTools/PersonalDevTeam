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
    out.push(builtin(super::pipeline::ID, super::pipeline::params(), super::pipeline::doc()));
    out.extend(BUILTINS.iter().map(|b| builtin(b.id, vec![], (b.doc)())));
    out
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
    }
}
