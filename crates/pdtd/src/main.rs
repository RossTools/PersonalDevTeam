mod config;
mod hook;
mod integrations;
mod lock;
mod logging;
mod seed;
mod shutdown;
mod start_at_login;

use anyhow::Context;
use pdt_api::loops::{Start, run_every};
use pdt_api::{
    AppState, apply_remote_access, plan_resume_interrupted, poll_pass, resume_runs, router, with_host_guard,
};
use std::time::Duration;

/// How often the poll (PRs and CI, merges, issues, Jira, workflow upkeep) runs.
const POLL_EVERY: Duration = Duration::from_secs(60);
use pdt_core::brand::{DAEMON_BIN, PRODUCT_NAME};
use pdt_store::Store;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `pdtd hook …` runs inside agent sessions: no runtime, no logging, never stdout, always exit 0.
    if args.get(1).map(String::as_str) == Some("hook") {
        std::process::exit(hook::run(&args[2..], &mut std::io::stdin(), &mut std::io::stdout(), &|k| {
            std::env::var(k).ok()
        }));
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    if let Err(e) = rt.block_on(run()) {
        tracing::error!("{e:#}");
        eprintln!("{DAEMON_BIN}: {e:#}");
        std::process::exit(2);
    }
}

async fn run() -> anyhow::Result<()> {
    let cfg = config::Config::from_env()?;
    // Order matters: bind the port, lock the data dir, start logging, and only then open (and migrate) the database.
    let listener = config::bind(cfg.port).await?;
    std::fs::create_dir_all(&cfg.data_dir).with_context(|| format!("creating data dir {}", cfg.data_dir.display()))?;
    let _lock = lock::lock_data_dir(&cfg.data_dir)?;
    // One redactor of known secrets' values (PDT-158) for the log, the store and the API.
    let redactor = std::sync::Arc::new(pdt_core::secret::Redactor::default());
    let _log_guard = logging::init(&cfg.data_dir.join("logs"), !cfg.supervised, redactor.clone())?;
    if let Ok(p) = std::env::var("PDT_PROFILE")
        && !matches!(p.trim(), "" | "dev" | "stable")
    {
        tracing::warn!("PDT_PROFILE={p:?} is not 'dev' or 'stable'; treating it as stable");
    }
    // pdt-host and the tray find this pdtd by it once Settings → Network names another port (PDT-231).
    if let Err(e) = std::fs::write(pdt_core::network::listening_path(&cfg.data_dir), format!("{}\n", cfg.port)) {
        tracing::warn!(error = %e, "could not record the port pdtd listens on");
    }
    // pdt-host restarts an adopted pdtd started with other variables than it would use (PDT-261).
    let started = pdt_core::handoff::StartedEnv::new(
        pdt_core::BUILD_VERSION,
        std::env::vars().filter(|(k, _)| k.starts_with("PDT_")),
    );
    if let Err(e) =
        std::fs::write(pdt_core::handoff::started_env_path(&cfg.data_dir), serde_json::to_vec(&started).expect("json"))
    {
        tracing::warn!(error = %e, "could not record the variables pdtd started with");
    }
    if cfg.web_dir.is_none() {
        tracing::warn!("the web UI is not served (no web dir found); set PDT_WEB_DIR to the built web/dist directory");
    }
    let db_path = cfg.data_dir.join(pdt_core::handoff::DB_FILE);
    let store = Store::open(&db_path)
        .await
        .with_context(|| format!("opening database {}", db_path.display()))?
        // The dev profile's runs default to their own port range, apart from the stable instance's.
        .with_host_defaults(pdt_core::host::HostSettings::for_profile(cfg.profile))
        .with_redactor(redactor);
    if let Some(which) = cfg.seed {
        if cfg.profile == pdt_core::brand::Profile::Dev {
            let now = jiff::Timestamp::now().as_millisecond();
            let (seeded, dir) = match which {
                config::Seed::Demo => (seed::seed_demo(&store, &cfg.data_dir, now).await?, seed::DEMO_DIR),
                config::Seed::Website => {
                    (seed::website::seed_website(&store, &cfg.data_dir, now).await?, seed::website::WEBSITE_DIR)
                }
            };
            if seeded {
                tracing::info!(dir = %cfg.data_dir.join(dir).display(), seed = ?which, "seeded sample data (PDT_SEED)");
            }
        } else {
            tracing::warn!("PDT_SEED is ignored outside the dev profile");
        }
    }
    let interrupted = store.mark_running_interrupted(jiff::Timestamp::now().as_millisecond()).await?;
    if interrupted > 0 {
        tracing::info!(sessions = interrupted, "sessions from the previous run are now interrupted");
    }
    let mut state = AppState::new(store.clone(), cfg.data_dir.clone());
    state.integrations = std::sync::Arc::new(pdt_api::Integrations::new(integrations::built()));
    // The demo data has a document provider to try the Documents view with (PDT-314): it keeps them in memory, so
    // what is made in it is gone after a restart. Its two sample files are there to import.
    if cfg.profile == pdt_core::brand::Profile::Dev && cfg.seed == Some(config::Seed::Demo) {
        let sample = pdt_api::documents::testing::FakeProvider::new("sample", "Sample (in memory)");
        sample.put("Launch plan", pdt_core::document::DocumentKind::Doc, "# Launch plan\n\n## Goal\n\nShip it.\n");
        sample.put("Budget", pdt_core::document::DocumentKind::Sheet, "");
        state.documents =
            pdt_api::documents::DocumentProviders::new(vec![state.local.clone(), std::sync::Arc::new(sample)]);
    }
    pdt_api::load_enabled_integrations(&state).await?;
    tracing::info!(built = ?state.integrations.built().iter().map(|i| i.id()).collect::<Vec<_>>(),
                   enabled = ?state.integrations.enabled(), "integrations");
    if let Some(p) = &cfg.claude_program {
        state.platform.claude_program = p.clone();
    }
    if let Some(p) = &cfg.codex_program {
        state.platform.codex_program = p.clone();
    }
    if let Some(p) = &cfg.devin_program {
        state.platform.devin_program = p.clone();
    }
    state.owner_from_env = cfg.owner_login.clone();
    state.refresh_owner().await;
    let (secrets, info) = secret_store(&cfg).await;
    state.secrets = std::sync::Arc::new(pdt_api::secrets::Redacting::new(secrets, state.store.redactor().clone()));
    state.secret_store = info;
    state.listen_port = cfg.port;
    state.host_env = std::sync::Arc::new(pdt_api::HostEnv {
        port: cfg.port_from_env.then_some(cfg.port),
        allowed_hosts: cfg.allowed_hosts_from_env.clone(),
        claude_program: cfg.claude_program.clone(),
        codex_program: cfg.codex_program.clone(),
        devin_program: cfg.devin_program.clone(),
        network_at_start: cfg.network.clone(),
        supervised: cfg.supervised,
        default_port: cfg.profile.port(),
    });
    state.instance = pdt_core::instance::InstanceInfo::new(cfg.profile, cfg.port);
    // A stable instance on the port Settings → Network gives it is still the stable one.
    state.instance.dev = cfg.profile == pdt_core::brand::Profile::Dev || (cfg.port_from_env && state.instance.dev);
    state.remote_https_port = cfg.remote_https_port;
    // A slow step never holds up the engine's next tick for long: its run goes on in the background.
    state.engine_tick_wait = std::time::Duration::from_secs(1);
    state.allowed_hosts = pdt_api::AllowedHosts::new(cfg.allowed_hosts.clone());
    // Likewise the GitHub forge over the API (attempts' pull requests, CI, merges and issues); it signs in with the
    // login in the keychain, read in the background below.
    // The forge plugins of the integrations built in (GitLab) share its transport.
    let forge_transport: std::sync::Arc<dyn pdt_forge::transport::Transport> = if cfg.github_offline {
        std::sync::Arc::new(pdt_forge::transport::Offline)
    } else {
        std::sync::Arc::new(pdt_forge::transport::ReqwestTransport::current())
    };
    let mut forges = if cfg.github_offline {
        state.forges.plugins().to_vec()
    } else {
        let api = std::sync::Arc::new(pdt_github::GithubApi::new(forge_transport.clone()));
        vec![std::sync::Arc::new(pdt_github::plugin::GithubPlugin::new(api))
            as std::sync::Arc<dyn pdt_forge::ForgePlugin>]
    };
    forges.extend(integrations::forges(forge_transport));
    state.forges = pdt_forge::ForgeRegistry::new(forges);
    if let Some(id) = &cfg.github_client_id {
        state.github_client_id = id.clone();
    }
    if let Some(id) = &cfg.gitlab_client_id {
        state.forge_client_ids.insert("gitlab".into(), id.clone());
    }
    // The real CLI is installed only here; `AppState::new` runs no tailscale command at all.
    state.tailscale = match &cfg.tailscale_bin {
        Some(bin) => std::sync::Arc::new(pdt_tailscale::TailscaleCli::new(bin.clone())),
        None => std::sync::Arc::new(pdt_tailscale::TailscaleCli::detect(state.platform.login_path.as_deref())),
    };
    // pdt-host's login item is changed only here (Settings → This host).
    state.start_at_login =
        std::sync::Arc::new(start_at_login::HostLogin::new(cfg.profile, cfg.supervised, cfg.data_dir.clone()));
    // Installs for the ACP profiles reach the network only here (PDT-239).
    state.toolchains = std::sync::Arc::new(pdt_api::toolchains::Http::new());
    // The machine's real free space: `AppState::new` reads none, so tests never see this disk (PDT-324).
    state.disk_space = std::sync::Arc::new(pdt_api::storage::os_disk_space);
    state.update = std::sync::Arc::new(
        pdt_api::UpdateService::with_disabled_reason(
            pdt_update::GITHUB_API,
            pdt_core::brand::RELEASE_REPO,
            cfg.update_disabled_reason(),
            pdt_core::BUILD_VERSION,
        )
        .with_repo_id(pdt_core::brand::RELEASE_REPO_ID)
        .with_env_repo(cfg.release_repo.clone()),
    );
    if let Ok(bytes) = std::fs::read(pdt_core::handoff::result_path(&cfg.data_dir))
        && let Ok(r) = serde_json::from_slice::<pdt_core::handoff::UpdateResult>(&bytes)
    {
        tracing::info!(outcome = ?r.outcome, to = %r.to_version, "last update result");
        state.update.set_last_result(Some(r));
    }
    // The stable instance's default data dir keeps the plain keychain account; any other data dir (dev instances,
    // tests, explicit PDT_DATA_DIR) gets its own, so it never reads or deletes the stable instance's token.
    let stable_data_dir =
        dirs::data_local_dir().map(|root| root.join(pdt_core::brand::Profile::Stable.data_dir_name()));
    state.update_token_account = pdt_api::secrets::update_token_account(&cfg.data_dir, stable_data_dir.as_deref());
    state.github_account = pdt_api::secrets::github_login_account(&cfg.data_dir, stable_data_dir.as_deref());
    state.forge_logins_account = pdt_api::secrets::forge_logins_account(&cfg.data_dir, stable_data_dir.as_deref());
    // In the background: serving (and pdt-host's health check after an update) never waits on the keychain.
    tokio::spawn({
        let st = state.clone();
        async move {
            pdt_api::update::sync_token_state(&st).await;
            pdt_api::forge_login::load(&st).await;
            // The secrets' values, so they are redacted from the start.
            pdt_api::vault::load_known(&st).await;
        }
    });
    if cfg.rehearsal {
        return rehearse(listener, state, store, cfg).await;
    }
    // Bindings are decided now; the sessions themselves restart in the background once pdtd is serving, because
    // fresh Codex launches in one folder are spaced out and must not delay the health check.
    let plan = plan_resume_interrupted(&state).await;
    // Conversations need no restart: a turn the previous run left running ends as interrupted, open requests expire.
    pdt_api::recover_conversations(&state).await;
    let app = with_host_guard(router(state.clone(), cfg.web_dir.clone()), state.allowed_hosts.clone());
    tracing::info!(
        profile = ?cfg.profile, port = cfg.port, data_dir = %cfg.data_dir.display(),
        web_dir = ?cfg.web_dir, "{PRODUCT_NAME} daemon listening on http://127.0.0.1:{}", cfg.port
    );
    // One background task, in order (ruling F4): remote access first, so resumed runs get tailnet links and their
    // ports' mappings are this instance's; then the runs and sessions the previous pdtd left (after which the run-port
    // mappings a previous pdtd made, kept in the store, go when no resumed run uses them); then the 60 s poll
    // (`poll_pass`: gone runs, PRs and CI, merges, issues, Jira, workflow upkeep). With remote access off and nothing
    // left mapped by this instance this runs no tailscale command; `serve --bg` mappings persist across restarts, so
    // with it on this re-adds the allowlist entry and reports the state.
    // Every loop's passes are contained (a panic ends the pass, not the loop) and recorded for /api/health (PDT-127).
    state.loops.register("poll", POLL_EVERY, Some(pdt_api::loops::POLL_STALE_AFTER.as_millis() as i64));
    {
        let st = state.clone();
        tokio::spawn(async move {
            let startup = st.clone();
            pdt_api::loops::contain("the startup resume", None, async move {
                let st = startup;
                let r = apply_remote_access(&st).await;
                tracing::info!(enabled = r.enabled, active = r.active, error = ?r.error, "remote access applied");
                // From here on remote access follows Tailscale: retried while it is not active (Tailscale often comes
                // up after pdtd at login), checked while it is (PDT-148).
                tokio::spawn(pdt_api::run_remote_watch(st.clone()));
                let runs = resume_runs(&st).await;
                if runs > 0 {
                    tracing::info!(runs, "resumed runs from the previous daemon");
                }
                if !plan.is_empty() {
                    let resumed = plan.start(&st).await;
                    tracing::info!(sessions = resumed, "resumed sessions from the previous run");
                }
            })
            .await;
            run_every(st, "poll", POLL_EVERY, Start::Now, poll_pass).await;
        });
    }
    // The loops and tasks that run for as long as pdtd does.
    pdt_api::start_background(&state);
    shutdown::serve(listener, app, state).await?;
    store.close().await;
    tracing::info!("stopped");
    Ok(())
}

/// The secret store: the OS keychain (bounded by a timeout, so an approval prompt never hangs pdtd), or on Linux
/// without a Secret Service the data folder's file (PDT-197); memory for tests and rehearsals. On Linux the choice
/// sticks to this data folder (`StoreHistory`): the file once it exists, the keychain once it was chosen.
async fn secret_store(
    cfg: &config::Config,
) -> (std::sync::Arc<dyn pdt_api::secrets::SecretStore>, pdt_api::secrets::SecretStoreInfo) {
    use pdt_api::secrets::{
        Cached, FileSecretStore, KeyringStore, MemorySecretStore, SecretBackend, SecretStoreInfo, StoreHistory,
        TimeLimited, choose_backend, keychain_marker, secrets_file,
    };
    let forced = if cfg.memory_secrets {
        Some("memory")
    } else if cfg.file_secrets {
        Some("file")
    } else {
        None
    };
    let linux = cfg!(target_os = "linux");
    let (file, marker) = (secrets_file(&cfg.data_dir), keychain_marker(&cfg.data_dir));
    let history = StoreHistory { file: file.is_file(), keychain: marker.is_file() };
    // Asked only on Linux, and not once secrets went to the file; a D-Bus that does not answer counts as no Secret
    // Service. A keychain used before is still asked, to report one that does not answer.
    let reachable = || async {
        let probe = tokio::task::spawn_blocking(|| KeyringStore::new(pdt_core::brand::KEYCHAIN_SERVICE).reachable());
        tokio::time::timeout(Duration::from_secs(5), probe).await.ok().and_then(Result::ok).unwrap_or(false)
    };
    let service = if linux && forced.is_none() && !history.file { reachable().await } else { true };
    let backend = choose_backend(linux, forced, history, || service);
    match backend {
        SecretBackend::Memory => {
            (std::sync::Arc::new(MemorySecretStore::default()), SecretStoreInfo { backend, path: None, problem: None })
        }
        SecretBackend::File => {
            tracing::warn!(file = %file.display(), "secrets are kept in a file in the data folder that only this user \
                can read (no Secret Service was reachable when this data folder first stored one)");
            let store = FileSecretStore::new(file.clone());
            // Made now, so the next start keeps using it even if a Secret Service turns up.
            if let Err(e) = store.ensure() {
                tracing::error!("{e}");
            }
            let info = SecretStoreInfo { backend, path: Some(file.to_string_lossy().into_owned()), problem: None };
            (std::sync::Arc::new(store), info)
        }
        SecretBackend::Keychain => {
            let mut problem = None;
            if linux && forced.is_none() {
                if service {
                    if !history.keychain
                        && let Err(e) = std::fs::write(&marker, b"")
                    {
                        tracing::error!(file = %marker.display(), "could not record that secrets use the keychain: {e}");
                    }
                } else {
                    let why = "the Secret Service (system keyring) this data folder uses did not answer at startup: \
                               it may not be running yet, or be locked. Secrets cannot be read until it answers; PDT \
                               keeps trying it and never moves them to a file.";
                    tracing::warn!("{why}");
                    problem = Some(why.to_string());
                }
            }
            // Reads are kept in memory, so pollers do not ask the keychain each time (PDT-215).
            let store = Cached::new(std::sync::Arc::new(TimeLimited::new(
                std::sync::Arc::new(KeyringStore::new(pdt_core::brand::KEYCHAIN_SERVICE)),
                Duration::from_secs(5),
            )));
            (std::sync::Arc::new(store), SecretStoreInfo { backend, path: None, problem })
        }
    }
}

/// An upgrade rehearsal (`PDT_REHEARSAL`, PDT-179): the copy is migrated and served for `scripts/rehearse-upgrade.sh`
/// to check, with the built-in workflows and personas installed for what is enabled. Nothing resumes and no background
/// loop starts, so the copy's runs, sessions, workflows and repos are never acted on.
async fn rehearse(
    listener: tokio::net::TcpListener,
    state: AppState,
    store: Store,
    cfg: config::Config,
) -> anyhow::Result<()> {
    pdt_api::recover_conversations(&state).await;
    pdt_api::install_builtins(&state).await;
    let app = with_host_guard(router(state.clone(), cfg.web_dir.clone()), state.allowed_hosts.clone());
    tracing::info!(
        port = cfg.port, data_dir = %cfg.data_dir.display(),
        "{PRODUCT_NAME} upgrade rehearsal listening on http://127.0.0.1:{} (nothing resumes, no background loop)", cfg.port
    );
    shutdown::serve(listener, app, state).await?;
    store.close().await;
    tracing::info!("stopped");
    Ok(())
}
