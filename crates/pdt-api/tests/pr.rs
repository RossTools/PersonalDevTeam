//! Pull requests and CI of ticket attempts (PDT-25), with a fake GitHub and a stand-in agent.

mod common;

use axum::http::StatusCode;
use common::{argv_agent, call, git};
use pdt_api::{AppState, router};
use pdt_core::pr::{CiRun, CiState, Mergeable, PrState, QueueRemoval};
use pdt_github::PullRequest;
use pdt_github::testing::FakeGithub;
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

const BRANCH: &str = "pdt-PDT-1-ship-it";

struct Env {
    app: axum::Router,
    state: AppState,
    gh: Arc<FakeGithub>,
    repo: std::path::PathBuf,
    ticket: String,
    session: String,
}

/// A project on GitHub (its origin says so; nothing is fetched) with one started ticket whose agent waits for input,
/// in a terminal.
async fn setup(tmp: &std::path::Path) -> Env {
    setup_as(tmp, "terminal").await
}

/// [`setup`], with the agent as a `terminal` or a `conversation`.
async fn setup_as(tmp: &std::path::Path, mode: &str) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    git(&repo, &["remote", "add", "origin", "https://github.com/o/r.git"]);
    // A merge check's fetch of origin fails at once rather than going to the network.
    git(&repo, &["config", "http.proxy", "http://127.0.0.1:1"]);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.platform.claude_config_dir = Some(tmp.join("claude-config"));
    state.platform.claude_program = if mode == "conversation" {
        env!("CARGO_BIN_EXE_fake-agent").to_string()
    } else {
        argv_agent(&bin, "fakeclaude").to_string_lossy().into_owned()
    };
    state.conversations.notice_quiet = Duration::ZERO;
    let gh = Arc::new(FakeGithub::default());
    state.github = gh.clone();
    let app = router(state.clone(), None);
    let (s, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "Personal Dev Team", "repoPath": repo.to_string_lossy()})),
    )
    .await;
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
    Env { app, state, gh, repo, ticket, session }
}

fn pr() -> PullRequest {
    PullRequest {
        number: 12,
        url: "https://github.com/o/r/pull/12".into(),
        title: "Ship it".into(),
        state: PrState::Open,
        draft: false,
        head_sha: "abcdef1234".into(),
        base_branch: "main".into(),
        mergeable: Mergeable::Mergeable,
        behind: false,
        auto_merge: true,
        in_merge_queue: false,
        queue_position: None,
        queue_removal: None,
    }
}

fn run(id: u64, state: CiState) -> CiRun {
    CiRun {
        id,
        attempt: 1,
        workflow: "ci".into(),
        state,
        url: format!("https://github.com/o/r/actions/runs/{id}"),
        head_sha: "abcdef1234".into(),
        head_branch: BRANCH.into(),
        created_at: "2026-10-18T00:00:00Z".into(),
        failed_jobs: vec![],
    }
}

async fn ticket(env: &Env) -> Value {
    call(&env.app, "GET", &format!("/api/tickets/{}", env.ticket), None).await.1
}

async fn pr_events(env: &Env) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{}/events", env.ticket), None).await;
    v["items"].as_array().unwrap().iter().filter(|e| e["kind"] == "pr").cloned().collect()
}

fn screen(env: &Env) -> String {
    String::from_utf8_lossy(&env.state.ptys.attach(&env.session).unwrap().scrollback).into_owned()
}

#[tokio::test]
async fn ci_shows_on_the_ticket_and_failures_are_told_to_the_agent_once() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;

    // No PR yet.
    pdt_api::refresh_prs(&env.state).await;
    assert!(ticket(&env).await["attempt"]["pr"].is_null());

    // CI running: shown, no news.
    env.gh.set_pr(BRANCH, Some(pr()));
    env.gh.set_runs(BRANCH, vec![run(7, CiState::Running)]);
    pdt_api::refresh_prs(&env.state).await;
    let t = ticket(&env).await;
    let p = &t["attempt"]["pr"];
    assert_eq!(p["number"], 12);
    assert_eq!(p["autoMerge"], true);
    assert_eq!(p["ci"]["state"], "running");
    assert_eq!(t["attempts"][0]["pr"]["number"], 12);
    let (_, list) = call(&env.app, "GET", "/api/tickets", None).await;
    assert_eq!(list["items"][0]["attempt"]["pr"]["ci"]["state"], "running", "the board's cards carry it too");
    pdt_api::run_workflows(&env.state).await;
    assert!(pr_events(&env).await.is_empty());

    // CI fails: the agent is told once, with the failed jobs.
    env.gh.set_runs(BRANCH, vec![run(7, CiState::Failed)]);
    env.gh.set_jobs(7, &["test (linux)"]);
    pdt_api::refresh_prs(&env.state).await;
    assert_eq!(ticket(&env).await["attempt"]["pr"]["ci"]["runs"][0]["failedJobs"][0], "test (linux)");
    pdt_api::run_workflows(&env.state).await;
    let events = pr_events(&env).await;
    assert_eq!(events.len(), 1, "{events:?}");
    let data = &events[0]["data"];
    assert_eq!(data["notice"], "ci_failed");
    assert_eq!(data["told"], true);
    assert_eq!(data["sessionId"], env.session.as_str());
    assert_eq!(data["pr"], 12);
    if cfg!(unix) {
        // The terminal echoes what was typed.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !screen(&env).contains("CI failed on PR #12") && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(screen(&env).contains("test (linux)"), "{}", screen(&env));
    }
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(pr_events(&env).await.len(), 1, "the same failure is news once");

    // The stand-in agent took its one line and exited; news with no running session is recorded as not told.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while env.state.ptys.is_running(&env.session) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut conflicting = pr();
    conflicting.mergeable = Mergeable::Conflicting;
    env.gh.set_pr(BRANCH, Some(conflicting));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let events = pr_events(&env).await;
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[1]["data"]["notice"], "conflict");
    assert_eq!(events[1]["data"]["told"], false);
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_removal_from_the_merge_queue_is_told_even_when_never_seen_queued() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    // Queued and dropped between two looks (a conflict is found within seconds): only the timeline knows.
    let mut removed = pr();
    removed.auto_merge = false;
    removed.queue_removal =
        Some(QueueRemoval { id: "RFMQE_1".into(), reason: "merge_conflict".into(), by_queue: true });
    env.gh.set_pr(BRANCH, Some(removed.clone()));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let events = pr_events(&env).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["data"]["notice"], "queue_left");
    assert_eq!(events[0]["data"]["key"], "queue_removed:RFMQE_1");
    assert_eq!(events[0]["data"]["told"], true);
    assert!(events[0]["data"]["text"].as_str().unwrap().contains("it conflicts with main"));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(pr_events(&env).await.len(), 1, "one removal is news once");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_busy_agent_is_told_later_and_a_merge_is_news() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    // Claude reports a turn starting: busy, so the notice waits.
    let (s, _) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{}/activity", env.session),
        Some(json!({"source": "claude", "event": "UserPromptSubmit"})),
    )
    .await;
    assert!(s.is_success(), "{s}");
    let mut merged = pr();
    merged.state = PrState::Merged;
    env.gh.set_pr(BRANCH, Some(merged));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    assert!(pr_events(&env).await.is_empty(), "waits while the agent works");
    let (s, _) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{}/activity", env.session),
        Some(json!({"source": "claude", "event": "Stop"})),
    )
    .await;
    assert!(s.is_success());
    pdt_api::run_workflows(&env.state).await;
    let events = pr_events(&env).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["data"]["notice"], "merged");
    assert_eq!(events[0]["data"]["told"], true);
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_ticket_workspace_carries_its_ticket_and_pr() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{}", env.ticket), None).await;
    let pid = t["projectId"].as_str().unwrap();
    let (_, t2) =
        call(&env.app, "POST", &format!("/api/projects/{pid}/tickets"), Some(json!({"title": "Own space"}))).await;
    let id2 = t2["id"].as_str().unwrap();
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{id2}/start"),
        Some(json!({"agent": "claude", "newWorkspace": true, "sessionMode": "terminal"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let mut p = pr();
    p.number = 13;
    p.mergeable = Mergeable::Conflicting;
    env.gh.set_pr("pdt-PDT-2-own-space", Some(p));
    pdt_api::refresh_prs(&env.state).await;
    let (_, list) = call(&env.app, "GET", "/api/workspaces", None).await;
    let list = list.as_array().unwrap();
    let mine: Vec<&Value> = list.iter().filter(|w| !w["ticket"].is_null()).collect();
    assert_eq!(mine.len(), 1, "only the workspace Start made: {list:?}");
    let ticket = &mine[0]["ticket"];
    assert_eq!(mine[0]["name"], "PDT-2");
    assert_eq!(ticket["key"], "PDT-2");
    assert_eq!(ticket["title"], "Own space");
    assert_eq!(ticket["stage"], "in_progress");
    assert_eq!(ticket["attemptState"], "active");
    assert_eq!(ticket["pr"]["number"], 13);
    assert_eq!(ticket["pr"]["mergeable"], "conflicting");
    assert!(
        list.iter().any(|w| w["projectId"].is_string() && w.get("ticket").is_none()),
        "project workspaces have none"
    );
    env.state.ptys.shutdown().await;
}

fn rev_parse(dir: &std::path::Path, rev: &str) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(["rev-parse", rev]).output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

async fn attempt_state(env: &Env) -> String {
    ticket(env).await["attempts"][0]["state"].as_str().unwrap().to_string()
}

/// The merge queue squashes a PR onto a newer base: when the base moved near the PR's edits, git's patch check finds
/// nothing, but GitHub's word that the PR merged at the attempt's head is enough (PDT-37).
#[tokio::test]
async fn a_pr_merged_on_github_ends_the_attempt_when_git_cannot_tell() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let wt = env.repo.join(".worktrees").join("pdt-PDT-1-ship-it");
    std::fs::write(wt.join("a.txt"), "two\n").unwrap();
    git(&wt, &["commit", "-q", "-am", "ship"]);
    let head = rev_parse(&wt, "HEAD");
    // Main changed the same line, as the queue's squash would sit on: the branch's patch is not in main.
    std::fs::write(env.repo.join("a.txt"), "uno\n").unwrap();
    git(&env.repo, &["commit", "-q", "-am", "other work"]);

    // Merged at another head (the branch moved on since): still active.
    let mut merged = pr();
    merged.state = PrState::Merged;
    env.gh.set_pr(BRANCH, Some(merged.clone()));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::check_merges(&env.state).await;
    assert_eq!(attempt_state(&env).await, "active");

    // Merged at the branch head: merged, by the loop.
    merged.head_sha = head.clone();
    env.gh.set_pr(BRANCH, Some(merged));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::check_merges(&env.state).await;
    assert_eq!(attempt_state(&env).await, "merged");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn opening_a_ticket_finds_its_pr_merged_on_github() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let wt = env.repo.join(".worktrees").join("pdt-PDT-1-ship-it");
    std::fs::write(wt.join("a.txt"), "two\n").unwrap();
    git(&wt, &["commit", "-q", "-am", "ship"]);
    std::fs::write(env.repo.join("a.txt"), "uno\n").unwrap();
    git(&env.repo, &["commit", "-q", "-am", "other work"]);
    let mut merged = pr();
    merged.state = PrState::Merged;
    merged.head_sha = rev_parse(&wt, "HEAD");
    env.gh.set_pr(BRANCH, Some(merged));
    // check-merge reads the PR first, then judges.
    let (s, t) = call(&env.app, "POST", &format!("/api/tickets/{}/check-merge", env.ticket), None).await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["attempts"][0]["state"], "merged");
    env.state.ptys.shutdown().await;
}

async fn conversation_events(env: &Env) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/sessions/{}/events?after=0", env.session), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut all = v.as_array().unwrap().clone();
    all.sort_by_key(|e| e["pos"].as_i64());
    all
}

/// Waits until the conversation has taken `n` turns.
async fn wait_turns(env: &Env, n: usize) -> Vec<Value> {
    for _ in 0..300 {
        let all = conversation_events(env).await;
        if all.iter().filter(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended").count() >= n {
            return all;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never saw {n} turns end: {:#?}", conversation_events(env).await);
}

#[tokio::test]
async fn a_conversation_is_told_as_pdt_once_it_is_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup_as(tmp.path(), "conversation").await;
    // Start sent the ticket as the first message.
    let all = wait_turns(&env, 1).await;
    let first = all.iter().find(|e| e["event"]["kind"] == "user_message").unwrap();
    assert!(first["event"]["text"].as_str().unwrap().starts_with("PDT-1: Ship it"), "{first}");
    assert_eq!(first["event"]["source"], "pdt");

    env.gh.set_pr(BRANCH, Some(pr()));
    env.gh.set_runs(BRANCH, vec![run(7, CiState::Failed)]);
    env.gh.set_jobs(7, &["test (linux)"]);
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let events = pr_events(&env).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!((&events[0]["data"]["told"], &events[0]["data"]["sessionId"]), (&json!(true), &json!(env.session)));
    let all = wait_turns(&env, 2).await;
    let told: Vec<&Value> = all.iter().filter(|e| e["event"]["kind"] == "user_message").collect();
    assert_eq!(told.len(), 2, "{all:#?}");
    assert_eq!(told[1]["event"]["source"], "notice", "shown as PDT's, not the user's");
    assert!(told[1]["event"]["text"].as_str().unwrap().contains("CI failed on PR #12"), "{}", told[1]);
    // Told once.
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(pr_events(&env).await.len(), 1);
}

/// Git's merge check can end the attempt in the tick GitHub still reports the PR queued: the next look reads the
/// ended attempt's PR again, so the badge says merged rather than "In queue" (PDT-59).
#[tokio::test]
async fn an_ended_attempts_pr_is_read_again_until_it_is_not_open() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let wt = env.repo.join(".worktrees").join("pdt-PDT-1-ship-it");
    std::fs::write(wt.join("a.txt"), "two\n").unwrap();
    git(&wt, &["commit", "-q", "-am", "ship"]);
    let mut queued = pr();
    queued.in_merge_queue = true;
    env.gh.set_pr(BRANCH, Some(queued));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::check_merges(&env.state).await;
    assert_eq!(attempt_state(&env).await, "active");

    // Main gets the branch while GitHub still says queued: git ends the attempt, the stale look stays.
    git(&env.repo, &["merge", "-q", "--ff-only", BRANCH]);
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::check_merges(&env.state).await;
    let t = ticket(&env).await;
    assert_eq!(t["attempts"][0]["state"], "merged");
    assert_eq!(t["attempts"][0]["pr"]["inMergeQueue"], true);

    // GitHub catches up: the next look reads it, and tells nothing new.
    pdt_api::run_workflows(&env.state).await;
    let told = pr_events(&env).await.len();
    assert_eq!(told, 1);
    let mut merged = pr();
    merged.state = PrState::Merged;
    env.gh.set_pr(BRANCH, Some(merged));
    pdt_api::refresh_prs(&env.state).await;
    let t = ticket(&env).await;
    assert_eq!(t["attempts"][0]["pr"]["state"], "merged");
    assert_eq!(t["attempts"][0]["pr"]["inMergeQueue"], false);
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(pr_events(&env).await.len(), told);
    env.state.ptys.shutdown().await;
}

fn comment(id: u64, author: &str, path: Option<&str>) -> pdt_github::PrComment {
    pdt_github::PrComment {
        id,
        author: author.into(),
        is_bot: author.ends_with("[bot]"),
        body: format!("comment {id}"),
        url: format!("https://github.com/o/r/pull/12#c{id}"),
        path: path.map(String::from),
        line: path.map(|_| 40),
        in_review: path.is_some(),
    }
}

#[tokio::test]
async fn pr_comments_and_reviews_reach_the_agent_once_the_workflow_is_on() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    pdt_api::run_workflows(&env.state).await;
    env.gh.set_pr(BRANCH, Some(pr()));
    env.gh.set_comments(12, vec![comment(1, "alice", None)]);
    // Off by default: the comments aren't even read.
    pdt_api::refresh_prs(&env.state).await;
    assert!(!env.gh.calls.lock().unwrap().iter().any(|c| c.starts_with("comments")));

    let (_, list) = call(&env.app, "GET", "/api/workflows", None).await;
    let id = list.as_array().unwrap().iter().find(|w| w["builtin"] == "pr-feedback").unwrap()["id"].clone();
    let id = id.as_str().unwrap();
    call(&env.app, "PATCH", &format!("/api/workflows/{id}"), Some(json!({"enabled": true}))).await;
    // The first look records what is there.
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    assert!(pr_events(&env).await.is_empty(), "an existing comment is not news");

    env.gh.set_comments(
        12,
        vec![
            comment(1, "alice", None),
            comment(2, "alice", None),
            comment(3, "me", None),
            comment(500, "ci[bot]", Some("src/a.rs")),
            comment(501, "bob", Some("src/a.rs")),
        ],
    );
    env.gh.set_reviews(
        12,
        vec![
            pdt_github::PrReview {
                id: 9,
                author: "bob".into(),
                is_bot: false,
                state: "changes_requested".into(),
                body: "Please split this.".into(),
                url: "https://github.com/o/r/pull/12#r9".into(),
            },
            pdt_github::PrReview {
                id: 10,
                author: "carol".into(),
                is_bot: false,
                state: "approved".into(),
                body: "LGTM".into(),
                url: "u".into(),
            },
        ],
    );
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::refresh_prs(&env.state).await;
    let (_, logged) = call(&env.app, "GET", "/api/events?type=pr.commented", None).await;
    assert_eq!(logged.as_array().unwrap().len(), 4, "each new comment once (mine and the bot's too): {logged}");
    pdt_api::run_workflows(&env.state).await;
    let mut keys: Vec<String> =
        pr_events(&env).await.iter().map(|e| e["data"]["key"].as_str().unwrap().into()).collect();
    keys.sort();
    assert_eq!(keys, ["comment:2", "comment:501", "review:9"], "not mine, not a bot's, not an approval");
    env.state.ptys.shutdown().await;
}
