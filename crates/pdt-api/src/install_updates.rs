//! Settings → Updates (PDT-369): the Install updates workflow's inputs (a time window, stable releases only, installing
//! anyway after a while, the check after installing), read and written as a new version of it. A workflow the user
//! changed beyond its inputs, or turned off, is only shown (with Reset to default). The host's update setting (off,
//! notify, install when idle) and channel stay host settings: they decide what the checks find.

use crate::error::{ApiError, OrNotFound, codes};
use crate::integrations::NotificationChannel;
use crate::routes::{AppState, now_ms};
use crate::workflow::builtins::stuck_agents::SETTINGS_NOTE;
use crate::workflow::builtins::{INSTALL_UPDATES, install_updates as install};
use crate::workflow::routes::{body, checked};
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::routing::{get, post};
use axum::{Json, Router};
use pdt_core::workflow::api::{Author, Workflow};
use pdt_core::workflow::template::TemplateParam;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ts_rs::TS;

pub fn router() -> Router<AppState> {
    Router::new().route("/install-updates", get(view).put(save)).route("/install-updates/reset", post(reset))
}

/// The workflow as Settings shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallUpdatesView {
    pub workflow_id: String,
    pub name: String,
    /// Off: no update installs by itself.
    pub enabled: bool,
    /// Changed beyond its inputs in the workflow editor: Settings can't change it.
    pub customized: bool,
    pub params: Vec<TemplateParam>,
    /// Its inputs now, by param id (`installAnywayAfter` in ms).
    #[ts(type = "Record<string, unknown>")]
    pub values: Map<String, Value>,
    #[ts(type = "Record<string, unknown>")]
    pub defaults: Map<String, Value>,
    /// The chat services a plugin that is on posts to (the connection kinds `chat` may be).
    #[serde(skip_deserializing)]
    pub channels: Vec<NotificationChannel>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallUpdatesInput {
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
    st.store.workflow_by_builtin(INSTALL_UPDATES).await?.or_404("workflow")
}

async fn current(st: &AppState, w: &Workflow) -> Result<pdt_core::workflow::doc::WorkflowDoc, ApiError> {
    st.store.workflow_doc(&w.id, w.version).await?.or_404("workflow version")
}

async fn view_of(st: &AppState) -> Result<InstallUpdatesView, ApiError> {
    let w = workflow(st).await?;
    let doc = current(st, &w).await?;
    Ok(InstallUpdatesView {
        workflow_id: w.id,
        name: w.name,
        enabled: w.enabled,
        customized: install::customized(&doc),
        params: install::install_params(),
        values: install::values_of(&doc),
        defaults: install::defaults(),
        channels: st.registry().notification_channels(),
    })
}

/// `GET /api/install-updates`.
async fn view(State(st): State<AppState>) -> Result<Json<InstallUpdatesView>, ApiError> {
    Ok(Json(view_of(&st).await?))
}

/// `PUT /api/install-updates {values}`: changed inputs, as a new version of the workflow (it still follows PDT's
/// newer versions). A customized workflow is refused.
async fn save(
    State(st): State<AppState>,
    payload: Result<Json<InstallUpdatesInput>, JsonRejection>,
) -> Result<Json<InstallUpdatesView>, ApiError> {
    let input = body(payload)?;
    {
        let _lock = st.settings_lock.lock().await;
        let w = workflow(&st).await?;
        let doc = current(&st, &w).await?;
        if install::customized(&doc) {
            return Err(ApiError::conflict(
                codes::CUSTOMIZED,
                format!("{} was changed in the workflow editor: change it there, or reset it to the default", w.name),
            ));
        }
        let mut values = install::values_of(&doc);
        for (k, v) in input.values {
            if !values.contains_key(&k) {
                return Err(ApiError::invalid(format!("there is no input {k}")));
            }
            values.insert(k, v);
        }
        let mut next = install::filled(&values).map_err(ApiError::invalid)?;
        (next.name, next.description) = (doc.name.clone(), doc.description.clone());
        checked(&st, &next)?;
        if next != doc {
            st.store.save_workflow_version(&w.id, &next, Author::User, SETTINGS_NOTE, now_ms()).await?;
        }
    }
    Ok(Json(view_of(&st).await?))
}

/// `POST /api/install-updates/reset`: the workflow as PDT ships it, on, following PDT's newer versions again.
async fn reset(State(st): State<AppState>) -> Result<Json<InstallUpdatesView>, ApiError> {
    {
        let _lock = st.settings_lock.lock().await;
        let w = workflow(&st).await?;
        let doc = current(&st, &w).await?;
        let default = install::filled(&Map::new()).map_err(ApiError::invalid)?;
        if default != doc || st.store.current_version_author(&w.id).await? != Some(Author::Builtin) {
            st.store.save_workflow_version(&w.id, &default, Author::Builtin, "reset to default", now_ms()).await?;
        }
        if !w.enabled {
            st.store.set_workflow_enabled(&w.id, true, now_ms()).await?;
        }
    }
    wake(&st).await;
    Ok(Json(view_of(&st).await?))
}

/// Updates were just set to install when idle: the update on offer, announced while they weren't, gets a run of
/// Install updates now (its run then ended at once), unless one is going already. Runs started by `update.available`
/// look at the setting as they wait, so this is only for the update already announced.
pub(crate) async fn wake(st: &AppState) {
    let Some(offer) = crate::update::on_offer(&st.update) else { return };
    let _engine = st.engine_lock.lock().await;
    let Ok(Some(w)) = st.store.workflow_by_builtin(INSTALL_UPDATES).await else { return };
    if !w.enabled {
        return;
    }
    let f = pdt_store::RunFilter { workflow_id: Some(w.id.clone()), limit: 20, ..Default::default() };
    match st.store.list_workflow_runs(&f).await {
        Ok(runs) if runs.iter().any(|r| !r.status.is_finished()) => return,
        Ok(_) => {}
        Err(e) => return tracing::warn!(error = %e, "could not read Install updates' runs"),
    }
    let Ok(Some(doc)) = st.store.workflow_doc(&w.id, w.version).await else { return };
    let Some(node) = doc.nodes.iter().find(|n| n.kind == pdt_core::workflow::events::UPDATE_AVAILABLE) else { return };
    let mut trigger = serde_json::to_value(&offer).unwrap_or(Value::Null);
    trigger["type"] = json!(pdt_core::workflow::events::UPDATE_AVAILABLE);
    let key = format!("{}:auto:{}:{}", node.id, offer.version, now_ms());
    let node = node.id.clone();
    if let Err(e) = crate::workflow::engine::start_run(st, &w, &doc, &node, trigger, None, key, None).await {
        tracing::warn!(error = %e, "could not start Install updates for the update on offer");
    }
}
