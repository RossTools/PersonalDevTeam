mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::call;
use pdt_api::{AllowedHosts, AppState, apply_remote_access, router, with_host_guard};
use pdt_core::host::HostSettings;
use pdt_tailscale::testing::RecordingTailscale;
use pdt_tailscale::{TsError, TsStatus};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

fn ts(https: bool) -> TsStatus {
    TsStatus { dns_name: "box.t.ts.net".into(), self_login: Some("me@example.com".into()), https_enabled: https }
}

async fn setup(tmp: &std::path::Path, fake: Arc<RecordingTailscale>) -> (axum::Router, AppState) {
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.listen_port = 7999;
    state.allowed_hosts = AllowedHosts::new(vec!["127.0.0.1:7999".into()]);
    state.tailscale = fake;
    let app = with_host_guard(router(state.clone(), None), state.allowed_hosts.clone());
    (app, state)
}

/// `call` sends no Host header; this one sends `Host: box.t.ts.net` like `tailscale serve` does.
async fn get_as_tailnet_host(app: &axum::Router) -> StatusCode {
    let req = Request::builder().uri("/api/health").header("host", "box.t.ts.net").body(Body::empty()).unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

#[tokio::test]
async fn enabling_needs_an_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, _) = setup(tmp.path(), fake.clone()).await;
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("owner_required")), "{v}");
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn enabling_serves_443_and_allows_the_tailnet_name() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, state) = setup(tmp.path(), fake.clone()).await;
    assert_eq!(get_as_tailnet_host(&app).await, StatusCode::FORBIDDEN);
    let (s, v) =
        call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": true, "ownerLogin": "me@example.com"})))
            .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["remote"]["active"], true);
    assert_eq!(v["remote"]["url"], "https://box.t.ts.net/");
    assert_eq!(v["remote"]["tailscaleLogin"], "me@example.com");
    assert_eq!(fake.calls(), ["serve --bg --https=443 http://127.0.0.1:7999"]);
    assert!(state.allowed_hosts.contains("box.t.ts.net"));
    assert_eq!(get_as_tailnet_host(&app).await, StatusCode::OK);

    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["remote"]["active"], false);
    assert_eq!(fake.calls().last().unwrap(), "serve --https=443 off");
    assert_eq!(get_as_tailnet_host(&app).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn tailscale_problems_are_reported_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(false))));
    let (app, state) = setup(tmp.path(), fake.clone()).await;
    let body = json!({"remoteAccess": true, "ownerLogin": "me@example.com"});
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["remote"]["enabled"].clone(), v["remote"]["active"].clone()), (json!(true), json!(false)));
    assert!(v["remote"]["error"].as_str().unwrap().contains("HTTPS"), "{v}");
    assert!(!state.allowed_hosts.contains("box.t.ts.net"));

    fake.set_status(Err(TsError::NotInstalled));
    let (_, v) = call(&app, "POST", "/api/host/remote/refresh", None).await;
    assert!(v["error"].as_str().unwrap().contains("not found"), "{v}");
    fake.set_status(Ok(ts(true)));
    let (_, v) = call(&app, "POST", "/api/host/remote/refresh", None).await;
    assert_eq!(v["active"], true, "retry succeeds once Tailscale is fixed");
    let (_, v) = call(&app, "GET", "/api/host/remote", None).await;
    assert_eq!(v["active"], true);
}

// Ruling F1: the machine's Tailscale also serves the stable instance on 443. A pdtd whose setting is off must not
// touch it at all, and turning the setting off undoes only what this process did.

#[tokio::test]
async fn applying_at_startup_with_remote_off_runs_no_tailscale_command() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, state) = setup(tmp.path(), fake.clone()).await;
    let r = apply_remote_access(&state).await;
    assert_eq!((r.enabled, r.active, r.error), (false, false, None));
    assert_eq!((fake.calls(), fake.status_calls()), (Vec::<String>::new(), 0));
    let (_, v) = call(&app, "POST", "/api/host/remote/refresh", None).await;
    assert_eq!(v["enabled"], false, "{v}");
    let (s, _) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((fake.calls(), fake.status_calls()), (Vec::<String>::new(), 0), "refresh and reads stay silent too");
}

#[tokio::test]
async fn turning_off_unserves_only_what_this_process_served() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, state) = setup(tmp.path(), fake.clone()).await;
    // The stored setting is on (left by an earlier run), but this process never applied it.
    let on = HostSettings { remote_access: true, owner_login: Some("me@example.com".into()), ..Default::default() };
    state.store.put_host_settings(&on).await.unwrap();
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["remote"]["enabled"], false);
    assert!(fake.calls().is_empty(), "nothing served here, so nothing to remove: {:?}", fake.calls());

    // Served by this process: turning off removes exactly that mapping, once.
    let (s, _) = call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": true}))).await;
    assert_eq!(s, StatusCode::OK);
    call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    call(&app, "POST", "/api/host/remote/refresh", None).await;
    assert_eq!(fake.calls(), ["serve --bg --https=443 http://127.0.0.1:7999", "serve --https=443 off"]);
}

#[tokio::test]
async fn app_state_defaults_to_no_tailscale() {
    let tmp = tempfile::tempdir().unwrap();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.path().join("data"));
    assert!(matches!(state.tailscale.status(), Err(TsError::NotInstalled)));
}

#[tokio::test]
async fn turning_off_keeps_a_configured_tailnet_host() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, state) = setup(tmp.path(), fake.clone()).await;
    state.allowed_hosts.add("box.t.ts.net"); // e.g. from PDT_ALLOWED_HOSTS
    let body = json!({"remoteAccess": true, "ownerLogin": "me@example.com"});
    assert_eq!(call(&app, "PUT", "/api/host/settings", Some(body)).await.0, StatusCode::OK);
    call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert!(state.allowed_hosts.contains("box.t.ts.net"), "{:?}", state.allowed_hosts.snapshot());
}
