use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::worktree::{NewWorktree, WORKTREES_DIR, Worktree, worktree_path};
use std::path::PathBuf;

pub(crate) async fn list(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<Worktree>>, ApiError> {
    st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    Ok(Json(st.store.list_worktrees(&id).await?))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<NewWorktree>, JsonRejection>,
) -> Result<(StatusCode, Json<Worktree>), ApiError> {
    let Json(input) = payload?;
    let valid = input.validate()?;
    let project = st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let repo = PathBuf::from(&project.repo_path);
    let path = worktree_path(&repo, &valid.branch);
    let base = valid.base.unwrap_or_else(|| project.base_branch.clone());
    let (branch, target) = (valid.branch.clone(), path.clone());
    let base_commit = tokio::task::spawn_blocking(move || {
        pdt_git::ensure_excluded(&repo, &format!("/{WORKTREES_DIR}/"))?;
        pdt_git::add_worktree(&repo, &target, &branch, &base)
    })
    .await
    .map_err(ApiError::internal)??;
    let wt = Worktree {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: project.id,
        branch: valid.branch,
        path: path.to_string_lossy().into_owned(),
        base_commit,
        created_at: now_ms(),
    };
    st.store.insert_worktree(&wt).await?;
    Ok((StatusCode::CREATED, Json(wt)))
}
