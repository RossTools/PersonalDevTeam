//! Review (Phase 6): a worktree's changes, line comments, Send feedback into the coder session, Approve and Merge,
//! with `fake-agent` as `claude` and a fake GitHub.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_core::pr::{Mergeable, PrState};
use pdt_github::testing::FakeGithub;
use pdt_github::{MergeSettings, PullRequest};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

/// A stand-in terminal agent that reads input until it is killed (a pasted multi-line message must not end it).
fn reading_agent(dir: &Path) -> PathBuf {
    let (file, body) = if cfg!(windows) {
        ("fakeclaude.cmd", "@echo off\r\n:again\r\nset /p x=\r\ngoto again\r\n")
    } else {
        ("fakeclaude", "#!/bin/sh\nwhile true; do read x || sleep 1; done\n")
    };
    let path = dir.join(file);
    std::fs::write(&path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

struct Env {
    app: axum::Router,
    state: AppState,
    gh: Arc<FakeGithub>,
    repo: PathBuf,
    ticket: String,
    attempt: String,
    worktree: String,
    wt_path: PathBuf,
    session: String,
    branch: String,
}

/// A project with one started ticket whose agent is a `conversation` (fake-agent) or a `terminal` (waits for input).
async fn setup(tmp: &Path, mode: &str) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    // The local merge commits with the user's own identity.
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(repo.join("old.txt"), "old\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.platform.claude_config_dir = Some(tmp.join("claude-config"));
    state.platform.login_path = None;
    state.platform.claude_program =
        if mode == "conversation" { FAKE.to_string() } else { reading_agent(&bin).to_string_lossy().into_owned() };
    let gh = Arc::new(FakeGithub::default());
    state.github = gh.clone();
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Sample", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let pid = p["id"].as_str().unwrap();
    let (_, t) = call(&app, "POST", &format!("/api/projects/{pid}/tickets"), Some(json!({"title": "Ship it"}))).await;
    let ticket = t["id"].as_str().unwrap().to_string();
    let (s, started) = call(
        &app,
        "POST",
        &format!("/api/tickets/{ticket}/start"),
        Some(json!({"agent": "claude", "sessionMode": mode})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let session = started["session"]["id"].as_str().unwrap().to_string();
    let (_, t) = call(&app, "GET", &format!("/api/tickets/{ticket}"), None).await;
    let attempt = t["attempt"]["id"].as_str().unwrap().to_string();
    let worktree = t["attempt"]["worktreeId"].as_str().unwrap().to_string();
    let branch = t["attempt"]["branch"].as_str().unwrap().to_string();
    let (_, wts) = call(&app, "GET", &format!("/api/projects/{pid}/worktrees"), None).await;
    let wt_path = PathBuf::from(
        wts.as_array().unwrap().iter().find(|w| w["id"] == json!(worktree)).unwrap()["path"].as_str().unwrap(),
    );
    Env { app, state, gh, repo, ticket, attempt, worktree, wt_path, session, branch }
}

fn commit_all(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
}

async fn comment(env: &Env, body: Value) -> Value {
    let (s, c) = call(&env.app, "POST", &format!("/api/worktrees/{}/comments", env.worktree), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    c
}

#[tokio::test]
async fn changes_list_files_and_serve_both_sides() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "terminal").await;
    let w = &env.worktree;
    let (s, c) = call(&env.app, "GET", &format!("/api/worktrees/{w}/changes"), None).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["files"], json!([]));
    assert_eq!((c["base"].as_str(), c["dirty"].as_bool()), (Some("main"), Some(false)));
    assert_eq!(c["attemptId"], json!(env.attempt));
    assert_eq!(c["attemptActive"], json!(true));

    std::fs::write(env.wt_path.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    commit_all(&env.wt_path, "two");
    std::fs::remove_file(env.wt_path.join("old.txt")).unwrap();
    std::fs::write(env.wt_path.join("new.txt"), "new\n").unwrap();
    let (_, c) = call(&env.app, "GET", &format!("/api/worktrees/{w}/changes"), None).await;
    let files: Vec<(&str, &str, u64, u64)> = c["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap(),
                f["status"].as_str().unwrap(),
                f["added"].as_u64().unwrap(),
                f["removed"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(files, [("a.txt", "modified", 1, 1), ("new.txt", "added", 1, 0), ("old.txt", "deleted", 0, 1)]);
    assert_eq!(c["dirty"], json!(true));

    let (s, f) = call(&env.app, "GET", &format!("/api/worktrees/{w}/changes/file?path=a.txt"), None).await;
    assert_eq!(s, StatusCode::OK, "{f}");
    assert_eq!((f["old"].as_str(), f["new"].as_str()), (Some("one\ntwo\nthree\n"), Some("one\nTWO\nthree\n")));
    let (_, f) = call(&env.app, "GET", &format!("/api/worktrees/{w}/changes/file?path=new.txt"), None).await;
    assert_eq!((f["old"].clone(), f["new"].as_str()), (Value::Null, Some("new\n")));
    let (_, f) = call(&env.app, "GET", &format!("/api/worktrees/{w}/changes/file?path=old.txt"), None).await;
    assert_eq!((f["old"].as_str(), f["new"].clone()), (Some("old\n"), Value::Null));
    let (s, _) = call(&env.app, "GET", &format!("/api/worktrees/{w}/changes/file?path=../x"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "GET", "/api/worktrees/nope/changes", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn comments_are_drafts_until_sent_and_follow_the_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "terminal").await;
    let root =
        comment(&env, json!({"path": "a.txt", "side": "new", "line": 2, "lineText": "TWO", "body": "  Lower case?  "}))
            .await;
    assert_eq!((root["body"].as_str(), root["endLine"].as_u64()), (Some("Lower case?"), Some(2)));
    assert!(root["sentAt"].is_null());
    let rid = root["id"].as_str().unwrap();
    let reply =
        comment(&env, json!({"path": "x", "side": "old", "line": 9, "lineText": "", "body": "Also…", "parentId": rid}))
            .await;
    assert_eq!(
        (reply["path"].as_str(), reply["line"].as_u64(), reply["side"].as_str()),
        (Some("a.txt"), Some(2), Some("new"))
    );
    // Replying to a reply attaches to the root.
    let rr = comment(
        &env,
        json!({"path": "", "side": "new", "line": 1, "lineText": "", "body": "x", "parentId": reply["id"]}),
    )
    .await;
    assert_eq!(rr["parentId"], json!(rid));

    let (s, _) = call(
        &env.app,
        "POST",
        &format!("/api/worktrees/{}/comments", env.worktree),
        Some(json!({"path": "a.txt", "side": "new", "line": 3, "endLine": 2, "lineText": "", "body": "x"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a backwards range");
    let (s, _) = call(
        &env.app,
        "POST",
        &format!("/api/worktrees/{}/comments", env.worktree),
        Some(json!({"path": "a.txt", "side": "new", "line": 1, "lineText": "", "body": "  "})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "no text");

    let (s, e) =
        call(&env.app, "PATCH", &format!("/api/review-comments/{rid}"), Some(json!({"body": "Lower case."}))).await;
    assert_eq!((s, e["body"].as_str()), (StatusCode::OK, Some("Lower case.")));
    let (s, _) = call(
        &env.app,
        "PATCH",
        &format!("/api/review-comments/{}", reply["id"].as_str().unwrap()),
        Some(json!({"resolved": true})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "only roots resolve");
    let (s, r) = call(&env.app, "PATCH", &format!("/api/review-comments/{rid}"), Some(json!({"resolved": true}))).await;
    assert!(s == StatusCode::OK && r["resolvedAt"].is_number());
    let (s, _) = call(&env.app, "DELETE", &format!("/api/review-comments/{}", rr["id"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, all) = call(&env.app, "GET", &format!("/api/worktrees/{}/comments", env.worktree), None).await;
    assert_eq!(all.as_array().unwrap().len(), 2);
}

async fn user_messages(env: &Env) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{}/events?after=0", env.session), None).await;
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"]["kind"] == "user_message")
        .map(|e| e["event"]["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn send_feedback_puts_every_draft_into_the_conversation_once() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "conversation").await;
    let (s, e) = call(&env.app, "POST", &format!("/api/worktrees/{}/feedback", env.worktree), None).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("no_drafts")));

    let root =
        comment(&env, json!({"path": "a.txt", "side": "new", "line": 2, "lineText": "two", "body": "Make it TWO."}))
            .await;
    comment(
        &env,
        json!({"path": "b.rs", "side": "old", "line": 1, "endLine": 2, "lineText": "x\ny", "body": "Why gone?"}),
    )
    .await;
    let (s, r) = call(&env.app, "POST", &format!("/api/worktrees/{}/feedback", env.worktree), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!((r["sessionId"].as_str(), r["sent"].as_u64()), (Some(env.session.as_str()), Some(2)));

    let mut found = None;
    for _ in 0..200 {
        found = user_messages(&env).await.into_iter().find(|t| t.starts_with("Review feedback on"));
        if found.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let text = found.expect("the feedback reached the conversation");
    assert!(text.contains(&format!("Review feedback on {} (2 comments)", env.branch)), "{text}");
    assert!(text.contains("1. a.txt:2\n   > two\n   Make it TWO."), "{text}");
    assert!(text.contains("2. b.rs:1-2 (removed lines)"), "{text}");

    let (_, all) = call(&env.app, "GET", &format!("/api/worktrees/{}/comments", env.worktree), None).await;
    assert!(all.as_array().unwrap().iter().all(|c| c["sentAt"].is_number() && c["sessionId"] == json!(env.session)));
    let (s, e) = call(
        &env.app,
        "PATCH",
        &format!("/api/review-comments/{}", root["id"].as_str().unwrap()),
        Some(json!({"body": "changed"})),
    )
    .await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("comment_sent")));
    let (s, _) =
        call(&env.app, "DELETE", &format!("/api/review-comments/{}", root["id"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, ev) = call(&env.app, "GET", &format!("/api/tickets/{}/events", env.ticket), None).await;
    let fb = ev["items"].as_array().unwrap().iter().find(|e| e["kind"] == "feedback").cloned().unwrap();
    assert_eq!(fb["data"]["count"], json!(2));
    // The timeline keeps what was sent: the comments go with the worktree.
    let body = fb["body"].as_str().unwrap();
    assert!(body.starts_with("**`a.txt:2`**\n\n```\ntwo\n```\n\nMake it TWO."), "{body}");
    assert!(body.contains("**`b.rs:1-2`** (removed lines)"), "{body}");
}

#[tokio::test]
async fn feedback_is_typed_into_a_terminal_session() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "terminal").await;
    comment(&env, json!({"path": "a.txt", "side": "new", "line": 1, "lineText": "one", "body": "Fine."})).await;
    let (s, r) = call(&env.app, "POST", &format!("/api/worktrees/{}/feedback", env.worktree), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let screen = String::from_utf8_lossy(&env.state.ptys.attach(&env.session).unwrap().scrollback).into_owned();
    assert!(screen.contains("Review feedback on"), "{screen}");

    // Once the terminal is stopped there is nowhere to send to.
    call(&env.app, "POST", &format!("/api/sessions/{}/kill", env.session), None).await;
    for _ in 0..100 {
        if !env.state.ptys.is_running(&env.session) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    comment(&env, json!({"path": "a.txt", "side": "new", "line": 1, "lineText": "one", "body": "Again."})).await;
    let (s, e) = call(&env.app, "POST", &format!("/api/worktrees/{}/feedback", env.worktree), None).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("session_not_running")), "{e}");
}

#[tokio::test]
async fn approve_holds_for_the_head_it_was_given() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "terminal").await;
    std::fs::write(env.wt_path.join("a.txt"), "x\n").unwrap();
    commit_all(&env.wt_path, "x");
    let (s, _) = call(&env.app, "POST", &format!("/api/attempts/{}/approve", env.attempt), None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, c) = call(&env.app, "GET", &format!("/api/worktrees/{}/changes", env.worktree), None).await;
    assert_eq!(c["approved"], json!(true));
    std::fs::write(env.wt_path.join("a.txt"), "y\n").unwrap();
    commit_all(&env.wt_path, "y");
    let (_, c) = call(&env.app, "GET", &format!("/api/worktrees/{}/changes", env.worktree), None).await;
    assert_eq!(c["approved"], json!(false), "a new commit needs a new look");
}

async fn preview(env: &Env) -> Value {
    let (s, p) = call(&env.app, "GET", &format!("/api/attempts/{}/merge", env.attempt), None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    p
}

#[tokio::test]
async fn a_local_merge_checks_first_then_merges_and_ends_the_attempt() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "terminal").await;
    let p = preview(&env).await;
    assert_eq!(
        (p["method"].as_str(), p["problem"].as_str(), p["commits"].as_u64()),
        (Some("local"), Some("nothing"), Some(0))
    );

    std::fs::write(env.wt_path.join("a.txt"), "one\nmine\nthree\n").unwrap();
    assert_eq!(preview(&env).await["problem"], "dirty");
    commit_all(&env.wt_path, "mine");
    // main moves on with a clashing change: a conflict.
    std::fs::write(env.repo.join("a.txt"), "one\ntheirs\nthree\n").unwrap();
    commit_all(&env.repo, "theirs");
    let p = preview(&env).await;
    assert_eq!((p["problem"].as_str(), p["conflicts"].clone()), (Some("conflict"), json!(["a.txt"])));
    let (s, e) = call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), None).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("merge_blocked")));
    let (s, r) = call(&env.app, "POST", &format!("/api/attempts/{}/ask-agent", env.attempt), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let screen = String::from_utf8_lossy(&env.state.ptys.attach(&env.session).unwrap().scrollback).into_owned();
    assert!(screen.contains("conflicts in these files"), "{screen}");

    // The agent resolves it.
    git(&env.wt_path, &["merge", "-q", "main", "-X", "ours", "-m", "merge main"]);
    let p = preview(&env).await;
    assert_eq!((p["problem"].clone(), p["commits"].as_u64()), (Value::Null, Some(2)));
    std::fs::write(env.repo.join("scratch.txt"), "dirty\n").unwrap();
    assert_eq!(preview(&env).await["problem"], "base_dirty");
    std::fs::remove_file(env.repo.join("scratch.txt")).unwrap();

    let (s, r) = call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["outcome"], "merged");
    let out =
        std::process::Command::new("git").current_dir(&env.repo).args(["log", "-1", "--format=%s"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("Merge {}: Ship it", env.branch));
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{}", env.ticket), None).await;
    assert_eq!(t["attempt"]["state"], "merged");
    assert_eq!(preview(&env).await["problem"], "inactive");
}

#[tokio::test]
async fn a_local_merge_can_squash() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "terminal").await;
    std::fs::write(env.wt_path.join("a.txt"), "one\nmine\nthree\n").unwrap();
    commit_all(&env.wt_path, "mine");
    std::fs::write(env.wt_path.join("c.txt"), "c\n").unwrap();
    commit_all(&env.wt_path, "c");
    let (s, e) =
        call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), Some(json!({"style": "nope"}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));

    let (s, r) =
        call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), Some(json!({"style": "squash"}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["outcome"], "merged");
    let log = |args: &[&str]| {
        let out = std::process::Command::new("git").current_dir(&env.repo).args(args).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{}", env.ticket), None).await;
    let key = t["key"].as_str().unwrap();
    assert_eq!(log(&["log", "-1", "--format=%B"]), format!("{key}: Ship it\n\nSquashed from {}.", env.branch));
    assert_eq!(log(&["rev-list", "--parents", "-n1", "HEAD"]).split(' ').count(), 2, "one parent");
    assert_eq!(std::fs::read_to_string(env.repo.join("c.txt")).unwrap(), "c\n");
    assert_eq!(t["attempt"]["state"], "merged");
}

#[tokio::test]
async fn with_an_open_pr_merge_turns_on_auto_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "terminal").await;
    git(&env.repo, &["remote", "add", "origin", "https://github.com/o/r.git"]);
    git(&env.repo, &["config", "http.proxy", "http://127.0.0.1:1"]);
    let mut pr = PullRequest {
        number: 12,
        url: "https://github.com/o/r/pull/12".into(),
        title: "Ship it".into(),
        state: PrState::Open,
        draft: false,
        head_sha: "abc".into(),
        base_branch: "main".into(),
        mergeable: Mergeable::Mergeable,
        behind: false,
        auto_merge: false,
        in_merge_queue: false,
        queue_position: None,
        queue_removal: None,
    };
    env.gh.set_pr(&env.branch, Some(pr.clone()));
    pdt_api::refresh_prs(&env.state).await;
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{}", env.ticket), None).await;
    assert_eq!(t["attempt"]["pr"]["number"], json!(12), "{t}");
    *env.gh.settings.lock().unwrap() = Some(MergeSettings { squash: true, merge: true, rebase: false, queue: true });
    let p = preview(&env).await;
    assert_eq!(
        (p["method"].as_str(), p["pr"]["number"].as_u64(), p["pr"]["queue"].as_bool()),
        (Some("github"), Some(12), Some(true))
    );
    assert!(p["problem"].is_null(), "{p}");
    let (s, r) = call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), None).await;
    assert_eq!((s, r["outcome"].as_str()), (StatusCode::OK, Some("queued")), "{r}");
    assert!(env.gh.calls.lock().unwrap().contains(&"merge o/r 12 -".to_string()), "a merge queue picks the method");

    pr.auto_merge = true;
    env.gh.set_pr(&env.branch, Some(pr.clone()));
    pdt_api::refresh_prs(&env.state).await;
    assert_eq!(preview(&env).await["problem"], "pr_queued");
    pr.auto_merge = false;
    pr.mergeable = Mergeable::Conflicting;
    env.gh.set_pr(&env.branch, Some(pr));
    pdt_api::refresh_prs(&env.state).await;
    assert_eq!(preview(&env).await["problem"], "pr_not_mergeable");
    let t: Value = call(&env.app, "GET", &format!("/api/tickets/{}", env.ticket), None).await.1;
    assert_eq!(t["attempt"]["state"], "active", "GitHub merges; the merge check ends the attempt later");
}

#[tokio::test]
async fn a_changes_tab_opens_for_a_worktree_and_only_closes() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "terminal").await;
    let (_, ws) = call(&env.app, "GET", "/api/workspaces", None).await;
    let wid = ws[0]["id"].as_str().unwrap();
    let (s, tab) =
        call(&env.app, "POST", &format!("/api/workspaces/{wid}/tabs"), Some(json!({"worktreeId": env.worktree}))).await;
    assert_eq!(s, StatusCode::OK, "{tab}");
    assert_eq!(tab["worktreeId"], json!(env.worktree));
    let (s, _) =
        call(&env.app, "POST", &format!("/api/workspaces/{wid}/tabs"), Some(json!({"worktreeId": "nope"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "DELETE", &format!("/api/tabs/{}", tab["id"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, c) = call(&env.app, "GET", &format!("/api/worktrees/{}/changes", env.worktree), None).await;
    assert_eq!(c["worktreeId"], json!(env.worktree), "the worktree is still there");
}

#[tokio::test]
async fn a_project_that_merges_through_prs_never_merges_locally() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "terminal").await;
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{}", env.ticket), None).await;
    let pid = t["projectId"].as_str().unwrap().to_string();
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{pid}/settings"),
        Some(json!({"agentCredentials": null, "runOverrides": null, "mergeViaPr": true})),
    )
    .await;
    assert_eq!((s, v["mergeViaPr"].as_bool()), (StatusCode::OK, Some(true)), "{v}");
    // A PUT that leaves it out keeps it.
    let (_, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{pid}/settings"),
        Some(json!({"agentCredentials": null, "runOverrides": null})),
    )
    .await;
    assert_eq!(v["mergeViaPr"], json!(true));

    std::fs::write(env.wt_path.join("a.txt"), "changed\n").unwrap();
    commit_all(&env.wt_path, "change");
    let p = preview(&env).await;
    assert_eq!((p["method"].as_str(), p["problem"].as_str()), (Some("github"), Some("no_pr")), "{p}");
    let (s, e) = call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), None).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("merge_blocked")));
    let (s, r) = call(&env.app, "POST", &format!("/api/attempts/{}/ask-agent", env.attempt), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let screen = String::from_utf8_lossy(&env.state.ptys.attach(&env.session).unwrap().scrollback).into_owned();
    assert!(screen.contains("open a pull request against main"), "{screen}");
    let out =
        std::process::Command::new("git").current_dir(&env.repo).args(["log", "-1", "--format=%s"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "init", "main was not touched");
}
