//! A ticket's attachments (PDT-300): uploaded from the UI, attached by agents and the PM by local path, listed with
//! their paths in `get_ticket`, and removed with the ticket. `fake-agent` stands in for Claude (it calls pdt-mcp tools
//! when a message says `mcp: <tool> <json>`).

mod common;

use axum::http::StatusCode;
use common::{call, call_raw, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDRfake";

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    tmp: tempfile::TempDir,
}

/// A project with a repo, served on a free local port (the agent's pdt-mcp calls come over TCP).
async fn setup() -> Env {
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
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, project: p["id"].as_str().unwrap().into(), tmp }
}

async fn ticket(env: &Env, title: &str) -> Value {
    let (s, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": title}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t
}

async fn upload(env: &Env, ticket: &str, name: &str, body: &[u8]) -> (StatusCode, Value) {
    let uri = format!("/api/tickets/{ticket}/attachments?name={name}&by=user");
    let (s, _, bytes) = call_raw(&env.app, "POST", &uri, body.to_vec()).await;
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn attachments_dir(env: &Env, ticket: &str) -> std::path::PathBuf {
    env.state.data_dir.join("tickets").join(ticket).join("attachments")
}

#[tokio::test]
async fn the_user_uploads_lists_reads_and_removes_attachments() {
    let env = setup().await;
    let t = ticket(&env, "Bug").await;
    let (id, key) = (t["id"].as_str().unwrap(), t["key"].as_str().unwrap());

    let (s, img) = upload(&env, id, "shot.png", PNG).await;
    assert_eq!(s, StatusCode::CREATED, "{img}");
    assert_eq!(
        (&img["mediaType"], &img["name"], &img["actor"]),
        (&json!("image/png"), &json!("shot.png"), &json!("user"))
    );
    // By key too (PDT-271); the bytes decide the type, the name only a text file's kind.
    let (s, log) = upload(&env, key, "build.log", b"error: boom\n").await;
    assert_eq!(s, StatusCode::CREATED, "{log}");
    assert_eq!(log["mediaType"], "text/plain");
    let (s, html) = upload(&env, id, "page.html", b"<script>alert(1)</script>").await;
    assert_eq!((s, &html["mediaType"]), (StatusCode::CREATED, &json!("text/plain")));

    let (s, list) = call(&env.app, "GET", &format!("/api/tickets/{key}/attachments"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 3);

    let url = |a: &Value| format!("/api/tickets/{id}/attachments/{}", a["id"].as_str().unwrap());
    let (s, ct, bytes) = call_raw(&env.app, "GET", &url(&img), vec![]).await;
    assert_eq!((s, ct.as_deref(), bytes.as_slice()), (StatusCode::OK, Some("image/png"), PNG));
    // Text is served as plain text, never as a page.
    let (_, ct, bytes) = call_raw(&env.app, "GET", &url(&html), vec![]).await;
    assert_eq!(ct.as_deref(), Some("text/plain; charset=utf-8"));
    assert_eq!(bytes, b"<script>alert(1)</script>");

    let (s, _) = call(&env.app, "DELETE", &url(&log), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = call_raw(&env.app, "GET", &url(&log), vec![]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(std::fs::read_dir(attachments_dir(&env, id)).unwrap().count(), 2);
}

#[tokio::test]
async fn binaries_oversized_files_and_too_many_are_refused() {
    let env = setup().await;
    let t = ticket(&env, "Bug").await;
    let id = t["id"].as_str().unwrap();
    let (s, e) = upload(&env, id, "a.bin", b"\x00\x01\x02\x03").await;
    assert_eq!((s, &e["code"]), (StatusCode::BAD_REQUEST, &json!("unsupported_attachment")));
    let (s, e) = upload(&env, id, "big.txt", &vec![b'a'; 1024 * 1024 + 1]).await;
    assert_eq!((s, &e["code"]), (StatusCode::PAYLOAD_TOO_LARGE, &json!("attachment_too_large")));
    let (s, _) = upload(&env, "no-such-ticket", "a.txt", b"hi").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    for i in 0..pdt_core::ticket_attachment::MAX_PER_TICKET {
        let (s, e) = upload(&env, id, &format!("{i}.txt"), b"hi").await;
        assert_eq!(s, StatusCode::CREATED, "{e}");
    }
    let (s, e) = upload(&env, id, "over.txt", b"hi").await;
    assert_eq!((s, &e["code"]), (StatusCode::CONFLICT, &json!("too_many_attachments")));
    assert_eq!(std::fs::read_dir(attachments_dir(&env, id)).unwrap().count(), 50, "the refused file is not kept");
}

#[tokio::test]
async fn attachments_go_with_their_ticket() {
    let env = setup().await;
    let gone = ticket(&env, "Gone").await;
    let kept = ticket(&env, "Kept").await;
    let (gone, kept) = (gone["id"].as_str().unwrap(), kept["id"].as_str().unwrap());
    upload(&env, gone, "a.png", PNG).await;
    upload(&env, kept, "b.png", PNG).await;
    let (s, _) = call(&env.app, "DELETE", &format!("/api/tickets/{gone}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(!env.state.data_dir.join("tickets").join(gone).exists());
    assert!(attachments_dir(&env, kept).exists());
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

/// Sends the agent `text` and waits for its answer to the tool call it makes.
async fn say(env: &Env, session: &str, text: &str) -> String {
    let before = agent_says(env, session).await.len();
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let said = agent_says(env, session).await;
        if let Some(m) = said.iter().skip(before).find(|m| m.starts_with("mcp ")) {
            return m.clone();
        }
        assert!(Instant::now() < deadline, "the agent said only {said:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn answer(said: &str, tool: &str) -> Value {
    let body = said.strip_prefix(&format!("mcp {tool}: ")).unwrap_or(said);
    serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {said}"))
}

#[tokio::test]
async fn an_agent_attaches_files_by_path_and_reads_them_back_from_get_ticket() {
    let env = setup().await;
    let t = ticket(&env, "Bug").await;
    let id = t["id"].as_str().unwrap();
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{id}/start"),
        Some(json!({"agent": "claude", "sessionMode": "conversation"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let session = started["session"]["id"].as_str().unwrap().to_string();

    let shots = env.tmp.path().join("shots");
    std::fs::create_dir_all(&shots).unwrap();
    let shot = shots.join("qa.png");
    std::fs::write(&shot, PNG).unwrap();
    let call_json = json!({"body": "Here it is.", "files": [shot.to_string_lossy()]});
    let said = say(&env, &session, &format!("mcp: add_ticket_comment {call_json}")).await;
    let got = answer(&said, "add_ticket_comment");
    assert_eq!(got["commented"], true, "{said}");
    let aid = got["attachments"][0]["id"].as_str().unwrap().to_string();

    // The comment shows the image inline and names it.
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{id}/events"), None).await;
    let comment = events["items"].as_array().unwrap().iter().find(|e| e["kind"] == "comment").unwrap().clone();
    assert_eq!(comment["body"], format!("Here it is.\n\n![qa.png](/api/tickets/{id}/attachments/{aid})"));
    assert_eq!(comment["data"]["attachments"], json!([aid]));

    // Images the user pasted into the conversation are `pasted:N`.
    let (s, _, _) = call_raw(&env.app, "POST", &format!("/api/sessions/{session}/attachments"), PNG.to_vec()).await;
    assert_eq!(s, StatusCode::CREATED);
    let said = say(&env, &session, r#"mcp: add_ticket_comment {"body": "Pasted.", "files": ["pasted:1"]}"#).await;
    assert_eq!(answer(&said, "add_ticket_comment")["commented"], true, "{said}");

    let said = say(&env, &session, "mcp: get_ticket {}").await;
    let got = answer(&said, "get_ticket");
    let list = got["attachments"].as_array().unwrap_or_else(|| panic!("{got:#}"));
    assert_eq!(list.len(), 2);
    assert_eq!(
        (&list[0]["name"], &list[0]["mediaType"], &list[0]["by"]),
        (&json!("qa.png"), &json!("image/png"), &json!("agent"))
    );
    let path = std::path::PathBuf::from(list[0]["path"].as_str().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), PNG, "the agent can open the file at its path");
    assert!(got["attachmentsNote"].as_str().unwrap().contains("open the images"));

    // Bad paths change nothing.
    let secrets = env.state.data_dir.join("secrets.json");
    std::fs::create_dir_all(&env.state.data_dir).unwrap();
    std::fs::write(&secrets, "{}").unwrap();
    let dotenv = env.tmp.path().join("repo").join(".env");
    std::fs::write(&dotenv, "TOKEN=x\n").unwrap();
    let ssh = env.tmp.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("id_ed25519"), "key\n").unwrap();
    let binary = shots.join("a.bin");
    std::fs::write(&binary, b"\x00\x01").unwrap();
    for (bad, why) in [
        (secrets.to_string_lossy().to_string(), "PDT's own data"),
        (dotenv.to_string_lossy().to_string(), "dotfiles"),
        (ssh.join("id_ed25519").to_string_lossy().to_string(), "key and credential"),
        ("relative.png".to_string(), "absolute"),
        (shots.to_string_lossy().to_string(), "not a file"),
        (binary.to_string_lossy().to_string(), "only images"),
        (shot.to_string_lossy().to_string() + ".missing", "can't read"),
    ] {
        let call_json = json!({"body": "x", "files": [shot.to_string_lossy(), bad]});
        let said = say(&env, &session, &format!("mcp: add_ticket_comment {call_json}")).await;
        assert!(said.contains(why), "{why}: {said}");
    }
    let (_, list) = call(&env.app, "GET", &format!("/api/tickets/{id}/attachments"), None).await;
    assert_eq!(list.as_array().unwrap().len(), 2, "nothing was attached from a call that named a bad file");
}
