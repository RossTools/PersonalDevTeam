use crate::error::ApiError;
use crate::routes::AppState;
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
    /// Why the token could not be read (e.g. the keychain is waiting on an approval prompt); `tokenPresent` is then
    /// false. The rest of the settings still load.
    pub token_error: Option<String>,
    /// The last result of applying the remote-access setting.
    pub remote: crate::remote::RemoteStatus,
    /// Where clones go: `settings.projectsDir`, else `~/work`.
    pub projects_dir: Option<String>,
}

impl From<HostSettingsError> for ApiError {
    fn from(e: HostSettingsError) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string())
    }
}

pub(crate) async fn view(st: &AppState) -> Result<HostSettingsView, ApiError> {
    let (token_present, token_error) = match crate::update::read_token(st).await {
        Ok(t) => (t.is_some(), None),
        Err(e) => (false, Some(e.message().to_string())),
    };
    let settings = st.store.get_host_settings().await?;
    let projects_dir =
        crate::projects::projects_dir(settings.projects_dir.as_deref()).map(|p| p.to_string_lossy().into_owned());
    Ok(HostSettingsView {
        projects_dir,
        settings,
        owner_from_env: st.owner_from_env.clone(),
        token_present,
        token_error,
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
    st.refresh_owner().await;
    if patch.agent_program.is_some() {
        st.refresh_programs().await;
        // The checks looked for the old program: the next look runs them again.
        *st.checks.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    if patch.remote_access.is_some() {
        crate::remote::apply_locked(&st).await;
    }
    drop(guard);
    if patch.default_agent.is_some() {
        crate::setup::confirm_default_agent(&st).await?;
    }
    Ok(Json(view(&st).await?))
}
