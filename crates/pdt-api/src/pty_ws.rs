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
    Ok(match classify(&st, &id) {
        Outcome::Attach(attachment) => ws.on_upgrade(move |socket| pump(socket, ptys, id, attachment)),
        // The startup auto-resume has not got to this session yet: wait for it rather than report "not running".
        Outcome::Wait => ws.on_upgrade(move |socket| wait_for_resume(socket, st, id)),
        Outcome::NotRunning => ws.on_upgrade(not_running),
    })
}

enum Outcome {
    Attach(Attachment),
    /// No terminal yet, but the startup auto-resume still has this session to deal with.
    Wait,
    NotRunning,
}

/// What a connecting terminal gets. Whether the resume is pending is read BEFORE trying to attach: the resume
/// starts the PTY and only then clears the pending mark, so "not pending and no PTY" really means no terminal,
/// whereas the other order could miss a PTY that appeared in between.
fn classify(st: &AppState, id: &str) -> Outcome {
    let pending = st.resume_pending(id);
    match st.ptys.attach(id) {
        Ok(att) => Outcome::Attach(att),
        Err(_) if pending => Outcome::Wait,
        Err(_) => Outcome::NotRunning,
    }
}

/// How long a terminal waits for the startup auto-resume to start (or give up on) its session.
const RESUME_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Holds the connection while the session is pending auto-resume (no session lock is taken), then attaches as
/// usual, or reports `not_running` if the resume skipped or failed it, shutdown began, or the wait ran out.
async fn wait_for_resume(mut socket: WebSocket, st: AppState, id: String) {
    // Subscribed before the first check, so a change in between is not missed.
    let mut changed = st.resume_changed.subscribe();
    let deadline = tokio::time::Instant::now() + RESUME_WAIT;
    // The browser sends its size as soon as it connects, before there is a terminal to apply it to.
    let mut size = None;
    loop {
        match classify(&st, &id) {
            Outcome::Attach(att) => {
                if let Some((cols, rows)) = size {
                    let _ = st.ptys.resize(&id, cols, rows);
                }
                return pump(socket, st.ptys.clone(), id, att).await;
            }
            Outcome::NotRunning => return not_running(socket).await,
            Outcome::Wait if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) => {
                return not_running(socket).await;
            }
            Outcome::Wait => {}
        }
        tokio::select! {
            _ = changed.changed() => {}
            _ = tokio::time::sleep_until(deadline) => return not_running(socket).await,
            msg = socket.next() => match msg {
                // The browser went away.
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(Message::Text(text))) => {
                    if let Ok(PtyClientMessage::Resize { cols, rows }) = serde_json::from_str(text.as_str()) {
                        size = Some((cols, rows));
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_store::Store;

    async fn state() -> (AppState, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        (AppState::new(Store::open(&tmp.path().join("pdt.db")).await.unwrap(), tmp.path().join("data")), tmp)
    }

    fn spec(id: &str) -> pdt_agents::SpawnSpec {
        let (program, args) = if cfg!(windows) {
            ("cmd.exe".into(), vec!["/d".to_string(), "/q".into()])
        } else {
            ("/bin/sh".into(), vec![])
        };
        pdt_agents::SpawnSpec {
            id: id.into(),
            program,
            args,
            cwd: std::env::temp_dir(),
            env_remove: vec![],
            env: vec![],
            cols: 80,
            rows: 24,
        }
    }

    #[tokio::test]
    async fn classify_follows_the_pending_mark_and_the_terminal() {
        let (st, _tmp) = state().await;
        assert!(matches!(classify(&st, "s"), Outcome::NotRunning), "nothing pending, no terminal");
        st.pending_resume.lock().unwrap().insert("s".into());
        assert!(matches!(classify(&st, "s"), Outcome::Wait), "pending, no terminal yet");
        // The resume starts the terminal, then clears the mark: a connection arriving at any point in between
        // or after attaches (the pending mark is read first, so it can never report "not running" for a live PTY).
        let _exit = st.ptys.spawn(spec("s")).unwrap();
        assert!(matches!(classify(&st, "s"), Outcome::Attach(_)), "terminal started, mark still set");
        st.finish_resume("s");
        assert!(matches!(classify(&st, "s"), Outcome::Attach(_)), "terminal started, mark cleared");
        st.ptys.shutdown().await;
    }
}
