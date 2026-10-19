use pdt_api::AppState;
use std::future::IntoFuture;
use std::time::Duration;
use tokio::net::TcpListener;

/// After cleanup, open connections get this long to finish before pdtd exits anyway.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

async fn wait_for_signal(st: &AppState) {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    // SIGHUP (the terminal that started pdtd closed) shuts down as gracefully as SIGTERM.
    #[cfg(unix)]
    let unix_signal = |kind: tokio::signal::unix::SignalKind| async move {
        match tokio::signal::unix::signal(kind) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(unix)]
    let (term, hup) = (
        unix_signal(tokio::signal::unix::SignalKind::terminate()),
        unix_signal(tokio::signal::unix::SignalKind::hangup()),
    );
    #[cfg(not(unix))]
    let (term, hup) = (std::future::pending::<()>(), std::future::pending::<()>());
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
        _ = hup => {}
        _ = st.shutdown.notified() => {}
    }
}

/// Serves until Ctrl-C, SIGTERM, SIGHUP or `POST /api/shutdown`. Running sessions are marked `interrupted` and their
/// processes killed (which also ends their WebSockets) before connections drain.
pub async fn serve(listener: TcpListener, app: axum::Router, state: AppState) -> anyhow::Result<()> {
    let (cleaned_tx, cleaned_rx) = tokio::sync::oneshot::channel::<()>();
    let st = state.clone();
    let graceful = async move {
        wait_for_signal(&st).await;
        tracing::info!("shutting down");
        st.shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);
        st.wake_resume_waiters();
        match st.store.mark_running_interrupted(jiff::Timestamp::now().as_millisecond()).await {
            Ok(n) => tracing::info!(sessions = n, "marked running sessions interrupted"),
            Err(e) => tracing::error!(error = %e, "failed to mark sessions interrupted"),
        }
        st.ptys.shutdown().await;
        pdt_api::shutdown_conversations(&st).await;
        let _ = cleaned_tx.send(());
    };
    // TCP_NODELAY: terminal echoes are tiny WebSocket frames; with Nagle's algorithm one can wait for the ACK of the
    // previous one, which over a remote link (Tailscale) makes typing lag.
    let listener = axum::serve::ListenerExt::tap_io(listener, |tcp| {
        if let Err(e) = tcp.set_nodelay(true) {
            tracing::debug!(error = %e, "could not set TCP_NODELAY");
        }
    });
    let server = axum::serve(listener, app).with_graceful_shutdown(graceful).into_future();
    tokio::select! {
        r = server => r?,
        _ = async {
            if cleaned_rx.await.is_ok() {
                tokio::time::sleep(DRAIN_TIMEOUT).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => tracing::warn!("connections did not close within {DRAIN_TIMEOUT:?}; exiting anyway"),
    }
    Ok(())
}
