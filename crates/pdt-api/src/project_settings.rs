use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::buildkite::{BUILDKITE, BuildkiteSettings};
use pdt_core::host::{CredentialPolicy, effective_credentials};
use pdt_core::jira::{DEFAULT_DONE, JIRA, JiraSettings};
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
    /// Its Jira project (Phase 7e): issues assigned to the user become tickets, Done moves the issue.
    pub jira: Option<JiraSettings>,
    /// Its Buildkite pipelines (Phase 7e): CI of its attempts' branches.
    pub buildkite: Option<BuildkiteSettings>,
}

/// Links a project to Jira; a `connection` of "" unlinks it.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraInput {
    pub connection: String,
    #[serde(default)]
    #[ts(optional)]
    pub scope: Option<String>,
    /// Default `Done`.
    #[serde(default)]
    #[ts(optional)]
    pub done: Option<String>,
}

/// Links a project to Buildkite pipelines; a `connection` of "" unlinks it.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BuildkiteInput {
    pub connection: String,
    #[serde(default)]
    #[ts(optional)]
    pub pipelines: Option<Vec<String>>,
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
    #[serde(default)]
    #[ts(optional)]
    pub jira: Option<JiraInput>,
    #[serde(default)]
    #[ts(optional)]
    pub buildkite: Option<BuildkiteInput>,
}

/// The highest running-attempt cap a project can set.
const MAX_RUNNING_ATTEMPTS: u32 = 100;
/// The longest start delay a project can set: a day.
const MAX_START_DELAY_SECONDS: u32 = 24 * 60 * 60;

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
        jira: p.jira,
        buildkite: p.buildkite,
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
    };
    let jira = match input.jira {
        None => current.jira,
        Some(j) => match linked_connection(&st, &j.connection, JIRA).await? {
            None => None,
            Some(connection) => {
                let done = j.done.as_deref().map(str::trim).filter(|d| !d.is_empty());
                let old = current.jira.as_ref();
                Some(JiraSettings {
                    connection,
                    scope: j.scope.map(|s| s.trim().to_string()).or(old.map(|o| o.scope.clone())).unwrap_or_default(),
                    done: done.map(String::from).or(old.map(|o| o.done.clone())).unwrap_or_else(|| DEFAULT_DONE.into()),
                })
            }
        },
    };
    let buildkite = match input.buildkite {
        None => current.buildkite,
        Some(b) => match linked_connection(&st, &b.connection, BUILDKITE).await? {
            None => None,
            Some(connection) => {
                let pipelines = match b.pipelines {
                    Some(list) => {
                        let mut out: Vec<String> = vec![];
                        for p in list.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
                            if !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                                return Err(invalid(format!("{p:?} is not a pipeline slug")));
                            }
                            if !out.iter().any(|o| o == p) {
                                out.push(p.to_string());
                            }
                        }
                        out
                    }
                    None => current.buildkite.map(|o| o.pipelines).unwrap_or_default(),
                };
                Some(BuildkiteSettings { connection, pipelines })
            }
        },
    };
    let settings = ProjectSettings {
        run_overrides,
        agent_credentials: input.agent_credentials,
        agent_instructions,
        merge_via_pr: input.merge_via_pr.unwrap_or(current.merge_via_pr),
        agent_permission_mode,
        policy,
        jira,
        buildkite,
    };
    st.store.put_project_settings(&id, &settings).await?;
    Ok(Json(view(&st, &id).await?))
}
