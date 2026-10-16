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

/// Runs one tailscale call on the blocking pool, bounded by `st.tailscale_timeout` so a hung CLI cannot stall
/// startup or hold `settings_lock` (the real CLI also kills its process at its own timeout).
async fn blocking<T: Send + 'static>(
    st: &AppState,
    f: impl FnOnce(&dyn pdt_tailscale::TailscaleApi) -> Result<T, TsError> + Send + 'static,
) -> Result<T, TsError> {
    let ts = st.tailscale.clone();
    let call = tokio::task::spawn_blocking(move || f(ts.as_ref()));
    match tokio::time::timeout(st.tailscale_timeout, call).await {
        Ok(joined) => joined.unwrap_or_else(|e| Err(TsError::Command(e.to_string()))),
        Err(_) => Err(TsError::TimedOut),
    }
}

/// The ports of the web runs this process is driving, mapped alongside pdtd while remote access is active. Rows a
/// previous pdtd left are not included until they are resumed: their old ports may now belong to the machine's
/// other instance.
fn extra_ports(st: &AppState) -> Vec<u16> {
    crate::run_manager::driven_web_runs(st).into_iter().map(|(_, p)| p).collect()
}

/// Maps a web run's port to itself while remote access is active, recording it as this process's mapping (so
/// turning remote access off removes it). With remote access inactive it runs no tailscale command.
pub(crate) async fn map_run_port(st: &AppState, port: u16) {
    let _guard = st.settings_lock.lock().await;
    if !st.remote.lock().unwrap_or_else(|e| e.into_inner()).active {
        return;
    }
    if let Err(e) = serve(st, port, port).await {
        tracing::warn!(port, error = %e, "could not map a run port");
    }
}

/// Removes a run port's mapping, but only one this process made (ruling F1); pdtd's own HTTPS port is never touched.
pub(crate) async fn unmap_run_port(st: &AppState, port: u16) {
    let _guard = st.settings_lock.lock().await;
    if port == st.remote_https_port || !applied(st).served.contains(&port) {
        return;
    }
    match blocking(st, move |t| t.unserve(port)).await {
        Ok(()) => {
            applied(st).served.remove(&port);
        }
        // Still recorded, so turning remote access off retries it.
        Err(e) => tracing::warn!(port, error = %e, "could not remove a run port's mapping"),
    }
}

fn applied(st: &AppState) -> std::sync::MutexGuard<'_, Applied> {
    st.remote_applied.lock().unwrap_or_else(|e| e.into_inner())
}

/// The Host value browsers send for `https://<dns_name>:<port>/` (no port for 443).
fn tailnet_host(dns_name: &str, https_port: u16) -> String {
    if https_port == HOST_HTTPS_PORT { dns_name.to_string() } else { format!("{dns_name}:{https_port}") }
}

/// Applies the stored setting: serve or unserve, update the allowlist, and record the outcome. Takes
/// `settings_lock`, so it cannot interleave with a settings change (pdtd awaits it at startup).
pub async fn apply_remote_access(st: &AppState) -> RemoteStatus {
    let _guard = st.settings_lock.lock().await;
    apply_locked(st).await
}

/// [`apply_remote_access`] for callers that already hold `settings_lock`.
pub(crate) async fn apply_locked(st: &AppState) -> RemoteStatus {
    let previous = st.remote.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let before = previous.active.then(|| previous.dns_name.clone()).flatten();
    let out = match st.store.get_host_settings().await {
        // Without the setting, change nothing.
        Err(e) => RemoteStatus { error: Some(format!("could not read the host settings: {e}")), ..previous },
        Ok(s) if s.remote_access => enable(st).await,
        Ok(_) => disable(st, previous).await,
    };
    // Running web runs' links follow the outcome (their ports were mapped or unmapped above).
    let tailnet = out.active.then_some(out.dns_name.as_deref()).flatten();
    crate::run_manager::relink_runs(st, tailnet).await;
    let host_changed = before.as_deref() != tailnet;
    *st.remote.lock().unwrap_or_else(|e| e.into_inner()) = out.clone();
    // Runs that baked the old host into their processes restart with the new one (final review #9).
    if host_changed {
        crate::run_manager::restart_host_dependent_runs(st);
    }
    out
}

/// Maps the HTTPS port to pdtd and allows the tailnet name. On failure the name is withdrawn from the allowlist, so
/// `active` and the allowlist always agree; mappings stay recorded so turning off still removes them.
async fn enable(st: &AppState) -> RemoteStatus {
    let status = blocking(st, |t| t.status()).await;
    let mut out = RemoteStatus {
        enabled: true,
        tailscale_login: status.as_ref().ok().and_then(|s| s.self_login.clone()),
        ..Default::default()
    };
    let failure = match status {
        Err(e) => Err(e),
        Ok(s) if !s.https_enabled => Err(TsError::HttpsDisabled),
        Ok(s) => {
            let (https_port, target) = (st.remote_https_port, st.listen_port);
            match serve(st, https_port, target).await {
                Err(e) => Err(e),
                Ok(()) => Ok((s, https_port)),
            }
        }
    };
    let (s, https_port) = match failure {
        Ok(ok) => ok,
        Err(e) => {
            if let Some(host) = applied(st).added_host.take() {
                st.allowed_hosts.remove(&host);
            }
            out.error = Some(e.to_string());
            return out;
        }
    };
    for p in extra_ports(st) {
        if let Err(e) = serve(st, p, p).await {
            tracing::warn!(port = p, error = %e, "could not map a run port");
        }
    }
    let host = tailnet_host(&s.dns_name, https_port);
    {
        let mut a = applied(st);
        if let Some(old) = a.added_host.take_if(|h| *h != host) {
            st.allowed_hosts.remove(&old);
        }
        if a.added_host.is_none() && !st.allowed_hosts.contains(&host) {
            st.allowed_hosts.add(&host);
            a.added_host = Some(host.clone());
        }
    }
    out.active = true;
    out.url = Some(format!("https://{host}/"));
    out.dns_name = Some(s.dns_name);
    out
}

/// One `tailscale serve`, recorded as this process's mapping when it succeeded or timed out (it may still land).
async fn serve(st: &AppState, https_port: u16, target_port: u16) -> Result<(), TsError> {
    let r = blocking(st, move |t| t.serve(https_port, target_port)).await;
    if matches!(r, Ok(()) | Err(TsError::TimedOut)) {
        applied(st).served.insert(https_port);
    }
    r
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
    Ok(Json(apply_remote_access(&st).await))
}
