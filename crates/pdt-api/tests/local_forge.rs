//! The local forge (PDT-178): a project with no remote, `forge = "local"` and a `[ci]` command. A started ticket's
//! commits open a change request, CI runs in its worktree, a failure is news for the agent, and once it is set to merge
//! and CI passes PDT merges it into the base itself. Nothing touches the network: there is no remote, and the GitHub
//! forge is the one that runs nothing.

mod common;

use axum::http::StatusCode;
use common::{argv_agent, call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

struct Env {
    app: axum::Router,
    state: AppState,
    repo: PathBuf,
    worktree: PathBuf,
    ticket: String,
    attempt: String,
}

/// A repository with no remote on the local forge, whose CI passes once `ok.txt` is committed, and one started ticket.
async fn setup(tmp: &Path, config: &str) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join(".pdt/project.toml"), config).unwrap();
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.platform.claude_config_dir = Some(tmp.join("claude-config"));
    state.platform.claude_program = argv_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    tokio::spawn(pdt_api::run_local_ci_watch(state.clone()));
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Offline", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let pid = p["id"].as_str().unwrap();
    let (_, t) = call(&app, "POST", &format!("/api/projects/{pid}/tickets"), Some(json!({"title": "Ship it"}))).await;
    let ticket = t["id"].as_str().unwrap().to_string();
    let (s, started) = call(
        &app,
        "POST",
        &format!("/api/tickets/{ticket}/start"),
        Some(json!({"agent": "claude", "sessionMode": "terminal"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let attempt = started["attempt"]["id"].as_str().unwrap().to_string();
    let wt = started["attempt"]["worktreeId"].as_str().unwrap();
    let worktree = PathBuf::from(state.store.get_worktree(wt).await.unwrap().unwrap().path);
    Env { app, state, repo, worktree, ticket, attempt }
}

const CI: &str = "forge = \"local\"\n\n[ci]\ncommand = \"git ls-files --error-unmatch ok.txt\"\n";

fn commit(dir: &Path, file: &str, msg: &str) {
    std::fs::write(dir.join(file), format!("{msg}\n")).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
}

fn rev(dir: &Path, rev: &str) -> String {
    let out = std::process::Command::new("git").current_dir(dir).args(["rev-parse", rev]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

async fn ticket(env: &Env) -> Value {
    call(&env.app, "GET", &format!("/api/tickets/{}", env.ticket), None).await.1
}

/// The ticket once `done` says so (polling, as CI runs on its own), or the last look.
async fn wait_for(env: &Env, done: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..300 {
        let t = ticket(env).await;
        if done(&t) {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let t = ticket(env).await;
    panic!("timed out; the ticket is {t:#}")
}

async fn events(env: &Env, kind: &str) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/events?type={kind}"), None).await;
    v.as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn a_ticket_goes_through_local_ci_to_a_local_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), CI).await;
    // Nothing of its own yet: no change request.
    pdt_api::refresh_prs(&env.state).await;
    assert!(ticket(&env).await["attempt"]["pr"].is_null());

    // A commit without ok.txt: change request #1, and CI fails.
    commit(&env.worktree, "work.txt", "work");
    pdt_api::refresh_prs(&env.state).await;
    let t = wait_for(&env, |t| t["attempt"]["pr"]["ci"]["state"] == "failed").await;
    let pr = &t["attempt"]["pr"];
    assert_eq!(
        (pr["number"].as_u64(), pr["local"].as_bool(), pr["state"].as_str()),
        (Some(1), Some(true), Some("open"))
    );
    assert_eq!(pr["title"], format!("{}: Ship it", t["key"].as_str().unwrap()));
    let url = pr["ci"]["url"].as_str().unwrap().to_string();
    let path = url.split_once("/api/").map(|(_, p)| format!("/api/{p}")).unwrap();
    let (s, ct, body) = common::call_raw(&env.app, "GET", &path, vec![]).await;
    assert_eq!((s, ct.as_deref()), (StatusCode::OK, Some("text/plain; charset=utf-8")));
    let body = String::from_utf8_lossy(&body).into_owned();
    assert!(body.contains("$ git ls-files --error-unmatch ok.txt") && body.contains("exited with"), "{body}");
    let failed = events(&env, "ci.run_failed").await;
    assert_eq!(failed.len(), 1, "{failed:#?}");
    let notice = failed[0]["payload"]["notice"]["text"].as_str().unwrap();
    assert!(notice.contains("change request #1") && notice.contains(&format!("curl -s {url}")), "{notice}");

    // Merge…: set to merge, it waits for CI.
    let (s, r) =
        call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), Some(json!({"by": "user"}))).await;
    assert_eq!((s, r["outcome"].as_str()), (StatusCode::OK, Some("queued")), "{r}");
    assert_eq!(ticket(&env).await["attempt"]["pr"]["autoMerge"], true);
    let base = rev(&env.repo, "main");

    // The fix: CI runs again on the new head, passes, and PDT merges it (a fast-forward) and ends the attempt.
    commit(&env.worktree, "ok.txt", "fix");
    let head = rev(&env.worktree, "HEAD");
    pdt_api::refresh_prs(&env.state).await;
    let t = wait_for(&env, |t| t["attempt"]["state"] == "merged").await;
    assert_eq!(rev(&env.repo, "main"), head, "main moved to the branch head");
    assert_ne!(head, base);
    assert!(env.repo.join("ok.txt").is_file(), "the checkout of main was moved too");
    assert_eq!(t["attempt"]["pr"]["state"], "merged", "{t:#}");
    // Merged in the read that saw CI pass (as auto-merge on a forge): the merge is the news.
    let merged = events(&env, "pr.merged").await;
    assert_eq!(merged.len(), 1, "{merged:#?}");
    assert!(merged[0]["payload"]["notice"]["text"].as_str().unwrap().contains("change request #1 (pdt-"));
}

#[tokio::test]
async fn without_a_ci_command_merge_is_at_once_and_a_conflict_goes_back_to_the_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "forge = \"local\"\n").await;
    commit(&env.worktree, "a.txt", "mine");
    commit(&env.repo, "a.txt", "theirs");
    pdt_api::refresh_prs(&env.state).await;
    let t = ticket(&env).await;
    let pr = &t["attempt"]["pr"];
    assert_eq!((pr["mergeable"].as_str(), pr["ci"].is_null()), (Some("conflicting"), true), "{t:#}");
    let conflict = events(&env, "pr.conflict").await;
    let text = conflict[0]["payload"]["notice"]["text"].as_str().unwrap();
    assert!(text.contains("change request #1") && text.contains("`git merge main`"), "{text}");
    let (s, r) =
        call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{r}");

    // The agent merges main in and resolves it: no CI, so Merge merges now, with a merge commit (main moved).
    git(&env.worktree, &["merge", "-q", "-s", "ours", "-m", "merge main", "main"]);
    pdt_api::refresh_prs(&env.state).await;
    let (s, r) =
        call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), Some(json!({"by": "user"}))).await;
    assert_eq!((s, r["outcome"].as_str()), (StatusCode::OK, Some("merged")), "{r}");
    assert_eq!(ticket(&env).await["attempt"]["state"], "merged");
    assert_eq!(rev(&env.repo, "main"), rev(&env.worktree, "HEAD"), "a fast-forward to the branch's merge");
}
