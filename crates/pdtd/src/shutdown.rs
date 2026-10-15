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
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
        _ = st.shutdown.notified() => {}
    }
}

/// Serves until Ctrl-C, SIGTERM or `POST /api/shutdown`. Running sessions are marked `interrupted` and their
/// processes killed (which also ends their WebSockets) before connections drain.
pub async fn serve(listener: TcpListener, app: axum::Router, state: AppState) -> anyhow::Result<()> {
    let (cleaned_tx, cleaned_rx) = tokio::sync::oneshot::channel::<()>();
    let st = state.clone();
    let graceful = async move {
        wait_for_signal(&st).await;
        tracing::info!("shutting down");
        match st.store.mark_running_interrupted(jiff::Timestamp::now().as_millisecond()).await {
            Ok(n) => tracing::info!(sessions = n, "marked running sessions interrupted"),
            Err(e) => tracing::error!(error = %e, "failed to mark sessions interrupted"),
        }
        st.ptys.shutdown().await;
        let _ = cleaned_tx.send(());
    };
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
