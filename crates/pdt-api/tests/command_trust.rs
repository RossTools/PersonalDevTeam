//! A repository's commands run only once the user approved them in PDT (PDT-405).

use crate::common;

use axum::http::StatusCode;
use common::{call, call_as_ui, git};
use pdt_api::{AppState, router};
use serde_json::{Value, json};
use std::path::PathBuf;

const CONFIG: &str = r#"
[[run]]
name = "hello"
command = "node -e \"process.exit(0)\""

[[test]]
name = "ok"
command = "node -e \"process.exit(1 - 1)\""
"#;

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
    repo: PathBuf,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join(".pdt").join("project.toml"), CONFIG).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.path().join("data"));
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().into(), repo, _tmp: tmp }
}

async fn start(env: &Env, body: Value) -> (StatusCode, Value) {
    call(&env.app, "POST", &format!("/api/projects/{}/runs", env.pid), Some(body)).await
}

async fn commands(env: &Env, worktree: Option<&str>) -> Value {
    let q = worktree.map(|w| format!("?worktreeId={w}")).unwrap_or_default();
    let (s, v) = call(&env.app, "GET", &format!("/api/projects/{}/commands{q}", env.pid), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

/// The fingerprints waiting on approval: in a listing, or in a refused start's commands.
fn untrusted(v: &Value) -> Vec<String> {
    let list = match v["untrusted"].as_array() {
        Some(l) => l.clone(),
        None => v["data"]["commands"].as_array().unwrap().iter().map(|c| c["fingerprint"].clone()).collect(),
    };
    list.iter().map(|f| f.as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn a_new_projects_commands_wait_for_the_users_approval() {
    let env = setup().await;
    let listed = commands(&env, None).await;
    let shown: Vec<&str> =
        listed["commands"].as_array().unwrap().iter().map(|c| c["uses"][0].as_str().unwrap()).collect();
    assert_eq!(shown, vec!["run \"hello\"", "test \"ok\""], "{listed}");
    assert_eq!(untrusted(&listed).len(), 2, "{listed}");

    let (s, v) = start(&env, json!({"name": "hello", "kind": "run"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("untrusted_commands")), "{v}");
    assert_eq!(v["data"]["projectId"], env.pid.as_str());
    assert_eq!(v["data"]["commands"][0]["command"], "node -e \"process.exit(0)\"", "{v}");
    assert!(v["message"].as_str().unwrap().contains("Only the user can approve"), "{v}");

    // An agent's call can't approve it.
    let trust = format!("/api/projects/{}/commands/trust", env.pid);
    let body = json!({ "fingerprints": untrusted(&v) });
    let (s, r) = call(&env.app, "POST", &trust, Some(body.clone())).await;
    assert_eq!((s, r["code"].as_str()), (StatusCode::FORBIDDEN, Some("not_the_user")), "{r}");
    assert_eq!(start(&env, json!({"name": "hello", "kind": "run"})).await.0, StatusCode::CONFLICT);

    // The user's can.
    let (s, r) = call_as_ui(&env.app, "POST", &trust, Some(body)).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{r}");
    let (s, v) = start(&env, json!({"name": "hello", "kind": "run"})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert!(untrusted(&commands(&env, None).await).len() == 1, "the test's command is still waiting");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_changed_command_asks_again() {
    let env = setup().await;
    common::approve_commands(&env.app, &env.pid, None).await;
    assert!(untrusted(&commands(&env, None).await).is_empty());
    // An agent (or a pull) changes the command in the checkout: it is a new command.
    let changed = CONFIG.replace("[[test]]", "[[run]]\nname = \"evil\"\ncommand = \"echo pwned\"\n\n[[test]]");
    std::fs::write(env.repo.join(".pdt").join("project.toml"), &changed).unwrap();
    let (s, v) = start(&env, json!({"name": "evil", "kind": "run"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("untrusted_commands")), "{v}");
    let waiting = commands(&env, None).await;
    assert_eq!(untrusted(&waiting).len(), 1, "{waiting}");
    // The approved ones still run.
    let (s, v) = start(&env, json!({"name": "hello", "kind": "run"})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_branch_without_a_worktree_asks_before_its_commands_run() {
    let env = setup().await;
    common::approve_commands(&env.app, &env.pid, None).await;
    git(&env.repo, &["checkout", "-q", "-b", "other"]);
    let changed = CONFIG.replacen("process.exit(0)", "process.exit(0); console.log(1)", 1);
    assert_ne!(changed, CONFIG);
    std::fs::write(env.repo.join(".pdt").join("project.toml"), &changed).unwrap();
    git(&env.repo, &["commit", "-q", "-am", "change"]);
    git(&env.repo, &["checkout", "-q", "main"]);
    let (s, v) = start(&env, json!({"name": "hello", "kind": "run", "branch": "other"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("untrusted_commands")), "{v}");
    assert_eq!(v["data"]["commands"][0]["command"], "node -e \"process.exit(0); console.log(1)\"", "{v}");
    // Nothing was made for it.
    let (_, wts) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.pid), None).await;
    assert!(!wts.to_string().contains("\"other\""), "{wts}");
}

#[tokio::test]
async fn overrides_the_user_saves_are_approved_but_an_api_callers_are_not() {
    let env = setup().await;
    let settings = format!("/api/projects/{}/settings", env.pid);
    let mine = "[[run]]\nname = \"mine\"\ncommand = \"node -e \\\"process.exit(0)\\\"\"\n";
    let (s, v) = call_as_ui(&env.app, "PUT", &settings, Some(json!({"runOverrides": mine}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = start(&env, json!({"name": "mine", "kind": "run"})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    let theirs = "[[run]]\nname = \"theirs\"\ncommand = \"echo pwned\"\n";
    let (s, v) = call(&env.app, "PUT", &settings, Some(json!({"runOverrides": theirs}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = start(&env, json!({"name": "theirs", "kind": "run"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("untrusted_commands")), "{v}");
    env.state.ptys.shutdown().await;
}
