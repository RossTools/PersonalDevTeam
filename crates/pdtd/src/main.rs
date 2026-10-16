mod config;
mod lock;
mod logging;
mod shutdown;

use anyhow::Context;
use pdt_api::{AppState, apply_remote_access, plan_resume_interrupted, router, with_host_guard};
use pdt_core::brand::{DAEMON_BIN, PRODUCT_NAME};
use pdt_store::Store;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
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
    let db_path = cfg.data_dir.join("pdt.db");
    let store = Store::open(&db_path).await.with_context(|| format!("opening database {}", db_path.display()))?;
    let interrupted = store.mark_running_interrupted(jiff::Timestamp::now().as_millisecond()).await?;
    if interrupted > 0 {
        tracing::info!(sessions = interrupted, "sessions from the previous run are now interrupted");
    }
    let mut state = AppState::new(store.clone(), cfg.data_dir.clone());
    state.owner_from_env = cfg.owner_login.clone();
    state.secrets = if cfg.memory_secrets {
        std::sync::Arc::new(pdt_api::secrets::MemorySecretStore::default())
    } else {
        std::sync::Arc::new(pdt_api::secrets::KeyringStore::new(pdt_core::brand::KEYCHAIN_SERVICE))
    };
    state.listen_port = cfg.port;
    state.remote_https_port = cfg.remote_https_port;
    state.allowed_hosts = pdt_api::AllowedHosts::new(cfg.allowed_hosts.clone());
    // The real CLI is installed only here; `AppState::new` runs no tailscale command at all.
    state.tailscale = match &cfg.tailscale_bin {
        Some(bin) => std::sync::Arc::new(pdt_tailscale::TailscaleCli::new(bin.clone())),
        None => std::sync::Arc::new(pdt_tailscale::TailscaleCli::detect(state.platform.login_path.as_deref())),
    };
    // Bindings are decided now; the sessions themselves restart in the background once pdtd is serving, because
    // fresh Codex launches in one folder are spaced out and must not delay the health check.
    let plan = plan_resume_interrupted(&state).await;
    let app = with_host_guard(router(state.clone(), cfg.web_dir.clone()), state.allowed_hosts.clone());
    tracing::info!(
        profile = ?cfg.profile, port = cfg.port, data_dir = %cfg.data_dir.display(),
        web_dir = ?cfg.web_dir, "{PRODUCT_NAME} daemon listening on http://127.0.0.1:{}", cfg.port
    );
    // One background task, in order: remote access first (so later steps see its outcome), then the session
    // resume. With remote access off this runs no tailscale command; `serve --bg` mappings persist across restarts,
    // so with it on this re-adds the allowlist entry and reports the state.
    {
        let st = state.clone();
        tokio::spawn(async move {
            let r = apply_remote_access(&st).await;
            tracing::info!(enabled = r.enabled, active = r.active, error = ?r.error, "remote access applied");
            if !plan.is_empty() {
                let resumed = plan.start(&st).await;
                tracing::info!(sessions = resumed, "resumed sessions from the previous run");
            }
        });
    }
    shutdown::serve(listener, app, state).await?;
    store.close().await;
    tracing::info!("stopped");
    Ok(())
}
