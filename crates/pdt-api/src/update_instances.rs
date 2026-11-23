//! Updating instances together (PDT-637). On a project's owner, the update step lists the instances joined to its
//! projects, says which need updating (an older version, or a protocol the link can't speak: "update win-box to
//! v0.7.3 to link it with studio") and asks one to update. The remote instance checks for an update on the owner's
//! channel (unless its remote project says to use its own), and installs it the way its own Install would: busy
//! sessions ask first, and pdt-host takes over. The request is signed for a project's link and taken whatever protocol
//! the owner speaks, so a mismatch can always be fixed from the owner.

use crate::error::{ApiError, codes};
use crate::extract::ApiPath;
use crate::link::{self, PeerError};
use crate::routes::{AppState, now_ms};
use crate::update::{self, CheckResult};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, Method};
use pdt_core::host::UpdateChannel;
use pdt_core::link::{LinkState, Side};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use ts_rs::TS;

/// How long the owner waits for a joined instance's check (it asks GitHub).
const UPDATE_CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// A joined instance, as the update step lists it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstanceUpdateView {
    pub instance_id: String,
    pub name: String,
    pub version: String,
    pub protocol: u32,
    pub state: LinkState,
    /// The keys of this instance's projects it joined.
    pub projects: Vec<String>,
    /// What it needs, when it needs updating ("update win-box to v0.7.3").
    pub needed: Option<String>,
}

/// The update step's joined instances.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstancesUpdateView {
    /// This instance's name and version.
    pub name: String,
    pub version: String,
    /// The channel they update on unless their remote project says otherwise (this instance's).
    pub channel: UpdateChannel,
    pub instances: Vec<InstanceUpdateView>,
}

/// What a joined instance did when asked to update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InstanceUpdateOutcome {
    /// It is downloading the update, and installs it once verified (it restarts).
    Installing,
    /// It runs the newest on its channel already.
    UpToDate,
}

/// The owner asks a joined instance to update (`/link/update`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateReq {
    /// The owner's channel.
    pub channel: UpdateChannel,
    /// Update even when its agents are working (they resume afterwards).
    pub force: bool,
}

/// `POST /api/host/update/instances/{instanceId}`'s answer, and the remote instance's to `/link/update`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstanceUpdateReply {
    pub name: String,
    pub outcome: InstanceUpdateOutcome,
    /// The version it installs (installing) or runs (up to date).
    pub version: String,
    /// The channel it checked.
    pub channel: UpdateChannel,
}

#[derive(Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInstanceBody {
    /// Update even when its agents are working.
    #[serde(default)]
    #[ts(optional)]
    pub force: Option<bool>,
}

async fn host_channel(st: &AppState) -> UpdateChannel {
    st.store.get_host_settings().await.map(|s| s.update_channel).unwrap_or_default()
}

/// `GET /api/host/update/instances`.
async fn list(State(st): State<AppState>) -> Result<Json<InstancesUpdateView>, ApiError> {
    let me = link::hello(&st).await?;
    let us = Side { name: &me.name, protocol: me.protocol, version: &me.version };
    let now = now_ms();
    let mut instances: Vec<InstanceUpdateView> = vec![];
    for i in st.store.list_remote_instances(None).await? {
        let key = st.store.get_project(&i.project_id).await?.map(|p| p.key).unwrap_or_default();
        let compatible = pdt_core::link::compatible(me.protocol, i.protocol);
        let state = pdt_core::link::state(i.last_seen_at, compatible, i.paused, now);
        if let Some(seen) = instances.iter_mut().find(|v| v.instance_id == i.instance_id) {
            seen.projects.push(key);
            // The best state any of its links has.
            if seen.state != LinkState::Connected && state == LinkState::Connected {
                seen.state = state;
            }
            continue;
        }
        let them = Side { name: &i.name, protocol: i.protocol, version: &i.version };
        instances.push(InstanceUpdateView {
            needed: pdt_core::link::update_needed(them, us),
            instance_id: i.instance_id,
            name: i.name,
            version: i.version,
            protocol: i.protocol,
            state,
            projects: vec![key],
        });
    }
    Ok(Json(InstancesUpdateView { name: me.name, version: me.version, channel: host_channel(&st).await, instances }))
}

/// `POST /api/host/update/instances/{instanceId}`: asks the instance to update, through the first of its links
/// that reaches it.
async fn update_one(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<InstanceUpdateReply>, ApiError> {
    let body: UpdateInstanceBody = crate::host_guard::optional_json(&headers, &body)?;
    let force = body.force.unwrap_or(false);
    let links: Vec<_> =
        st.store.list_remote_instances(None).await?.into_iter().filter(|i| i.instance_id == id).collect();
    let Some(first) = links.first() else {
        return Err(ApiError::not_found("remote instance"));
    };
    let name = first.name.clone();
    let req = UpdateReq { channel: host_channel(&st).await, force };
    let mut unreachable = None;
    for i in &links {
        let key = link::load_key(&st, &i.link_id).await?;
        let signed = Some((&key, i.link_id.as_str(), i.project_id.as_str()));
        match link::post_within(&st, UPDATE_CALL_TIMEOUT, &i.url, link::UPDATE_PATH, signed, &req).await {
            Ok(reply) => return Ok(Json(reply)),
            Err(e @ PeerError::Unreachable(_)) => unreachable = Some(e),
            Err(e) => return Err(refused(e, &name)),
        }
    }
    Err(unreachable.map(|e| e.into_api(&name)).unwrap_or_else(|| ApiError::not_found("remote instance")))
}

/// What the owner says about a joined instance's refusal: what it said, under its name.
fn refused(e: PeerError, name: &str) -> ApiError {
    match e {
        // A build from before updating together has no such route.
        PeerError::Refused { status: 404 | 405, .. } => ApiError::conflict(
            codes::UPDATES_DISABLED,
            format!("{name} can't be updated from here: update it on {name} itself"),
        ),
        PeerError::Refused { code, message, .. }
            if [codes::HOST_BUSY, codes::UPDATES_DISABLED, codes::UPDATE_FAILED, codes::UPDATE_IN_PROGRESS]
                .contains(&code.as_str()) =>
        {
            let code = [codes::HOST_BUSY, codes::UPDATES_DISABLED, codes::UPDATE_FAILED, codes::UPDATE_IN_PROGRESS]
                .into_iter()
                .find(|c| *c == code)
                .unwrap_or(codes::UPDATE_FAILED);
            ApiError::conflict(code, format!("{name}: {message}"))
        }
        other => other.into_api(name),
    }
}

/// `/link/update` on the remote instance: checks on the owner's channel (or its own) and installs what it finds.
pub(crate) async fn link_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<InstanceUpdateReply>, ApiError> {
    let link = link::from_owner_any_protocol(&st, &method, &uri, &headers, &body).await?;
    let req: UpdateReq = serde_json::from_slice(&body).map_err(|e| ApiError::invalid(e.to_string()))?;
    let name = link::instance_name(&st).await?;
    if !st.update.enabled {
        let why = st.update.snapshot().disabled_reason.unwrap_or_else(|| "updates are off on this instance".into());
        return Err(ApiError::conflict(codes::UPDATES_DISABLED, why));
    }
    let channel = (!link.own_update_channel).then_some(req.channel);
    let (status, result) = update::check_on(&st, channel).await;
    let version = match result {
        CheckResult::UpToDate { version } => {
            let reply = InstanceUpdateReply {
                name,
                outcome: InstanceUpdateOutcome::UpToDate,
                version,
                channel: status.channel,
            };
            return Ok(Json(reply));
        }
        CheckResult::CannotCheck { message, .. } => return Err(ApiError::conflict(codes::UPDATE_FAILED, message)),
        CheckResult::Available { version } => version,
    };
    if !req.force {
        let busy = update::busy_reasons(&st).await;
        if !busy.is_empty() {
            return Err(update::host_busy(&busy));
        }
    }
    tracing::info!(owner = %link.owner_name, version = %version, "the owner asked this instance to update");
    let bg = st.clone();
    tokio::spawn(async move {
        if let Err(e) = update::install(&bg, req.force, false).await {
            tracing::warn!(error = %e.message(), "the update the owner asked for failed");
        }
    });
    Ok(Json(InstanceUpdateReply { name, outcome: InstanceUpdateOutcome::Installing, version, channel: status.channel }))
}

/// This module's API routes (the owner's); `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/host/update/instances", get(list))
        .route("/host/update/instances/{id}", post(update_one))
}
