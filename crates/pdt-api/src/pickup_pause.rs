//! Pickup paused (PDT-370): while it is, auto-start starts no new tickets, the pickup watch tells nothing, and the
//! Board shows why on each ticket that would have started. Agents already running carry on. `pickup.pause` and
//! `pickup.resume` set it (the built-in Clean up storage pauses it while a disk is almost full); the user resumes it
//! from Settings › Storage. Kept over a restart.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where the pause is kept (a host value).
const KEY: &str = "pickup.paused";

/// Why pickup is paused, and by whom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PickupPause {
    /// Shown on the Board and in Settings › Storage ("the disk is almost full").
    pub reason: String,
    /// The workflow that paused it; its `pickup.resume` resumes it.
    pub workflow_id: Option<String>,
    #[ts(type = "number")]
    pub at: i64,
}

/// The pause, if pickup is paused.
pub async fn paused(st: &AppState) -> Option<PickupPause> {
    match st.store.get_host_value(KEY).await {
        Ok(v) => v.and_then(|v| serde_json::from_value(v).ok()),
        Err(e) => {
            tracing::warn!(error = %e, "could not read whether pickup is paused");
            None
        }
    }
}

/// Pauses pickup (or says why anew).
pub async fn pause(st: &AppState, reason: &str, workflow_id: Option<&str>) -> Result<PickupPause, String> {
    let p = PickupPause { reason: reason.trim().to_string(), workflow_id: workflow_id.map(String::from), at: now_ms() };
    let was = paused(st).await;
    if was.as_ref().is_some_and(|w| w.reason == p.reason && w.workflow_id == p.workflow_id) {
        return Ok(was.unwrap_or(p));
    }
    let v = serde_json::to_value(&p).map_err(|e| e.to_string())?;
    st.store.put_host_value(KEY, &v).await.map_err(|e| e.to_string())?;
    tracing::warn!(reason = %p.reason, "pickup paused: auto-start starts no new tickets");
    Ok(p)
}

/// Resumes pickup: a pause by `workflow_id` only (None: any). Whether it was paused and now isn't.
pub async fn resume(st: &AppState, workflow_id: Option<&str>) -> Result<bool, String> {
    let Some(p) = paused(st).await else { return Ok(false) };
    if workflow_id.is_some_and(|w| p.workflow_id.as_deref() != Some(w)) {
        return Ok(false);
    }
    st.store.delete_host_value(KEY).await.map_err(|e| e.to_string())?;
    tracing::info!("pickup resumed: auto-start goes on");
    Ok(true)
}

/// `GET /api/pickup/pause`: the pause, or null.
async fn get(State(st): State<AppState>) -> Json<Option<PickupPause>> {
    Json(paused(&st).await)
}

/// `DELETE /api/pickup/pause`: the user resumes pickup, whoever paused it.
async fn delete(State(st): State<AppState>) -> Result<axum::http::StatusCode, ApiError> {
    resume(&st, None).await.map_err(ApiError::from_text)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/pickup/pause", axum::routing::get(get).delete(delete))
}
