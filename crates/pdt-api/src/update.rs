//! Update checks and installs (remote-host spec §3.3–3.4). pdtd checks hourly (unless the policy is manual) and on
//! request, downloads and verifies a release, writes `update/request.json` and shuts down; pdt-host installs it.

use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use pdt_core::activity::Activity;
use pdt_core::handoff::{
    UpdateOutcome, UpdateRequest, UpdateResult, UpdateSkip, downloads_dir, request_path, result_path, skip_path,
    taken_request_path,
};
use pdt_core::host::{DEFAULT_UPDATE_CHECK_MINUTES, UpdateChannel, UpdatePolicy};
use pdt_core::release::{APPIMAGE_MARKER, HostOs, TokenState, Version, compare_base, parse_version, token_state};
use pdt_core::run::RunState;
use pdt_core::session::SessionStatus;
use pdt_core::workflow::api::NewLogEvent;
use pdt_core::workflow::events::{self as ev, UpdateAvailable, UpdateFailed, UpdateInstalled};
use pdt_update::{Changes, GitHubClient, Release, UpdateError, fetch_verified};
use serde::{Deserialize, Serialize};
use serde_json::json;
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
    /// A nightly build (`X.Y.Z-nightly.YYYYMMDD`, or an older experimental `X.Y.Z-exp.N`; a GitHub pre-release;
    /// PDT-210, PDT-283).
    pub nightly: bool,
    pub notes: String,
    /// GitHub's `published_at` as GitHub sends it (an ISO 8601 UTC string, e.g. `2026-10-15T00:00:00Z`), not Unix
    /// ms: it is only displayed (ruling F17).
    pub published_at: Option<String>,
    /// Every commit from the installed version to this one (PDT-169); None when GitHub could not compare them (the
    /// installed tag is gone, a local build, a failed call), and `notes` is all there is.
    pub changes: Option<ReleaseChanges>,
    /// Every published release from the installed version (exclusive) up to this one, newest first, with its notes
    /// (PDT-192); None when GitHub could not list them (a local build, a failed call).
    pub releases: Option<Vec<ReleaseNotes>>,
}

/// One release an update brings, and its notes.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleaseNotes {
    pub version: String,
    pub tag: String,
    /// A nightly build: its notes preview the changes not yet in a stable release.
    pub nightly: bool,
    pub notes: String,
    /// As GitHub sends it (ISO 8601 UTC).
    pub published_at: Option<String>,
}

/// The commits an update brings, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleaseChanges {
    /// The installed version (`0.1.741`).
    pub since: String,
    pub entries: Vec<ReleaseChange>,
    /// Every commit in the range: more than `entries` when GitHub cut the list off at 250.
    pub total: u32,
    /// The range on GitHub, when the list is cut off.
    pub more_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleaseChange {
    pub sha: String,
    /// The commit's first line.
    pub subject: String,
    /// The subject without its ticket key and pull request number.
    pub title: String,
    /// `PDT-167`.
    pub ticket: Option<String>,
    /// That ticket on this instance, when it has it (looked up for each answer, not cached).
    pub ticket_here: Option<TicketHere>,
    pub pr: Option<u32>,
    pub pr_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketHere {
    pub id: String,
    pub project_id: String,
}

/// What points this host at a release repository other than the built-in one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RepoOverride {
    /// Settings → This host → Updates.
    Setting,
    /// `PDT_RELEASE_REPO` in pdtd's environment (it wins over the setting).
    Environment,
}

/// Where updates come from and bug reports go (PDT-195).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleaseRepo {
    /// `owner/name`.
    pub repo: String,
    /// Its numeric GitHub id, tried first (the built-in repository only).
    #[ts(type = "number | null")]
    pub id: Option<u64>,
    /// Where a GitHub redirect showed the repository moved (`repositories/{id}` or `repos/{owner}/{name}`).
    pub moved_to: Option<String>,
    pub overridden_by: Option<RepoOverride>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateStatus {
    pub current: String,
    /// The installed version is a nightly build (`X.Y.Z-nightly.YYYYMMDD`, or an older `X.Y.Z-exp.N`).
    pub current_nightly: bool,
    /// The host's update channel (PDT-210).
    pub channel: UpdateChannel,
    pub state: UpdateState,
    pub disabled_reason: Option<String>,
    pub policy: UpdatePolicy,
    pub latest: Option<ReleaseSummary>,
    #[ts(type = "number | null")]
    pub last_checked_at: Option<i64>,
    pub error: Option<String>,
    pub token: TokenState,
    pub last_result: Option<UpdateResult>,
    pub release_repo: ReleaseRepo,
}

/// What a check found (PDT-223): the answer to "Check for updates", so the button never seems to do nothing.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum CheckResult {
    /// The installed version (`version`) is the newest on this host's channel.
    UpToDate { version: String },
    /// `version` is newer than the installed one.
    Available { version: String },
    /// The check did not ask GitHub, or GitHub did not answer: `message` says why, in words for the user.
    CannotCheck { reason: CannotCheckReason, message: String },
}

/// Why a check could not tell whether there is an update (each points at what to fix).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CannotCheckReason {
    /// Updates are off on this pdtd (`disabled_reason`).
    Disabled,
    /// No update token is saved.
    NoToken,
    /// GitHub rejected the saved token (expired or revoked).
    TokenRejected,
    /// The keychain could not be read.
    Keychain,
    /// GitHub could not be reached.
    Network,
    /// GitHub answered with an error (no release, the repository is hidden from the token, a server error).
    Github,
    /// This build's version is not X.Y.Z, so nothing is compared with it.
    UnknownVersion,
    /// An update is downloading or installing.
    Busy,
}

fn cannot(reason: CannotCheckReason, message: impl Into<String>) -> CheckResult {
    CheckResult::CannotCheck { reason, message: message.into() }
}

/// `POST /api/host/update/check`'s answer: the status, and what this check found.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckResponse {
    #[serde(flatten)]
    pub status: UpdateStatus,
    pub result: CheckResult,
}

/// Why an unsupervised pdtd never updates.
pub const UNSUPERVISED_REASON: &str = "updates are installed by pdt-host, and this pdtd is not running under it";

pub struct UpdateService {
    pub(crate) client: GitHubClient,
    /// The built-in release repository and its id.
    default_repo: (String, Option<u64>),
    /// `PDT_RELEASE_REPO`: the release repository this pdtd uses whatever the setting says.
    env_repo: Option<String>,
    /// Whether this pdtd may check for and hand off updates (only a supervised stable instance does).
    pub(crate) enabled: bool,
    current: String,
    status: Mutex<UpdateStatus>,
    /// The newer release a check found. Lock order: `status`, then `release`.
    pub(crate) release: Mutex<Option<Release>>,
    /// The last comparison: its base and head, so a check for the same pair does not ask GitHub again.
    changes: Mutex<Option<(String, String, ReleaseChanges)>>,
    /// The last list of releases an update brings: the installed version and the new tag it was listed for.
    releases: Mutex<Option<(String, String, Vec<ReleaseNotes>)>>,
    /// Held by the install task from the busy check until the hand-off (or failure).
    running: Arc<tokio::sync::Mutex<()>>,
    /// `requested_at` of the request this pdtd handed to pdt-host, while it shows `installing`.
    handed_off_at: Mutex<Option<i64>>,
    /// `run_update_loop`: the wait before its first check, and between checks (None: the host's
    /// `update_check_minutes`).
    pub first_check_delay: Duration,
    pub check_interval: Option<Duration>,
    /// `run_update_loop`: how often it wakes to look at the settings again, so a shorter interval or a policy that
    /// stops being manual takes effect without waiting out the old interval.
    pub settings_poll: Duration,
}

impl UpdateService {
    /// Updates are enabled when `supervised` (pdt-host started this pdtd), else disabled with the reason why.
    pub fn new(api_base: &str, repo: &str, supervised: bool, current: &str) -> Self {
        let reason = (!supervised).then(|| UNSUPERVISED_REASON.to_string());
        Self::with_disabled_reason(api_base, repo, reason, current)
    }

    /// Updates are enabled when `disabled_reason` is `None`; otherwise every check and install is refused with it.
    pub fn with_disabled_reason(api_base: &str, repo: &str, disabled_reason: Option<String>, current: &str) -> Self {
        let enabled = disabled_reason.is_none();
        Self {
            client: GitHubClient::new(api_base, repo),
            default_repo: (repo.to_string(), None),
            env_repo: None,
            enabled,
            current: current.into(),
            status: Mutex::new(UpdateStatus {
                current: current.into(),
                current_nightly: parse_version(current).is_some_and(|v| v.is_nightly()),
                channel: UpdateChannel::Stable,
                state: if enabled { UpdateState::Idle } else { UpdateState::Disabled },
                disabled_reason,
                policy: UpdatePolicy::Notify,
                latest: None,
                last_checked_at: None,
                error: None,
                token: TokenState::Missing,
                last_result: None,
                release_repo: ReleaseRepo { repo: repo.to_string(), id: None, moved_to: None, overridden_by: None },
            }),
            release: Mutex::new(None),
            changes: Mutex::new(None),
            releases: Mutex::new(None),
            running: Default::default(),
            handed_off_at: Mutex::new(None),
            first_check_delay: Duration::from_secs(30),
            check_interval: None,
            settings_poll: Duration::from_secs(60),
        }
    }

    /// Pins the built-in repository by its numeric id (`brand::RELEASE_REPO_ID`).
    pub fn with_repo_id(mut self, id: u64) -> Self {
        self.default_repo.1 = Some(id);
        self.use_repo(None);
        self
    }

    /// `PDT_RELEASE_REPO`: use this repository (`owner/name`) instead of the built-in one, whatever the setting says.
    pub fn with_env_repo(mut self, repo: Option<String>) -> Self {
        self.env_repo = repo;
        self.use_repo(None);
        self
    }

    /// Points the client at the repository to use: the environment's, else the host setting's, else the built-in
    /// one (only that one by its id).
    fn use_repo(&self, setting: Option<&str>) {
        let (repo, by) = match (&self.env_repo, setting) {
            (Some(r), _) => (r.as_str(), Some(RepoOverride::Environment)),
            (None, Some(r)) => (r, Some(RepoOverride::Setting)),
            (None, None) => (self.default_repo.0.as_str(), None),
        };
        let id = self.default_repo.1.filter(|_| repo.eq_ignore_ascii_case(&self.default_repo.0));
        self.client.set_repo(repo, id);
        self.note_repo(by);
    }

    /// Copies where the client finds the repository into the status (`by`: what overrides it).
    fn note_repo(&self, by: Option<RepoOverride>) {
        let r = ReleaseRepo {
            repo: self.client.repo(),
            id: self.client.repo_id(),
            moved_to: self.client.moved_to(),
            overridden_by: by,
        };
        self.edit(|s| s.release_repo = r);
    }

    pub fn set_last_result(&self, r: Option<UpdateResult>) {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).last_result = r;
    }

    pub(crate) fn snapshot(&self) -> UpdateStatus {
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
        .map_err(ApiError::keychain)
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
    .map_err(ApiError::keychain)
}

/// Brings `token` in line with the keychain (as host settings' `tokenPresent` sees it): missing without a token,
/// valid (expiry unknown until a check) for a token stored before this process knew of it, e.g. before startup.
/// A keychain that cannot be read (e.g. it is waiting on an approval prompt) is reported in `error`.
pub async fn sync_token_state(st: &AppState) {
    match read_token(st).await.map(|t| t.is_some()) {
        Ok(present) => {
            st.update.edit(|s| {
                match (present, &s.token) {
                    (false, _) => s.token = TokenState::Missing,
                    (true, TokenState::Missing) => s.token = TokenState::Valid { expires_at: None },
                    (true, _) => {}
                }
                // The keychain answers again: its earlier failure no longer applies (other errors stay).
                if s.error.as_deref().is_some_and(|e| e.starts_with(crate::secrets::KEYCHAIN_FAILED)) {
                    s.error = None;
                }
            });
        }
        Err(e) => {
            st.update.edit(|s| s.error = Some(e.message().to_string()));
        }
    }
}

/// Reads the host settings this service follows: the update policy and channel, and the release repository override
/// (a failed read keeps the repository it has).
pub(crate) async fn refresh_settings(st: &AppState) -> UpdatePolicy {
    let settings = st.store.get_host_settings().await;
    let p = settings.as_ref().map(|s| s.update_policy).unwrap_or(UpdatePolicy::Notify);
    let channel = settings.as_ref().map(|s| s.update_channel).unwrap_or_default();
    st.update.edit(|s| {
        s.policy = p;
        s.channel = channel;
    });
    if let Ok(s) = &settings {
        st.update.use_repo(s.release_repo.as_deref());
    }
    p
}

/// Asks GitHub for the latest release (only with a token). Never fails: problems land in `error` / `token`, and the
/// result says what the check found. Runs in its own task, so a dropped request cannot leave `checking` behind.
pub(crate) async fn check(st: &AppState) -> (UpdateStatus, CheckResult) {
    let task = tokio::spawn({
        let st = st.clone();
        async move { check_now(&st).await }
    });
    task.await.unwrap_or_else(|e| {
        let s = st.update.snapshot();
        (s, cannot(CannotCheckReason::Github, format!("the check failed: {e}")))
    })
}

async fn check_now(st: &AppState) -> (UpdateStatus, CheckResult) {
    let svc = &st.update;
    refresh_settings(st).await;
    if !svc.enabled {
        let s = svc.snapshot();
        let why = s.disabled_reason.clone().unwrap_or_else(|| "updates are off on this host".into());
        return (s, cannot(CannotCheckReason::Disabled, why));
    }
    let tok = match read_token(st).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            let s = svc.edit(|s| {
                s.token = TokenState::Missing;
                if !in_progress(s.state) {
                    s.state = UpdateState::Idle;
                    s.error = None;
                }
            });
            return (s, cannot(CannotCheckReason::NoToken, "no GitHub token is saved, so PDT can't read its releases"));
        }
        Err(e) => {
            let msg = e.message().to_string();
            return (svc.edit(|s| s.error = Some(msg.clone())), cannot(CannotCheckReason::Keychain, msg));
        }
    };
    let Some(current) = parse_version(&svc.current) else {
        let msg = format!("this build's version {:?} is not X.Y.Z, so no update is offered", svc.current);
        let s = svc.edit(|s| {
            s.error = Some(msg.clone());
            s.latest = None;
            if !in_progress(s.state) {
                s.state = UpdateState::Idle;
            }
        });
        return (s, cannot(CannotCheckReason::UnknownVersion, msg));
    };
    // Claimed under the lock: never over a download or install in progress.
    let claimed = svc.edit(|s| {
        if !in_progress(s.state) {
            s.state = UpdateState::Checking;
        }
    });
    if claimed.state != UpdateState::Checking {
        return (claimed, cannot(CannotCheckReason::Busy, "an update is already downloading or installing"));
    }
    // The channel the settings named when this check started (refresh_settings above).
    let channel = claimed.channel;
    let result = svc.client.newest(&tok, channel).await;
    let (changes, releases) = match &result {
        Ok(l) if l.release.version.is_newer_than(&current) => {
            (release_changes(svc, &tok, &l.release).await, releases_since(svc, &tok, &current, &l.release).await)
        }
        _ => (None, None),
    };
    let now = now_ms();
    // A newer version is a workflow event (PDT-365), once per version: logged once the status lock is let go.
    let available = match &result {
        Ok(l) if l.release.version.is_newer_than(&current) => Some(UpdateAvailable {
            version: l.release.version.to_string(),
            current: svc.current.clone(),
            tag: l.release.tag.clone(),
            channel: channel_name(channel),
            nightly: l.release.version.is_nightly(),
            notes_url: notes_url(svc, &l.release.tag),
            notes: l.release.notes.clone(),
            published_at: l.release.published_at.clone(),
        }),
        _ => None,
    };
    let out = {
        let mut s = svc.status.lock().unwrap_or_else(|e| e.into_inner());
        s.last_checked_at = Some(now);
        s.release_repo.moved_to = svc.client.moved_to();
        // An install that started while GitHub was answering keeps its state and release.
        let busy = in_progress(s.state);
        let found = match result {
            Ok(l) => {
                s.token = token_state(true, l.token_expires_at, false, now);
                if !busy {
                    set_found(&mut s, svc, &l.release, &current, changes, releases);
                }
                let version = l.release.version.to_string();
                if l.release.version.is_newer_than(&current) {
                    CheckResult::Available { version }
                } else {
                    CheckResult::UpToDate { version: current.to_string() }
                }
            }
            Err(e) => {
                let reason = match e {
                    UpdateError::Unauthorized => CannotCheckReason::TokenRejected,
                    UpdateError::Network(_) => CannotCheckReason::Network,
                    _ => CannotCheckReason::Github,
                };
                if e == UpdateError::Unauthorized {
                    s.token = TokenState::Expired;
                }
                s.error = Some(e.to_string());
                if !busy {
                    s.state = resting(&s);
                }
                cannot(reason, e.to_string())
            }
        };
        (s.clone(), found)
    };
    if let Some(a) = available {
        let key = format!("update:{}", a.version);
        log_once(st, ev::UPDATE_AVAILABLE, key, a).await;
    }
    out
}

/// Where the update events note the last one they logged: the event log forgets after a while, a version waiting
/// longer than that is still not news.
const UPDATE_EVENTS_KEY: &str = "sensor.update_events";

/// Logs `kind` with `key` unless the last one of its kind had that key.
async fn log_once(st: &AppState, kind: &str, key: String, payload: impl Serialize) {
    let mut seen = st
        .store
        .get_host_value(UPDATE_EVENTS_KEY)
        .await
        .ok()
        .flatten()
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| json!({}));
    if seen[kind].as_str() == Some(key.as_str()) {
        return;
    }
    if let Err(e) = st.store.emit_event(&NewLogEvent::new(kind, key.clone(), "update", payload), now_ms()).await {
        return tracing::warn!(kind, error = %e, "could not log an update event");
    }
    seen[kind] = json!(key);
    if let Err(e) = st.store.put_host_value(UPDATE_EVENTS_KEY, &seen).await {
        tracing::warn!(error = %e, "could not record the update event logged");
    }
}

/// A channel's name in an event (`stable`, `nightly`).
fn channel_name(c: UpdateChannel) -> String {
    serde_json::to_value(c).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

/// The page of the release tagged `tag` in the release repository.
fn notes_url(svc: &UpdateService, tag: &str) -> String {
    format!("https://github.com/{}/releases/tag/{tag}", svc.client.repo())
}

/// pdtd started on the version pdt-host just installed: `update.installed` (PDT-365), once per install (the event's key
/// is the install's), with whether pdtd asked for it by itself (the request pdt-host took says). After an update that
/// failed or was rolled back, on the version it ran before: `update.failed` (PDT-369), once per failure.
pub async fn log_installed(st: &AppState) {
    let svc = &st.update;
    let Some(r) =
        std::fs::read(result_path(&st.data_dir)).ok().and_then(|b| serde_json::from_slice::<UpdateResult>(&b).ok())
    else {
        return;
    };
    if r.outcome != UpdateOutcome::Installed {
        if r.from_version == svc.current {
            let key = format!("update:{}:failed:{}", r.to_version, r.at);
            let payload = UpdateFailed {
                version: r.to_version,
                from_version: r.from_version,
                rolled_back: r.outcome == UpdateOutcome::RolledBack,
                message: r.message,
                log: r.log,
                at: r.at,
            };
            log_once(st, ev::UPDATE_FAILED, key, payload).await;
        }
        return;
    }
    if r.to_version != svc.current {
        return;
    }
    let taken = std::fs::read(taken_request_path(&st.data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<UpdateRequest>(&b).ok())
        .filter(|q| q.version == r.to_version);
    let tag = taken.as_ref().map_or_else(|| format!("v{}", r.to_version), |q| q.tag.clone());
    let channel = host_channel(st).await;
    let payload = UpdateInstalled {
        version: r.to_version.clone(),
        from_version: r.from_version.clone(),
        channel: channel_name(channel),
        notes_url: notes_url(svc, &tag),
        automatic: taken.is_some_and(|q| q.automatic),
        at: r.at,
    };
    let key = format!("update:{}:installed:{}", r.to_version, r.at);
    log_once(st, ev::UPDATE_INSTALLED, key, payload).await;
}

/// The update on offer now, as `update.available` tells it (None while there is none).
pub(crate) fn on_offer(svc: &UpdateService) -> Option<UpdateAvailable> {
    let s = svc.snapshot();
    let l = s.latest?;
    Some(UpdateAvailable {
        notes_url: notes_url(svc, &l.tag),
        version: l.version,
        current: svc.current.clone(),
        tag: l.tag,
        channel: channel_name(s.channel),
        nightly: l.nightly,
        notes: l.notes,
        published_at: l.published_at,
    })
}

/// The version this pdtd runs.
pub(crate) fn current(svc: &UpdateService) -> &str {
    &svc.current
}

/// Whether `version` is the one running, or older.
pub(crate) fn installed(svc: &UpdateService, version: &str) -> bool {
    match (parse_version(version), parse_version(&svc.current)) {
        (Some(v), Some(c)) => !v.is_newer_than(&c),
        _ => version == svc.current,
    }
}

/// The host's update channel, from its settings.
async fn host_channel(st: &AppState) -> UpdateChannel {
    st.store.get_host_settings().await.map(|s| s.update_channel).unwrap_or_default()
}

/// The commits from the installed version to `release`, from the cache when the pair was compared before. None when
/// there is nothing to compare from or GitHub fails (the release's notes stand in).
async fn release_changes(svc: &UpdateService, tok: &str, release: &Release) -> Option<ReleaseChanges> {
    let base = compare_base(&svc.current)?;
    if let Some((b, h, c)) = &*svc.changes.lock().unwrap_or_else(|e| e.into_inner())
        && (b.as_str(), h.as_str()) == (base.as_str(), release.tag.as_str())
    {
        return Some(c.clone());
    }
    let changes = match svc.client.compare(tok, &base, &release.tag).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, base, head = release.tag, "could not list the changes in the update");
            return None;
        }
    };
    let since = parse_version(&svc.current).map_or_else(|| svc.current.clone(), |v| v.to_string());
    let out = summarize(&changes, &since, &svc.client.repo());
    *svc.changes.lock().unwrap_or_else(|e| e.into_inner()) = Some((base, release.tag.clone(), out.clone()));
    Some(out)
}

/// The releases from the installed version up to `release`, with their notes, from the cache when that pair was
/// listed before. None when GitHub fails (the commits, or the release's own notes, stand in).
async fn releases_since(
    svc: &UpdateService,
    tok: &str,
    current: &Version,
    release: &Release,
) -> Option<Vec<ReleaseNotes>> {
    if current.local {
        return None;
    }
    let since = current.to_string();
    if let Some((c, t, list)) = &*svc.releases.lock().unwrap_or_else(|e| e.into_inner())
        && (c.as_str(), t.as_str()) == (since.as_str(), release.tag.as_str())
    {
        return Some(list.clone());
    }
    let list = match svc.client.releases_between(tok, current, &release.version).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, since, head = release.tag, "could not list the releases in the update");
            return None;
        }
    };
    let out: Vec<ReleaseNotes> = list
        .into_iter()
        .map(|r| ReleaseNotes {
            version: r.version.to_string(),
            nightly: r.version.is_nightly(),
            tag: r.tag,
            notes: r.notes,
            published_at: r.published_at,
        })
        .collect();
    *svc.releases.lock().unwrap_or_else(|e| e.into_inner()) = Some((since, release.tag.clone(), out.clone()));
    Some(out)
}

fn summarize(c: &Changes, since: &str, repo: &str) -> ReleaseChanges {
    let entries = c
        .entries
        .iter()
        .map(|e| {
            let pr = e.pr.and_then(|n| u32::try_from(n).ok());
            ReleaseChange {
                sha: e.sha.clone(),
                subject: e.subject.clone(),
                title: e.title.clone(),
                ticket: e.ticket.clone(),
                ticket_here: None,
                pr,
                pr_url: pr.map(|n| format!("https://github.com/{repo}/pull/{n}")),
            }
        })
        .collect();
    ReleaseChanges {
        since: since.to_string(),
        entries,
        total: u32::try_from(c.total).unwrap_or(u32::MAX),
        more_url: c.truncated().then(|| c.url.clone()),
    }
}

/// Points each change's ticket key at that ticket here, when this instance has it.
async fn link_tickets(st: &AppState, mut s: UpdateStatus) -> UpdateStatus {
    let Some(changes) = s.latest.as_mut().and_then(|l| l.changes.as_mut()) else { return s };
    let Ok(projects) = st.store.list_projects().await else { return s };
    for e in &mut changes.entries {
        let Some(k) = e.ticket.as_deref().and_then(pdt_core::ticket::parse_key) else { continue };
        let Some(p) = projects.iter().find(|p| k.key.as_deref().is_some_and(|key| p.key.eq_ignore_ascii_case(key)))
        else {
            continue;
        };
        if let Ok(Some(id)) = st.store.ticket_id_by_number(&p.id, k.number).await {
            e.ticket_here = Some(TicketHere { id, project_id: p.id.clone() });
        }
    }
    s
}

/// Records a check's answer (the status lock is held by the caller).
fn set_found(
    s: &mut UpdateStatus,
    svc: &UpdateService,
    release: &Release,
    current: &Version,
    changes: Option<ReleaseChanges>,
    releases: Option<Vec<ReleaseNotes>>,
) {
    let newer = release.version.is_newer_than(current);
    *svc.release.lock().unwrap_or_else(|e| e.into_inner()) = newer.then(|| release.clone());
    s.error = None;
    s.latest = newer.then(|| ReleaseSummary {
        version: release.version.to_string(),
        tag: release.tag.clone(),
        nightly: release.version.is_nightly(),
        notes: release.notes.clone(),
        published_at: release.published_at.clone(),
        changes,
        releases,
    });
    s.state = resting(s);
}

/// What makes the host busy (spec §3.3): agents working, of unknown activity or waiting on background tasks, runs
/// setting up. A store error counts as busy, so an automatic update never starts on a guess.
pub async fn busy_reasons(st: &AppState) -> Vec<String> {
    let mut out = Vec::new();
    match st.store.list_sessions_by_status(SessionStatus::Running).await {
        Ok(sessions) => {
            for s in sessions {
                let name = format!("{} · {}", pdt_core::session::kind_label(s.kind, s.agent), s.title);
                match s.activity {
                    Some(Activity::Working) => out.push(format!("{name} is working")),
                    Some(Activity::Unknown) => out.push(format!("{name} may be working (activity unknown)")),
                    // Idle between turns, but an update would stop the tasks it waits on (PDT-311).
                    _ => match crate::conversations::background(st, &s.id).filter(|b| b.live).map(|b| b.tasks.len()) {
                        Some(1) => out.push(format!("{name} is waiting on 1 background task")),
                        Some(n) if n > 1 => out.push(format!("{name} is waiting on {n} background tasks")),
                        _ => {}
                    },
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

/// The platform whose release asset this pdtd installs. On Linux, an AppImage's payload folder (where pdtd runs from)
/// holds `APPIMAGE_MARKER`; a tarball's does not.
fn this_host() -> HostOs {
    let appimage = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(APPIMAGE_MARKER)))
        .is_some_and(|m| m.is_file());
    HostOs::current_linux(appimage)
}

fn host_busy(busy: &[String]) -> ApiError {
    ApiError::conflict(
        codes::HOST_BUSY,
        format!("Busy: {}. Update anyway? Sessions resume afterwards.", busy.join("; ")),
    )
}

fn in_progress_error() -> ApiError {
    ApiError::conflict(codes::UPDATE_IN_PROGRESS, "an update is already in progress")
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
/// `automatic`: pdtd installs it by itself (auto-when-idle), not because the user asked.
pub(crate) async fn install(st: &AppState, force: bool, automatic: bool) -> Result<UpdateStatus, ApiError> {
    let svc = &st.update;
    if !svc.enabled {
        return Err(ApiError::conflict(codes::UPDATES_DISABLED, svc.snapshot().disabled_reason.unwrap_or_default()));
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
        .ok_or_else(|| ApiError::conflict(codes::NO_UPDATE, "no update is available; check for updates first"))?;
    if !force {
        let busy = busy_reasons(st).await;
        if !busy.is_empty() {
            return Err(host_busy(&busy));
        }
    }
    let tok = read_token(st).await?.ok_or_else(|| ApiError::conflict(codes::NO_TOKEN, "no update token is stored"))?;
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
            download_and_hand_off(&st, release, tok, force, automatic).await
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
    automatic: bool,
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
    let asset = fetch_verified(&svc.client, &tok, &release, this_host(), &downloads.join(&version))
        .await
        .map_err(|e| fail(StatusCode::BAD_GATEWAY, codes::UPDATE_FAILED, e.to_string()))?;
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
        automatic,
    };
    write_atomic(&request_path(&st.data_dir), &serde_json::to_vec_pretty(&req).expect("json")).map_err(|e| {
        fail(StatusCode::INTERNAL_SERVER_ERROR, "internal", format!("could not write the update request: {e}"))
    })?;
    prune_downloads(&downloads, &req.version);
    tracing::info!(version = %req.version, "update downloaded and verified; handing over to pdt-host");
    *svc.handed_off_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(req.requested_at);
    let status = svc.edit(|s| s.state = UpdateState::Installing);
    reset.done = true;
    st.shutdown.notify_one();
    Ok(status)
}

/// pdt-host recorded a failed or rolled-back update to `version` (ruling B): it is never installed automatically
/// again; a manual update may retry it. Read from disk each time, so dismissing the result does not lift it.
pub(crate) fn skipped(data_dir: &Path, version: &str) -> bool {
    std::fs::read(skip_path(data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<UpdateSkip>(&b).ok())
        .is_some_and(|skip| skip.version == version)
}

/// The wait between automatic checks: the service's own (tests), else the host's setting.
async fn check_interval(st: &AppState) -> Duration {
    if let Some(d) = st.update.check_interval {
        return d;
    }
    let minutes =
        st.store.get_host_settings().await.map(|s| s.update_check_minutes).unwrap_or(DEFAULT_UPDATE_CHECK_MINUTES);
    Duration::from_secs(u64::from(minutes) * 60)
}

/// Checks every `update_check_minutes` (unless the policy is manual). Runs only under pdt-host. What a check finds is
/// `update.available`; the built-in Install updates workflow installs it under auto-when-idle (PDT-369).
pub async fn run_update_loop(st: AppState) {
    if !st.update.enabled {
        return;
    }
    tokio::time::sleep(st.update.first_check_delay).await;
    let mut last_check: Option<tokio::time::Instant> = None;
    loop {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let interval = check_interval(&st).await;
        let due = last_check.is_none_or(|t| t.elapsed() >= interval);
        let policy = refresh_settings(&st).await;
        if due && policy != UpdatePolicy::Manual {
            last_check = Some(tokio::time::Instant::now());
            check(&st).await;
        }
        let wait = last_check.map_or(interval, |t| interval.saturating_sub(t.elapsed()));
        tokio::time::sleep(wait.min(st.update.settings_poll)).await;
    }
}

/// Leaves `installing` when pdt-host has taken this pdtd's request and recorded its result while this pdtd kept
/// running (it did not stop, or the host cannot install): the result is shown and the state goes back to resting.
/// Until then (request still waiting, or taken but not decided) nothing changes.
fn settle_handed_off(st: &AppState) {
    let svc = &st.update;
    let Some(handed_off_at) = *svc.handed_off_at.lock().unwrap_or_else(|e| e.into_inner()) else { return };
    if svc.snapshot().state != UpdateState::Installing || request_path(&st.data_dir).exists() {
        return;
    }
    let Some(result) = std::fs::read(result_path(&st.data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<UpdateResult>(&b).ok())
        .filter(|r| r.at >= handed_off_at)
    else {
        return;
    };
    *svc.handed_off_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
    svc.edit(|s| {
        if s.state == UpdateState::Installing {
            s.state = resting(s);
            s.last_result = Some(result);
        }
    });
}

pub(crate) async fn get_status(State(st): State<AppState>) -> Json<UpdateStatus> {
    settle_handed_off(&st);
    refresh_settings(&st).await;
    sync_token_state(&st).await;
    Json(link_tickets(&st, st.update.snapshot()).await)
}

/// A check the user asked for: the status, and what it found (up to date, an update, or why it could not tell).
pub(crate) async fn post_check(State(st): State<AppState>) -> Json<CheckResponse> {
    let (status, result) = check(&st).await;
    Json(CheckResponse { status: link_tickets(&st, status).await, result })
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
    let status = install(&st, b.force, false).await?;
    // A manual update of a skipped version is the retry ruling B allows; the skip is lifted.
    let _ = std::fs::remove_file(skip_path(&st.data_dir));
    Ok(Json(link_tickets(&st, status).await))
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
        return Err(ApiError::bad_request(codes::INVALID_TOKEN, "the token is empty"));
    }
    let expires_at = match st.update.client.latest(&tok).await {
        Err(UpdateError::Unauthorized) => {
            return Err(ApiError::bad_request(codes::INVALID_TOKEN, "GitHub rejected this token"));
        }
        Ok(l) => l.token_expires_at,
        Err(_) => None,
    };
    write_token(&st, Some(tok)).await?;
    st.update.edit(|s| s.token = token_state(true, expires_at, false, now_ms()));
    Ok(Json(link_tickets(&st, check(&st).await.0).await))
}

pub(crate) async fn delete_token(State(st): State<AppState>) -> Result<StatusCode, ApiError> {
    write_token(&st, None).await?;
    let svc = &st.update;
    let mut s = svc.status.lock().unwrap_or_else(|e| e.into_inner());
    s.token = TokenState::Missing;
    if !in_progress(s.state) {
        *svc.release.lock().unwrap_or_else(|e| e.into_inner()) = None;
        s.latest = None;
        s.state = if svc.enabled { UpdateState::Idle } else { UpdateState::Disabled };
    }
    Ok(StatusCode::NO_CONTENT)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post, put};
    axum::Router::new()
        .route("/host/update", get(get_status))
        .route("/host/update/check", post(post_check))
        .route("/host/update/install", post(post_install))
        .route("/host/update/dismiss", post(post_dismiss))
        .route("/host/token", put(put_token).delete(delete_token))
}
