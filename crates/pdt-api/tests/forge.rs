//! PDT-177 through the API: the in-app GitHub login (a pasted token, the device flow, disconnecting), the Inbox's
//! "Connect GitHub", and the forge tools an agent uses instead of `gh`. GitHub is recorded answers
//! (`pdt_github::transport::testing::Recorded`) or `FakeForge`; `fake-agent` stands in for Claude (it calls pdt-mcp
//! tools when a message says `mcp: <tool> <json>`).

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_forge::JobLog;
use pdt_forge::testing::FakeForge;
use pdt_github::transport::Response;
use pdt_github::transport::testing::Recorded;
use pdt_github::{Forge, GithubApi};
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    _tmp: tempfile::TempDir,
}

/// A project whose origin is on github.com; `forge` is the GitHub forge unless None (then the state's own, signed out
/// and offline).
async fn setup(forge: Option<Arc<dyn Forge>>, github: Option<Arc<GithubApi>>) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    git(&repo, &["remote", "add", "origin", "https://github.com/o/r.git"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    if let Some(g) = github {
        state.github = g.clone();
        state.github_api = g;
    }
    if let Some(f) = forge {
        state.github = f;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, project: p["id"].as_str().unwrap().into(), _tmp: tmp }
}

/// A conversation session on a new ticket: its id and branch.
async fn session(env: &Env) -> (String, String) {
    let (s, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Fix CI"})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{}/start", t["id"].as_str().unwrap()),
        Some(json!({"agent": "claude", "sessionMode": "conversation"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let id = started["session"]["id"].as_str().unwrap().to_string();
    (id, started["attempt"]["branch"].as_str().unwrap().to_string())
}

async fn agent_says(env: &Env, session: &str) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

async fn wait_said(env: &Env, session: &str, n: usize) -> String {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let said = agent_says(env, session).await;
        if said.len() >= n {
            return said[n - 1].clone();
        }
        assert!(Instant::now() < deadline, "the agent said only {said:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn say(env: &Env, session: &str, text: &str) -> String {
    let before = agent_says(env, session).await.len();
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    wait_said(env, session, before + 1).await
}

fn run(id: u64, branch: &str, state: pdt_core::pr::CiState, at: &str) -> pdt_core::pr::CiRun {
    pdt_core::pr::CiRun {
        id,
        attempt: 1,
        workflow: "ci".into(),
        state,
        url: format!("https://github.com/o/r/actions/runs/{id}"),
        head_sha: "abc".into(),
        head_branch: branch.into(),
        created_at: at.into(),
        failed_jobs: vec![],
    }
}

fn pr(number: u32) -> pdt_forge::ChangeRequest {
    pdt_forge::ChangeRequest {
        number,
        url: format!("https://github.com/o/r/pull/{number}"),
        title: "Fix CI".into(),
        state: pdt_core::pr::PrState::Open,
        draft: false,
        head_sha: "abc".into(),
        base_branch: "main".into(),
        mergeable: pdt_core::pr::Mergeable::Mergeable,
        behind: false,
        auto_merge: false,
        in_merge_queue: false,
        queue_position: None,
        queue_removal: None,
        created_at: None,
    }
}

#[tokio::test]
async fn an_agent_reads_failed_ci_logs_and_answers_on_its_pr_through_pdt_tools() {
    use pdt_core::pr::CiState;
    let gh = Arc::new(FakeForge::default());
    let env = setup(Some(gh.clone()), None).await;
    let (s, branch) = session(&env).await;
    wait_said(&env, &s, 1).await;

    gh.set_runs(
        &branch,
        vec![
            run(7, &branch, CiState::Failed, "2026-10-27T10:00:00Z"),
            run(8, &branch, CiState::Passed, "2026-10-27T11:00:00Z"),
        ],
    );
    gh.set_jobs(7, &["test (linux)"]);
    gh.logs.lock().unwrap().insert(
        7,
        vec![JobLog { name: "test (linux)".into(), url: "https://j/1".into(), log: "error: boom\n".into() }],
    );
    let said = say(&env, &s, "mcp: ci_runs {}").await;
    assert!(said.contains("failedJobs") && said.contains("test (linux)"), "{said}");
    let said = say(&env, &s, "mcp: ci_failed_logs {}").await;
    assert!(said.contains("\"runId\": 7") && said.contains("error: boom"), "the newest failed run: {said}");
    assert!(gh.calls.lock().unwrap().contains(&"logs o/r 7".to_string()));

    let said = say(&env, &s, "mcp: change_request_comment {\"body\": \"Fixed in abc\"}").await;
    assert!(said.contains("has no pull request yet"), "{said}");
    gh.set_pr(&branch, Some(pr(12)));
    gh.set_comments(
        12,
        vec![pdt_forge::ChangeComment {
            id: 55,
            author: "al".into(),
            is_bot: false,
            body: "nit".into(),
            url: "u".into(),
            path: Some("a.txt".into()),
            line: Some(1),
            in_review: true,
        }],
    );
    let said = say(&env, &s, "mcp: change_request_view {}").await;
    assert!(said.contains("\"number\": 12") && said.contains("\"reviewComment\": true"), "{said}");
    let said = say(&env, &s, "mcp: change_request_comment {\"body\": \"Fixed in abc\"}").await;
    assert!(said.contains("pull/12#issuecomment-1"), "{said}");
    let said = say(&env, &s, "mcp: change_request_comment {\"body\": \"Done\", \"replyTo\": 55}").await;
    assert!(said.contains("discussion_r2"), "{said}");
    assert_eq!(
        *gh.change_comments_added.lock().unwrap(),
        [(12, None, "Fixed in abc".to_string()), (12, Some(55), "Done".to_string())]
    );

    gh.set_issues(
        "o/r",
        vec![pdt_forge::Issue {
            number: 3,
            title: "Crash".into(),
            body: "b".into(),
            url: "u".into(),
            author: "al".into(),
            labels: vec![],
            created_at: "t".into(),
        }],
    );
    let said = say(&env, &s, "mcp: issue_comment {\"number\": 3, \"body\": \"Looking\"}").await;
    assert!(said.contains("issues/3#issuecomment-1"), "{said}");
    let said = say(&env, &s, "mcp: issue_view {\"number\": 3}").await;
    assert!(said.contains("\"title\": \"Crash\"") && said.contains("Looking"), "{said}");
}

#[tokio::test]
async fn without_a_login_the_inbox_asks_to_connect_github_until_a_token_is_saved() {
    let t = Arc::new(Recorded::new());
    let g = Arc::new(GithubApi::new(t.clone()));
    let env = setup(None, Some(g.clone())).await;
    let (_, v) = call(&env.app, "GET", "/api/github/login", None).await;
    assert_eq!((v["connected"].as_bool(), v["scopes"].as_str()), (Some(false), Some("repo")));
    assert_eq!(v["deviceFlow"], json!(!pdt_core::brand::GITHUB_OAUTH_CLIENT_ID.is_empty()));
    let connect = |inbox: &Value| inbox["items"].as_array().unwrap().iter().any(|i| i["kind"] == "connect");
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    assert!(!connect(&inbox), "nothing wanted GitHub yet");

    // The PR poller (or a merge, or an agent's tool) wants GitHub.
    let g2 = g.clone();
    let e = tokio::task::spawn_blocking(move || g2.branch_runs("o/r", "b")).await.unwrap().unwrap_err();
    assert!(e.to_string().contains("GitHub is not connected"), "{e}");
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    assert!(connect(&inbox), "{inbox}");
    let item = inbox["items"].as_array().unwrap().iter().find(|i| i["kind"] == "connect").unwrap().clone();
    assert_eq!((item["title"].as_str(), item["group"].as_str()), (Some("Connect GitHub"), Some("notice")));
    assert!(t.sent().is_empty(), "nothing reached GitHub");

    t.on("GET", "https://api.github.com/user", Response::new(401, r#"{"message":"Bad credentials"}"#));
    let (s, v) = call(&env.app, "POST", "/api/github/login/token", Some(json!({"token": "bad"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["message"].as_str().unwrap().contains("does not accept"), "{v}");
    let (s, _) = call(&env.app, "POST", "/api/github/login/token", Some(json!({"token": " "}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    t.ok("GET", "https://api.github.com/user", r#"{"login":"ross"}"#);
    let (s, v) = call(&env.app, "POST", "/api/github/login/token", Some(json!({"token": "gho_good"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["connected"].as_bool(), v["login"].as_str()), (Some(true), Some("ross")));
    assert!(!v.to_string().contains("gho_good"), "the token never leaves PDT");
    let stored = env.state.secrets.get(&env.state.github_account).unwrap().unwrap();
    assert!(stored.contains("gho_good") && stored.contains("https://api.github.com"), "in the keychain");
    assert_eq!(g.account().unwrap().login, "ross");
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    assert!(!connect(&inbox), "connected: {inbox}");

    let (s, v) = call(&env.app, "DELETE", "/api/github/login", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["connected"], false);
    assert_eq!(env.state.secrets.get(&env.state.github_account).unwrap(), None);
    assert!(g.account().is_none());

    // pdtd's startup signs in with the stored login.
    env.state
        .secrets
        .set(&env.state.github_account, r#"{"token":"t","apiUrl":"https://ghe.example.com/api/v3","login":"ross"}"#)
        .unwrap();
    pdt_api::github_login::load(&env.state).await;
    let (_, v) = call(&env.app, "GET", "/api/github/login", None).await;
    assert_eq!(v["webUrl"], "https://ghe.example.com");
}

#[tokio::test]
async fn connect_github_enters_a_code_then_signs_in() {
    let t = Arc::new(Recorded::new());
    let g = Arc::new(GithubApi::new(t.clone()));
    let mut env = setup(None, Some(g.clone())).await;
    // No client ID: only a pasted token.
    env.state.github_client_id = String::new();
    env.app = router(env.state.clone(), None);
    let (s, v) = call(&env.app, "POST", "/api/github/login/device", Some(json!({}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("no_client_id")), "{v}");

    env.state.github_client_id = "Iv1.pdt".into();
    env.app = router(env.state.clone(), None);
    t.ok(
        "POST",
        "https://github.com/login/device/code",
        r#"{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device",
            "expires_in":900,"interval":0}"#,
    );
    let token = "https://github.com/login/oauth/access_token";
    t.ok("POST", token, r#"{"error":"authorization_pending"}"#);
    t.ok("POST", token, r#"{"access_token":"gho_dev","token_type":"bearer","scope":"repo"}"#);
    t.on("GET", "https://api.github.com/user", Response::new(200, r#"{"login":"ross"}"#));
    let (s, v) = call(&env.app, "POST", "/api/github/login/device", Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["pending"]["userCode"], "ABCD-1234");
    assert_eq!(v["pending"]["verificationUri"], "https://github.com/login/device");
    let deadline = Instant::now() + Duration::from_secs(10);
    let v = loop {
        let (_, v) = call(&env.app, "GET", "/api/github/login", None).await;
        if v["connected"] == true || v["error"].is_string() {
            break v;
        }
        assert!(Instant::now() < deadline, "never signed in: {v}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!((v["login"].as_str(), v["pending"].is_null(), v["error"].is_null()), (Some("ross"), true, true), "{v}");
    assert_eq!(g.account().unwrap().token, "gho_dev");
    assert!(t.sent()[0].body.as_deref().unwrap().contains("client_id=Iv1.pdt"));

    // A code that is never entered: cancelling stops the wait.
    t.ok(
        "POST",
        "https://github.com/login/device/code",
        r#"{"device_code":"dc2","user_code":"WXYZ-0000","verification_uri":"https://github.com/login/device",
            "expires_in":900,"interval":5}"#,
    );
    let (_, v) = call(&env.app, "POST", "/api/github/login/device", Some(json!({}))).await;
    assert_eq!(v["pending"]["userCode"], "WXYZ-0000");
    let (_, v) = call(&env.app, "DELETE", "/api/github/login/device", None).await;
    assert!(v["pending"].is_null() && v["connected"] == true, "{v}");
}

#[tokio::test]
async fn an_agent_opens_its_pr_and_asks_for_the_merge_again_after_a_queue_drop() {
    let gh = Arc::new(FakeForge::default());
    let env = setup(Some(gh.clone()), None).await;
    let (s, branch) = session(&env).await;
    wait_said(&env, &s, 1).await;

    let said = say(&env, &s, "mcp: change_request_open {\"title\": \" \"}").await;
    assert!(said.contains("needs a title"), "{said}");
    let said = say(&env, &s, "mcp: change_request_open {\"title\": \"Fix CI\", \"body\": \"Why\"}").await;
    assert!(said.contains("\"number\": 100") && said.contains("\"existing\": false"), "{said}");
    assert!(said.contains("\"base\": \"main\""), "the attempt's base: {said}");
    let opened = gh.opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 1);
    assert_eq!(
        (opened[0].head.as_str(), opened[0].base.as_str(), opened[0].body.as_str()),
        (branch.as_str(), "main", "Why")
    );
    let said = say(&env, &s, "mcp: change_request_open {\"title\": \"Again\"}").await;
    assert!(said.contains("\"number\": 100") && said.contains("\"existing\": true"), "{said}");
    assert_eq!(gh.opened.lock().unwrap().len(), 1, "the branch's open one, not a second");

    // Never set to merge: the merge is the user's or the workflow's.
    let said = say(&env, &s, "mcp: change_request_merge {}").await;
    assert!(said.contains("was never set to merge"), "{said}");
    assert!(!gh.calls.lock().unwrap().iter().any(|c| c.starts_with("merge ")), "nothing merged");

    // The merge queue dropped it: asking again sets it to merge.
    let mut p = pr(100);
    p.queue_removal =
        Some(pdt_core::pr::QueueRemoval { id: "RFMQE_1".into(), reason: "failed_checks".into(), by_queue: true });
    gh.set_pr(&branch, Some(p.clone()));
    let said = say(&env, &s, "mcp: change_request_merge {}").await;
    assert!(said.contains("\"outcome\": \"queued\""), "{said}");
    assert!(gh.calls.lock().unwrap().iter().any(|c| c.starts_with("merge o/r 100")), "{:?}", gh.calls.lock().unwrap());

    p.auto_merge = true;
    gh.set_pr(&branch, Some(p));
    let said = say(&env, &s, "mcp: change_request_merge {}").await;
    assert!(said.contains("\"outcome\": \"already\""), "{said}");
}
