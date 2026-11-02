//! The forge logins (PDT-177, PDT-244): a forge plugin's page (Settings › Plugins › GitHub) lists its accounts, one
//! per host (github.com and any GitHub Enterprise Server at once), and adds one with a pasted token for any host, or
//! with the OAuth device flow on a forge that has one (the user enters a code on the forge; PDT polls for the token).
//! The accounts live in the OS keychain only, as one entry per instance (`secrets::forge_logins_account`: an
//! [`AccountStore`] keyed `<kind>:<host>`); each signs in the plugin's forge for its host ([`ForgeLogin::set_account`]),
//! which a project on that host uses. The single GitHub login from before PDT-244 is copied in on the first read.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_forge::login::{AccountStore, DevicePoll, TokenGrant};
use pdt_forge::{ForgeAccount, ForgeKind, ForgeLogin, ForgePlugin};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ts_rs::TS;

/// A device flow under way.
#[derive(Debug, Clone)]
struct Pending {
    user_code: String,
    verification_uri: String,
    expires_at: i64,
    api_url: String,
}

/// One forge's login flow: the code the user is to enter, or why the last flow failed. `flow` numbers flows, so a
/// cancelled or replaced one's poller stops.
#[derive(Debug, Default)]
struct LoginState {
    pending: Option<Pending>,
    error: Option<String>,
    flow: u64,
}

/// The login flows by forge kind, and the lock every change to the stored accounts takes (read, change, write).
#[derive(Debug, Default)]
pub(crate) struct ForgeLogins {
    flows: Mutex<HashMap<String, LoginState>>,
    keychain: Mutex<()>,
}

/// A device flow's code, for the user to enter on the forge.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForgeDeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    #[ts(type = "number")]
    pub expires_at: i64,
    /// The API the code signs in to.
    pub api_url: String,
}

/// One account a forge is signed in with. Never its token.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForgeAccountView {
    /// The web host, the account's key (`github.com`, `ghe.example.com`).
    pub host: String,
    pub api_url: String,
    pub web_url: String,
    /// Who it signs in as.
    pub login: String,
    #[ts(type = "number | null")]
    pub signed_in_at: Option<i64>,
    /// When the forge stopped accepting it (PDT-212): it is kept, but PDT sends that host nothing until the user signs
    /// in again.
    #[ts(type = "number | null")]
    pub rejected_at: Option<i64>,
    /// When its token expires (PDT-260): None for one that doesn't.
    #[ts(type = "number | null")]
    pub expires_at: Option<i64>,
    /// PDT renews the token before it expires (it has a refresh token).
    pub renews: bool,
}

/// `GET /api/forges/{kind}/login`: a forge's accounts and how to add one.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForgeLoginView {
    pub kind: String,
    pub label: String,
    /// What the forge calls a change request (`pull request`, GitLab's `merge request`), for its page's wording.
    pub change_request_name: String,
    /// The public host's API, for the form (github.com's); None when the forge is only self-hosted.
    pub default_api_url: Option<String>,
    /// An example of a self-hosted server's API URL.
    pub api_url_example: String,
    /// The scopes a token needs, as the forge names them.
    pub scopes: String,
    pub token_help: String,
    /// The forge has a device flow (on another host it needs that host's OAuth App client ID).
    pub has_device_flow: bool,
    /// PDT has an OAuth App client ID for the public host, so the device flow works there without one.
    pub device_flow: bool,
    /// The accounts, the public host's first.
    pub accounts: Vec<ForgeAccountView>,
    /// The code to enter, while a device flow waits for it.
    pub pending: Option<ForgeDeviceCode>,
    /// Why the last login failed.
    pub error: Option<String>,
}

/// `POST /api/forges/{kind}/login/test`: who a token signs in as, without saving it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForgeTokenCheck {
    pub host: String,
    pub api_url: String,
    pub web_url: String,
    pub login: String,
    /// The scopes the forge says the token has (none for a GitHub fine-grained token, which has permissions instead).
    pub scopes: Vec<String>,
}

fn bad(code: &'static str, m: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, code, m)
}

/// The plugin of `kind`, when it has a login.
fn plugin(st: &AppState, kind: &str) -> Result<Arc<dyn ForgePlugin>, ApiError> {
    ForgeKind::new(kind).and_then(|k| st.forges.get(&k).cloned()).filter(|p| p.login().is_some()).ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, "unknown_forge", format!("PDT has no forge {kind} to sign in to"))
    })
}

fn login_of(p: &dyn ForgePlugin) -> &dyn ForgeLogin {
    p.login().expect("a plugin with a login")
}

/// The OAuth App PDT's device flow uses on `kind`'s public host; empty when it has none.
fn client_id(st: &AppState, kind: &ForgeKind) -> String {
    let id = if *kind == ForgeKind::GITHUB {
        st.github_client_id.as_str()
    } else {
        st.forge_client_ids.get(kind.as_str()).map_or("", String::as_str)
    };
    id.trim().to_string()
}

fn flows(st: &AppState) -> std::sync::MutexGuard<'_, HashMap<String, LoginState>> {
    st.forge_logins.flows.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn view(st: &AppState, p: &dyn ForgePlugin) -> ForgeLoginView {
    let l = login_of(p);
    let spec = l.spec();
    let kind = p.kind();
    let accounts = l
        .accounts()
        .into_iter()
        .map(|a| ForgeAccountView {
            host: a.host,
            web_url: l.web_url(&a.account.api_url),
            api_url: a.account.api_url,
            login: a.account.login,
            signed_in_at: a.account.signed_in_at,
            rejected_at: a.rejected_at,
            expires_at: a.account.expires_at,
            renews: a.account.refresh.is_some(),
        })
        .collect();
    let flows = flows(st);
    let s = flows.get(kind.as_str());
    ForgeLoginView {
        kind: kind.to_string(),
        label: p.label().into(),
        change_request_name: p.change_request_name().into(),
        default_api_url: spec.default_api_url,
        api_url_example: spec.api_url_example,
        scopes: spec.scopes,
        token_help: spec.token_help,
        has_device_flow: spec.device_flow,
        device_flow: spec.device_flow && !client_id(st, &kind).is_empty(),
        accounts,
        pending: s.and_then(|s| s.pending.as_ref()).map(|p| ForgeDeviceCode {
            user_code: p.user_code.clone(),
            verification_uri: p.verification_uri.clone(),
            expires_at: p.expires_at,
            api_url: p.api_url.clone(),
        }),
        error: s.and_then(|s| s.error.clone()),
    }
}

/// Whether any forge plugin is signed in to `kind` (the setup checklist's GitHub step).
pub(crate) fn connected(st: &AppState, kind: &ForgeKind) -> bool {
    st.forges.get(kind).and_then(|p| p.login().map(|l| !l.accounts().is_empty())).unwrap_or(false)
}

fn keychain_error(e: crate::secrets::SecretError) -> ApiError {
    tracing::warn!(error = %e, "keychain call failed");
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "keychain_unavailable", e.to_string())
}

/// The stored accounts. Without any yet, the GitHub login from before PDT-244 (`legacy`) becomes the account of its
/// host, and is written so; the old entry stays, for an older PDT rolled back to. Blocking.
fn read_store(
    secrets: &dyn crate::secrets::SecretStore,
    name: &str,
    legacy: &str,
) -> Result<AccountStore, crate::secrets::SecretError> {
    if let Some(v) = secrets.get(name)? {
        return Ok(serde_json::from_str(&v).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "the stored forge logins could not be read: starting with none");
            AccountStore::default()
        }));
    }
    let Some(old) = secrets.get(legacy)? else { return Ok(AccountStore::default()) };
    let a = match serde_json::from_str::<ForgeAccount>(&old) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "the stored GitHub login could not be read");
            return Ok(AccountStore::default());
        }
    };
    let host = pdt_github::login::web_host(&a.api_url);
    tracing::info!(login = %a.login, host, "the GitHub login becomes the forge account github:{host}");
    let store = AccountStore::migrated(&ForgeKind::GITHUB, &host, a);
    secrets.set(name, &serde_json::to_string(&store).map_err(|e| crate::secrets::SecretError(e.to_string()))?)?;
    Ok(store)
}

/// Stores (`Some`) or removes (`None`) `kind`'s account on `host`. Removing the host the old GitHub login was for
/// removes that entry too, so a disconnected token leaves the keychain.
async fn store_account(
    st: &AppState,
    kind: &ForgeKind,
    host: &str,
    account: Option<ForgeAccount>,
) -> Result<(), ApiError> {
    let (secrets, name, legacy, logins) =
        (st.secrets.clone(), st.forge_logins_account.clone(), st.github_account.clone(), st.forge_logins.clone());
    let (kind, host) = (kind.clone(), host.to_string());
    tokio::task::spawn_blocking(move || {
        let _held = logins.keychain.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = read_store(secrets.as_ref(), &name, &legacy)?;
        let removed = account.is_none();
        match account {
            Some(a) => store.put(&kind, &host, a),
            None => {
                store.remove(&kind, &host);
            }
        }
        secrets.set(&name, &serde_json::to_string(&store).map_err(|e| crate::secrets::SecretError(e.to_string()))?)?;
        if removed
            && kind == ForgeKind::GITHUB
            && let Some(old) = secrets.get(&legacy)?
            && serde_json::from_str::<ForgeAccount>(&old).is_ok_and(|a| pdt_github::login::web_host(&a.api_url) == host)
        {
            secrets.delete(&legacy)?;
        }
        Ok(())
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(keychain_error)
}

/// Has every forge store the accounts it renews itself (PDT-260), as soon as it has: the old refresh token no longer
/// works, so a new one lost is a login lost at the next start. Only over the account it renewed, not over one the user
/// signed in with meanwhile.
fn watch_renewals(st: &AppState) {
    for p in st.forges.plugins() {
        let Some(l) = p.login() else { continue };
        let (secrets, name, legacy, logins) =
            (st.secrets.clone(), st.forge_logins_account.clone(), st.github_account.clone(), st.forge_logins.clone());
        let kind = p.kind();
        l.on_renewed(Arc::new(move |host: &str, old: &str, a: &ForgeAccount| {
            let _held = logins.keychain.lock().unwrap_or_else(|e| e.into_inner());
            let saved = read_store(secrets.as_ref(), &name, &legacy).and_then(|mut store| {
                if store.get(&kind, host).is_none_or(|x| x.token != old) {
                    tracing::info!(forge = %kind, host, "a renewed login was replaced meanwhile: not stored");
                    return Ok(());
                }
                store.put(&kind, host, a.clone());
                let json = serde_json::to_string(&store).map_err(|e| crate::secrets::SecretError(e.to_string()))?;
                secrets.set(&name, &json)
            });
            if let Err(e) = saved {
                tracing::warn!(forge = %kind, host, error = %e, "the renewed login could not be stored: it lasts until pdtd restarts");
            }
        }));
    }
}

/// Stores or removes the account of `host`, then signs that host's forge in (or out).
async fn save(
    st: &AppState,
    p: &Arc<dyn ForgePlugin>,
    host: &str,
    account: Option<ForgeAccount>,
) -> Result<(), ApiError> {
    store_account(st, &p.kind(), host, account.clone()).await?;
    watch_renewals(st);
    let l = login_of(p.as_ref());
    // The pollers waited for this host's login (none, or one the forge refused): they look again now.
    let waited = l.accounts().into_iter().find(|a| a.host == host).is_none_or(|a| a.rejected_at.is_some());
    let signed_in = account.is_some();
    l.set_account(host, account);
    crate::notifications::kick(st);
    if signed_in && waited {
        let st = st.clone();
        tokio::spawn(async move {
            crate::refresh_prs(&st).await;
            crate::refresh_issues(&st).await;
            crate::refresh_pulls(&st).await;
        });
    }
    Ok(())
}

/// Signs every forge in with its stored accounts (pdtd's startup); copies in the old GitHub login the first time.
pub async fn load(st: &AppState) {
    let (secrets, name, legacy) = (st.secrets.clone(), st.forge_logins_account.clone(), st.github_account.clone());
    let logins = st.forge_logins.clone();
    let store = match tokio::task::spawn_blocking(move || {
        let _held = logins.keychain.lock().unwrap_or_else(|e| e.into_inner());
        read_store(secrets.as_ref(), &name, &legacy)
    })
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return tracing::warn!(error = %e, "the forge logins could not be read from the keychain"),
        Err(e) => return tracing::warn!(error = %e, "reading the forge logins failed"),
    };
    watch_renewals(st);
    for p in st.forges.plugins() {
        let Some(l) = p.login() else { continue };
        for (host, a) in store.of_kind(&p.kind()) {
            let age = a.signed_in_at.map(|t| (now_ms() - t) / 60_000);
            let expires_in_min = a.expires_at.map(|t| (t - now_ms()) / 60_000);
            tracing::info!(forge = %p.kind(), host, login = %a.login, api = %a.api_url, signed_in_min_ago = ?age, ?expires_in_min, renews = a.refresh.is_some(), "using the stored forge login");
            l.set_account(host, Some(a.clone()));
        }
    }
}

/// How often [`check`] runs.
pub const CHECK_EVERY: Duration = Duration::from_secs(5 * 60);

/// The light login check (PDT-252), every [`CHECK_EVERY`], of every account: tells when a forge stops accepting one,
/// to the minute, and clears a refused account's mark once the forge accepts it again (then the pollers look at once).
pub async fn check(st: &AppState) {
    let mut recovered = false;
    // Only the forges in use: a turned-off GitHub isn't called (PDT-251).
    for p in st.forge_plugins() {
        if p.login().is_none() {
            continue;
        }
        recovered |= tokio::task::spawn_blocking(move || login_of(p.as_ref()).check_accounts()).await.unwrap_or(false);
    }
    log_refused(st).await;
    if recovered {
        crate::notifications::kick(st);
        crate::refresh_prs(st).await;
        crate::refresh_issues(st).await;
        crate::refresh_pulls(st).await;
    }
}

/// The refusals [`log_refused`] logged, by account: the event log forgets after a while, a refusal is news once.
const REFUSED_KEY: &str = "sensor.forge_refused";

/// Each account a forge refuses is a `connection.failed` event (PDT-365), once per refusal (its time is in the key).
pub(crate) async fn log_refused(st: &AppState) {
    let before = st.store.get_host_value(REFUSED_KEY).await.ok().flatten().unwrap_or_else(|| json!({}));
    let mut after = serde_json::Map::new();
    for p in st.forge_plugins() {
        let Some(l) = p.login() else { continue };
        for a in l.accounts() {
            let Some(at) = a.rejected_at else { continue };
            let key = format!("forge:{}:{}:{at}", p.kind(), a.host);
            let account = format!("{}:{}", p.kind(), a.host);
            after.insert(account.clone(), json!(key));
            if before[&account].as_str() == Some(key.as_str()) {
                continue;
            }
            let name = format!("{} ({})", p.label(), a.host);
            let error = format!("{} refused the login of {} on {}: sign in again", p.label(), a.account.login, a.host);
            crate::connection_health::log_failed(st, &name, p.kind().as_str(), "forge", &error, &key).await;
        }
    }
    let after = serde_json::Value::Object(after);
    if after != before
        && let Err(e) = st.store.put_host_value(REFUSED_KEY, &after).await
    {
        tracing::warn!(error = %e, "could not record the refused forge logins");
    }
}

/// How the user signed in, for the log.
enum How<'a> {
    Pasted,
    DeviceFlow { client_id: &'a str },
}

/// Asks the forge who `token` is on `api_url`. Err is the forge's refusal, for the user.
async fn whoami(p: &Arc<dyn ForgePlugin>, api_url: &str, token: &str) -> Result<pdt_forge::login::Whoami, String> {
    let (p, api, tok) = (p.clone(), api_url.to_string(), token.to_string());
    tokio::task::spawn_blocking(move || login_of(p.as_ref()).whoami(&api, &tok)).await.map_err(|e| e.to_string())?
}

/// Checks `grant`'s token with the forge and saves it as the account of `api_url`'s host, with its expiry and refresh
/// token when it has them (PDT-260).
async fn sign_in(
    st: &AppState,
    p: &Arc<dyn ForgePlugin>,
    grant: TokenGrant,
    api_url: String,
    how: How<'_>,
) -> Result<(), String> {
    let who = whoami(p, &api_url, &grant.token).await?;
    let host = login_of(p.as_ref()).host_of(&api_url);
    // Every sign-in is logged (PDT-252): which instance, how, and with which OAuth App. Never the token.
    let (how, client_id) = match how {
        How::Pasted => ("pasted token", ""),
        How::DeviceFlow { client_id } => ("device flow", client_id),
    };
    let now = now_ms();
    let (expires_at, refresh) = grant.renewal(now, client_id);
    let expires_in_min = expires_at.map(|t| (t - now) / 60_000);
    tracing::info!(
        instance = %st.forge_logins_account, data_dir = %st.data_dir.display(), forge = %p.kind(), host, how,
        client_id, login = %who.login, api = %api_url, scopes = %who.scopes.join(","), ?expires_in_min,
        renews = refresh.is_some(), "signed in to a forge"
    );
    let account =
        ForgeAccount { token: grant.token, api_url, login: who.login, signed_in_at: Some(now), expires_at, refresh };
    save(st, p, &host, Some(account)).await.map_err(|e| e.message().to_string())
}

/// `GET /api/forges/{kind}/login`.
pub(crate) async fn get(
    State(st): State<AppState>,
    Path(kind): Path<String>,
) -> Result<Json<ForgeLoginView>, ApiError> {
    let p = plugin(&st, &kind)?;
    Ok(Json(view(&st, p.as_ref())))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TokenBody {
    token: String,
    /// The host's API URL; the public host's when left out.
    #[serde(default)]
    api_url: Option<String>,
}

impl TokenBody {
    fn parts(&self, l: &dyn ForgeLogin) -> Result<(String, String), ApiError> {
        let token = self.token.trim().to_string();
        if token.is_empty() {
            return Err(bad("invalid_token", "the token is empty"));
        }
        let api = l.api_url(self.api_url.as_deref().unwrap_or_default());
        Ok((token, api))
    }
}

/// `POST /api/forges/{kind}/login/test`: who a pasted token signs in as, without saving it.
pub(crate) async fn test_token(
    State(st): State<AppState>,
    Path(kind): Path<String>,
    payload: Result<Json<TokenBody>, JsonRejection>,
) -> Result<Json<ForgeTokenCheck>, ApiError> {
    let p = plugin(&st, &kind)?;
    let Json(b) = payload?;
    let l = login_of(p.as_ref());
    let (token, api) = b.parts(l)?;
    let who = whoami(&p, &api, &token).await.map_err(|e| bad("invalid_token", e))?;
    Ok(Json(ForgeTokenCheck {
        host: l.host_of(&api),
        web_url: l.web_url(&api),
        api_url: api,
        login: who.login,
        scopes: who.scopes,
    }))
}

/// `POST /api/forges/{kind}/login/token`: signs in to a host with a pasted token (a personal access token, or `gh auth
/// token`'s), replacing that host's account.
pub(crate) async fn put_token(
    State(st): State<AppState>,
    Path(kind): Path<String>,
    payload: Result<Json<TokenBody>, JsonRejection>,
) -> Result<Json<ForgeLoginView>, ApiError> {
    let p = plugin(&st, &kind)?;
    let Json(b) = payload?;
    let (token, api) = b.parts(login_of(p.as_ref()))?;
    sign_in(&st, &p, TokenGrant { token, ..TokenGrant::default() }, api, How::Pasted)
        .await
        .map_err(|e| bad("invalid_token", e))?;
    cancel(&st, &p.kind());
    Ok(Json(view(&st, p.as_ref())))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeviceBody {
    #[serde(default)]
    api_url: Option<String>,
    /// The OAuth App's client ID, for another host (its own app); PDT's when left out.
    #[serde(default)]
    client_id: Option<String>,
}

/// Ends `kind`'s device flow under way (its poller stops at its next look).
fn cancel(st: &AppState, kind: &ForgeKind) {
    let mut f = flows(st);
    let s = f.entry(kind.to_string()).or_default();
    s.flow += 1;
    s.pending = None;
}

/// `POST /api/forges/{kind}/login/device`: starts the device flow. The answer has the code to enter; PDT polls the
/// forge in the background and signs in once the user has entered it (`GET` shows the account, or the error).
pub(crate) async fn start_device(
    State(st): State<AppState>,
    Path(kind): Path<String>,
    payload: Result<Json<DeviceBody>, JsonRejection>,
) -> Result<Json<ForgeLoginView>, ApiError> {
    let p = plugin(&st, &kind)?;
    let Json(b) = payload?;
    let l = login_of(p.as_ref());
    if !l.spec().device_flow {
        return Err(bad("no_device_flow", format!("{} has no device flow: paste a token instead", p.label())));
    }
    let api = l.api_url(b.api_url.as_deref().unwrap_or_default());
    let client_id = b
        .client_id
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| client_id(&st, &p.kind()));
    if client_id.is_empty() {
        return Err(bad("no_client_id", "no OAuth App client ID is set: paste a token instead"));
    }
    let (pp, a, c) = (p.clone(), api.clone(), client_id.clone());
    let code = tokio::task::spawn_blocking(move || login_of(pp.as_ref()).start_device_flow(&a, &c))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| bad("device_flow_failed", e))?;
    let flow = {
        let mut f = flows(&st);
        let s = f.entry(p.kind().to_string()).or_default();
        s.flow += 1;
        s.error = None;
        s.pending = Some(Pending {
            user_code: code.user_code.clone(),
            verification_uri: code.verification_uri.clone(),
            expires_at: now_ms() + i64::try_from(code.expires_in).unwrap_or(900) * 1000,
            api_url: api.clone(),
        });
        s.flow
    };
    let interval = Duration::from_secs(code.interval);
    tokio::spawn(poll(st.clone(), p.clone(), flow, api, client_id, code.device_code, interval));
    Ok(Json(view(&st, p.as_ref())))
}

/// Polls the device flow `flow` until the user entered the code, it failed or expired, or another flow replaced it.
async fn poll(
    st: AppState,
    p: Arc<dyn ForgePlugin>,
    flow: u64,
    api: String,
    client_id: String,
    device_code: String,
    mut interval: Duration,
) {
    let kind = p.kind().to_string();
    let current = |st: &AppState| {
        let f = flows(st);
        f.get(&kind).filter(|s| s.flow == flow).and_then(|s| s.pending.as_ref().map(|p| p.expires_at))
    };
    let end = |st: &AppState, error: Option<String>| {
        let mut f = flows(st);
        if let Some(s) = f.get_mut(&kind).filter(|s| s.flow == flow) {
            s.pending = None;
            s.error = error;
        }
    };
    let expired = || Some("The code expired before it was entered: connect again.".to_string());
    loop {
        tokio::time::sleep(interval).await;
        let Some(expires_at) = current(&st) else { return };
        if now_ms() >= expires_at {
            return end(&st, expired());
        }
        let (pp, a, c, d) = (p.clone(), api.clone(), client_id.clone(), device_code.clone());
        let got = tokio::task::spawn_blocking(move || login_of(pp.as_ref()).poll_device_flow(&a, &c, &d))
            .await
            .unwrap_or_else(|e| DevicePoll::Failed(e.to_string()));
        if current(&st).is_none() {
            return;
        }
        match got {
            DevicePoll::Pending => {}
            DevicePoll::SlowDown(s) => interval = Duration::from_secs(s),
            DevicePoll::Token(grant) => {
                let r = sign_in(&st, &p, grant, api.clone(), How::DeviceFlow { client_id: &client_id }).await;
                return end(&st, r.err());
            }
            DevicePoll::Expired => return end(&st, expired()),
            DevicePoll::Denied => return end(&st, Some(format!("The login was cancelled on {}.", p.label()))),
            DevicePoll::Failed(e) => return end(&st, Some(e)),
        }
    }
}

/// `DELETE /api/forges/{kind}/login/device`: stops waiting for the code.
pub(crate) async fn cancel_device(
    State(st): State<AppState>,
    Path(kind): Path<String>,
) -> Result<Json<ForgeLoginView>, ApiError> {
    let p = plugin(&st, &kind)?;
    cancel(&st, &p.kind());
    Ok(Json(view(&st, p.as_ref())))
}

/// `DELETE /api/forges/{kind}/login/accounts/{host}`: signs out of one host (its account leaves the keychain; the
/// forge keeps an OAuth App's authorization until the user revokes it there).
pub(crate) async fn disconnect(
    State(st): State<AppState>,
    Path((kind, host)): Path<(String, String)>,
) -> Result<Json<ForgeLoginView>, ApiError> {
    let p = plugin(&st, &kind)?;
    let host = host.trim().to_ascii_lowercase();
    if !login_of(p.as_ref()).accounts().iter().any(|a| a.host == host) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "no_account",
            format!("{} has no account on {host}", p.label()),
        ));
    }
    save(&st, &p, &host, None).await?;
    if let Some(s) = flows(&st).get_mut(p.kind().as_str()) {
        s.error = None;
    }
    Ok(Json(view(&st, p.as_ref())))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{delete, get, post};
    axum::Router::new()
        .route("/forges/{kind}/login", get(self::get))
        .route("/forges/{kind}/login/test", post(test_token))
        .route("/forges/{kind}/login/token", post(put_token))
        .route("/forges/{kind}/login/device", post(start_device).delete(cancel_device))
        .route("/forges/{kind}/login/accounts/{host}", delete(disconnect))
}
