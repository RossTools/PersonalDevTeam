//! The Jira check's liveness (PDT-127): its calls end within their timeout against a server that never answers, and
//! the poll's later members still run after it.

#[path = "../../pdt-api/tests/common/mod.rs"]
mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_core::workflow::api::NewLogEvent;
use pdt_store::Store;
use serde_json::json;
use std::time::{Duration, Instant};

/// A server that accepts connections and never answers (a half-open connection, a black-holing proxy).
async fn black_hole() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = vec![];
        while let Ok((s, _)) = listener.accept().await {
            held.push(s);
        }
    });
    format!("http://{addr}")
}

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.integrations = std::sync::Arc::new(pdt_api::Integrations::new(vec![std::sync::Arc::new(pdt_int_jira::Jira)]));
    state.integrations.enable(&["jira".into()]);
    state.sensor_timeout = Duration::from_millis(300);
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Work", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    Env { app, state, project, _tmp: tmp }
}

/// Links the project to a Jira connection at `url`, as if the sensor had looked once already (so the next look
/// calls Jira).
async fn link_jira(env: &Env, url: &str) {
    let conn = json!({"name": "work-jira", "kind": "jira", "url": url, "config": {"email": "me@example.com"}});
    let (s, c) = call(&env.app, "POST", "/api/connections", Some(conn)).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let id = c["id"].as_str().unwrap();
    let (s, _) = call(&env.app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": "t"}))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{}/settings", env.project),
        Some(json!({"agentCredentials": null, "runOverrides": null,
            "integrations": {"jira": {"connection": "work-jira", "scope": "project = ABC"}}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let v = json!({"since": 1_000, "connection": id, "scope": "project = ABC"});
    env.state.store.put_host_value(&format!("sensor.jira.{}", env.project), &v).await.unwrap();
}

#[tokio::test]
async fn a_sensor_call_to_a_server_that_never_answers_ends_within_its_timeout() {
    let env = setup().await;
    link_jira(&env, &black_hole().await).await;
    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(10), pdt_int_jira::refresh_jira(&env.state))
        .await
        .expect("the Jira check ended");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    // Not counted as a look: the next one asks again from the same point.
    let key = format!("sensor.jira.{}", env.project);
    assert_eq!(env.state.store.get_host_value(&key).await.unwrap().unwrap()["since"], 1_000);
}

#[tokio::test]
async fn the_poll_goes_on_past_a_hung_sensor() {
    let env = setup().await;
    link_jira(&env, &black_hole().await).await;
    // An event old enough that workflow upkeep (the poll's last member) prunes it.
    let old = NewLogEvent::new("test.old", "k1", "test", json!({}));
    let logged = env.state.store.emit_event(&old, 1).await.unwrap().unwrap();
    env.state.store.mark_event_processed(logged.seq, 2).await.unwrap();
    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(20), pdt_api::poll_pass(env.state.clone())).await.expect("the poll ended");
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    let (_, v) = call(&env.app, "GET", "/api/events?type=test.old", None).await;
    assert_eq!(v, json!([]), "workflow upkeep ran after the Jira check");
}
