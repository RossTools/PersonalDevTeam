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
    let attachment = st
        .ptys
        .attach(&id)
        .map_err(|_| ApiError::new(StatusCode::CONFLICT, "not_running", "this session has no terminal; restart it"))?;
    let ptys = st.ptys.clone();
    Ok(ws.on_upgrade(move |socket| pump(socket, ptys, id, attachment)))
}

fn exit_message(code: i32) -> Message {
    Message::Text(serde_json::to_string(&PtyServerMessage::Exit { code }).expect("serializable").into())
}

async fn pump(socket: WebSocket, ptys: PtyManager, id: String, att: Attachment) {
    let (mut tx, mut rx) = socket.split();
    let Attachment { scrollback, exited, mut events } = att;
    if !scrollback.is_empty() && tx.send(Message::Binary(scrollback.into())).await.is_err() {
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
