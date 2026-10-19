mod common;

use axum::http::StatusCode;
use common::call;
use pdt_core::activity::Activity;
use pdt_core::handoff::{downloads_dir, request_path};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use serde_json::{Value, json};
use sha2::Digest;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

const WIN: &[u8] = b"win";
const MAC: &[u8] = b"mac";
/// The token the fake GitHub accepts. Distinctive, so a leak is easy to spot in any output.
const TOKEN: &str = "ghp_test_token_value";
const ASSET: &str = if cfg!(windows) { "P_0.1.7_x64-setup.exe" } else { "P.app.tar.gz" };

#[derive(Clone)]
struct Fake {
    /// Requests for the latest release.
    hits: Arc<AtomicUsize>,
    sums: Arc<std::sync::Mutex<String>>,
    /// While set, the latest-release answer (or the installer download) waits.
    hold_latest: Arc<AtomicBool>,
    hold_assets: Arc<AtomicBool>,
}

async fn wait_while(flag: &AtomicBool) {
    while flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn good_sums() -> String {
    let sha = |b: &[u8]| hex::encode(sha2::Sha256::digest(b));
    format!("{}  P_0.1.7_x64-setup.exe\n{}  P.app.tar.gz\n", sha(WIN), sha(MAC))
}

async fn fake_github() -> (String, Fake) {
    use axum::extract::{Path, State};
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    let fake = Fake {
        hits: Default::default(),
        sums: Arc::new(std::sync::Mutex::new(good_sums())),
        hold_latest: Default::default(),
        hold_assets: Default::default(),
    };
    let ok = |h: &HeaderMap| {
        h.get("authorization").and_then(|v| v.to_str().ok()) == Some(format!("Bearer {TOKEN}").as_str())
    };
    let app = axum::Router::new()
        .route(
            "/repos/o/r/releases/latest",
            axum::routing::get(move |State(f): State<Fake>, h: HeaderMap| async move {
                f.hits.fetch_add(1, Ordering::SeqCst);
                wait_while(&f.hold_latest).await;
                if !ok(&h) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                axum::Json(json!({"tag_name": "v0.1.7", "body": "notes", "published_at": null, "assets": [
                    {"id": 1, "name": "P_0.1.7_x64-setup.exe", "size": 3},
                    {"id": 2, "name": "P.app.tar.gz", "size": 3},
                    {"id": 3, "name": "SHA256SUMS", "size": 1}]}))
                .into_response()
            }),
        )
        .route(
            "/repos/o/r/releases/assets/{id}",
            axum::routing::get(|State(f): State<Fake>, Path(id): Path<u64>| async move {
                if id != 3 {
                    wait_while(&f.hold_assets).await;
                }
                match id {
                    1 => WIN.to_vec().into_response(),
                    2 => MAC.to_vec().into_response(),
                    _ => f.sums.lock().unwrap().clone().into_response(),
                }
            }),
        )
        .with_state(fake.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, fake)
}

async fn setup_with(tmp: &std::path::Path, supervised: bool, current: &str) -> (axum::Router, pdt_api::AppState, Fake) {
    let (base, fake) = fake_github().await;
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = pdt_api::AppState::new(store, tmp.join("data"));
    let mut svc = pdt_api::UpdateService::new(&base, "o/r", supervised, current);
    svc.first_check_delay = Duration::ZERO;
    svc.check_interval = Duration::from_millis(50);
    state.update = Arc::new(svc);
    (pdt_api::router(state.clone(), None), state, fake)
}

async fn setup(tmp: &std::path::Path, supervised: bool) -> (axum::Router, pdt_api::AppState, Fake) {
    setup_with(tmp, supervised, "0.1.0-local").await
}

fn store_token(state: &pdt_api::AppState) {
    state.secrets.set(&state.update_token_account, TOKEN).unwrap();
}

fn data(tmp: &std::path::Path) -> std::path::PathBuf {
    tmp.join("data")
}

/// Adds a running Claude session that is `working`, which makes the host busy.
async fn add_busy_session(app: &axum::Router, state: &pdt_api::AppState, tmp: &std::path::Path) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q"]).arg(&repo).status().unwrap().success());
    let (_, p) =
        call(app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    state
        .store
        .insert_session(&Session {
            id: "busy".into(),
            project_id: p["id"].as_str().unwrap().into(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Claude),
            bypass_permissions: false,
            title: "feature".into(),
            cwd: repo.to_string_lossy().into(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at: 1,
            ended_at: None,
            external_session_id: None,
            fresh_started_at: None,
            activity: Some(Activity::Working),
            attention_at: None,
            last_viewed_at: None,
            unseen: false,
            attempt_id: None,
            ticket: None,
        })
        .await
        .unwrap();
}

/// Polls GET /api/host/update until `pred` holds (5 s at most) and returns that status.
async fn wait_status(app: &axum::Router, pred: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..500 {
        let (_, v) = call(app, "GET", "/api/host/update", None).await;
        if pred(&v) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (_, v) = call(app, "GET", "/api/host/update", None).await;
    panic!("status never matched: {v}");
}

fn state_is(s: &'static str) -> impl Fn(&Value) -> bool {
    move |v| v["state"] == s
}

#[tokio::test]
async fn unsupervised_pdtd_never_updates() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), false).await;
    store_token(&state);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["state"], "disabled");
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("updates_disabled")));
    assert_eq!(fake.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn no_token_means_no_network_traffic() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, fake) = setup(tmp.path(), true).await;
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["state"].as_str(), v["token"]["state"].as_str()), (Some("idle"), Some("missing")));
    assert_eq!(fake.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn busy_hosts_ask_first_then_the_release_is_handed_off() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    let (s, _) = call(&app, "PUT", "/api/host/token", Some(json!({"token": TOKEN}))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["state"], "available", "{v}");
    assert_eq!(v["latest"]["version"], "0.1.7");
    assert_eq!(v["latest"]["notes"], "notes");

    add_busy_session(&app, &state, tmp.path()).await;
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": false}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("host_busy")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("Claude · feature"));
    assert!(!request_path(&data(tmp.path())).exists());

    // An older download is pruned once the new one is handed off.
    let old = downloads_dir(&data(tmp.path())).join("0.1.6");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("old-setup.exe"), b"old").unwrap();

    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "installing");
    let req: pdt_core::handoff::UpdateRequest =
        serde_json::from_slice(&std::fs::read(request_path(&data(tmp.path()))).unwrap()).unwrap();
    assert_eq!((req.version.as_str(), req.from_version.as_str()), ("0.1.7", "0.1.0-local"));
    assert_eq!(std::fs::read(&req.asset).unwrap(), if cfg!(windows) { WIN } else { MAC });
    assert_eq!(req.asset, downloads_dir(&data(tmp.path())).join("0.1.7").join(ASSET), "the folder is the version");
    assert!(!old.exists(), "older downloads are pruned");
    tokio::time::timeout(Duration::from_secs(1), state.shutdown.notified()).await.expect("pdtd asked to stop");
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["state"], "installing", "a check does not undo the hand-off");
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("update_in_progress")), "{v}");
}

#[tokio::test]
async fn bad_checksum_hands_nothing_off() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    *fake.sums.lock().unwrap() =
        format!("{}  P_0.1.7_x64-setup.exe\n{}  P.app.tar.gz\n", "0".repeat(64), "0".repeat(64));
    call(&app, "POST", "/api/host/update/check", None).await;
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("update_failed")), "{v}");
    assert!(!v.to_string().contains(TOKEN), "{v}");
    assert!(!request_path(&data(tmp.path())).exists());
    let (_, v) = call(&app, "GET", "/api/host/update", None).await;
    assert_eq!(v["state"], "available", "can be retried");
    assert!(!v.to_string().contains(TOKEN), "{v}");
}

#[tokio::test]
async fn tokens_are_checked_stored_and_never_echoed() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    let acct = state.update_token_account.clone();
    let (s, v) = call(&app, "PUT", "/api/host/token", Some(json!({"token": "expired"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_token")));
    assert_eq!(state.secrets.get(&acct).unwrap(), None);
    let (s, v) = call(&app, "PUT", "/api/host/token", Some(json!({"token": format!(" {TOKEN} ")}))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!v.to_string().contains(TOKEN), "{v}");
    assert_eq!(state.secrets.get(&acct).unwrap().as_deref(), Some(TOKEN));
    assert_eq!(call(&app, "DELETE", "/api/host/token", None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(state.secrets.get(&acct).unwrap(), None);
}

#[tokio::test]
async fn token_state_follows_the_keychain() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), false).await;
    // Stored before this pdtd started: reported without a check (and without any network traffic).
    store_token(&state);
    let (_, v) = call(&app, "GET", "/api/host/update", None).await;
    assert_eq!(v["token"], json!({"state": "valid", "expiresAt": null}), "{v}");
    let (_, h) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(h["tokenPresent"], true);
    assert_eq!(call(&app, "DELETE", "/api/host/token", None).await.0, StatusCode::NO_CONTENT);
    let (_, v) = call(&app, "GET", "/api/host/update", None).await;
    assert_eq!(v["token"]["state"], "missing");
    let (s, v) = call(&app, "PUT", "/api/host/token", Some(json!({"token": TOKEN}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["token"]["state"], "valid", "known as soon as it is stored: {v}");
    assert_eq!(fake.hits.load(Ordering::SeqCst), 1, "only the validation of the new token");
}

/// A keychain waiting on an approval prompt (one only the person at the Mac can answer).
struct StalledKeychain;
impl pdt_api::secrets::SecretStore for StalledKeychain {
    fn get(&self, _: &str) -> Result<Option<String>, pdt_api::secrets::SecretError> {
        std::thread::sleep(Duration::from_secs(30));
        Ok(None)
    }
    fn set(&self, _: &str, _: &str) -> Result<(), pdt_api::secrets::SecretError> {
        self.get("").map(|_| ())
    }
    fn delete(&self, _: &str) -> Result<(), pdt_api::secrets::SecretError> {
        self.get("").map(|_| ())
    }
}

#[tokio::test]
async fn a_stalled_keychain_is_reported_and_never_hangs_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, mut state, fake) = setup(tmp.path(), true).await;
    state.secrets = Arc::new(pdt_api::secrets::TimeLimited::new(Arc::new(StalledKeychain), Duration::from_millis(200)));
    let app = pdt_api::router(state.clone(), None);
    let started = std::time::Instant::now();
    pdt_api::update::sync_token_state(&state).await;
    let (s, v) = call(&app, "GET", "/api/host/update", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["error"].as_str().unwrap_or("").ends_with(pdt_api::secrets::KEYCHAIN_STALLED), "{v}");
    let (s, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["error"].as_str().unwrap_or("").ends_with(pdt_api::secrets::KEYCHAIN_STALLED), "{v}");
    let (s, e) = call(&app, "PUT", "/api/host/token", Some(json!({"token": TOKEN}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("keychain_unavailable")), "{e}");
    // Host settings still load; only the token part reports the keychain.
    let (s, v) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["tokenPresent"], false, "{v}");
    assert!(v["tokenError"].as_str().unwrap_or("").ends_with(pdt_api::secrets::KEYCHAIN_STALLED), "{v}");
    assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
    assert_eq!(fake.hits.load(Ordering::SeqCst), 1, "only the new token's validation reached GitHub");
}

/// A keychain that fails while `failing` is set (e.g. a prompt nobody has answered yet), then works.
struct FlakyKeychain {
    failing: Arc<AtomicBool>,
    inner: pdt_api::secrets::MemorySecretStore,
}
impl pdt_api::secrets::SecretStore for FlakyKeychain {
    fn get(&self, name: &str) -> Result<Option<String>, pdt_api::secrets::SecretError> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(pdt_api::secrets::SecretError(pdt_api::secrets::KEYCHAIN_STALLED.into()));
        }
        self.inner.get(name)
    }
    fn set(&self, name: &str, value: &str) -> Result<(), pdt_api::secrets::SecretError> {
        self.inner.set(name, value)
    }
    fn delete(&self, name: &str) -> Result<(), pdt_api::secrets::SecretError> {
        self.inner.delete(name)
    }
}

#[tokio::test]
async fn a_keychain_error_clears_once_the_keychain_answers() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, mut state, _) = setup(tmp.path(), true).await;
    let failing = Arc::new(AtomicBool::new(true));
    state.secrets = Arc::new(FlakyKeychain { failing: failing.clone(), inner: Default::default() });
    let app = pdt_api::router(state.clone(), None);
    let (_, v) = call(&app, "GET", "/api/host/update", None).await;
    assert!(v["error"].as_str().unwrap_or("").starts_with(pdt_api::secrets::KEYCHAIN_FAILED), "{v}");
    failing.store(false, Ordering::SeqCst);
    let (_, v) = call(&app, "GET", "/api/host/update", None).await;
    assert_eq!(v["error"], Value::Null, "{v}");
    assert_eq!(v["token"]["state"], "missing");
}

#[tokio::test]
async fn each_data_dir_has_its_own_token() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("a")).unwrap();
    let (app_a, state_a, _) = setup(&tmp.path().join("a"), true).await;
    let store_b = pdt_store::Store::open(&tmp.path().join("b.db")).await.unwrap();
    let mut state_b = pdt_api::AppState::new(store_b, tmp.path().join("b").join("data"));
    state_b.secrets = state_a.secrets.clone();
    let app_b = pdt_api::router(state_b.clone(), None);
    assert_ne!(state_a.update_token_account, state_b.update_token_account);
    store_token(&state_a);
    assert_eq!(call(&app_a, "GET", "/api/host/settings", None).await.1["tokenPresent"], true);
    assert_eq!(call(&app_b, "GET", "/api/host/settings", None).await.1["tokenPresent"], false);
    assert_eq!(call(&app_b, "DELETE", "/api/host/token", None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(state_a.secrets.get(&state_a.update_token_account).unwrap().as_deref(), Some(TOKEN));
}

#[tokio::test]
async fn an_unparseable_current_version_is_never_offered_an_update() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup_with(tmp.path(), true, "garbage").await;
    store_token(&state);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["state"].as_str(), &v["latest"]), (Some("idle"), &Value::Null), "{v}");
    assert!(v["error"].as_str().unwrap().contains("garbage"), "{v}");
}

#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn a_dropped_install_request_still_finishes() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    call(&app, "POST", "/api/host/update/check", None).await;

    // A failing download whose request was dropped: the state recovers and checks work again.
    *fake.sums.lock().unwrap() = format!("{}  {ASSET}\n", "0".repeat(64));
    fake.hold_assets.store(true, Ordering::SeqCst);
    let install = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true})));
    assert!(tokio::time::timeout(Duration::from_millis(200), install).await.is_err(), "the download is held");
    assert_eq!(call(&app, "GET", "/api/host/update", None).await.1["state"], "downloading");
    fake.hold_assets.store(false, Ordering::SeqCst);
    let v = wait_status(&app, state_is("available")).await;
    assert!(v["error"].as_str().unwrap().contains("checksum"), "{v}");
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["state"].as_str(), &v["error"]), (Some("available"), &Value::Null), "{v}");

    // A good download whose request was dropped is still handed off.
    *fake.sums.lock().unwrap() = good_sums();
    fake.hold_assets.store(true, Ordering::SeqCst);
    let install = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true})));
    assert!(tokio::time::timeout(Duration::from_millis(200), install).await.is_err());
    fake.hold_assets.store(false, Ordering::SeqCst);
    wait_status(&app, state_is("installing")).await;
    assert!(request_path(&data(tmp.path())).exists());
}

fn write_result(tmp: &std::path::Path, outcome: pdt_core::handoff::UpdateOutcome, at: i64) {
    let r = pdt_core::handoff::UpdateResult {
        outcome,
        from_version: "0.1.0-local".into(),
        to_version: "0.1.7".into(),
        message: "Update to v0.1.7 was not installed: the running version did not stop".into(),
        log: None,
        at,
    };
    let p = pdt_core::handoff::result_path(&data(tmp));
    std::fs::write(p, serde_json::to_vec(&r).unwrap()).unwrap();
}

/// Final review #10: pdt-host took the request and recorded a failure while this pdtd kept running (it did not
/// stop): the next status read leaves `installing` and shows the result.
#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn a_failed_hand_off_while_pdtd_keeps_running_does_not_stay_installing() {
    use pdt_core::handoff::UpdateOutcome;
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    store_token(&state);
    call(&app, "POST", "/api/host/update/check", None).await;
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!((s, v["state"].as_str()), (StatusCode::OK, Some("installing")), "{v}");
    let requested_at: i64 = serde_json::from_slice::<Value>(&std::fs::read(request_path(&data(tmp.path()))).unwrap())
        .unwrap()["requestedAt"]
        .as_i64()
        .unwrap();

    // A result from before this hand-off, with the request still waiting: still installing.
    write_result(tmp.path(), UpdateOutcome::Failed, requested_at - 1);
    assert_eq!(call(&app, "GET", "/api/host/update", None).await.1["state"], "installing");
    // pdt-host took the request but has not decided yet: still installing.
    std::fs::remove_file(request_path(&data(tmp.path()))).unwrap();
    assert_eq!(call(&app, "GET", "/api/host/update", None).await.1["state"], "installing");
    // pdt-host recorded the failure.
    write_result(tmp.path(), UpdateOutcome::Failed, requested_at + 1);
    let (_, v) = call(&app, "GET", "/api/host/update", None).await;
    assert_eq!(v["state"], "available", "{v}");
    assert_eq!(v["lastResult"]["outcome"], "failed", "{v}");
    assert!(v["lastResult"]["message"].as_str().unwrap().contains("did not stop"), "{v}");
}

#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn a_host_that_got_busy_during_the_download_is_asked_again() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    call(&app, "POST", "/api/host/update/check", None).await;
    fake.hold_assets.store(true, Ordering::SeqCst);
    let install = tokio::spawn({
        let app = app.clone();
        async move { call(&app, "POST", "/api/host/update/install", Some(json!({"force": false}))).await }
    });
    wait_status(&app, state_is("downloading")).await;
    add_busy_session(&app, &state, tmp.path()).await;
    fake.hold_assets.store(false, Ordering::SeqCst);
    let (s, v) = install.await.unwrap();
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("host_busy")), "{v}");
    assert!(!request_path(&data(tmp.path())).exists());
    let kept = downloads_dir(&data(tmp.path())).join("0.1.7").join(ASSET);
    assert!(kept.exists(), "the verified download is kept for the next attempt");
    assert_eq!(call(&app, "GET", "/api/host/update", None).await.1["state"], "available");
}

#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn checks_never_overwrite_an_install_and_never_stick() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    call(&app, "POST", "/api/host/update/check", None).await;

    // A dropped check request does not leave `checking` behind.
    fake.hold_latest.store(true, Ordering::SeqCst);
    let check = call(&app, "POST", "/api/host/update/check", None);
    assert!(tokio::time::timeout(Duration::from_millis(200), check).await.is_err());
    fake.hold_latest.store(false, Ordering::SeqCst);
    wait_status(&app, state_is("available")).await;

    // A check that started before an install finishes during the download: the download stays.
    fake.hold_latest.store(true, Ordering::SeqCst);
    let check = tokio::spawn({
        let app = app.clone();
        async move { call(&app, "POST", "/api/host/update/check", None).await }
    });
    wait_status(&app, state_is("checking")).await;
    fake.hold_assets.store(true, Ordering::SeqCst);
    let install = tokio::spawn({
        let app = app.clone();
        async move { call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await }
    });
    wait_status(&app, state_is("downloading")).await;
    fake.hold_latest.store(false, Ordering::SeqCst);
    check.await.unwrap();
    assert_eq!(call(&app, "GET", "/api/host/update", None).await.1["state"], "downloading");
    fake.hold_assets.store(false, Ordering::SeqCst);
    assert_eq!(install.await.unwrap().0, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/api/host/update", None).await.1["state"], "installing");
}

async fn set_policy(app: &axum::Router, policy: &str) {
    let (s, v) = call(app, "PUT", "/api/host/settings", Some(json!({"updatePolicy": policy}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn the_loop_never_checks_under_the_manual_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    set_policy(&app, "manual").await;
    let task = tokio::spawn(pdt_api::run_update_loop(state.clone()));
    tokio::time::sleep(Duration::from_millis(300)).await;
    task.abort();
    assert_eq!(fake.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_loop_only_notifies_under_the_notify_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    set_policy(&app, "notify").await;
    let task = tokio::spawn(pdt_api::run_update_loop(state.clone()));
    wait_status(&app, state_is("available")).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    task.abort();
    assert!(fake.hits.load(Ordering::SeqCst) >= 2, "it checks again every interval");
    assert!(!request_path(&data(tmp.path())).exists());
}

#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn the_loop_installs_when_idle_under_auto_when_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    store_token(&state);
    set_policy(&app, "auto_when_idle").await;
    let task = tokio::spawn(pdt_api::run_update_loop(state.clone()));
    tokio::time::timeout(Duration::from_secs(5), state.shutdown.notified()).await.expect("pdtd asked to stop");
    task.abort();
    assert!(request_path(&data(tmp.path())).exists());
    assert_eq!(call(&app, "GET", "/api/host/update", None).await.1["state"], "installing");
}

#[tokio::test]
async fn the_loop_waits_for_a_busy_host_under_auto_when_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    set_policy(&app, "auto_when_idle").await;
    add_busy_session(&app, &state, tmp.path()).await;
    let task = tokio::spawn(pdt_api::run_update_loop(state.clone()));
    wait_status(&app, state_is("available")).await;
    while fake.hits.load(Ordering::SeqCst) < 3 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    task.abort();
    assert!(!request_path(&data(tmp.path())).exists());
}

fn write_skip(tmp: &std::path::Path, version: &str) {
    let skip = pdt_core::handoff::UpdateSkip { version: version.into(), reason: "rolled back".into(), at: 1 };
    let p = pdt_core::handoff::skip_path(&data(tmp));
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, serde_json::to_vec(&skip).unwrap()).unwrap();
}

/// Ruling B: a version that failed or was rolled back (pdt-host's skip.json) is never installed automatically again,
/// even after the result is dismissed; a manual update retries it and clears the skip.
#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn the_loop_never_auto_installs_a_version_that_failed_even_after_a_dismiss() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    set_policy(&app, "auto_when_idle").await;
    write_skip(tmp.path(), "0.1.7");
    let (code, _) = call(&app, "POST", "/api/host/update/dismiss", None).await;
    assert_eq!(code, StatusCode::NO_CONTENT);
    let task = tokio::spawn(pdt_api::run_update_loop(state.clone()));
    wait_status(&app, state_is("available")).await;
    while fake.hits.load(Ordering::SeqCst) < 3 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    task.abort();
    assert!(!request_path(&data(tmp.path())).exists(), "not installed automatically");
    assert!(pdt_core::handoff::skip_path(&data(tmp.path())).exists(), "dismiss keeps the skip");
    let (code, v) = call(&app, "POST", "/api/host/update/install", Some(json!({}))).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert!(request_path(&data(tmp.path())).exists(), "a manual update retries it");
    assert!(!pdt_core::handoff::skip_path(&data(tmp.path())).exists(), "and clears the skip");
}

#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn the_loop_auto_installs_a_newer_version_than_the_one_that_failed() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    store_token(&state);
    set_policy(&app, "auto_when_idle").await;
    write_skip(tmp.path(), "0.1.6");
    let task = tokio::spawn(pdt_api::run_update_loop(state.clone()));
    tokio::time::timeout(Duration::from_secs(5), state.shutdown.notified()).await.expect("pdtd asked to stop");
    task.abort();
    assert!(request_path(&data(tmp.path())).exists());
}
