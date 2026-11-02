//! PDT-401: the UI token goes only to a browser the user paired, never to a caller that just sends the headers a
//! browser would.

use crate::common;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

struct Env {
    app: axum::Router,
    state: AppState,
    opened: Arc<Mutex<Vec<String>>>,
    _tmp: tempfile::TempDir,
}

async fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.listen_port = 7999;
    let opened = Arc::new(Mutex::new(Vec::new()));
    let o = opened.clone();
    state.browser = Arc::new(move |url: &str| {
        o.lock().unwrap().push(url.to_string());
        Ok(())
    });
    Env { app: router(state.clone(), None), state, opened, _tmp: tmp }
}

/// A raw request: status, headers and the JSON body (`Null` when none).
async fn send(env: &Env, method: &str, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, header::HeaderMap, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = env.app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let (status, h) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, h, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The token as a browser fetches it with `cookie`.
async fn token(env: &Env, cookie: &str) -> (StatusCode, Value) {
    let (s, _, v) = send(env, "GET", "/api/ui/token", &[("sec-fetch-site", "same-origin"), ("cookie", cookie)]).await;
    (s, v)
}

/// Opens a pairing link: the cookie it sets (`name=value`), if any, and where it goes on to.
async fn follow(env: &Env, path: &str) -> (Option<String>, String) {
    let (s, h, _) = send(env, "GET", path, &[]).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let cookie = h.get(header::SET_COOKIE).map(|c| c.to_str().unwrap().to_string());
    if let Some(c) = &cookie {
        for part in ["HttpOnly", "SameSite=Strict", "Path=/"] {
            assert!(c.contains(part), "{c}");
        }
    }
    let to = h.get(header::LOCATION).unwrap().to_str().unwrap().to_string();
    (cookie.map(|c| c.split(';').next().unwrap().to_string()), to)
}

fn path_of(url: &str) -> String {
    url.strip_prefix("http://127.0.0.1:7999").unwrap_or_else(|| panic!("{url}")).to_string()
}

#[tokio::test]
async fn headers_a_caller_chooses_never_get_the_token() {
    let env = env().await;
    let (s, v) = token(&env, "").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("not_paired")), "{v}");
    // Tailscale's identity headers can be sent by anyone on this machine as well.
    let forged = [
        ("sec-fetch-site", "same-origin"),
        ("tailscale-user-login", "owner@example.com"),
        ("x-forwarded-for", "100.64.0.1"),
    ];
    let (s, _, _) = send(&env, "GET", "/api/ui/token", &forged).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // A made-up cookie, or another instance's.
    assert_eq!(token(&env, "pdt_ui_7999=made-up").await.0, StatusCode::FORBIDDEN);
    let other = pdt_api::ui_pairing::paired_cookie(&env.state).await.replace("7999", "7420");
    assert_eq!(token(&env, &other).await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn pdt_opens_a_pairing_link_in_the_browser_and_the_caller_never_sees_it() {
    let env = env().await;
    let (s, _, v) = send(&env, "GET", "/api/ui/pairing", &[]).await;
    assert_eq!((s, v["paired"].as_bool(), v["pairedBrowsers"].as_u64()), (StatusCode::OK, Some(false), Some(0)));

    let (s, _, body) = send(&env, "POST", "/api/ui/open", &[]).await;
    assert_eq!((s, body), (StatusCode::NO_CONTENT, Value::Null), "nothing for the caller");
    let (s, _, v) = send(&env, "POST", "/api/ui/open", &[]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("too_soon")), "{v}");
    let url = env.opened.lock().unwrap().clone();
    assert_eq!(url.len(), 1);
    assert!(url[0].starts_with("http://127.0.0.1:7999/api/ui/pair?code="), "{url:?}");

    // The browser opens it: paired, and it gets the token.
    let (cookie, to) = follow(&env, &path_of(&url[0])).await;
    assert_eq!(to, "/");
    let cookie = cookie.expect("a pairing cookie");
    let (s, v) = token(&env, &cookie).await;
    assert!(s == StatusCode::OK && v["token"].is_string(), "{v}");
    let (_, _, v) = send(&env, "GET", "/api/ui/pairing", &[("cookie", &cookie)]).await;
    assert_eq!((v["paired"].as_bool(), v["pairedBrowsers"].as_u64()), (Some(true), Some(1)));
    // Still same-origin only, so another page on this host that shares the cookie can't fetch it.
    let (s, _, _) = send(&env, "GET", "/api/ui/token", &[("cookie", &cookie)]).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // The link is good once.
    let (again, to) = follow(&env, &path_of(&url[0])).await;
    assert_eq!((again, to.as_str()), (None, "/?pairing=expired"));
    assert_eq!(follow(&env, "/api/ui/pair?code=made-up").await.0, None);
}

#[tokio::test]
async fn a_pdt_that_opens_no_browser_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let app = router(AppState::new(store, tmp.path().join("data")), None);
    let (s, v) = common::call(&app, "POST", "/api/ui/open", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("no_browser")), "{v}");
}

#[tokio::test]
async fn a_paired_browser_pairs_another_and_can_unpair_them_all() {
    let env = env().await;
    // Only the user's click, from a paired browser.
    let (s, v) = common::call(&env.app, "POST", "/api/ui/pair-links", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("not_the_ui")), "{v}");
    let (s, v) = common::call_as_ui(&env.state, &env.app, "POST", "/api/ui/pair-links", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["expiresAt"].as_i64().unwrap() > 0);
    let (phone, _) = follow(&env, v["path"].as_str().unwrap()).await;
    let phone = phone.expect("the other browser is paired");
    assert_eq!(token(&env, &phone).await.0, StatusCode::OK);

    let (s, _) = common::call(&env.app, "DELETE", "/api/ui/paired", None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, held) = token(&env, &phone).await;
    let (s, _) = common::call_as_ui(&env.state, &env.app, "DELETE", "/api/ui/paired", None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // A page that already held the token loses it.
    let held = held["token"].as_str().unwrap();
    let (s, v) = common::call_with(&env.app, "POST", "/api/ui/pair-links", None, &[("x-ui-token", held)]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("stale_ui_token")), "{v}");
    assert_eq!(token(&env, &phone).await.0, StatusCode::FORBIDDEN, "unpaired");
}
