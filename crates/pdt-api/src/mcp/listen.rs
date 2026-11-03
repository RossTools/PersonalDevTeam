//! `notifications/tools/list_changed` (PDT-501): a running session hears when the tools it would list differ from
//! those it last listed (a plugin turned on or off, its persona saved, or pdtd updated with changed tools), and lists
//! them again. The list it last got is a fingerprint in its folder, so a change made while pdtd was down (an update)
//! is told too. Two ways reach it: the `GET /api/mcp` stream, and a POST answered as a stream with the notice first.

use super::{authorize, session_tools, unauthorized};
use crate::routes::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use pdt_core::mcp::{self as proto, ToolInfo};
use pdt_core::session::{Session, SessionStatus};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

/// The file in a session's folder that holds the fingerprint of the tools it last listed.
const LISTED_FILE: &str = "mcp-tools";

/// Wakes the sessions' streams when their tools may have changed, and remembers the change each was last told of.
#[derive(Default)]
pub(crate) struct ToolWatch {
    changed: tokio::sync::watch::Sender<u64>,
    /// Session id → the fingerprint it was last told of, so it is told once per change.
    notified: Mutex<HashMap<String, String>>,
}

/// The tools a session's tools may have changed with: a plugin turned on or off, a persona saved or updated by PDT.
pub(crate) fn tools_may_have_changed(st: &AppState) {
    st.mcp_tools.changed.send_modify(|n| *n += 1);
}

/// A session's `tools/list`.
pub(super) async fn listed(st: &AppState, s: &Session) -> Vec<ToolInfo> {
    let allowed = session_tools(st, s).await;
    st.registry().tool_infos().into_iter().filter(|t| allowed.contains(&t.name)).collect()
}

fn fingerprint(list: &[ToolInfo]) -> String {
    super::hex_encode(&Sha256::digest(serde_json::to_vec(list).unwrap_or_default()))
}

fn listed_path(st: &AppState, id: &str) -> std::path::PathBuf {
    st.data_dir.join("sessions").join(id).join(LISTED_FILE)
}

/// Records the list a session was just given.
pub(super) fn record(st: &AppState, s: &Session, list: &[ToolInfo]) {
    let path = listed_path(st, &s.id);
    let written =
        path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, fingerprint(list)));
    if let Err(e) = written {
        tracing::warn!(session = %s.id, error = %e, "could not record the session's pdt-mcp tools");
    }
}

/// Whether a session should be told its tools changed: it listed them, they differ now, and it wasn't told of this
/// list already.
pub(super) async fn changed_since_listed(st: &AppState, s: &Session) -> bool {
    let Ok(last) = std::fs::read_to_string(listed_path(st, &s.id)) else { return false };
    let now = fingerprint(&listed(st, s).await);
    if last.trim() == now {
        return false;
    }
    let mut told = st.mcp_tools.notified.lock().unwrap_or_else(|e| e.into_inner());
    if told.get(&s.id) == Some(&now) {
        return false;
    }
    told.insert(s.id.clone(), now);
    true
}

pub(super) fn accepts_stream(headers: &HeaderMap) -> bool {
    headers.get_all(header::ACCEPT).iter().any(|v| v.to_str().is_ok_and(|v| v.contains("text/event-stream")))
}

/// A POST's answer as a stream: the notice, then the answer.
pub(super) fn notice_then(answer: &proto::Response) -> Response {
    let body = proto::sse_message(&proto::tools_list_changed()) + &proto::sse_message(answer);
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/event-stream"), (header::CACHE_CONTROL, "no-cache")], body)
        .into_response()
}

struct Listening {
    st: AppState,
    id: String,
    changed: tokio::sync::watch::Receiver<u64>,
    /// Shutdown starting (as the notifications stream), so pdtd's graceful stop isn't held open.
    wake: tokio::sync::watch::Receiver<u64>,
    first: bool,
}

/// `GET /api/mcp`: the session's stream of server messages, `notifications/tools/list_changed` alone. It ends with the
/// session, or when pdtd stops.
pub(crate) async fn get(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if !accepts_stream(&headers) {
        return super::not_allowed().await;
    }
    let Some(session) = authorize(&st, &headers).await else { return unauthorized() };
    let init = Listening {
        changed: st.mcp_tools.changed.subscribe(),
        wake: st.resume_changed.subscribe(),
        id: session.id,
        st,
        first: true,
    };
    let s = futures_util::stream::unfold(init, |mut l| async move {
        loop {
            if !l.first {
                tokio::select! {
                    r = l.changed.changed() => r.ok()?,
                    w = l.wake.changed() => w.ok()?,
                }
            }
            l.first = false;
            if l.st.shutting_down.load(Ordering::SeqCst) {
                return None;
            }
            let s = l.st.store.get_session(&l.id).await.ok()??;
            if s.status == SessionStatus::Finished {
                return None;
            }
            if changed_since_listed(&l.st, &s).await {
                let data = serde_json::to_string(&proto::tools_list_changed()).unwrap_or_default();
                return Some((Ok::<_, std::convert::Infallible>(Event::default().event("message").data(data)), l));
            }
        }
    });
    Sse::new(s).keep_alive(KeepAlive::default()).into_response()
}
