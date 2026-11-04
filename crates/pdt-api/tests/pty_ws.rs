use futures_util::{SinkExt, StreamExt};
use pdt_api::{AppState, router, with_host_guard};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const WAIT: Duration = Duration::from_secs(30);

/// Serves the guarded router on a random port. Returns (port, state).
async fn serve(tmp: &Path) -> (u16, AppState) {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = crate::common::app_state(store, tmp);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = with_host_guard(router(state.clone(), None), vec![format!("127.0.0.1:{port}")]);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (port, state)
}

/// Minimal HTTP/1.0 JSON client (no extra dependencies).
async fn http(port: u16, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

async fn shell_session(tmp: &Path, port: u16) -> String {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    crate::common::init_repo(&repo);
    let (_, p) =
        http(port, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    let (status, s) = http(
        port,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": p["id"], "kind": "shell", "cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!(status, 201, "{s}");
    s["id"].as_str().unwrap().to_string()
}

/// Typed text contains "4000+2000+21"; only the shell's evaluated output contains "6021".
const PROBE: &str = if cfg!(windows) { "echo (4000+2000+21)\r" } else { "echo $((4000+2000+21))\r" };

async fn read_until<S>(ws: &mut S, needle: &str) -> String
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut out = String::new();
    tokio::time::timeout(WAIT, async {
        while let Some(Ok(msg)) = ws.next().await {
            match msg {
                Message::Binary(b) => out.push_str(&String::from_utf8_lossy(&b)),
                Message::Text(t) => out.push_str(t.as_str()),
                _ => {}
            }
            if out.contains(needle) {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}; got {out:?}"));
    out
}

/// Reads binary frames up to the first text frame. Returns (binary bytes as text, the text frame).
async fn until_first_text<S>(ws: &mut S) -> (String, String)
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut out = String::new();
    tokio::time::timeout(WAIT, async {
        while let Some(Ok(msg)) = ws.next().await {
            match msg {
                Message::Binary(b) => out.push_str(&String::from_utf8_lossy(&b)),
                Message::Text(t) => return (out, t.as_str().to_string()),
                _ => {}
            }
        }
        panic!("socket closed before a text frame; got {out:?}");
    })
    .await
    .expect("timed out waiting for a text frame")
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_streams_input_output_and_exit() {
    let tmp = tempfile::tempdir().unwrap();
    let (port, state) = serve(tmp.path()).await;
    let id = shell_session(tmp.path(), port).await;
    let url = format!("ws://127.0.0.1:{port}/api/sessions/{id}/pty");

    let (mut ws, _) = connect_async(&url).await.unwrap();
    ws.send(Message::Text(r#"{"type":"resize","cols":100,"rows":30}"#.into())).await.unwrap();
    ws.send(Message::Binary(PROBE.as_bytes().to_vec().into())).await.unwrap();
    read_until(&mut ws, "6021").await;

    // second_client_gets_scrollback: a late client sees earlier output immediately, then the ready marker.
    let (mut ws2, _) = connect_async(&url).await.unwrap();
    let (replay, first_text) = until_first_text(&mut ws2).await;
    assert!(replay.contains("6021"), "{replay:?}");
    assert_eq!(first_text, r#"{"type":"ready"}"#);

    let (status, _) = http(port, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert_eq!(status, 204);
    let tail = read_until(&mut ws, "\"type\":\"exit\"").await;
    assert!(tail.contains("\"code\""), "{tail}");
    state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_asks_gets_the_scrollback_gzipped() {
    use std::io::Read;
    let tmp = tempfile::tempdir().unwrap();
    let (port, state) = serve(tmp.path()).await;
    let id = shell_session(tmp.path(), port).await;
    let url = format!("ws://127.0.0.1:{port}/api/sessions/{id}/pty");
    let (mut ws, _) = connect_async(&url).await.unwrap();
    ws.send(Message::Binary(PROBE.as_bytes().to_vec().into())).await.unwrap();
    read_until(&mut ws, "6021").await;

    let (mut gz, _) = connect_async(format!("{url}?replay=gzip")).await.unwrap();
    let mut frames = Vec::new();
    let ready = tokio::time::timeout(WAIT, async {
        loop {
            match gz.next().await.unwrap().unwrap() {
                Message::Binary(b) => frames.push(b.to_vec()),
                Message::Text(t) => return t.to_string(),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(ready, r#"{"type":"ready"}"#);
    assert_eq!(frames.len(), 1, "the replay is one gzip frame");
    let mut replay = String::new();
    flate2::read::GzDecoder::new(&frames[0][..]).read_to_string(&mut replay).unwrap();
    assert!(replay.contains("6021"), "{replay:?}");

    // Live output after `ready` is never compressed.
    gz.send(Message::Binary(PROBE.as_bytes().to_vec().into())).await.unwrap();
    read_until(&mut gz, "6021").await;
    state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_rejects_foreign_origin_and_unknown_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let (port, state) = serve(tmp.path()).await;
    let id = shell_session(tmp.path(), port).await;

    let mut req = format!("ws://127.0.0.1:{port}/api/sessions/{id}/pty").into_client_request().unwrap();
    req.headers_mut().insert("origin", "http://evil.example".parse().unwrap());
    match connect_async(req).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => assert_eq!(res.status(), 403),
        other => panic!("expected 403, got {other:?}"),
    }

    match connect_async(format!("ws://127.0.0.1:{port}/api/sessions/nope/pty")).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => assert_eq!(res.status(), 404),
        other => panic!("expected 404, got {other:?}"),
    }

    let (status, body) = http(port, "GET", &format!("/api/sessions/{id}/pty"), None).await;
    assert_eq!((status, body["code"].as_str().unwrap()), (400, "websocket_required"));
    state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn session_without_a_terminal_reports_not_running_then_closes() {
    use pdt_core::session::SessionStatus;
    let tmp = tempfile::tempdir().unwrap();
    let (port, state) = serve(tmp.path()).await;
    let id = shell_session(tmp.path(), port).await;
    // A session left interrupted by a daemon restart has no terminal at all.
    let mut orphan = state.store.get_session(&id).await.unwrap().unwrap();
    orphan.id = "orphan".into();
    orphan.status = SessionStatus::Interrupted;
    state.store.insert_session(&orphan).await.unwrap();

    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/api/sessions/orphan/pty")).await.unwrap();
    let (replay, text) = until_first_text(&mut ws).await;
    assert_eq!((replay.as_str(), text.as_str()), ("", r#"{"type":"not_running"}"#));
    let end = tokio::time::timeout(WAIT, async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                _ => {}
            }
        }
    })
    .await;
    assert!(end.is_ok(), "the server closes after not_running");
    state.ptys.shutdown().await;
}

/// An interrupted session (no terminal) that the startup plan will restart, plus the held plan.
async fn pending_session(tmp: &Path, port: u16, state: &AppState, id: &str, cwd: Option<&Path>) -> pdt_api::ResumePlan {
    use pdt_core::session::SessionStatus;
    let template = shell_session(tmp, port).await;
    let mut s = state.store.get_session(&template).await.unwrap().unwrap();
    s.id = id.into();
    s.status = SessionStatus::Interrupted;
    if let Some(c) = cwd {
        s.cwd = c.to_string_lossy().into_owned();
    }
    state.store.insert_session(&s).await.unwrap();
    pdt_api::plan_resume_interrupted(state).await
}

async fn next_message<S>(ws: &mut S, within: Duration) -> Option<Message>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    match tokio::time::timeout(within, ws.next()).await {
        Ok(Some(Ok(m))) => Some(m),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_connecting_during_auto_resume_attaches_once_the_session_starts() {
    let tmp = tempfile::tempdir().unwrap();
    let (port, state) = serve(tmp.path()).await;
    let plan = pending_session(tmp.path(), port, &state, "pending", None).await;

    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/api/sessions/pending/pty")).await.unwrap();
    assert!(next_message(&mut ws, Duration::from_millis(500)).await.is_none(), "it waits, saying nothing");
    assert!(!state.ptys.is_running("pending"));

    assert_eq!(plan.start(&state).await, 1);
    let (_, text) = until_first_text(&mut ws).await;
    assert_eq!(text, r#"{"type":"ready"}"#, "attached to the resumed terminal, not told it is not running");
    ws.send(Message::Binary(PROBE.as_bytes().to_vec().into())).await.unwrap();
    read_until(&mut ws, "6021").await;
    state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_waiting_for_a_resume_that_fails_is_told_not_running() {
    let tmp = tempfile::tempdir().unwrap();
    let (port, state) = serve(tmp.path()).await;
    let gone = tmp.path().join("deleted folder");
    let plan = pending_session(tmp.path(), port, &state, "doomed", Some(&gone)).await;

    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/api/sessions/doomed/pty")).await.unwrap();
    assert!(next_message(&mut ws, Duration::from_millis(300)).await.is_none());
    assert_eq!(plan.start(&state).await, 0);
    let (_, text) = until_first_text(&mut ws).await;
    assert_eq!(text, r#"{"type":"not_running"}"#);
    state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_releases_terminals_waiting_for_a_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let (port, state) = serve(tmp.path()).await;
    let _plan = pending_session(tmp.path(), port, &state, "held", None).await;

    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/api/sessions/held/pty")).await.unwrap();
    assert!(next_message(&mut ws, Duration::from_millis(300)).await.is_none());
    state.shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);
    state.wake_resume_waiters();
    let (_, text) = until_first_text(&mut ws).await;
    assert_eq!(text, r#"{"type":"not_running"}"#);
    state.ptys.shutdown().await;
}

/// Prints the terminal size as "<rows> <cols>" on unix and "<cols*1000+rows>" on Windows.
const SIZE_PROBE: &str =
    if cfg!(windows) { "echo ([Console]::WindowWidth*1000+[Console]::WindowHeight)\r" } else { "stty size\r" };
const SIZE_100X30: &str = if cfg!(windows) { "100030" } else { "30 100" };

#[tokio::test(flavor = "multi_thread")]
async fn the_size_sent_while_waiting_is_applied_to_the_resumed_terminal() {
    let tmp = tempfile::tempdir().unwrap();
    let (port, state) = serve(tmp.path()).await;
    let plan = pending_session(tmp.path(), port, &state, "sized", None).await;

    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/api/sessions/sized/pty")).await.unwrap();
    ws.send(Message::Text(r#"{"type":"resize","cols":100,"rows":30}"#.into())).await.unwrap();
    assert!(next_message(&mut ws, Duration::from_millis(400)).await.is_none());
    // The resume starts the terminal at 120x30; the size the browser sent while waiting must win.
    assert_eq!(plan.start(&state).await, 1);
    let (_, text) = until_first_text(&mut ws).await;
    assert_eq!(text, r#"{"type":"ready"}"#);
    ws.send(Message::Binary(SIZE_PROBE.as_bytes().to_vec().into())).await.unwrap();
    read_until(&mut ws, SIZE_100X30).await;
    state.ptys.shutdown().await;
}
