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
    /// Minimum gap between two fresh Codex launches in one folder. Codex's conversations are matched to sessions by
    /// start time, so launches this close together would be ambiguous.
    pub codex_spawn_spacing: std::time::Duration,
    /// When the last fresh Codex launch in each folder (see `folder_key`) happened, in ms.
    pub(crate) codex_spawns: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, i64>>>,
    /// Set when pdtd starts shutting down, so a background auto-resume stops launching sessions.
    pub shutting_down: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Sessions the startup auto-resume has yet to finish with (restarted, skipped or failed). A terminal that
    /// connects to one of them waits for the outcome instead of being told it is not running.
    pub(crate) pending_resume: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// Bumped whenever `pending_resume` shrinks or shutdown starts, to wake those terminals.
    pub(crate) resume_changed: std::sync::Arc<tokio::sync::watch::Sender<u64>>,
    /// Notified by POST /api/shutdown; pdtd's serve loop waits on it.
    pub shutdown: std::sync::Arc<tokio::sync::Notify>,
    /// pdtd's data directory (runs, per-session agent settings, update downloads).
    pub data_dir: std::path::PathBuf,
    /// Where secrets live. pdtd sets the keychain store; `new` uses an in-memory one (tests).
    pub secrets: std::sync::Arc<dyn crate::secrets::SecretStore>,
    /// `PDT_OWNER_LOGIN`: when set, it is the owner and Settings shows it read-only.
    pub owner_from_env: Option<String>,
    /// Serializes read-modify-write of host settings and the side effects applied with them.
    pub(crate) settings_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// The effective owner (`PDT_OWNER_LOGIN`, else the stored setting) the identity guard checks; see `refresh_owner`.
    pub(crate) owner_cache: std::sync::Arc<std::sync::RwLock<Option<String>>>,
    /// The port pdtd listens on (0 in tests that do not serve). Tailscale maps 443 to it.
    pub listen_port: u16,
    /// The live Host/Origin allowlist (the guard and remote access share it).
    pub allowed_hosts: crate::host_guard::AllowedHosts,
    /// The Tailscale CLI. `new` installs [`pdt_tailscale::Unavailable`], which runs nothing; only pdtd's production
    /// wiring installs the real CLI, so tests can never touch the machine's Tailscale (it serves the stable instance).
    pub tailscale: std::sync::Arc<dyn pdt_tailscale::TailscaleApi>,
    /// The HTTPS port remote access claims on the tailnet name: 443 for the stable instance (the default), the
    /// instance's own port for any other, so a dev instance never takes over the stable one's mapping.
    pub remote_https_port: u16,
    /// The longest one tailscale command may take; a hung CLI then counts as an error.
    pub tailscale_timeout: std::time::Duration,
    /// The last result of applying the remote-access setting.
    pub remote: std::sync::Arc<std::sync::Mutex<crate::remote::RemoteStatus>>,
    /// What remote access changed in this process (mapped ports, the allowlist entry). Turning it off undoes only
    /// these, never mappings another instance (e.g. the stable one) made.
    pub(crate) remote_applied: std::sync::Arc<std::sync::Mutex<crate::remote::Applied>>,
    /// The driver of each active run in this daemon run (stop requests, completion).
    pub(crate) run_controls: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<crate::run_manager::RunControl>>>,
    >,
    /// Serializes run starts (limit check, port choice, worktree and row creation), so concurrent starts can neither
    /// exceed the run limit nor pick the same port.
    pub(crate) run_start_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// How long stopping a run waits for its processes to end before reporting `run_stopping`.
    pub run_stop_wait: std::time::Duration,
    /// The program agent hooks run (`<this pdtd> hook …`).
    pub hook_program: std::path::PathBuf,
    /// The activity tracker of each agent session running in this daemon run.
    pub(crate) activity: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, crate::activity::Tracker>>>,
    /// Per-session ordering point for activity transitions and their store writes.
    pub(crate) activity_order:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>>,
    /// The thresholds of the time-based activity signals (tests shorten them).
    pub activity_timing: pdt_core::activity::Timing,
    /// How often `run_activity_ticker` checks them.
    pub activity_tick: std::time::Duration,
    /// Update checks and installs. `new` installs a disabled (unsupervised) service; pdtd replaces it.
    pub update: std::sync::Arc<crate::update::UpdateService>,
}

impl AppState {
    pub fn new(store: Store, data_dir: std::path::PathBuf) -> Self {
        Self {
            store,
            ptys: pdt_agents::PtyManager::new(),
            platform: pdt_agents::launch::detect_platform(),
            exit_tasks: Default::default(),
            session_locks: Default::default(),
            codex_spawn_spacing: std::time::Duration::from_secs(5),
            codex_spawns: Default::default(),
            shutting_down: Default::default(),
            pending_resume: Default::default(),
            resume_changed: std::sync::Arc::new(tokio::sync::watch::channel(0).0),
            shutdown: std::sync::Arc::new(tokio::sync::Notify::new()),
            data_dir,
            secrets: std::sync::Arc::new(crate::secrets::MemorySecretStore::default()),
            owner_from_env: None,
            settings_lock: Default::default(),
            owner_cache: Default::default(),
            listen_port: 0,
            allowed_hosts: Default::default(),
            tailscale: std::sync::Arc::new(pdt_tailscale::Unavailable),
            remote_https_port: crate::remote::HOST_HTTPS_PORT,
            tailscale_timeout: pdt_tailscale::DEFAULT_TIMEOUT,
            remote: Default::default(),
            remote_applied: Default::default(),
            run_controls: Default::default(),
            run_start_lock: Default::default(),
            run_stop_wait: std::time::Duration::from_secs(20),
            hook_program: std::env::current_exe().unwrap_or_default(),
            activity: Default::default(),
            activity_order: Default::default(),
            activity_timing: Default::default(),
            activity_tick: std::time::Duration::from_secs(5),
            update: std::sync::Arc::new(crate::update::UpdateService::new(
                pdt_update::GITHUB_API,
                pdt_core::brand::RELEASE_REPO,
                false,
                pdt_core::BUILD_VERSION,
            )),
        }
    }
}

impl AppState {
    /// Reloads the effective owner (`PDT_OWNER_LOGIN`, else the stored setting) used by the identity guard.
    pub async fn refresh_owner(&self) {
        let stored = self.store.get_host_settings().await.ok().and_then(|s| s.owner_login);
        *self.owner_cache.write().unwrap_or_else(|e| e.into_inner()) = self.owner_from_env.clone().or(stored);
    }

    pub(crate) fn owner(&self) -> Option<String> {
        self.owner_cache.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Takes the session's lock. The map's std mutex is only held to look the lock up, never across an await.
    pub(crate) async fn lock_session(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock =
            self.session_locks.lock().unwrap_or_else(|e| e.into_inner()).entry(id.to_string()).or_default().clone();
        lock.lock_owned().await
    }

    pub(crate) fn resume_pending(&self, id: &str) -> bool {
        self.pending_resume.lock().unwrap_or_else(|e| e.into_inner()).contains(id)
    }

    /// The auto-resume is done with this session, whatever the outcome.
    pub(crate) fn finish_resume(&self, id: &str) {
        self.pending_resume.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        self.wake_resume_waiters();
    }

    /// Wakes terminals waiting for an auto-resume to re-check (also used when shutdown starts).
    pub fn wake_resume_waiters(&self) {
        self.resume_changed.send_modify(|v| *v += 1);
    }

    /// Kills a terminal off the async runtime: on Windows this waits for `taskkill` to end the process tree.
    pub(crate) async fn kill_pty(&self, id: &str) -> Result<(), pdt_agents::PtyError> {
        let (ptys, id) = (self.ptys.clone(), id.to_string());
        tokio::task::spawn_blocking(move || ptys.kill(&id))
            .await
            .unwrap_or_else(|e| Err(pdt_agents::PtyError::Io(e.to_string())))
    }

    /// Forgets a terminal (killing it if it still runs) off the async runtime, like [`AppState::kill_pty`].
    pub(crate) async fn remove_pty(&self, id: &str) {
        let (ptys, id) = (self.ptys.clone(), id.to_string());
        let _ = tokio::task::spawn_blocking(move || ptys.remove(&id)).await;
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
    let guard_state = state.clone();
    let api = Router::new()
        .route("/health", get(health))
        .route("/shutdown", axum::routing::post(request_shutdown))
        .route("/host/settings", get(crate::host::get_settings).put(crate::host::put_settings))
        .route("/host/whoami", get(crate::identity_guard::whoami))
        .route("/host/remote", get(crate::remote::get_remote))
        .route("/host/remote/refresh", axum::routing::post(crate::remote::refresh))
        .route("/host/update", get(crate::update::get_status))
        .route("/host/update/check", axum::routing::post(crate::update::post_check))
        .route("/host/update/install", axum::routing::post(crate::update::post_install))
        .route("/host/update/dismiss", axum::routing::post(crate::update::post_dismiss))
        .route("/host/token", axum::routing::put(crate::update::put_token).delete(crate::update::delete_token))
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{id}", get(get_project))
        .route(
            "/projects/{id}/settings",
            get(crate::project_settings::get_settings).put(crate::project_settings::put_settings),
        )
        .route("/projects/{id}/run-config", get(crate::runconfig::get_config))
        .route("/projects/{id}/worktrees", get(worktrees::list).post(worktrees::create))
        .route("/projects/{id}/sessions", get(sessions::list))
        .route("/projects/{id}/runs", get(crate::runs::list).post(crate::runs::create))
        .route("/projects/{id}/branches", get(crate::runs::branches))
        .route("/runs", get(crate::runs::list_active))
        .route("/runs/{id}", get(crate::runs::get_one).delete(crate::runs::remove))
        .route("/runs/{id}/stop", axum::routing::post(crate::runs::stop))
        .route("/runs/{id}/restart", axum::routing::post(crate::runs::restart))
        .route("/runs/{id}/reset-data", axum::routing::post(crate::runs::reset_data))
        .route("/runs/{id}/pty/{step}", get(crate::runs::pty))
        .route("/sessions", axum::routing::post(sessions::create))
        .route("/sessions/{id}", get(sessions::get_one).patch(sessions::rename).delete(sessions::remove))
        .route("/sessions/{id}/kill", axum::routing::post(sessions::kill))
        .route("/sessions/{id}/restart", axum::routing::post(sessions::restart))
        .route("/sessions/{id}/pty", get(pty_ws::connect))
        .route("/sessions/{id}/activity", axum::routing::post(crate::activity::post_report))
        .route("/sessions/{id}/view", axum::routing::post(crate::activity::post_view))
        .fallback(|| async { ApiError::not_found("route") })
        .method_not_allowed_fallback(|| async {
            ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed", "method not allowed")
        })
        .with_state(state);
    let app = Router::new().nest("/api", api);
    let app = match web_dir {
        Some(dir) => {
            let index = dir.join("index.html");
            app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => app,
    };
    app.layer(axum::middleware::from_fn_with_state(guard_state, crate::identity_guard::guard))
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok".into(), name: PRODUCT_NAME.into(), version: pdt_core::BUILD_VERSION.into() })
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
