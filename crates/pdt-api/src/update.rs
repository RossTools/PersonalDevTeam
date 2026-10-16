//! Update checks and installs (remote-host spec §3.3–3.4). pdtd checks hourly (unless the policy is manual) and on
//! request, downloads and verifies a release, writes `update/request.json` and shuts down; pdt-host installs it.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use crate::secrets::UPDATE_TOKEN;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use pdt_core::activity::Activity;
use pdt_core::handoff::{UpdateRequest, UpdateResult, downloads_dir, request_path, result_path};
use pdt_core::host::UpdatePolicy;
use pdt_core::release::{HostOs, TokenState, parse_version, token_state};
use pdt_core::run::RunState;
use pdt_core::session::SessionStatus;
use pdt_update::{GitHubClient, Release, UpdateError, fetch_verified};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UpdateState {
    Disabled,
    Idle,
    Checking,
    Available,
    Downloading,
    Installing,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleaseSummary {
    pub version: String,
    pub tag: String,
    pub notes: String,
    /// GitHub's `published_at` as GitHub sends it (an ISO 8601 UTC string, e.g. `2026-10-15T00:00:00Z`), not Unix
    /// ms: it is only displayed (ruling F17).
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateStatus {
    pub current: String,
    pub state: UpdateState,
    pub disabled_reason: Option<String>,
    pub policy: UpdatePolicy,
    pub latest: Option<ReleaseSummary>,
    #[ts(type = "number | null")]
    pub last_checked_at: Option<i64>,
    pub error: Option<String>,
    pub token: TokenState,
    pub last_result: Option<UpdateResult>,
}

pub struct UpdateService {
    pub(crate) client: GitHubClient,
    pub(crate) supervised: bool,
    current: String,
    status: Mutex<UpdateStatus>,
    pub(crate) release: Mutex<Option<Release>>,
    running: tokio::sync::Mutex<()>,
}

impl UpdateService {
    pub fn new(api_base: &str, repo: &str, supervised: bool, current: &str) -> Self {
        Self {
            client: GitHubClient::new(api_base, repo),
            supervised,
            current: current.into(),
            status: Mutex::new(UpdateStatus {
                current: current.into(),
                state: if supervised { UpdateState::Idle } else { UpdateState::Disabled },
                disabled_reason: (!supervised)
                    .then(|| "updates are installed by pdt-host, and this pdtd is not running under it".into()),
                policy: UpdatePolicy::Notify,
                latest: None,
                last_checked_at: None,
                error: None,
                token: TokenState::Missing,
                last_result: None,
            }),
            release: Mutex::new(None),
            running: tokio::sync::Mutex::new(()),
        }
    }

    pub fn set_last_result(&self, r: Option<UpdateResult>) {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).last_result = r;
    }

    fn snapshot(&self) -> UpdateStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn edit(&self, f: impl FnOnce(&mut UpdateStatus)) -> UpdateStatus {
        let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut s);
        s.clone()
    }
}

async fn token(st: &AppState) -> Result<Option<String>, ApiError> {
    let secrets = st.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.get(UPDATE_TOKEN))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)
}

async fn refresh_policy(st: &AppState) -> UpdatePolicy {
    let p = st.store.get_host_settings().await.map(|s| s.update_policy).unwrap_or(UpdatePolicy::Notify);
    st.update.edit(|s| s.policy = p);
    p
}

/// Asks GitHub for the latest release (only with a token). Never fails: problems land in `error` / `token`.
pub(crate) async fn check(st: &AppState) -> UpdateStatus {
    let svc = &st.update;
    refresh_policy(st).await;
    let now_state = svc.snapshot().state;
    // A check must not overwrite a download or install in progress with `available`.
    if !svc.supervised || matches!(now_state, UpdateState::Downloading | UpdateState::Installing) {
        return svc.snapshot();
    }
    let tok = match token(st).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return svc.edit(|s| {
                s.token = TokenState::Missing;
                s.state = UpdateState::Idle;
                s.error = None;
            });
        }
        Err(e) => return svc.edit(|s| s.error = Some(e.message().to_string())),
    };
    svc.edit(|s| s.state = UpdateState::Checking);
    let result = svc.client.latest(&tok).await;
    let now = now_ms();
    match result {
        Ok(l) => {
            let newer = parse_version(&svc.current).is_none_or(|c| l.release.version.is_newer_than(&c));
            let summary = ReleaseSummary {
                version: l.release.version.to_string(),
                tag: l.release.tag.clone(),
                notes: l.release.notes.clone(),
                published_at: l.release.published_at.clone(),
            };
            *svc.release.lock().unwrap_or_else(|e| e.into_inner()) = newer.then(|| l.release.clone());
            svc.edit(|s| {
                s.token = token_state(true, l.token_expires_at, false, now);
                s.last_checked_at = Some(now);
                s.error = None;
                s.latest = newer.then_some(summary);
                s.state = if newer { UpdateState::Available } else { UpdateState::Idle };
            })
        }
        Err(e) => svc.edit(|s| {
            if e == UpdateError::Unauthorized {
                s.token = TokenState::Expired;
            }
            s.last_checked_at = Some(now);
            s.error = Some(e.to_string());
            s.state = if s.latest.is_some() { UpdateState::Available } else { UpdateState::Idle };
        }),
    }
}

/// What makes the host busy (spec §3.3): agents working or of unknown activity, runs setting up. A store error
/// counts as busy, so an automatic update never starts on a guess.
pub(crate) async fn busy_reasons(st: &AppState) -> Vec<String> {
    let mut out = Vec::new();
    match st.store.list_sessions_by_status(SessionStatus::Running).await {
        Ok(sessions) => {
            for s in sessions {
                match s.activity {
                    Some(Activity::Working) => out.push(format!("{} is working", s.title)),
                    Some(Activity::Unknown) => out.push(format!("{} may be working (activity unknown)", s.title)),
                    _ => {}
                }
            }
        }
        Err(e) => out.push(format!("sessions could not be read ({e})")),
    }
    match st.store.list_active_runs().await {
        Ok(runs) => out.extend(
            runs.into_iter()
                .filter(|r| r.state == RunState::SettingUp)
                .map(|r| format!("run {} · {} is setting up", r.name, r.branch)),
        ),
        Err(e) => out.push(format!("runs could not be read ({e})")),
    }
    out
}

fn conflict(code: &'static str, msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, msg)
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("a parent"))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

/// Downloads and verifies the available release, hands it to pdt-host and asks pdtd to stop.
pub(crate) async fn install(st: &AppState, force: bool) -> Result<UpdateStatus, ApiError> {
    let svc = &st.update;
    if !svc.supervised {
        return Err(conflict("updates_disabled", svc.snapshot().disabled_reason.unwrap_or_default()));
    }
    let _guard =
        svc.running.try_lock().map_err(|_| conflict("update_in_progress", "an update is already in progress"))?;
    let release = svc
        .release
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| conflict("no_update", "no update is available; check for updates first"))?;
    if !force {
        let busy = busy_reasons(st).await;
        if !busy.is_empty() {
            return Err(conflict(
                "host_busy",
                format!("Busy: {}. Update anyway? Sessions resume afterwards.", busy.join("; ")),
            ));
        }
    }
    let tok = token(st).await?.ok_or_else(|| conflict("no_token", "no update token is stored"))?;
    svc.edit(|s| {
        s.state = UpdateState::Downloading;
        s.error = None;
    });
    let dir = downloads_dir(&st.data_dir).join(&release.tag);
    let asset = match fetch_verified(&svc.client, &tok, &release, HostOs::current(), &dir).await {
        Ok(p) => p,
        Err(e) => {
            svc.edit(|s| {
                s.state = UpdateState::Available;
                s.error = Some(e.to_string());
            });
            return Err(ApiError::new(StatusCode::BAD_GATEWAY, "update_failed", e.to_string()));
        }
    };
    let req = UpdateRequest {
        version: release.version.to_string(),
        tag: release.tag.clone(),
        asset,
        from_version: svc.current.clone(),
        requested_at: now_ms(),
    };
    write_atomic(&request_path(&st.data_dir), &serde_json::to_vec_pretty(&req).expect("json"))
        .map_err(ApiError::internal)?;
    tracing::info!(version = %req.version, "update downloaded and verified; handing over to pdt-host");
    let status = svc.edit(|s| s.state = UpdateState::Installing);
    st.shutdown.notify_one();
    Ok(status)
}

/// Hourly checks, and the auto-when-idle install (spec §3.3). Runs only under pdt-host.
pub async fn run_update_loop(st: AppState) {
    if !st.update.supervised {
        return;
    }
    tokio::time::sleep(Duration::from_secs(30)).await;
    loop {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let policy = refresh_policy(&st).await;
        if policy != UpdatePolicy::Manual {
            let s = check(&st).await;
            if s.state == UpdateState::Available
                && policy == UpdatePolicy::AutoWhenIdle
                && busy_reasons(&st).await.is_empty()
                && let Err(e) = install(&st, false).await
            {
                tracing::warn!(error = ?e, "automatic update did not start");
            }
        }
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

pub(crate) async fn get_status(State(st): State<AppState>) -> Json<UpdateStatus> {
    refresh_policy(&st).await;
    Json(st.update.snapshot())
}

pub(crate) async fn post_check(State(st): State<AppState>) -> Json<UpdateStatus> {
    Json(check(&st).await)
}

#[derive(Deserialize)]
pub(crate) struct InstallBody {
    #[serde(default)]
    force: bool,
}

pub(crate) async fn post_install(
    State(st): State<AppState>,
    payload: Result<Json<InstallBody>, JsonRejection>,
) -> Result<Json<UpdateStatus>, ApiError> {
    let Json(b) = payload?;
    Ok(Json(install(&st, b.force).await?))
}

pub(crate) async fn post_dismiss(State(st): State<AppState>) -> StatusCode {
    let _ = std::fs::remove_file(result_path(&st.data_dir));
    st.update.set_last_result(None);
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub(crate) struct TokenBody {
    token: String,
}

/// Stores the token after GitHub accepts it (a 401 is refused; other problems are reported but the token is kept).
pub(crate) async fn put_token(
    State(st): State<AppState>,
    payload: Result<Json<TokenBody>, JsonRejection>,
) -> Result<Json<UpdateStatus>, ApiError> {
    let Json(b) = payload?;
    let tok = b.token.trim().to_string();
    if tok.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid_token", "the token is empty"));
    }
    if st.update.client.latest(&tok).await == Err(UpdateError::Unauthorized) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid_token", "GitHub rejected this token"));
    }
    let secrets = st.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.set(UPDATE_TOKEN, &tok))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    Ok(Json(check(&st).await))
}

pub(crate) async fn delete_token(State(st): State<AppState>) -> Result<StatusCode, ApiError> {
    let secrets = st.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.delete(UPDATE_TOKEN))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    *st.update.release.lock().unwrap_or_else(|e| e.into_inner()) = None;
    st.update.edit(|s| {
        s.token = TokenState::Missing;
        s.latest = None;
        s.state = if st.update.supervised { UpdateState::Idle } else { UpdateState::Disabled };
    });
    Ok(StatusCode::NO_CONTENT)
}
