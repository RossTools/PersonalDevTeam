//! Remote access: `tailscale serve` maps https://<machine>.<tailnet>.ts.net/ to pdtd, and the tailnet name
//! joins the Host/Origin allowlist. The setting is the truth; this makes reality match it and reports what happened.
//!
//! The machine's Tailscale is shared with every other instance on it (the stable one serves 443 there), so this
//! process only ever undoes what it did itself (ruling F1): with the setting off and nothing mapped by this process,
//! applying runs no tailscale command at all, and turning the setting off unmaps only the ports this process mapped.
//! `serve --bg` mappings outlive pdtd, so the ports this instance mapped are kept in its store (`remote.served`) and
//! count as its own after a restart: turning the setting off removes them, and so does resuming the runs when a run
//! moved to another port or did not come back.
//!
//! A web run's port is never mapped to the run itself: it is mapped to a proxy of pdtd's ([`crate::remote_proxy`])
//! that applies the owner check first, so the run is reachable only by the owner, like pdtd.

use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use pdt_tailscale::TsError;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::Ordering;
use std::time::Duration;
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

/// The host value that keeps [`Applied::served`] across restarts.
const SERVED_KEY: &str = "remote.served";

/// What this instance changed, so that turning remote access off undoes exactly that.
#[derive(Debug, Default)]
pub(crate) struct Applied {
    /// HTTPS ports this instance mapped with `tailscale serve`, by this process or a previous one (kept under
    /// [`SERVED_KEY`]; changed only under `settings_lock`).
    pub(crate) served: BTreeSet<u16>,
    /// Whether `served` has had the stored ports added (once per process, on first use).
    loaded: bool,
    /// The tailnet name this process added to the allowlist (absent when it was already allowed by configuration).
    pub(crate) added_host: Option<String>,
    /// The owner-checking proxy in front of each mapped web run, by the run's port (the HTTPS port it is mapped on).
    proxies: HashMap<u16, crate::remote_proxy::RunProxy>,
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
    load_served(st).await;
    if !st.remote.lock().unwrap_or_else(|e| e.into_inner()).active {
        return;
    }
    if let Err(e) = serve_run(st, port).await {
        tracing::warn!(port, error = %e, "could not map a run port");
    }
}

/// Removes a run port's mapping, but only one this instance made (ruling F1); pdtd's own HTTPS port is never touched.
pub(crate) async fn unmap_run_port(st: &AppState, port: u16) {
    let _guard = st.settings_lock.lock().await;
    load_served(st).await;
    unmap_locked(st, port).await;
    save_served(st).await;
}

/// Removes the run-port mappings a previous process made for runs this one does not drive (a run that moved to
/// another port, failed to resume or whose row was removed). pdtd awaits it after resuming the runs.
pub(crate) async fn unmap_stale_run_ports(st: &AppState) {
    let _guard = st.settings_lock.lock().await;
    load_served(st).await;
    let driven = extra_ports(st);
    let stale: Vec<u16> =
        applied(st).served.iter().copied().filter(|p| *p != st.remote_https_port && !driven.contains(p)).collect();
    for p in stale {
        unmap_locked(st, p).await;
    }
    save_served(st).await;
}

/// [`unmap_run_port`] for callers that hold `settings_lock` and save `served` afterwards.
async fn unmap_locked(st: &AppState, port: u16) {
    if port == st.remote_https_port {
        return;
    }
    // Without its proxy the mapping leads nowhere, even when removing it fails below.
    applied(st).proxies.remove(&port);
    if !applied(st).served.contains(&port) {
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

/// Adds the ports a previous process mapped to `served`, once. A stored value that cannot be read is logged and
/// left alone (`save_served` then replaces it).
async fn load_served(st: &AppState) {
    if applied(st).loaded {
        return;
    }
    let stored: Vec<u16> = match st.store.get_host_value(SERVED_KEY).await {
        Ok(v) => v.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default(),
        Err(e) => {
            tracing::warn!(error = %e, "could not read the ports a previous process mapped");
            Vec::new()
        }
    };
    let mut a = applied(st);
    a.served.extend(stored);
    a.loaded = true;
}

/// Stores `served`, so the next process knows which mappings are this instance's.
async fn save_served(st: &AppState) {
    let ports: Vec<u16> = applied(st).served.iter().copied().collect();
    let r = if ports.is_empty() {
        st.store.delete_host_value(SERVED_KEY).await
    } else {
        st.store.put_host_value(SERVED_KEY, &serde_json::json!(ports)).await
    };
    if let Err(e) = r {
        tracing::warn!(error = %e, "could not store the ports this instance mapped");
    }
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
    load_served(st).await;
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
        if let Err(e) = serve_run(st, p).await {
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
    if matches!(r, Ok(()) | Err(TsError::TimedOut)) && applied(st).served.insert(https_port) {
        save_served(st).await;
    }
    r
}

/// Maps a web run's `port` on the tailnet to the run's owner-checking proxy, starting the proxy first when the run
/// has none. A proxy started here is stopped again when the mapping failed outright.
async fn serve_run(st: &AppState, port: u16) -> Result<(), TsError> {
    let existing = applied(st).proxies.get(&port).map(|p| p.port);
    let (proxy_port, fresh) = match existing {
        Some(p) => (p, false),
        None => {
            let proxy = crate::remote_proxy::start(st, port)
                .await
                .map_err(|e| TsError::Command(format!("could not start the run's proxy: {e}")))?;
            let p = proxy.port;
            applied(st).proxies.insert(port, proxy);
            (p, true)
        }
    };
    let r = serve(st, port, proxy_port).await;
    if fresh && matches!(&r, Err(e) if !matches!(e, TsError::TimedOut)) {
        applied(st).proxies.remove(&port);
    }
    r
}

/// Undoes what this instance did, and nothing else: no tailscale command runs when it mapped nothing.
async fn disable(st: &AppState, previous: RemoteStatus) -> RemoteStatus {
    let (served, added_host) = {
        let mut a = applied(st);
        a.proxies.clear();
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
    save_served(st).await;
    RemoteStatus { enabled: false, error, tailscale_login: previous.tailscale_login, ..Default::default() }
}

/// How often [`run_remote_watch`] looks at remote access (PDT-148).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteWatch {
    /// The first retry after remote access is enabled but not active; each failed retry doubles the wait, up to
    /// `retry_max`.
    pub retry_first: Duration,
    pub retry_max: Duration,
    /// How often active remote access is checked (one `tailscale status`), and how often the loop looks while the
    /// setting is off (then it runs no tailscale command).
    pub check_every: Duration,
}

impl Default for RemoteWatch {
    fn default() -> Self {
        Self {
            retry_first: Duration::from_secs(30),
            retry_max: Duration::from_secs(5 * 60),
            check_every: Duration::from_secs(3 * 60),
        }
    }
}

impl RemoteWatch {
    /// The wait before the next pass after `failures` passes in a row left remote access enabled but not active.
    pub fn next_wait(&self, failures: u32) -> Duration {
        match failures {
            0 => self.check_every,
            n => self.retry_first.saturating_mul(1 << (n - 1).min(16)).min(self.retry_max),
        }
    }
}

/// The remote-access watch's name among the background loops.
const WATCH_LOOP: &str = "remote";

/// Keeps remote access in step with Tailscale, until shutdown (PDT-148). While the setting is on but remote access
/// is not active (Tailscale stopped or not logged in yet, a CLI timing out), it applies the setting again, backing
/// off from [`RemoteWatch::retry_first`] to [`RemoteWatch::retry_max`]; while it is active, it asks `tailscale status`
/// every [`RemoteWatch::check_every`] and applies again only when Tailscale stopped or its name changed. With the
/// setting off it runs no tailscale command (ruling F1). pdtd starts it after the startup apply.
pub async fn run_remote_watch(st: AppState) {
    let timing = st.remote_watch;
    let longest = timing.check_every.max(timing.retry_max);
    let stale = pdt_core::liveness::stale_after(i64::try_from(longest.as_millis()).unwrap_or(i64::MAX));
    st.loops.register(WATCH_LOOP, timing.check_every, Some(stale));
    let mut failures = u32::from(not_active(&st));
    loop {
        tokio::time::sleep(timing.next_wait(failures)).await;
        if st.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let pass = st.clone();
        let ended = crate::loops::contain(WATCH_LOOP, None, async move { watch_pass(&pass).await }).await;
        st.loops.passed(WATCH_LOOP, crate::routes::now_ms(), ended == crate::loops::Ended::Panicked);
        failures = if not_active(&st) { failures.saturating_add(1) } else { 0 };
    }
}

/// The last apply left remote access enabled but not active.
fn not_active(st: &AppState) -> bool {
    let r = st.remote.lock().unwrap_or_else(|e| e.into_inner());
    r.enabled && !r.active
}

/// One look: nothing with the setting off; a `tailscale status` while active, applying again only on a change;
/// otherwise an apply.
async fn watch_pass(st: &AppState) {
    match st.store.get_host_settings().await {
        Ok(s) if s.remote_access => {}
        Ok(_) => return,
        Err(e) => {
            tracing::warn!(error = %e, "could not read the host settings to check remote access");
            return;
        }
    }
    let current = st.remote.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if current.active {
        let seen = blocking(st, |t| t.status()).await;
        match &seen {
            Ok(s) if s.https_enabled && current.dns_name.as_deref() == Some(s.dns_name.as_str()) => return,
            Ok(s) => tracing::info!(dns_name = %s.dns_name, "Tailscale changed; applying remote access again"),
            Err(e) => tracing::info!(error = %e, "Tailscale stopped answering; applying remote access again"),
        }
    }
    let r = apply_remote_access(st).await;
    if r.active != current.active {
        tracing::info!(active = r.active, url = ?r.url, error = ?r.error, "remote access changed");
    } else if !r.active {
        tracing::debug!(error = ?r.error, "remote access is still not active");
    }
}

pub(crate) async fn get_remote(State(st): State<AppState>) -> Json<RemoteStatus> {
    Json(st.remote.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

pub(crate) async fn refresh(State(st): State<AppState>) -> Result<Json<RemoteStatus>, ApiError> {
    Ok(Json(apply_remote_access(&st).await))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new().route("/host/remote", get(get_remote)).route("/host/remote/refresh", post(refresh))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_back_off_to_the_cap_and_active_access_is_checked_steadily() {
        let w = RemoteWatch::default();
        let secs = |n| w.next_wait(n).as_secs();
        assert_eq!(secs(0), 180, "active (or off): one check every three minutes");
        assert_eq!([secs(1), secs(2), secs(3), secs(4), secs(5)], [30, 60, 120, 240, 300]);
        assert_eq!(secs(u32::MAX), 300, "never past the cap, never overflowing");
    }
}
