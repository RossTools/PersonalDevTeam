use crate::error::{ApiError, codes};
use crate::routes::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use pdt_core::brand::PROJECT_CONFIG_FILE;
use pdt_core::project::Project;
use pdt_core::runconfig::{RunConfig, RunConfigError, Source, merge, parse};
use serde::{Deserialize, Serialize};
use std::path::{Path as FsPath, PathBuf};
use ts_rs::TS;

impl From<RunConfigError> for ApiError {
    fn from(e: RunConfigError) -> Self {
        ApiError::bad_request(codes::INVALID_RUN_CONFIG, e.to_string())
    }
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunConfigView {
    pub file_path: String,
    /// The file's text, or null when the checkout has none.
    pub file: Option<String>,
    pub overrides: Option<String>,
    /// Null when the file or the overrides are invalid (see `error`).
    pub effective: Option<RunConfig>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Where {
    worktree_id: Option<String>,
}

/// The folder a run in this place uses, and its branch: the worktree, or the project's main checkout.
pub(crate) async fn checkout_dir(
    st: &AppState,
    project: &Project,
    worktree_id: Option<&str>,
) -> Result<(PathBuf, String), ApiError> {
    match worktree_id {
        Some(wid) => {
            let wt = st
                .store
                .get_worktree(wid)
                .await?
                .filter(|w| w.project_id == project.id)
                .ok_or_else(|| ApiError::invalid("worktree does not belong to this project"))?;
            Ok((PathBuf::from(wt.path), wt.branch))
        }
        None => {
            let repo = PathBuf::from(&project.repo_path);
            let r = repo.clone();
            let info =
                tokio::task::spawn_blocking(move || pdt_git::resolve_repo(&r)).await.map_err(ApiError::internal)??;
            Ok((repo, info.current_branch.unwrap_or_else(|| project.base_branch.clone())))
        }
    }
}

/// The checkout's run configuration file, read on the blocking pool (`GET /api/tickets` reads it for every project
/// with active attempts, on every poll).
async fn read_file(checkout: &FsPath) -> Result<Option<String>, ApiError> {
    let path = checkout.join(PROJECT_CONFIG_FILE);
    let read = tokio::task::spawn_blocking(move || std::fs::read_to_string(path)).await.map_err(ApiError::internal)?;
    match read {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ApiError::from(e)),
    }
}

fn effective(file: Option<&str>, overrides: Option<&str>) -> Result<RunConfig, RunConfigError> {
    let f = file.map(|t| parse(t, Source::File)).transpose()?;
    let o = overrides.map(|t| parse(t, Source::Overrides)).transpose()?;
    merge(f, o)
}

/// What the overrides alone configure (entries the file has no part in); None when they are invalid alone.
pub(crate) fn overrides_only(text: &str) -> Option<RunConfig> {
    effective(None, Some(text)).ok()
}

/// The effective configuration for a run in `checkout`.
pub(crate) async fn load_effective(st: &AppState, project_id: &str, checkout: &FsPath) -> Result<RunConfig, ApiError> {
    effective_for_file(st, project_id, read_file(checkout).await?.as_deref()).await
}

/// The effective configuration for a checkout whose file text is `file` (e.g. read from a branch with git).
pub(crate) async fn effective_for_file(
    st: &AppState,
    project_id: &str,
    file: Option<&str>,
) -> Result<RunConfig, ApiError> {
    let overrides = st.store.get_project_settings(project_id).await?.run_overrides;
    Ok(effective(file, overrides.as_deref())?)
}

/// Checks override text before it is stored. Unlike the repository's file, which may be written for a newer PDT, the
/// overrides are written for this one, so a section it does not know is a mistake.
pub(crate) fn validate_overrides(text: &str) -> Result<(), ApiError> {
    let o = parse(text, Source::Overrides)?;
    if let Some(s) = o.ignored.first() {
        return Err(ApiError::bad_request(
            codes::INVALID_RUN_CONFIG,
            format!("{} is not valid: unknown section `{s}` (expected run, test or area)", Source::Overrides),
        ));
    }
    Ok(())
}

pub(crate) async fn get_config(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(w): Query<Where>,
) -> Result<Json<RunConfigView>, ApiError> {
    let project = st.project(&id).await?;
    let (dir, _) = checkout_dir(&st, &project, w.worktree_id.as_deref()).await?;
    let file = read_file(&dir).await?;
    let overrides = st.store.get_project_settings(&id).await?.run_overrides;
    let (effective, error) = match effective(file.as_deref(), overrides.as_deref()) {
        Ok(c) => (Some(c), None),
        Err(e) => (None, Some(e.to_string())),
    };
    Ok(Json(RunConfigView {
        file_path: dir.join(PROJECT_CONFIG_FILE).to_string_lossy().into_owned(),
        file,
        overrides,
        effective,
        error,
    }))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new().route("/projects/{id}/run-config", get(get_config))
}
