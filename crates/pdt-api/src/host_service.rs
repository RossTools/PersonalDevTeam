//! PDT's background service (PDT-489): whether pdt-host starts at login (Settings → This host), and what stopping
//! PDT would interrupt (the tray asks before "Stop PDT and quit").

use crate::error::{ApiError, codes};
use crate::routes::AppState;
use crate::update::UpdateState;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Registers and unregisters pdt-host's login item. `AppState::new` installs [`Unavailable`], which touches nothing;
/// only pdtd's production wiring installs the real one, so tests never change the machine's login items.
pub trait StartAtLogin: Send + Sync {
    /// Why PDT can't be started at login here, or None when it can.
    fn unavailable(&self) -> Option<String>;
    /// Whether pdt-host is registered to start at login.
    fn registered(&self) -> bool;
    /// Registers or unregisters it; the running PDT keeps running either way.
    fn set(&self, on: bool) -> Result<(), String>;
}

/// Starting at login can't be changed here (tests, a dev instance).
pub struct Unavailable(pub String);

impl StartAtLogin for Unavailable {
    fn unavailable(&self) -> Option<String> {
        Some(self.0.clone())
    }
    fn registered(&self) -> bool {
        false
    }
    fn set(&self, _: bool) -> Result<(), String> {
        Err(self.0.clone())
    }
}

/// Settings → This host → Start PDT at login.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartAtLoginView {
    /// pdt-host is registered to start at login.
    pub on: bool,
    /// Why it can't be changed here; null when it can.
    pub unavailable: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct StartAtLoginPatch {
    pub on: bool,
}

/// What stopping PDT now would interrupt; empty when nothing.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct HostBusy {
    pub reasons: Vec<String>,
}

async fn view(st: &AppState) -> Result<StartAtLoginView, ApiError> {
    let login = st.start_at_login.clone();
    tokio::task::spawn_blocking(move || match login.unavailable() {
        Some(why) => StartAtLoginView { on: false, unavailable: Some(why) },
        None => StartAtLoginView { on: login.registered(), unavailable: None },
    })
    .await
    .map_err(ApiError::internal)
}

async fn get_start_at_login(State(st): State<AppState>) -> Result<Json<StartAtLoginView>, ApiError> {
    Ok(Json(view(&st).await?))
}

async fn put_start_at_login(
    State(st): State<AppState>,
    payload: Result<Json<StartAtLoginPatch>, JsonRejection>,
) -> Result<Json<StartAtLoginView>, ApiError> {
    let Json(p) = payload?;
    let login = st.start_at_login.clone();
    if let Some(why) = login.unavailable() {
        return Err(ApiError::conflict(codes::START_AT_LOGIN_UNAVAILABLE, why));
    }
    tokio::task::spawn_blocking(move || login.set(p.on))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, codes::START_AT_LOGIN_FAILED, e))?;
    tracing::info!(on = p.on, "start PDT at login changed");
    Ok(Json(view(&st).await?))
}

/// What the update check counts as busy, and an update being downloaded or installed.
async fn get_busy(State(st): State<AppState>) -> Json<HostBusy> {
    let mut reasons = crate::update::busy_reasons(&st).await;
    match st.update.snapshot().state {
        UpdateState::Downloading => reasons.push("an update is downloading".into()),
        UpdateState::Installing => reasons.push("an update is being installed".into()),
        _ => {}
    }
    Json(HostBusy { reasons })
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new()
        .route("/host/busy", get(get_busy))
        .route("/host/start-at-login", get(get_start_at_login).put(put_start_at_login))
}
