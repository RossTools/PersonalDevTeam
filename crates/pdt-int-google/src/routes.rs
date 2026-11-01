//! The Google plugin's routes (PDT-316), under `/api/google`: its accounts and their sign-in state, signing in (the
//! browser goes to Google's consent page and comes back to [`callback`]) and out, and what Google's file picker needs
//! in the browser (an access token, the API key and the project number). Signing in and out and the picker's token
//! are for PDT's web app only (its UI token), never an agent's curl.

use crate::accounts::{self, Pending, Shared, SignIn, config_str};
use crate::oauth;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Html;
use pdt_api::sdk::{ApiError, AppState, from_ui, now_ms};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

/// A Google account (connection) as Settings shows it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GoogleAccount {
    pub connection_id: String,
    pub name: String,
    pub signed_in: bool,
    /// The account's email, once signed in.
    pub email: Option<String>,
    /// Has an OAuth client ID and secret (signing in needs both).
    pub has_client: bool,
    /// Has the API key and project number Google's file picker needs.
    pub picker: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GoogleSignInInput {
    /// The address PDT is open at in the browser (`http://127.0.0.1:7420`): Google sends the browser back there.
    pub origin: String,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GoogleSignInStart {
    /// Google's consent page, to open in a new window.
    pub url: String,
    /// The address Google sends the browser back to (it must be one of the OAuth client's redirect URIs).
    pub redirect_uri: String,
}

/// What Google's file picker needs, in the browser.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GooglePicker {
    pub access_token: String,
    pub api_key: String,
    /// The Google Cloud project's number: files picked are then shared with PDT's OAuth client.
    pub app_id: String,
    pub client_id: String,
}

fn ui_only(st: &AppState, h: &HeaderMap) -> Result<(), ApiError> {
    if from_ui(st, h)? {
        return Ok(());
    }
    Err(ApiError::new(StatusCode::FORBIDDEN, "not_the_ui", "only PDT's web app signs in to Google or opens its picker"))
}

fn bad(m: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", m)
}

async fn connection(st: &AppState, id: &str) -> Result<pdt_core::workflow::api::Connection, ApiError> {
    st.store
        .get_connection(id)
        .await?
        .filter(|c| c.kind == accounts::KIND)
        .ok_or_else(|| ApiError::not_found("Google account"))
}

/// `GET /api/google/accounts`.
pub async fn list(State(st): State<AppState>) -> Result<Json<Vec<GoogleAccount>>, ApiError> {
    let mut out = vec![];
    for c in accounts::connections(&st).await? {
        let s = accounts::sign_in(&st, &c.id).await?;
        out.push(GoogleAccount {
            signed_in: s.is_some(),
            email: s.and_then(|s| s.email),
            has_client: config_str(&c, "clientId").is_some() && c.has_secret,
            picker: config_str(&c, "apiKey").is_some() && config_str(&c, "appId").is_some(),
            connection_id: c.id,
            name: c.name,
        });
    }
    Ok(Json(out))
}

/// The address Google sends the browser back to, from where PDT is open.
pub fn redirect_uri(origin: &str) -> Result<String, String> {
    let o = origin.trim().trim_end_matches('/');
    let rest = o.strip_prefix("http://").or_else(|| o.strip_prefix("https://"));
    match rest {
        Some(host) if !host.is_empty() && !host.contains(['/', '?', '#', ' ']) => {
            Ok(format!("{o}/api/google/callback"))
        }
        _ => Err(format!("{origin} is not an address PDT is open at (http://host:port)")),
    }
}

/// `POST /api/google/accounts/{id}/sign-in`: where the browser goes to sign in.
pub async fn start(
    State(st): State<AppState>,
    shared: axum::Extension<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<GoogleSignInInput>,
) -> Result<Json<GoogleSignInStart>, ApiError> {
    ui_only(&st, &headers)?;
    let c = connection(&st, &id).await?;
    let (client_id, _) = accounts::client(&st, &c).await?;
    let redirect = redirect_uri(&input.origin).map_err(bad)?;
    let (state, verifier) = (oauth::random(24), oauth::random(48));
    let hint = accounts::sign_in(&st, &id).await?.and_then(|s| s.email);
    let url = oauth::auth_url(&client_id, &redirect, &state, &verifier, hint.as_deref());
    let now = now_ms();
    let mut p = shared.pending.lock().unwrap_or_else(|e| e.into_inner());
    p.retain(|_, x| now - x.started_ms < accounts::PENDING_MS);
    p.insert(state, Pending { connection_id: id, verifier, redirect_uri: redirect.clone(), started_ms: now });
    Ok(Json(GoogleSignInStart { url, redirect_uri: redirect }))
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

fn page(ok: bool, message: &str) -> (StatusCode, Html<String>) {
    let esc = message.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let title = if ok { "Signed in to Google" } else { "Google sign-in failed" };
    let status = if ok { StatusCode::OK } else { StatusCode::BAD_REQUEST };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title>\
         <body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:4rem auto;line-height:1.5\">\
         <h1 style=\"font-size:1.25rem\">{title}</h1><p>{esc}</p><p>You can close this window.</p>\
         <script>try{{window.opener&&window.opener.postMessage({{pdtGoogle:{ok}}},location.origin)}}catch(e){{}}\
         {close}</script></body>",
        close = if ok { "setTimeout(function(){window.close()},1500)" } else { "" }
    );
    (status, Html(html))
}

/// `GET /api/google/callback`: Google sends the browser back here with a code, which PDT trades for the account's
/// refresh token.
pub async fn callback(
    State(st): State<AppState>,
    shared: axum::Extension<Arc<Shared>>,
    Query(q): Query<CallbackQuery>,
) -> (StatusCode, Html<String>) {
    if let Some(e) = q.error {
        return page(false, &format!("Google said: {e}. Try again from Settings → Plugins → Google."));
    }
    let pending = q.state.and_then(|s| shared.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&s));
    let Some(p) = pending.filter(|p| now_ms() - p.started_ms < accounts::PENDING_MS) else {
        return page(false, "This sign-in is unknown or too old. Start it again from Settings → Plugins → Google.");
    };
    let Some(code) = q.code else { return page(false, "Google sent no code back.") };
    let c = match connection(&st, &p.connection_id).await {
        Ok(c) => c,
        Err(_) => return page(false, "The Google connection was deleted while signing in."),
    };
    let (client_id, secret) = match accounts::client(&st, &c).await {
        Ok(x) => x,
        Err(e) => return page(false, &e.to_string()),
    };
    let transport = shared.transport.clone();
    let traded = tokio::task::spawn_blocking(move || {
        let t = oauth::exchange(transport.as_ref(), &client_id, &secret, &code, &p.verifier, &p.redirect_uri)?;
        let email = oauth::email(transport.as_ref(), &t.access_token).ok();
        Ok::<_, oauth::OauthError>((t, email))
    })
    .await;
    let (token, email) = match traded {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => return page(false, &format!("PDT couldn't finish signing in: {e}")),
        Err(e) => return page(false, &e.to_string()),
    };
    // Google gives a refresh token with consent; a sign-in again without one keeps the one PDT has.
    let refresh = match token.refresh_token {
        Some(r) => r,
        None => match accounts::sign_in(&st, &c.id).await.ok().flatten() {
            Some(s) => s.refresh_token,
            None => {
                return page(
                    false,
                    "Google gave PDT no refresh token. Remove PDT's access in your Google account and sign in again.",
                );
            }
        },
    };
    if let Err(e) = accounts::store_sign_in(&st, &c.id, &SignIn { refresh_token: refresh, email: email.clone() }).await
    {
        return page(false, &format!("PDT couldn't keep the sign-in: {e}"));
    }
    shared.tokens.forget(&c.id);
    if let Some(e) = &email {
        shared.emails.write().unwrap_or_else(|e| e.into_inner()).insert(c.id.clone(), e.clone());
    }
    let who = email.unwrap_or_else(|| "your Google account".to_string());
    page(true, &format!("PDT is signed in to Google as {who} for {}.", c.name))
}

/// `DELETE /api/google/accounts/{id}/sign-in`: signs out; Google forgets PDT's access too.
pub async fn sign_out(
    State(st): State<AppState>,
    shared: axum::Extension<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    ui_only(&st, &headers)?;
    connection(&st, &id).await?;
    accounts::sign_out(&st, &shared, &id).await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/google/accounts/{id}/picker`: what Google's file picker needs in the browser.
pub async fn picker(
    State(st): State<AppState>,
    shared: axum::Extension<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<GooglePicker>, ApiError> {
    ui_only(&st, &headers)?;
    let c = connection(&st, &id).await?;
    let (Some(api_key), Some(app_id)) = (config_str(&c, "apiKey"), config_str(&c, "appId")) else {
        return Err(bad(format!(
            "{} has no API key and project number for Google's file picker: add them to the connection",
            c.name
        )));
    };
    let a = accounts::account(&st, &shared, &c).await?;
    let client_id = a.client_id.clone();
    let sh = shared.0.clone();
    let token = tokio::task::spawn_blocking(move || {
        crate::api::Session::new(sh.transport.as_ref(), &a, &sh.tokens).access_token(false)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(GooglePicker { access_token: token, api_key, app_id, client_id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_comes_back_to_where_pdt_is_open() {
        assert_eq!(redirect_uri("http://127.0.0.1:7420/").unwrap(), "http://127.0.0.1:7420/api/google/callback");
        assert_eq!(redirect_uri("https://mac.tail1.ts.net").unwrap(), "https://mac.tail1.ts.net/api/google/callback");
        assert!(redirect_uri("javascript:alert(1)").is_err());
        assert!(redirect_uri("http://x/y").is_err());
    }

    #[test]
    fn the_page_escapes_what_it_shows() {
        let (_, Html(h)) = page(false, "<b>x</b>");
        assert!(h.contains("&lt;b&gt;x") && !h.contains("<b>x"));
    }
}
