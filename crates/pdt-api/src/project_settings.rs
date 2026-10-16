use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use pdt_core::host::{CredentialPolicy, effective_credentials};
use pdt_store::ProjectSettings;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectSettingsView {
    /// The project's own choice; null means it follows the host setting.
    pub agent_credentials: Option<CredentialPolicy>,
    pub effective_credentials: CredentialPolicy,
    /// Host overrides of the repo's `.pdt/project.toml`, as TOML text.
    pub run_overrides: Option<String>,
}

/// Replaces both fields.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectSettingsInput {
    pub agent_credentials: Option<CredentialPolicy>,
    pub run_overrides: Option<String>,
}

pub(crate) async fn view(st: &AppState, project_id: &str) -> Result<ProjectSettingsView, ApiError> {
    st.store.get_project(project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let p = st.store.get_project_settings(project_id).await?;
    let host = st.store.get_host_settings().await?.agent_credentials;
    Ok(ProjectSettingsView {
        agent_credentials: p.agent_credentials,
        effective_credentials: effective_credentials(host, p.agent_credentials),
        run_overrides: p.run_overrides,
    })
}

pub(crate) async fn get_settings(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectSettingsView>, ApiError> {
    Ok(Json(view(&st, &id).await?))
}

pub(crate) async fn put_settings(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<ProjectSettingsInput>, JsonRejection>,
) -> Result<Json<ProjectSettingsView>, ApiError> {
    let Json(input) = payload?;
    st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let run_overrides = input.run_overrides.filter(|t| !t.trim().is_empty());
    st.store
        .put_project_settings(&id, &ProjectSettings { run_overrides, agent_credentials: input.agent_credentials })
        .await?;
    Ok(Json(view(&st, &id).await?))
}
