//! `POST /tickets/{id}/stop` (PDT-159), and what it deletes: with `removeWorktree` the worktree goes, uncommitted
//! changes and all; with `deleteBranch` too, the branch with its commits (PDT-446). Without them nothing on disk goes.

use crate::common;

use axum::http::StatusCode;
use common::{argv_agent, call, call_as_ui, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
    repo: PathBuf,
}

/// A project (key PDT) whose repo has one commit on main, with a stand-in `claude`.
async fn setup(tmp: &Path) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp);
    state.platform.claude_program = argv_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    let app = router(state.clone(), None);
    let body = json!({"name": "Personal Dev Team", "repoPath": repo.to_string_lossy()});
    let (s, p) = call(&app, "POST", "/api/projects", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().to_string(), repo }
}

/// A ticket started with `body` (no session unless it says): its id, branch and worktree.
async fn started(env: &Env, title: &str, body: Value) -> (String, String, PathBuf) {
    let (s, t) = call_as_ui(
        &env.app,
        "POST",
        &format!("/api/projects/{}/tickets", env.pid),
        Some(json!({"title": title, "stage": "ready"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap().to_string();
    let mut body = body;
    body["agent"] = json!("claude");
    if body.get("sessionMode").is_none() {
        body["noSession"] = json!(true);
    }
    let (s, r) = call(&env.app, "POST", &format!("/api/tickets/{id}/start"), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let branch = r["attempt"]["branch"].as_str().unwrap().to_string();
    let wt = env.repo.join(".worktrees").join(&branch);
    assert!(wt.is_dir());
    (id, branch, wt)
}

/// Commits a file in the worktree (work only this branch holds) and leaves another uncommitted.
fn unsaved_work(wt: &Path) {
    std::fs::write(wt.join("done.txt"), "committed\n").unwrap();
    git(wt, &["add", "-A"]);
    git(wt, &["commit", "-q", "-m", "work"]);
    std::fs::write(wt.join("wip.txt"), "uncommitted\n").unwrap();
}

async fn stop(env: &Env, id: &str, body: Value) -> Value {
    let (s, r) = call(&env.app, "POST", &format!("/api/tickets/{id}/stop"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    r
}

async fn ticket(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await.1
}

async fn worktrees(env: &Env) -> Vec<Value> {
    call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.pid), None).await.1.as_array().unwrap().clone()
}

/// `removeWorktree` with `deleteBranch`: the session stops and goes, the attempt is abandoned and the worktree,
/// branch and workspace are removed, even with uncommitted changes and commits found nowhere else.
#[tokio::test]
async fn stop_with_delete_branch_removes_the_worktree_and_the_branch_with_its_commits() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (id, branch, wt) = started(&env, "Throw away", json!({"sessionMode": "terminal", "newWorkspace": true})).await;
    unsaved_work(&wt);
    let r = stop(&env, &id, json!({"removeWorktree": true, "deleteBranch": true})).await;
    assert_eq!(r["key"], "PDT-1");
    assert_eq!(r["stage"], "todo", "an in-progress ticket goes back to To do");
    assert!(r["abandoned"].is_string(), "{r}");
    assert_eq!((r["sessionsStopped"].as_u64(), r["worktreeRemoved"].as_bool()), (Some(1), Some(true)), "{r}");
    assert_eq!((r["workspaceRemoved"].as_bool(), &r["branchNote"]), (Some(true), &Value::Null), "{r}");
    assert!(!wt.exists());
    assert!(!pdt_git::branch_exists(&env.repo, &branch), "the branch is deleted");
    assert!(worktrees(&env).await.is_empty());
    let t = ticket(&env, &id).await;
    assert_eq!(t["attempts"][0]["state"], "abandoned", "{t}");
    let (_, sessions) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.pid), None).await;
    assert_eq!(sessions, json!([]), "the worktree's session goes with it");
    // main is untouched.
    assert_eq!(std::fs::read_to_string(env.repo.join("a.txt")).unwrap(), "one\n");
    assert!(!env.repo.join("done.txt").exists());

    // A new start begins clean, on a new branch from main.
    let (s, r) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{id}/start"),
        Some(json!({"agent": "claude", "noSession": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert!(!wt.join("done.txt").exists() && !wt.join("wip.txt").exists());
    env.state.ptys.shutdown().await;
}

/// `removeWorktree` alone keeps the branch and its commits; `deleteBranch` alone deletes nothing.
#[tokio::test]
async fn delete_branch_needs_remove_worktree_and_remove_worktree_alone_keeps_the_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (id, branch, wt) = started(&env, "Keep my commits", json!({})).await;
    unsaved_work(&wt);
    let head = pdt_git::rev_parse(&env.repo, &format!("refs/heads/{branch}")).unwrap();

    // deleteBranch without removeWorktree: the work stops, everything on disk stays.
    let r = stop(&env, &id, json!({"deleteBranch": true})).await;
    assert_eq!((r["stage"].as_str(), r["abandoned"].is_string()), (Some("todo"), true), "{r}");
    assert_eq!(r["worktreeRemoved"], false, "{r}");
    assert!(wt.join("wip.txt").is_file(), "uncommitted changes kept");
    assert!(pdt_git::branch_exists(&env.repo, &branch));
    assert_eq!(worktrees(&env).await.len(), 1);

    // removeWorktree: the worktree (an earlier stop's) goes, uncommitted changes too; the branch stays at its commit.
    let r = stop(&env, &id, json!({"removeWorktree": true})).await;
    assert_eq!((r["abandoned"].is_null(), r["worktreeRemoved"].as_bool()), (true, Some(true)), "{r}");
    assert_eq!(r["branchNote"], Value::Null);
    assert!(!wt.exists());
    assert_eq!(pdt_git::rev_parse(&env.repo, &format!("refs/heads/{branch}")).as_deref(), Some(head.as_str()));
    assert!(worktrees(&env).await.is_empty());
}

/// A branch git won't delete (checked out in a worktree PDT doesn't know of): the worktree still goes, the branch
/// stays and `branchNote` says why.
#[tokio::test]
async fn a_branch_that_will_not_delete_is_kept_and_said() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (id, branch, wt) = started(&env, "Shared branch", json!({})).await;
    let other = tmp.path().join("other");
    git(&env.repo, &["worktree", "add", "-q", "-f", &other.to_string_lossy(), &branch]);
    let r = stop(&env, &id, json!({"removeWorktree": true, "deleteBranch": true})).await;
    assert_eq!(r["worktreeRemoved"], true, "{r}");
    let note = r["branchNote"].as_str().unwrap_or_else(|| panic!("a note: {r}"));
    assert!(note.contains("not deleted"), "{note}");
    assert!(!wt.exists());
    assert!(other.is_dir(), "the other worktree is not PDT's to remove");
    assert!(pdt_git::branch_exists(&env.repo, &branch));
}

/// A ticket with no attempt: the stage moves as asked, nothing else happens; an unknown ticket is a 404.
#[tokio::test]
async fn stopping_a_ticket_without_work_only_moves_it() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (s, t) = call_as_ui(
        &env.app,
        "POST",
        &format!("/api/projects/{}/tickets", env.pid),
        Some(json!({"title": "Idle", "stage": "todo"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap();
    let r = stop(&env, id, json!({"stage": "draft", "removeWorktree": true, "deleteBranch": true})).await;
    assert_eq!(r["stage"], "draft");
    assert_eq!((r["abandoned"].is_null(), r["worktreeRemoved"].as_bool()), (true, Some(false)), "{r}");
    assert_eq!((r["sessionsStopped"].as_u64(), r["runsCancelled"].as_u64()), (Some(0), Some(0)));
    // No body at all is a stop with the defaults.
    let (s, r) = call(&env.app, "POST", &format!("/api/tickets/{id}/stop"), None).await;
    assert_eq!((s, r["stage"].as_str()), (StatusCode::OK, Some("draft")), "{r}");
    let (s, r) = call(&env.app, "POST", "/api/tickets/nope/stop", Some(json!({}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{r}");
    assert!(pdt_git::branch_exists(&env.repo, "main"));
}
