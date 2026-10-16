//! Remote access (spec §2): `tailscale serve` maps https://<machine>.<tailnet>.ts.net/ to pdtd, and the tailnet name
//! joins the Host/Origin allowlist. The setting is the truth; this makes reality match it and reports what happened.
//!
//! The machine's Tailscale is shared with every other instance on it (the stable one serves 443 there), so this
//! process only ever undoes what it did itself (ruling F1): with the setting off and nothing mapped by this process,
//! applying runs no tailscale command at all, and turning the setting off unmaps only the ports this process mapped.

use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use pdt_tailscale::TsError;
use serde::Serialize;
use std::collections::BTreeSet;
use ts_rs::TS;

pub const HOST_HTTPS_PORT: u16 = 443;

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteStatus {
    /// The setting.
    pub enabled: bool,
    /// The mapping is in place and the tailnet name is allowed.
    pub active: bool,
    pub url: Option<String>,
    pub dns_name: Option<String>,
    /// Why it is not active (e.g. "Tailscale is not running", "tailscale CLI not found").
    pub error: Option<String>,
    /// The login this machine's Tailscale is signed in as (offered as the owner).
    pub tailscale_login: Option<String>,
}

/// What this process changed, so that turning remote access off undoes exactly that.
#[derive(Debug, Default)]
pub(crate) struct Applied {
    /// HTTPS ports this process mapped with `tailscale serve`.
    pub(crate) served: BTreeSet<u16>,
    /// The tailnet name this process added to the allowlist (absent when it was already allowed by configuration).
    pub(crate) added_host: Option<String>,
}

async fn blocking<T: Send + 'static>(
    st: &AppState,
    f: impl FnOnce(&dyn pdt_tailscale::TailscaleApi) -> Result<T, TsError> + Send + 'static,
) -> Result<T, TsError> {
    let ts = st.tailscale.clone();
    tokio::task::spawn_blocking(move || f(ts.as_ref())).await.unwrap_or_else(|e| Err(TsError::Command(e.to_string())))
}

/// Ports of other things to map alongside pdtd (web runs, Task 7).
pub(crate) async fn extra_ports(_st: &AppState) -> Vec<u16> {
    Vec::new()
}

fn applied(st: &AppState) -> std::sync::MutexGuard<'_, Applied> {
    st.remote_applied.lock().unwrap_or_else(|e| e.into_inner())
}

/// Applies the stored setting: serve or unserve, update the allowlist, and record the outcome. Callers that can race
/// a settings change hold `settings_lock`; pdtd awaits it once at startup.
pub async fn apply_remote_access(st: &AppState) -> RemoteStatus {
    let previous = st.remote.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let out = match st.store.get_host_settings().await {
        // Without the setting, change nothing.
        Err(e) => RemoteStatus { error: Some(format!("could not read the host settings: {e}")), ..previous },
        Ok(s) if s.remote_access => enable(st).await,
        Ok(_) => disable(st, previous).await,
    };
    *st.remote.lock().unwrap_or_else(|e| e.into_inner()) = out.clone();
    out
}

async fn enable(st: &AppState) -> RemoteStatus {
    let status = blocking(st, |t| t.status()).await;
    let mut out = RemoteStatus {
        enabled: true,
        tailscale_login: status.as_ref().ok().and_then(|s| s.self_login.clone()),
        ..Default::default()
    };
    let s = match status {
        Err(e) => {
            out.error = Some(e.to_string());
            return out;
        }
        Ok(s) if !s.https_enabled => {
            out.error = Some(TsError::HttpsDisabled.to_string());
            return out;
        }
        Ok(s) => s,
    };
    let port = st.listen_port;
    if let Err(e) = blocking(st, move |t| t.serve(HOST_HTTPS_PORT, port)).await {
        out.error = Some(e.to_string());
        return out;
    }
    applied(st).served.insert(HOST_HTTPS_PORT);
    for p in extra_ports(st).await {
        match blocking(st, move |t| t.serve(p, p)).await {
            Ok(()) => {
                applied(st).served.insert(p);
            }
            Err(e) => tracing::warn!(port = p, error = %e, "could not map a run port"),
        }
    }
    {
        let mut a = applied(st);
        if let Some(old) = a.added_host.take_if(|h| *h != s.dns_name) {
            st.allowed_hosts.remove(&old);
        }
        if a.added_host.is_none() && !st.allowed_hosts.contains(&s.dns_name) {
            st.allowed_hosts.add(&s.dns_name);
            a.added_host = Some(s.dns_name.clone());
        }
    }
    out.active = true;
    out.url = Some(format!("https://{}/", s.dns_name));
    out.dns_name = Some(s.dns_name);
    out
}

/// Undoes what this process did, and nothing else: no tailscale command runs when it mapped nothing.
async fn disable(st: &AppState, previous: RemoteStatus) -> RemoteStatus {
    let (served, added_host) = {
        let mut a = applied(st);
        (std::mem::take(&mut a.served), a.added_host.take())
    };
    if let Some(host) = added_host {
        st.allowed_hosts.remove(&host);
    }
    let mut failed = Vec::new();
    for p in served {
        if let Err(e) = blocking(st, move |t| t.unserve(p)).await {
            tracing::warn!(port = p, error = %e, "could not remove a tailscale serve mapping");
            failed.push((p, e));
        }
    }
    let error = failed.first().map(|(p, e)| format!("could not remove the mapping on port {p}: {e}"));
    // Keep what is still mapped, so a later refresh retries it.
    applied(st).served.extend(failed.iter().map(|(p, _)| *p));
    RemoteStatus { enabled: false, error, tailscale_login: previous.tailscale_login, ..Default::default() }
}

pub(crate) async fn get_remote(State(st): State<AppState>) -> Json<RemoteStatus> {
    Json(st.remote.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

pub(crate) async fn refresh(State(st): State<AppState>) -> Result<Json<RemoteStatus>, ApiError> {
    let _guard = st.settings_lock.lock().await;
    Ok(Json(apply_remote_access(&st).await))
}
