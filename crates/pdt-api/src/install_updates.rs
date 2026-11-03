//! Settings → Updates (PDT-369): the Install updates workflow's inputs (a time window, stable releases only, installing
//! anyway after a while, the check after installing), read and written as a new version of it, through
//! [`crate::builtin_settings`]. A workflow the user changed beyond its inputs, or turned off, is only shown (with
//! Reset to default). The host's update setting (off, notify, install when idle) and channel stay host settings: they
//! decide what the checks find.

use crate::builtin_settings::{Reset, router_with};
use crate::routes::{AppState, now_ms};
use crate::workflow::builtins::INSTALL_UPDATES;
use crate::workflow::builtins::install_updates::SETTINGS;
use axum::Router;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

/// Through [`crate::builtin_settings`]; Reset to default turns the workflow on and installs the update on offer.
pub fn router() -> Router<AppState> {
    router_with("/install-updates", &SETTINGS, Reset { turns_on: true, then: Some(wake_boxed) })
}

fn wake_boxed(st: &AppState) -> BoxFuture<'_, ()> {
    Box::pin(wake(st))
}

/// Updates were just set to install when idle: the update on offer, announced while they weren't, gets a run of
/// Install updates now (its run then ended at once), unless one is going already. Runs started by `update.available`
/// look at the setting as they wait, so this is only for the update already announced.
pub(crate) async fn wake(st: &AppState) {
    let Some(offer) = crate::update::on_offer(&st.update) else { return };
    let _engine = st.engine_lock.lock().await;
    let Ok(Some(w)) = st.store.workflow_by_builtin(INSTALL_UPDATES).await else { return };
    if !w.enabled {
        return;
    }
    let f = pdt_store::RunFilter { workflow_id: Some(w.id.clone()), limit: 20, ..Default::default() };
    match st.store.list_workflow_runs(&f).await {
        Ok(runs) if runs.iter().any(|r| !r.status.is_finished()) => return,
        Ok(_) => {}
        Err(e) => return tracing::warn!(error = %e, "could not read Install updates' runs"),
    }
    let Ok(Some(doc)) = st.store.workflow_doc(&w.id, w.version).await else { return };
    let Some(node) = doc.nodes.iter().find(|n| n.kind == pdt_core::workflow::events::UPDATE_AVAILABLE) else { return };
    let mut trigger = serde_json::to_value(&offer).unwrap_or(Value::Null);
    trigger["type"] = json!(pdt_core::workflow::events::UPDATE_AVAILABLE);
    let key = format!("{}:auto:{}:{}", node.id, offer.version, now_ms());
    let node = node.id.clone();
    if let Err(e) = crate::workflow::engine::start_run(st, &w, &doc, &node, trigger, None, key, None).await {
        tracing::warn!(error = %e, "could not start Install updates for the update on offer");
    }
}
