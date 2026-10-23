//! PDT-81 end to end: the PM's sessions in the host project, its tools, asking before acting (and the project policy
//! that lets it act), notes, the journal, the briefing and rotation, with `fake-agent` standing in for Claude (it calls
//! pdt-mcp tools when a message says `mcp: <tool> <json>`, asking first for tools `--allowedTools` leaves out).

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    gh: std::sync::Arc<pdt_github::testing::FakeGithub>,
    _tmp: tempfile::TempDir,
}

/// A project `P` (key `PX`) with one ticket, served on a free local port (the agent's pdt-mcp calls come over TCP).
async fn setup(rotate_age_ms: Option<i64>) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_program = FAKE.to_string();
    state.platform.codex_program = FAKE.to_string();
    state.platform.devin_program = FAKE.to_string();
    let devin_config = tmp.path().join("devin-user").join("config.json");
    std::fs::create_dir_all(devin_config.parent().unwrap()).unwrap();
    std::fs::write(&devin_config, "{}").unwrap();
    state.platform.devin_config_file = Some(devin_config);
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    let gh = std::sync::Arc::new(pdt_github::testing::FakeGithub::default());
    state.github = gh.clone();
    if let Some(ms) = rotate_age_ms {
        state.pm_rotate_age_ms = ms;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "P", "key": "PX", "repoPath": repo.to_string_lossy()})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    let (s, t) = call(
        &app,
        "POST",
        &format!("/api/projects/{project}/tickets"),
        Some(json!({"title": "Fix the login", "stage": "todo"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    Env { app, state, project, gh, _tmp: tmp }
}

async fn start_pm(env: &Env, text: &str) -> String {
    let (s, v) = call(&env.app, "POST", "/api/pm/sessions", Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

async fn send(env: &Env, session: &str, text: &str) {
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
}

async fn events(env: &Env, session: &str) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array().unwrap().clone()
}

async fn agent_says(env: &Env, session: &str) -> Vec<String> {
    events(env, session)
        .await
        .iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

/// Waits until the agent has said `n` things (ticking the engine, which delivers outbox messages); returns them.
async fn wait_said(env: &Env, session: &str, n: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let said = agent_says(env, session).await;
        if said.len() >= n {
            return said;
        }
        assert!(Instant::now() < deadline, "the agent said only {said:?}");
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Sends a message and returns what the agent said to it.
async fn ask(env: &Env, session: &str, text: &str) -> String {
    let n = agent_says(env, session).await.len();
    send(env, session, text).await;
    wait_said(env, session, n + 1).await.pop().unwrap()
}

async fn open_request(env: &Env, session: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let all = events(env, session).await;
        if let Some(e) = all.iter().find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open") {
            return e["event"]["id"].as_str().unwrap().to_string();
        }
        assert!(Instant::now() < deadline, "no open request");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn ticket_titles(env: &Env) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    let mut t: Vec<String> =
        v["items"].as_array().unwrap().iter().map(|t| t["title"].as_str().unwrap().into()).collect();
    t.sort();
    t
}

fn instructions(env: &Env, session: &str) -> String {
    std::fs::read_to_string(env.state.data_dir.join("sessions").join(session).join("instructions.md")).unwrap()
}

#[tokio::test]
async fn the_pm_answers_from_pdt_and_creates_tickets_after_a_yes() {
    let env = setup(None).await;
    let (_, pm) = call(&env.app, "GET", "/api/pm", None).await;
    assert_eq!(pm, json!({"current": null, "sessions": []}));

    let id = start_pm(&env, "who").await;
    let said = wait_said(&env, &id, 1).await;
    assert!(said[0].contains("instructions=true") && said[0].contains("mode=default"), "{said:?}");
    let (_, pm) = call(&env.app, "GET", "/api/pm", None).await;
    assert_eq!(pm["current"]["id"], id.as_str());
    assert_eq!(pm["current"]["persona"]["name"], "PM");
    // The host project is PDT's own: never listed.
    let (_, projects) = call(&env.app, "GET", "/api/projects", None).await;
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_ne!(pm["current"]["projectId"], env.project.as_str());

    let text = instructions(&env, &id);
    let in_repo = format!("- P (`PX`) in `{}`: To do 1", projects[0]["repoPath"].as_str().unwrap());
    for want in ["project manager", "# Briefing", &in_repo, "`create_ticket`", "## Your role"] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }

    let said = ask(&env, &id, r#"mcp: search_tickets {"query": "login"}"#).await;
    assert!(said.contains("Fix the login") && said.contains("\"key\": \"PX-1\""), "{said}");
    let said = ask(&env, &id, "mcp: get_overview {}").await;
    assert!(said.contains("\"openTickets\"") && said.contains("\"To do\": 1"), "{said}");
    let said = ask(&env, &id, "mcp: get_ticket {\"ticket\": \"PX-1\"}").await;
    assert!(said.contains("Fix the login"), "{said}");

    // Creating a ticket waits for the user's yes.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "Add a logout"}"#).await;
    let req = open_request(&env, &id).await;
    assert_eq!(ticket_titles(&env).await, ["Fix the login"], "nothing before the yes");
    let url = format!("/api/sessions/{id}/requests/{req}");
    let (s, _) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.starts_with("mcp create_ticket:") && said.contains("\"key\": \"PX-2\""), "{said}");
    assert_eq!(ticket_titles(&env).await, ["Add a logout", "Fix the login"]);
    let (_, t) = call(&env.app, "GET", "/api/tickets?project=&q=PX-2", None).await;
    assert_eq!(t["items"][0]["stage"], "draft");

    // A no leaves it undone.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "Not this"}"#).await;
    let req = open_request(&env, &id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "deny"}}))).await;
    assert_eq!(wait_said(&env, &id, n + 1).await.pop().unwrap(), "mcp create_ticket denied");
    assert_eq!(ticket_titles(&env).await.len(), 2);

    // A project that lets the PM act: PDT says yes for the user.
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["policy"]["pmActs"], true);
    let said = ask(&env, &id, r#"mcp: edit_ticket {"ticket": "PX-2", "stage": "ready"}"#).await;
    assert!(said.contains("\"updated\": true") && said.contains("\"stage\": \"ready\""), "{said}");

    // Messaging a session asks too (its project doesn't let the PM act), and a PM can't message itself.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, &format!(r#"mcp: send_to_session {{"session": "{id}", "text": "hi"}}"#)).await;
    let req = open_request(&env, &id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("that is this session"), "{said}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-100: every agent's PM runs in the work folder (the folder holding the projects' checkouts) with its agent's own
/// tools; Codex's also with live web search and the network in its sandbox.
#[tokio::test]
async fn the_pm_works_from_the_work_folder_with_its_agents_own_tools() {
    let env = setup(None).await;
    let work = env._tmp.path().canonicalize().unwrap();
    let said_cwd = |said: &str| {
        let dir = said.strip_prefix("cwd=").unwrap().split(' ').next().unwrap().to_string();
        std::path::PathBuf::from(dir).canonicalize().unwrap()
    };
    let (_, personas) = call(&env.app, "GET", "/api/personas", None).await;
    let pm = personas.as_array().unwrap().iter().find(|p| p["builtin"] == "pm").unwrap().clone();

    let id = start_pm(&env, "where").await;
    let said = wait_said(&env, &id, 1).await.pop().unwrap();
    assert_eq!(said_cwd(&said), work, "{said}");
    assert!(said.ends_with("tools=\"all\""), "Claude keeps its built-in tools: {said}");
    let text = instructions(&env, &id);
    let (_, projects) = call(&env.app, "GET", "/api/projects", None).await;
    let repo = std::path::PathBuf::from(projects[0]["repoPath"].as_str().unwrap());
    assert!(text.contains(&format!("from the work folder `{}`", repo.parent().unwrap().display())), "{text}");

    for (agent, want) in [("codex", " web_search=live network=true"), ("devin", "")] {
        let mut setup = pm["setup"].clone();
        setup["agent"] = json!(agent);
        let (s, v) = call(&env.app, "PUT", &format!("/api/personas/{}", pm["id"].as_str().unwrap()), Some(setup)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let id = start_pm(&env, "where").await;
        let said = wait_said(&env, &id, 1).await.pop().unwrap();
        assert_eq!(said_cwd(&said), work, "{agent}: {said}");
        assert!(said.ends_with(want), "{agent}: {said}");
        call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    }
}

#[tokio::test]
async fn notes_and_the_journal_carry_into_the_next_session() {
    let env = setup(None).await;
    let (s, n) = call(
        &env.app,
        "POST",
        "/api/pm/notes",
        Some(json!({"title": "Work hours", "body": "The user works 9 to 5.", "pinned": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{n}");
    let (_, found) = call(&env.app, "GET", "/api/pm/notes?q=work", None).await;
    assert_eq!(found[0]["title"], "Work hours");

    let first = start_pm(&env, "hello").await;
    wait_said(&env, &first, 1).await;
    let said = ask(
        &env,
        &first,
        r#"mcp: save_note {"project": "PX", "title": "Deploys", "body": "Deploy keys are in the vault.", "pinned": true}"#,
    )
    .await;
    assert!(said.contains("\"saved\": true"), "{said}");
    let said = ask(&env, &first, r#"mcp: search_notes {"query": "vault"}"#).await;
    assert!(said.contains("Deploys"), "{said}");
    let said = ask(&env, &first, r#"mcp: write_journal {"body": "We planned the PM's notes."}"#).await;
    assert!(said.contains("\"written\": true"), "{said}");

    let second = start_pm(&env, "hello again").await;
    wait_said(&env, &second, 1).await;
    let (_, pm) = call(&env.app, "GET", "/api/pm", None).await;
    assert_eq!(pm["current"]["id"], second.as_str());
    let old = &pm["sessions"][1];
    assert_eq!((old["session"]["id"].as_str(), old["rotation"].as_str()), (Some(first.as_str()), Some("done")));
    assert_eq!((old["reason"].as_str(), old["session"]["status"].as_str()), (Some("user"), Some("finished")));
    assert_eq!(old["journal"], "We planned the PM's notes.");
    let text = instructions(&env, &second);
    for want in ["We planned the PM's notes.", "host-wide: Work hours", "· PX: Deploys", "## Since the last PM session"]
    {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    call(&env.app, "POST", &format!("/api/sessions/{second}/kill"), None).await;
}

#[tokio::test]
async fn an_old_session_writes_its_journal_and_ends() {
    let env = setup(Some(0)).await;
    let id = start_pm(&env, "hello").await;
    // The first turn ends: PDT asks for the journal; the agent's answer to it ends the session.
    let said = wait_said(&env, &id, 2).await;
    assert!(said[1].contains("This PM session is ending"), "{said:?}");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let (_, pm) = call(&env.app, "GET", "/api/pm", None).await;
        // The session is done before its "(no journal)" is written: wait for both.
        if pm["current"].is_null() && !pm["sessions"][0]["journal"].is_null() {
            let s = &pm["sessions"][0];
            assert_eq!((s["rotation"].as_str(), s["reason"].as_str()), (Some("done"), Some("age")));
            assert_eq!(s["journal"], "(no journal)");
            break;
        }
        assert!(Instant::now() < deadline, "the session never ended: {pm}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn notes_are_checked_and_edited() {
    let env = setup(None).await;
    let (s, v) = call(&env.app, "POST", "/api/pm/notes", Some(json!({"title": " "}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = call(&env.app, "POST", "/api/pm/notes", Some(json!({"title": "x", "project": "NOPE"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (_, n) = call(&env.app, "POST", "/api/pm/notes", Some(json!({"title": "Style", "project": "PX"}))).await;
    assert_eq!(n["projectId"], env.project.as_str());
    let id = n["id"].as_str().unwrap();
    let (s, n) =
        call(&env.app, "PATCH", &format!("/api/pm/notes/{id}"), Some(json!({"project": "", "pinned": true}))).await;
    assert_eq!(s, StatusCode::OK, "{n}");
    assert_eq!((n["projectId"].clone(), n["pinned"].clone()), (Value::Null, json!(true)));
    let (s, _) = call(&env.app, "DELETE", &format!("/api/pm/notes/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&env.app, "DELETE", &format!("/api/pm/notes/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

fn issue(n: u64, title: &str, body: &str) -> pdt_github::Issue {
    pdt_github::Issue {
        number: n,
        title: title.into(),
        body: body.into(),
        url: format!("https://github.com/o/r/issues/{n}"),
        author: "al".into(),
        labels: vec!["bug".into()],
        created_at: "2026-10-20T00:00:00Z".into(),
    }
}

#[tokio::test]
async fn the_triage_template_has_the_pm_draft_a_ticket_for_a_new_issue() {
    let env = setup(None).await;
    let (s, w) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:issue-triage/instantiate",
        Some(json!({"params": {"repo": "o/r", "project": "PX"}, "scope": "project", "projectId": env.project})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap().to_string();
    call(&env.app, "PATCH", &format!("/api/workflows/{wf}"), Some(json!({"enabled": true}))).await;
    // The first look is the baseline; the next new issue starts a run.
    env.gh.set_issues("o/r", vec![issue(1, "Old", "")]);
    pdt_api::refresh_issues(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    env.gh.set_issues("o/r", vec![issue(2, "Login fails on Safari", "It spins forever."), issue(1, "Old", "")]);
    pdt_api::refresh_issues(&env.state).await;

    // The PM's step session starts with the issue and where its ticket goes.
    let deadline = Instant::now() + Duration::from_secs(20);
    let session = loop {
        pdt_api::run_workflows(&env.state).await;
        let (_, list) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.project), None).await;
        if let Some(s) = list.as_array().unwrap().iter().find(|s| s["persona"]["name"] == "PM") {
            break s["id"].as_str().unwrap().to_string();
        }
        assert!(Instant::now() < deadline, "no triage session");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let said = wait_said(&env, &session, 1).await;
    let first = events(&env, &session).await;
    let prompt = first.iter().find(|e| e["event"]["kind"] == "user_message").unwrap()["event"]["text"].clone();
    let prompt = prompt.as_str().unwrap();
    assert!(prompt.contains("#2 \"Login fails on Safari\"") && prompt.contains("It spins forever."), "{prompt}");
    assert!(prompt.contains("project PX") && prompt.contains("submit_outcome"), "{prompt}");

    // Drafting the ticket asks first; then the outcome ends the run by `drafted`.
    let n = said.len();
    send(&env, &session, r#"mcp: create_ticket {"project": "PX", "title": "Login fails on Safari"}"#).await;
    let req = open_request(&env, &session).await;
    let url = format!("/api/sessions/{session}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert!(wait_said(&env, &session, n + 1).await.pop().unwrap().contains("\"created\": true"));
    assert_eq!(ticket_titles(&env).await, ["Fix the login", "Login fails on Safari"]);
    send(&env, &session, r#"mcp: submit_outcome {"outcome": "drafted", "summary": "Drafted PX-2."}"#).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pdt_api::run_workflows(&env.state).await;
        let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await;
        let r = &runs[0];
        if r["status"] == "succeeded" {
            assert_eq!(r["output"]["summary"], "Drafted PX-2.", "{r}");
            break;
        }
        assert!(Instant::now() < deadline, "the run never ended: {r}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    call(&env.app, "POST", &format!("/api/sessions/{session}/kill"), None).await;
}

// ---- Discord (PDT-98) ----

/// One request the fake Discord got: method, path, JSON body, Authorization.
type Seen = (String, String, Value, String);

/// A fake Discord: one server (`42`, "Home") with `#general`; records every request (method, path, body, auth).
#[derive(Clone, Default)]
struct FakeDiscord {
    channels: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
    hooks: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
    seen: std::sync::Arc<std::sync::Mutex<Vec<Seen>>>,
}

impl FakeDiscord {
    fn answer(&self, method: &str, path: &str, body: &Value) -> (StatusCode, Value) {
        let mut channels = self.channels.lock().unwrap();
        let mut hooks = self.hooks.lock().unwrap();
        let parts: Vec<&str> = path.split('?').next().unwrap().trim_start_matches('/').split('/').collect();
        let next = || format!("{}", 100 + channels.len() + hooks.len());
        match (method, parts.as_slice()) {
            ("GET", ["guilds", "42"]) => (StatusCode::OK, json!({"id": "42", "name": "Home"})),
            ("GET", ["guilds", "42", "channels"]) => (StatusCode::OK, json!(*channels)),
            ("POST", ["guilds", "42", "channels"]) => {
                let c = json!({"id": next(), "name": body["name"], "type": body["type"],
                               "parent_id": body["parent_id"], "position": channels.len()});
                channels.push(c.clone());
                (StatusCode::CREATED, c)
            }
            ("PATCH", ["channels", id]) => match channels.iter_mut().find(|c| c["id"] == *id) {
                Some(c) => {
                    for (k, v) in body.as_object().unwrap() {
                        c[k] = v.clone();
                    }
                    (StatusCode::OK, c.clone())
                }
                None => (StatusCode::NOT_FOUND, json!({"message": "Unknown Channel"})),
            },
            ("GET", ["guilds", "42", "webhooks"]) => (StatusCode::OK, json!(*hooks)),
            ("POST", ["channels", id, "webhooks"]) => {
                let h = json!({"id": next(), "name": body["name"], "channel_id": id, "token": "SECRET-TOKEN"});
                hooks.push(h.clone());
                (StatusCode::OK, h)
            }
            ("POST", ["webhooks", _, "SECRET-TOKEN"]) => (StatusCode::OK, json!({"id": "m1"})),
            _ => (StatusCode::NOT_FOUND, json!({"message": "404: Not Found"})),
        }
    }

    async fn serve() -> (String, FakeDiscord) {
        use http_body_util::BodyExt;
        let fake = FakeDiscord::default();
        fake.channels.lock().unwrap().push(json!({"id": "1", "name": "general", "type": 0, "position": 0}));
        let f = fake.clone();
        let app = axum::Router::new().fallback(move |req: axum::http::Request<axum::body::Body>| {
            let f = f.clone();
            async move {
                let method = req.method().to_string();
                let path = req.uri().to_string();
                let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                let bytes = req.into_body().collect().await.unwrap().to_bytes();
                let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                let (status, v) = f.answer(&method, &path, &body);
                f.seen.lock().unwrap().push((method, path, body, auth));
                (status, axum::Json(v))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), fake)
    }

    fn channel(&self, name: &str) -> Option<Value> {
        self.channels.lock().unwrap().iter().find(|c| c["name"] == name).cloned()
    }
}

async fn connection_named(env: &Env, name: &str) -> Option<Value> {
    let (_, all) = call(&env.app, "GET", "/api/connections", None).await;
    all.as_array().unwrap().iter().find(|c| c["name"] == name).cloned()
}

#[tokio::test]
async fn the_pm_sets_up_a_discord_channel_and_webhook_that_a_workflow_posts_to() {
    let env = setup(None).await;
    let (base, discord) = FakeDiscord::serve().await;

    // A bot connection needs its server's id.
    let (s, e) = call(&env.app, "POST", "/api/connections", Some(json!({"name": "home", "kind": "discord_bot"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    assert!(e["message"].as_str().unwrap().contains("server's id"), "{e}");
    let (s, bot) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": "home", "kind": "discord_bot", "url": base, "config": {"guildId": "42"}})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{bot}");
    let bot_id = bot["id"].as_str().unwrap().to_string();
    let url = format!("/api/connections/{bot_id}/secret");
    call(&env.app, "PUT", &url, Some(json!({"secret": "bot-token"}))).await;

    let id = start_pm(&env, "hi").await;
    wait_said(&env, &id, 1).await;
    assert!(instructions(&env, &id).contains("`discord_webhook`"));

    // Looking needs no yes.
    let said = ask(&env, &id, "mcp: discord_get {}").await;
    assert!(said.contains("\"name\": \"Home\"") && said.contains("\"general\""), "{said}");
    assert!(discord.seen.lock().unwrap().iter().all(|r| r.3 == "Bot bot-token"));

    // "Set up a #ai-briefs channel with a webhook for the briefs workflow": the PM asks, the user says yes.
    let n = agent_says(&env, &id).await.len();
    let call_text = r##"mcp: discord_webhook {"channel": "#ai-briefs", "name": "briefs", "createChannel": true, "category": "Bots"}"##;
    send(&env, &id, call_text).await;
    let req = open_request(&env, &id).await;
    let title = events(&env, &id)
        .await
        .iter()
        .find(|e| e["event"]["kind"] == "request")
        .map(|e| e["event"]["title"].as_str().unwrap_or_default().to_string())
        .unwrap();
    assert_eq!(title, "Discord: a webhook on #ai-briefs (made if missing), saved as connection briefs");
    assert!(discord.channel("ai-briefs").is_none(), "nothing before the yes");
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("\"saved\": true") && said.contains("\"channelCreated\": true"), "{said}");
    assert!(!said.contains("SECRET-TOKEN"), "the webhook's URL never reaches the agent: {said}");
    let category = discord.channel("Bots").expect("the category");
    let channel = discord.channel("ai-briefs").expect("the channel");
    assert_eq!(channel["parent_id"], category["id"]);
    let saved = connection_named(&env, "briefs").await.expect("the connection");
    assert_eq!((saved["kind"].as_str(), saved["hasSecret"].as_bool()), (Some("discord_webhook"), Some(true)));
    assert!(!saved.to_string().contains("SECRET-TOKEN"));

    // discord_get says which connection the webhook is.
    let said = ask(&env, &id, "mcp: discord_get {}").await;
    assert!(said.contains("\"savedAs\": \"briefs\"") && !said.contains("SECRET-TOKEN"), "{said}");

    // A workflow posts through it.
    let doc = json!({"name": "Briefs", "nodes": [{"id": "go", "kind": "manual", "version": 1},
        {"id": "post", "kind": "discord.post", "version": 1, "config": {"connection": "briefs", "content": "Hello"}}],
        "edges": [{"from": "go", "to": "post"}]});
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    pdt_api::run_workflows(&env.state).await;
    let (_, run) = call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["runId"].as_str().unwrap()), None).await;
    assert_eq!(run["status"], "succeeded", "{run}");
    let hook = discord.hooks.lock().unwrap()[0]["id"].as_str().unwrap().to_string();
    let posted = discord.seen.lock().unwrap().iter().find(|r| r.1.contains("/webhooks/")).cloned().unwrap();
    assert!(posted.1.starts_with(&format!("/webhooks/{hook}/SECRET-TOKEN")), "{posted:?}");
    assert_eq!(posted.2["content"], "Hello");

    // A name already taken is refused before anything is made.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: discord_webhook {"channel": "general", "name": "briefs"}"#).await;
    let req = open_request(&env, &id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("already called briefs"), "{said}");
    assert_eq!(discord.hooks.lock().unwrap().len(), 1);

    // A bot connection that lets the PM act: PDT says yes for the user.
    let (s, v) = call(
        &env.app,
        "PATCH",
        &format!("/api/connections/{bot_id}"),
        Some(json!({"config": {"guildId": "42", "pmActs": true}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let said = ask(
        &env,
        &id,
        r##"mcp: discord_change {"changes": [{"op": "rename", "channel": "#ai-briefs", "name": "Morning Briefs"}, {"op": "archive", "channel": "general"}]}"##,
    )
    .await;
    assert!(said.contains("\"renamed\"") && said.contains("\"archived\""), "{said}");
    assert!(discord.channel("morning-briefs").is_some());
    let archive = discord.channel("Archive").expect("the Archive category");
    assert_eq!(discord.channel("general").unwrap()["parent_id"], archive["id"]);

    // A change that fails says what was done and what wasn't.
    let said = ask(
        &env,
        &id,
        r#"mcp: discord_change {"changes": [{"op": "create", "name": "notes"}, {"op": "move", "channel": "nope"}, {"op": "create", "name": "later"}]}"#,
    )
    .await;
    assert!(
        said.contains("\"failed\"") && said.contains("no channel called nope") && said.contains("create #later"),
        "{said}"
    );
    assert!(discord.channel("notes").is_some() && discord.channel("later").is_none());
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}
