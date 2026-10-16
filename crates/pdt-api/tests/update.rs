mod common;

use axum::http::StatusCode;
use common::call;
use pdt_core::activity::Activity;
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use serde_json::json;
use sha2::Digest;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const WIN: &[u8] = b"win";
const MAC: &[u8] = b"mac";

#[derive(Clone)]
struct Fake {
    hits: Arc<AtomicUsize>,
    sums: Arc<std::sync::Mutex<String>>,
}

async fn fake_github() -> (String, Fake) {
    use axum::extract::{Path, State};
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    let sha = |b: &[u8]| hex::encode(sha2::Sha256::digest(b));
    let fake = Fake {
        hits: Default::default(),
        sums: Arc::new(std::sync::Mutex::new(format!(
            "{}  P_0.1.7_x64-setup.exe\n{}  P.app.tar.gz\n",
            sha(WIN),
            sha(MAC)
        ))),
    };
    let ok = |h: &HeaderMap| h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer good");
    let app = axum::Router::new()
        .route(
            "/repos/o/r/releases/latest",
            axum::routing::get(move |State(f): State<Fake>, h: HeaderMap| async move {
                f.hits.fetch_add(1, Ordering::SeqCst);
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

async fn setup(tmp: &std::path::Path, supervised: bool) -> (axum::Router, pdt_api::AppState, Fake) {
    let (base, fake) = fake_github().await;
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = pdt_api::AppState::new(store, tmp.join("data"));
    state.update = Arc::new(pdt_api::UpdateService::new(&base, "o/r", supervised, "0.1.0-local"));
    (pdt_api::router(state.clone(), None), state, fake)
}

#[tokio::test]
async fn unsupervised_pdtd_never_updates() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), false).await;
    state.secrets.set(pdt_api::secrets::UPDATE_TOKEN, "good").unwrap();
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
async fn busy_hosts_ask_first_then_the_release_is_handed_off() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    let (s, _) = call(&app, "PUT", "/api/host/token", Some(json!({"token": "good"}))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["state"], "available", "{v}");
    assert_eq!(v["latest"]["version"], "0.1.7");
    assert_eq!(v["latest"]["notes"], "notes");

    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q"]).arg(&repo).status().unwrap().success());
    let (_, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    state
        .store
        .insert_session(&Session {
            id: "busy".into(),
            project_id: p["id"].as_str().unwrap().into(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Claude),
            title: "Claude · feature".into(),
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
        })
        .await
        .unwrap();
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": false}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("host_busy")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("Claude · feature"));
    assert!(!pdt_core::handoff::request_path(&tmp.path().join("data")).exists());

    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "installing");
    let req: pdt_core::handoff::UpdateRequest =
        serde_json::from_slice(&std::fs::read(pdt_core::handoff::request_path(&tmp.path().join("data"))).unwrap())
            .unwrap();
    assert_eq!((req.version.as_str(), req.from_version.as_str()), ("0.1.7", "0.1.0-local"));
    assert_eq!(std::fs::read(&req.asset).unwrap(), if cfg!(windows) { WIN } else { MAC });
    tokio::time::timeout(std::time::Duration::from_secs(1), state.shutdown.notified())
        .await
        .expect("pdtd asked to stop");
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["state"], "installing", "a check does not undo the hand-off");
}

#[tokio::test]
async fn bad_checksum_hands_nothing_off() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    state.secrets.set(pdt_api::secrets::UPDATE_TOKEN, "good").unwrap();
    *fake.sums.lock().unwrap() =
        format!("{}  P_0.1.7_x64-setup.exe\n{}  P.app.tar.gz\n", "0".repeat(64), "0".repeat(64));
    call(&app, "POST", "/api/host/update/check", None).await;
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("update_failed")), "{v}");
    assert!(!pdt_core::handoff::request_path(&tmp.path().join("data")).exists());
    let (_, v) = call(&app, "GET", "/api/host/update", None).await;
    assert_eq!(v["state"], "available", "can be retried");
}

#[tokio::test]
async fn tokens_are_checked_stored_and_never_echoed() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    let (s, v) = call(&app, "PUT", "/api/host/token", Some(json!({"token": "expired"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_token")));
    assert_eq!(state.secrets.get(pdt_api::secrets::UPDATE_TOKEN).unwrap(), None);
    let (s, v) = call(&app, "PUT", "/api/host/token", Some(json!({"token": " good "}))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!v.to_string().contains("good"), "{v}");
    assert_eq!(state.secrets.get(pdt_api::secrets::UPDATE_TOKEN).unwrap().as_deref(), Some("good"));
    assert_eq!(call(&app, "DELETE", "/api/host/token", None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(state.secrets.get(pdt_api::secrets::UPDATE_TOKEN).unwrap(), None);
}
