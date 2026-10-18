mod config;
mod hook;
mod lock;
mod logging;
mod seed;
mod shutdown;

use anyhow::Context;
use pdt_api::{
    AppState, apply_remote_access, plan_resume_interrupted, resume_runs, router, run_activity_ticker, sweep_gone_runs,
    with_host_guard,
};
use pdt_core::brand::{DAEMON_BIN, PRODUCT_NAME};
use pdt_store::Store;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `pdtd hook …` runs inside agent sessions: no runtime, no logging, never stdout, always exit 0.
    if args.get(1).map(String::as_str) == Some("hook") {
        std::process::exit(hook::run(&args[2..], &mut std::io::stdin(), &|k| std::env::var(k).ok()));
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
    let _log_guard = logging::init(&cfg.data_dir.join("logs"))?;
    if let Ok(p) = std::env::var("PDT_PROFILE")
        && !matches!(p.trim(), "" | "dev" | "stable")
    {
        tracing::warn!("PDT_PROFILE={p:?} is not 'dev' or 'stable'; treating it as stable");
    }
    if cfg.web_dir.is_none() {
        tracing::warn!("the web UI is not served (no web dir found); set PDT_WEB_DIR to the built web/dist directory");
    }
    let db_path = cfg.data_dir.join(pdt_core::handoff::DB_FILE);
    let store = Store::open(&db_path)
        .await
        .with_context(|| format!("opening database {}", db_path.display()))?
        // The dev profile's runs default to their own port range, apart from the stable instance's.
        .with_host_defaults(pdt_core::host::HostSettings::for_profile(cfg.profile));
    if cfg.seed == Some(config::Seed::Demo) {
        if cfg.profile == pdt_core::brand::Profile::Dev {
            if seed::seed_demo(&store, &cfg.data_dir, jiff::Timestamp::now().as_millisecond()).await? {
                tracing::info!(dir = %cfg.data_dir.join(seed::DEMO_DIR).display(), "seeded demo data (PDT_SEED=demo)");
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
    if let Some(p) = &cfg.claude_program {
        state.platform.claude_program = p.clone();
    }
    if let Some(p) = &cfg.codex_program {
        state.platform.codex_program = p.clone();
    }
    state.owner_from_env = cfg.owner_login.clone();
    state.refresh_owner().await;
    state.secrets = if cfg.memory_secrets {
        std::sync::Arc::new(pdt_api::secrets::MemorySecretStore::default())
    } else {
        // A keychain waiting on an approval prompt must never hang pdtd (or its health check).
        std::sync::Arc::new(pdt_api::secrets::TimeLimited::new(
            std::sync::Arc::new(pdt_api::secrets::KeyringStore::new(pdt_core::brand::KEYCHAIN_SERVICE)),
            std::time::Duration::from_secs(5),
        ))
    };
    state.listen_port = cfg.port;
    state.instance = pdt_core::instance::InstanceInfo::new(cfg.profile, cfg.port);
    state.remote_https_port = cfg.remote_https_port;
    state.allowed_hosts = pdt_api::AllowedHosts::new(cfg.allowed_hosts.clone());
    // The real CLI is installed only here; `AppState::new` runs no tailscale command at all.
    state.tailscale = match &cfg.tailscale_bin {
        Some(bin) => std::sync::Arc::new(pdt_tailscale::TailscaleCli::new(bin.clone())),
        None => std::sync::Arc::new(pdt_tailscale::TailscaleCli::detect(state.platform.login_path.as_deref())),
    };
    state.update = std::sync::Arc::new(pdt_api::UpdateService::with_disabled_reason(
        pdt_update::GITHUB_API,
        pdt_core::brand::RELEASE_REPO,
        cfg.update_disabled_reason(),
        pdt_core::BUILD_VERSION,
    ));
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
    // In the background: serving (and pdt-host's health check after an update) never waits on the keychain.
    tokio::spawn({
        let st = state.clone();
        async move { pdt_api::update::sync_token_state(&st).await }
    });
    // Bindings are decided now; the sessions themselves restart in the background once pdtd is serving, because
    // fresh Codex launches in one folder are spaced out and must not delay the health check.
    let plan = plan_resume_interrupted(&state).await;
    let app = with_host_guard(router(state.clone(), cfg.web_dir.clone()), state.allowed_hosts.clone());
    tracing::info!(
        profile = ?cfg.profile, port = cfg.port, data_dir = %cfg.data_dir.display(),
        web_dir = ?cfg.web_dir, "{PRODUCT_NAME} daemon listening on http://127.0.0.1:{}", cfg.port
    );
    // One background task, in order (ruling F4): remote access first, so resumed runs get tailnet links and their
    // ports' mappings are this process's (an old port can then be unmapped when a run moves); then the runs and
    // sessions the previous pdtd left; then the sweep for runs whose branch or worktree is gone, every 60 s.
    // With remote access off this runs no tailscale command; `serve --bg` mappings persist across restarts, so with
    // it on this re-adds the allowlist entry and reports the state.
    {
        let st = state.clone();
        tokio::spawn(async move {
            let r = apply_remote_access(&st).await;
            tracing::info!(enabled = r.enabled, active = r.active, error = ?r.error, "remote access applied");
            let runs = resume_runs(&st).await;
            if runs > 0 {
                tracing::info!(runs, "resumed runs from the previous daemon");
            }
            if !plan.is_empty() {
                let resumed = plan.start(&st).await;
                tracing::info!(sessions = resumed, "resumed sessions from the previous run");
            }
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tick.tick().await;
                if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                sweep_gone_runs(&st).await;
            }
        });
    }
    // Time-based activity signals (no output while working, a Claude hook that never came).
    tokio::spawn(run_activity_ticker(state.clone()));
    // Warn at startup when an agent CLI is missing or not logged in for the user this daemon runs as (spec §3.1).
    {
        let st = state.clone();
        tokio::spawn(async move {
            let c = pdt_api::run_host_checks(&st).await;
            for (name, check) in [("claude", &c.claude), ("codex", &c.codex)] {
                if !check.ok {
                    tracing::warn!(tool = name, detail = %check.detail, "agent CLI is missing or not logged in for this user");
                }
            }
        });
    }
    // Hourly update checks (and the auto-when-idle install); returns at once unless pdt-host supervises this pdtd.
    tokio::spawn(pdt_api::run_update_loop(state.clone()));
    shutdown::serve(listener, app, state).await?;
    store.close().await;
    tracing::info!("stopped");
    Ok(())
}
