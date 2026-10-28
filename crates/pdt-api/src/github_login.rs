//! The in-app GitHub login (PDT-177): Settings › This host › GitHub. "Connect GitHub" runs GitHub's OAuth device
//! flow (the user enters a code on GitHub; PDT polls for the token), or the user pastes a token. The login (token, API
//! URL, who it is) lives in the OS keychain only, under this instance's own account
//! (`secrets::github_login_account`), and the GitHub forge ([`pdt_github::GithubApi`]) signs in with it. Without
//! one the forge refuses every call and the Inbox asks the user to connect.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use pdt_github::Account;
use pdt_github::login::{self, DevicePoll};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use ts_rs::TS;

/// A device flow under way.
#[derive(Debug, Clone)]
struct Pending {
    user_code: String,
    verification_uri: String,
    expires_at: i64,
}

/// The login flow's state: the code the user is to enter, or why the last flow failed. `flow` numbers flows, so a
/// cancelled or replaced one's poller stops.
#[derive(Debug, Default)]
pub(crate) struct LoginState {
    pending: Option<Pending>,
    error: Option<String>,
    flow: u64,
}

/// A device flow's code, for the user to enter on GitHub.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubDeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    #[ts(type = "number")]
    pub expires_at: i64,
}

/// `GET /api/github/login`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubLoginView {
    pub connected: bool,
    /// Who PDT is signed in as.
    pub login: Option<String>,
    /// The API the login is for (github.com's when not connected).
    pub api_url: String,
    pub web_url: String,
    /// An OAuth App client ID is set, so Connect GitHub (the device flow) is offered.
    pub device_flow: bool,
    /// The OAuth scopes PDT asks for.
    pub scopes: String,
    /// The code to enter, while a device flow waits for it.
    pub pending: Option<GithubDeviceCode>,
    /// Why the last login failed.
    pub error: Option<String>,
    /// When GitHub stopped accepting the login (PDT-212): it is kept, but PDT sends GitHub nothing until the user
    /// connects again.
    #[ts(type = "number | null")]
    pub rejected_at: Option<i64>,
}

pub(crate) fn view(st: &AppState) -> GithubLoginView {
    let account = st.github_api.account();
    let s = st.github_login.lock().unwrap_or_else(|e| e.into_inner());
    let api_url = account.as_ref().map_or_else(|| login::GITHUB_API.to_string(), |a| a.api_url.clone());
    GithubLoginView {
        connected: account.is_some(),
        login: account.map(|a| a.login),
        web_url: login::web_url(&api_url),
        api_url,
        device_flow: !st.github_client_id.trim().is_empty(),
        scopes: login::SCOPES.into(),
        pending: s.pending.as_ref().map(|p| GithubDeviceCode {
            user_code: p.user_code.clone(),
            verification_uri: p.verification_uri.clone(),
            expires_at: p.expires_at,
        }),
        error: s.error.clone(),
        rejected_at: st.github_api.rejected_at(),
    }
}

fn keychain_error(e: crate::secrets::SecretError) -> ApiError {
    tracing::warn!(error = %e, "keychain call failed");
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "keychain_unavailable", e.to_string())
}

/// Stores (`Some`) or deletes (`None`) the login in the keychain, then signs the forge in (or out).
async fn save(st: &AppState, account: Option<Account>) -> Result<(), ApiError> {
    let (secrets, name) = (st.secrets.clone(), st.github_account.clone());
    let value = match &account {
        Some(a) => Some(serde_json::to_string(a).map_err(ApiError::internal)?),
        None => None,
    };
    tokio::task::spawn_blocking(move || match value {
        Some(v) => secrets.set(&name, &v),
        None => secrets.delete(&name),
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(keychain_error)?;
    // The pollers waited for a login (none, or one GitHub refused): they look again now, not at their next tick.
    let waited = st.github_api.rejected_at().is_some() || st.github_api.account().is_none();
    let signed_in = account.is_some();
    st.github_api.set_account(account);
    crate::notifications::kick(st);
    if signed_in && waited {
        let st = st.clone();
        tokio::spawn(async move {
            crate::refresh_prs(&st).await;
            crate::refresh_issues(&st).await;
        });
    }
    Ok(())
}

/// Signs the forge in with the login in the keychain, if there is one (pdtd's startup).
pub async fn load(st: &AppState) {
    let (secrets, name) = (st.secrets.clone(), st.github_account.clone());
    match tokio::task::spawn_blocking(move || secrets.get(&name)).await {
        Ok(Ok(Some(v))) => match serde_json::from_str::<Account>(&v) {
            Ok(a) => st.github_api.set_account(Some(a)),
            Err(e) => tracing::warn!(error = %e, "the stored GitHub login could not be read"),
        },
        Ok(Ok(None)) => {}
        Ok(Err(e)) => tracing::warn!(error = %e, "the GitHub login could not be read from the keychain"),
        Err(e) => tracing::warn!(error = %e, "reading the GitHub login failed"),
    }
}

/// Checks `token` with GitHub and saves the login.
async fn sign_in(st: &AppState, token: String, api_url: String) -> Result<(), String> {
    let t = st.github_api.transport();
    let (tok, api) = (token.clone(), api_url.clone());
    let who = tokio::task::spawn_blocking(move || login::whoami(t.as_ref(), &api, &tok))
        .await
        .map_err(|e| e.to_string())??;
    save(st, Some(Account { token, api_url, login: who.login })).await.map_err(|e| e.message().to_string())
}

fn bad(code: &'static str, m: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, code, m)
}

/// `GET /api/github/login`.
pub(crate) async fn get(State(st): State<AppState>) -> Json<GithubLoginView> {
    Json(view(&st))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TokenBody {
    token: String,
    /// A GitHub Enterprise Server's API URL; github.com's when left out.
    #[serde(default)]
    api_url: Option<String>,
}

/// `POST /api/github/login/token`: signs in with a pasted token (a personal access token, or `gh auth token`'s).
pub(crate) async fn put_token(
    State(st): State<AppState>,
    payload: Result<Json<TokenBody>, JsonRejection>,
) -> Result<Json<GithubLoginView>, ApiError> {
    let Json(b) = payload?;
    let token = b.token.trim().to_string();
    if token.is_empty() {
        return Err(bad("invalid_token", "the token is empty"));
    }
    let api = login::api_url(b.api_url.as_deref().unwrap_or_default());
    sign_in(&st, token, api).await.map_err(|e| bad("invalid_token", e))?;
    cancel(&st);
    Ok(Json(view(&st)))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeviceBody {
    #[serde(default)]
    api_url: Option<String>,
    /// The OAuth App's client ID, for a GitHub Enterprise Server (its own app); PDT's when left out.
    #[serde(default)]
    client_id: Option<String>,
}

/// Ends any device flow under way (its poller stops at its next look).
fn cancel(st: &AppState) {
    let mut s = st.github_login.lock().unwrap_or_else(|e| e.into_inner());
    s.flow += 1;
    s.pending = None;
}

/// `POST /api/github/login/device`: starts Connect GitHub. The answer has the code to enter; PDT polls GitHub in the
/// background and signs in once the user has entered it (`GET` shows it connected, or the error).
pub(crate) async fn start_device(
    State(st): State<AppState>,
    payload: Result<Json<DeviceBody>, JsonRejection>,
) -> Result<Json<GithubLoginView>, ApiError> {
    let Json(b) = payload?;
    let api = login::api_url(b.api_url.as_deref().unwrap_or_default());
    let client_id = b.client_id.filter(|c| !c.trim().is_empty()).unwrap_or_else(|| st.github_client_id.clone());
    let client_id = client_id.trim().to_string();
    if client_id.is_empty() {
        return Err(bad("no_client_id", "no OAuth App client ID is set: paste a token instead"));
    }
    let t = st.github_api.transport();
    let (a, c) = (api.clone(), client_id.clone());
    let code = tokio::task::spawn_blocking(move || login::start_device_flow(t.as_ref(), &a, &c))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| bad("device_flow_failed", e))?;
    let flow = {
        let mut s = st.github_login.lock().unwrap_or_else(|e| e.into_inner());
        s.flow += 1;
        s.error = None;
        s.pending = Some(Pending {
            user_code: code.user_code.clone(),
            verification_uri: code.verification_uri.clone(),
            expires_at: now_ms() + i64::try_from(code.expires_in).unwrap_or(900) * 1000,
        });
        s.flow
    };
    let interval = Duration::from_secs(code.interval);
    tokio::spawn(poll(st.clone(), flow, api, client_id, code.device_code, interval));
    Ok(Json(view(&st)))
}

/// Polls the device flow `flow` until the user entered the code, it failed or expired, or another flow replaced it.
async fn poll(st: AppState, flow: u64, api: String, client_id: String, device_code: String, mut interval: Duration) {
    let current = |st: &AppState| {
        let s = st.github_login.lock().unwrap_or_else(|e| e.into_inner());
        (s.flow == flow).then(|| s.pending.as_ref().map(|p| p.expires_at)).flatten()
    };
    let end = |st: &AppState, error: Option<String>| {
        let mut s = st.github_login.lock().unwrap_or_else(|e| e.into_inner());
        if s.flow == flow {
            s.pending = None;
            s.error = error;
        }
    };
    loop {
        tokio::time::sleep(interval).await;
        let Some(expires_at) = current(&st) else { return };
        if now_ms() >= expires_at {
            return end(&st, Some("The code expired before it was entered: connect again.".into()));
        }
        let t = st.github_api.transport();
        let (a, c, d) = (api.clone(), client_id.clone(), device_code.clone());
        let got = tokio::task::spawn_blocking(move || login::poll_device_flow(t.as_ref(), &a, &c, &d))
            .await
            .unwrap_or_else(|e| DevicePoll::Failed(e.to_string()));
        if current(&st).is_none() {
            return;
        }
        match got {
            DevicePoll::Pending => {}
            DevicePoll::SlowDown(s) => interval = Duration::from_secs(s),
            DevicePoll::Token(token) => {
                let r = sign_in(&st, token, api.clone()).await;
                return end(&st, r.err());
            }
            DevicePoll::Expired => {
                return end(&st, Some("The code expired before it was entered: connect again.".into()));
            }
            DevicePoll::Denied => return end(&st, Some("The login was cancelled on GitHub.".into())),
            DevicePoll::Failed(e) => return end(&st, Some(e)),
        }
    }
}

/// `DELETE /api/github/login/device`: stops waiting for the code.
pub(crate) async fn cancel_device(State(st): State<AppState>) -> Json<GithubLoginView> {
    cancel(&st);
    Json(view(&st))
}

/// `DELETE /api/github/login`: disconnects (the login leaves the keychain; GitHub keeps the app's authorization until
/// the user revokes it there).
pub(crate) async fn disconnect(State(st): State<AppState>) -> Result<Json<GithubLoginView>, ApiError> {
    cancel(&st);
    save(&st, None).await?;
    st.github_login.lock().unwrap_or_else(|e| e.into_inner()).error = None;
    Ok(Json(view(&st)))
}
