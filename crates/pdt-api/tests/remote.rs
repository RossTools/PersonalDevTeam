use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::call;
use pdt_api::{AllowedHosts, AppState, apply_remote_access, router, with_host_guard};
use pdt_core::host::HostSettings;
use pdt_tailscale::testing::RecordingTailscale;
use pdt_tailscale::{TsError, TsStatus};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

fn ts(https: bool) -> TsStatus {
    TsStatus { dns_name: "box.t.ts.net".into(), self_login: Some("me@example.com".into()), https_enabled: https }
}

async fn setup(tmp: &std::path::Path, fake: Arc<RecordingTailscale>) -> (axum::Router, AppState) {
    setup_with(tmp, fake, |_| {}).await
}

/// Like `setup`, with a chance to adjust the state before the router takes its copy.
async fn setup_with(
    tmp: &std::path::Path,
    fake: Arc<RecordingTailscale>,
    adjust: impl FnOnce(&mut AppState),
) -> (axum::Router, AppState) {
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp);
    state.listen_port = 7999;
    state.allowed_hosts = AllowedHosts::new(vec!["127.0.0.1:7999".into()]);
    state.tailscale = fake;
    adjust(&mut state);
    let app = with_host_guard(router(state.clone(), None), state.allowed_hosts.clone());
    (app, state)
}

/// `call` sends no Host header; this one sends `Host: box.t.ts.net` like `tailscale serve` does.
async fn get_as_tailnet_host(app: &axum::Router) -> StatusCode {
    get_as(app, "box.t.ts.net").await
}

async fn get_as(app: &axum::Router, host: &str) -> StatusCode {
    let req = Request::builder().uri("/api/health").header("host", host).body(Body::empty()).unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

const ON: &str = r#"{"remoteAccess": true, "ownerLogin": "me@example.com"}"#;

fn on() -> serde_json::Value {
    serde_json::from_str(ON).unwrap()
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
    let state = common::app_state(store, tmp.path());
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

#[tokio::test]
async fn startup_apply_and_a_concurrent_change_end_consistent() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, state) = setup(tmp.path(), fake.clone()).await;
    let stored = HostSettings { remote_access: true, owner_login: Some("me@example.com".into()), ..Default::default() };
    state.store.put_host_settings(&stored).await.unwrap();
    fake.set_delay(Duration::from_millis(300));
    // pdtd's startup apply is in the middle of a slow `tailscale status` when the owner turns remote access off.
    let st = state.clone();
    let startup = tokio::spawn(async move { apply_remote_access(&st).await });
    common::wait::wait_until("the startup apply's `tailscale status`", async || fake.status_calls() == 1).await;
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    startup.await.unwrap();

    let calls = fake.calls();
    let served = calls.iter().filter(|c| c.starts_with("serve --bg")).count();
    let removed = calls.iter().filter(|c| c.ends_with(" off")).count();
    assert_eq!(served, removed, "every mapping made was removed: {calls:?}");
    assert_eq!(fake.status_calls(), 1, "the startup apply ran first");
    assert!(!state.allowed_hosts.contains("box.t.ts.net"));
    let (_, v) = call(&app, "GET", "/api/host/remote", None).await;
    assert_eq!((v["enabled"].clone(), v["active"].clone()), (json!(false), json!(false)), "{v}");
}

#[tokio::test]
async fn the_stable_instance_claims_443_and_others_their_own_port() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, state) = setup_with(tmp.path(), fake.clone(), |s| s.remote_https_port = 7999).await;
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(on())).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["remote"]["url"], "https://box.t.ts.net:7999/");
    assert_eq!(fake.calls(), ["serve --bg --https=7999 http://127.0.0.1:7999"]);
    assert!(state.allowed_hosts.contains("box.t.ts.net:7999"));
    assert_eq!(get_as(&app, "box.t.ts.net:7999").await, StatusCode::OK);
    assert_eq!(get_as(&app, "box.t.ts.net").await, StatusCode::FORBIDDEN, "443 belongs to the stable instance");
    call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert_eq!(fake.calls().last().unwrap(), "serve --https=7999 off");
    assert_eq!(get_as(&app, "box.t.ts.net:7999").await, StatusCode::FORBIDDEN);

    // AppState::new defaults to the stable instance's port.
    let tmp = tempfile::tempdir().unwrap();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    assert_eq!(common::app_state(store, tmp.path()).remote_https_port, 443);
}

#[tokio::test]
async fn a_hung_tailscale_times_out_and_releases_the_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    // Hangs well past the 100 ms limit, so "returned in time" can allow for a loaded CI runner (3 s).
    fake.set_delay(Duration::from_secs(5));
    let (app, _) = setup_with(tmp.path(), fake.clone(), |s| s.tailscale_timeout = Duration::from_millis(100)).await;
    let started = Instant::now();
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(on())).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    assert_eq!(v["remote"]["active"], false);
    assert!(v["remote"]["error"].as_str().unwrap().contains("in time"), "{v}");
    // The settings lock was released: another change goes through at once.
    let started = Instant::now();
    let (s, _) = call(&app, "PUT", "/api/host/settings", Some(json!({"updatePolicy": "manual"}))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
}

#[tokio::test]
async fn a_failed_reapply_withdraws_the_tailnet_name() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, state) = setup(tmp.path(), fake.clone()).await;
    let (_, v) = call(&app, "PUT", "/api/host/settings", Some(on())).await;
    assert_eq!(v["remote"]["active"], true, "{v}");
    fake.set_status(Err(TsError::NotRunning("state Stopped".into())));
    let (_, v) = call(&app, "POST", "/api/host/remote/refresh", None).await;
    assert_eq!((v["active"].clone(), v["url"].clone()), (json!(false), json!(null)), "{v}");
    assert!(!state.allowed_hosts.contains("box.t.ts.net"), "not active, so not allowed");
    fake.set_status(Ok(ts(true)));
    let (_, v) = call(&app, "POST", "/api/host/remote/refresh", None).await;
    assert_eq!(v["active"], true, "{v}");
    assert!(state.allowed_hosts.contains("box.t.ts.net"));
    // The mapping made before the failure is still this process's to remove.
    call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert_eq!(fake.calls().last().unwrap(), "serve --https=443 off");
}

#[tokio::test]
async fn a_failed_unserve_is_reported_and_retried() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, state) = setup(tmp.path(), fake.clone()).await;
    call(&app, "PUT", "/api/host/settings", Some(on())).await;
    fake.fail_unserve(Some(TsError::NotRunning("state Stopped".into())));
    let (_, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert_eq!((v["remote"]["enabled"].clone(), v["remote"]["active"].clone()), (json!(false), json!(false)));
    assert!(v["remote"]["error"].as_str().unwrap().contains("443"), "{v}");
    assert!(!state.allowed_hosts.contains("box.t.ts.net"));

    fake.fail_unserve(None);
    let (_, v) = call(&app, "POST", "/api/host/remote/refresh", None).await;
    assert_eq!(v["error"], json!(null), "{v}");
    call(&app, "POST", "/api/host/remote/refresh", None).await;
    assert_eq!(
        fake.calls(),
        ["serve --bg --https=443 http://127.0.0.1:7999", "serve --https=443 off", "serve --https=443 off"],
        "retried once after the failure, then nothing left to remove"
    );
}

/// PDT-355: `serve --bg` mappings outlive pdtd, so the next process on the same data folder knows them as this
/// instance's: turned off after a restart in which Tailscale was down, it still removes the mapping.
#[tokio::test]
async fn turning_off_after_a_restart_unserves_what_the_previous_process_served() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, _) = setup(tmp.path(), fake.clone()).await;
    call(&app, "PUT", "/api/host/settings", Some(on())).await;
    assert_eq!(fake.calls(), ["serve --bg --https=443 http://127.0.0.1:7999"]);

    let next = Arc::new(RecordingTailscale::new(Err(TsError::NotRunning("state Stopped".into()))));
    let (app, state) = setup(tmp.path(), next.clone()).await;
    let r = apply_remote_access(&state).await;
    assert!(!r.active);
    next.set_status(Ok(ts(true)));
    let (_, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert_eq!(v["remote"]["error"], json!(null), "{v}");
    assert_eq!(next.calls(), ["serve --https=443 off"]);
}

// PDT-148: after a restart Tailscale can come up after pdtd; the watch makes remote access follow it.

fn quick_watch(state: &mut AppState) {
    state.remote_watch = pdt_api::RemoteWatch {
        retry_first: Duration::from_millis(20),
        retry_max: Duration::from_millis(80),
        check_every: Duration::from_millis(40),
    };
}

/// Waits up to 5 s for `f` to hold.
async fn eventually(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn the_watch_activates_remote_access_once_tailscale_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Err(TsError::NotRunning("state Stopped".into()))));
    let (app, state) = setup_with(tmp.path(), fake.clone(), quick_watch).await;
    let mut settings = state.store.get_host_settings().await.unwrap();
    (settings.remote_access, settings.owner_login) = (true, Some("me@example.com".into()));
    state.store.put_host_settings(&settings).await.unwrap();
    let r = apply_remote_access(&state).await;
    assert_eq!((r.enabled, r.active), (true, false), "{r:?}");
    assert_eq!(get_as_tailnet_host(&app).await, StatusCode::FORBIDDEN);

    tokio::spawn(pdt_api::run_remote_watch(state.clone()));
    eventually("a few retries", || fake.status_calls() >= 3).await;
    assert!(fake.calls().is_empty(), "nothing is served while Tailscale is stopped");
    fake.set_status(Ok(ts(true)));
    eventually("remote access to be active", || state.remote.lock().unwrap().active).await;
    assert_eq!(get_as_tailnet_host(&app).await, StatusCode::OK, "no manual refresh needed");
    assert_eq!(fake.calls(), ["serve --bg --https=443 http://127.0.0.1:7999"]);

    // Active: it only asks for the status, and serves nothing again while nothing changed.
    let asked = fake.status_calls();
    eventually("a check while active", || fake.status_calls() >= asked + 2).await;
    assert_eq!(fake.calls().len(), 1, "{:?}", fake.calls());
    let (_, v) = call(&app, "GET", "/api/health", None).await;
    let watch = v["loops"].as_array().unwrap().iter().find(|l| l["name"] == "remote").cloned();
    assert!(watch.is_some_and(|l| !l["lastPassAt"].is_null()), "{v}");
}

#[tokio::test]
async fn the_watch_notices_tailscale_stopping_and_a_new_name() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, state) = setup_with(tmp.path(), fake.clone(), quick_watch).await;
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(on())).await;
    assert_eq!((s, v["remote"]["active"].clone()), (StatusCode::OK, json!(true)), "{v}");
    tokio::spawn(pdt_api::run_remote_watch(state.clone()));

    fake.set_status(Err(TsError::NotRunning("state Stopped".into())));
    eventually("remote access to go inactive", || !state.remote.lock().unwrap().active).await;
    assert_eq!(get_as_tailnet_host(&app).await, StatusCode::FORBIDDEN, "the name is withdrawn");

    fake.set_status(Ok(TsStatus { dns_name: "box.new.ts.net".into(), ..ts(true) }));
    eventually("the new name", || state.remote.lock().unwrap().dns_name.as_deref() == Some("box.new.ts.net")).await;
    assert!(state.remote.lock().unwrap().active);
    assert_eq!(get_as(&app, "box.new.ts.net").await, StatusCode::OK);
    assert_eq!(get_as_tailnet_host(&app).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_watch_runs_no_tailscale_command_with_remote_off() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (_app, state) = setup_with(tmp.path(), fake.clone(), quick_watch).await;
    apply_remote_access(&state).await;
    tokio::spawn(pdt_api::run_remote_watch(state.clone()));
    eventually("a few passes", || {
        state.loops.health(i64::MAX).iter().any(|l| l.name == "remote" && l.last_pass_at.is_some())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!((fake.status_calls(), fake.calls().len()), (0, 0));
}
