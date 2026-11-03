use crate::common;

use axum::http::StatusCode;
use common::call;
use pdt_api::host_service::StartAtLogin;
use pdt_core::activity::Activity;
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use pdt_store::Store;
use serde_json::json;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

async fn setup(tmp: &std::path::Path) -> (axum::Router, pdt_api::AppState) {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = common::app_state(store, tmp);
    (pdt_api::router(state.clone(), None), state)
}

/// A login item that only remembers what it was told.
#[derive(Default)]
struct FakeLogin {
    on: AtomicBool,
    calls: Mutex<Vec<bool>>,
    fail: AtomicBool,
}

impl StartAtLogin for FakeLogin {
    fn unavailable(&self) -> Option<String> {
        None
    }
    fn registered(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }
    fn set(&self, on: bool) -> Result<(), String> {
        self.calls.lock().unwrap().push(on);
        if self.fail.load(Ordering::SeqCst) {
            return Err("launchd said no".into());
        }
        self.on.store(on, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn start_at_login_switches_both_ways() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    let login = std::sync::Arc::new(FakeLogin { on: AtomicBool::new(true), ..Default::default() });
    state.start_at_login = login.clone();
    let app = pdt_api::router(state, None);

    let (s, v) = call(&app, "GET", "/api/host/start-at-login", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!({"on": true, "unavailable": null}));
    let (_, v) = call(&app, "PUT", "/api/host/start-at-login", Some(json!({"on": false}))).await;
    assert_eq!(v["on"], false);
    let (_, v) = call(&app, "PUT", "/api/host/start-at-login", Some(json!({"on": true}))).await;
    assert_eq!(v["on"], true);
    assert_eq!(*login.calls.lock().unwrap(), [false, true]);

    login.fail.store(true, Ordering::SeqCst);
    let (s, v) = call(&app, "PUT", "/api/host/start-at-login", Some(json!({"on": false}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::INTERNAL_SERVER_ERROR, Some("start_at_login_failed")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("launchd said no"));
}

#[tokio::test]
async fn start_at_login_says_why_it_cannot_change_here() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup(tmp.path()).await;
    let (_, v) = call(&app, "GET", "/api/host/start-at-login", None).await;
    assert_eq!(v["on"], false);
    assert!(v["unavailable"].is_string(), "{v}");
    let (s, v) = call(&app, "PUT", "/api/host/start-at-login", Some(json!({"on": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("start_at_login_unavailable")), "{v}");
}

#[tokio::test]
async fn busy_lists_the_working_agents_stopping_would_interrupt() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state) = setup(tmp.path()).await;
    let (_, v) = call(&app, "GET", "/api/host/busy", None).await;
    assert_eq!(v, json!({"reasons": []}));

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
    let (_, v) = call(&app, "GET", "/api/host/busy", None).await;
    let reasons: Vec<&str> = v["reasons"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
    assert_eq!(reasons.len(), 1, "{v}");
    assert!(reasons[0].contains("feature is working"), "{v}");
}
