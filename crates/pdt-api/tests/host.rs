mod common;

use axum::http::StatusCode;
use common::call;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};

async fn setup(tmp: &std::path::Path) -> (axum::Router, AppState) {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.join("data"));
    (router(state.clone(), None), state)
}

#[tokio::test]
async fn settings_start_at_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup(tmp.path()).await;
    let (s, v) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["settings"]["remoteAccess"], false);
    assert_eq!(v["settings"]["updatePolicy"], "notify");
    assert_eq!(v["settings"]["runPorts"], json!({"start": 8400, "end": 8499}));
    assert_eq!(v["settings"]["maxConcurrentRuns"], 4);
    assert_eq!(v["settings"]["agentCredentials"], "subscription");
    assert_eq!(v["ownerFromEnv"], Value::Null);
    assert_eq!(v["tokenPresent"], false);
}

#[tokio::test]
async fn settings_patch_persists_and_validates() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state) = setup(tmp.path()).await;
    let (s, v) = call(
        &app,
        "PUT",
        "/api/host/settings",
        Some(json!({"updatePolicy": "manual", "maxConcurrentRuns": 2, "ownerLogin": " me@example.com "})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["settings"]["updatePolicy"], "manual");
    assert_eq!(v["settings"]["ownerLogin"], "me@example.com");
    assert_eq!(state.store.get_host_settings().await.unwrap().max_concurrent_runs, 2);

    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"maxConcurrentRuns": 0}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (s, _) = call(&app, "PUT", "/api/host/settings", Some(json!({"updatePolicy": "sometimes"}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.store.get_host_settings().await.unwrap().max_concurrent_runs, 2, "unchanged");
}

#[tokio::test]
async fn token_presence_comes_from_the_secret_store() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state) = setup(tmp.path()).await;
    state.secrets.set(&state.update_token_account, "github_pat_x").unwrap();
    let (_, v) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(v["tokenPresent"], true);
    assert!(!v.to_string().contains("github_pat_x"), "the token itself is never returned");
}
