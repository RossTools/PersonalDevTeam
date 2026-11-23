//! The owner's side of remote projects (PDT-631): Project Settings → Remote instances. A pairing code to show, the
//! instances joined to the project with what their heartbeats said, a heartbeat on demand, and Remove (which revokes
//! the link at both ends). The link itself is `crate::link`.

use crate::error::{ApiError, codes};
use crate::extract::{ApiJson, ApiPath};
use crate::link;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use pdt_core::link::{LinkState, RemoteInstance, Side};
use pdt_core::project::Project;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An instance joined to the project, as Remote instances lists it.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteInstanceView {
    #[serde(flatten)]
    pub instance: RemoteInstance,
    pub state: LinkState,
    /// What to do about an incompatible one ("update win-box to v0.7.2 to link it with studio").
    pub problem: Option<String>,
    /// The most of the project's tickets in flight on it at once: the lower of this project's cap for it and its own.
    pub pool_cap: u32,
}

/// A joined instance's settings on the owner.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteInstancePatch {
    /// The most of the project's tickets in flight on it at once; 0: no cap set here (its own, else one).
    #[serde(default)]
    #[ts(optional)]
    pub cap: Option<u32>,
}

/// A link ended from this end: Remove on the owner, Leave on a remote instance.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LinkEnded {
    /// The other end was told. False: it couldn't be reached, and still lists the project until it finds out (or is
    /// removed there too).
    pub told: bool,
}

/// A pairing code on show.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PairingView {
    pub code: String,
    #[ts(type = "number")]
    pub expires_at: i64,
}

/// Project Settings → Remote instances.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteInstancesView {
    /// This instance's name.
    pub name: String,
    /// The address a remote instance enters to reach this one (remote access's tailnet address); null while remote
    /// access is off, which pairing needs.
    pub address: Option<String>,
    pub instances: Vec<RemoteInstanceView>,
    pub pairing: Option<PairingView>,
}

pub(crate) fn view_of(i: RemoteInstance, me: Side<'_>, now: i64) -> RemoteInstanceView {
    let compatible = pdt_core::link::compatible(me.protocol, i.protocol);
    let state = pdt_core::link::state(i.last_seen_at, compatible, i.paused, now);
    let problem = (!compatible).then(|| {
        let them = Side { name: &i.name, protocol: i.protocol, version: &i.version };
        pdt_core::link::incompatible_message(them, me)
    });
    let pool_cap = i.pool_cap();
    RemoteInstanceView { instance: i, state, problem, pool_cap }
}

/// A project of this instance's own that can have remote instances: one with a repository.
async fn own_project(st: &AppState, id: &str) -> Result<Project, ApiError> {
    let p = st.project(id).await?;
    if !p.has_repo() {
        return Err(ApiError::bad_request(
            codes::PERSONAL_PROJECT,
            format!("{} has no repository, so no other instance can work on it", p.name),
        ));
    }
    Ok(p)
}

async fn view(st: &AppState, project: &Project) -> Result<RemoteInstancesView, ApiError> {
    let me = link::hello(st).await?;
    let us = Side { name: &me.name, protocol: me.protocol, version: &me.version };
    let now = now_ms();
    let instances =
        st.store.list_remote_instances(Some(&project.id)).await?.into_iter().map(|i| view_of(i, us, now)).collect();
    let pairing = st.link.code_for(&project.id, now).map(|p| PairingView { code: p.code, expires_at: p.expires_at });
    Ok(RemoteInstancesView { name: me.name.clone(), address: link::own_url(st), instances, pairing })
}

async fn list(State(st): State<AppState>, ApiPath(id): ApiPath<String>) -> Result<Json<RemoteInstancesView>, ApiError> {
    let p = own_project(&st, &id).await?;
    Ok(Json(view(&st, &p).await?))
}

/// Shows a new pairing code for the project (replacing the one on show).
async fn new_pairing(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<RemoteInstancesView>, ApiError> {
    let p = own_project(&st, &id).await?;
    st.link.new_code(&p.id, now_ms());
    Ok(Json(view(&st, &p).await?))
}

async fn cancel_pairing(State(st): State<AppState>, ApiPath(id): ApiPath<String>) -> Result<StatusCode, ApiError> {
    let p = own_project(&st, &id).await?;
    st.link.cancel_code(&p.id);
    Ok(StatusCode::NO_CONTENT)
}

/// Sends every joined instance a heartbeat now, asking each to check its agents again.
async fn check(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<RemoteInstancesView>, ApiError> {
    let p = own_project(&st, &id).await?;
    let instances = st.store.list_remote_instances(Some(&p.id)).await?;
    futures_util::future::join_all(instances.iter().map(|i| link::heartbeat(&st, i, true))).await;
    Ok(Json(view(&st, &p).await?))
}

/// Sets a joined instance's cap.
async fn patch(
    State(st): State<AppState>,
    ApiPath((id, link_id)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<RemoteInstancePatch>,
) -> Result<Json<RemoteInstancesView>, ApiError> {
    let p = own_project(&st, &id).await?;
    if st.store.get_remote_instance(&link_id).await?.is_none_or(|i| i.project_id != p.id) {
        return Err(ApiError::not_found("remote instance"));
    }
    if let Some(cap) = body.cap {
        st.store.set_remote_instance_cap(&link_id, Some(cap).filter(|c| *c > 0)).await?;
    }
    Ok(Json(view(&st, &p).await?))
}

/// Removes a joined instance: it is told (so it drops the remote project and its key), and the owner's key and row
/// go. An instance that can't be reached is removed all the same (its key no longer signs anything this one takes),
/// and the reply says it wasn't told: it finds out when it next asks.
async fn remove(
    State(st): State<AppState>,
    ApiPath((id, link_id)): ApiPath<(String, String)>,
) -> Result<Json<LinkEnded>, ApiError> {
    let p = own_project(&st, &id).await?;
    let Some(i) = st.store.get_remote_instance(&link_id).await?.filter(|i| i.project_id == p.id) else {
        return Err(ApiError::not_found("remote instance"));
    };
    let told = link::revoke(&st, &i, &p.id).await;
    st.store.delete_remote_instance(&i.link_id).await?;
    tracing::info!(project = %p.key, instance = %i.name, told, "removed a remote instance");
    Ok(Json(LinkEnded { told }))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{delete, get, post};
    axum::Router::new()
        .route("/projects/{id}/remote-instances", get(list))
        .route("/projects/{id}/remote-instances/pairing", post(new_pairing).delete(cancel_pairing))
        .route("/projects/{id}/remote-instances/check", post(check))
        .route("/projects/{id}/remote-instances/{link}", delete(remove).patch(patch))
}
