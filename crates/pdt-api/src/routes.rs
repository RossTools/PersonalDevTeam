use crate::error::{ApiError, codes};
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
    /// Refused webhook requests per hook name, to slow guessing a secret.
    pub(crate) hook_failures: std::sync::Arc<crate::hooks::HookFailures>,
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
    /// Which store `secrets` is (Settings → Secrets shows it).
    pub secret_store: crate::secrets::SecretStoreInfo,
    /// `PDT_OWNER_LOGIN`: when set, it is the owner and Settings shows it read-only.
    pub owner_from_env: Option<String>,
    /// Serializes read-modify-write of host settings and the side effects applied with them.
    pub(crate) settings_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// Serializes read-modify-write of Get started's state (`crate::setup`).
    pub(crate) setup_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// The effective owner (`PDT_OWNER_LOGIN`, else the stored setting) the identity guard checks; see `refresh_owner`.
    pub(crate) owner_cache: std::sync::Arc<std::sync::RwLock<Option<String>>>,
    /// The port pdtd listens on (0 in tests that do not serve). Tailscale maps 443 to it.
    pub listen_port: u16,
    /// What overrides Settings → Network and the agent programs from the environment, Network as it was when pdtd
    /// started, and whether pdt-host supervises it (so Restart now can work): PDT-231.
    pub host_env: std::sync::Arc<crate::host::HostEnv>,
    /// The live Host/Origin allowlist (the guard and remote access share it).
    pub allowed_hosts: crate::host_guard::AllowedHosts,
    /// The Tailscale CLI. `new` installs [`pdt_tailscale::Unavailable`], which runs nothing; only pdtd's production
    /// wiring installs the real CLI, so tests can never touch the machine's Tailscale (it serves the stable instance).
    pub tailscale: std::sync::Arc<dyn pdt_tailscale::TailscaleApi>,
    /// pdt-host's login item (Settings → This host). `new` installs [`crate::host_service::Unavailable`], which
    /// changes nothing; only pdtd's production wiring installs the real one.
    pub start_at_login: std::sync::Arc<dyn crate::host_service::StartAtLogin>,
    /// The forge plugins (PDT-241), which `crate::forge::project_forge` asks for a project's forge (a plugin counts
    /// while the integration that declares its kind is enabled, [`crate::Integration::forge_kind`]). `new` has GitHub
    /// with no network ([`pdt_github::GithubApi::offline`]); only pdtd's production wiring installs one that reaches
    /// GitHub; a test sets a fake (`pdt_forge::HostedForge`). The forge logins sign the plugins in (PDT-244).
    pub forges: pdt_forge::ForgeRegistry,
    /// PDT's own document providers (PDT-314); the enabled integrations add theirs (`crate::documents::providers`).
    /// `new` has the local one ([`AppState::local`]); a test may set a fake
    /// (`crate::documents::testing::FakeProvider`).
    pub documents: crate::documents::DocumentProviders,
    /// PDT's local document provider (PDT-315): files in the documents folder, live sheets. It counts while
    /// `documents` lists it.
    pub local: std::sync::Arc<crate::documents::local::LocalProvider>,
    /// The keychain account of this instance's forge logins (see `secrets::forge_logins_account`), kept apart from
    /// the stable instance's as `update_token_account` is.
    pub forge_logins_account: String,
    /// The keychain account of this instance's GitHub login from before PDT-244 (`secrets::github_login_account`):
    /// copied into the forge logins on their first read.
    pub github_account: String,
    /// The OAuth App the device flow signs in with ([`pdt_core::brand::GITHUB_OAUTH_CLIENT_ID`], or
    /// `PDT_GITHUB_CLIENT_ID`); empty: only a pasted token.
    pub github_client_id: String,
    /// The OAuth App the device flow signs in with on another forge's public host, by forge kind (GitLab's from
    /// `PDT_GITLAB_CLIENT_ID`: PDT has registered none on gitlab.com yet); none: a pasted token, or the user's own app.
    pub forge_client_ids: std::collections::HashMap<String, String>,
    /// Each forge's device flow under way, or why its last login failed.
    pub(crate) forge_logins: std::sync::Arc<crate::forge_login::ForgeLogins>,
    /// Each attempt's pull request as last seen, and the notices waiting for their agent session.
    pub(crate) pr_watch: crate::github::SharedPrWatch,
    /// The local forge's CI runs in this pdtd (PDT-178).
    pub(crate) local_ci: std::sync::Arc<crate::local_forge::LocalCiState>,
    /// The user's yes to the PM's calls that may let an external ticket in (PDT-201).
    pub(crate) grants: std::sync::Arc<crate::untrusted::Grants>,
    /// Asking-tool calls waiting on their Inbox item for the user's answer (PDT-492).
    pub(crate) confirm_waits: std::sync::Arc<crate::workflow::agent::ConfirmWaits>,
    /// This pdtd's UI token (PDT-204): PDT's web client sends it, an agent's curl doesn't.
    pub(crate) ui_token: crate::untrusted::UiToken,
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
    /// How often `run_remote_watch` retries remote access that is not active, and checks it while it is.
    pub remote_watch: crate::remote::RemoteWatch,
    /// The driver of each active run in this daemon run (stop requests, completion).
    pub(crate) run_controls: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<crate::run_manager::RunControl>>>,
    >,
    /// How many times each run was asked to stop in this daemon run; a restart PDT makes on its own is cancelled
    /// when the count moves while it waits (the user's Stop wins).
    pub(crate) run_stops: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
    /// Serializes run starts (limit check, port choice, worktree and row creation), so concurrent starts can neither
    /// exceed the run limit nor pick the same port.
    pub(crate) run_start_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// Serializes a project's ticket Starts up to the attempt row (checks, worktree, attempt), so two Starts of one
    /// ticket can't both make a worktree and the project's cap holds. Keyed by project: one project's Start never
    /// waits for another's (PDT-440).
    pub(crate) ticket_start_locks: std::sync::Arc<crate::keyed_locks::KeyedLocks>,
    /// The finishes going on, per ticket: they run one at a time and spare each other's workflow runs (PDT-182).
    pub(crate) finishes: std::sync::Arc<crate::finish::Finishes>,
    /// Each active attempt's changed files, refreshed by `refresh_overlaps` (actual overlap).
    pub(crate) changed_files: crate::overlap::ChangedFiles,
    /// Parsed run configurations, by checkout (`crate::runconfig::ConfigCache`).
    pub(crate) run_configs: std::sync::Arc<crate::runconfig::ConfigCache>,
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
    /// Serializes the workflow engine's state changes (claiming a step, writing its result, matching events) and
    /// the commands that change runs. Never held while a node runs.
    pub(crate) engine_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// One lock per run, held while the engine advances it (node bodies included) and by the commands that act on
    /// one of its steps, so they never see a step half-run. Taken before `engine_lock`.
    pub(crate) run_locks: std::sync::Arc<crate::keyed_locks::KeyedLocks>,
    /// Steps' results the engine could not write (PDT-323), written once writes work.
    pub(crate) unsaved_results: std::sync::Arc<crate::workflow::engine::UnsavedResults>,
    /// Agent steps start one at a time (the host's running cap), though runs advance side by side.
    pub(crate) agent_step_start: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// How many runs advance at once.
    pub(crate) engine_runs: std::sync::Arc<tokio::sync::Semaphore>,
    /// How long an engine tick waits for the runs it advances before it goes on without them (they keep running).
    /// `new` waits long, so a test's tick ends with its runs; pdtd sets a short wait.
    pub engine_tick_wait: std::time::Duration,
    /// Parsed workflow documents of enabled workflows, for the schedule and matching passes.
    pub(crate) workflow_docs: std::sync::Arc<crate::workflow::engine::DocCache>,
    /// The built-in workflows were installed or updated in this daemon run.
    pub(crate) builtins_installed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// pdt-mcp's host key (`<data dir>/mcp.key`), once read.
    pub(crate) mcp_key: std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>>,
    /// pdt-mcp's `notifications/tools/list_changed`: the sessions' streams' wake-up, and what each was told.
    pub(crate) mcp_tools: std::sync::Arc<crate::mcp::ToolWatch>,
    /// The built-in personas were installed or updated in this daemon run.
    pub(crate) personas_installed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Tickets auto-start failed to start, with their `updated_at` and the error then: skipped until they change, or
    /// until the user's Try again (PDT-381). Start refusing for now (an overlap, the cap, a blocker) is never one.
    pub(crate) auto_start_failed:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, pdt_core::pickup::FailedStart>>>,
    /// Tickets "Pick up tickets" last passed over for overlapping work in flight (PDT-329): the Board shows why while
    /// it is recent. Each choose replaces its projects' entries.
    pub(crate) pickup_holds:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, crate::scheduler::PickupHold>>>,
    /// Held by `ticket.start` while it checks and starts (PDT-327): two pickup runs never start the same ticket or go
    /// over a cap together.
    pub(crate) pickup_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// What the pickup watch saw last (startable tickets, tickets in flight, free agent slots), to log what changed.
    pub(crate) pickup_watch: std::sync::Arc<std::sync::Mutex<crate::scheduler::PickupWatch>>,
    /// Worktrees' sizes as last measured, and the storage loop's state (PDT-324).
    pub(crate) storage: std::sync::Arc<crate::storage::StorageState>,
    /// Reads a disk's free space. `new` reads none (no disk is ever low), so tests never depend on the machine's own
    /// disk; only pdtd's production wiring installs the OS's (`storage::os_disk_space`), and a test sets a fake.
    pub disk_space: crate::storage::DiskProbe,
    /// MCP servers PDT is a client of: their live clients and cached tools.
    pub(crate) mcp: std::sync::Arc<crate::mcp_client::McpPool>,
    /// How long a watch's notes wait for more before they are told, and the longest they wait (PDT-319; tests
    /// shorten them).
    pub watch_wait_ms: (i64, i64),
    /// How long a reset waits for the conversation's journal before it records none (PDT-308).
    pub pm_reset_wait_ms: i64,
    /// The PM conversations a reset is running for (PDT-308): one stays listed in place until its fresh one exists.
    pub(crate) pm_resets: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// Makes and clears notifications (PDT-108), and streams them.
    pub notifier: std::sync::Arc<crate::notifications::Notifier>,
    /// What the Inbox and session sensors remember (PDT-364).
    pub(crate) sensors: std::sync::Arc<crate::workflow::sensors::Sensors>,
    /// The background loops' last passes (PDT-127), for `/api/health` and `host.loop_stale`.
    pub loops: std::sync::Arc<crate::loops::Loops>,
    /// The longest one member of the 60 s poll may take (tests shorten it).
    pub poll_part_limit: std::time::Duration,
    /// The longest one call of a sensor (an integration's check) to its service may take, connecting included (tests
    /// shorten it). A half-open connection then fails the call instead of holding up the poll.
    pub sensor_timeout: std::time::Duration,
    /// How long a session may be unable to take its outbox messages (not idle, yet not working either: stuck at
    /// unknown, waiting on the user) before they are given up on and reported (PDT-129; tests shorten it).
    pub outbox_deadline: std::time::Duration,
    /// How long nobody may have typed into a terminal (counted from when PDT started watching it) before the outbox
    /// types into it (tests shorten it).
    pub typing_quiet: std::time::Duration,
    /// When each session's outbox messages were first seen waiting on it while it was not working (ms).
    pub(crate) outbox_stuck: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, i64>>>,
    /// Bug reports filed in the last minute, by nonce (PDT-153).
    pub(crate) bug_report_nonces: crate::bug_reports::Nonces,
    /// The integrations this build has and the ones this instance enabled (PDT-173). `new` has none of the optional
    /// ones; pdtd (and a test) sets the ones it was built with, then [`crate::integrations::load_enabled`].
    pub integrations: std::sync::Arc<crate::integrations::Integrations>,
    /// Where the programs PDT installs for its ACP profiles come from (PDT-239). `new` has
    /// [`crate::toolchains::Offline`]; only pdtd's production wiring reaches the network, and a test sets a fake.
    pub toolchains: std::sync::Arc<dyn crate::toolchains::ToolchainSource>,
    /// Serializes installs.
    pub(crate) toolchain_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl AppState {
    pub fn new(store: Store, data_dir: std::path::PathBuf) -> Self {
        let update_token_account = crate::secrets::update_token_account(&data_dir, None);
        let github_account = crate::secrets::github_login_account(&data_dir, None);
        let forge_logins_account = crate::secrets::forge_logins_account(&data_dir, None);
        let redactor = store.redactor().clone();
        let local = std::sync::Arc::new(crate::documents::local::LocalProvider::new(
            store.clone(),
            data_dir.clone(),
            std::env::home_dir(),
        ));
        Self {
            store,
            ptys: pdt_agents::PtyManager::new(),
            platform: pdt_agents::launch::detect_platform(),
            exit_tasks: Default::default(),
            session_locks: Default::default(),
            codex_spawn_spacing: std::time::Duration::from_secs(5),
            codex_spawns: Default::default(),
            hook_failures: Default::default(),
            shutting_down: Default::default(),
            pending_resume: Default::default(),
            resume_changed: std::sync::Arc::new(tokio::sync::watch::channel(0).0),
            shutdown: std::sync::Arc::new(tokio::sync::Notify::new()),
            data_dir,
            secrets: std::sync::Arc::new(crate::secrets::Redacting::new(
                std::sync::Arc::new(crate::secrets::MemorySecretStore::default()),
                redactor,
            )),
            secret_store: crate::secrets::SecretStoreInfo {
                backend: crate::secrets::SecretBackend::Memory,
                path: None,
                problem: None,
            },
            owner_from_env: None,
            settings_lock: Default::default(),
            setup_lock: Default::default(),
            owner_cache: Default::default(),
            listen_port: 0,
            host_env: Default::default(),
            allowed_hosts: Default::default(),
            tailscale: std::sync::Arc::new(pdt_tailscale::Unavailable),
            start_at_login: std::sync::Arc::new(crate::host_service::Unavailable(
                "PDT wasn't started by its host supervisor".into(),
            )),
            forges: pdt_forge::ForgeRegistry::new(vec![std::sync::Arc::new(pdt_github::plugin::GithubPlugin::new(
                std::sync::Arc::new(pdt_github::GithubApi::offline()),
            ))]),
            documents: crate::documents::DocumentProviders::new(vec![local.clone()]),
            local,
            forge_logins_account,
            github_account,
            github_client_id: pdt_core::brand::GITHUB_OAUTH_CLIENT_ID.into(),
            forge_client_ids: Default::default(),
            forge_logins: Default::default(),
            pr_watch: Default::default(),
            local_ci: Default::default(),
            grants: Default::default(),
            confirm_waits: Default::default(),
            ui_token: Default::default(),
            remote_https_port: crate::remote::HOST_HTTPS_PORT,
            tailscale_timeout: pdt_tailscale::DEFAULT_TIMEOUT,
            remote: Default::default(),
            remote_applied: Default::default(),
            remote_watch: Default::default(),
            run_controls: Default::default(),
            run_stops: Default::default(),
            run_start_lock: Default::default(),
            ticket_start_locks: Default::default(),
            finishes: Default::default(),
            changed_files: Default::default(),
            run_configs: Default::default(),
            run_stop_wait: std::time::Duration::from_secs(20),
            hook_program: std::env::current_exe().unwrap_or_default(),
            activity: Default::default(),
            activity_order: Default::default(),
            activity_timing: Default::default(),
            activity_tick: std::time::Duration::from_secs(5),
            update: std::sync::Arc::new(
                crate::update::UpdateService::new(
                    pdt_update::GITHUB_API,
                    pdt_core::brand::RELEASE_REPO,
                    false,
                    pdt_core::BUILD_VERSION,
                )
                .with_repo_id(pdt_core::brand::RELEASE_REPO_ID),
            ),
            update_token_account,
            checks: Default::default(),
            instance: pdt_core::instance::InstanceInfo::new(pdt_core::brand::Profile::Dev, 0),
            home_dir: std::env::home_dir(),
            conversations: Default::default(),
            engine_lock: Default::default(),
            run_locks: Default::default(),
            unsaved_results: Default::default(),
            agent_step_start: Default::default(),
            engine_runs: std::sync::Arc::new(tokio::sync::Semaphore::new(crate::workflow::engine::CONCURRENT_RUNS)),
            engine_tick_wait: std::time::Duration::from_secs(10 * 60),
            workflow_docs: Default::default(),
            auto_start_failed: Default::default(),
            pickup_holds: Default::default(),
            pickup_lock: Default::default(),
            pickup_watch: Default::default(),
            storage: Default::default(),
            disk_space: std::sync::Arc::new(|_| None),
            mcp: Default::default(),
            integrations: Default::default(),
            toolchains: std::sync::Arc::new(crate::toolchains::Offline),
            toolchain_lock: Default::default(),
            builtins_installed: Default::default(),
            mcp_key: Default::default(),
            mcp_tools: Default::default(),
            personas_installed: Default::default(),
            watch_wait_ms: (pdt_core::watch::QUIET_MS, pdt_core::watch::MAX_WAIT_MS),
            pm_reset_wait_ms: pdt_core::pm::RESET_WAIT_MS,
            pm_resets: Default::default(),
            notifier: Default::default(),
            sensors: Default::default(),
            loops: Default::default(),
            poll_part_limit: crate::loops::POLL_PART_LIMIT,
            sensor_timeout: std::time::Duration::from_secs(30),
            outbox_deadline: crate::workflow::outbox::DEADLINE,
            typing_quiet: std::time::Duration::from_secs(20),
            outbox_stuck: Default::default(),
            bug_report_nonces: Default::default(),
        }
    }
}

impl AppState {
    /// Reloads the effective owner (`PDT_OWNER_LOGIN`, else the stored setting) used by the identity guard.
    pub async fn refresh_owner(&self) {
        let stored = self.store.get_host_settings().await.ok().and_then(|s| s.owner_login);
        *self.owner_cache.write().unwrap_or_else(|e| e.into_inner()) = self.owner_from_env.clone().or(stored);
    }

    /// How many times a run configuration file has been read; an unchanged one is not read again.
    pub fn run_config_reads(&self) -> u64 {
        self.run_configs.reads()
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

    /// The session's lock if nobody holds it now.
    pub(crate) fn try_lock_session(&self, id: &str) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let lock =
            self.session_locks.lock().unwrap_or_else(|e| e.into_inner()).entry(id.to_string()).or_default().clone();
        lock.try_lock_owned().ok()
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
        // PDT stopped it: its exit is not news (PDT-108).
        if self.ptys.is_running(id) {
            self.notifier.expect_exit(id);
        }
        let (ptys, id) = (self.ptys.clone(), id.to_string());
        tokio::task::spawn_blocking(move || ptys.kill(&id))
            .await
            .unwrap_or_else(|e| Err(pdt_agents::PtyError::Io(e.to_string())))
    }

    /// Forgets a terminal (killing it if it still runs) off the async runtime, like [`AppState::kill_pty`].
    pub(crate) async fn remove_pty(&self, id: &str) {
        if self.ptys.is_running(id) {
            self.notifier.expect_exit(id);
        }
        let (ptys, id) = (self.ptys.clone(), id.to_string());
        let _ = tokio::task::spawn_blocking(move || ptys.remove(&id)).await;
    }

    pub(crate) fn forget_session_lock(&self, id: &str) {
        self.session_locks.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }
}

pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Health {
    pub status: String,
    pub name: String,
    /// The product's website (`brand::WEBSITE`), for links to its docs.
    pub website: String,
    pub version: String,
    pub profile: pdt_core::brand::Profile,
    pub port: u16,
    pub dev: bool,
    /// A short hash of the index.html pdtd serves now, or null without a web dir. Every dev build reports the same
    /// `version`, so the UI reloads itself when this changes (a dev instance restarted with a rebuilt web/dist).
    pub web_build: Option<String>,
    /// pdtd's background loops (PDT-127): each one's last finished pass and whether it is stale.
    pub loops: Vec<pdt_core::liveness::LoopHealth>,
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
        // Each module lists its own routes in `router()`; one sorted line per module here.
        .merge(crate::activity::router())
        .merge(crate::attachments::router())
        .merge(crate::board::router())
        .merge(crate::bug_reports::router())
        .merge(crate::checks::router())
        .merge(crate::conversations::router())
        .merge(crate::documents::routes::router())
        .merge(crate::files::router())
        .merge(crate::files_search::router())
        .merge(crate::finish::router())
        .merge(crate::folders::router())
        .merge(crate::forge_login::router())
        .merge(crate::guides::router())
        .merge(crate::hooks::router())
        .merge(crate::host::router())
        .merge(crate::host_service::router())
        .merge(crate::identity_guard::router())
        .merge(crate::inbox::router())
        .merge(crate::issues::router())
        .merge(crate::issue_links::router())
        .merge(crate::local_forge::router())
        .merge(crate::mcp::router())
        .merge(crate::mcp_servers::router())
        .merge(crate::notifications::router())
        .merge(crate::overlap::router())
        .merge(crate::parts::router())
        .merge(crate::personas::router())
        .merge(crate::pm::router())
        .merge(crate::project_settings::router())
        .merge(crate::projects::router())
        .merge(crate::pty_ws::router())
        .merge(crate::remote::router())
        .merge(crate::review::router())
        .merge(crate::runconfig::router())
        .merge(crate::command_trust::router())
        .merge(crate::runs::router())
        .merge(crate::sessions::router())
        .merge(crate::setup::router())
        .merge(crate::start::router())
        .merge(crate::stop::router())
        .merge(crate::storage::router())
        .merge(crate::stuck_agents::router())
        .merge(crate::notification_rules::router())
        .merge(crate::install_updates::router())
        .merge(crate::pm_checkpoint::router())
        .merge(crate::pickup_pause::router())
        .merge(crate::ticket_attachments::router())
        .merge(crate::tickets::router())
        .merge(crate::todos::router())
        .merge(crate::toolchains::router())
        .merge(crate::untrusted::router())
        .merge(crate::update::router())
        .merge(crate::vault::router())
        .merge(crate::watches::router())
        .merge(crate::workflow::routes::router())
        .merge(crate::workspaces::router())
        .merge(crate::worktrees::router())
        .merge(crate::integrations::routes(&state))
        .fallback(|| async { ApiError::not_found("route") })
        .method_not_allowed_fallback(|| async {
            ApiError::new(StatusCode::METHOD_NOT_ALLOWED, codes::METHOD_NOT_ALLOWED, "method not allowed")
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
        .layer(axum::middleware::from_fn_with_state(guard_state.clone(), crate::error::note_write_failures))
        .layer(axum::middleware::from_fn_with_state(guard_state.clone(), crate::identity_guard::guard))
        .layer(axum::middleware::from_fn_with_state(guard_state, crate::security_headers::add))
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
        website: pdt_core::brand::WEBSITE.into(),
        version: pdt_core::BUILD_VERSION.into(),
        profile: i.profile,
        port: i.port,
        dev: i.dev,
        web_build,
        loops: st.loops.health(now_ms()),
    })
}

/// The first 12 hex digits of index.html's SHA-256. Vite names every asset by its content hash and index.html links
/// them, so any change to the bundle changes it.
fn web_build_id(index_html: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(index_html).iter().take(6).map(|b| format!("{b:02x}")).collect()
}
