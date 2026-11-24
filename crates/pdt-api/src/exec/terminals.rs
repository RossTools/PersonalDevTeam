//! The owner's end of remote terminals (PDT-633): starting and stopping a terminal session's PTY on its instance,
//! typing into it, its exit (it comes back with the events), the browser's terminal bridged to it, and the agent's
//! hooks the remote instance passes on.

use super::owner::{Route, call, ensure_poller, instance};
use super::*;
use crate::error::{ApiError, codes};
use crate::link::{self, PeerError};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, Method};
use axum::response::Response;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use pdt_core::link::RemoteInstance;
use pdt_core::session::{PtyServerMessage, Session, SessionKind, SessionStatus, TerminalSize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio_tungstenite::tungstenite;

/// How long starting a terminal there may take; one in the checkout may clone it first.
const OPEN_TIMEOUT: Duration = Duration::from_secs(120);
const CLONE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long a keystroke-sized call may take.
const SMALL_TIMEOUT: Duration = Duration::from_secs(10);

/// Whether `id` is a terminal session whose PTY runs on a remote instance now.
pub(crate) fn is_remote_terminal(st: &AppState, id: &str) -> bool {
    st.exec.owner.routes().get(id).is_some_and(|r| r.sink.is_none() && r.alive.load(Ordering::SeqCst))
}

/// Starts a terminal session's PTY on its remote instance: `fresh` at creation (with its first `prompt`), else a
/// restart that resumes the agent's conversation when the remote instance has it. The remote decides the folder (the
/// checkout or the worktree's), and the row takes it.
pub(crate) async fn open(
    st: &AppState,
    s: &Session,
    size: TerminalSize,
    fresh: bool,
    prompt: Option<String>,
    model: Option<String>,
) -> Result<(), ApiError> {
    let on = s.instance.as_ref().ok_or_else(|| ApiError::internal("not a remote session"))?;
    let i = instance(st, &s.project_id, &on.id).await?;
    let project = st.project(&s.project_id).await?;
    let branch = match &s.worktree_id {
        Some(w) => st.store.get_worktree(w).await?.map(|w| w.branch),
        None => None,
    };
    let instructions = crate::sessions::agent_instructions(st, s).await?.map(|ins| WireInstructions {
        file_name: ins.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or("instructions.md".into()),
        text: ins.text,
    });
    let mcp = crate::mcp::server_for(st, s).await.map(|m| WireMcp {
        token: m.token,
        tools: m.tools,
        asks: m.asks,
        pm: m.pm,
        locked: m.locked,
        only_pdt: m.only_pdt,
    });
    let timeout = if branch.is_none() { CLONE_TIMEOUT } else { OPEN_TIMEOUT };
    let req = PtyOpenReq {
        session: s.clone(),
        ticket_key: s.ticket.as_ref().map(|t| t.key.clone()),
        branch,
        base: project.base_branch.clone(),
        cols: size.cols,
        rows: size.rows,
        fresh,
        prompt,
        model,
        credentials: crate::sessions::credentials_for(st, &s.project_id).await?,
        instructions,
        mcp,
    };
    let alive = Arc::new(AtomicBool::new(true));
    st.exec
        .owner
        .routes()
        .insert(s.id.clone(), Route { sink: None, proc: None, alive, link_id: i.link_id.clone(), opened_at: 0 });
    ensure_poller(st, &i.link_id);
    let opened: Opened = match call(st, &i, PTY_OPEN_PATH, &req, timeout).await {
        Ok(o) => o,
        Err(e) => {
            st.exec.owner.routes().remove(&s.id);
            return Err(e.into_api(&i.name));
        }
    };
    if let Some(r) = st.exec.owner.routes().get_mut(&s.id) {
        r.proc = Some(opened.proc);
        r.opened_at = now_ms();
    }
    if opened.cwd != s.cwd {
        st.store.set_session_cwd(&s.id, &opened.cwd).await?;
    }
    crate::activity::track_remote(st, s).await;
    Ok(())
}

/// Stops a remote terminal (its exit comes back with the events). Not running there: nothing to do.
pub(crate) async fn close(st: &AppState, s: &Session) -> Result<(), ApiError> {
    let Some(on) = &s.instance else { return Ok(()) };
    if !is_remote_terminal(st, &s.id) {
        return Ok(());
    }
    let i = instance(st, &s.project_id, &on.id).await?;
    st.notifier.expect_exit(&s.id);
    let _: Value = call(st, &i, CLOSE_PATH, &SessionReq { session_id: s.id.clone() }, OPEN_TIMEOUT)
        .await
        .map_err(|e| e.into_api(&i.name))?;
    Ok(())
}

/// Types into a remote terminal.
pub(crate) async fn write(st: &AppState, id: &str, bytes: &[u8]) -> Result<(), pdt_agents::PtyError> {
    let not_running = || pdt_agents::PtyError::NotRunning(id.to_string());
    let s = st.store.get_session(id).await.ok().flatten().ok_or_else(not_running)?;
    let on = s.instance.as_ref().ok_or_else(not_running)?;
    let i = instance(st, &s.project_id, &on.id).await.map_err(|_| not_running())?;
    let req = PtyWriteReq { session_id: id.to_string(), data: base64::engine::general_purpose::STANDARD.encode(bytes) };
    let _: Value = call(st, &i, PTY_WRITE_PATH, &req, SMALL_TIMEOUT).await.map_err(|e| match e {
        PeerError::Refused { code, .. } if code == codes::AGENT_NOT_RUNNING => not_running(),
        e => pdt_agents::PtyError::Io(format!("{}: {}", i.name, e.text())),
    })?;
    Ok(())
}

/// A remote terminal ended (or its instance no longer runs it: `code` None): the session ends as a local terminal's
/// does.
pub(super) async fn exited(st: &AppState, id: &str, code: Option<i32>) {
    crate::activity::untrack(st, id).await;
    let Ok(Some(s)) = st.store.get_session(id).await else { return };
    if s.status != SessionStatus::Running {
        return;
    }
    if let Err(e) = st.store.end_session(id, SessionStatus::Finished, code, now_ms()).await {
        tracing::warn!(session = %id, error = %e, "failed to record a remote terminal's exit");
    }
    let stopped = st.notifier.take_expected_exit(id);
    if let Some(code) = code {
        crate::notifications::session_exited(st, id, code, stopped).await;
    }
    crate::workflow::sensors::session_exited(st, id, code, stopped).await;
}

/// The remote instance found the Codex conversation a terminal ran (in its own Codex files): restarts resume it.
pub(super) async fn conversation_found(st: &AppState, id: &str, conversation: &str) {
    match st.store.bind_external_session_id(id, conversation).await {
        Ok(true) => tracing::info!(session = %id, conversation, "bound a remote terminal to its Codex conversation"),
        Ok(false) => {}
        Err(e) => tracing::warn!(session = %id, error = %e, "could not record a remote terminal's Codex conversation"),
    }
}

/// After the owner restarts: the terminals still running on `i`, so their exits end them and typing reaches them.
pub(super) async fn resume_routes(st: &AppState, i: &RemoteInstance) {
    let Ok(list) = st.store.list_sessions(&i.project_id).await else { return };
    for s in list {
        let ours = s.instance.as_ref().is_some_and(|r| r.id == i.instance_id);
        if !ours || s.status != SessionStatus::Running || s.mode == pdt_core::conversation::SessionMode::Conversation {
            continue;
        }
        let route = Route {
            sink: None,
            proc: None,
            alive: Arc::new(AtomicBool::new(true)),
            link_id: i.link_id.clone(),
            opened_at: now_ms(),
        };
        st.exec.owner.routes().entry(s.id.clone()).or_insert(route);
        crate::activity::track_remote(st, &s).await;
    }
}

/// The browser's terminal for a remote session: bridged, frame for frame, to the PTY's WebSocket on its instance (the
/// same protocol as a local one, so the scrollback, gzip, resize and exit pass through). A session not running there
/// gets `not_running`; an instance that can't be reached closes the socket, so the browser reconnects.
pub(crate) fn bridge(st: AppState, s: Session, gzip: bool, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| async move { bridge_socket(socket, st, s, gzip).await })
}

async fn not_running(mut socket: WebSocket) {
    let msg = serde_json::to_string(&PtyServerMessage::NotRunning).expect("serializable");
    let _ = socket.send(Message::Text(msg.into())).await;
    let _ = socket.close().await;
}

/// Closes the browser's socket so it tries again (the instance can't be reached).
async fn unreachable(mut socket: WebSocket, why: &str) {
    let frame = CloseFrame { code: 1011, reason: why.chars().take(100).collect::<String>().into() };
    let _ = socket.send(Message::Close(Some(frame))).await;
}

/// `http://host:port` → `ws://host:port<path>`.
fn ws_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let rest = base.strip_prefix("https://").map(|r| format!("wss://{r}"));
    let rest = rest.or_else(|| base.strip_prefix("http://").map(|r| format!("ws://{r}")));
    format!("{}{path}", rest.unwrap_or_else(|| base.to_string()))
}

async fn bridge_socket(socket: WebSocket, st: AppState, s: Session, gzip: bool) {
    let Some(on) = s.instance.clone() else { return not_running(socket).await };
    if s.status != SessionStatus::Running || !is_remote_terminal(&st, &s.id) {
        return not_running(socket).await;
    }
    let i = match instance(&st, &s.project_id, &on.id).await {
        Ok(i) => i,
        Err(_) => return not_running(socket).await,
    };
    let ticket: PtyTicket =
        match call(&st, &i, PTY_ATTACH_PATH, &SessionReq { session_id: s.id.clone() }, SMALL_TIMEOUT).await {
            Ok(t) => t,
            Err(PeerError::Refused { code, .. }) if code == codes::AGENT_NOT_RUNNING => {
                return not_running(socket).await;
            }
            Err(e) => return unreachable(socket, &e.text()).await,
        };
    let mut url = ws_url(&i.url, &format!("{PTY_WS_PATH}/{}", ticket.ticket));
    if gzip {
        url.push_str("?replay=gzip");
    }
    // Nagle off: a keystroke goes out at once, not with the next one.
    let remote = match tokio_tungstenite::connect_async_with_config(url, None, true).await {
        Ok((r, _)) => r,
        Err(e) => return unreachable(socket, &e.to_string()).await,
    };
    tracing::debug!(session = %s.id, instance = %i.name, "bridged a terminal to its instance");
    let (mut to_browser, mut from_browser) = socket.split();
    let (mut to_remote, mut from_remote) = remote.split();
    let down = async {
        while let Some(Ok(msg)) = from_remote.next().await {
            let out = match msg {
                tungstenite::Message::Binary(b) => Message::Binary(b),
                tungstenite::Message::Text(t) => Message::Text(t.as_str().into()),
                tungstenite::Message::Close(frame) => {
                    let frame = frame.map(|f| CloseFrame { code: f.code.into(), reason: f.reason.as_str().into() });
                    let _ = to_browser.send(Message::Close(frame)).await;
                    return;
                }
                _ => continue,
            };
            if to_browser.send(out).await.is_err() {
                return;
            }
        }
        // The instance went away mid-stream: the browser reconnects.
        let frame = CloseFrame { code: 1011, reason: format!("lost {}", i.name).into() };
        let _ = to_browser.send(Message::Close(Some(frame))).await;
    };
    let up = async {
        while let Some(Ok(msg)) = from_browser.next().await {
            let out = match msg {
                Message::Binary(b) => {
                    crate::activity::on_input(&st, &s.id, &b);
                    tungstenite::Message::Binary(b)
                }
                Message::Text(t) => tungstenite::Message::Text(t.as_str().into()),
                Message::Close(_) => break,
                _ => continue,
            };
            if to_remote.send(out).await.is_err() {
                return;
            }
        }
        let _ = to_remote.close().await;
    };
    tokio::select! {
        _ = down => {}
        _ = up => {}
    }
}

/// `/link/exec/activity`: a remote instance passes on a terminal agent's hook report; it is taken as the session's
/// own when the session runs there.
pub(crate) async fn activity_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let i = link::from_remote(&st, &method, &uri, &headers, &body).await?;
    let req: ActivityReq = serde_json::from_slice(&body).map_err(|e| ApiError::invalid(e.to_string()))?;
    let ours = matches!(
        st.store.get_session(&req.session_id).await?,
        Some(s) if s.project_id == i.project_id && s.kind == SessionKind::AgentTui
            && s.instance.as_ref().is_some_and(|r| r.id == i.instance_id)
    );
    if !ours {
        return Err(ApiError::forbidden(codes::LINK_REFUSED, "that session doesn't run on this instance"));
    }
    crate::activity::report(&st, &req.session_id, req.report).await?;
    Ok(Json(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_addresses_become_websocket_ones() {
        assert_eq!(ws_url("http://127.0.0.1:7/", "/api/x"), "ws://127.0.0.1:7/api/x");
        assert_eq!(ws_url("https://win-box.t.ts.net", "/api/x"), "wss://win-box.t.ts.net/api/x");
    }
}
