//! Cloud instances (PDT-638), on the owner: a joined instance that is a VM at a cloud provider. A provider is a
//! plugin's [`CloudProvider`] ([`crate::Integration::cloud_provider`]: AWS EC2, Azure), reached with a connection of
//! its kind. Every 30 s the owner checks each cloud instance ([`check_all`]): it starts a stopped one that tickets are
//! placed on (pickup holds them, "starting win-cloud…"; once its PDT answers, the heartbeats free the slot and pickup
//! dispatches), stops one that has been idle (no live sessions or runs, nothing waiting) for its minutes, and meters
//! its running time. The rules are pdt-core `cloud`; the user's settings are Project Settings → Remote instances
//! (`PUT`/`DELETE /api/projects/{id}/remote-instances/{link}/cloud`, Start and Stop).

use crate::error::{ApiError, codes};
use crate::extract::{ApiJson, ApiPath};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use pdt_core::cloud::{CloudAction, CloudSetup, CloudState, CloudUsage, Observed, Power};
use pdt_core::link::{LinkState, RemoteInstance};
use pdt_core::workflow::api::Connection;
use pdt_store::CloudInstance;
use serde::Serialize;
use std::sync::Arc;
use ts_rs::TS;

/// The VM a call is about, and the provider's credentials.
pub struct CloudTarget {
    /// The connection with the credentials: its config holds the provider's own settings (a region, a tenant).
    pub connection: Connection,
    /// The connection's secret.
    pub secret: String,
    /// The VM at the provider (an EC2 instance id, an Azure `resource-group/vm-name`).
    pub machine: String,
}

/// A cloud provider (a plugin's): start, stop and read the power state of one VM. Blocking (it reaches the provider
/// through a `pdt_forge::transport::Transport`); PDT calls it off the async workers. Errors are messages for the user,
/// without credentials.
pub trait CloudProvider: Send + Sync {
    /// The connection kind whose connections reach it.
    fn kind(&self) -> &'static str;
    /// What a machine is called at this provider, for the settings' placeholder.
    fn machine_hint(&self) -> &'static str;
    /// Checks a machine's name. Err is a message for the user.
    fn check_machine(&self, _machine: &str) -> Result<(), String> {
        Ok(())
    }
    fn power(&self, t: &CloudTarget) -> Result<Power, String>;
    fn start(&self, t: &CloudTarget) -> Result<(), String>;
    /// Stops it so that its compute isn't billed (Azure: deallocates it).
    fn stop(&self, t: &CloudTarget) -> Result<(), String>;
}

/// The enabled plugins' providers.
fn providers(st: &AppState) -> Vec<Arc<dyn CloudProvider>> {
    st.integrations.registry().integrations().iter().filter_map(|i| i.cloud_provider()).collect()
}

/// The provider whose connections are of `kind`, while its plugin is enabled.
fn provider(st: &AppState, kind: &str) -> Option<Arc<dyn CloudProvider>> {
    providers(st).into_iter().find(|p| p.kind() == kind)
}

/// The provider and target of a cloud instance, or why there are none.
async fn target(st: &AppState, setup: &CloudSetup) -> Result<(Arc<dyn CloudProvider>, CloudTarget), String> {
    let (connection, secret) = crate::workflow::routes::connection_with_secret(st, &setup.connection).await?;
    let p = provider(st, &connection.kind).ok_or_else(|| {
        format!("{} is a {} connection, which no enabled plugin starts machines with", connection.name, connection.kind)
    })?;
    let secret = secret.filter(|s| !s.is_empty()).ok_or_else(|| format!("{} has no secret", connection.name))?;
    Ok((p, CloudTarget { connection, secret, machine: setup.machine.clone() }))
}

/// Runs a provider call off the async workers.
async fn call<T: Send + 'static>(
    p: &Arc<dyn CloudProvider>,
    t: &Arc<CloudTarget>,
    f: impl FnOnce(&dyn CloudProvider, &CloudTarget) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (p, t) = (p.clone(), t.clone());
    tokio::task::spawn_blocking(move || f(p.as_ref(), &t)).await.map_err(|e| e.to_string())?
}

/// Checks every cloud instance now (the 30 s loop).
pub async fn check_all(st: &AppState) {
    check_at(st, now_ms()).await
}

/// Checks every cloud instance as at `now` (ms): tests pass a later time to see idle instances stop.
pub async fn check_at(st: &AppState, now: i64) {
    let clouds = match st.store.list_cloud_instances(None).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "can't list the cloud instances");
            return;
        }
    };
    futures_util::future::join_all(clouds.iter().map(|c| check_one(st, c, now))).await;
}

async fn check_one(st: &AppState, c: &CloudInstance, now: i64) {
    let Ok(Some(i)) = st.store.get_remote_instance(&c.link_id).await else { return };
    let prev = &c.state;
    let failed = async |e: String| {
        let s = CloudState { last_error: Some(e), ..prev.clone() };
        if let Err(e) = st.store.set_cloud_state(&c.link_id, &s).await {
            tracing::warn!(instance = %i.name, error = %e, "can't keep a cloud instance's state");
        }
    };
    let (p, t) = match target(st, &c.setup).await {
        Ok((p, t)) => (p, Arc::new(t)),
        Err(e) => return failed(e).await,
    };
    let power = match call(&p, &t, |p, t| p.power(t)).await {
        Ok(power) => power,
        Err(e) => return failed(format!("can't read its state: {e}")).await,
    };
    let state = pdt_core::link::state(i.last_seen_at, true, i.paused, now);
    let observed = Observed {
        power,
        busy: st.store.instance_busy(&i.project_id, &i.instance_id).await.unwrap_or(true),
        wanted: !i.paused && !crate::offline_instances::waiting_for(st, &i, now).is_empty(),
        connected: matches!(state, LinkState::Connected | LinkState::Paused),
        now,
    };
    let mut step = pdt_core::cloud::step(&c.setup, prev, &observed);
    if step.metered_ms > 0 {
        let month = pdt_core::cloud::month_of(now);
        let cost = pdt_core::cloud::cost(step.metered_ms, c.setup.hourly_rate);
        if let Err(e) = st.store.add_cloud_usage(&c.link_id, &month, step.metered_ms, cost).await {
            tracing::warn!(instance = %i.name, error = %e, "can't meter a cloud instance");
        }
    }
    if let Some(action) = step.action {
        if let Err(e) = act(&p, &t, action).await {
            step.state.last_error = Some(e);
        } else {
            tracing::info!(instance = %i.name, ?action, "asked the cloud provider to {}", verb(action));
        }
    }
    if let Err(e) = st.store.set_cloud_state(&c.link_id, &step.state).await {
        tracing::warn!(instance = %i.name, error = %e, "can't keep a cloud instance's state");
    }
}

fn verb(a: CloudAction) -> &'static str {
    match a {
        CloudAction::Start => "start it",
        CloudAction::Stop => "stop it",
    }
}

async fn act(p: &Arc<dyn CloudProvider>, t: &Arc<CloudTarget>, action: CloudAction) -> Result<(), String> {
    let done = match action {
        CloudAction::Start => call(p, t, |p, t| p.start(t)).await,
        CloudAction::Stop => call(p, t, |p, t| p.stop(t)).await,
    };
    done.map_err(|e| format!("couldn't {}: {e}", verb(action)))
}

/// A cloud instance as Remote instances shows it.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CloudView {
    pub setup: CloudSetup,
    pub state: CloudState,
    /// The owner is bringing it up for work placed on it (or the user's Start).
    pub starting: bool,
    /// Its meter, newest month first.
    pub usage: Vec<CloudUsage>,
}

/// The cloud views of a project's cloud instances, by link id.
pub(crate) async fn views(
    st: &AppState,
    project_id: &str,
) -> Result<std::collections::HashMap<String, CloudView>, ApiError> {
    let mut out = std::collections::HashMap::new();
    for c in st.store.list_cloud_instances(Some(project_id)).await? {
        let usage = st.store.cloud_usage(&c.link_id).await?;
        let starting = pdt_core::cloud::starting(&c.state);
        out.insert(c.link_id, CloudView { setup: c.setup, state: c.state, starting, usage });
    }
    Ok(out)
}

/// A connection kind with a cloud provider, for the settings' picker.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CloudProviderView {
    pub kind: String,
    pub machine_hint: String,
}

pub(crate) fn provider_views(st: &AppState) -> Vec<CloudProviderView> {
    providers(st)
        .iter()
        .map(|p| CloudProviderView { kind: p.kind().into(), machine_hint: p.machine_hint().into() })
        .collect()
}

/// A joined instance of `project_id`, or 404.
async fn instance_of(st: &AppState, project_id: &str, link_id: &str) -> Result<RemoteInstance, ApiError> {
    st.store
        .get_remote_instance(link_id)
        .await?
        .filter(|i| i.project_id == project_id)
        .ok_or_else(|| ApiError::not_found("remote instance"))
}

/// `PUT /api/projects/{id}/remote-instances/{link}/cloud`: makes a joined instance a cloud instance (or changes its
/// setup). The connection must be of an enabled provider's kind.
async fn put_setup(
    State(st): State<AppState>,
    ApiPath((id, link_id)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<CloudSetup>,
) -> Result<Json<crate::remote_instances::RemoteInstancesView>, ApiError> {
    let p = crate::remote_instances::own_project(&st, &id).await?;
    instance_of(&st, &p.id, &link_id).await?;
    let setup = body.checked().map_err(ApiError::invalid)?;
    let conn = st
        .store
        .connection_by_name(&setup.connection)
        .await?
        .ok_or_else(|| ApiError::invalid(format!("no connection is called {}", setup.connection)))?;
    let provider = provider(&st, &conn.kind).ok_or_else(|| {
        ApiError::invalid(format!("{} is a {} connection, not a cloud provider's", conn.name, conn.kind))
    })?;
    provider.check_machine(&setup.machine).map_err(ApiError::invalid)?;
    st.store.set_cloud_setup(&link_id, Some(&setup), now_ms()).await?;
    Ok(Json(crate::remote_instances::view(&st, &p).await?))
}

/// `DELETE /api/projects/{id}/remote-instances/{link}/cloud`: it is no cloud instance any more (the VM is left as it is).
async fn delete_setup(
    State(st): State<AppState>,
    ApiPath((id, link_id)): ApiPath<(String, String)>,
) -> Result<Json<crate::remote_instances::RemoteInstancesView>, ApiError> {
    let p = crate::remote_instances::own_project(&st, &id).await?;
    instance_of(&st, &p.id, &link_id).await?;
    st.store.set_cloud_setup(&link_id, None, now_ms()).await?;
    Ok(Json(crate::remote_instances::view(&st, &p).await?))
}

/// Start or Stop by hand: asks the provider now. A started instance stops again once idle for its minutes.
async fn by_hand(
    st: &AppState,
    id: &str,
    link_id: &str,
    action: CloudAction,
) -> Result<Json<crate::remote_instances::RemoteInstancesView>, ApiError> {
    let p = crate::remote_instances::own_project(st, id).await?;
    let i = instance_of(st, &p.id, link_id).await?;
    let c = st.store.get_cloud_instance(link_id).await?.ok_or_else(|| {
        ApiError::bad_request(codes::INVALID_INPUT, format!("{} isn't set up as a cloud instance", i.name))
    })?;
    let (provider, t) = target(st, &c.setup).await.map_err(ApiError::invalid)?;
    act(&provider, &Arc::new(t), action).await.map_err(|e| ApiError::upstream(codes::CLOUD_PROVIDER, e))?;
    let now = now_ms();
    let mut s = CloudState { last_error: None, ..c.state };
    match action {
        CloudAction::Start => {
            s.start_requested_at = Some(now);
            s.stop_requested_at = None;
            s.last_busy_at = Some(now);
        }
        CloudAction::Stop => {
            s.stop_requested_at = Some(now);
            s.start_requested_at = None;
        }
    }
    st.store.set_cloud_state(link_id, &s).await?;
    tracing::info!(instance = %i.name, ?action, "asked the cloud provider to {} by hand", verb(action));
    Ok(Json(crate::remote_instances::view(st, &p).await?))
}

async fn start(
    State(st): State<AppState>,
    ApiPath((id, link_id)): ApiPath<(String, String)>,
) -> Result<Json<crate::remote_instances::RemoteInstancesView>, ApiError> {
    by_hand(&st, &id, &link_id, CloudAction::Start).await
}

async fn stop(
    State(st): State<AppState>,
    ApiPath((id, link_id)): ApiPath<(String, String)>,
) -> Result<Json<crate::remote_instances::RemoteInstancesView>, ApiError> {
    by_hand(&st, &id, &link_id, CloudAction::Stop).await
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<AppState> {
    use axum::routing::{post, put};
    axum::Router::new()
        .route("/projects/{id}/remote-instances/{link}/cloud", put(put_setup).delete(delete_setup))
        .route("/projects/{id}/remote-instances/{link}/cloud/start", post(start))
        .route("/projects/{id}/remote-instances/{link}/cloud/stop", post(stop))
}
