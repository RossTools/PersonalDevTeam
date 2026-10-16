//! Run routes (spec §4.2). The lifecycle itself lives in `run_manager`.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use crate::run_manager;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use pdt_core::run::{NewRun, Run, RunKind, RunState, RunStep, pty_id, run_data_dir};
use pdt_core::runconfig::RunDef;
use pdt_core::worktree::{WORKTREES_DIR, Worktree, worktree_path};
use serde::Serialize;
use std::path::PathBuf;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Branch {
    pub name: String,
    /// False when the branch exists only on `origin` (starting a run there creates a local tracking branch).
    pub local: bool,
}

async fn load(st: &AppState, id: &str) -> Result<Run, ApiError> {
    st.store.get_run(id).await?.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "unknown_run", "run not found"))
}

fn conflict(code: &'static str, msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, msg)
}

fn no_free_port() -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "no_free_port", "no free port in the host's run port range")
}

/// The definition named by `run` in the configuration of its own checkout. Err is a message for the run's error.
pub(crate) async fn definition_for(st: &AppState, run: &Run) -> Result<RunDef, String> {
    let cfg = crate::runconfig::load_effective(st, &run.project_id, std::path::Path::new(&run.cwd))
        .await
        .map_err(|e| format!("the run configuration is invalid: {}", e.message()))?;
    let list = if run.kind == RunKind::Test { cfg.tests } else { cfg.runs };
    list.into_iter().find(|d| d.name == run.name).ok_or_else(|| format!("the configuration has no \"{}\"", run.name))
}

pub(crate) async fn list(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<Run>>, ApiError> {
    st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    Ok(Json(st.store.list_runs(&id).await?))
}

pub(crate) async fn list_active(State(st): State<AppState>) -> Result<Json<Vec<Run>>, ApiError> {
    Ok(Json(st.store.list_active_runs().await?))
}

pub(crate) async fn get_one(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Run>, ApiError> {
    Ok(Json(load(&st, &id).await?))
}

pub(crate) async fn branches(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Branch>>, ApiError> {
    let p = st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let repo = PathBuf::from(p.repo_path);
    let list =
        tokio::task::spawn_blocking(move || pdt_git::list_branches(&repo)).await.map_err(ApiError::internal)??;
    Ok(Json(list.into_iter().map(|b| Branch { name: b.name, local: b.local }).collect()))
}

/// Resolves where a new run goes: an existing worktree, a branch (creating its worktree), or the main checkout.
async fn place(
    st: &AppState,
    project: &pdt_core::project::Project,
    input: &NewRun,
) -> Result<Option<String>, ApiError> {
    if let Some(w) = &input.worktree_id {
        return Ok(Some(w.clone()));
    }
    let Some(branch) = input.branch.as_deref().map(str::trim).filter(|b| !b.is_empty()) else { return Ok(None) };
    if let Some(wt) = st.store.list_worktrees(&project.id).await?.into_iter().find(|w| w.branch == branch) {
        return Ok(Some(wt.id));
    }
    let (_, main_branch) = crate::runconfig::checkout_dir(st, project, None).await?;
    if main_branch == branch {
        return Ok(None);
    }
    let repo = PathBuf::from(&project.repo_path);
    let path = worktree_path(&repo, branch);
    let (b, target) = (branch.to_string(), path.clone());
    let base_commit = tokio::task::spawn_blocking(move || {
        pdt_git::ensure_excluded(&repo, &format!("/{WORKTREES_DIR}/"))?;
        pdt_git::add_worktree_for_branch(&repo, &target, &b)
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
    };
    st.store.insert_worktree(&wt).await?;
    Ok(Some(wt.id))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    Path(project_id): Path<String>,
    payload: Result<Json<NewRun>, JsonRejection>,
) -> Result<(StatusCode, Json<Run>), ApiError> {
    let Json(input) = payload?;
    let project = st.store.get_project(&project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let worktree_id = place(&st, &project, &input).await?;
    let (cwd, branch) = crate::runconfig::checkout_dir(&st, &project, worktree_id.as_deref()).await?;
    let cfg = crate::runconfig::load_effective(&st, &project.id, &cwd).await?;
    let list = if input.kind == RunKind::Test { &cfg.tests } else { &cfg.runs };
    let def = list.iter().find(|d| d.name == input.name).cloned().ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, "unknown_run", format!("the configuration has no \"{}\"", input.name))
    })?;
    if input.kind == RunKind::Run {
        let settings = st.store.get_host_settings().await?;
        let active: Vec<Run> =
            st.store.list_active_runs().await?.into_iter().filter(|r| r.kind == RunKind::Run).collect();
        if active.len() as u32 >= settings.max_concurrent_runs {
            match input.stop_run_id.as_deref().filter(|id| active.iter().any(|r| r.id == *id)) {
                Some(id) => run_manager::stop(&st, id, None).await,
                None => {
                    let oldest = &active[0];
                    return Err(conflict(
                        "too_many_runs",
                        format!(
                            "{} runs are active (the maximum is {}). Stop \"{} · {}\" to start this one?",
                            active.len(),
                            settings.max_concurrent_runs,
                            oldest.name,
                            oldest.branch
                        ),
                    ));
                }
            }
        }
    }
    let port =
        if def.web { Some(run_manager::allocate_port(&st, None, None).await.ok_or_else(no_free_port)?) } else { None };
    let data_dir =
        def.data.then(|| run_data_dir(&st.data_dir, &project.id, &branch, &def.name).to_string_lossy().into_owned());
    let now = now_ms();
    let run = Run {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: project.id,
        worktree_id,
        branch,
        name: def.name.clone(),
        kind: input.kind,
        state: if def.setup.is_some() { RunState::SettingUp } else { RunState::Starting },
        port,
        url: port.map(|p| run_manager::run_url(&st, p)),
        data_dir,
        cwd: cwd.to_string_lossy().into_owned(),
        exit_code: None,
        error: None,
        created_at: now,
        updated_at: now,
    };
    st.store.insert_run(&run).await?;
    run_manager::start(&st, run.clone(), def);
    Ok((StatusCode::CREATED, Json(run)))
}

pub(crate) async fn stop(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    load(&st, &id).await?;
    run_manager::stop(&st, &id, None).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn restart(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Run>, ApiError> {
    let run = load(&st, &id).await?;
    if run.state.is_active() {
        run_manager::stop(&st, &id, None).await;
    }
    let def =
        definition_for(&st, &run).await.map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "invalid_run_config", m))?;
    let mut run = load(&st, &id).await?;
    if def.web {
        let p = run_manager::allocate_port(&st, run.port, Some(&id)).await.ok_or_else(no_free_port)?;
        let url = run_manager::run_url(&st, p);
        st.store.set_run_port(&id, Some(p), Some(&url)).await?;
        run.port = Some(p);
        run.url = Some(url);
    }
    let first = if def.setup.is_some() { RunState::SettingUp } else { RunState::Starting };
    st.store.set_run_state(&id, first, now_ms()).await?;
    run.state = first;
    run.exit_code = None;
    run.error = None;
    run_manager::start(&st, run.clone(), def);
    Ok(Json(run))
}

pub(crate) async fn reset_data(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let run = load(&st, &id).await?;
    if run.state.is_active() {
        return Err(conflict("run_active", "stop the run before resetting its data"));
    }
    if let Some(dir) = run.data_dir.map(PathBuf::from) {
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            std::fs::create_dir_all(&dir)
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let run = load(&st, &id).await?;
    if run.state.is_active() {
        return Err(conflict("run_active", "stop the run before removing it"));
    }
    for step in [RunStep::Setup, RunStep::Command] {
        st.ptys.remove(&pty_id(&id, step));
    }
    let _ = std::fs::remove_dir_all(run_manager::scripts_dir(&st, &id));
    st.store.delete_run(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn pty(
    State(st): State<AppState>,
    Path((id, step)): Path<(String, String)>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    load(&st, &id).await?;
    let step = RunStep::parse(&step).ok_or_else(|| ApiError::not_found("run step"))?;
    crate::pty_ws::upgrade(st, pty_id(&id, step), ws)
}
