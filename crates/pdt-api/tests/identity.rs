use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::router;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common;

async fn req(app: &axum::Router, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, Value) {
    let mut r = Request::builder().uri(uri);
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let res = app.clone().oneshot(r.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn put(app: &axum::Router, body: Value) {
    let (status, _) = common::call(app, "PUT", "/api/host/settings", Some(body)).await;
    assert_eq!(status, StatusCode::OK);
}

async fn app(tmp: &std::path::Path, env_owner: Option<&str>, web_dir: Option<std::path::PathBuf>) -> axum::Router {
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp);
    state.owner_from_env = env_owner.map(String::from);
    state.refresh_owner().await;
    router(state, web_dir)
}

const ME: (&str, &str) = ("tailscale-user-login", "me@example.com");
const FWD: (&str, &str) = ("x-forwarded-for", "100.64.0.7");

#[tokio::test]
async fn local_requests_pass_and_whoami_says_local() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), None, None).await;
    let (s, v) = req(&app, "/api/host/whoami", &[]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, json!({"caller": "local", "login": null}));
}

#[tokio::test]
async fn only_the_owner_gets_in_remotely() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), None, None).await;
    let (s, v) = req(&app, "/api/health", &[ME, FWD]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden_identity")), "no owner yet");
    put(&app, json!({"ownerLogin": "Me@Example.com"})).await;
    assert_eq!(req(&app, "/api/health", &[ME, FWD]).await.0, StatusCode::OK);
    let (_, v) = req(&app, "/api/host/whoami", &[ME, FWD]).await;
    assert_eq!(v, json!({"caller": "tailnet", "login": "me@example.com"}));
    let (s, v) = req(&app, "/api/health", &[("tailscale-user-login", "eve@example.com"), FWD]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden_identity")));
    assert!(v["message"].as_str().unwrap().contains("eve@example.com"));
}

#[tokio::test]
async fn forwarded_without_identity_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), Some("me@example.com"), None).await;
    for headers in
        [vec![FWD], vec![("x-forwarded-host", "box.t.ts.net")], vec![ME, FWD, ("tailscale-funnel-request", "?1")]]
    {
        let (s, v) = req(&app, "/api/health", &headers).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden_identity")), "{headers:?}");
    }
}

#[tokio::test]
async fn env_owner_overrides_the_setting_and_guards_the_ui() {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<html>ui</html>").unwrap();
    let app = app(tmp.path(), Some("me@example.com"), Some(web)).await;
    put(&app, json!({"ownerLogin": "someone-else@example.com"})).await;
    assert_eq!(req(&app, "/api/health", &[ME, FWD]).await.0, StatusCode::OK, "env wins");
    let (s, _) = req(&app, "/", &[("tailscale-user-login", "eve@example.com"), FWD]).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the static UI is guarded too");
    assert_eq!(req(&app, "/", &[]).await.0, StatusCode::OK);
}
