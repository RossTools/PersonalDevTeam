mod common;

use axum::http::StatusCode;
use common::call;
use serde_json::{Value, json};

async fn setup(tmp: &std::path::Path) -> (axum::Router, String) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(
        std::process::Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success()
    );
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let app = pdt_api::router(pdt_api::AppState::new(store, tmp.join("data")), None);
    let (_, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    (app, p["id"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn project_credentials_override_the_host() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid) = setup(tmp.path()).await;
    let uri = format!("/api/projects/{pid}/settings");
    let (s, v) = call(&app, "GET", &uri, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["agentCredentials"], Value::Null);
    assert_eq!(v["effectiveCredentials"], "subscription");

    let body = |c: Value| Some(json!({"agentCredentials": c, "runOverrides": null}));
    let (s, v) = call(&app, "PUT", &uri, body(json!("pass_through"))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["effectiveCredentials"], "pass_through");

    call(&app, "PUT", "/api/host/settings", Some(json!({"agentCredentials": "pass_through"}))).await;
    let (_, v) = call(&app, "PUT", &uri, body(json!("subscription"))).await;
    assert_eq!(v["effectiveCredentials"], "subscription", "a project can tighten the host policy too");
    let (_, v) = call(&app, "PUT", &uri, body(Value::Null)).await;
    assert_eq!(v["effectiveCredentials"], "pass_through", "null falls back to the host");

    let (s, _) = call(&app, "GET", "/api/projects/nope/settings", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
