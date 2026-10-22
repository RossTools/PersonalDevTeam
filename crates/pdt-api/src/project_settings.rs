use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::host::{CredentialPolicy, effective_credentials};
use pdt_core::policy::{AutoStart, MergeBy, ProjectPolicy};
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
    /// Given to every agent session in the project, after PDT's own context (kept on this host, not in the repo).
    pub agent_instructions: Option<String>,
    /// Merge… goes through the attempt's pull request only (no local merge into the base branch).
    pub merge_via_pr: bool,
    /// The permission mode of every persona's session here, over the persona's own (null: the persona's, else the
    /// host default).
    pub agent_permission_mode: Option<pdt_core::conversation::PermissionMode>,
    /// Who may merge, approvals, the running-attempt cap and auto-start (Phase 7c).
    pub policy: ProjectPolicy,
}

/// Replaces `agentCredentials` and `runOverrides`. `agentInstructions` ("" clears it), `mergeViaPr`,
/// `agentPermissionMode` ("" clears it) and the policies (`maxRunningAttempts` 0 clears it) are replaced when present,
/// so a client that does not know them keeps them.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectSettingsInput {
    pub agent_credentials: Option<CredentialPolicy>,
    pub run_overrides: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub agent_instructions: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub merge_via_pr: Option<bool>,
    /// `ask`, `accept_edits`, `plan`, `bypass`, or "" for none.
    #[serde(default)]
    #[ts(optional)]
    pub agent_permission_mode: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub merge_by: Option<MergeBy>,
    #[serde(default)]
    #[ts(optional)]
    pub require_approval: Option<bool>,
    /// 0 clears it (no cap).
    #[serde(default)]
    #[ts(optional)]
    pub max_running_attempts: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub auto_start: Option<AutoStart>,
}

/// The highest running-attempt cap a project can set.
const MAX_RUNNING_ATTEMPTS: u32 = 100;

pub(crate) async fn view(st: &AppState, project_id: &str) -> Result<ProjectSettingsView, ApiError> {
    st.store.get_project(project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let p = st.store.get_project_settings(project_id).await?;
    let host = st.store.get_host_settings().await?.agent_credentials;
    Ok(ProjectSettingsView {
        agent_credentials: p.agent_credentials,
        effective_credentials: effective_credentials(host, p.agent_credentials),
        run_overrides: p.run_overrides,
        agent_instructions: p.agent_instructions,
        merge_via_pr: p.merge_via_pr,
        agent_permission_mode: p.agent_permission_mode,
        policy: p.policy,
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
    if let Some(t) = &run_overrides {
        crate::runconfig::validate_overrides(t)?;
    }
    let current = st.store.get_project_settings(&id).await?;
    let agent_instructions = match input.agent_instructions.as_deref() {
        Some(text) => pdt_core::agent_context::clean_project_instructions(Some(text))
            .map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", m))?,
        None => current.agent_instructions,
    };
    let agent_permission_mode = match input.agent_permission_mode.as_deref().map(str::trim) {
        None => current.agent_permission_mode,
        Some("") => None,
        Some(m) => Some(pdt_core::conversation::PermissionMode::parse(m).ok_or_else(|| {
            ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", format!("{m} is not a permission mode"))
        })?),
    };
    let max_running_attempts = match input.max_running_attempts {
        None => current.policy.max_running_attempts,
        Some(0) => None,
        Some(n) if n > MAX_RUNNING_ATTEMPTS => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                format!("the running-attempt cap is at most {MAX_RUNNING_ATTEMPTS} (0: no cap)"),
            ));
        }
        Some(n) => Some(n),
    };
    let policy = ProjectPolicy {
        merge_by: input.merge_by.unwrap_or(current.policy.merge_by),
        require_approval: input.require_approval.unwrap_or(current.policy.require_approval),
        max_running_attempts,
        auto_start: input.auto_start.unwrap_or(current.policy.auto_start),
    };
    let settings = ProjectSettings {
        run_overrides,
        agent_credentials: input.agent_credentials,
        agent_instructions,
        merge_via_pr: input.merge_via_pr.unwrap_or(current.merge_via_pr),
        agent_permission_mode,
        policy,
    };
    st.store.put_project_settings(&id, &settings).await?;
    Ok(Json(view(&st, &id).await?))
}
