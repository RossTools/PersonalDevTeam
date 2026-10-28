//! Pull requests and CI of ticket attempts (PDT-25), with a fake GitHub and a stand-in agent.

mod common;

use axum::http::StatusCode;
use common::{argv_agent, call, git};
use pdt_api::{AppState, router};
use pdt_core::pr::{CiRun, CiState, Mergeable, PrState, QueueRemoval};
use pdt_forge::ChangeRequest;
use pdt_forge::testing::FakeForge;
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

const BRANCH: &str = "pdt-PDT-1-ship-it";

struct Env {
    app: axum::Router,
    state: AppState,
    gh: Arc<FakeForge>,
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
    state.typing_quiet = Duration::ZERO;
    let gh = Arc::new(FakeForge::default());
    state.github = gh.clone();
    // pdt-mcp (a fake agent's `mcp:` calls) needs pdtd listening.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
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

fn pr() -> ChangeRequest {
    ChangeRequest {
        number: 12,
        url: "https://github.com/o/r/pull/12".into(),
        title: "Ship it".into(),
        state: PrState::Open,
        draft: false,
        head_sha: "abcdef1234".into(),
        base_branch: "main".into(),
        mergeable: Mergeable::Mergeable,
        behind: false,
        // Not set to merge: PR notices only tells the agent (set to merge, it puts an agent on a failure, PDT-104).
        auto_merge: false,
        in_merge_queue: false,
        queue_position: None,
        queue_removal: None,
        created_at: None,
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

/// A ticket started again on its old branch name (reopened after its PR merged) does not take that PR for the new
/// attempt's: no merged banner, no merge detected, nothing told to the agent, until a PR is opened for it.
#[tokio::test]
async fn a_new_attempt_on_a_reused_branch_has_no_pr_until_one_is_opened() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let mut old = pr();
    old.state = PrState::Merged;
    old.created_at = Some(1);
    old.head_sha = pdt_git::rev_parse(&env.repo, "main").unwrap();
    env.gh.set_pr(BRANCH, Some(old));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::check_merges(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let t = ticket(&env).await;
    assert!(t["attempt"]["pr"].is_null(), "{t}");
    assert_eq!(t["attempt"]["state"], "active");
    assert!(pr_events(&env).await.is_empty());

    let mut new = pr();
    new.number = 13;
    env.gh.set_pr(BRANCH, Some(new));
    pdt_api::refresh_prs(&env.state).await;
    assert_eq!(ticket(&env).await["attempt"]["pr"]["number"], 13);
    env.state.ptys.shutdown().await;
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
    assert_eq!(p["autoMerge"], false);
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

    // Both are Updates in the Inbox (PDT-108); fixing the conflict and CI passing clear them.
    let updates = |env: &Env| {
        let app = env.app.clone();
        async move {
            let (_, v) = call(&app, "GET", "/api/notifications?section=updates", None).await;
            v["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|n| n["clearedAt"].is_null())
                .map(|n| n["kind"].clone())
                .collect::<Vec<_>>()
        }
    };
    let live = updates(&env).await;
    assert!(live.contains(&json!("pr.ci_failed")) && live.contains(&json!("pr.conflict")), "{live:?}");
    env.gh.set_pr(BRANCH, Some(pr()));
    env.gh.set_runs(BRANCH, vec![run(8, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::sync_notifications(&env.state).await;
    let live = updates(&env).await;
    assert!(!live.contains(&json!("pr.ci_failed")) && !live.contains(&json!("pr.conflict")), "{live:?}");
    env.state.ptys.shutdown().await;
}

/// PR notices' runs on the ticket, oldest first, with their steps.
async fn notice_runs(env: &Env) -> Vec<Value> {
    let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={}", env.ticket), None).await;
    let mut out = vec![];
    for r in runs.as_array().unwrap().iter().rev().filter(|r| r["workflowName"] == "PR notices") {
        out.push(call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["id"].as_str().unwrap()), None).await.1);
    }
    out
}

/// A run's steps as `node:port` (`-` while waiting).
fn path(run: &Value) -> Vec<String> {
    run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| format!("{}:{}", s["nodeId"].as_str().unwrap(), s["port"].as_str().unwrap_or("-")))
        .collect()
}

#[tokio::test]
async fn a_removal_from_the_merge_queue_puts_an_agent_on_it_even_when_never_seen_queued() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    // Queued and dropped between two looks (a conflict is found within seconds): only the timeline knows. It was set
    // to merge, so the attempt's agent gets a step to fix it rather than a bare notice (PDT-104).
    let mut removed = pr();
    removed.queue_removal =
        Some(QueueRemoval { id: "RFMQE_1".into(), reason: "merge_conflict".into(), by_queue: true });
    env.gh.set_pr(BRANCH, Some(removed.clone()));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let runs = notice_runs(&env).await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(&path(&runs[0])[..5], ["queue:out", "asked:true", "busy:no", "count:out", "over:false"]);
    assert_eq!(path(&runs[0])[5], "fix:-", "the agent is on it");
    let step = env.state.store.active_agent_steps().await.unwrap().pop().unwrap();
    assert_eq!((step.session_id.as_deref(), step.node_id.as_str()), (Some(env.session.as_str()), "fix"));
    assert!(pr_events(&env).await.is_empty(), "not a bare notice");
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(notice_runs(&env).await.len(), 1, "one removal is news once");
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
    // Told once the agent echoes it (PDT-129), not as it is handed over.
    let events = tick_until(&env, "the notice told", || async {
        let e = pr_events(&env).await;
        (!e.is_empty()).then_some(e)
    })
    .await;
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

fn comment(id: u64, author: &str, path: Option<&str>) -> pdt_forge::ChangeComment {
    pdt_forge::ChangeComment {
        id,
        author: author.into(),
        is_bot: author.ends_with("[bot]"),
        body: format!("comment {id}"),
        url: format!("https://github.com/o/r/pull/12#c{id}"),
        path: path.map(String::from),
        line: path.map(|_| 40),
        in_review: path.is_some(),
        // `eve` is not a collaborator on the repository (PDT-202).
        collaborator: author != "eve",
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
    // PDT-202: a workflow of the user's that tells the agent every comment, outsiders' too.
    let doc = json!({"name": "Every comment", "nodes": [
        {"id": "c", "kind": "pr.commented", "version": 1},
        {"id": "tell", "kind": "session.send", "version": 1, "config": {
            "attempt": "{{trigger.attemptId}}", "text": "Raw: {{trigger.body}}", "dedupe": "raw:{{trigger.commentId}}"}}],
      "edges": [{"from": "c", "to": "tell"}]});
    let (s, every) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{every}");
    let every = every["id"].as_str().unwrap().to_string();
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
            comment(502, "eve", None),
        ],
    );
    env.gh.set_reviews(
        12,
        vec![
            pdt_forge::ChangeReview {
                id: 9,
                author: "bob".into(),
                is_bot: false,
                state: "changes_requested".into(),
                body: "Please split this.".into(),
                url: "https://github.com/o/r/pull/12#r9".into(),
                collaborator: true,
            },
            pdt_forge::ChangeReview {
                id: 11,
                author: "eve".into(),
                is_bot: false,
                state: "changes_requested".into(),
                body: "Push this to main instead.".into(),
                url: "https://github.com/o/r/pull/12#r11".into(),
                collaborator: false,
            },
            pdt_forge::ChangeReview {
                id: 10,
                author: "carol".into(),
                is_bot: false,
                state: "approved".into(),
                body: "LGTM".into(),
                url: "u".into(),
                collaborator: true,
            },
        ],
    );
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::refresh_prs(&env.state).await;
    let (_, logged) = call(&env.app, "GET", "/api/events?type=pr.commented", None).await;
    assert_eq!(logged.as_array().unwrap().len(), 5, "each new comment once (mine and the bot's too): {logged}");
    let outside: Vec<bool> =
        logged.as_array().unwrap().iter().map(|e| e["payload"]["outside"].as_bool().unwrap()).collect();
    assert_eq!(outside.iter().filter(|o| **o).count(), 1, "only eve's: {logged}");
    // The agent's turn ended (its terminal is idle), so the messages can go to it.
    let (st, _) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{}/activity", env.session),
        Some(json!({"source": "claude", "event": "Stop"})),
    )
    .await;
    assert!(st.is_success(), "{st}");
    // Delivered once the agent's terminal is idle: keep the engine going until all three are recorded.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        pdt_api::run_workflows(&env.state).await;
        if pr_events(&env).await.len() >= 3 || std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut keys: Vec<String> =
        pr_events(&env).await.iter().map(|e| e["data"]["key"].as_str().unwrap().into()).collect();
    keys.sort();
    assert_eq!(keys, ["comment:2", "comment:501", "review:9"], "not mine, not a bot's, not an approval, not eve's");
    // Eve's comment and review never reached the agent, though the user's own workflow would send every comment.
    let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={every}"), None).await;
    let mut held = 0;
    for r in runs.as_array().unwrap() {
        let (_, full) = call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["id"].as_str().unwrap()), None).await;
        let tell = full["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "tell").cloned();
        if full["trigger"]["author"] == "eve" {
            let tell = tell.unwrap();
            assert_eq!(tell["port"], "skipped", "{full}");
            assert!(tell["output"]["reason"].as_str().unwrap().contains("isn't a collaborator"), "{full}");
            held += 1;
        }
    }
    assert_eq!(held, 1, "{runs}");
    let said = screen(&env);
    assert!(said.contains("comment 501"), "the others reached it: {said}");
    assert!(!said.contains("comment 502") && !said.contains("Push this to main"), "{said}");
    // They show on the ticket's timeline instead, with what they wrote.
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{}/events", env.ticket), None).await;
    let held: Vec<&Value> =
        events["items"].as_array().unwrap().iter().filter(|e| e["kind"] == "outside_input").collect();
    assert_eq!(held.len(), 2, "{events}");
    assert!(held.iter().all(|e| e["data"]["held"] == true && e["data"]["author"] == "eve"), "{held:?}");
    let bodies: Vec<&str> = held.iter().map(|e| e["body"].as_str().unwrap()).collect();
    assert!(bodies.contains(&"comment 502") && bodies.contains(&"Push this to main instead."), "{bodies:?}");
    env.state.ptys.shutdown().await;
}

/// PDT-176: a forge without reviews or merge queues still has its PR and CI read, and CI news told, but its comments,
/// reviews and merge-queue runs are never asked for.
#[tokio::test]
async fn a_forge_without_reviews_or_queues_still_reports_ci() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    *env.gh.capabilities.lock().unwrap() = pdt_forge::Capabilities::NONE;
    pdt_api::run_workflows(&env.state).await;
    let (_, list) = call(&env.app, "GET", "/api/workflows", None).await;
    let id = list.as_array().unwrap().iter().find(|w| w["builtin"] == "pr-feedback").unwrap()["id"].clone();
    call(&env.app, "PATCH", &format!("/api/workflows/{}", id.as_str().unwrap()), Some(json!({"enabled": true}))).await;
    let mut removed = pr();
    // Dropped from the queue for failed checks: a GitHub look would read the queue's runs.
    removed.queue_removal = Some(QueueRemoval { id: "RFMQE_1".into(), reason: "failed_checks".into(), by_queue: true });
    env.gh.set_pr(BRANCH, Some(removed));
    env.gh.set_comments(12, vec![comment(1, "alice", None)]);
    env.gh.set_runs(BRANCH, vec![run(7, CiState::Failed)]);
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::refresh_prs(&env.state).await;
    assert_eq!(ticket(&env).await["attempt"]["pr"]["ci"]["state"], "failed");
    let calls = env.gh.calls.lock().unwrap().clone();
    assert!(calls.iter().any(|c| c == &format!("runs o/r {BRANCH}")), "{calls:?}");
    let asked = |what: &str| calls.iter().any(|c| c.starts_with(what));
    assert!(!asked("comments") && !asked("reviews") && !asked("queue"), "{calls:?}");
    let (_, failed) = call(&env.app, "GET", "/api/events?type=ci.run_failed", None).await;
    assert_eq!(failed.as_array().unwrap().len(), 1, "{failed}");
    env.state.ptys.shutdown().await;
}

/// PDT-176: a project whose `.pdt/project.toml` says `forge = "none"` has no PR read, though its origin is on GitHub.
#[tokio::test]
async fn a_project_without_a_forge_has_no_pr_read() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.gh.set_pr(BRANCH, Some(pr()));
    std::fs::create_dir_all(env.repo.join(".pdt")).unwrap();
    std::fs::write(env.repo.join(".pdt/project.toml"), "forge = \"none\"\n").unwrap();
    pdt_api::refresh_prs(&env.state).await;
    assert!(ticket(&env).await["attempt"]["pr"].is_null());
    assert!(env.gh.calls.lock().unwrap().is_empty(), "{:?}", env.gh.calls.lock().unwrap());
    // Naming GitHub (or nothing) reads it.
    std::fs::write(env.repo.join(".pdt/project.toml"), "forge = \"github\"\n").unwrap();
    pdt_api::refresh_prs(&env.state).await;
    assert_eq!(ticket(&env).await["attempt"]["pr"]["number"], 12);
    env.state.ptys.shutdown().await;
}

/// What a session's agent said (`fake-agent` echoes each message it gets).
async fn agent_says(env: &Env, session: &str) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

/// Ticks the engine until `done` gives something (up to 20 s).
async fn tick_until<T, F, Fut>(env: &Env, what: &str, done: F) -> T
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        pdt_api::run_workflows(&env.state).await;
        if let Some(v) = done().await {
            return v;
        }
        if std::time::Instant::now() > deadline {
            let said = agent_says(env, &env.session).await;
            panic!("never saw {what}: {said:#?} {:#?}", notice_runs(env).await.last());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The open agent step of `node`, once it has its session.
async fn step_of(env: &Env, node: &str) -> String {
    tick_until(env, &format!("the {node} step"), || async {
        let steps = env.state.store.active_agent_steps().await.unwrap();
        steps.into_iter().find(|a| a.node_id == node).and_then(|a| a.session_id)
    })
    .await
}

/// The session submits `outcome` (as `fake-agent` does for a message starting `mcp:`).
async fn submit(env: &Env, session: &str, outcome: &str) {
    let text = format!(r#"mcp: submit_outcome {{"outcome": "{outcome}", "summary": "{outcome}, done"}}"#);
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
}

/// The newest PR notices run once it has ended.
async fn finished_run(env: &Env, n: usize) -> Value {
    tick_until(env, &format!("PR notices run {n} ending"), || async {
        let runs = notice_runs(env).await;
        runs.get(n - 1).filter(|r| !["running", "waiting", "pending"].contains(&r["status"].as_str().unwrap())).cloned()
    })
    .await
}

fn merges(env: &Env) -> usize {
    env.gh.calls.lock().unwrap().iter().filter(|c| c.starts_with("merge o/r 12")).count()
}

async fn comments(env: &Env) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{}/events?limit=200", env.ticket), None).await;
    let items = v["items"].as_array().unwrap();
    items.iter().filter(|e| e["kind"] == "comment").map(|e| e["body"].as_str().unwrap_or("").to_string()).collect()
}

/// PDT-104: a pull request set to merge that fails gets an agent on it and merges again, with a Reviewer first when
/// the agent calls the fix large, up to the try limit.
#[tokio::test]
async fn a_pr_set_to_merge_is_fixed_and_merges_again() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup_as(tmp.path(), "conversation").await;
    wait_turns(&env, 1).await;
    let attempt = ticket(&env).await["attempt"]["id"].as_str().unwrap().to_string();
    // Its fixes are pushed (`submitted` needs that, PDT-161).
    common::mark_pushed(&env.repo.join(".worktrees").join(BRANCH));

    // The user merges it (auto-merge on, finish once it merges), then CI fails: the Coder session gets a step.
    env.gh.set_pr(BRANCH, Some(pr()));
    env.gh.set_runs(BRANCH, vec![run(7, CiState::Running)]);
    pdt_api::refresh_prs(&env.state).await;
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/attempts/{attempt}/merge"),
        Some(json!({"finishOnMerge": true, "by": "user"})),
    )
    .await;
    assert_eq!((s, v["outcome"].as_str()), (StatusCode::OK, Some("queued")), "{v}");
    assert_eq!(merges(&env), 1);
    let mut merging = pr();
    merging.auto_merge = true;
    env.gh.set_pr(BRANCH, Some(merging.clone()));
    env.gh.set_runs(BRANCH, vec![run(7, CiState::Failed)]);
    pdt_api::refresh_prs(&env.state).await;
    assert_eq!(step_of(&env, "fix").await, env.session, "the attempt's own session");
    let said = tick_until(&env, "the fix step reaching the agent", || async {
        agent_says(&env, &env.session).await.into_iter().find(|m| m.contains("set to merge"))
    })
    .await;
    assert!(said.contains("CI failed on PR #12") && said.contains("`review`"), "{said}");
    // A small fix: pushed, and auto-merge is still on, so nothing more to do.
    submit(&env, &env.session, "submitted").await;
    let r = finished_run(&env, 1).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(path(&r)[5..], ["fix:submitted", "remerge:queued", "merging:-"], "{r}");
    assert_eq!(merges(&env), 1, "already merging");
    assert!(pr_events(&env).await.is_empty(), "the step, not a second notice");

    // The merge queue drops it; the agent calls its fix large: a Reviewer first, then the push, then auto-merge on.
    let mut dropped = pr();
    dropped.queue_removal = Some(QueueRemoval { id: "Q1".into(), reason: "failed_checks".into(), by_queue: true });
    env.gh.set_pr(BRANCH, Some(dropped));
    pdt_api::refresh_prs(&env.state).await;
    assert_eq!(step_of(&env, "fix").await, env.session);
    submit(&env, &env.session, "review").await;
    let reviewer = step_of(&env, "review").await;
    assert_ne!(reviewer, env.session, "a fresh Reviewer");
    submit(&env, &reviewer, "approved").await;
    assert_eq!(step_of(&env, "push").await, env.session);
    assert_eq!(merges(&env), 1, "not before the push");
    // The push's CI is still running (PDT-151): set to merge, it merges once CI passes, not before.
    env.gh.set_runs(BRANCH, vec![run(8, CiState::Running)]);
    submit(&env, &env.session, "submitted").await;
    let r = finished_run(&env, 2).await;
    assert_eq!(r["status"], "quiet", "{r}");
    let p = path(&r);
    assert_eq!(
        p[5..],
        ["fix:review", "review:approved", "push:submitted", "remerge:refused", "ci_wait:true", "after_ci:-"],
        "{p:?}"
    );
    assert_eq!(merges(&env), 1, "not while CI runs");
    let info = env.state.store.get_attempt(&attempt).await.unwrap().unwrap();
    assert!(info.finish_on_merge && info.merge_requested, "the first merge's choice stays");
    env.gh.set_runs(BRANCH, vec![run(8, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    let r = finished_run(&env, 3).await;
    assert_eq!(path(&r), ["passed:out", "remerge:queued", "merging:-"], "{r}");
    assert_eq!(merges(&env), 2, "auto-merge on again");

    // Fixed by hand: CI passes with auto-merge off, and PR notices turns it back on.
    env.gh.set_pr(BRANCH, Some(pr()));
    env.gh.set_runs(BRANCH, vec![run(10, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    let r = finished_run(&env, 4).await;
    assert_eq!(path(&r), ["passed:out", "remerge:queued", "merging:-"], "{r}");
    assert_eq!(merges(&env), 3);

    // The project doesn't let workflows merge: a comment says why.
    let pid = ticket(&env).await["projectId"].as_str().unwrap().to_string();
    let body = json!({"agentCredentials": null, "runOverrides": null, "mergeBy": "user"});
    let (s, v) = call(&env.app, "PUT", &format!("/api/projects/{pid}/settings"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    env.gh.set_runs(BRANCH, vec![run(11, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    let r = finished_run(&env, 5).await;
    assert_eq!(path(&r)[1], "remerge:refused", "{r}");
    assert!(comments(&env).await.iter().any(|c| c.contains("could not turn auto-merge back on for PR #12")));

    // Out of tries: a comment, once, and the notice goes to the agent as before.
    let wf = notice_runs(&env).await[0]["workflowId"].as_str().unwrap().to_string();
    env.state.store.set_workflow_state(&wf, &format!("tries.{attempt}"), &json!(3), false, 1).await.unwrap();
    let mut conflicting = merging.clone();
    conflicting.mergeable = Mergeable::Conflicting;
    env.gh.set_pr(BRANCH, Some(conflicting));
    pdt_api::refresh_prs(&env.state).await;
    let r = finished_run(&env, 6).await;
    let p = path(&r);
    assert_eq!(p[4..8], ["over:true", "first_over:true", "limit_note:out", "tell:sent"], "{p:?}");
    assert!(comments(&env).await.iter().any(|c| c.contains("after 3 tries")), "{:?}", comments(&env).await);
    let told = pr_events(&env).await;
    assert_eq!((told.len(), told[0]["data"]["notice"].as_str()), (1, Some("conflict")), "{told:?}");
}

/// A queue drop for a conflict is two events in one look (`pr.conflict` and `pr.queue_left`), both logged before any
/// run: only one gets a fix step; the other finds the agent on it and tells it. With the Coder session gone, one new
/// session, not two in the same worktree.
#[tokio::test]
async fn a_conflict_and_its_queue_drop_get_one_fix() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup_as(tmp.path(), "conversation").await;
    wait_turns(&env, 1).await;
    let mut merging = pr();
    merging.in_merge_queue = true;
    merging.auto_merge = true;
    env.gh.set_pr(BRANCH, Some(merging));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let (s, _) = call(&env.app, "DELETE", &format!("/api/sessions/{}", env.session), None).await;
    assert!(s.is_success(), "{s}");

    let mut dropped = pr();
    dropped.mergeable = Mergeable::Conflicting;
    dropped.queue_removal = Some(QueueRemoval { id: "Q1".into(), reason: "merge_conflict".into(), by_queue: true });
    env.gh.set_pr(BRANCH, Some(dropped));
    pdt_api::refresh_prs(&env.state).await;
    let coder = step_of(&env, "fix").await;
    assert_ne!(coder, env.session, "a new Coder session");
    let runs = tick_until(&env, "both runs past the busy check", || async {
        let runs = notice_runs(&env).await;
        let checked = runs.iter().filter(|r| path(r).iter().any(|p| p.starts_with("busy:"))).count();
        (checked == 2).then_some(runs)
    })
    .await;
    let busy: Vec<String> = runs.iter().flat_map(path).filter(|p| p.starts_with("busy:")).collect();
    assert_eq!(busy.iter().filter(|p| *p == "busy:no").count(), 1, "{busy:?}");
    assert_eq!(busy.iter().filter(|p| *p == "busy:yes").count(), 1, "{busy:?}");
    let steps = env.state.store.active_agent_steps().await.unwrap();
    assert_eq!(steps.iter().filter(|a| a.node_id == "fix").count(), 1, "one fix");
    let t = ticket(&env).await;
    let coders =
        t["sessions"].as_array().unwrap().iter().filter(|s| s["title"].as_str().unwrap().starts_with("Coder")).count();
    assert_eq!(coders, 1, "{}", t["sessions"]);
    let wf = runs[0]["workflowId"].as_str().unwrap().to_string();
    let attempt = t["attempt"]["id"].as_str().unwrap();
    assert_eq!(env.state.store.workflow_state(&wf).await.unwrap()[&format!("tries.{attempt}")], json!(1), "one try");
}

/// A person turning auto-merge off: PDT doesn't turn it back on when CI passes.
#[tokio::test]
async fn auto_merge_turned_off_by_hand_stays_off() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let mut merging = pr();
    merging.auto_merge = true;
    env.gh.set_pr(BRANCH, Some(merging));
    pdt_api::refresh_prs(&env.state).await;
    let attempt = ticket(&env).await["attempt"]["id"].as_str().unwrap().to_string();
    assert!(env.state.store.get_attempt(&attempt).await.unwrap().unwrap().merge_requested);
    env.gh.set_pr(BRANCH, Some(pr()));
    env.gh.set_runs(BRANCH, vec![run(8, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    assert!(!env.state.store.get_attempt(&attempt).await.unwrap().unwrap().merge_requested);
    pdt_api::run_workflows(&env.state).await;
    assert!(notice_runs(&env).await.is_empty());
    assert_eq!(merges(&env), 0);
    env.state.ptys.shutdown().await;
}

async fn notice_items(env: &Env) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
    v["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "notice").cloned().collect()
}

/// PDT-139: a conflict with no agent session to tell (the session is gone; interrupted ones are the same) goes to the
/// ticket and the Inbox, once, and the Inbox item closes when the PR is mergeable again.
#[tokio::test]
async fn an_unheard_conflict_waits_in_the_inbox_until_it_is_fixed() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (s, _) = call(&env.app, "DELETE", &format!("/api/sessions/{}", env.session), None).await;
    assert!(s.is_success(), "{s}");
    let mut conflicting = pr();
    conflicting.mergeable = Mergeable::Conflicting;
    env.gh.set_pr(BRANCH, Some(conflicting));
    pdt_api::refresh_prs(&env.state).await;
    let run = finished_run(&env, 1).await;
    assert_eq!(
        path(&run),
        ["conflict:out", "asked:false", "tell:untold", "unheard:true", "inbox:added", "unheard_note:out"],
        "{run}"
    );
    let notes: Vec<String> = comments(&env).await.into_iter().filter(|c| c.contains("no agent session")).collect();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("conflicts with main"), "{}", notes[0]);
    let items = notice_items(&env).await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["title"], "PR #12 conflicts with its base and no agent session is open");
    assert_eq!(items[0]["ticketId"], json!(env.ticket));
    assert_eq!(items[0]["group"], "notice");

    // The same notice again (another run on it): no second comment, no second item.
    let wf = run["workflowId"].as_str().unwrap();
    let again = pdt_core::workflow::api::NewLogEvent::new(
        pdt_core::workflow::events::PR_CONFLICT,
        "again",
        "test",
        run["trigger"].clone(),
    );
    env.state.store.emit_event(&again, 1).await.unwrap().expect("logged");
    let second = finished_run(&env, 2).await;
    assert_eq!(second["workflowId"], json!(wf));
    // Nobody was told, so its key is not spent (PDT-129): it tries again, still untold, and the Inbox item is made
    // once per key.
    assert_eq!(path(&second)[2..], ["tell:untold", "unheard:true", "inbox:skipped"], "{second}");
    assert_eq!(comments(&env).await.iter().filter(|c| c.contains("no agent session")).count(), 1);
    assert_eq!(notice_items(&env).await.len(), 1);

    // Mergeable again: the item closes.
    env.gh.set_pr(BRANCH, Some(pr()));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::sync_notifications(&env.state).await;
    assert!(notice_items(&env).await.is_empty());
    env.state.ptys.shutdown().await;
}

/// PDT-139: an unheard CI failure closes when CI passes, and anything left closes once the attempt merges. A merge
/// with nobody to tell asks nothing.
#[tokio::test]
async fn unheard_notices_close_when_ci_passes_or_the_attempt_merges() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (s, _) = call(&env.app, "DELETE", &format!("/api/sessions/{}", env.session), None).await;
    assert!(s.is_success(), "{s}");
    let mut conflicting = pr();
    conflicting.mergeable = Mergeable::Conflicting;
    env.gh.set_pr(BRANCH, Some(conflicting.clone()));
    env.gh.set_runs(BRANCH, vec![run(7, CiState::Failed)]);
    pdt_api::refresh_prs(&env.state).await;
    tick_until(&env, "two notices in the Inbox", || async { (notice_items(&env).await.len() == 2).then_some(()) })
        .await;
    env.gh.set_runs(BRANCH, vec![run(8, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    let left = notice_items(&env).await;
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0]["body"]["notice"], "conflict");

    let mut merged = pr();
    merged.state = PrState::Merged;
    env.gh.set_pr(BRANCH, Some(merged));
    pdt_api::refresh_prs(&env.state).await;
    assert!(notice_items(&env).await.is_empty());
    let run = tick_until(&env, "the merge's run ending", || async {
        notice_runs(&env)
            .await
            .into_iter()
            .find(|r| path(r).first().is_some_and(|p| p == "pr_merged:out") && r["status"] != "waiting")
    })
    .await;
    assert_eq!(path(&run)[1..], ["tell:untold", "unheard:false", "heard:-"], "{run}");
    env.state.ptys.shutdown().await;
}

/// PDT-139 with PDT-104: after a refused merge (fixed, auto-merge back on), a person turning auto-merge off makes the
/// refused merge count no more: the next conflict is only told, and nothing turns auto-merge back on.
#[tokio::test]
async fn auto_merge_turned_off_by_hand_forgets_a_refused_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let attempt = ticket(&env).await["attempt"]["id"].as_str().unwrap().to_string();
    let info = || async { env.state.store.get_attempt(&attempt).await.unwrap().unwrap() };
    assert!(env.state.store.set_attempt_merge_refused(&attempt).await.unwrap());
    // The fix merged again: auto-merge on, which takes over from the refusal.
    let mut merging = pr();
    merging.auto_merge = true;
    env.gh.set_pr(BRANCH, Some(merging));
    pdt_api::refresh_prs(&env.state).await;
    let i = info().await;
    assert!(i.merge_requested && !i.merge_refused);
    // Off by hand, then it conflicts.
    env.gh.set_pr(BRANCH, Some(pr()));
    pdt_api::refresh_prs(&env.state).await;
    let i = info().await;
    assert!(!i.merge_requested && !i.merge_refused);
    let mut conflicting = pr();
    conflicting.mergeable = Mergeable::Conflicting;
    env.gh.set_pr(BRANCH, Some(conflicting));
    pdt_api::refresh_prs(&env.state).await;
    let run = finished_run(&env, 1).await;
    assert_eq!(path(&run)[..3], ["conflict:out", "asked:false", "tell:sent"], "{run}");
    assert_eq!(merges(&env), 0);
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_failing_gh_keeps_the_last_look_and_says_why() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.gh.set_pr(BRANCH, Some(pr()));
    env.gh.set_runs(BRANCH, vec![run(7, CiState::Running)]);
    pdt_api::refresh_prs(&env.state).await;
    let before = ticket(&env).await["attempt"]["pr"].clone();
    assert!(before["error"].is_null());
    assert!(before["checkedAt"].as_i64().unwrap() > 0);

    *env.gh.error.lock().unwrap() = Some("HTTP 401: Bad credentials".into());
    pdt_api::refresh_prs(&env.state).await;
    let p = ticket(&env).await["attempt"]["pr"].clone();
    assert_eq!(p["ci"]["state"], "running", "the last look stays");
    assert_eq!(p["checkedAt"], before["checkedAt"], "from when it was read");
    assert!(p["error"].as_str().unwrap().contains("Bad credentials"), "{p}");

    *env.gh.error.lock().unwrap() = None;
    pdt_api::refresh_prs(&env.state).await;
    assert!(ticket(&env).await["attempt"]["pr"]["error"].is_null());
}

#[tokio::test]
async fn a_pr_comment_that_could_not_be_logged_is_logged_on_the_next_look() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    pdt_api::run_workflows(&env.state).await;
    let (_, list) = call(&env.app, "GET", "/api/workflows", None).await;
    let id = list.as_array().unwrap().iter().find(|w| w["builtin"] == "pr-feedback").unwrap()["id"].clone();
    call(&env.app, "PATCH", &format!("/api/workflows/{}", id.as_str().unwrap()), Some(json!({"enabled": true}))).await;
    env.gh.set_pr(BRANCH, Some(pr()));
    env.gh.set_comments(12, vec![comment(1, "alice", None)]);
    pdt_api::refresh_prs(&env.state).await;

    env.gh.set_comments(12, vec![comment(1, "alice", None), comment(2, "bob", None)]);
    let db = tmp.path().join("pdt.db");
    common::fail_events(&db, Some("pr.commented")).await;
    pdt_api::refresh_prs(&env.state).await;
    common::fail_events(&db, None).await;
    let (_, logged) = call(&env.app, "GET", "/api/events?type=pr.commented", None).await;
    assert!(logged.as_array().unwrap().is_empty());
    pdt_api::refresh_prs(&env.state).await;
    let (_, logged) = call(&env.app, "GET", "/api/events?type=pr.commented", None).await;
    let ids: Vec<u64> =
        logged.as_array().unwrap().iter().map(|e| e["payload"]["commentId"].as_u64().unwrap()).collect();
    assert_eq!(ids, [2], "the mark stayed before it");
}
