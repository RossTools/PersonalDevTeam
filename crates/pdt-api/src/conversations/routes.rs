//! The conversation API routes and their handlers.

use super::live::conv;
use super::terminal::{close_terminal, open_terminal, switch_mode};
use super::{
    Conv, PAGE, Sender, acp_agent, adapter_error, adapter_for, conflict, conversation_session, login_path_env,
    scrubbed, send_message,
};
use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pdt_agents::adapter::{AdapterError, OpenSpec};
use pdt_core::conversation::draft::{DRAFT_PROMPT, TicketDraft, draft_schema};
use pdt_core::conversation::{
    AgentCapabilities, AnswerRequest, ConversationServerMessage, PendingMessage, PermissionMode, SendMessage,
};
use pdt_core::session::AcpAgentId;
use pdt_core::session::{AgentKind, Session};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tokio::sync::broadcast;

#[derive(serde::Deserialize)]
pub(crate) struct EventsQuery {
    #[serde(default)]
    after: Option<i64>,
    #[serde(default)]
    limit: Option<i64>,
    /// The newest `last` events instead (oldest first); `after` and `limit` are ignored.
    #[serde(default)]
    last: Option<i64>,
}

/// `GET /api/sessions/{id}/events`: the stored events (JSON), or the live stream (WebSocket upgrade).
pub(crate) async fn events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<EventsQuery>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let session = conversation_session(&st, &id).await?;
    let after = q.after.unwrap_or(0).max(0);
    match ws {
        Ok(ws) => {
            let c = conv(&st, &session).await?;
            Ok(ws.on_upgrade(move |socket| stream(st, c, socket, after)).into_response())
        }
        Err(_) if let Some(last) = q.last => {
            Ok(Json(st.store.recent_session_events(&id, last.clamp(1, PAGE)).await?).into_response())
        }
        Err(_) => {
            let limit = q.limit.unwrap_or(PAGE).clamp(1, PAGE * 2);
            Ok(Json(st.store.list_session_events(&id, after, limit).await?).into_response())
        }
    }
}

async fn send_frame(socket: &mut WebSocket, m: &ConversationServerMessage) -> bool {
    match serde_json::to_string(m) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => true,
    }
}

/// Replays stored events after `after`, then the state and any streaming text, then live frames.
async fn stream(st: AppState, c: Arc<Conv>, mut socket: WebSocket, after: i64) {
    // Subscribed before the replay, so nothing that happens during it is missed (duplicates are skipped by seq).
    let mut rx = c.tx.subscribe();
    let mut shutdown = st.resume_changed.subscribe();
    let mut last = after;
    if !replay(&st, &c.id, &mut socket, &mut last).await {
        return;
    }
    let (state, partial) = {
        let s = c.state();
        let r = st.store.redactor();
        (s.snapshot(), s.partial.iter().map(|(k, (f, t))| (k.clone(), *f, r.string(t))).collect::<Vec<_>>())
    };
    if !send_frame(&mut socket, &ConversationServerMessage::State { state }).await {
        return;
    }
    for (item_id, field, text) in partial {
        if !send_frame(&mut socket, &ConversationServerMessage::Delta { item_id, field, text }).await {
            return;
        }
    }
    if !send_frame(&mut socket, &ConversationServerMessage::Ready {}).await {
        return;
    }
    loop {
        tokio::select! {
            m = rx.recv() => match m {
                Ok(ConversationServerMessage::Event { event }) if event.seq <= last => {}
                Ok(m) => {
                    if let ConversationServerMessage::Event { event } = &m {
                        last = event.seq;
                    }
                    if !send_frame(&mut socket, &m).await {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Too slow to keep up: catch up from the store.
                    if !replay(&st, &c.id, &mut socket, &mut last).await {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
            _ = shutdown.changed() => {
                if st.shutting_down.load(Ordering::SeqCst) {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            }
        }
    }
}

async fn replay(st: &AppState, id: &str, socket: &mut WebSocket, last: &mut i64) -> bool {
    loop {
        let page = match st.store.list_session_events(id, *last, PAGE).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(session = %id, error = %e, "could not read conversation events");
                return false;
            }
        };
        let n = page.len() as i64;
        for event in page {
            *last = event.seq;
            if !send_frame(socket, &ConversationServerMessage::Event { event }).await {
                return false;
            }
        }
        if n < PAGE {
            return true;
        }
    }
}

/// `POST /api/sessions/{id}/messages`.
pub(crate) async fn post_message(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<SendMessage>, JsonRejection>,
) -> Result<(StatusCode, Json<PendingMessage>), ApiError> {
    let Json(body) = payload?;
    let session = conversation_session(&st, &id).await?;
    let images = body.images.unwrap_or_default();
    let pending =
        send_message(&st, &session, &body.text, &images, body.delivery.unwrap_or_default(), Sender::User, None).await?;
    crate::pm::user_wrote(&st, &session, &body.text).await;
    Ok((StatusCode::ACCEPTED, Json(pending)))
}

/// `POST /api/sessions/{id}/requests/{requestId}`.
pub(crate) async fn post_answer(
    State(st): State<AppState>,
    Path((id, request_id)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    payload: Result<Json<AnswerRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(body) = payload?;
    let from_ui = crate::untrusted::from_ui(&st, &headers)?;
    let session = conversation_session(&st, &id).await?;
    // Its Inbox item, for `inbox.answered` (PDT-364).
    let item_id = format!("approval:{}:{request_id}", session.id);
    let item = crate::inbox::approvals(&st, Some(&session.project_id)).await?.into_iter().find(|i| i.id == item_id);
    answer_request(&st, &session, &request_id, &body.answer).await?;
    // The user's own answer: a yes to a call PDT held for them is good for that call (PDT-201), when it came from PDT's
    // web client and not an agent's curl (PDT-204).
    let approved = from_ui && matches!(body.answer, pdt_core::conversation::Answer::Approve { .. });
    st.grants.answered(&session.id, &request_id, approved);
    // Told after the grant: the agent's call may already be checking it.
    if let Some(item) = item {
        use pdt_core::conversation::Answer;
        let (choice, text) = match &body.answer {
            Answer::Approve { always: true } => ("allow_always", None),
            Answer::Approve { .. } => ("allow", None),
            Answer::Deny { message } => ("deny", message.clone()),
            Answer::Question { answers } => {
                ("answer", Some(answers.values().map(|a| a.join(", ")).collect::<Vec<_>>().join("\n")))
            }
            Answer::Plan { approve, feedback, .. } => {
                (if *approve { "approve" } else { "keep_planning" }, feedback.clone())
            }
            #[allow(unreachable_patterns)]
            _ => ("answer", None),
        };
        let by = if from_ui { "user" } else { "agent" };
        crate::workflow::sensors::inbox_answered(&st, &item, Some(choice), text.as_deref(), by).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Answers one of a live conversation's open requests.
pub(crate) async fn answer_request(
    st: &AppState,
    session: &Session,
    request_id: &str,
    answer: &pdt_core::conversation::Answer,
) -> Result<(), ApiError> {
    let c = conv(st, session).await?;
    let live = c.current_live().await.ok_or_else(|| adapter_error(AdapterError::RequestClosed))?;
    live.answer(request_id, answer).await.map_err(adapter_error)?;
    c.state().last_active = Instant::now();
    Ok(())
}

/// `POST /api/sessions/{id}/ticket-draft`: Make ticket's draft. A hidden one-turn side request forks the
/// saved conversation (Claude: `--fork-session --no-session-persistence`; Codex: an ephemeral `thread/fork`), asks
/// for a title and description with no tools, and saves nothing; the conversation and its process are untouched.
/// Only the user's click sends it, and nothing retries.
pub(crate) async fn ticket_draft(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TicketDraft>, ApiError> {
    let session = conversation_session(&st, &id).await?;
    if let Some(agent @ (AgentKind::Devin | AgentKind::Acp(_))) = session.agent {
        // ACP has no fork: a side request would land in the conversation itself.
        let name = match agent {
            AgentKind::Acp(_) => acp_agent(&st, agent).await.map(|d| d.label).unwrap_or_else(|_| "ACP".into()),
            _ => agent.label().to_string(),
        };
        return Err(conflict(
            "draft_unsupported",
            &format!("Make ticket is not available for {name} conversations yet"),
        ));
    }
    let adapter = adapter_for(&st, session.agent).await?;
    let creds = crate::sessions::credentials_for(&st, &session.project_id).await?;
    let (program, saved) = match session.agent {
        Some(AgentKind::Codex) => (st.agent_platform().await.codex_program, session.external_session_id.is_some()),
        _ => {
            let on_disk = st.platform.claude_config_dir.as_deref().is_none_or(|dir| {
                pdt_agents::launch::claude_conversation_exists(dir, session.claude_conversation_id())
            });
            (
                st.agent_platform().await.claude_program,
                on_disk && st.store.last_session_event_seq(&session.id).await? > 0,
            )
        }
    };
    if !saved {
        return Err(conflict("nothing_to_draft", "the agent has no conversation to draft a ticket from yet"));
    }
    let spec = OpenSpec {
        session_id: session.id.clone(),
        external_id: session.external_session_id.clone(),
        resume: true,
        cwd: PathBuf::from(&session.cwd),
        program,
        search_path: st.platform.login_path.clone(),
        model: st.store.get_host_settings().await?.model_for(session.agent, session.model.as_deref()),
        permission_mode: PermissionMode::Ask,
        env: login_path_env(&st),
        env_remove: scrubbed(creds),
        // The fork carries the conversation as it is; the side request adds nothing to it.
        instructions: None,
        config_file: None,
        import_after: None,
        mcp: None,
    };
    let answer = adapter.draft(spec, DRAFT_PROMPT, &draft_schema()).await.map_err(|e| match e {
        AdapterError::Timeout => {
            ApiError::new(StatusCode::GATEWAY_TIMEOUT, "draft_timeout", "the agent took too long to draft a ticket")
        }
        e => adapter_error(e),
    })?;
    TicketDraft::from_value(&answer).map(Json).ok_or_else(|| {
        ApiError::new(StatusCode::BAD_GATEWAY, "agent_error", "the agent's answer was not a ticket draft")
    })
}

/// `POST /api/sessions/{id}/interrupt`: stops the running turn (nothing to do when none runs).
pub(crate) async fn post_interrupt(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let session = conversation_session(&st, &id).await?;
    let c = conv(&st, &session).await?;
    let running = {
        let s = c.state();
        s.turn_running || !s.pending.is_empty()
    };
    if running && let Some(live) = c.current_live().await {
        match live.interrupt().await {
            Ok(()) | Err(AdapterError::NotRunning) => {}
            Err(e) => return Err(adapter_error(e)),
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/agents/{agent}/capabilities`.
pub(crate) async fn capabilities(
    State(st): State<AppState>,
    Path(agent): Path<String>,
) -> Result<Json<AgentCapabilities>, ApiError> {
    // An ACP agent only when the host has it: the path is outside input, and parsing interns the id.
    let mut acp_modes = None;
    if let Some(id) = agent.strip_prefix(AcpAgentId::PREFIX) {
        let host = st.store.get_host_settings().await?;
        let Some(def) = pdt_core::acp_agent::find(&host.acp_agents, id) else {
            return Err(ApiError::not_found("agent"));
        };
        acp_modes = Some(def.offered_modes());
    }
    let agent = AgentKind::parse(&agent).ok_or_else(|| ApiError::not_found("agent"))?;
    let known = st.conversations.caps.lock().unwrap_or_else(|e| e.into_inner()).get(&agent).cloned();
    let known = known.or_else(|| match agent {
        AgentKind::Codex => st.conversations.codex.get().and_then(|s| s.capabilities()),
        AgentKind::Claude | AgentKind::Devin | AgentKind::Acp(_) => None,
    });
    Ok(Json(known.unwrap_or_else(|| AgentCapabilities {
        conversation: true,
        permission_modes: match agent {
            AgentKind::Devin => pdt_core::conversation::devin::DEVIN_MODES.to_vec(),
            _ => acp_modes.unwrap_or_else(|| PermissionMode::ALL.to_vec()),
        },
        ..Default::default()
    })))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/sessions/{id}/events", get(events))
        .route("/sessions/{id}/messages", post(post_message))
        .route("/sessions/{id}/requests/{request_id}", post(post_answer))
        .route("/sessions/{id}/interrupt", post(post_interrupt))
        .route("/sessions/{id}/ticket-draft", post(ticket_draft))
        .route("/sessions/{id}/mode", post(switch_mode))
        .route("/sessions/{id}/terminal", post(open_terminal).delete(close_terminal))
        .route("/agents/{agent}/capabilities", get(capabilities))
}
