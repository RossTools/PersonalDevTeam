//! OAuth sign-in for remote MCP servers (PDT-322), PDT as the MCP authorization spec's client: the server's protected
//! resource metadata (RFC 9728) names its authorization server, whose metadata (RFC 8414 / OpenID discovery) gives
//! the endpoints; PDT registers itself (RFC 7591) unless the user gave a client id, and signs in with the
//! authorization code flow, PKCE (S256) and the resource indicator, the browser coming back to
//! `/api/mcp-oauth/callback` on whichever address the user reached PDT at.
//!
//! The tokens live in PDT's secret store (account `mcp-oauth:<connection id>`) and the redactor, never the database,
//! and are refreshed before they expire (when a session starts, and every minute in the background). Agents get a
//! bearer header ([`access_token`]); a sign-in that can't be refreshed (revoked, expired) is an Inbox item until the
//! user signs in again.

use crate::error::{ApiError, OrNotFound, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use pdt_core::inbox::{InboxGroup, InboxItem, InboxKind};
use pdt_core::mcp_servers::{self as ms, AuthServer, OAuthTokens, TokenResponse};
use pdt_core::workflow::api::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use ts_rs::TS;

/// The callback's path; the redirect URI is the address the user reached PDT at, plus this.
pub const CALLBACK: &str = "/api/mcp-oauth/callback";
/// How long a sign-in begun may take to come back.
const PENDING_MS: i64 = 15 * 60 * 1000;
const TIMEOUT: Duration = Duration::from_secs(20);

fn account(id: &str) -> String {
    format!("mcp-oauth:{id}")
}

/// A sign-in begun: what the callback needs to finish it.
#[derive(Debug, Clone)]
struct Pending {
    connection_id: String,
    verifier: String,
    client_id: String,
    client_secret: Option<String>,
    token_endpoint: String,
    redirect_uri: String,
    resource: String,
    at: i64,
}

/// The tokens known (by connection id) and the sign-ins under way (by `state`).
#[derive(Default)]
pub struct OAuthCache {
    tokens: Mutex<HashMap<String, Option<OAuthTokens>>>,
    pending: Mutex<HashMap<String, Pending>>,
    /// By connection id: held while that server's tokens are refreshed, one refresh at a time per server (a slow
    /// service holds up no other).
    refreshing: Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
}

/// A server's sign-in, as Settings shows it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OAuthStatus {
    pub signed_in: bool,
    pub account: Option<String>,
    #[ts(type = "number | null")]
    pub expires_at: Option<i64>,
    #[ts(type = "number | null")]
    pub signed_in_at: Option<i64>,
    /// Why it can't be used: refreshing failed (revoked), or it expired with nothing to refresh it.
    pub problem: Option<String>,
}

async fn keychain_get(st: &AppState, id: &str) -> Result<Option<OAuthTokens>, String> {
    let (secrets, acct) = (st.secrets.clone(), account(id));
    let what = "an MCP server's sign-in".to_string();
    let raw = tokio::task::spawn_blocking(move || secrets.get(&acct).map_err(|e| e.naming(&acct, &what)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(raw.and_then(|r| serde_json::from_str(&r).ok()))
}

fn redact(st: &AppState, name: &str, t: &OAuthTokens) {
    st.store.redactor().set(&format!("mcp-{name}-access"), &t.access_token);
    if let Some(r) = &t.refresh_token {
        st.store.redactor().set(&format!("mcp-{name}-refresh"), r);
    }
}

/// A connection's tokens: known already, else from the secret store (and known from then on).
pub(crate) async fn tokens(st: &AppState, conn: &Connection) -> Result<Option<OAuthTokens>, String> {
    if let Some(t) = st.mcp.oauth.tokens.lock().unwrap_or_else(|e| e.into_inner()).get(&conn.id) {
        return Ok(t.clone());
    }
    let t = keychain_get(st, &conn.id).await?;
    if let Some(t) = &t {
        redact(st, &conn.name, t);
    }
    st.mcp.oauth.tokens.lock().unwrap_or_else(|e| e.into_inner()).insert(conn.id.clone(), t.clone());
    Ok(t)
}

async fn save(st: &AppState, conn: &Connection, t: &OAuthTokens) -> Result<(), String> {
    let (secrets, acct, text) = (st.secrets.clone(), account(&conn.id), serde_json::to_string(t).unwrap_or_default());
    tokio::task::spawn_blocking(move || secrets.set(&acct, &text))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    redact(st, &conn.name, t);
    st.mcp.oauth.tokens.lock().unwrap_or_else(|e| e.into_inner()).insert(conn.id.clone(), Some(t.clone()));
    Ok(())
}

/// Forgets a connection's sign-in (Sign out, or the connection deleted). Err: the keychain kept the tokens, so the
/// connection is still signed in.
pub(crate) async fn forget(st: &AppState, id: &str) -> Result<(), ApiError> {
    st.mcp.oauth.refreshing.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    let (secrets, acct) = (st.secrets.clone(), account(id));
    tokio::task::spawn_blocking(move || secrets.delete(&acct))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::keychain)?;
    st.mcp.oauth.tokens.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), None);
    Ok(())
}

pub(crate) fn status_of(t: Option<&OAuthTokens>, now: i64) -> OAuthStatus {
    let Some(t) = t else {
        return OAuthStatus { signed_in: false, account: None, expires_at: None, signed_in_at: None, problem: None };
    };
    let problem = t
        .failed
        .clone()
        .or_else(|| (!t.usable(now) && t.refresh_token.is_none()).then(|| "the sign-in expired".to_string()));
    OAuthStatus {
        signed_in: true,
        account: t.account.clone(),
        expires_at: t.expires_at,
        signed_in_at: Some(t.signed_in_at),
        problem,
    }
}

/// The access token to give agents, refreshed first when it is due. Err says why there is none (never signed in,
/// revoked, expired).
pub(crate) async fn access_token(st: &AppState, conn: &Connection) -> Result<String, String> {
    let again = format!("sign in to {} again in Settings → MCP servers", conn.name);
    let t = tokens(st, conn)
        .await?
        .ok_or_else(|| format!("{} isn't signed in: sign in in Settings → MCP servers", conn.name))?;
    if let Some(why) = &t.failed {
        return Err(format!("{}'s sign-in no longer works ({why}): {again}", conn.name));
    }
    let now = now_ms();
    if !t.due(now) {
        return Ok(t.access_token);
    }
    match refresh(st, conn).await {
        Ok(t) => Ok(t.access_token),
        // A refresh that failed for now (the service down) leaves a token that still works in use.
        Err(_) if t.usable(now) => Ok(t.access_token),
        Err(e) => Err(format!("{}: {e}: {again}", conn.name)),
    }
}

/// Refreshes a connection's tokens now. A refusal (revoked, expired) marks the sign-in failed; a failure to reach the
/// service doesn't.
pub(crate) async fn refresh(st: &AppState, conn: &Connection) -> Result<OAuthTokens, String> {
    let gate =
        st.mcp.oauth.refreshing.lock().unwrap_or_else(|e| e.into_inner()).entry(conn.id.clone()).or_default().clone();
    let _one = gate.lock().await;
    let t = tokens(st, conn).await?.ok_or("not signed in")?;
    if !t.due(now_ms()) && t.failed.is_none() {
        // Another caller refreshed it meanwhile.
        return Ok(t);
    }
    let Some(refresh_token) = t.refresh_token.clone() else {
        return Err("the sign-in expired and can't be refreshed".into());
    };
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token),
        ("client_id", t.client_id.clone()),
        ("resource", conn.url.trim().to_string()),
    ];
    if let Some(s) = &t.client_secret {
        form.push(("client_secret", s.clone()));
    }
    match token_request(&t.token_endpoint, &form).await {
        Ok(r) => {
            let n = t.updated(&r, now_ms());
            save(st, conn, &n).await?;
            Ok(n)
        }
        Err(TokenError::Refused(why)) => {
            let n = OAuthTokens { failed: Some(why.clone()), ..t };
            save(st, conn, &n).await?;
            Err(why)
        }
        Err(TokenError::Unreachable(why)) => Err(why),
    }
}

enum TokenError {
    /// The service said no (`invalid_grant`, a 4xx).
    Refused(String),
    /// The service couldn't be reached, or answered nonsense.
    Unreachable(String),
}

async fn token_request(endpoint: &str, form: &[(&str, String)]) -> Result<TokenResponse, TokenError> {
    if !ms::endpoint_ok(endpoint) {
        return Err(TokenError::Refused(format!("the token address ({endpoint}) isn't https: sign in again")));
    }
    let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let r = crate::workflow::nodes::http::client()
        .post(endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(ms::form_encode(&pairs))
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| TokenError::Unreachable(crate::workflow::nodes::http::send_error(e)))?;
    let status = r.status();
    let body: Value = r.json().await.unwrap_or(Value::Null);
    if status.is_client_error() {
        let why = body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("refused").to_string();
        return Err(TokenError::Refused(format!("the service refused the sign-in: {why}")));
    }
    if !status.is_success() {
        return Err(TokenError::Unreachable(format!("the token endpoint answered {status}")));
    }
    serde_json::from_value(body)
        .map_err(|_| TokenError::Unreachable("the token endpoint's answer has no access token".into()))
}

async fn get_json(url: &str) -> Option<Value> {
    let r = crate::workflow::nodes::http::client()
        .get(url)
        .header("Accept", "application/json")
        .timeout(TIMEOUT)
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.json().await.ok()
}

/// The server's authorization server and the scopes it asks for: from its protected resource metadata (named by its
/// 401's `WWW-Authenticate`, else at the well-known places), else (older servers) the server's own origin.
async fn discover(server_url: &str) -> Result<(AuthServer, Vec<String>), String> {
    let probe = crate::workflow::nodes::http::client()
        .post(server_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("{server_url} could not be reached: {}", crate::workflow::nodes::http::send_error(e)))?;
    let named =
        probe.headers().get("www-authenticate").and_then(|h| h.to_str().ok()).and_then(ms::resource_metadata_from);
    let mut prm = None;
    for url in named.into_iter().chain(ms::resource_metadata_urls(server_url)) {
        if let Some(v) = get_json(&url).await {
            prm = Some(v);
            break;
        }
    }
    let scopes: Vec<String> = prm
        .as_ref()
        .and_then(|p| p["scopes_supported"].as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let issuer = prm
        .as_ref()
        .and_then(|p| p["authorization_servers"][0].as_str().map(String::from))
        .or_else(|| server_url.split_once("://").map(|(s, r)| format!("{s}://{}", r.split('/').next().unwrap_or(r))))
        .ok_or_else(|| format!("{server_url} is not an address"))?;
    if !ms::endpoint_ok(&issuer) {
        return Err(format!("{server_url} signs in at {issuer}, which isn't https (or http on this computer)"));
    }
    for url in ms::auth_metadata_urls(&issuer) {
        if let Some(v) = get_json(&url).await
            && let Ok(a) = serde_json::from_value::<AuthServer>(v)
        {
            // The browser opens its sign-in address in PDT's own tab, and PDT sends its token address a client
            // secret: neither may be a `javascript:` address or plain http over the network.
            a.check(&issuer)?;
            return Ok((a, scopes));
        }
    }
    Err(format!("{server_url} doesn't say how to sign in (no OAuth metadata at {issuer}); it may not use OAuth"))
}

/// Registers PDT as a client of the authorization server (RFC 7591): a public client with PKCE.
async fn register(a: &AuthServer, redirect_uri: &str) -> Result<(String, Option<String>), String> {
    let endpoint = a.registration_endpoint.as_deref().ok_or(
        "the service doesn't let PDT register itself: register an app with it and enter its client id in the server's \
         OAuth settings",
    )?;
    let body = json!({
        "client_name": pdt_core::brand::PRODUCT_NAME,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let r =
        crate::workflow::nodes::http::client().post(endpoint).json(&body).timeout(TIMEOUT).send().await.map_err(
            |e| format!("registering with the service failed: {}", crate::workflow::nodes::http::send_error(e)),
        )?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    let id = v["client_id"].as_str().filter(|_| status.is_success()).ok_or_else(|| {
        let why = v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("no client id");
        format!("the service refused PDT's registration ({status}): {why}")
    })?;
    Ok((id.to_string(), v["client_secret"].as_str().map(String::from)))
}

fn pkce() -> (String, String) {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let verifier = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SignInRequest {
    /// The address the user reaches PDT at (`window.location.origin`): the browser comes back there.
    #[serde(default)]
    #[ts(optional)]
    pub origin: Option<String>,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SignInStart {
    /// Where the browser goes to sign in.
    pub url: String,
}

fn origin_of(given: Option<&str>, port: u16) -> Result<String, ApiError> {
    let fallback = format!("http://127.0.0.1:{port}");
    let o = given.map(|o| o.trim().trim_end_matches('/')).filter(|o| !o.is_empty()).unwrap_or(&fallback);
    let ok =
        (o.starts_with("http://") || o.starts_with("https://")) && !o[o.find("://").unwrap_or(0) + 3..].contains('/');
    if !ok {
        return Err(ApiError::invalid("origin must be an http(s) address with no path"));
    }
    Ok(o.to_string())
}

/// `POST /api/mcp-servers/{id}/sign-in`: begins a sign-in, giving the address the browser opens.
pub(crate) async fn sign_in(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<SignInRequest>>,
) -> Result<Json<SignInStart>, ApiError> {
    let conn = st.store.get_connection(&id).await?.or_404("MCP server")?;
    let settings = ms::oauth_of(&conn.config)
        .ok_or_else(|| ApiError::invalid(format!("{} doesn't sign in with OAuth", conn.name)))?;
    if !matches!(crate::mcp_client::transport(&conn), "http" | "sse") {
        return Err(ApiError::invalid("only a remote server signs in"));
    }
    let origin = origin_of(body.as_ref().and_then(|b| b.origin.as_deref()), st.listen_port)?;
    let redirect_uri = format!("{origin}{CALLBACK}");
    let server_url = conn.url.trim().to_string();
    let failed = |m: String| ApiError::upstream(codes::SIGN_IN_FAILED, m);
    let (auth, scopes) = discover(&server_url).await.map_err(failed)?;
    let (client_id, client_secret) = match settings.client_id.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => {
            let secret = match settings.client_secret.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(name) => {
                    let usage = crate::vault::Usage {
                        by: "PDT".into(),
                        how: format!("MCP server {} sign-in", conn.name),
                        session_id: None,
                        run_id: None,
                    };
                    let v = crate::vault::values_for_user(&st, &[name.to_string()], &usage)
                        .await
                        .map_err(ApiError::invalid)?;
                    v.get(name).cloned()
                }
                None => None,
            };
            (c.to_string(), secret)
        }
        None => register(&auth, &redirect_uri).await.map_err(failed)?,
    };
    let (verifier, challenge) = pkce();
    let state = uuid::Uuid::new_v4().simple().to_string();
    let scope = settings
        .scopes
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| (!scopes.is_empty()).then(|| scopes.join(" ")));
    let url = ms::authorize_url(&ms::Authorize {
        endpoint: &auth.authorization_endpoint,
        client_id: &client_id,
        redirect_uri: &redirect_uri,
        state: &state,
        challenge: &challenge,
        scope: scope.as_deref(),
        resource: &server_url,
    });
    let now = now_ms();
    let mut pending = st.mcp.oauth.pending.lock().unwrap_or_else(|e| e.into_inner());
    pending.retain(|_, p| now - p.at < PENDING_MS);
    pending.insert(
        state,
        Pending {
            connection_id: conn.id.clone(),
            verifier,
            client_id,
            client_secret,
            token_endpoint: auth.token_endpoint.clone(),
            redirect_uri,
            resource: server_url,
            at: now,
        },
    );
    Ok(Json(SignInStart { url }))
}

/// `DELETE /api/mcp-servers/{id}/sign-in`: signs out (PDT forgets the tokens).
pub(crate) async fn sign_out(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    st.store.get_connection(&id).await?.or_404("MCP server")?;
    forget(&st, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub(crate) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn page(ok: bool, text: &str) -> Response {
    let esc = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let title = if ok { "Signed in" } else { "Sign-in failed" };
    let status = if ok { StatusCode::OK } else { StatusCode::BAD_REQUEST };
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title>\
         <body style=\"font-family:system-ui;max-width:32rem;margin:4rem auto;line-height:1.5\">\
         <h1 style=\"font-size:1.25rem\">{title}</h1><p>{esc}</p></body>"
    );
    (status, Html(body)).into_response()
}

/// Who signed in, when the token response or its id token says.
fn account_of(raw: &Value) -> Option<String> {
    use base64::Engine;
    let from = |v: &Value| {
        ["email", "name", "preferred_username", "user", "login"]
            .iter()
            .find_map(|k| v[*k].as_str().filter(|s| !s.is_empty()).map(String::from))
    };
    if let Some(a) = from(raw).or_else(|| raw["team"]["name"].as_str().map(String::from)) {
        return Some(a);
    }
    let claims = raw["id_token"].as_str()?.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(claims.trim_end_matches('=')).ok()?;
    from(&serde_json::from_slice(&bytes).ok()?)
}

/// `GET /api/mcp-oauth/callback`: the browser back from the service; PDT trades the code for tokens.
pub(crate) async fn callback(State(st): State<AppState>, Query(q): Query<CallbackQuery>) -> Response {
    if let Some(e) = q.error {
        let why = q.error_description.unwrap_or(e);
        return page(false, &format!("The service didn't sign PDT in: {why}. Try again from Settings → MCP servers."));
    }
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return page(false, "This page needs the service's code and state. Sign in from Settings → MCP servers.");
    };
    let Some(p) = st
        .mcp
        .oauth
        .pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&state)
        .filter(|p| now_ms() - p.at < PENDING_MS)
    else {
        return page(false, "This sign-in is unknown or too old. Sign in again from Settings → MCP servers.");
    };
    let Ok(Some(conn)) = st.store.get_connection(&p.connection_id).await else {
        return page(false, "The MCP server is gone.");
    };
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", p.redirect_uri.clone()),
        ("client_id", p.client_id.clone()),
        ("code_verifier", p.verifier.clone()),
        ("resource", p.resource.clone()),
    ];
    if let Some(s) = &p.client_secret {
        form.push(("client_secret", s.clone()));
    }
    // The raw answer too, for who signed in.
    if !ms::endpoint_ok(&p.token_endpoint) {
        return page(false, "The service's token address isn't https. Sign in again from Settings → MCP servers.");
    }
    let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let r = crate::workflow::nodes::http::client()
        .post(&p.token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(ms::form_encode(&pairs))
        .timeout(TIMEOUT)
        .send()
        .await;
    let raw: Value = match r {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
        Ok(r) => {
            let status = r.status();
            let v: Value = r.json().await.unwrap_or(Value::Null);
            let why = v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("no reason given");
            return page(false, &format!("The service refused the sign-in ({status}): {why}."));
        }
        Err(e) => {
            return page(
                false,
                &format!("The service could not be reached: {}.", crate::workflow::nodes::http::send_error(e)),
            );
        }
    };
    let Ok(resp) = serde_json::from_value::<TokenResponse>(raw.clone()) else {
        return page(false, "The service's answer had no access token.");
    };
    let now = now_ms();
    let t = OAuthTokens {
        account: account_of(&raw),
        client_id: p.client_id,
        client_secret: p.client_secret,
        token_endpoint: p.token_endpoint,
        signed_in_at: now,
        ..OAuthTokens::default()
    }
    .updated(&resp, now);
    if let Err(e) = save(&st, &conn, &t).await {
        return page(false, &format!("PDT could not keep the sign-in: {e}."));
    }
    crate::mcp_client::connection_changed(&st, Some(&conn)).await;
    let who = t.account.map(|a| format!(" as {a}")).unwrap_or_default();
    page(true, &format!("PDT is signed in to {}{who}. You can close this tab.", conn.name))
}

/// Background upkeep (every minute): refreshes sign-ins about to expire, so new sessions start with a working token
/// and a revoked one shows in the Inbox.
pub(crate) async fn upkeep(st: &AppState) {
    for c in crate::agent_mcp::servers(st).await.into_iter().filter(|c| ms::oauth_of(&c.config).is_some()) {
        match tokens(st, &c).await {
            Ok(Some(t)) if t.failed.is_none() && t.due(now_ms()) && t.refresh_token.is_some() => {
                if let Err(e) = refresh(st, &c).await {
                    tracing::info!(server = %c.name, error = %e, "could not refresh an MCP server's sign-in");
                }
            }
            Ok(_) => {}
            Err(e) => tracing::debug!(server = %c.name, error = %e, "could not read an MCP server's sign-in"),
        }
    }
}

/// "Sign in to <server> again" while a server's sign-in was revoked or expired. Not stored: listed while it lasts,
/// from the sign-ins PDT has read (it reads them as it starts, in [`upkeep`]).
pub(crate) async fn inbox_items(st: &AppState) -> Vec<InboxItem> {
    let now = now_ms();
    let known = st.mcp.oauth.tokens.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut out = vec![];
    for c in crate::agent_mcp::servers(st).await.into_iter().filter(|c| ms::oauth_of(&c.config).is_some()) {
        let Some(Some(t)) = known.get(&c.id) else { continue };
        let Some(problem) = status_of(Some(t), now).problem else { continue };
        out.push(InboxItem::new(
            format!("mcp-sign-in:{}", c.id),
            InboxKind::McpSignIn,
            InboxGroup::Notice,
            format!("Sign in to {} again", c.name),
            json!({"connectionId": c.id, "server": c.name, "text": format!("PDT's sign-in to the MCP server {} \
no longer works ({problem}), so new agent sessions start without it. Sign in again in Settings › MCP servers.", c.name)}),
            t.expires_at.unwrap_or(t.signed_in_at),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenges_are_the_verifiers_sha256() {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let (v, c) = pkce();
        assert_eq!(v.len(), 64);
        assert_eq!(c, base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes())));
        assert_ne!(pkce().0, v);
    }

    #[test]
    fn who_signed_in_comes_from_the_answer_or_its_id_token() {
        use base64::Engine;
        assert_eq!(account_of(&json!({"team": {"name": "Acme"}})), Some("Acme".into()));
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"email":"me@x.example"}"#);
        assert_eq!(account_of(&json!({"id_token": format!("h.{claims}.s")})), Some("me@x.example".into()));
        assert_eq!(account_of(&json!({})), None);
    }

    #[test]
    fn the_redirect_goes_back_to_the_address_pdt_was_reached_at() {
        assert_eq!(origin_of(None, 7420).unwrap(), "http://127.0.0.1:7420");
        assert_eq!(origin_of(Some("https://mac.tail.ts.net/"), 7420).unwrap(), "https://mac.tail.ts.net");
        assert!(origin_of(Some("https://x/evil"), 7420).is_err());
        assert!(origin_of(Some("javascript:x"), 7420).is_err());
    }

    #[test]
    fn a_status_names_its_problem() {
        assert!(!status_of(None, 0).signed_in);
        let t = OAuthTokens { access_token: "a".into(), expires_at: Some(10), ..Default::default() };
        assert_eq!(status_of(Some(&t), 20).problem.as_deref(), Some("the sign-in expired"));
        let r = OAuthTokens { refresh_token: Some("r".into()), ..t.clone() };
        assert!(status_of(Some(&r), 20).problem.is_none(), "it refreshes");
        let f = OAuthTokens { failed: Some("revoked".into()), ..r };
        assert_eq!(status_of(Some(&f), 0).problem.as_deref(), Some("revoked"));
    }
}
