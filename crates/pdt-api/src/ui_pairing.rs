//! Paired browsers (PDT-401): the UI token (`untrusted.rs`) goes only to a browser the user paired, which carries an
//! `HttpOnly; SameSite=Strict` cookie holding a secret of its own. A browser is paired by opening a pairing link: a
//! one-time code, good for a few minutes. pdtd opens one in the system's browser itself (`POST /api/ui/open`, which
//! the tray's Open and the web app's "Pair this browser" call), so the code goes from pdtd to the browser and never
//! to the caller; or a paired browser makes one to open elsewhere (a phone). No link is ever written to the log.
//! Every header a caller sends is its own to choose (`Sec-Fetch-Site`, Tailscale's identity headers), so none of them
//! pairs a browser.

use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use ts_rs::TS;

/// How long a pairing code is good for.
const CODE_TTL: Duration = Duration::from_secs(5 * 60);
/// The least time between two browsers pdtd opens, so a caller can't flood the screen with tabs.
const OPEN_GAP: Duration = Duration::from_secs(3);
/// The most browsers kept paired; pairing one more forgets the longest-paired.
const MAX_PAIRED: usize = 50;
/// The host value that lists the paired browsers: the SHA-256 of each one's secret, never the secret.
const PAIRED_KEY: &str = "ui.paired_browsers";
/// How long a browser stays paired without opening PDT (the cookie's age; browsers cap it near 400 days).
const COOKIE_MAX_AGE: u64 = 400 * 24 * 60 * 60;

/// Opens a URL in the system's browser. `AppState::new` has one that opens nothing; only pdtd's wiring installs the
/// real one, so tests never open a browser.
pub type BrowserOpener = std::sync::Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// The opener of a pdtd that opens no browser.
pub fn no_browser() -> BrowserOpener {
    std::sync::Arc::new(|_| Err("this PDT opens no browser".into()))
}

/// The opener of the system's browser, or of `program` (`PDT_BROWSER`: scratch daemons and tests stand one in), which
/// is run with the URL as its one argument.
pub fn system_browser(program: Option<String>) -> BrowserOpener {
    std::sync::Arc::new(move |url: &str| {
        let mut cmd = match &program {
            Some(p) => std::process::Command::new(p),
            None if cfg!(target_os = "macos") => std::process::Command::new("open"),
            None if cfg!(windows) => {
                let mut c = std::process::Command::new("rundll32");
                c.arg("url.dll,FileProtocolHandler");
                c
            }
            None => std::process::Command::new("xdg-open"),
        };
        cmd.arg(url).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        let status = cmd.status().map_err(|e| e.to_string())?;
        if status.success() { Ok(()) } else { Err(format!("the browser opener exited with {status}")) }
    })
}

/// The pairing codes waiting to be used, and when pdtd last opened a browser.
#[derive(Default)]
pub(crate) struct Pairing {
    codes: std::sync::Mutex<HashMap<String, Instant>>,
    last_open: std::sync::Mutex<Option<Instant>>,
    /// Serializes changes to the paired list.
    write: tokio::sync::Mutex<()>,
}

impl Pairing {
    /// A new one-time code.
    fn mint(&self) -> String {
        let code = secret();
        let mut c = self.codes.lock().unwrap_or_else(|e| e.into_inner());
        c.retain(|_, at| at.elapsed() < CODE_TTL);
        c.insert(code.clone(), Instant::now());
        code
    }

    /// Uses up a code: whether it was one and still good.
    fn redeem(&self, code: &str) -> bool {
        let mut c = self.codes.lock().unwrap_or_else(|e| e.into_inner());
        c.remove(code).is_some_and(|at| at.elapsed() < CODE_TTL)
    }

    /// Whether pdtd may open a browser now (it notes that it does).
    fn may_open(&self) -> bool {
        let mut last = self.last_open.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|at| at.elapsed() < OPEN_GAP) {
            return false;
        }
        *last = Some(Instant::now());
        true
    }
}

/// 256 random bits, hex.
fn secret() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn digest(secret: &str) -> String {
    hex::encode(sha2::Sha256::digest(secret.as_bytes()))
}

/// The cookie's name: per port, since browsers keep cookies per host, not per port, and the stable and a dev
/// instance share `127.0.0.1`.
fn cookie_name(st: &AppState) -> String {
    format!("{}_ui_{}", pdt_core::brand::SLUG, port(st))
}

fn port(st: &AppState) -> u16 {
    if st.listen_port != 0 { st.listen_port } else { st.instance.port }
}

/// The value of a cookie the request carries.
fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.trim())
}

#[derive(Default, Serialize, Deserialize)]
struct PairedBrowser {
    hash: String,
    #[serde(rename = "pairedAt")]
    paired_at: i64,
}

async fn paired_list(st: &AppState) -> Result<Vec<PairedBrowser>, ApiError> {
    let v = st.store.get_host_value(PAIRED_KEY).await?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
}

/// Whether the request comes from a browser the user paired: it carries this instance's pairing cookie, and the
/// secret in it is one PDT handed out.
pub(crate) async fn is_paired(st: &AppState, headers: &HeaderMap) -> Result<bool, ApiError> {
    let Some(secret) = cookie(headers, &cookie_name(st)).filter(|s| !s.is_empty()) else { return Ok(false) };
    let hash = digest(secret);
    Ok(paired_list(st).await?.iter().any(|b| b.hash == hash))
}

/// Pairs one more browser: its new secret, kept only as a hash.
async fn pair(st: &AppState) -> Result<String, ApiError> {
    let _w = st.pairing.write.lock().await;
    let secret = secret();
    let mut list = paired_list(st).await?;
    list.push(PairedBrowser { hash: digest(&secret), paired_at: crate::routes::now_ms() });
    let over = list.len().saturating_sub(MAX_PAIRED);
    list.drain(..over);
    st.store.put_host_value(PAIRED_KEY, &serde_json::to_value(&list).unwrap_or_default()).await?;
    Ok(secret)
}

/// The path of a pairing link with a new code (relative: the browser that opens it adds its own origin).
fn link_path(code: &str) -> String {
    format!("/api/ui/pair?code={code}")
}

/// Whether this browser is paired, and how many are.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PairingStatus {
    pub paired: bool,
    pub paired_browsers: u32,
}

/// `GET /api/ui/pairing`.
async fn status(State(st): State<AppState>, headers: HeaderMap) -> Result<Json<PairingStatus>, ApiError> {
    let paired = is_paired(&st, &headers).await?;
    let n = paired_list(&st).await?.len();
    Ok(Json(PairingStatus { paired, paired_browsers: n as u32 }))
}

/// `POST /api/ui/open`: opens PDT in the system's browser with a pairing link. The caller learns nothing of the code;
/// the browser that opens is paired. Anyone may ask (the tray, an unpaired page): the most it does is open a tab.
async fn open(State(st): State<AppState>) -> Result<StatusCode, ApiError> {
    if !st.pairing.may_open() {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_soon",
            "PDT just opened a browser; wait a moment",
        ));
    }
    let url = format!("http://127.0.0.1:{}{}", port(&st), link_path(&st.pairing.mint()));
    let opener = st.browser.clone();
    match tokio::task::spawn_blocking(move || opener(&url)).await {
        Ok(Ok(())) => Ok(StatusCode::NO_CONTENT),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "could not open the browser");
            Err(ApiError::new(StatusCode::CONFLICT, "no_browser", format!("PDT could not open a browser: {e}")))
        }
        Err(e) => Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "no_browser", e.to_string())),
    }
}

#[derive(Deserialize)]
struct PairQuery {
    #[serde(default)]
    code: String,
}

/// `GET /api/ui/pair?code=…`: a pairing link. A good code pairs the browser (sets its cookie); either way it goes on
/// to the app, which says when the link was used up or too old.
async fn pair_link(State(st): State<AppState>, Query(q): Query<PairQuery>) -> Result<Response, ApiError> {
    if !st.pairing.redeem(&q.code) {
        return Ok(redirect("/?pairing=expired"));
    }
    let secret = pair(&st).await?;
    tracing::info!("paired a browser");
    let mut res = redirect("/");
    let cookie = format!("{}={secret}; Path=/; Max-Age={COOKIE_MAX_AGE}; HttpOnly; SameSite=Strict", cookie_name(&st));
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    Ok(res)
}

fn redirect(to: &'static str) -> Response {
    let mut res = StatusCode::SEE_OTHER.into_response();
    res.headers_mut().insert(header::LOCATION, HeaderValue::from_static(to));
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

/// A pairing link for another browser.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PairLink {
    /// The link's path; the page adds its own origin.
    pub path: String,
    #[ts(type = "number")]
    pub expires_at: i64,
}

/// `POST /api/ui/pair-links`: a paired browser's link for another one (the user's click: it needs the UI token).
async fn new_link(State(st): State<AppState>, headers: HeaderMap) -> Result<Json<PairLink>, ApiError> {
    user_only(&st, &headers)?;
    let path = link_path(&st.pairing.mint());
    Ok(Json(PairLink { path, expires_at: crate::routes::now_ms() + CODE_TTL.as_millis() as i64 }))
}

/// `DELETE /api/ui/paired`: unpairs every browser, this one too (the user's click: it needs the UI token). The UI token
/// changes as well, so pages that already hold it lose it.
async fn unpair_all(State(st): State<AppState>, headers: HeaderMap) -> Result<StatusCode, ApiError> {
    user_only(&st, &headers)?;
    let _w = st.pairing.write.lock().await;
    st.store.delete_host_value(PAIRED_KEY).await?;
    st.ui_token.rotate();
    tracing::info!("unpaired every browser");
    Ok(StatusCode::NO_CONTENT)
}

fn user_only(st: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if crate::untrusted::from_ui(st, headers)? {
        return Ok(());
    }
    Err(ApiError::new(StatusCode::FORBIDDEN, "not_the_ui", "only the user can do this, from a paired browser"))
}

/// Tests: a browser paired the way a pairing link pairs it, as its `Cookie` header.
#[doc(hidden)]
pub async fn paired_cookie(st: &AppState) -> String {
    let secret = pair(st).await.expect("pair a browser");
    format!("{}={secret}", cookie_name(st))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{delete, get, post};
    axum::Router::new()
        .route("/ui/pairing", get(status))
        .route("/ui/open", post(open))
        .route("/ui/pair", get(pair_link))
        .route("/ui/pair-links", post(new_link))
        .route("/ui/paired", delete(unpair_all))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_good_once() {
        let p = Pairing::default();
        let code = p.mint();
        assert!(!p.redeem("not-a-code"));
        assert!(p.redeem(&code));
        assert!(!p.redeem(&code), "used up");
    }

    #[test]
    fn a_stale_code_is_refused() {
        let p = Pairing::default();
        let code = p.mint();
        p.codes.lock().unwrap().insert(code.clone(), Instant::now() - CODE_TTL - Duration::from_secs(1));
        assert!(!p.redeem(&code));
    }

    #[test]
    fn browsers_open_no_faster_than_the_gap() {
        let p = Pairing::default();
        assert!(p.may_open());
        assert!(!p.may_open());
        *p.last_open.lock().unwrap() = Some(Instant::now() - OPEN_GAP);
        assert!(p.may_open());
    }

    #[test]
    fn cookies_are_read_by_name() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; pdt_ui_7420=abc ; pdt_ui_7421=def"));
        assert_eq!(cookie(&h, "pdt_ui_7420"), Some("abc"));
        assert_eq!(cookie(&h, "pdt_ui_7421"), Some("def"));
        assert_eq!(cookie(&h, "pdt_ui_1"), None);
    }
}
