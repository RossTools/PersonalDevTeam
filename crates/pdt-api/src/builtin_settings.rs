//! Settings backed by a built-in workflow's inputs (PDT-384): `GET <path>` shows them, `PUT <path> {values}` writes
//! changed ones as a new version of the workflow (it still follows PDT's newer versions), `POST <path>/reset` puts back
//! the workflow as PDT ships it, turned on. A workflow changed beyond its inputs, or turned off, is only named by
//! Settings, with Open and Reset to default. One [`InputsBuiltin`] each; its route module is one [`router`] call (or
//! [`router_with`], for what its reset does differently).

use crate::error::{ApiError, OrNotFound, codes};
use crate::integrations::NotificationChannel;
use crate::routes::{AppState, now_ms};
use crate::workflow::builtins::inputs::{InputsBuiltin, SETTINGS_NOTE};
use crate::workflow::routes::{body, checked};
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::future::BoxFuture;
use pdt_core::workflow::api::{Author, Workflow};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::template::TemplateParam;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

/// The routes for one built-in's Settings at `path` (`/pm-checkpoint`).
pub fn router(path: &str, b: &'static InputsBuiltin) -> Router<AppState> {
    router_with(path, b, Reset::TURNS_ON)
}

/// What Reset to default does besides putting back the workflow as PDT ships it.
#[derive(Clone, Copy)]
pub struct Reset {
    /// It turns the workflow on (else it stays off: a Settings switch of its own turns it on and off).
    pub turns_on: bool,
    /// Done after it, with the workflow reset (Install updates installs the update on offer).
    pub then: Option<for<'a> fn(&'a AppState) -> BoxFuture<'a, ()>>,
}

impl Reset {
    pub const TURNS_ON: Reset = Reset { turns_on: true, then: None };
}

/// [`router`], with what its reset does.
pub fn router_with(path: &str, b: &'static InputsBuiltin, r: Reset) -> Router<AppState> {
    Router::new()
        .route(
            path,
            get(move |State(st): State<AppState>| async move { view_of(&st, b).await.map(Json) }).put(
                move |State(st): State<AppState>, payload: Result<Json<BuiltinSettingsInput>, JsonRejection>| async move {
                    save(&st, b, body(payload)?).await.map(Json)
                },
            ),
        )
        .route(
            &format!("{path}/reset"),
            post(move |State(st): State<AppState>| async move {
                reset(&st, b, r.turns_on).await?;
                if let Some(then) = r.then {
                    then(&st).await;
                }
                view_of(&st, b).await.map(Json)
            }),
        )
}

/// The workflow as Settings shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BuiltinSettingsView {
    pub workflow_id: String,
    pub name: String,
    /// Off: Settings names it, with Reset to default (which turns it on).
    pub enabled: bool,
    /// Changed beyond its inputs in the workflow editor: Settings can't change it.
    pub customized: bool,
    pub params: Vec<TemplateParam>,
    /// Its inputs now, by param id (durations in ms).
    #[ts(type = "Record<string, unknown>")]
    pub values: Map<String, Value>,
    #[ts(type = "Record<string, unknown>")]
    pub defaults: Map<String, Value>,
    /// The chat services a plugin that is on posts to (the connection kinds a `connection` input may be).
    #[serde(skip_deserializing)]
    pub channels: Vec<NotificationChannel>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BuiltinSettingsInput {
    /// The inputs to change, by param id; the others stay as they are.
    #[ts(type = "Record<string, unknown>")]
    pub values: Map<String, Value>,
}

/// The built-in, installed if this pdtd hasn't yet.
pub async fn workflow(st: &AppState, b: &InputsBuiltin) -> Result<Workflow, ApiError> {
    {
        let _engine = st.engine_lock.lock().await;
        crate::workflow::builtins::ensure_installed(st).await;
    }
    st.store.workflow_by_builtin(b.id).await?.or_404("workflow")
}

pub async fn current(st: &AppState, w: &Workflow) -> Result<WorkflowDoc, ApiError> {
    st.store.workflow_doc(&w.id, w.version).await?.or_404("workflow version")
}

pub async fn view_of(st: &AppState, b: &InputsBuiltin) -> Result<BuiltinSettingsView, ApiError> {
    let w = workflow(st, b).await?;
    let doc = current(st, &w).await?;
    Ok(BuiltinSettingsView {
        workflow_id: w.id,
        name: w.name,
        enabled: w.enabled,
        customized: b.customized(&doc),
        params: (b.params)(),
        values: b.values_of(&doc),
        defaults: b.defaults(),
        channels: st.registry().notification_channels(),
    })
}

/// Changed inputs, as a new version of the workflow; a customized workflow is refused.
pub async fn save(
    st: &AppState,
    b: &InputsBuiltin,
    input: BuiltinSettingsInput,
) -> Result<BuiltinSettingsView, ApiError> {
    change(st, b, |values| {
        for (k, v) in input.values {
            if !values.contains_key(&k) {
                return Err(ApiError::invalid(format!("there is no input {k}")));
            }
            values.insert(k, v);
        }
        Ok(())
    })
    .await?;
    view_of(st, b).await
}

/// Changes the workflow's inputs (`edit` given them as they are): a new version of it, noted as changed in Settings
/// (it still follows PDT's newer versions). A customized workflow is refused.
pub async fn change(
    st: &AppState,
    b: &InputsBuiltin,
    edit: impl FnOnce(&mut Map<String, Value>) -> Result<(), ApiError>,
) -> Result<(), ApiError> {
    let _lock = st.settings_lock.lock().await;
    let w = workflow(st, b).await?;
    let doc = current(st, &w).await?;
    if b.customized(&doc) {
        return Err(ApiError::conflict(
            codes::CUSTOMIZED,
            format!("{} was changed in the workflow editor: change it there, or reset it to the default", w.name),
        ));
    }
    let mut values = b.values_of(&doc);
    edit(&mut values)?;
    let mut next = b.filled(&values).map_err(ApiError::invalid)?;
    (next.name, next.description) = (doc.name.clone(), doc.description.clone());
    checked(st, &next)?;
    if next != doc {
        st.store.save_workflow_version(&w.id, &next, Author::User, SETTINGS_NOTE, now_ms()).await?;
    }
    Ok(())
}

/// The workflow as PDT ships it, following PDT's newer versions again; turned on if `turn_on`.
pub async fn reset(st: &AppState, b: &InputsBuiltin, turn_on: bool) -> Result<(), ApiError> {
    let _lock = st.settings_lock.lock().await;
    let w = workflow(st, b).await?;
    let doc = current(st, &w).await?;
    let default = b.filled(&Map::new()).map_err(ApiError::invalid)?;
    if default != doc || st.store.current_version_author(&w.id).await? != Some(Author::Builtin) {
        st.store.save_workflow_version(&w.id, &default, Author::Builtin, "reset to default", now_ms()).await?;
    }
    if turn_on && !w.enabled {
        st.store.set_workflow_enabled(&w.id, true, now_ms()).await?;
    }
    Ok(())
}
