use crate::error::ApiError;
use crate::{pty_ws, sessions, worktrees};
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use pdt_core::brand::PRODUCT_NAME;
use pdt_core::project::{NewProject, Project};
use pdt_store::Store;
use serde::Serialize;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};
use ts_rs::TS;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub ptys: pdt_agents::PtyManager,
    pub platform: pdt_core::session::Platform,
    /// Exit-recorder task per session, so a restart can wait for the previous run's exit write to land first.
    pub(crate) exit_tasks:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, tokio::task::JoinHandle<()>>>>,
    /// Per-session lock serializing kill, restart and remove, so concurrent requests cannot interleave.
    pub(crate) session_locks:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>>,
    /// Notified by POST /api/shutdown; pdtd's serve loop waits on it.
    pub shutdown: std::sync::Arc<tokio::sync::Notify>,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            ptys: pdt_agents::PtyManager::new(),
            platform: pdt_agents::launch::detect_platform(),
            exit_tasks: Default::default(),
            session_locks: Default::default(),
            shutdown: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }
}

impl AppState {
    /// Takes the session's lock. The map's std mutex is only held to look the lock up, never across an await.
    pub(crate) async fn lock_session(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock =
            self.session_locks.lock().unwrap_or_else(|e| e.into_inner()).entry(id.to_string()).or_default().clone();
        lock.lock_owned().await
    }

    pub(crate) fn forget_session_lock(&self, id: &str) {
        self.session_locks.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }
}

pub(crate) fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Health {
    pub status: String,
    pub name: String,
    pub version: String,
}

/// Asks pdtd to stop gracefully (running sessions become `interrupted`). Guarded by the Host/Origin checks.
async fn request_shutdown(State(st): State<AppState>) -> StatusCode {
    tracing::info!("shutdown requested over the API");
    st.shutdown.notify_one();
    StatusCode::ACCEPTED
}

pub fn router(state: AppState, web_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/shutdown", axum::routing::post(request_shutdown))
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{id}", get(get_project))
        .route("/projects/{id}/worktrees", get(worktrees::list).post(worktrees::create))
        .route("/projects/{id}/sessions", get(sessions::list))
        .route("/sessions", axum::routing::post(sessions::create))
        .route("/sessions/{id}", get(sessions::get_one).patch(sessions::rename).delete(sessions::remove))
        .route("/sessions/{id}/kill", axum::routing::post(sessions::kill))
        .route("/sessions/{id}/restart", axum::routing::post(sessions::restart))
        .route("/sessions/{id}/pty", get(pty_ws::connect))
        .fallback(|| async { ApiError::not_found("route") })
        .method_not_allowed_fallback(|| async {
            ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed", "method not allowed")
        })
        .with_state(state);
    let app = Router::new().nest("/api", api);
    match web_dir {
        Some(dir) => {
            let index = dir.join("index.html");
            app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => app,
    }
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok".into(), name: PRODUCT_NAME.into(), version: env!("CARGO_PKG_VERSION").into() })
}

async fn list_projects(State(st): State<AppState>) -> Result<Json<Vec<Project>>, ApiError> {
    Ok(Json(st.store.list_projects().await?))
}

async fn get_project(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Project>, ApiError> {
    st.store.get_project(&id).await?.map(Json).ok_or_else(|| ApiError::not_found("project"))
}

async fn create_project(
    State(st): State<AppState>,
    payload: Result<Json<NewProject>, JsonRejection>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let Json(input) = payload?;
    let valid = input.validate()?;
    let path = PathBuf::from(&valid.repo_path);
    let info =
        tokio::task::spawn_blocking(move || pdt_git::resolve_repo(&path)).await.map_err(ApiError::internal)??;
    let project = Project {
        id: uuid::Uuid::now_v7().to_string(),
        name: valid.name,
        repo_path: info.root.to_string_lossy().into_owned(),
        base_branch: valid.base_branch.or(info.current_branch).unwrap_or_else(|| "main".into()),
        created_at: now_ms(),
    };
    st.store.insert_project(&project).await.map_err(|e| match e {
        pdt_store::StoreError::Conflict(_) => {
            ApiError::new(StatusCode::CONFLICT, "conflict", "a project for this repository already exists")
        }
        other => other.into(),
    })?;
    Ok((StatusCode::CREATED, Json(project)))
}
