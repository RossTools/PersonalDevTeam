//! A project's branch naming (PDT-594): the setting, the repository's `[branch]` (which wins), the name Start gives,
//! and taken names.

use crate::common;

use axum::http::StatusCode;
use common::{call, call_as_ui, git};
use pdt_api::{AppState, router};
use pdt_core::issue_link::LinkOrigin;
use pdt_store::{Store, StoredExternalLink};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
    repo: PathBuf,
}

/// A project (key PDT) whose repo has one commit on main.
async fn setup(tmp: &Path) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = common::app_state(store, tmp);
    let app = router(state.clone(), None);
    let body = json!({"name": "Personal Dev Team", "repoPath": repo.to_string_lossy()});
    let (s, p) = call(&app, "POST", "/api/projects", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().to_string(), repo }
}

async fn ticket(env: &Env, body: Value) -> String {
    let (s, v) = call_as_ui(&env.app, "POST", &format!("/api/projects/{}/tickets", env.pid), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

async fn default_branch(env: &Env, id: &str) -> String {
    let (s, v) = call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["defaultBranch"].as_str().unwrap().to_string()
}

/// Starts the ticket with no session; its attempt's branch.
async fn start(env: &Env, id: &str) -> String {
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{id}/start"),
        Some(json!({"agent": "claude", "noSession": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["attempt"]["branch"].as_str().unwrap().to_string()
}

async fn put(env: &Env, body: Value) -> (StatusCode, Value) {
    call(&env.app, "PUT", &format!("/api/projects/{}/branch-names", env.pid), Some(body)).await
}

async fn view(env: &Env) -> Value {
    let (s, v) = call(&env.app, "GET", &format!("/api/projects/{}/branch-names", env.pid), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

fn write_config(env: &Env, text: &str) {
    std::fs::create_dir_all(env.repo.join(".pdt")).unwrap();
    std::fs::write(env.repo.join(".pdt/project.toml"), text).unwrap();
}

#[tokio::test]
async fn without_a_template_branches_keep_their_names() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = ticket(&env, json!({"title": "Fix the login", "stage": "ready"})).await;
    assert_eq!(default_branch(&env, &id).await, "pdt-PDT-1-fix-the-login");
    let v = view(&env).await;
    assert_eq!((v["source"].as_str(), v["template"].as_str()), (Some("default"), Some("pdt-{key}-{slug}")));
    assert_eq!(v["previews"][0]["name"], "pdt-PDT-48-fix-the-login-crash");
    assert_eq!(v["previews"][2], json!({"label": "PDT-1 Fix the login", "name": "pdt-PDT-1-fix-the-login"}));
    assert_eq!(start(&env, &id).await, "pdt-PDT-1-fix-the-login");
}

#[tokio::test]
async fn the_setting_names_branches_with_the_tickets_links_and_labels() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (s, v) = put(&env, json!({"template": "{type}/{jira|issue|key}-{slug}", "types": {"chore": "chore"}})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["source"], "setting");
    assert_eq!(v["slash"], true);
    assert_eq!(v["types"], json!({"bug": "fix", "chore": "chore"}));
    let names: Vec<&str> = v["previews"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["fix/ABC-123-fix-the-login-crash", "feat/PDT-48-add-a-dark-theme"]);

    let bug = ticket(&env, json!({"title": "Login crash", "labels": ["bug"], "stage": "ready"})).await;
    assert_eq!(default_branch(&env, &bug).await, "fix/PDT-1-login-crash");
    let link = StoredExternalLink::new(
        &bug,
        "github",
        "acme/app",
        "acme/app#77",
        "https://github.com/acme/app/issues/77",
        LinkOrigin::Manual,
    );
    env.state.store.link_external(&link, pdt_core::ticket::Actor::System, 1).await.unwrap().unwrap();
    assert_eq!(default_branch(&env, &bug).await, "fix/77-login-crash");
    let jira = StoredExternalLink::new(
        &bug,
        "jira",
        "work",
        "ABC-9",
        "https://acme.atlassian.net/browse/ABC-9",
        LinkOrigin::Imported,
    );
    env.state.store.link_external(&jira, pdt_core::ticket::Actor::System, 1).await.unwrap().unwrap();
    assert_eq!(default_branch(&env, &bug).await, "fix/ABC-9-login-crash");
    let branch = start(&env, &bug).await;
    assert_eq!(branch, "fix/ABC-9-login-crash");
    assert!(env.repo.join(".worktrees/fix-ABC-9-login-crash").is_dir(), "a / in the branch is a - in the folder");

    // Linking later doesn't rename the started branch.
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{bug}"), None).await;
    assert_eq!(t["attempts"][0]["branch"], "fix/ABC-9-login-crash");

    // Back to the default.
    let (s, v) = put(&env, json!({"template": ""})).await;
    assert_eq!((s, v["source"].as_str()), (StatusCode::OK, Some("default")));
}

#[tokio::test]
async fn a_bad_template_is_refused_and_previewed_with_its_error() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (s, v) = put(&env, json!({"template": "{jira}"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["message"].as_str().unwrap().contains("empty name"), "{v}");
    let (s, v) = put(&env, json!({"template": "{nope}"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(view(&env).await["source"], "default", "nothing was saved");

    let uri = format!("/api/projects/{}/branch-names/preview", env.pid);
    let (s, v) = call(&env.app, "POST", &uri, Some(json!({"template": "{nope}"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["error"].as_str().unwrap().contains("{nope} is not a part"), "{v}");
    let (_, v) = call(&env.app, "POST", &uri, Some(json!({"template": "feature/{key}"}))).await;
    assert_eq!((v["error"].clone(), v["slash"].clone()), (Value::Null, json!(true)));
    assert_eq!(v["previews"][0]["name"], "feature/PDT-48");
}

#[tokio::test]
async fn the_repository_file_wins_and_a_bad_one_falls_back_to_the_default() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    put(&env, json!({"template": "mine/{key}"})).await;
    let id = ticket(&env, json!({"title": "Docs pass", "labels": ["docs"], "stage": "ready"})).await;
    assert_eq!(default_branch(&env, &id).await, "mine/PDT-1");

    write_config(&env, "[branch]\ntemplate = \"{type}-{key}-{slug}\"\ntypes = { docs = \"docs\" }\n");
    let v = view(&env).await;
    assert_eq!((v["source"].as_str(), v["template"].as_str()), (Some("file"), Some("{type}-{key}-{slug}")));
    assert_eq!(v["setting"]["template"], "mine/{key}", "the setting is kept");
    assert_eq!(default_branch(&env, &id).await, "docs-PDT-1-docs-pass");

    // A bad [branch] is reported and the default used; runs are unaffected.
    write_config(&env, "[branch]\ntemplate = \"{nope}\"\n");
    let v = view(&env).await;
    assert_eq!(v["source"], "file");
    assert!(v["fileError"].as_str().unwrap().contains("{nope} is not a part"), "{v}");
    assert_eq!(default_branch(&env, &id).await, "pdt-PDT-1-docs-pass");
    let (_, c) = call(&env.app, "GET", &format!("/api/projects/{}/run-config", env.pid), None).await;
    assert!(c["effective"]["branchError"].as_str().unwrap().contains("{nope}"), "{c}");

    // A file that doesn't parse leaves the setting.
    write_config(&env, "[[run]]\nname = \n");
    let v = view(&env).await;
    assert_eq!(v["source"], "setting");
    assert!(v["fileError"].is_string(), "{v}");
    assert_eq!(default_branch(&env, &id).await, "mine/PDT-1");
}

#[tokio::test]
async fn the_overrides_cannot_name_branches() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let body = json!({"agentCredentials": null, "runOverrides": "[branch]\ntemplate = \"x/{key}\"\n"});
    let (s, v) = call_as_ui(&env.app, "PUT", &format!("/api/projects/{}/settings", env.pid), Some(body)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["message"].as_str().unwrap().contains("Branch names setting"), "{v}");
}

#[tokio::test]
async fn a_taken_name_gets_a_number_and_a_tickets_own_branch_is_reused() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    put(&env, json!({"template": "{type}/{slug}"})).await;
    // Someone's branch has the name already, and the next one is on origin.
    git(&env.repo, &["branch", "feat/login"]);
    let upstream = tmp.path().join("upstream.git");
    git(tmp.path(), &["init", "-q", "--bare", "-b", "main", &upstream.to_string_lossy()]);
    git(&env.repo, &["remote", "add", "origin", &upstream.to_string_lossy()]);
    git(&env.repo, &["push", "-q", "origin", "main:feat/login-2"]);
    git(&env.repo, &["fetch", "-q", "origin"]);

    let a = ticket(&env, json!({"title": "Login", "stage": "ready"})).await;
    assert_eq!(default_branch(&env, &a).await, "feat/login");
    assert_eq!(start(&env, &a).await, "feat/login-3");
    let b = ticket(&env, json!({"title": "Login", "stage": "ready"})).await;
    assert_eq!(start(&env, &b).await, "feat/login-4", "another ticket's worktree has -3");

    // Abandoned, a ticket's kept worktree is still taken for others; started again, the ticket gets its branch back.
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{a}"), None).await;
    let attempt = t["attempts"][0]["id"].as_str().unwrap().to_string();
    // Uncommitted work keeps the worktree.
    std::fs::write(env.repo.join(".worktrees/feat-login-3/wip.txt"), "wip\n").unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{attempt}/abandon"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(env.repo.join(".worktrees/feat-login-3").is_dir(), "the abandoned attempt's worktree is kept");
    let c = ticket(&env, json!({"title": "Login", "stage": "ready"})).await;
    assert_eq!(start(&env, &c).await, "feat/login-5", "not the abandoned ticket's -3");
    assert_eq!(start(&env, &a).await, "feat/login-3");

    // A chat's worktree (made through the API) is taken too.
    let body = json!({"branch": "feat/chat"});
    let (s, v) = call(&env.app, "POST", &format!("/api/projects/{}/worktrees", env.pid), Some(body)).await;
    assert!(s.is_success(), "{v}");
    let d = ticket(&env, json!({"title": "Chat", "stage": "ready"})).await;
    assert_eq!(start(&env, &d).await, "feat/chat-2");
}

#[tokio::test]
async fn only_a_name_pdt_chose_is_recorded_as_pdts() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    put(&env, json!({"template": "{type}/{slug}"})).await;
    git(&env.repo, &["branch", "feat/login"]);
    let a = ticket(&env, json!({"title": "Login", "stage": "ready"})).await;
    // Given the template's name, Start takes it as given: someone's branch, not PDT's.
    let body = json!({"agent": "claude", "noSession": true, "branch": "feat/login"});
    let (s, v) = call(&env.app, "POST", &format!("/api/tickets/{a}/start"), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let attempt = v["attempt"]["id"].as_str().unwrap();
    let info = env.state.store.list_active_attempts().await.unwrap();
    let info = info.iter().find(|i| i.attempt.id == attempt).unwrap();
    assert_eq!((info.attempt.branch.as_str(), info.branch_named), ("feat/login", Some(false)));
}
