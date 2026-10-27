use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::host::{CredentialPolicy, effective_credentials};
use pdt_core::integration::IntegrationSettings;
use pdt_core::policy::{AutoStart, MergeBy, ProjectPolicy};
use pdt_store::ProjectSettings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// Its settings for each enabled integration it is linked to, by the integration's id (PDT-174): `connection` and
    /// the fields `GET /api/integrations/project-settings` declares. A turned-off integration's are kept but not shown
    /// (PDT-173).
    #[ts(type = "Record<string, Record<string, unknown>>")]
    pub integrations: BTreeMap<String, IntegrationSettings>,
}

/// Replaces `agentCredentials` and `runOverrides`. `agentInstructions` ("" clears it), `mergeViaPr`,
/// `agentPermissionMode` ("" clears it), the policies (`maxRunningAttempts` 0 clears it) and each integration named in
/// `integrations` are replaced when present,
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
    /// Tickets with any of these labels skip `requireApproval` ([] clears them).
    #[serde(default)]
    #[ts(optional)]
    pub approval_exempt_labels: Option<Vec<String>>,
    /// The PM may create and edit tickets and message sessions here without asking (PDT-81).
    #[serde(default)]
    #[ts(optional)]
    pub pm_acts: Option<bool>,
    /// Auto-start takes a ticket only once it has gone this many seconds unchanged (PDT-146); 0: at once.
    #[serde(default)]
    #[ts(optional)]
    pub start_delay_seconds: Option<u32>,
    /// An automated merge waits for green CI on the pull request's head (PDT-151).
    #[serde(default)]
    #[ts(optional)]
    pub require_green_ci: Option<bool>,
    /// Integration settings by integration id: each one named is set (keys left out keep their values), null or a
    /// `connection` of "" unlinks it, and integrations not named are kept.
    #[serde(default)]
    #[ts(optional, type = "Record<string, Record<string, unknown> | null>")]
    pub integrations: Option<BTreeMap<String, Option<IntegrationSettings>>>,
}

/// The highest running-attempt cap a project can set.
const MAX_RUNNING_ATTEMPTS: u32 = 100;
/// The longest start delay a project can set: a day.
const MAX_START_DELAY_SECONDS: u32 = 24 * 60 * 60;

pub(crate) async fn view(st: &AppState, project_id: &str) -> Result<ProjectSettingsView, ApiError> {
    st.store.get_project(project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let p = st.store.get_project_settings(project_id).await?;
    let host = st.store.get_host_settings().await?.agent_credentials;
    let reg = st.registry();
    let mut integrations = p.integrations;
    integrations.retain(|id, _| reg.project_settings().any(|s| s.integration == *id));
    Ok(ProjectSettingsView {
        agent_credentials: p.agent_credentials,
        effective_credentials: effective_credentials(host, p.agent_credentials),
        run_overrides: p.run_overrides,
        agent_instructions: p.agent_instructions,
        merge_via_pr: p.merge_via_pr,
        agent_permission_mode: p.agent_permission_mode,
        policy: p.policy,
        integrations,
    })
}

fn invalid(m: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", m)
}

/// A connection name, checked to be a connection of `kind`; None for "" (unlink).
async fn linked_connection(st: &AppState, name: &str, kind: &str) -> Result<Option<String>, ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    match st.store.connection_by_name(name).await? {
        Some(c) if c.kind == kind => Ok(Some(c.name)),
        Some(c) => Err(invalid(format!("{} is a {} connection, not {kind}", c.name, c.kind))),
        None => Err(invalid(format!("no connection is called {name}"))),
    }
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
    if input.start_delay_seconds.is_some_and(|d| d > MAX_START_DELAY_SECONDS) {
        return Err(invalid(format!("the start delay is at most {MAX_START_DELAY_SECONDS} seconds")));
    }
    let approval_exempt_labels = match &input.approval_exempt_labels {
        None => current.policy.approval_exempt_labels,
        Some(l) => pdt_core::ticket::normalize_labels(l)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string()))?,
    };
    let policy = ProjectPolicy {
        merge_by: input.merge_by.unwrap_or(current.policy.merge_by),
        require_approval: input.require_approval.unwrap_or(current.policy.require_approval),
        max_running_attempts,
        auto_start: input.auto_start.unwrap_or(current.policy.auto_start),
        approval_exempt_labels,
        pm_acts: input.pm_acts.unwrap_or(current.policy.pm_acts),
        start_delay_seconds: input.start_delay_seconds.unwrap_or(current.policy.start_delay_seconds),
        require_green_ci: input.require_green_ci.unwrap_or(current.policy.require_green_ci),
    };
    let mut integrations = current.integrations;
    let reg = st.registry();
    for (id, given) in input.integrations.unwrap_or_default() {
        let applied = match given {
            None => None,
            Some(given) => reg.apply_project_settings(&id, &given, integrations.get(&id)).map_err(invalid)?,
        };
        match applied {
            None => {
                integrations.remove(&id);
            }
            Some((schema, mut settings)) => {
                let name = settings.get("connection").and_then(|c| c.as_str()).unwrap_or_default().to_string();
                let Some(name) = linked_connection(&st, &name, &schema.connection_kind).await? else {
                    integrations.remove(&id);
                    continue;
                };
                settings.insert("connection".into(), name.into());
                integrations.insert(id, settings);
            }
        }
    }
    let settings = ProjectSettings {
        run_overrides,
        agent_credentials: input.agent_credentials,
        agent_instructions,
        merge_via_pr: input.merge_via_pr.unwrap_or(current.merge_via_pr),
        agent_permission_mode,
        policy,
        integrations,
    };
    st.store.put_project_settings(&id, &settings).await?;
    Ok(Json(view(&st, &id).await?))
}
