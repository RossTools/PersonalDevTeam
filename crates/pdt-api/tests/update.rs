use crate::common;

use axum::http::StatusCode;
use common::call;
use common::wait::{still_after, wait_until};
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
    /// Requests for a comparison of two versions.
    compares: Arc<AtomicUsize>,
    /// The latest release's tag (v0.1.7 unless a test moves it).
    latest_tag: Arc<std::sync::Mutex<String>>,
    /// Requests for the list of releases.
    release_lists: Arc<AtomicUsize>,
    /// Releases listed before the others (newest first): `(tag, prerelease)`.
    newer: Arc<std::sync::Mutex<Vec<(String, bool)>>>,
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
        compares: Default::default(),
        latest_tag: Arc::new(std::sync::Mutex::new("v0.1.7".into())),
        release_lists: Default::default(),
        newer: Default::default(),
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
                let tag = f.latest_tag.lock().unwrap().clone();
                axum::Json(json!({"tag_name": tag, "body": "notes", "published_at": null, "assets": [
                    {"id": 1, "name": "P_0.1.7_x64-setup.exe", "size": 3},
                    {"id": 2, "name": "P.app.tar.gz", "size": 3},
                    {"id": 3, "name": "SHA256SUMS", "size": 1}]}))
                .into_response()
            }),
        )
        .route(
            "/repos/o/r/releases",
            axum::routing::get(|State(f): State<Fake>| async move {
                f.release_lists.fetch_add(1, Ordering::SeqCst);
                let r = |tag: &str, draft: bool, prerelease: bool| {
                    json!({"tag_name": tag, "body": format!("Notes for {tag}"), "published_at": "2026-10-15T00:00:00Z",
                           "draft": draft, "prerelease": prerelease, "assets": []})
                };
                let mut list: Vec<Value> = f.newer.lock().unwrap().iter().map(|(t, pre)| r(t, false, *pre)).collect();
                list.extend([
                    r("v0.2.0", false, false),
                    r("v0.1.8", true, false),
                    r("v0.1.7", false, false),
                    r("v0.1.6", false, false),
                    r("v0.1.5", false, false),
                    r("v0.1.4", false, false),
                ]);
                axum::Json(list)
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
        .route(
            "/repos/o/r/compare/{range}",
            axum::routing::get(|State(f): State<Fake>, Path(range): Path<String>| async move {
                f.compares.fetch_add(1, Ordering::SeqCst);
                let commit = |sha: String, message: String| json!({"sha": sha, "commit": {"message": message}});
                // v0.1.5 and the dev build abc1234 are two merges behind; v0.1.1 is 300 commits behind (GitHub sends
                // 250); anything older is not found.
                let (commits, total): (Vec<Value>, usize) = match range.as_str() {
                    "v0.1.5...v0.1.7" | "abc1234...v0.1.7" => (
                        vec![
                            commit("a1".into(), "TK-1: First change (#10)\n\nIts body.".into()),
                            commit("a2".into(), "TK-9: Second change (#11)".into()),
                        ],
                        2,
                    ),
                    "v0.1.1...v0.1.7" => {
                        ((0..250).map(|i| commit(format!("s{i}"), format!("Change {i} (#{i})"))).collect(), 300)
                    }
                    _ => return (StatusCode::NOT_FOUND, r#"{"message":"Not Found"}"#).into_response(),
                };
                let url = format!("https://github.com/o/r/compare/{range}");
                axum::Json(json!({"total_commits": total, "html_url": url, "commits": commits})).into_response()
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
    let mut state = common::app_state(store, tmp);
    let mut svc = pdt_api::UpdateService::new(&base, "o/r", supervised, current);
    svc.first_check_delay = Duration::ZERO;
    svc.check_interval = Some(Duration::from_millis(50));
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
            mode: pdt_core::conversation::SessionMode::Terminal,
            model: None,
            permission_mode: None,
            persona: None,
            background: vec![],
        })
        .await
        .unwrap();
}

/// Polls GET /api/host/update until `pred` holds and returns that status.
async fn wait_status(app: &axum::Router, pred: impl Fn(&Value) -> bool) -> Value {
    wait_until("the update status", async || {
        let (_, v) = call(app, "GET", "/api/host/update", None).await;
        if pred(&v) { Ok(v) } else { Err(v) }
    })
    .await
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
    assert!(!req.automatic, "the user asked for it");
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
    assert!(v["error"].as_str().unwrap_or("").ends_with(&pdt_api::secrets::stalled("the update token").0), "{v}");
    let (s, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["error"].as_str().unwrap_or("").ends_with(&pdt_api::secrets::stalled("the update token").0), "{v}");
    let (s, e) = call(&app, "PUT", "/api/host/token", Some(json!({"token": TOKEN}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("keychain_unavailable")), "{e}");
    // Host settings still load; only the token part reports the keychain.
    let (s, v) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["tokenPresent"], false, "{v}");
    assert!(v["tokenError"].as_str().unwrap_or("").ends_with(&pdt_api::secrets::stalled("the update token").0), "{v}");
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
            return Err(pdt_api::secrets::stalled("the update token"));
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
    drop_install_once_downloading(&app).await;
    fake.hold_assets.store(false, Ordering::SeqCst);
    let v = wait_status(&app, state_is("available")).await;
    assert!(v["error"].as_str().unwrap().contains("checksum"), "{v}");
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["state"].as_str(), &v["error"]), (Some("available"), &Value::Null), "{v}");

    // A good download whose request was dropped is still handed off.
    *fake.sums.lock().unwrap() = good_sums();
    fake.hold_assets.store(true, Ordering::SeqCst);
    drop_install_once_downloading(&app).await;
    fake.hold_assets.store(false, Ordering::SeqCst);
    wait_status(&app, state_is("installing")).await;
    assert!(request_path(&data(tmp.path())).exists());
}

/// Starts an install whose download the fake holds, then drops the request once the download is under way (not after
/// a fixed delay: on a loaded runner the request may not have got that far yet).
async fn drop_install_once_downloading(app: &axum::Router) {
    let a = app.clone();
    let install =
        tokio::spawn(async move { call(&a, "POST", "/api/host/update/install", Some(json!({"force": true}))).await });
    wait_status(app, state_is("downloading")).await;
    assert!(!install.is_finished(), "the download is held");
    install.abort();
    let _ = install.await;
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

/// The workflow engine, ticking as pdtd's does (faster): Install updates installs what the checks find.
fn engine(state: &pdt_api::AppState) -> tokio::task::JoinHandle<()> {
    let st = state.clone();
    tokio::spawn(async move {
        loop {
            pdt_api::run_workflows(&st).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
}

/// Install updates' runs.
async fn install_runs(app: &axum::Router) -> Vec<Value> {
    let (_, v) = call(app, "GET", "/api/install-updates", None).await;
    let wf = v["workflowId"].as_str().unwrap().to_string();
    call(app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await.1.as_array().unwrap().clone()
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
    still_after("a check for updates", Duration::from_millis(300), async || fake.hits.load(Ordering::SeqCst) > 0).await;
    task.abort();
}

#[tokio::test]
async fn the_loop_only_notifies_under_the_notify_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    set_policy(&app, "notify").await;
    let (task, workflows) = (tokio::spawn(pdt_api::run_update_loop(state.clone())), engine(&state));
    wait_status(&app, state_is("available")).await;
    // It checks again every interval, and the engine has settled the version's run.
    let runs = wait_until("a second check and a settled run", async || {
        let runs = install_runs(&app).await;
        let settled = !runs.is_empty() && runs.iter().all(|r| r["status"] == "quiet");
        if fake.hits.load(Ordering::SeqCst) >= 2 && settled { Ok(runs) } else { Err(runs) }
    })
    .await;
    task.abort();
    workflows.abort();
    assert!(!request_path(&data(tmp.path())).exists());
    assert_eq!(runs.len(), 1, "one per version: {runs:?}");
    assert_eq!(runs[0]["status"], "quiet", "nothing waits while updates only notify: {runs:?}");
}

#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn the_loop_installs_when_idle_under_auto_when_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    store_token(&state);
    set_policy(&app, "auto_when_idle").await;
    let (task, workflows) = (tokio::spawn(pdt_api::run_update_loop(state.clone())), engine(&state));
    tokio::time::timeout(Duration::from_secs(5), state.shutdown.notified()).await.expect("pdtd asked to stop");
    task.abort();
    workflows.abort();
    assert!(request_path(&data(tmp.path())).exists());
    assert_eq!(call(&app, "GET", "/api/host/update", None).await.1["state"], "installing");
    let req: pdt_core::handoff::UpdateRequest =
        serde_json::from_slice(&std::fs::read(request_path(&data(tmp.path()))).unwrap()).unwrap();
    assert!(req.automatic, "pdtd installed it by itself");
}

#[tokio::test]
async fn the_loop_waits_for_a_busy_host_under_auto_when_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    set_policy(&app, "auto_when_idle").await;
    add_busy_session(&app, &state, tmp.path()).await;
    let (task, workflows) = (tokio::spawn(pdt_api::run_update_loop(state.clone())), engine(&state));
    wait_status(&app, state_is("available")).await;
    while fake.hits.load(Ordering::SeqCst) < 3 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    task.abort();
    workflows.abort();
    assert!(!request_path(&data(tmp.path())).exists());
    let runs = install_runs(&app).await;
    assert_eq!((runs.len(), runs[0]["status"].as_str()), (1, Some("waiting")), "it waits for the agent: {runs:?}");
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
    let (task, workflows) = (tokio::spawn(pdt_api::run_update_loop(state.clone())), engine(&state));
    wait_status(&app, state_is("available")).await;
    while fake.hits.load(Ordering::SeqCst) < 3 || install_runs(&app).await.iter().any(|r| r["status"] != "quiet") {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    task.abort();
    workflows.abort();
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
    let (task, workflows) = (tokio::spawn(pdt_api::run_update_loop(state.clone())), engine(&state));
    tokio::time::timeout(Duration::from_secs(5), state.shutdown.notified()).await.expect("pdtd asked to stop");
    task.abort();
    workflows.abort();
    assert!(request_path(&data(tmp.path())).exists());
}

/// PDT-369: an update announced while updates only notified installs once they are set to install when idle.
#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn setting_updates_to_install_when_idle_installs_the_one_on_offer() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    store_token(&state);
    set_policy(&app, "notify").await;
    let (task, workflows) = (tokio::spawn(pdt_api::run_update_loop(state.clone())), engine(&state));
    wait_status(&app, state_is("available")).await;
    while install_runs(&app).await.iter().all(|r| r["status"] != "quiet") {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!request_path(&data(tmp.path())).exists());
    set_policy(&app, "auto_when_idle").await;
    tokio::time::timeout(Duration::from_secs(5), state.shutdown.notified()).await.expect("pdtd asked to stop");
    task.abort();
    workflows.abort();
    assert!(request_path(&data(tmp.path())).exists());
    assert_eq!(install_runs(&app).await.len(), 2, "a run for the update on offer");
}

#[tokio::test]
async fn an_update_lists_every_change_since_the_installed_release() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup_with(tmp.path(), true, "0.1.5").await;
    store_token(&state);
    // TK-1 is a ticket here; TK-9 is not.
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q"]).arg(&repo).status().unwrap().success());
    let (_, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "T", "repoPath": repo.to_string_lossy(), "key": "TK"})),
    )
    .await;
    let pid = p["id"].as_str().unwrap();
    let (_, t) = call(&app, "POST", &format!("/api/projects/{pid}/tickets"), Some(json!({"title": "x"}))).await;
    assert_eq!(t["key"], "TK-1");

    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    let c = &v["latest"]["changes"];
    assert_eq!(
        (c["since"].as_str(), c["total"].as_u64(), &c["moreUrl"]),
        (Some("0.1.5"), Some(2), &Value::Null),
        "{v}"
    );
    assert_eq!(
        c["entries"][0],
        json!({"sha": "a2", "subject": "TK-9: Second change (#11)", "title": "Second change", "ticket": "TK-9",
               "ticketHere": null, "pr": 11, "prUrl": "https://github.com/o/r/pull/11"}),
        "newest first"
    );
    assert_eq!(c["entries"][1]["ticketHere"], json!({"id": t["id"], "projectId": pid}));
    assert_eq!(v["latest"]["notes"], "notes", "the release's own notes stay");

    // The same pair is not compared again.
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["latest"]["changes"]["entries"].as_array().unwrap().len(), 2);
    let (_, v) = call(&app, "GET", "/api/host/update", None).await;
    assert_eq!(v["latest"]["changes"]["entries"][1]["ticketHere"]["id"], t["id"]);
    assert_eq!(fake.compares.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_update_lists_the_notes_of_every_release_since_the_installed_one() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup_with(tmp.path(), true, "0.1.5").await;
    store_token(&state);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(
        v["latest"]["releases"],
        json!([
            {"version": "0.1.7", "tag": "v0.1.7", "nightly": false, "notes": "Notes for v0.1.7",
             "publishedAt": "2026-10-15T00:00:00Z"},
            {"version": "0.1.6", "tag": "v0.1.6", "nightly": false, "notes": "Notes for v0.1.6",
             "publishedAt": "2026-10-15T00:00:00Z"}
        ]),
        "newest first, up to the latest release, no drafts: {v}"
    );
    // The same pair is not listed again.
    call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(fake.release_lists.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_per_merge_releases_move_to_the_first_deliberate_one() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup_with(tmp.path(), true, "0.1.840").await;
    store_token(&state);
    *fake.latest_tag.lock().unwrap() = "v0.2.0".into();
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["state"].as_str(), v["latest"]["version"].as_str()), (Some("available"), Some("0.2.0")), "{v}");
    let versions: Vec<_> =
        v["latest"]["releases"].as_array().unwrap().iter().map(|r| r["version"].as_str().unwrap()).collect();
    assert_eq!(versions, ["0.2.0"]);
}

#[tokio::test]
async fn an_older_latest_release_is_never_offered() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup_with(tmp.path(), true, "0.2.0").await;
    store_token(&state);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["state"].as_str(), &v["latest"], &v["error"]), (Some("idle"), &Value::Null, &Value::Null), "{v}");
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("no_update")), "no downgrade");
    assert_eq!(fake.release_lists.load(Ordering::SeqCst), 0);
}

async fn set_channel(app: &axum::Router, channel: &str) {
    let (s, v) = call(app, "PUT", "/api/host/settings", Some(json!({"updateChannel": channel}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn only_a_nightly_host_is_offered_a_nightly_build_until_its_release_ships() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup_with(tmp.path(), true, "0.2.0").await;
    store_token(&state);
    *fake.latest_tag.lock().unwrap() = "v0.2.0".into();
    fake.newer.lock().unwrap().push(("v0.2.1-nightly.20261030".into(), true));

    // A stable host ignores it.
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(
        (v["channel"].as_str(), v["state"].as_str(), &v["latest"]),
        (Some("stable"), Some("idle"), &Value::Null)
    );

    // A nightly host offers it, with its own notes.
    set_channel(&app, "nightly").await;
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(
        (v["channel"].as_str(), v["state"].as_str(), v["latest"]["version"].as_str()),
        (Some("nightly"), Some("available"), Some("0.2.1-nightly.20261030")),
        "{v}"
    );
    assert_eq!(v["latest"]["nightly"], true);
    let notes: Vec<_> = v["latest"]["releases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["tag"].as_str(), r["nightly"].as_bool()))
        .collect();
    assert_eq!(notes, [(Some("v0.2.1-nightly.20261030"), Some(true))]);

    // Then the stable 0.2.1 once it exists: it sorts above its nightlies.
    fake.newer.lock().unwrap().insert(0, ("v0.2.1".into(), false));
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["latest"]["version"].as_str(), v["latest"]["nightly"].as_bool()), (Some("0.2.1"), Some(false)));
}

/// PDT-283: a host on an experimental build whose channel was set to experimental (the old name, as the API still
/// takes it) is on the nightly channel and gets the next nightly.
#[tokio::test]
async fn an_experimental_host_gets_the_next_nightly() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup_with(tmp.path(), true, "0.2.1-exp.3").await;
    store_token(&state);
    *fake.latest_tag.lock().unwrap() = "v0.2.0".into();
    fake.newer.lock().unwrap().push(("v0.2.1-nightly.20261030".into(), true));
    fake.newer.lock().unwrap().push(("v0.2.1-exp.3".into(), true));
    set_channel(&app, "experimental").await;
    let (_, v) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(v["settings"]["updateChannel"], "nightly", "{v}");
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(
        (v["currentNightly"].as_bool(), v["channel"].as_str(), v["latest"]["version"].as_str()),
        (Some(true), Some("nightly"), Some("0.2.1-nightly.20261030")),
        "{v}"
    );
}

#[tokio::test]
async fn back_on_stable_a_nightly_build_stays_until_a_newer_stable_release() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup_with(tmp.path(), true, "0.2.1-nightly.20261030").await;
    store_token(&state);
    *fake.latest_tag.lock().unwrap() = "v0.2.0".into();
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["currentNightly"].as_bool(), v["channel"].as_str()), (Some(true), Some("stable")));
    assert_eq!((v["state"].as_str(), &v["latest"], &v["error"]), (Some("idle"), &Value::Null, &Value::Null), "{v}");
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("no_update")), "never a downgrade to 0.2.0");

    *fake.latest_tag.lock().unwrap() = "v0.2.1".into();
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["state"].as_str(), v["latest"]["version"].as_str()), (Some("available"), Some("0.2.1")), "{v}");
}

#[tokio::test]
async fn a_dev_build_compares_from_its_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup_with(tmp.path(), true, "0.1.5+gabc1234").await;
    store_token(&state);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    let c = &v["latest"]["changes"];
    assert_eq!((c["since"].as_str(), c["entries"].as_array().map(Vec::len)), (Some("0.1.5"), Some(2)), "{v}");
}

#[tokio::test]
async fn without_a_comparison_the_release_notes_stand_in() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup_with(tmp.path(), true, "0.1.4").await;
    store_token(&state);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["state"], "available", "{v}");
    assert_eq!((&v["latest"]["changes"], &v["latest"]["notes"]), (&Value::Null, &json!("notes")), "{v}");
    assert_eq!(v["error"], Value::Null, "a failed comparison is not an update error");
    // A failure is not cached: the next check asks again.
    call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(fake.compares.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_range_too_long_for_github_links_to_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup_with(tmp.path(), true, "0.1.1").await;
    store_token(&state);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    let c = &v["latest"]["changes"];
    assert_eq!((c["entries"].as_array().map(Vec::len), c["total"].as_u64()), (Some(250), Some(300)), "{v}");
    assert_eq!(c["moreUrl"], "https://github.com/o/r/compare/v0.1.1...v0.1.7");
    assert_eq!(c["entries"][0]["sha"], "s249");
}

/// A pdtd whose built-in release repository (`gone/r`, id 1) is not on the fake GitHub; `env` stands for
/// `PDT_RELEASE_REPO`.
async fn setup_moved(tmp: &std::path::Path, env: Option<&str>) -> (axum::Router, pdt_api::AppState) {
    let (base, _) = fake_github().await;
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp);
    let svc = pdt_api::UpdateService::new(&base, "gone/r", true, "0.1.5")
        .with_repo_id(1)
        .with_env_repo(env.map(String::from));
    state.update = Arc::new(svc);
    store_token(&state);
    (pdt_api::router(state.clone(), None), state)
}

#[tokio::test]
async fn the_release_repository_can_be_overridden_in_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup_moved(tmp.path(), None).await;
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["releaseRepo"], json!({"repo": "gone/r", "id": 1, "movedTo": null, "overriddenBy": null}));
    assert!(v["error"].is_string() && v["latest"].is_null(), "{v}");

    let (s, e) = call(&app, "PUT", "/api/host/settings", Some(json!({"releaseRepo": "not a repo"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    let (s, _) = call(&app, "PUT", "/api/host/settings", Some(json!({"releaseRepo": "o/r"}))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["releaseRepo"], json!({"repo": "o/r", "id": null, "movedTo": null, "overriddenBy": "setting"}));
    assert_eq!((v["latest"]["version"].as_str(), &v["error"]), (Some("0.1.7"), &Value::Null), "{v}");
    let (_, b) = call(&app, "GET", "/api/bug-reports/preview", None).await;
    assert_eq!(b["repo"], "o/r", "bug reports go there too");

    // Cleared: back to the built-in repository, by its id.
    call(&app, "PUT", "/api/host/settings", Some(json!({"releaseRepo": ""}))).await;
    let (_, v) = call(&app, "GET", "/api/host/update", None).await;
    assert_eq!(v["releaseRepo"]["repo"], "gone/r");
    assert_eq!(v["releaseRepo"]["id"], 1);
}

#[tokio::test]
async fn the_environment_override_wins_over_the_setting() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup_moved(tmp.path(), Some("o/r")).await;
    call(&app, "PUT", "/api/host/settings", Some(json!({"releaseRepo": "x/y"}))).await;
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["releaseRepo"], json!({"repo": "o/r", "id": null, "movedTo": null, "overriddenBy": "environment"}));
    assert_eq!(v["latest"]["version"], "0.1.7", "{v}");
}

#[tokio::test]
async fn a_check_says_the_host_is_up_to_date() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup_with(tmp.path(), true, "0.2.0").await;
    store_token(&state);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["result"], json!({"kind": "upToDate", "version": "0.2.0"}), "{v}");
    assert_eq!(v["state"], "idle", "the status is still all there");
}

#[tokio::test]
async fn a_check_says_which_update_is_available() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    store_token(&state);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["result"], json!({"kind": "available", "version": "0.1.7"}), "{v}");
    assert_eq!(v["latest"]["version"], "0.1.7");
}

#[tokio::test]
async fn a_check_without_a_token_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, fake) = setup(tmp.path(), true).await;
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["result"]["kind"].as_str(), v["result"]["reason"].as_str()), (Some("cannotCheck"), Some("noToken")));
    assert!(v["result"]["message"].as_str().unwrap().contains("token"), "{v}");
    assert_eq!(fake.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_check_with_a_rejected_token_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    state.secrets.set(&state.update_token_account, "revoked").unwrap();
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["result"]["reason"], "tokenRejected", "{v}");
    assert_eq!(v["token"]["state"], "expired");
}

#[tokio::test]
async fn a_check_that_cannot_reach_github_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    // A port nothing listens on: bound, then closed.
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = pdt_api::AppState::new(store, data(tmp.path()));
    state.update = Arc::new(pdt_api::UpdateService::new(&format!("http://127.0.0.1:{closed}"), "o/r", true, "0.1.0"));
    let app = pdt_api::router(state.clone(), None);
    store_token(&state);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["result"]["reason"], "network", "{v}");
    assert!(v["result"]["message"].as_str().unwrap().contains("could not reach GitHub"), "{v}");
}

#[tokio::test]
async fn a_check_with_updates_off_says_why() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, _) = setup(tmp.path(), false).await;
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["result"]["reason"], "disabled", "{v}");
}

async fn update_events(state: &pdt_api::AppState, kind: &str) -> Vec<pdt_core::workflow::api::LogEvent> {
    let f = pdt_store::EventFilter { kind: Some(kind.into()), limit: 100, ..Default::default() };
    state.store.list_log_events(&f).await.unwrap()
}

/// PDT-365: a newer version is `update.available` once, however often the checks find it.
#[tokio::test]
async fn an_available_update_is_a_workflow_event_once_per_version() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    store_token(&state);
    for _ in 0..2 {
        let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
        assert_eq!(v["state"], "available", "{v}");
    }
    let got = update_events(&state, "update.available").await;
    assert_eq!(got.len(), 1, "{got:?}");
    let p = &got[0].payload;
    assert_eq!(
        (&p["version"], &p["current"], &p["channel"], &p["nightly"], &p["notes"]),
        (&json!("0.1.7"), &json!("0.1.0-local"), &json!("stable"), &json!(false), &json!("notes"))
    );
    assert_eq!(p["notesUrl"], "https://github.com/o/r/releases/tag/v0.1.7");
    // Still waiting once the event log has forgotten it: still not news.
    pdt_api::run_workflows(&state).await;
    assert!(state.store.prune_events(i64::MAX).await.unwrap() > 0);
    call(&app, "POST", "/api/host/update/check", None).await;
    assert!(update_events(&state, "update.available").await.is_empty());

    // A newer one is news again; being up to date is none.
    *fake.latest_tag.lock().unwrap() = "v0.1.8".into();
    call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(update_events(&state, "update.available").await.len(), 1);
}

fn write_json(path: &std::path::Path, v: &impl serde::Serialize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(v).unwrap()).unwrap();
}

/// PDT-365: pdtd starting on the version pdt-host just installed is `update.installed` once, saying whether pdtd
/// asked for it by itself; a rollback, or a result for another version, is not.
#[tokio::test]
async fn an_installed_update_is_a_workflow_event_once() {
    use pdt_core::handoff::{UpdateOutcome, UpdateRequest, UpdateResult, result_path, taken_request_path};
    let tmp = tempfile::tempdir().unwrap();
    let (_, state, _) = setup_with(tmp.path(), true, "0.1.7").await;
    let result = |outcome, to: &str| UpdateResult {
        outcome,
        from_version: "0.1.6".into(),
        to_version: to.into(),
        message: String::new(),
        log: None,
        at: 5,
    };
    let taken = UpdateRequest {
        version: "0.1.7".into(),
        tag: "v0.1.7".into(),
        asset: "a".into(),
        from_version: "0.1.6".into(),
        requested_at: 1,
        automatic: true,
    };
    let d = data(tmp.path());
    write_json(&result_path(&d), &result(UpdateOutcome::RolledBack, "0.1.7"));
    pdt_api::update::log_installed(&state).await;
    write_json(&result_path(&d), &result(UpdateOutcome::Installed, "0.1.8"));
    pdt_api::update::log_installed(&state).await;
    assert!(update_events(&state, "update.installed").await.is_empty());

    write_json(&result_path(&d), &result(UpdateOutcome::Installed, "0.1.7"));
    write_json(&taken_request_path(&d), &taken);
    pdt_api::update::log_installed(&state).await;
    pdt_api::update::log_installed(&state).await;
    let got = update_events(&state, "update.installed").await;
    assert_eq!(got.len(), 1, "once, also over a restart: {got:?}");
    let p = &got[0].payload;
    assert_eq!(
        (&p["version"], &p["fromVersion"], &p["automatic"], &p["channel"], &p["at"]),
        (&json!("0.1.7"), &json!("0.1.6"), &json!(true), &json!("stable"), &json!(5))
    );
    assert_eq!(p["notesUrl"], "https://github.com/o/r/releases/tag/v0.1.7");
}
