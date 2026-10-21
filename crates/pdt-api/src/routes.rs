use crate::error::ApiError;
use crate::{pty_ws, sessions, worktrees};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use pdt_core::brand::PRODUCT_NAME;
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
    /// The GitHub CLI for attempts' pull requests. `new` installs [`pdt_github::Unavailable`], which runs nothing;
    /// only pdtd's production wiring installs the real CLI, so tests never reach GitHub.
    pub github: std::sync::Arc<dyn pdt_github::GithubApi>,
    /// Each attempt's pull request as last seen, and the notices waiting for their agent session.
    pub(crate) pr_watch: crate::github::SharedPrWatch,
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
    /// Serializes ticket Starts up to the attempt row (checks, worktree, attempt), so two Starts of one ticket can't
    /// both make a worktree.
    pub(crate) ticket_start_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// Each active attempt's changed files, refreshed by `refresh_overlaps` (actual overlap, tickets spec §7).
    pub(crate) changed_files: crate::overlap::ChangedFiles,
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
    /// The keychain account of this instance's update token (see `secrets::update_token_account`). `new` derives it
    /// from `data_dir` alone, so it is never the stable instance's account; pdtd sets the real one.
    pub update_token_account: String,
    /// The last host checks (tool and login checks); the first `GET /host/checks` runs them.
    pub(crate) checks: std::sync::Arc<std::sync::Mutex<Option<crate::checks::HostChecks>>>,
    /// Which instance this is (profile, port, dev), reported by `/api/health`. `new` makes it a dev instance on port
    /// 0; pdtd sets the real one.
    pub instance: pdt_core::instance::InstanceInfo,
    /// The home folder the folder picker starts in and writes as `~`. `new` reads it from the OS; tests set it.
    pub home_dir: Option<std::path::PathBuf>,
    /// Conversation sessions' runtimes (structured agents).
    pub conversations: crate::conversations::Conversations,
    /// Serializes the workflow engine's ticks and the commands that change runs.
    pub(crate) engine_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// The built-in workflows were installed or updated in this daemon run.
    pub(crate) builtins_installed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// pdt-mcp's host key (`<data dir>/mcp.key`), once read.
    pub(crate) mcp_key: std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>>,
    /// The built-in personas were installed or updated in this daemon run.
    pub(crate) personas_installed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Tickets auto-start failed to start, with their `updated_at` then: skipped until they change.
    pub(crate) auto_start_failed: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, i64>>>,
    /// How old a PM session gets before it rotates (PDT-81; tests shorten it).
    pub pm_rotate_age_ms: i64,
}

impl AppState {
    pub fn new(store: Store, data_dir: std::path::PathBuf) -> Self {
        let update_token_account = crate::secrets::update_token_account(&data_dir, None);
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
            github: std::sync::Arc::new(pdt_github::Unavailable),
            pr_watch: Default::default(),
            remote_https_port: crate::remote::HOST_HTTPS_PORT,
            tailscale_timeout: pdt_tailscale::DEFAULT_TIMEOUT,
            remote: Default::default(),
            remote_applied: Default::default(),
            run_controls: Default::default(),
            run_start_lock: Default::default(),
            ticket_start_lock: Default::default(),
            changed_files: Default::default(),
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
            update_token_account,
            checks: Default::default(),
            instance: pdt_core::instance::InstanceInfo::new(pdt_core::brand::Profile::Dev, 0),
            home_dir: std::env::home_dir(),
            conversations: Default::default(),
            engine_lock: Default::default(),
            auto_start_failed: Default::default(),
            builtins_installed: Default::default(),
            mcp_key: Default::default(),
            personas_installed: Default::default(),
            pm_rotate_age_ms: pdt_core::pm::ROTATE_AGE_MS,
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
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Health {
    pub status: String,
    pub name: String,
    pub version: String,
    pub profile: pdt_core::brand::Profile,
    pub port: u16,
    pub dev: bool,
    /// A short hash of the index.html pdtd serves now, or null without a web dir. Every dev build reports the same
    /// `version`, so the UI reloads itself when this changes (a dev instance restarted with a rebuilt web/dist).
    pub web_build: Option<String>,
}

/// Asks pdtd to stop gracefully (running sessions become `interrupted`). Guarded by the Host/Origin checks.
async fn request_shutdown(State(st): State<AppState>) -> StatusCode {
    tracing::info!("shutdown requested over the API");
    st.shutdown.notify_one();
    StatusCode::ACCEPTED
}

pub fn router(state: AppState, web_dir: Option<PathBuf>) -> Router {
    let guard_state = state.clone();
    let index = web_dir.as_ref().map(|d| d.join("index.html"));
    let api = Router::new()
        .route("/health", get(move |st: State<AppState>| health(st, index.clone())))
        .route("/shutdown", axum::routing::post(request_shutdown))
        .route("/host/settings", get(crate::host::get_settings).put(crate::host::put_settings))
        .route("/host/whoami", get(crate::identity_guard::whoami))
        .route("/host/checks", get(crate::checks::get_checks))
        .route("/host/checks/refresh", axum::routing::post(crate::checks::refresh_checks))
        .route("/host/remote", get(crate::remote::get_remote))
        .route("/host/remote/refresh", axum::routing::post(crate::remote::refresh))
        .route("/host/update", get(crate::update::get_status))
        .route("/host/update/check", axum::routing::post(crate::update::post_check))
        .route("/host/update/install", axum::routing::post(crate::update::post_install))
        .route("/host/update/dismiss", axum::routing::post(crate::update::post_dismiss))
        .route("/host/token", axum::routing::put(crate::update::put_token).delete(crate::update::delete_token))
        .route("/host/folders", get(crate::folders::list).post(crate::folders::create))
        .route("/guides", get(crate::guides::list))
        .route("/guides/{name}", get(crate::guides::get_one))
        .route("/projects", get(crate::projects::list).post(crate::projects::create))
        .route("/projects/clone", axum::routing::post(crate::projects::clone))
        .route(
            "/projects/{id}",
            get(crate::projects::get_one).patch(crate::projects::update).delete(crate::projects::remove),
        )
        .route("/projects/{id}/pull", axum::routing::post(crate::projects::pull))
        .route("/projects/{id}/files", get(crate::files::list))
        .route("/projects/{id}/files/content", get(crate::files::content))
        .route("/worktrees/{id}", axum::routing::delete(worktrees::remove))
        .route("/worktrees/{id}/changes", get(crate::review::changes))
        .route("/worktrees/{id}/changes/file", get(crate::review::file))
        .route("/worktrees/{id}/comments", get(crate::review::list_comments).post(crate::review::create_comment))
        .route("/worktrees/{id}/feedback", axum::routing::post(crate::review::feedback))
        .route(
            "/review-comments/{id}",
            axum::routing::patch(crate::review::update_comment).delete(crate::review::delete_comment),
        )
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
        .route("/projects/{id}/tickets", axum::routing::post(crate::tickets::create))
        .route("/projects/{id}/labels", get(crate::tickets::labels))
        .route("/projects/{id}/areas", get(crate::overlap::areas))
        .route("/tickets", get(crate::tickets::list))
        .route("/tickets/bulk", axum::routing::post(crate::tickets::bulk))
        .route(
            "/tickets/{id}",
            get(crate::tickets::get_one).patch(crate::tickets::update).delete(crate::tickets::remove),
        )
        .route("/tickets/{id}/rank", axum::routing::post(crate::tickets::rank))
        .route("/tickets/{id}/links", axum::routing::post(crate::tickets::add_link).delete(crate::tickets::remove_link))
        .route("/tickets/{id}/events", get(crate::tickets::events))
        .route("/tickets/{id}/comments", axum::routing::post(crate::tickets::comment))
        .route("/tickets/{id}/parts", axum::routing::post(crate::parts::create))
        .route("/tickets/{id}/start", axum::routing::post(crate::start::start))
        .route("/tickets/{id}/refine", axum::routing::post(crate::tickets::refine))
        .route("/tickets/{id}/check-merge", axum::routing::post(crate::start::check_merge))
        .route("/tickets/{id}/finish", axum::routing::post(crate::finish::finish_route))
        .route("/attempts/{id}/abandon", axum::routing::post(crate::start::abandon))
        .route("/attempts/{id}/approve", axum::routing::post(crate::review::approve))
        .route("/attempts/{id}/merge", get(crate::review::merge_preview).post(crate::review::merge))
        .route("/attempts/{id}/ask-agent", axum::routing::post(crate::review::ask_agent))
        .route("/workspaces", get(crate::workspaces::list).post(crate::workspaces::create))
        .route("/workspaces/{id}", axum::routing::patch(crate::workspaces::update).delete(crate::workspaces::remove))
        .route("/workspaces/{id}/tabs", axum::routing::post(crate::workspaces::open_tab))
        .route("/tabs/{id}", axum::routing::patch(crate::workspaces::update_tab).delete(crate::workspaces::close_tab))
        .route("/sessions", axum::routing::post(sessions::create))
        .route("/sessions/{id}", get(sessions::get_one).patch(sessions::update).delete(sessions::remove))
        .route("/sessions/{id}/events", get(crate::conversations::events))
        .route("/sessions/{id}/messages", axum::routing::post(crate::conversations::post_message))
        .route("/sessions/{id}/requests/{request_id}", axum::routing::post(crate::conversations::post_answer))
        .route("/sessions/{id}/interrupt", axum::routing::post(crate::conversations::post_interrupt))
        .route("/sessions/{id}/ticket-draft", axum::routing::post(crate::conversations::ticket_draft))
        .route("/sessions/{id}/mode", axum::routing::post(crate::conversations::switch_mode))
        .route(
            "/sessions/{id}/attachments",
            axum::routing::post(crate::attachments::upload)
                // Room for the largest image; anything bigger is refused with a clear error.
                .layer(axum::extract::DefaultBodyLimit::max(pdt_core::conversation::composer::MAX_IMAGE_BYTES + 1)),
        )
        .route("/sessions/{id}/attachments/{attachment_id}", get(crate::attachments::get))
        .route("/sessions/{id}/files", get(crate::attachments::files))
        .route(
            "/sessions/{id}/terminal",
            axum::routing::post(crate::conversations::open_terminal).delete(crate::conversations::close_terminal),
        )
        .route("/agents/{agent}/capabilities", get(crate::conversations::capabilities))
        .route("/sessions/{id}/kill", axum::routing::post(sessions::kill))
        .route("/sessions/{id}/restart", axum::routing::post(sessions::restart))
        .route("/sessions/{id}/pty", get(pty_ws::connect))
        .route("/sessions/{id}/activity", axum::routing::post(crate::activity::post_report))
        .route("/sessions/{id}/view", axum::routing::post(crate::activity::post_view))
        .merge(crate::workflow::routes::router())
        .route(
            "/mcp",
            axum::routing::post(crate::mcp::post).get(crate::mcp::not_allowed).delete(crate::mcp::not_allowed),
        )
        .route("/personas", get(crate::personas::list).post(crate::personas::create))
        .route(
            "/personas/{id}",
            get(crate::personas::get_one)
                .put(crate::personas::save)
                .patch(crate::personas::patch)
                .delete(crate::personas::remove),
        )
        .route("/personas/{id}/versions", get(crate::personas::versions))
        .route("/personas/{id}/versions/{v}", get(crate::personas::version))
        .route("/personas/{id}/diff", get(crate::personas::diff))
        .route("/pm", get(crate::pm::state))
        .route("/pm/sessions", axum::routing::post(crate::pm::start))
        .route("/pm/notes", get(crate::pm::list_notes).post(crate::pm::create_note))
        .route("/pm/notes/{id}", axum::routing::patch(crate::pm::patch_note).delete(crate::pm::delete_note))
        .route("/inbox", get(crate::inbox::list))
        .route("/inbox/{id}/answer", axum::routing::post(crate::inbox::answer))
        .route("/inbox/{id}/dismiss", axum::routing::post(crate::inbox::dismiss))
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
                .layer(axum::middleware::from_fn(web_cache_headers))
        }
        None => app,
    };
    // Over a remote link (Tailscale) the UI bundle and the JSON polls are worth compressing; WebSocket upgrades and
    // tiny bodies are left alone. Quality 4 keeps it fast on every response.
    let compression =
        tower_http::compression::CompressionLayer::new().quality(tower_http::compression::CompressionLevel::Precise(4));
    app.layer(compression)
        .layer(axum::middleware::from_fn_with_state(guard_state, crate::identity_guard::guard))
        .layer(axum::middleware::from_fn(crate::slow::log_slow))
}

/// Vite names every file in `/assets/` by its content hash, so browsers may keep them for good; everything else the
/// web dir serves is index.html (for any app route), which must be revalidated so a new release is picked up.
async fn web_cache_headers(req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    let path = req.uri().path().to_owned();
    let mut res = next.run(req).await;
    if path.starts_with("/api/") || !res.status().is_success() {
        return res;
    }
    let value = if path.starts_with("/assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    res.headers_mut().insert(axum::http::header::CACHE_CONTROL, axum::http::HeaderValue::from_static(value));
    res
}

async fn health(State(st): State<AppState>, index: Option<PathBuf>) -> Json<Health> {
    let i = st.instance;
    // Read on every call: the web dir can be rebuilt while pdtd runs, and ServeDir serves the new files at once.
    let web_build = match index {
        Some(path) => tokio::fs::read(path).await.ok().map(|bytes| web_build_id(&bytes)),
        None => None,
    };
    Json(Health {
        status: "ok".into(),
        name: PRODUCT_NAME.into(),
        version: pdt_core::BUILD_VERSION.into(),
        profile: i.profile,
        port: i.port,
        dev: i.dev,
        web_build,
    })
}

/// The first 12 hex digits of index.html's SHA-256. Vite names every asset by its content hash and index.html links
/// them, so any change to the bundle changes it.
fn web_build_id(index_html: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(index_html).iter().take(6).map(|b| format!("{b:02x}")).collect()
}
