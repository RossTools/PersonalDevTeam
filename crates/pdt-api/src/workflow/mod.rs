//! Workflows (Phase 7a, docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md): the node catalog, the durable
//! engine, `session.send`'s outbox, the built-ins, workflows kept in a repo, and the API.

pub(crate) mod builtins;
pub(crate) mod catalog;
pub(crate) mod engine;
pub(crate) mod nodes;
pub(crate) mod outbox;
pub(crate) mod repo;
pub(crate) mod routes;

use crate::routes::{AppState, now_ms};

/// Housekeeping pdtd runs every minute: reads `.pdt/workflows/` in each project's repo and prunes old events and
/// finished runs.
pub async fn maintain_workflows(st: &AppState) {
    repo::sync_all(st).await;
    let now = now_ms();
    if let Err(e) = st.store.prune_events(now - pdt_store::EVENT_RETENTION_MS).await {
        tracing::warn!(error = %e, "could not prune old events");
    }
    if let Err(e) = st.store.prune_workflow_runs(now - pdt_store::RUN_RETENTION_MS).await {
        tracing::warn!(error = %e, "could not prune old workflow runs");
    }
}
