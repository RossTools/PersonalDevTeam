use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::project::Project;
use pdt_core::worktree::{NewWorktree, WORKTREES_DIR, Worktree, worktree_path};
use serde::Deserialize;
use std::path::{Path as FsPath, PathBuf};

fn mark_missing(mut w: Worktree) -> Worktree {
    w.missing = !FsPath::new(&w.path).is_dir();
    w
}

pub(crate) async fn list(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<Worktree>>, ApiError> {
    st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    Ok(Json(st.store.list_worktrees(&id).await?.into_iter().map(mark_missing).collect()))
}

/// The HEAD of `path` when git already has it as a worktree of `repo` checked out on `branch` (one made outside
/// PDT, e.g. with `git worktree add`), so PDT can register it instead of failing. Blocking.
pub(crate) fn adoptable(repo: &FsPath, path: &FsPath, branch: &str) -> Option<String> {
    let want = std::fs::canonicalize(path).ok()?;
    pdt_git::list_worktrees(repo)
        .ok()?
        .into_iter()
        .find(|w| w.branch.as_deref() == Some(branch) && std::fs::canonicalize(&w.path).ok().as_ref() == Some(&want))
        .map(|w| w.head)
}

pub(crate) async fn create(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<NewWorktree>, JsonRejection>,
) -> Result<(StatusCode, Json<Worktree>), ApiError> {
    let Json(input) = payload?;
    let valid = input.validate()?;
    let project = st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let base = valid.base.unwrap_or_else(|| project.base_branch.clone());
    let wt = make_worktree(&st, &project, &valid.branch, &base, false).await?;
    Ok((StatusCode::CREATED, Json(wt)))
}

/// Makes `.worktrees/<branch>` on a new branch from `base` and records it. A worktree git already has there for
/// `branch` (made outside PDT) is adopted; any other folder there is `worktree_path_taken`. With `existing_branch`,
/// a branch that already exists is checked out instead of refused (`branch_exists`).
pub(crate) async fn make_worktree(
    st: &AppState,
    project: &Project,
    branch: &str,
    base: &str,
    existing_branch: bool,
) -> Result<Worktree, ApiError> {
    let repo = PathBuf::from(&project.repo_path);
    let path = worktree_path(&repo, branch);
    let (b, target, base) = (branch.to_string(), path.clone(), base.to_string());
    let base_commit = tokio::task::spawn_blocking(move || -> Result<String, ApiError> {
        pdt_git::ensure_excluded(&repo, &format!("/{WORKTREES_DIR}/"))?;
        if target.exists() {
            return adoptable(&repo, &target, &b).ok_or_else(|| {
                ApiError::new(
                    StatusCode::CONFLICT,
                    "worktree_path_taken",
                    format!("the worktree folder {} is already in use (by another branch?)", target.display()),
                )
            });
        }
        if existing_branch && pdt_git::branch_exists(&repo, &b) {
            return Ok(pdt_git::add_worktree_for_branch(&repo, &target, &b)?);
        }
        Ok(pdt_git::add_worktree(&repo, &target, &b, &base)?)
    })
    .await
    .map_err(ApiError::internal)??;
    let wt = Worktree {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: project.id.clone(),
        branch: branch.to_string(),
        path: path.to_string_lossy().into_owned(),
        base_commit,
        created_at: now_ms(),
        missing: false,
    };
    st.store.insert_worktree(&wt).await?;
    Ok(wt)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct RemoveQuery {
    /// Remove even with uncommitted changes (the user confirmed).
    force: bool,
    /// Also delete the branch.
    delete_branch: bool,
}

fn conflict(code: &'static str, msg: &str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, msg.to_string())
}

/// Removes a worktree's folder (git worktree remove) and forgets it. A worktree whose folder is already gone is
/// just forgotten.
pub(crate) async fn remove(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<RemoveQuery>,
) -> Result<StatusCode, ApiError> {
    remove_worktree(&st, &id, q.force, q.delete_branch).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a worktree (see `remove`): 409 `worktree_busy` while a session or run uses it, `worktree_dirty` with
/// uncommitted changes unless `force`, `branch_not_deleted` when only the branch stayed.
pub(crate) async fn remove_worktree(st: &AppState, id: &str, force: bool, delete_branch: bool) -> Result<(), ApiError> {
    let id = id.to_string();
    let wt = st.store.get_worktree(&id).await?.ok_or_else(|| ApiError::not_found("worktree"))?;
    let project = st.store.get_project(&wt.project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let sessions: Vec<_> = st
        .store
        .list_sessions(&project.id)
        .await?
        .into_iter()
        .filter(|s| s.worktree_id.as_deref() == Some(&id))
        .collect();
    if sessions.iter().any(|s| st.ptys.is_running(&s.id)) {
        return Err(conflict("worktree_busy", "stop this branch's sessions first"));
    }
    let active_run = st.store.list_active_runs().await?.into_iter().any(|r| r.worktree_id.as_deref() == Some(&id));
    if active_run {
        return Err(conflict("worktree_busy", "stop this branch's runs first"));
    }
    let (repo, path, branch) = (PathBuf::from(&project.repo_path), PathBuf::from(&wt.path), wt.branch.clone());
    let branch_error = tokio::task::spawn_blocking(move || -> Result<Option<String>, ApiError> {
        if path.is_dir() {
            if !force && pdt_git::is_dirty(&path)? {
                return Err(conflict(
                    "worktree_dirty",
                    "this worktree has uncommitted changes; remove it anyway to discard them",
                ));
            }
            pdt_git::remove_worktree(&repo, &path, force)?;
        } else {
            pdt_git::prune_worktrees(&repo)?;
        }
        // The folder is gone either way; a branch that won't delete is reported, not fatal.
        Ok(if delete_branch { pdt_git::delete_branch(&repo, &branch).err().map(|e| e.to_string()) } else { None })
    })
    .await
    .map_err(ApiError::internal)??;
    for s in sessions {
        crate::sessions::discard(st, &s.id).await;
    }
    st.store.delete_worktree(&id).await?;
    match branch_error {
        None => Ok(()),
        Some(msg) => Err(ApiError::new(
            StatusCode::CONFLICT,
            "branch_not_deleted",
            format!("the worktree was removed, but its branch was not deleted: {msg}"),
        )),
    }
}
