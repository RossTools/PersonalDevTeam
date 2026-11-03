//! Run routes (spec §4.2). The lifecycle itself lives in `run_manager`.

use crate::error::{ApiError, OrNotFound, codes};
use crate::routes::{AppState, now_ms};
use crate::run_manager;
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use pdt_core::brand::PROJECT_CONFIG_FILE;
use pdt_core::project::Project;
use pdt_core::run::{NewRun, RestartRun, Run, RunKind, RunState, RunStep, pty_id, run_data_dir};
use pdt_core::runconfig::{RunConfig, RunDef};
use pdt_core::workspace::TabPlacement;
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
    st.store.get_run(id).await?.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, codes::UNKNOWN_RUN, "run not found"))
}

/// The definition named by `run` in the configuration of its own checkout. Err is a message for the run's error.
pub(crate) async fn definition_for(st: &AppState, run: &Run) -> Result<RunDef, String> {
    let cfg = crate::runconfig::load_effective(st, &run.project_id, std::path::Path::new(&run.cwd))
        .await
        .map_err(|e| format!("the run configuration is invalid: {}", e.message()))?;
    if !cfg.ignored.is_empty() {
        tracing::info!(run = %run.name, cwd = %run.cwd, sections = ?cfg.ignored, "skipped run configuration sections this version does not know");
    }
    let list = if run.kind == RunKind::Test { cfg.tests } else { cfg.runs };
    list.into_iter().find(|d| d.name == run.name).ok_or_else(|| format!("the configuration has no \"{}\"", run.name))
}

pub(crate) async fn list(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<Run>>, ApiError> {
    st.project(&id).await?;
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
    let p = st.project(&id).await?;
    if !p.has_repo() {
        return Ok(Json(vec![]));
    }
    let repo = PathBuf::from(p.repo_path);
    let list =
        tokio::task::spawn_blocking(move || pdt_git::list_branches(&repo)).await.map_err(ApiError::internal)??;
    Ok(Json(list.into_iter().map(|b| Branch { name: b.name, local: b.local }).collect()))
}

/// Where a new run goes.
enum Place {
    Worktree(String),
    Main,
    /// A branch with no worktree yet: one is created once the start is allowed.
    NewBranch(String),
}

/// Resolves where a new run goes without changing anything.
async fn place(st: &AppState, project: &Project, input: &NewRun) -> Result<Place, ApiError> {
    if let Some(w) = &input.worktree_id {
        return Ok(Place::Worktree(w.clone()));
    }
    let Some(branch) = input.branch.as_deref().map(str::trim).filter(|b| !b.is_empty()) else { return Ok(Place::Main) };
    if let Some(wt) = st.store.list_worktrees(&project.id).await?.into_iter().find(|w| w.branch == branch) {
        return Ok(Place::Worktree(wt.id));
    }
    let (_, main_branch) = crate::runconfig::checkout_dir(st, project, None).await?;
    if main_branch == branch {
        return Ok(Place::Main);
    }
    Ok(Place::NewBranch(branch.to_string()))
}

/// The effective configuration where the run goes; for a branch without a worktree, read from the branch with git.
async fn config_for(st: &AppState, project: &Project, place: &Place) -> Result<RunConfig, ApiError> {
    let worktree_id = match place {
        Place::Worktree(id) => Some(id.as_str()),
        Place::Main => None,
        Place::NewBranch(branch) => {
            let (repo, b) = (PathBuf::from(&project.repo_path), branch.clone());
            let file = tokio::task::spawn_blocking(move || pdt_git::branch_file(&repo, &b, PROJECT_CONFIG_FILE))
                .await
                .map_err(ApiError::internal)??;
            return crate::runconfig::effective_for_file(st, &project.id, file.as_deref()).await;
        }
    };
    let (cwd, _) = crate::runconfig::checkout_dir(st, project, worktree_id).await?;
    crate::runconfig::load_effective(st, &project.id, &cwd).await
}

/// The worktree for `branch`: an existing one, else a new one at `.worktrees/<branch>`. Callers hold
/// `run_start_lock`.
async fn worktree_for(st: &AppState, project: &Project, branch: &str) -> Result<String, ApiError> {
    let existing = st.store.list_worktrees(&project.id).await?;
    if let Some(wt) = existing.iter().find(|w| w.branch == branch) {
        return Ok(wt.id.clone());
    }
    let repo = PathBuf::from(&project.repo_path);
    let path = worktree_path(&repo, branch);
    let path_str = path.to_string_lossy().into_owned();
    // A worktree made outside PDT (plain `git worktree add`) for this branch is registered as it is.
    let adopted = if path.exists() && !existing.iter().any(|w| w.path == path_str) {
        let (r, p, b) = (repo.clone(), path.clone(), branch.to_string());
        tokio::task::spawn_blocking(move || crate::worktrees::adoptable(&r, &p, &b))
            .await
            .map_err(ApiError::internal)?
    } else {
        None
    };
    if let Some(head) = adopted {
        let wt = Worktree {
            id: uuid::Uuid::now_v7().to_string(),
            project_id: project.id.clone(),
            branch: branch.to_string(),
            path: path_str,
            base_commit: head,
            created_at: now_ms(),
            missing: false,
        };
        st.store.insert_worktree(&wt).await?;
        return Ok(wt.id);
    }
    if path.exists() || existing.iter().any(|w| w.path == path_str) {
        return Err(ApiError::conflict(
            codes::WORKTREE_PATH_TAKEN,
            format!("the worktree folder {path_str} is already in use (by another branch with a similar name)"),
        ));
    }
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
        path: path_str,
        base_commit,
        created_at: now_ms(),
        missing: false,
    };
    st.store.insert_worktree(&wt).await?;
    Ok(wt.id)
}

fn still_stopping() -> ApiError {
    ApiError::conflict(codes::RUN_STOPPING, "the run has not stopped yet; try again in a moment")
}

/// Makes room for one more run under the host's maximum, stopping `stop_run_id` when the user confirmed that.
/// Callers hold `run_start_lock`.
async fn check_run_limit(st: &AppState, stop_run_id: Option<&str>) -> Result<(), ApiError> {
    let settings = st.store.get_host_settings().await?;
    let active: Vec<Run> = st.store.list_active_runs().await?.into_iter().filter(|r| r.kind == RunKind::Run).collect();
    if (active.len() as u32) < settings.max_concurrent_runs {
        return Ok(());
    }
    match stop_run_id.filter(|id| active.iter().any(|r| r.id == *id)) {
        Some(id) if run_manager::stop(st, id, None).await => Ok(()),
        Some(_) => Err(still_stopping()),
        None => {
            let oldest = &active[0];
            Err(ApiError::conflict(
                codes::TOO_MANY_RUNS,
                format!(
                    "{} runs are active (the maximum is {}). Stop \"{} · {}\" to start this one?",
                    active.len(),
                    settings.max_concurrent_runs,
                    oldest.name,
                    oldest.branch
                ),
            ))
        }
    }
}

/// Where the tab of a run started without a placement goes: the workspace of the active attempt working in its
/// worktree or on its branch (PDT-160). None leaves it to the project's workspace.
async fn attempt_placement(
    st: &AppState,
    project_id: &str,
    worktree_id: Option<&str>,
    branch: &str,
) -> Result<Option<TabPlacement>, ApiError> {
    let attempt = st.store.list_active_attempts().await?.into_iter().find(|i| {
        i.project_id == project_id
            && ((worktree_id.is_some() && i.attempt.worktree_id.as_deref() == worktree_id)
                || i.attempt.branch == branch)
    });
    let Some(workspace_id) = attempt.and_then(|i| i.attempt.workspace_id) else { return Ok(None) };
    if st.store.get_workspace(&workspace_id).await?.is_none() {
        return Ok(None);
    }
    Ok(Some(TabPlacement { workspace_id, position: None }))
}

/// Starts a run or test. One that already ran in the same checkout under the same name and has finished starts again
/// in its tab (200); otherwise a new run opens a tab (201): where asked, else in its attempt's workspace, else in its
/// project's.
pub(crate) async fn create(
    State(st): State<AppState>,
    Path(project_id): Path<String>,
    payload: Result<Json<NewRun>, JsonRejection>,
) -> Result<(StatusCode, Json<Run>), ApiError> {
    let Json(input) = payload?;
    let project = st.project(&project_id).await?;
    crate::projects::needs_repo(&project, "runs or tests")?;
    // Everything that can refuse the start comes before anything is created.
    crate::workspaces::check_placement(&st, input.tab.as_ref()).await?;
    let place = place(&st, &project, &input).await?;
    let cfg = config_for(&st, &project, &place).await?;
    let list = if input.kind == RunKind::Test { &cfg.tests } else { &cfg.runs };
    let def = list.iter().find(|d| d.name == input.name).cloned().ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, codes::UNKNOWN_RUN, format!("the configuration has no \"{}\"", input.name))
    })?;
    // A command the user hasn't approved (a branch's or an agent-edited worktree's config, a new clone) waits for them.
    let commands = vec![pdt_core::command_trust::of_run(&def, input.kind == RunKind::Test)];
    crate::command_trust::check(&st, &project.id, &project.repo_path, commands).await?;
    let _start = st.run_start_lock.lock().await;
    if input.kind == RunKind::Run {
        check_run_limit(&st, input.stop_run_id.as_deref()).await?;
    }
    let worktree_id = match &place {
        Place::Worktree(id) => Some(id.clone()),
        Place::Main => None,
        Place::NewBranch(branch) => Some(worktree_for(&st, &project, branch).await?),
    };
    let (cwd, branch) = crate::runconfig::checkout_dir(&st, &project, worktree_id.as_deref()).await?;
    let tab = match input.tab {
        Some(t) => Some(t),
        None => attempt_placement(&st, &project.id, worktree_id.as_deref(), &branch).await?,
    };
    // Starting a run again restarts its finished run in the same checkout, in its tab, instead of adding one (PDT-160).
    let again = st.store.list_runs(&project.id).await?.into_iter().find(|r| {
        r.worktree_id == worktree_id
            && r.branch == branch
            && r.kind == input.kind
            && r.name == def.name
            && !r.state.is_active()
    });
    if let Some(run) = again {
        let item = pdt_core::workspace::TabItem::Run(run.id.clone());
        if st.store.tab_for(&item).await?.is_none() {
            crate::workspaces::open_for_new(&st, &run.project_id, item, tab.as_ref()).await;
        }
        return Ok((StatusCode::OK, Json(run_manager::relaunch(&st, run, def).await?)));
    }
    let port = if def.web {
        Some(run_manager::allocate_port(&st, None, None).await.ok_or_else(run_manager::no_free_port)?)
    } else {
        None
    };
    let data_dir =
        def.data.then(|| run_data_dir(&st.data_dir, &project.id, &branch, &def.name).to_string_lossy().into_owned());
    let now = now_ms();
    // The link and the driver are settled under the lock applying remote access holds (see `run_manager::relaunch`).
    let _link = st.settings_lock.lock().await;
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
        detached: def.detach,
        process: None,
        created_at: now,
        updated_at: now,
    };
    st.store.insert_run(&run).await?;
    let item = pdt_core::workspace::TabItem::Run(run.id.clone());
    crate::workspaces::open_for_new(&st, &run.project_id, item, tab.as_ref()).await;
    run_manager::start(&st, run.clone(), def);
    Ok((StatusCode::CREATED, Json(run)))
}

pub(crate) async fn stop(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    load(&st, &id).await?;
    if !run_manager::stop(&st, &id, None).await {
        return Err(still_stopping());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn restart(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Run>, ApiError> {
    // The body is optional: empty means no stopRunId.
    let input: RestartRun = if body.iter().all(u8::is_ascii_whitespace) {
        RestartRun::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| ApiError::invalid(e.to_string()))?
    };
    let run = load(&st, &id).await?;
    let was_active = run.state.is_active();
    // A command the user hasn't approved refuses the restart before the running one is stopped.
    let def = definition_for(&st, &run).await.map_err(|m| ApiError::bad_request(codes::INVALID_RUN_CONFIG, m))?;
    crate::command_trust::check_run(&st, &run.project_id, &def, run.kind == RunKind::Test).await?;
    if was_active && !run_manager::stop(&st, &id, None).await {
        return Err(still_stopping());
    }
    if was_active && def.web {
        run_manager::await_port_release(run.port).await;
    }
    let _start = st.run_start_lock.lock().await;
    let run = load(&st, &id).await?;
    if run.state.is_active() {
        return Err(ApiError::conflict(codes::RUN_ACTIVE, "the run was started again meanwhile"));
    }
    // A restarted run counts toward the limit like a new one.
    if run.kind == RunKind::Run {
        check_run_limit(&st, input.stop_run_id.as_deref()).await?;
    }
    Ok(Json(run_manager::relaunch(&st, run, def).await?))
}

pub(crate) async fn reset_data(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let run = load(&st, &id).await?;
    if run.state.is_active() {
        return Err(ApiError::conflict(codes::RUN_ACTIVE, "stop the run before resetting its data"));
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

/// Stops a run if it is active and forgets its terminals and scripts (not its data folder). Used when its project
/// goes away. Returns false if it did not stop in time.
pub(crate) async fn discard(st: &AppState, run: &Run) -> bool {
    if run.state.is_active() && !run_manager::stop(st, &run.id, None).await {
        return false;
    }
    for step in [RunStep::Setup, RunStep::Command] {
        st.remove_pty(&pty_id(&run.id, step)).await;
    }
    let _ = std::fs::remove_dir_all(run_manager::scripts_dir(st, &run.id));
    let _ = std::fs::remove_file(run_manager::log_path(st, &run.id));
    true
}

pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let run = load(&st, &id).await?;
    if run.state.is_active() {
        return Err(ApiError::conflict(codes::RUN_ACTIVE, "stop the run before removing it"));
    }
    delete(&st, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete(st: &AppState, id: &str) -> Result<(), ApiError> {
    for step in [RunStep::Setup, RunStep::Command] {
        st.remove_pty(&pty_id(id, step)).await;
    }
    let _ = std::fs::remove_dir_all(run_manager::scripts_dir(st, id));
    let _ = std::fs::remove_file(run_manager::log_path(st, id));
    st.store.delete_run(id).await?;
    Ok(())
}

/// Stops a run if it is active, then deletes it (closing its tab, PDT-28). A run that is already gone is fine.
pub(crate) async fn stop_and_remove(st: &AppState, id: &str) -> Result<(), ApiError> {
    let Some(run) = st.store.get_run(id).await? else { return Ok(()) };
    if run.state.is_active() && !run_manager::stop(st, id, None).await {
        return Err(still_stopping());
    }
    delete(st, id).await
}

pub(crate) async fn pty(
    State(st): State<AppState>,
    Path((id, step)): Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<crate::pty_ws::PtyParams>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    load(&st, &id).await?;
    let step = RunStep::parse(&step).or_404("run step")?;
    crate::pty_ws::upgrade(st, pty_id(&id, step), params.gzip_replay(), ws)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/projects/{id}/runs", get(list).post(create))
        .route("/projects/{id}/branches", get(branches))
        .route("/runs", get(list_active))
        .route("/runs/{id}", get(get_one).delete(remove))
        .route("/runs/{id}/stop", post(stop))
        .route("/runs/{id}/restart", post(restart))
        .route("/runs/{id}/reset-data", post(reset_data))
        .route("/runs/{id}/pty/{step}", get(pty))
}
