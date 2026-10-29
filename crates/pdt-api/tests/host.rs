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

/// A state whose data dir exists, with `env` as what the environment sets (PDT-231).
async fn setup_env(tmp: &std::path::Path, env: pdt_api::HostEnv) -> (axum::Router, AppState) {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    std::fs::create_dir_all(tmp.join("data")).unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.listen_port = 7420;
    state.host_env = std::sync::Arc::new(env);
    (router(state.clone(), None), state)
}

#[tokio::test]
async fn network_settings_are_saved_to_their_file_and_ask_for_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state) = setup_env(tmp.path(), pdt_api::HostEnv { default_port: 7420, ..Default::default() }).await;
    let (_, v) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(v["network"]["settings"], json!({"port": null, "allowedHosts": []}));
    assert_eq!(
        (v["network"]["restartNeeded"].as_bool(), v["network"]["canRestart"].as_bool()),
        (Some(false), Some(false))
    );
    assert_eq!((v["network"]["listenPort"].as_u64(), v["network"]["defaultPort"].as_u64()), (Some(7420), Some(7420)));

    let (s, v) =
        call(&app, "PUT", "/api/host/network", Some(json!({"port": 7500, "allowedHosts": [" pdt.local ", ""]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["network"]["settings"], json!({"port": 7500, "allowedHosts": ["pdt.local"]}));
    assert_eq!(v["network"]["restartNeeded"], true);
    let file = std::fs::read(pdt_core::network::network_path(&state.data_dir)).unwrap();
    assert_eq!(pdt_core::network::NetworkSettings::parse(&file).port, Some(7500));

    // Within the runs' ports, or not a host name: refused, and the file stays.
    let (s, v) = call(&app, "PUT", "/api/host/network", Some(json!({"port": 8450, "allowedHosts": []}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")), "{v}");
    let (s, _) =
        call(&app, "PUT", "/api/host/network", Some(json!({"port": 7500, "allowedHosts": ["http://x"]}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, v) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(v["network"]["settings"]["port"], 7500);

    // Back to what pdtd started with: nothing to restart for.
    let (_, v) = call(&app, "PUT", "/api/host/network", Some(json!({"port": null, "allowedHosts": []}))).await;
    assert_eq!(v["network"]["restartNeeded"], false);
}

#[tokio::test]
async fn the_environment_wins_over_the_network_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let env = pdt_api::HostEnv { port: Some(9000), allowed_hosts: Some(vec!["other:1".into()]), ..Default::default() };
    let (app, _) = setup_env(tmp.path(), env).await;
    let (_, v) =
        call(&app, "PUT", "/api/host/network", Some(json!({"port": 7500, "allowedHosts": ["pdt.local"]}))).await;
    assert_eq!(v["network"]["portFromEnv"], 9000);
    assert_eq!(v["network"]["allowedHostsFromEnv"], json!(["other:1"]));
    assert_eq!(v["network"]["restartNeeded"], false, "a restart would change nothing");
}

#[tokio::test]
async fn restart_needs_the_host_supervisor() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup_env(tmp.path(), Default::default()).await;
    let (s, v) = call(&app, "POST", "/api/host/restart", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("not_supervised")));

    let tmp = tempfile::tempdir().unwrap();
    let (app, state) = setup_env(tmp.path(), pdt_api::HostEnv { supervised: true, ..Default::default() }).await;
    let asked = state.shutdown.clone();
    let waiting = tokio::spawn(async move { asked.notified().await });
    tokio::task::yield_now().await;
    let (s, _) = call(&app, "POST", "/api/host/restart", None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    tokio::time::timeout(std::time::Duration::from_secs(5), waiting).await.expect("pdtd was asked to stop").unwrap();
}

#[tokio::test]
async fn agent_programs_come_from_the_environment_then_the_setting() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join(if cfg!(windows) { "my-claude.exe" } else { "my-claude" });
    std::fs::write(&claude, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let claude = claude.to_string_lossy().into_owned();
    let env = pdt_api::HostEnv { codex_program: Some("/env/codex".into()), ..Default::default() };
    let (app, mut state) = setup_env(tmp.path(), env).await;
    state.platform.codex_program = "/env/codex".into();
    let missing = tmp.path().join("no-such-devin").to_string_lossy().into_owned();
    let set_codex = tmp.path().join("set-codex").to_string_lossy().into_owned();
    let (s, v) = call(
        &app,
        "PUT",
        "/api/host/settings",
        Some(json!({"claudeProgram": claude, "codexProgram": set_codex, "devinProgram": missing})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let programs = v["programs"].as_array().unwrap();
    assert_eq!(programs[0]["agent"], "claude");
    assert_eq!(programs[0]["found"].as_str(), Some(claude.as_str()));
    assert_eq!(programs[1]["fromEnv"], "/env/codex");
    assert_eq!(programs[1]["envVar"], "PDT_CODEX_PROGRAM");
    assert_eq!(programs[2]["found"], Value::Null);
    // Sessions launch the setting's program, except where the environment names one.
    let p = state.agent_platform().await;
    assert_eq!(
        (p.claude_program.as_str(), p.codex_program.as_str(), p.devin_program.as_str()),
        (claude.as_str(), "/env/codex", missing.as_str())
    );
    // Relative paths are refused.
    let (s, _) = call(&app, "PUT", "/api/host/settings", Some(json!({"claudeProgram": "claude"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
