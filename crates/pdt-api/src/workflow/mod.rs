//! Workflows (Phase 7a, docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md): the node catalog, the durable
//! engine, `session.send`'s outbox, the built-ins, workflows kept in a repo, and the API.

pub(crate) mod agent;
pub(crate) mod builtins;
pub(crate) mod catalog;
pub(crate) mod engine;
pub(crate) mod nodes;
pub(crate) mod outbox;
pub(crate) mod pipeline;
pub(crate) mod repo;
pub(crate) mod routes;
pub(crate) mod templates;
pub(crate) mod transfer;

use crate::routes::{AppState, now_ms};

/// How often `maintain_workflows` empties SQLite's WAL.
const CHECKPOINT_EVERY_MS: i64 = 10 * 60 * 1000;

static LAST_CHECKPOINT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Housekeeping pdtd runs every minute: reads `.pdt/workflows/` in each project's repo, stops idle MCP servers and
/// lists stale ones' tools again, prunes old events, finished runs, told messages, ended Inbox items and old
/// conversation frames, and every ten minutes empties the database's WAL.
pub async fn maintain_workflows(st: &AppState) {
    repo::sync_all(st).await;
    crate::mcp_client::maintain(st).await;
    let now = now_ms();
    if let Err(e) = st.store.prune_events(now - pdt_store::EVENT_RETENTION_MS).await {
        tracing::warn!(error = %e, "could not prune old events");
    }
    if let Err(e) = st.store.prune_workflow_runs(now - pdt_store::RUN_RETENTION_MS).await {
        tracing::warn!(error = %e, "could not prune old workflow runs");
    }
    if let Err(e) = st.store.prune_session_messages(now - pdt_store::EVENT_RETENTION_MS).await {
        tracing::warn!(error = %e, "could not prune old session messages");
    }
    if let Err(e) = st.store.prune_inbox_items(now - pdt_store::EVENT_RETENTION_MS).await {
        tracing::warn!(error = %e, "could not prune old Inbox items");
    }
    if let Err(e) = st.store.prune_session_event_raw(now - pdt_store::RAW_RETENTION_MS).await {
        tracing::warn!(error = %e, "could not prune old conversation frames");
    }
    let last = LAST_CHECKPOINT.load(std::sync::atomic::Ordering::Relaxed);
    if now - last >= CHECKPOINT_EVERY_MS {
        LAST_CHECKPOINT.store(now, std::sync::atomic::Ordering::Relaxed);
        match st.store.checkpoint().await {
            Ok(true) => {}
            Ok(false) => tracing::debug!("the WAL checkpoint was blocked; the next one tries again"),
            Err(e) => tracing::warn!(error = %e, "could not checkpoint the database"),
        }
    }
}
