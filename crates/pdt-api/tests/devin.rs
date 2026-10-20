//! Devin terminal sessions (PDT-54; conversations: devin_conversations.rs) with a stand-in `devin`: launch flags, the per-session config with PDT's hook,
//! the conversation id its SessionStart hook reports, and resuming it.

mod common;

use axum::http::StatusCode;
use common::{argv_agent, call, git, recorded_launches};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
    bin: PathBuf,
}

/// A project whose repo has one commit, a stand-in `devin`, and a user Devin config of our own (never the real one).
async fn setup(tmp: &Path) -> Env {
    let repo = tmp.join("my repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let user_config = tmp.join("devin-user").join("config.json");
    std::fs::create_dir_all(user_config.parent().unwrap()).unwrap();
    std::fs::write(&user_config, "{\n  // mine\n  \"devin\": { \"org_id\": \"org-1\" },\n}\n").unwrap();
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.platform.devin_program = argv_agent(&bin, "fakedevin").to_string_lossy().into_owned();
    state.platform.devin_config_file = Some(user_config);
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().to_string(), bin }
}

async fn wait_status(app: &axum::Router, id: &str, want: &str) -> Value {
    for _ in 0..100 {
        let (_, s) = call(app, "GET", &format!("/api/sessions/{id}"), None).await;
        if s["status"] == want {
            return s;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("session {id} never became {want}");
}

#[tokio::test]
async fn devin_runs_as_a_terminal_with_pdts_hook_and_resumes_the_conversation_it_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let body = json!({
        "projectId": env.pid, "kind": "agent_tui", "agent": "devin", "mode": "terminal", "bypassPermissions": true,
        "initialPrompt": "fix the tests", "cols": 80, "rows": 24,
    });
    let (s, sess) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{sess}");
    assert_eq!((sess["mode"].as_str(), sess["agent"].as_str()), (Some("terminal"), Some("devin")));
    let id = sess["id"].as_str().unwrap().to_string();

    let config = env.state.data_dir.join("sessions").join(&id).join("devin-config.json");
    let launches = recorded_launches(&env.bin, "fakedevin", 1).await;
    assert_eq!(launches.len(), 1);
    if cfg!(windows) {
        let line = &launches[0][0];
        assert!(line.contains("--config") && line.contains("--permission-mode dangerous -- "), "{line}");
    } else {
        assert_eq!(
            launches[0],
            ["--config", &config.to_string_lossy(), "--permission-mode", "dangerous", "--", "fix the tests"],
            "the config first, then Devin's flags, then the prompt after `--` (a bare argument is a folder to Devin)"
        );
    }
    // The user's own settings are kept; PDT's hook hands over the session's instructions.
    let written: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(written["devin"]["org_id"], "org-1");
    let hook = written["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str().unwrap();
    assert!(hook.contains("hook devin --instructions"), "{hook}");
    let instructions = env.state.data_dir.join("sessions").join(&id).join("instructions.md");
    assert!(hook.contains(&*instructions.to_string_lossy()), "{hook}");
    assert!(std::fs::read_to_string(&instructions).unwrap().contains(&id), "the instructions name the session");

    // Its SessionStart hook names the conversation; restarting resumes it (and never repeats the prompt).
    let report = json!({"source": "devin", "event": "SessionStart", "sessionId": "brisk-otter"});
    let (s, _) = call(&env.app, "POST", &format!("/api/sessions/{id}/activity"), Some(report)).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, got) = call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(got["externalSessionId"], "brisk-otter");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    wait_status(&env.app, &id, "finished").await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{id}/restart"), Some(json!({"cols": 80, "rows": 24}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let launches = recorded_launches(&env.bin, "fakedevin", 2).await;
    assert_eq!(launches.len(), 2);
    if cfg!(windows) {
        assert!(launches[1][0].contains("--resume brisk-otter"), "{:?}", launches[1]);
    } else {
        assert_eq!(
            launches[1],
            ["--config", &config.to_string_lossy(), "--resume", "brisk-otter", "--permission-mode", "dangerous"]
        );
    }
    // A resumed conversation Devin never saved comes back under a new id; the latest report wins.
    let report = json!({"source": "devin", "event": "SessionStart", "sessionId": "rhinestone-violet"});
    call(&env.app, "POST", &format!("/api/sessions/{id}/activity"), Some(report)).await;
    let (_, got) = call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(got["externalSessionId"], "rhinestone-violet");
    // Junk is never recorded.
    let report = json!({"source": "devin", "event": "SessionStart", "sessionId": "a b; rm -rf"});
    call(&env.app, "POST", &format!("/api/sessions/{id}/activity"), Some(report)).await;
    let (_, got) = call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(got["externalSessionId"], "rhinestone-violet");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn an_unreadable_user_config_starts_devin_on_its_own_config_without_pdts_hook() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    std::fs::write(env.state.platform.devin_config_file.as_ref().unwrap(), "{ not json").unwrap();
    let body = json!({"projectId": env.pid, "kind": "agent_tui", "agent": "devin", "mode": "terminal", "cols": 80, "rows": 24});
    let (s, sess) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{sess}");
    let launches = recorded_launches(&env.bin, "fakedevin", 1).await;
    if cfg!(windows) {
        assert!(!launches[0][0].contains("--config"), "{:?}", launches[0]);
    } else {
        assert_eq!(launches[0], Vec::<String>::new(), "no --config that would hide the user's settings");
    }
    env.state.ptys.shutdown().await;
}
