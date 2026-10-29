//! Get started (PDT-230): first-run detection, steps ticked off from real state, skipping GitHub, dismissing, and the
//! agent program set by hand.

mod common;

use axum::http::StatusCode;
use common::call;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;

fn missing(dir: &Path, name: &str) -> String {
    dir.join(format!("no-such-{name}")).to_string_lossy().into_owned()
}

/// A fake agent program that answers its login check.
fn fake(dir: &Path, name: &str) -> String {
    if cfg!(windows) {
        let p = dir.join(format!("{name}.cmd"));
        std::fs::write(&p, "@echo off\r\necho Logged in\r\nexit /b 0\r\n").unwrap();
        p.to_string_lossy().into()
    } else {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\necho 'Logged in'\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p.to_string_lossy().into()
    }
}

/// A host with no agent found (their programs point nowhere) unless `codex` is given.
async fn host(tmp: &Path, codex: Option<String>) -> (axum::Router, AppState) {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut st = AppState::new(store, tmp.join("data"));
    st.platform.claude_program = missing(tmp, "claude");
    st.platform.codex_program = codex.unwrap_or_else(|| missing(tmp, "codex"));
    st.platform.devin_program = missing(tmp, "devin");
    (router(st.clone(), None), st)
}

async fn add_project(app: &axum::Router, dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q", "-b", "main"]).arg(dir).status().unwrap().success());
    let (s, v) =
        call(app, "POST", "/api/projects", Some(json!({"name": "demo", "repoPath": dir.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

async fn setup(app: &axum::Router) -> Value {
    let (s, v) = call(app, "GET", "/api/host/setup", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

#[tokio::test]
async fn a_new_install_opens_on_get_started_and_steps_tick_off_from_real_state() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = host(tmp.path(), None).await;
    let v = setup(&app).await;
    assert_eq!(v["opensOnLaunch"], true, "{v}");
    assert_eq!(v["state"]["needed"], true);
    assert_eq!(v["done"], 0);
    assert_eq!(v["steps"], json!({"agents": "todo", "github": "todo", "project": "todo", "defaultAgent": "todo"}));

    // Skipping GitHub counts as done.
    let (s, v) = call(&app, "PUT", "/api/host/setup", Some(json!({"githubSkipped": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["steps"]["github"].as_str(), v["done"].as_u64()), (Some("skipped"), Some(1)));

    // A project.
    add_project(&app, &tmp.path().join("repo")).await;
    assert_eq!(setup(&app).await["steps"]["project"], "done");

    // Setting the default agent (the same call Settings makes) confirms it.
    let (s, _) = call(&app, "PUT", "/api/host/settings", Some(json!({"defaultAgent": "claude"}))).await;
    assert_eq!(s, StatusCode::OK);
    let v = setup(&app).await;
    assert_eq!(v["steps"]["defaultAgent"], "done");
    assert_eq!((v["done"].as_u64(), v["opensOnLaunch"].as_bool()), (Some(3), Some(true)), "no agent yet");

    // "Set a path by hand" makes a missing agent found once the checks run again.
    let codex = fake(tmp.path(), "my-codex");
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"codexProgram": codex}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["settings"]["codexProgram"].as_str(), Some(codex.as_str()));
    let (_, checks) = call(&app, "POST", "/api/host/checks/refresh", None).await;
    assert_eq!(checks["codex"]["found"], true, "{checks}");
    assert!(checks["codex"]["path"].as_str().unwrap().contains("my-codex"), "{checks}");
    let v = setup(&app).await;
    assert_eq!(v["steps"]["agents"], "done", "{v}");
    assert_eq!(v["done"], 4);
    assert_eq!((v["state"]["finished"].as_bool(), v["opensOnLaunch"].as_bool()), (Some(true), Some(false)));

    // Finished stays finished, even when a step comes undone.
    call(&app, "PUT", "/api/host/setup", Some(json!({"githubSkipped": false}))).await;
    let v = setup(&app).await;
    assert_eq!((v["done"].as_u64(), v["opensOnLaunch"].as_bool()), (Some(3), Some(false)));
}

#[tokio::test]
async fn dismiss_hides_it_and_settings_reopens_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = host(tmp.path(), None).await;
    let (_, v) = call(&app, "PUT", "/api/host/setup", Some(json!({"dismissed": true}))).await;
    assert_eq!(v["opensOnLaunch"], false, "{v}");
    assert_eq!(setup(&app).await["opensOnLaunch"], false, "stored on the host");
    let (_, v) = call(&app, "PUT", "/api/host/setup", Some(json!({"dismissed": false}))).await;
    assert_eq!(v["opensOnLaunch"], true);
}

#[tokio::test]
async fn an_existing_install_with_a_project_and_an_agent_never_sees_it() {
    let tmp = tempfile::tempdir().unwrap();
    let codex = fake(tmp.path(), "codex");
    let (app, _) = host(tmp.path(), Some(codex)).await;
    add_project(&app, &tmp.path().join("repo")).await;
    let v = setup(&app).await;
    assert_eq!(v["state"]["needed"], false, "{v}");
    assert_eq!(v["opensOnLaunch"], false);
}
