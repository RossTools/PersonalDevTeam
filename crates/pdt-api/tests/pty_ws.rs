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
    let state = AppState::new(store);
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
    assert!(
        std::process::Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success()
    );
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
