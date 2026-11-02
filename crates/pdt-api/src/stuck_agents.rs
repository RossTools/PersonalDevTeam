//! Settings → Agents → When an agent is stuck (PDT-367): the built-in workflow's inputs, read and written as a new
//! version of it. A workflow the user changed beyond its inputs is only shown (with Reset to default).

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use crate::workflow::builtins::{STUCK_AGENTS, stuck_agents as stuck};
use crate::workflow::routes::{body, checked, conflict, invalid};
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::routing::{get, post};
use axum::{Json, Router};
use pdt_core::workflow::api::{Author, Workflow};
use pdt_core::workflow::template::TemplateParam;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

pub fn router() -> Router<AppState> {
    Router::new().route("/stuck-agents", get(view).put(save)).route("/stuck-agents/reset", post(reset))
}

/// The workflow as Settings shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StuckAgentsView {
    pub workflow_id: String,
    pub name: String,
    pub enabled: bool,
    /// Changed beyond its inputs in the workflow editor: Settings can't change it.
    pub customized: bool,
    pub params: Vec<TemplateParam>,
    /// Its inputs now, by param id.
    #[ts(type = "Record<string, unknown>")]
    pub values: Map<String, Value>,
    #[ts(type = "Record<string, unknown>")]
    pub defaults: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StuckAgentsInput {
    /// The inputs to change, by param id; the others stay as they are.
    #[ts(type = "Record<string, unknown>")]
    pub values: Map<String, Value>,
}

/// The built-in, installed if this pdtd hasn't yet.
async fn workflow(st: &AppState) -> Result<Workflow, ApiError> {
    {
        let _engine = st.engine_lock.lock().await;
        crate::workflow::builtins::ensure_installed(st).await;
    }
    st.store.workflow_by_builtin(STUCK_AGENTS).await?.ok_or_else(|| ApiError::not_found("workflow"))
}

async fn current(st: &AppState, w: &Workflow) -> Result<pdt_core::workflow::doc::WorkflowDoc, ApiError> {
    st.store.workflow_doc(&w.id, w.version).await?.ok_or_else(|| ApiError::not_found("workflow version"))
}

async fn view_of(st: &AppState) -> Result<StuckAgentsView, ApiError> {
    let w = workflow(st).await?;
    let doc = current(st, &w).await?;
    Ok(StuckAgentsView {
        workflow_id: w.id,
        name: w.name,
        enabled: w.enabled,
        customized: stuck::customized(&doc),
        params: stuck::stuck_params(),
        values: stuck::values_of(&doc),
        defaults: stuck::defaults(),
    })
}

/// `GET /api/stuck-agents`.
async fn view(State(st): State<AppState>) -> Result<Json<StuckAgentsView>, ApiError> {
    Ok(Json(view_of(&st).await?))
}

/// `PUT /api/stuck-agents`: changed inputs, as a new version of the workflow.
async fn save(
    State(st): State<AppState>,
    payload: Result<Json<StuckAgentsInput>, JsonRejection>,
) -> Result<Json<StuckAgentsView>, ApiError> {
    let input = body(payload)?;
    let w = workflow(&st).await?;
    let doc = current(&st, &w).await?;
    if stuck::customized(&doc) {
        return Err(conflict(
            "customized",
            format!("{} was changed in the workflow editor: change it there, or reset it to the default", w.name),
        ));
    }
    let mut values = stuck::values_of(&doc);
    for (k, v) in input.values {
        if !values.contains_key(&k) {
            return Err(invalid(format!("there is no input {k}")));
        }
        values.insert(k, v);
    }
    let mut next = stuck::filled(&values).map_err(invalid)?;
    (next.name, next.description) = (doc.name.clone(), doc.description.clone());
    checked(&st, &next)?;
    if next != doc {
        st.store.save_workflow_version(&w.id, &next, Author::User, stuck::SETTINGS_NOTE, now_ms()).await?;
    }
    Ok(Json(view_of(&st).await?))
}

/// `POST /api/stuck-agents/reset`: the workflow as PDT ships it, following PDT's newer versions again.
async fn reset(State(st): State<AppState>) -> Result<Json<StuckAgentsView>, ApiError> {
    let w = workflow(&st).await?;
    let doc = current(&st, &w).await?;
    let default = stuck::filled(&Map::new()).map_err(invalid)?;
    if default != doc || st.store.current_version_author(&w.id).await? != Some(Author::Builtin) {
        st.store.save_workflow_version(&w.id, &default, Author::Builtin, "reset to default", now_ms()).await?;
    }
    Ok(Json(view_of(&st).await?))
}
