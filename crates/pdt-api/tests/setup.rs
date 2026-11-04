//! Get started (PDT-230): first-run detection, steps ticked off from real state, skipping GitHub, dismissing, the
//! agent program set by hand, and tickets' permission mode (Ask on a new install, PDT-562).

use crate::common;

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
    fake_answering(dir, name, true)
}

/// A fake agent program whose login check says it is logged in (`logged_in`) or not.
fn fake_answering(dir: &Path, name: &str, logged_in: bool) -> String {
    let (said, code) = if logged_in { ("Logged in", 0) } else { ("Not logged in", 1) };
    if cfg!(windows) {
        let p = dir.join(format!("{name}.cmd"));
        std::fs::write(&p, format!("@echo off\r\necho {said}\r\nexit /b {code}\r\n")).unwrap();
        p.to_string_lossy().into()
    } else {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\necho '{said}'\nexit {code}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p.to_string_lossy().into()
    }
}

/// A host with no agent found (their programs point nowhere, the ACP agents' too, whatever this machine has) unless
/// `codex` is given.
async fn host(tmp: &Path, codex: Option<String>) -> (axum::Router, AppState) {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut st = common::app_state(store, tmp);
    st.platform.claude_program = missing(tmp, "claude");
    st.platform.codex_program = codex.unwrap_or_else(|| missing(tmp, "codex"));
    st.platform.devin_program = missing(tmp, "devin");
    st.host_env =
        std::sync::Arc::new(pdt_api::HostEnv { acp_program: Some(missing(tmp, "acp")), ..Default::default() });
    (router(st.clone(), None), st)
}

async fn add_project(app: &axum::Router, dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    crate::common::init_repo(dir);
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
    assert_eq!(
        v["steps"],
        json!({"agents": "todo", "github": "todo", "project": "todo", "defaultAgent": "todo", "permissions": "todo"})
    );
    // A new install's tickets start in Ask until the user picks another mode.
    let (_, h) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(h["settings"]["ticketPermissionMode"], "ask", "{h}");

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
    assert_eq!((v["done"].as_u64(), v["opensOnLaunch"].as_bool()), (Some(4), Some(true)), "permissions to do");

    // Picking tickets' permission mode (the same call Settings makes) confirms it.
    let (s, h) = call(&app, "PUT", "/api/host/settings", Some(json!({"ticketPermissionMode": "bypass"}))).await;
    assert_eq!(s, StatusCode::OK, "{h}");
    let v = setup(&app).await;
    assert_eq!(v["steps"]["permissions"], "done", "{v}");
    assert_eq!(v["done"], 5);
    assert_eq!((v["state"]["finished"].as_bool(), v["opensOnLaunch"].as_bool()), (Some(true), Some(false)));

    // Finished stays finished, even when a step comes undone.
    call(&app, "PUT", "/api/host/setup", Some(json!({"githubSkipped": false}))).await;
    let v = setup(&app).await;
    assert_eq!((v["done"].as_u64(), v["opensOnLaunch"].as_bool()), (Some(4), Some(false)));
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

#[tokio::test]
async fn an_agent_found_but_not_logged_in_leaves_the_agents_step_to_do() {
    let tmp = tempfile::tempdir().unwrap();
    let codex = fake_answering(tmp.path(), "codex", false);
    let (app, _) = host(tmp.path(), Some(codex)).await;
    let (_, checks) = call(&app, "POST", "/api/host/checks/refresh", None).await;
    assert_eq!(
        (checks["codex"]["found"].as_bool(), checks["codex"]["ok"].as_bool()),
        (Some(true), Some(false)),
        "{checks}"
    );
    let v = setup(&app).await;
    assert_eq!(v["steps"]["agents"], "todo", "{v}");
}

/// Get started and the checks on a host with no built-in agent, whose ACP agents all run `acp_program`.
async fn with_acp_program(tmp: &Path, acp_program: impl FnOnce(&Path) -> String) -> (Value, Value) {
    let (_, mut st) = host(tmp, None).await;
    st.host_env = std::sync::Arc::new(pdt_api::HostEnv { acp_program: Some(acp_program(tmp)), ..Default::default() });
    let app = router(st, None);
    let (_, checks) = call(&app, "POST", "/api/host/checks/refresh", None).await;
    (setup(&app).await, checks)
}

#[tokio::test]
async fn an_acp_agent_alone_finishes_the_agents_step_and_pdt_acp_program_turns_them_off() {
    // Every ACP agent runs the stand-in: found, so the step is done with no built-in agent.
    let tmp = tempfile::tempdir().unwrap();
    let (v, checks) = with_acp_program(tmp.path(), |d| fake(d, "acp-agent")).await;
    assert_eq!(checks["acp"]["acp:omp"]["found"], true, "{checks}");
    assert_eq!(v["steps"]["agents"], "done", "{v}");
    // A missing stand-in: no ACP agent is found, whatever this machine has installed.
    let tmp = tempfile::tempdir().unwrap();
    let (v, checks) = with_acp_program(tmp.path(), |d| missing(d, "acp")).await;
    assert!(checks["acp"].as_object().unwrap().values().all(|c| c["found"] == false), "{checks}");
    assert_eq!(v["steps"]["agents"], "todo", "{v}");
}
