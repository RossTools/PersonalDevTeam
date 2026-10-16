//! WebSocket bridge between a browser terminal (xterm.js) and a PTY session. See the protocol in
//! docs/superpowers/plans/2026-10-15-phase-1-terminals.md, decision 9.

use crate::error::ApiError;
use crate::routes::AppState;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use pdt_agents::{Attachment, PtyEvent, PtyManager};
use pdt_core::session::{PtyClientMessage, PtyServerMessage};
use tokio::sync::broadcast::error::RecvError;

/// Close code telling the client it fell behind and should reconnect (it will get the scrollback again).
const CLOSE_LAGGED: u16 = 4000;

pub(crate) async fn connect(
    State(st): State<AppState>,
    Path(id): Path<String>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    st.store.get_session(&id).await?.ok_or_else(|| ApiError::not_found("session"))?;
    let ws = ws.map_err(|_| {
        ApiError::new(StatusCode::BAD_REQUEST, "websocket_required", "this endpoint needs a WebSocket upgrade")
    })?;
    let ptys = st.ptys.clone();
    // A session with no terminal (stopped, or interrupted by a daemon restart) still gets the upgrade, so the
    // browser can tell "not running" from "daemon unreachable" and stop retrying.
    Ok(match st.ptys.attach(&id) {
        Ok(attachment) => ws.on_upgrade(move |socket| pump(socket, ptys, id, attachment)),
        // The startup auto-resume has not got to this session yet: wait for it rather than report "not running".
        Err(_) if st.resume_pending(&id) => ws.on_upgrade(move |socket| wait_for_resume(socket, st, id)),
        Err(_) => ws.on_upgrade(not_running),
    })
}

/// How long a terminal waits for the startup auto-resume to start (or give up on) its session.
const RESUME_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Holds the connection while the session is pending auto-resume (no session lock is taken), then attaches as
/// usual, or reports `not_running` if the resume skipped or failed it, shutdown began, or the wait ran out.
async fn wait_for_resume(mut socket: WebSocket, st: AppState, id: String) {
    // Subscribed before the first check, so a change in between is not missed.
    let mut changed = st.resume_changed.subscribe();
    let deadline = tokio::time::Instant::now() + RESUME_WAIT;
    loop {
        if let Ok(att) = st.ptys.attach(&id) {
            return pump(socket, st.ptys.clone(), id, att).await;
        }
        if !st.resume_pending(&id) || st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return not_running(socket).await;
        }
        tokio::select! {
            _ = changed.changed() => {}
            _ = tokio::time::sleep_until(deadline) => return not_running(socket).await,
            msg = socket.next() => match msg {
                // The browser went away (or sent something before `ready`, which is ignored).
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

fn control(msg: &PtyServerMessage) -> Message {
    Message::Text(serde_json::to_string(msg).expect("serializable").into())
}

fn exit_message(code: i32) -> Message {
    control(&PtyServerMessage::Exit { code })
}

async fn not_running(mut socket: WebSocket) {
    let _ = socket.send(control(&PtyServerMessage::NotRunning)).await;
    let _ = socket.close().await;
}

async fn pump(socket: WebSocket, ptys: PtyManager, id: String, att: Attachment) {
    let (mut tx, mut rx) = socket.split();
    let Attachment { scrollback, exited, mut events } = att;
    if !scrollback.is_empty() && tx.send(Message::Binary(scrollback.into())).await.is_err() {
        return;
    }
    if tx.send(control(&PtyServerMessage::Ready)).await.is_err() {
        return;
    }
    if let Some(code) = exited {
        let _ = tx.send(exit_message(code)).await;
        let _ = tx.close().await;
        return;
    }
    let output = async {
        loop {
            match events.recv().await {
                Ok(PtyEvent::Output(bytes)) => {
                    if tx.send(Message::Binary(bytes.to_vec().into())).await.is_err() {
                        return;
                    }
                }
                Ok(PtyEvent::Exit(code)) => {
                    let _ = tx.send(exit_message(code)).await;
                    break;
                }
                Err(RecvError::Lagged(_)) => {
                    let frame = CloseFrame { code: CLOSE_LAGGED, reason: "lagged".into() };
                    let _ = tx.send(Message::Close(Some(frame))).await;
                    return;
                }
                Err(RecvError::Closed) => break,
            }
        }
        let _ = tx.close().await;
    };
    let input = async {
        while let Some(Ok(msg)) = rx.next().await {
            match msg {
                Message::Binary(bytes) => {
                    let _ = ptys.write(&id, &bytes);
                }
                Message::Text(text) => {
                    if let Ok(PtyClientMessage::Resize { cols, rows }) = serde_json::from_str(text.as_str()) {
                        let _ = ptys.resize(&id, cols, rows);
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    };
    tokio::select! {
        _ = output => {}
        _ = input => {}
    }
}
