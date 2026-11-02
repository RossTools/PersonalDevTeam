//! Settings → Notifications (PDT-368): the Notification rules workflow's inputs (each kind's level, escalation, quiet
//! hours, a chat channel), read and written as a new version of it. A workflow the user changed beyond its inputs is
//! only shown (with Reset to default).

use crate::error::{ApiError, OrNotFound, codes};
use crate::integrations::NotificationChannel;
use crate::routes::{AppState, now_ms};
use crate::workflow::builtins::stuck_agents::SETTINGS_NOTE;
use crate::workflow::builtins::{NOTIFICATION_RULES, notification_rules as rules};
use crate::workflow::routes::{body, checked};
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::routing::{get, post};
use axum::{Json, Router};
use pdt_core::notification::{KindSetting, NotificationKind, NotificationSettings, Show};
use pdt_core::workflow::api::{Author, Workflow};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use ts_rs::TS;

pub fn router() -> Router<AppState> {
    Router::new().route("/notification-rules", get(view).put(save)).route("/notification-rules/reset", post(reset))
}

/// The workflow as Settings shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NotificationRulesView {
    pub workflow_id: String,
    pub name: String,
    pub enabled: bool,
    /// Changed beyond its inputs in the workflow editor: Settings can't change it.
    pub customized: bool,
    /// Every kind at the level the workflow sets for it (host-wide).
    pub kinds: Vec<KindSetting>,
    /// Its other inputs now, by param id (`escalate`, `escalateAfter` in ms, `quietHours`, `quietFrom`, `quietTo`,
    /// `chat`, `chatQuestions`, `chatApprovals`, `chatFailedRuns`).
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
pub struct NotificationRulesInput {
    /// Levels to change, by kind (`null`: back to PDT's default).
    #[serde(default)]
    #[ts(as = "Option<HashMap<String, Option<Show>>>", optional)]
    pub levels: HashMap<String, Option<Show>>,
    /// Other inputs to change, by param id; the others stay as they are.
    #[serde(default)]
    #[ts(optional, type = "Record<string, unknown>")]
    pub values: Map<String, Value>,
}

/// The built-in, installed if this pdtd hasn't yet.
async fn workflow(st: &AppState) -> Result<Workflow, ApiError> {
    {
        let _engine = st.engine_lock.lock().await;
        crate::workflow::builtins::ensure_installed(st).await;
    }
    st.store.workflow_by_builtin(NOTIFICATION_RULES).await?.or_404("workflow")
}

async fn current(st: &AppState, w: &Workflow) -> Result<pdt_core::workflow::doc::WorkflowDoc, ApiError> {
    st.store.workflow_doc(&w.id, w.version).await?.or_404("workflow version")
}

/// The levels in a set of inputs.
fn levels_in(values: &Map<String, Value>) -> NotificationSettings {
    values.get(rules::LEVELS).map(NotificationSettings::from_json).unwrap_or_default()
}

async fn view_of(st: &AppState) -> Result<NotificationRulesView, ApiError> {
    let w = workflow(st).await?;
    let doc = current(st, &w).await?;
    let customized = rules::customized(&doc);
    let mut values = rules::values_of(&doc);
    // A customized workflow's levels are its own Show levels step's (as the Notifier reads them, but on or off).
    let kinds = if customized {
        let l = doc.node(rules::LEVELS_NODE).and_then(|n| {
            serde_json::from_value::<crate::workflow::nodes::notification_levels::LevelsConfig>(n.config.clone()).ok()
        });
        l.and_then(|c| c.levels().ok()).map(|l| l.levels).unwrap_or_default().kinds()
    } else {
        levels_in(&values).kinds()
    };
    values.remove(rules::LEVELS);
    let mut defaults = rules::defaults();
    defaults.remove(rules::LEVELS);
    Ok(NotificationRulesView {
        workflow_id: w.id,
        name: w.name,
        enabled: w.enabled,
        customized,
        kinds,
        values,
        defaults,
        channels: st.registry().notification_channels(),
    })
}

/// Changes the workflow's inputs: a new version of it, noted as changed in Settings (it still follows PDT's newer
/// versions). A customized workflow is refused.
pub(crate) async fn change(
    st: &AppState,
    levels: HashMap<String, Option<Show>>,
    changes: Map<String, Value>,
) -> Result<(), ApiError> {
    let _lock = st.settings_lock.lock().await;
    let w = workflow(st).await?;
    let doc = current(st, &w).await?;
    if rules::customized(&doc) {
        return Err(ApiError::conflict(
            codes::CUSTOMIZED,
            format!("{} was changed in the workflow editor: change it there, or reset it to the default", w.name),
        ));
    }
    let mut values = rules::values_of(&doc);
    let mut l = levels_in(&values);
    for (kind, show) in levels {
        let k = NotificationKind::parse(&kind)
            .ok_or_else(|| ApiError::invalid(format!("{kind} is not a kind of notification")))?;
        l.set(k, show).map_err(ApiError::invalid)?;
    }
    values.insert(rules::LEVELS.into(), l.to_json());
    for (k, v) in changes {
        if k == rules::LEVELS || !values.contains_key(&k) {
            return Err(ApiError::invalid(format!("there is no input {k}")));
        }
        values.insert(k, v);
    }
    let mut next = rules::filled(&values).map_err(ApiError::invalid)?;
    (next.name, next.description) = (doc.name.clone(), doc.description.clone());
    checked(st, &next)?;
    if next != doc {
        st.store.save_workflow_version(&w.id, &next, Author::User, SETTINGS_NOTE, now_ms()).await?;
    }
    // Rows of kinds turned down below `inbox` go; kinds turned up get theirs.
    crate::notifications::kick(st);
    Ok(())
}

/// `GET /api/notification-rules`.
async fn view(State(st): State<AppState>) -> Result<Json<NotificationRulesView>, ApiError> {
    Ok(Json(view_of(&st).await?))
}

/// `PUT /api/notification-rules {levels?, values?}`: changed inputs, as a new version of the workflow.
async fn save(
    State(st): State<AppState>,
    payload: Result<Json<NotificationRulesInput>, JsonRejection>,
) -> Result<Json<NotificationRulesView>, ApiError> {
    let input = body(payload)?;
    change(&st, input.levels, input.values).await?;
    Ok(Json(view_of(&st).await?))
}

/// `POST /api/notification-rules/reset`: the workflow as PDT ships it (every level at its default, every rule off),
/// following PDT's newer versions again.
async fn reset(State(st): State<AppState>) -> Result<Json<NotificationRulesView>, ApiError> {
    {
        let _lock = st.settings_lock.lock().await;
        let w = workflow(&st).await?;
        let doc = current(&st, &w).await?;
        let default = rules::filled(&Map::new()).map_err(ApiError::invalid)?;
        if default != doc || st.store.current_version_author(&w.id).await? != Some(Author::Builtin) {
            st.store.save_workflow_version(&w.id, &default, Author::Builtin, "reset to default", now_ms()).await?;
        }
    }
    crate::notifications::kick(&st);
    Ok(Json(view_of(&st).await?))
}
