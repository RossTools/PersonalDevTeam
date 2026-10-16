//! Update checks and installs (remote-host spec §3.3–3.4). pdtd checks hourly (unless the policy is manual) and on
//! request, downloads and verifies a release, writes `update/request.json` and shuts down; pdt-host installs it.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use pdt_core::activity::Activity;
use pdt_core::handoff::{UpdateRequest, UpdateResult, UpdateSkip, downloads_dir, request_path, result_path, skip_path};
use pdt_core::host::UpdatePolicy;
use pdt_core::release::{HostOs, TokenState, Version, parse_version, token_state};
use pdt_core::run::RunState;
use pdt_core::session::SessionStatus;
use pdt_update::{GitHubClient, Release, UpdateError, fetch_verified};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
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
    /// The newer release a check found. Lock order: `status`, then `release`.
    pub(crate) release: Mutex<Option<Release>>,
    /// Held by the install task from the busy check until the hand-off (or failure).
    running: Arc<tokio::sync::Mutex<()>>,
    /// `run_update_loop`: the wait before its first check, and between checks.
    pub first_check_delay: Duration,
    pub check_interval: Duration,
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
            running: Default::default(),
            first_check_delay: Duration::from_secs(30),
            check_interval: Duration::from_secs(3600),
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

/// The state to return to when nothing is in progress.
fn resting(s: &UpdateStatus) -> UpdateState {
    if s.latest.is_some() { UpdateState::Available } else { UpdateState::Idle }
}

fn in_progress(state: UpdateState) -> bool {
    matches!(state, UpdateState::Downloading | UpdateState::Installing)
}

/// This instance's update token, from its own keychain account (see `secrets::update_token_account`).
pub(crate) async fn read_token(st: &AppState) -> Result<Option<String>, ApiError> {
    let (secrets, account) = (st.secrets.clone(), st.update_token_account.clone());
    tokio::task::spawn_blocking(move || secrets.get(&account))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)
}

/// Stores (`Some`) or deletes (`None`) this instance's update token.
async fn write_token(st: &AppState, token: Option<String>) -> Result<(), ApiError> {
    let (secrets, account) = (st.secrets.clone(), st.update_token_account.clone());
    tokio::task::spawn_blocking(move || match token {
        Some(t) => secrets.set(&account, &t),
        None => secrets.delete(&account),
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)
}

/// Brings `token` in line with the keychain (as host settings' `tokenPresent` sees it): missing without a token,
/// valid (expiry unknown until a check) for a token stored before this process knew of it, e.g. before startup.
pub async fn sync_token_state(st: &AppState) {
    if let Ok(present) = read_token(st).await.map(|t| t.is_some()) {
        st.update.edit(|s| match (present, &s.token) {
            (false, _) => s.token = TokenState::Missing,
            (true, TokenState::Missing) => s.token = TokenState::Valid { expires_at: None },
            (true, _) => {}
        });
    }
}

async fn refresh_policy(st: &AppState) -> UpdatePolicy {
    let p = st.store.get_host_settings().await.map(|s| s.update_policy).unwrap_or(UpdatePolicy::Notify);
    st.update.edit(|s| s.policy = p);
    p
}

/// Asks GitHub for the latest release (only with a token). Never fails: problems land in `error` / `token`. Runs in
/// its own task, so a dropped request cannot leave `checking` behind.
pub(crate) async fn check(st: &AppState) -> UpdateStatus {
    let task = tokio::spawn({
        let st = st.clone();
        async move { check_now(&st).await }
    });
    task.await.unwrap_or_else(|_| st.update.snapshot())
}

async fn check_now(st: &AppState) -> UpdateStatus {
    let svc = &st.update;
    refresh_policy(st).await;
    if !svc.supervised {
        return svc.snapshot();
    }
    let tok = match read_token(st).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return svc.edit(|s| {
                s.token = TokenState::Missing;
                if !in_progress(s.state) {
                    s.state = UpdateState::Idle;
                    s.error = None;
                }
            });
        }
        Err(e) => return svc.edit(|s| s.error = Some(e.message().to_string())),
    };
    let Some(current) = parse_version(&svc.current) else {
        return svc.edit(|s| {
            s.error = Some(format!("this build's version {:?} is not X.Y.Z, so no update is offered", svc.current));
            s.latest = None;
            if !in_progress(s.state) {
                s.state = UpdateState::Idle;
            }
        });
    };
    // Claimed under the lock: never over a download or install in progress.
    let claimed = svc.edit(|s| {
        if !in_progress(s.state) {
            s.state = UpdateState::Checking;
        }
    });
    if claimed.state != UpdateState::Checking {
        return claimed;
    }
    let result = svc.client.latest(&tok).await;
    let now = now_ms();
    let mut s = svc.status.lock().unwrap_or_else(|e| e.into_inner());
    s.last_checked_at = Some(now);
    // An install that started while GitHub was answering keeps its state and release.
    let busy = in_progress(s.state);
    match result {
        Ok(l) => {
            s.token = token_state(true, l.token_expires_at, false, now);
            if !busy {
                set_found(&mut s, svc, &l.release, &current);
            }
        }
        Err(e) => {
            if e == UpdateError::Unauthorized {
                s.token = TokenState::Expired;
            }
            s.error = Some(e.to_string());
            if !busy {
                s.state = resting(&s);
            }
        }
    }
    s.clone()
}

/// Records a check's answer (the status lock is held by the caller).
fn set_found(s: &mut UpdateStatus, svc: &UpdateService, release: &Release, current: &Version) {
    let newer = release.version.is_newer_than(current);
    *svc.release.lock().unwrap_or_else(|e| e.into_inner()) = newer.then(|| release.clone());
    s.error = None;
    s.latest = newer.then(|| ReleaseSummary {
        version: release.version.to_string(),
        tag: release.tag.clone(),
        notes: release.notes.clone(),
        published_at: release.published_at.clone(),
    });
    s.state = resting(s);
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

fn host_busy(busy: &[String]) -> ApiError {
    conflict("host_busy", format!("Busy: {}. Update anyway? Sessions resume afterwards.", busy.join("; ")))
}

fn in_progress_error() -> ApiError {
    conflict("update_in_progress", "an update is already in progress")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("a parent"))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

/// Removes everything in the downloads folder except `keep` (the version just handed off).
fn prune_downloads(downloads: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(downloads) else { return };
    for e in entries.flatten() {
        if e.file_name() != keep {
            let p = e.path();
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        }
    }
}

/// Downloads and verifies the available release, hands it to pdt-host and asks pdtd to stop. The download and
/// hand-off run in their own task, so a dropped request does not cancel them halfway.
pub(crate) async fn install(st: &AppState, force: bool) -> Result<UpdateStatus, ApiError> {
    let svc = &st.update;
    if !svc.supervised {
        return Err(conflict("updates_disabled", svc.snapshot().disabled_reason.unwrap_or_default()));
    }
    if svc.snapshot().state == UpdateState::Installing {
        return Err(in_progress_error());
    }
    let guard = svc.running.clone().try_lock_owned().map_err(|_| in_progress_error())?;
    let release = svc
        .release
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| conflict("no_update", "no update is available; check for updates first"))?;
    if !force {
        let busy = busy_reasons(st).await;
        if !busy.is_empty() {
            return Err(host_busy(&busy));
        }
    }
    let tok = read_token(st).await?.ok_or_else(|| conflict("no_token", "no update token is stored"))?;
    let started = svc.edit(|s| {
        if s.state != UpdateState::Installing {
            s.state = UpdateState::Downloading;
            s.error = None;
        }
    });
    if started.state != UpdateState::Downloading {
        return Err(in_progress_error());
    }
    let task = tokio::spawn({
        let st = st.clone();
        async move {
            let _guard = guard;
            download_and_hand_off(&st, release, tok, force).await
        }
    });
    task.await.map_err(ApiError::internal)?
}

/// Puts `downloading` back to the resting state unless the hand-off completed, whatever ended the task (an error
/// path or a panic).
struct ResetOnDrop<'a> {
    svc: &'a UpdateService,
    done: bool,
}

impl Drop for ResetOnDrop<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.svc.edit(|s| {
                if s.state == UpdateState::Downloading {
                    s.state = resting(s);
                }
            });
        }
    }
}

async fn download_and_hand_off(
    st: &AppState,
    release: Release,
    tok: String,
    force: bool,
) -> Result<UpdateStatus, ApiError> {
    let svc = &st.update;
    let mut reset = ResetOnDrop { svc, done: false };
    let fail = |code: StatusCode, name: &'static str, msg: String| {
        svc.edit(|s| {
            s.state = resting(s);
            s.error = Some(msg.clone());
        });
        ApiError::new(code, name, msg)
    };
    let version = release.version.to_string();
    let downloads = downloads_dir(&st.data_dir);
    let asset = fetch_verified(&svc.client, &tok, &release, HostOs::current(), &downloads.join(&version))
        .await
        .map_err(|e| fail(StatusCode::BAD_GATEWAY, "update_failed", e.to_string()))?;
    // The download can take minutes: ask again, keeping the verified file for the next attempt.
    if !force {
        let busy = busy_reasons(st).await;
        if !busy.is_empty() {
            svc.edit(|s| s.state = resting(s));
            return Err(host_busy(&busy));
        }
    }
    let req = UpdateRequest {
        version,
        tag: release.tag.clone(),
        asset,
        from_version: svc.current.clone(),
        requested_at: now_ms(),
    };
    write_atomic(&request_path(&st.data_dir), &serde_json::to_vec_pretty(&req).expect("json")).map_err(|e| {
        fail(StatusCode::INTERNAL_SERVER_ERROR, "internal", format!("could not write the update request: {e}"))
    })?;
    prune_downloads(&downloads, &req.version);
    tracing::info!(version = %req.version, "update downloaded and verified; handing over to pdt-host");
    let status = svc.edit(|s| s.state = UpdateState::Installing);
    reset.done = true;
    st.shutdown.notify_one();
    Ok(status)
}

/// pdt-host recorded a failed or rolled-back update to `version` (ruling B): it is never installed automatically
/// again; a manual update may retry it. Read from disk each time, so dismissing the result does not lift it.
fn skipped(data_dir: &Path, version: &str) -> bool {
    std::fs::read(skip_path(data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<UpdateSkip>(&b).ok())
        .is_some_and(|skip| skip.version == version)
}

/// Checks every `check_interval` (unless the policy is manual), and installs when idle under auto-when-idle (spec
/// §3.3). Runs only under pdt-host.
pub async fn run_update_loop(st: AppState) {
    if !st.update.supervised {
        return;
    }
    tokio::time::sleep(st.update.first_check_delay).await;
    loop {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let policy = refresh_policy(&st).await;
        if policy != UpdatePolicy::Manual {
            let s = check(&st).await;
            let failed_before = s.latest.as_ref().is_some_and(|l| skipped(&st.data_dir, &l.version));
            if failed_before {
                tracing::info!("not installing automatically: the last attempt at this version failed");
            }
            if s.state == UpdateState::Available
                && policy == UpdatePolicy::AutoWhenIdle
                && !failed_before
                && busy_reasons(&st).await.is_empty()
                && let Err(e) = install(&st, false).await
            {
                tracing::warn!(error = e.message(), "automatic update did not start");
            }
        }
        tokio::time::sleep(st.update.check_interval).await;
    }
}

pub(crate) async fn get_status(State(st): State<AppState>) -> Json<UpdateStatus> {
    refresh_policy(&st).await;
    sync_token_state(&st).await;
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
    let status = install(&st, b.force).await?;
    // A manual update of a skipped version is the retry ruling B allows; the skip is lifted.
    let _ = std::fs::remove_file(skip_path(&st.data_dir));
    Ok(Json(status))
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
    let expires_at = match st.update.client.latest(&tok).await {
        Err(UpdateError::Unauthorized) => {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid_token", "GitHub rejected this token"));
        }
        Ok(l) => l.token_expires_at,
        Err(_) => None,
    };
    write_token(&st, Some(tok)).await?;
    st.update.edit(|s| s.token = token_state(true, expires_at, false, now_ms()));
    Ok(Json(check(&st).await))
}

pub(crate) async fn delete_token(State(st): State<AppState>) -> Result<StatusCode, ApiError> {
    write_token(&st, None).await?;
    let svc = &st.update;
    let mut s = svc.status.lock().unwrap_or_else(|e| e.into_inner());
    s.token = TokenState::Missing;
    if !in_progress(s.state) {
        *svc.release.lock().unwrap_or_else(|e| e.into_inner()) = None;
        s.latest = None;
        s.state = if svc.supervised { UpdateState::Idle } else { UpdateState::Disabled };
    }
    Ok(StatusCode::NO_CONTENT)
}
