use crate::error::ApiError;
use crate::routes::AppState;
use crate::secrets::UPDATE_TOKEN;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use pdt_core::host::{HostSettings, HostSettingsError, HostSettingsPatch};
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostSettingsView {
    pub settings: HostSettings,
    /// `PDT_OWNER_LOGIN`, which overrides `settings.ownerLogin` when set.
    pub owner_from_env: Option<String>,
    pub token_present: bool,
    /// The last result of applying the remote-access setting.
    pub remote: crate::remote::RemoteStatus,
}

impl From<HostSettingsError> for ApiError {
    fn from(e: HostSettingsError) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string())
    }
}

pub(crate) async fn token_present(st: &AppState) -> Result<bool, ApiError> {
    let secrets = st.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.get(UPDATE_TOKEN))
        .await
        .map_err(ApiError::internal)?
        .map(|t| t.is_some())
        .map_err(ApiError::internal)
}

pub(crate) async fn view(st: &AppState) -> Result<HostSettingsView, ApiError> {
    Ok(HostSettingsView {
        settings: st.store.get_host_settings().await?,
        owner_from_env: st.owner_from_env.clone(),
        token_present: token_present(st).await?,
        remote: st.remote.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    })
}

pub(crate) async fn get_settings(State(st): State<AppState>) -> Result<Json<HostSettingsView>, ApiError> {
    Ok(Json(view(&st).await?))
}

pub(crate) async fn put_settings(
    State(st): State<AppState>,
    payload: Result<Json<HostSettingsPatch>, JsonRejection>,
) -> Result<Json<HostSettingsView>, ApiError> {
    let Json(patch) = payload?;
    let guard = st.settings_lock.lock().await;
    let next = st.store.get_host_settings().await?.apply(&patch)?;
    let effective_owner = st.owner_from_env.clone().or(next.owner_login.clone());
    if next.remote_access && effective_owner.is_none() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "owner_required",
            "set the owner (a Tailscale login) before turning on remote access",
        ));
    }
    st.store.put_host_settings(&next).await?;
    if patch.remote_access.is_some() {
        crate::remote::apply_remote_access(&st).await;
    }
    drop(guard);
    Ok(Json(view(&st).await?))
}
