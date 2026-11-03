//! The Google plugin's routes (PDT-316), under `/api/google`: its accounts and their sign-in state, signing in (the
//! browser goes to Google's consent page and comes back to [`callback`]) and out, the agents' permission (PDT-360),
//! and Google's file picker in the browser ([`picker_js`]) with what it needs (an access token, the API key and the
//! project number). These are the plugin's sign-in ([`pdt_core::integration::SignIn`]) and picker routes. With
//! PDT's own clients Google comes back through the website's page ([`crate::builtin`]), and signing in with no account
//! adds one. Signing in and out, the agents' permission and the picker's token are for PDT's web app only (its UI
//! token), never an agent's curl.

use crate::accounts::{self, Pending, Shared, SignIn, config_str};
use crate::builtin::{self, Client};
use crate::oauth;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Html;
use pdt_api::sdk::{ApiError, AppState, OrNotFound, codes, from_ui, now_ms};
use pdt_core::integration::{AccountSignInInput, AccountSignInStart, SignInAccount};
use pdt_core::workflow::api::Connection;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// What Google's file picker needs, in the browser (`picker.js`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
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
    Err(ApiError::forbidden(codes::NOT_THE_UI, "only PDT's web app signs in to Google or opens its picker"))
}

async fn connection(st: &AppState, id: &str) -> Result<pdt_core::workflow::api::Connection, ApiError> {
    st.store.get_connection(id).await?.filter(|c| c.kind == accounts::KIND).or_404("Google account")
}

/// `GET /api/google/accounts`.
pub async fn list(
    State(st): State<AppState>,
    shared: axum::Extension<Arc<Shared>>,
) -> Result<Json<Vec<SignInAccount>>, ApiError> {
    let mut out = vec![];
    for c in accounts::connections(&st).await? {
        let s = accounts::sign_in(&st, &c.id).await?;
        let built_in = shared.uses_built_in(&c);
        let agents = s.is_some() && (!built_in || accounts::agent_sign_in(&st, &c.id).await?.is_some());
        let picker = match &shared.built_in {
            Some(b) if built_in => b.picker.is_some(),
            _ => config_str(&c, "apiKey").is_some() && config_str(&c, "appId").is_some(),
        };
        // An account with PDT's own client can't be given a key: this build lacks one (PDT-493).
        let note = match (picker, built_in) {
            (true, _) => None,
            (false, true) => Some("No file picker in this build of PDT"),
            (false, false) => Some("No file picker (needs an API key and project number)"),
        };
        out.push(SignInAccount {
            note: note.map(str::to_string),
            signed_in: s.is_some(),
            email: s.and_then(|s| s.email),
            has_client: built_in || (config_str(&c, "clientId").is_some() && c.has_secret),
            built_in,
            agents,
            picker,
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

/// Where the browser goes to sign `connection` in (a new account for None), or to ask its consent for agents.
fn begin(
    shared: &Shared,
    connection: Option<&Connection>,
    client: Client,
    origin: &str,
    agent: bool,
    hint: Option<&str>,
) -> Result<AccountSignInStart, ApiError> {
    let built_in = connection.is_none_or(|c| shared.uses_built_in(c));
    let nonce = oauth::random(24);
    let (state, redirect) = if built_in {
        // The browser starts at the website's page (`relay`), and Google comes back to it, which sends the browser on
        // to `origin`.
        (builtin::state(&nonce, &builtin::pdt_origin(origin).map_err(ApiError::invalid)?), builtin::redirect_uri())
    } else {
        (nonce, redirect_uri(origin).map_err(ApiError::invalid)?)
    };
    let verifier = oauth::random(48);
    let url = oauth::auth_url(&client.id, &redirect, &state, &verifier, hint);
    let now = now_ms();
    let mut p = shared.pending.lock().unwrap_or_else(|e| e.into_inner());
    p.retain(|_, x| now - x.started_ms < accounts::PENDING_MS);
    p.insert(
        state,
        Pending {
            connection_id: connection.map(|c| c.id.clone()),
            verifier,
            redirect_uri: redirect.clone(),
            client,
            agent,
            started_ms: now,
        },
    );
    let relay = built_in.then(|| redirect.clone());
    Ok(AccountSignInStart { url, redirect_uri: redirect, relay })
}

/// `POST /api/google/sign-in`: where the browser goes to add an account, with PDT's own clients.
pub async fn start_new(
    State(st): State<AppState>,
    shared: axum::Extension<Arc<Shared>>,
    headers: HeaderMap,
    Json(input): Json<AccountSignInInput>,
) -> Result<Json<AccountSignInStart>, ApiError> {
    ui_only(&st, &headers)?;
    let Some(b) = &shared.built_in else {
        return Err(ApiError::invalid(
            "this PDT was built without its own Google sign-in: add an account with your own OAuth client",
        ));
    };
    Ok(Json(begin(&shared, None, b.standard.clone(), &input.origin, false, None)?))
}

/// `POST /api/google/accounts/{id}/sign-in`: where the browser goes to sign in, or to let agents use the account.
pub async fn start(
    State(st): State<AppState>,
    shared: axum::Extension<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<AccountSignInInput>,
) -> Result<Json<AccountSignInStart>, ApiError> {
    ui_only(&st, &headers)?;
    let c = connection(&st, &id).await?;
    let agent = input.agent == Some(true);
    let signed = accounts::sign_in(&st, &id).await?;
    if agent {
        if !shared.uses_built_in(&c) {
            return Err(ApiError::invalid(format!("{} has its own OAuth client: agents use its sign-in", c.name)));
        }
        if signed.is_none() {
            return Err(ApiError::invalid(format!("sign {} in first, then allow agents", c.name)));
        }
    }
    let client = accounts::client(&st, &shared, &c, agent).await?;
    let hint = signed.and_then(|s| s.email);
    Ok(Json(begin(&shared, Some(&c), client, &input.origin, agent, hint.as_deref())?))
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
         <script>try{{window.opener&&window.opener.postMessage({{pdtSignIn:{ok}}},location.origin)}}catch(e){{}}\
         {close}</script></body>",
        close = if ok { "setTimeout(function(){window.close()},1500)" } else { "" }
    );
    (status, Html(html))
}

/// The connection a new account's sign-in goes to: the one with PDT's own clients already signed in as `email`, else
/// a new one named after it.
async fn added(st: &AppState, shared: &Shared, email: Option<&str>) -> Result<Connection, String> {
    let email = email.ok_or("Google didn't say which account signed in, so PDT can't add it. Try again.")?;
    let all = accounts::connections(st).await.map_err(|e| e.to_string())?;
    for c in &all {
        let mine = accounts::sign_in(st, &c.id).await.map_err(|e| e.to_string())?.and_then(|s| s.email);
        if shared.uses_built_in(c) && mine.is_some_and(|m| m.eq_ignore_ascii_case(email)) {
            return Ok(c.clone());
        }
    }
    let taken = st.store.list_connections().await.map_err(|e| e.to_string())?;
    let name = (1..)
        .map(|n| if n == 1 { email.to_string() } else { format!("{email} ({n})") })
        .find(|n| !taken.iter().any(|c| c.name.eq_ignore_ascii_case(n)))
        .unwrap_or_default();
    let input = pdt_api::sdk::connections::ConnectionInput {
        name: Some(name),
        kind: Some(accounts::KIND.into()),
        url: None,
        config: Some(serde_json::json!({})),
    };
    let made = pdt_api::sdk::connections::create_connection(State(st.clone()), Ok(Json(input))).await;
    made.map(|(_, Json(c))| c).map_err(|e| e.message().to_string())
}

/// `GET /api/google/callback`: Google sends the browser back here (through the website's page, with PDT's own
/// clients) with a code, which PDT trades for the account's refresh token, or the agents' for their permission. Only
/// a `state` PDT issued, still under way, is taken, and the code is traded with its PKCE verifier.
pub async fn callback(
    State(st): State<AppState>,
    shared: axum::Extension<Arc<Shared>>,
    Query(q): Query<CallbackQuery>,
) -> (StatusCode, Html<String>) {
    let pending = q.state.and_then(|s| shared.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&s));
    if let Some(e) = q.error {
        return page(false, &format!("Google said: {e}. Try again from Settings → Plugins → Google."));
    }
    let Some(p) = pending.filter(|p| now_ms() - p.started_ms < accounts::PENDING_MS) else {
        return page(false, "This sign-in is unknown or too old. Start it again from Settings → Plugins → Google.");
    };
    let Some(code) = q.code else { return page(false, "Google sent no code back.") };
    let existing = match &p.connection_id {
        Some(id) => match connection(&st, id).await {
            Ok(c) => Some(c),
            Err(_) => return page(false, "The Google connection was deleted while signing in."),
        },
        None => None,
    };
    let (transport, client, verifier, redirect) =
        (shared.transport.clone(), p.client.clone(), p.verifier.clone(), p.redirect_uri.clone());
    let traded = tokio::task::spawn_blocking(move || {
        let t = oauth::exchange(transport.as_ref(), &client.id, &client.secret, &code, &verifier, &redirect)?;
        let email = oauth::email(transport.as_ref(), &t.access_token).ok();
        Ok::<_, oauth::OauthError>((t, email))
    })
    .await;
    let (token, email) = match traded {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => return page(false, &format!("PDT couldn't finish signing in: {e}")),
        Err(e) => return page(false, &e.to_string()),
    };
    let c = match existing {
        Some(c) => c,
        None => match added(&st, &shared, email.as_deref()).await {
            Ok(c) => c,
            Err(e) => return page(false, &format!("PDT couldn't add the Google account: {e}")),
        },
    };
    let who = email.clone().unwrap_or_else(|| "your Google account".to_string());
    let had = if p.agent {
        // Agents act as the account the user signed in, never another one.
        let mine = accounts::sign_in(&st, &c.id).await.ok().flatten().and_then(|s| s.email);
        if let Some(m) = mine.filter(|m| !email.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(m))) {
            return page(
                false,
                &format!("You allowed agents as {who}, but {} is {m}. Try again and pick {m}.", c.name),
            );
        }
        accounts::agent_sign_in(&st, &c.id).await
    } else {
        accounts::sign_in(&st, &c.id).await
    };
    // Google gives a refresh token with consent; a sign-in again without one keeps the one PDT has.
    let refresh = match token.refresh_token.or_else(|| had.ok().flatten().map(|s| s.refresh_token)) {
        Some(r) => r,
        None => {
            return page(
                false,
                "Google gave PDT no refresh token. Remove PDT's access in your Google account and sign in again.",
            );
        }
    };
    let key = if p.agent { accounts::agent_key(&c.id) } else { accounts::sign_in_key(&c.id) };
    if let Err(e) = accounts::store_sign_in(&st, key, &SignIn { refresh_token: refresh, email: email.clone() }).await {
        return page(false, &format!("PDT couldn't keep the sign-in: {e}"));
    }
    shared.tokens.forget(&crate::api::token_key(&c.id, p.agent));
    if p.agent {
        return page(true, &format!("Agents may now use {who}'s Google docs through PDT ({}).", c.name));
    }
    if let Some(e) = &email {
        shared.emails.write().unwrap_or_else(|e| e.into_inner()).insert(c.id.clone(), e.clone());
    }
    page(true, &format!("PDT is signed in to Google as {who} for {}.", c.name))
}

/// `DELETE /api/google/accounts/{id}/agents`: agents may no longer use the account; PDT forgets their sign-in.
pub async fn forget_agents(
    State(st): State<AppState>,
    shared: axum::Extension<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    ui_only(&st, &headers)?;
    connection(&st, &id).await?;
    accounts::forget_agents(&st, &shared, &id).await.map_err(ApiError::from_text)?;
    Ok(StatusCode::NO_CONTENT)
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
    accounts::sign_out(&st, &shared, &id).await.map_err(ApiError::from_text)?;
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
    let own = (config_str(&c, "apiKey"), config_str(&c, "appId"));
    let picker = match &shared.built_in {
        Some(b) if shared.uses_built_in(&c) => b.picker.clone(),
        _ => own.0.zip(own.1),
    };
    let Some((api_key, app_id)) = picker else {
        return Err(ApiError::invalid(format!(
            "{} has no API key and project number for Google's file picker: add them to the connection",
            c.name
        )));
    };
    let a = accounts::account(&st, &shared, &c, false).await?;
    let client_id = a.client_id.clone();
    let sh = shared.0.clone();
    let token = tokio::task::spawn_blocking(move || {
        crate::api::Session::new(sh.transport.as_ref(), &a, &sh.tokens).access_token(false)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(GooglePicker { access_token: token, api_key, app_id, client_id }))
}

/// Google's file picker in the browser ([`pdt_api::sdk::documents::ProviderPicker`]).
const PICKER_JS: &str = include_str!("picker.js");

/// `GET /api/google/picker.js`: the module the import dialog runs to open Google's picker as an account.
pub async fn picker_js() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "text/javascript; charset=utf-8")], PICKER_JS)
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
