//! Conversations (structured agents spec §3, §5, §9): one runtime per conversation session that owns its live agent
//! process (opened on demand, parked when idle), appends the adapter's events to the store, fans them out to
//! WebSocket viewers and derives the session's activity from them.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pdt_agents::adapter::claude::ClaudeAdapter;
use pdt_agents::adapter::codex::{CodexAdapter, CodexServer, CodexServerConfig, TOKEN_ENV};
use pdt_agents::adapter::{
    AdapterError, AdapterOutput, AgentAdapter, LiveConversation, OpenSpec, Outgoing, SessionChange,
};
use pdt_core::activity::{Activity, needs_attention};
use pdt_core::conversation::draft::{DRAFT_PROMPT, TicketDraft, draft_schema};
use pdt_core::conversation::{
    ActivityFacts, AgentCapabilities, AgentEvent, AnswerRequest, ConversationServerMessage, ConversationState,
    Delivery, DeltaField, NoticeLevel, PendingMessage, PermissionMode, RequestState, SendMessage, SessionMode,
    TurnPhase, TurnStatus, conversation_activity, valid_message,
};
use pdt_core::host::CredentialPolicy;
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus, is_scrubbed};
use pdt_store::NewEvent;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// How long an idle conversation's process lives before it is parked (spec §3.2).
pub const IDLE_PARK: Duration = Duration::from_secs(15 * 60);
/// How many stored events one page of the replay or `GET …/events` returns at most.
const PAGE: i64 = 500;

/// Every conversation runtime in this pdtd, plus what the agents said about themselves.
#[derive(Clone)]
pub struct Conversations {
    map: Arc<Mutex<HashMap<String, Arc<Conv>>>>,
    caps: Arc<Mutex<HashMap<AgentKind, AgentCapabilities>>>,
    generation: Arc<AtomicU64>,
    /// Parks a live process idle this long (tests shorten it).
    pub idle_park: Duration,
    /// How long a conversation must be quiet before PDT sends a notice of its own (tests shorten it).
    pub notice_quiet: Duration,
    /// PDT's `codex app-server`, made on first use.
    codex: Arc<std::sync::OnceLock<Arc<CodexServer>>>,
}

impl Default for Conversations {
    fn default() -> Self {
        Self {
            map: Default::default(),
            caps: Default::default(),
            generation: Arc::new(AtomicU64::new(1)),
            idle_park: IDLE_PARK,
            notice_quiet: Duration::from_secs(20),
            codex: Default::default(),
        }
    }
}

/// The environment variables removed from agents under a credential policy.
fn scrubbed(creds: CredentialPolicy) -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| is_scrubbed(SessionKind::AgentTui, k, creds))
        .collect()
}

fn login_path_env(st: &AppState) -> Vec<(String, String)> {
    st.platform.login_path.iter().map(|p| ("PATH".to_string(), p.clone())).collect()
}

/// PDT's Codex app-server, shared by every Codex conversation. One server serves every project, so it follows the
/// host's credential setting (a project override applies to Claude and to Codex terminals only).
async fn codex_server(st: &AppState) -> Result<Arc<CodexServer>, ApiError> {
    if let Some(s) = st.conversations.codex.get() {
        return Ok(s.clone());
    }
    let creds = st.store.get_host_settings().await?.agent_credentials;
    let config = CodexServerConfig {
        program: st.platform.codex_program.clone(),
        search_path: st.platform.login_path.clone(),
        dir: st.data_dir.join("codex"),
        env: login_path_env(st),
        env_remove: scrubbed(creds),
    };
    Ok(st.conversations.codex.get_or_init(|| CodexServer::new(config)).clone())
}

type LiveSlot = Option<(Arc<dyn LiveConversation>, u64)>;

pub(crate) struct Conv {
    id: String,
    tx: broadcast::Sender<ConversationServerMessage>,
    /// The live process; held across opening and closing so they never interleave.
    live: tokio::sync::Mutex<LiveSlot>,
    state: Mutex<ConvState>,
    /// Orders activity writes so the row always ends with the latest state.
    activity_write: tokio::sync::Mutex<()>,
    /// The newest process generation whose exit has been fully applied (its last events stored).
    exits: tokio::sync::watch::Sender<u64>,
}

struct ConvState {
    turn_running: bool,
    open_requests: HashSet<String>,
    errored: bool,
    has_history: bool,
    pending: Vec<PendingMessage>,
    stopped: Option<String>,
    model: Option<String>,
    permission_mode: PermissionMode,
    agent_version: Option<String>,
    live: bool,
    /// Text so far of items still streaming.
    partial: HashMap<String, (DeltaField, String)>,
    last_active: Instant,
    activity: Option<Activity>,
    generation: u64,
    terminal_open: bool,
    billing: Option<String>,
    /// A rate limit is hit until then (ms): nothing is sent on PDT's own (spec §14).
    limited_until: Option<i64>,
}

impl ConvState {
    fn snapshot(&self) -> ConversationState {
        ConversationState {
            activity: self.activity,
            live: self.live,
            turn_running: self.turn_running,
            model: self.model.clone(),
            permission_mode: self.permission_mode,
            pending: self.pending.clone(),
            stopped: self.stopped.clone(),
            agent_version: self.agent_version.clone(),
            terminal_open: self.terminal_open,
            billing: self.billing.clone(),
        }
    }

    fn facts(&self) -> ActivityFacts {
        ActivityFacts {
            // A message on its way counts as work.
            turn_running: self.turn_running || !self.pending.is_empty(),
            open_requests: self.open_requests.len(),
            errored: self.errored,
            has_history: self.has_history,
        }
    }
}

impl Conv {
    fn state(&self) -> std::sync::MutexGuard<'_, ConvState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push_state(&self) {
        let state = self.state().snapshot();
        let _ = self.tx.send(ConversationServerMessage::State { state });
    }

    async fn current_live(&self) -> Option<Arc<dyn LiveConversation>> {
        self.live.lock().await.as_ref().filter(|(l, _)| l.is_alive()).map(|(l, _)| l.clone())
    }
}

async fn adapter_for(st: &AppState, agent: Option<AgentKind>) -> Result<Box<dyn AgentAdapter>, ApiError> {
    match agent {
        Some(AgentKind::Claude) => Ok(Box::new(ClaudeAdapter)),
        Some(AgentKind::Codex) => Ok(Box::new(CodexAdapter { server: codex_server(st).await? })),
        Some(AgentKind::Devin) => Err(conflict("conversation_unsupported", "Devin runs only as a terminal")),
        None => Err(conflict("not_conversation", "only agent sessions are conversations")),
    }
}

fn conflict(code: &'static str, message: &str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, message)
}

fn adapter_error(e: AdapterError) -> ApiError {
    match e {
        AdapterError::ProgramNotFound(p) => ApiError::new(
            StatusCode::BAD_REQUEST,
            "agent_not_found",
            format!("{p} was not found; install it or set its path"),
        ),
        AdapterError::RequestClosed => conflict("request_closed", "this request was already answered or has expired"),
        AdapterError::WrongAnswer => {
            ApiError::new(StatusCode::BAD_REQUEST, "wrong_answer", "that answer does not fit this request")
        }
        AdapterError::NotRunning => conflict("agent_not_running", "the agent is not running"),
        e => ApiError::new(StatusCode::BAD_GATEWAY, "agent_error", e.to_string()),
    }
}

/// The session as a conversation, or 404 / 409.
async fn conversation_session(st: &AppState, id: &str) -> Result<Session, ApiError> {
    let s = st.store.get_session(id).await?.ok_or_else(|| ApiError::not_found("session"))?;
    if s.mode != SessionMode::Conversation {
        return Err(conflict("not_conversation", "this session is a terminal, not a conversation"));
    }
    Ok(s)
}

/// The runtime for a conversation session, made from the stored row on first use.
pub(crate) async fn conv(st: &AppState, session: &Session) -> Result<Arc<Conv>, ApiError> {
    if let Some(c) = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(&session.id) {
        return Ok(c.clone());
    }
    let has_history = st.store.last_session_event_seq(&session.id).await? > 0;
    let c = Arc::new(Conv {
        id: session.id.clone(),
        tx: broadcast::channel(1024).0,
        live: tokio::sync::Mutex::new(None),
        state: Mutex::new(ConvState {
            turn_running: false,
            open_requests: HashSet::new(),
            errored: false,
            has_history,
            pending: Vec::new(),
            stopped: None,
            model: session.model.clone(),
            permission_mode: session.permission_mode.unwrap_or_default(),
            agent_version: None,
            live: false,
            partial: HashMap::new(),
            last_active: Instant::now(),
            activity: session.activity,
            generation: 0,
            terminal_open: false,
            billing: None,
            limited_until: None,
        }),
        activity_write: tokio::sync::Mutex::new(()),
        exits: tokio::sync::watch::channel(0).0,
    });
    let mut map = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner());
    Ok(map.entry(session.id.clone()).or_insert(c).clone())
}

/// Starts the agent process for a conversation if it is not running, resuming its saved conversation. Marks the
/// session running.
async fn ensure_live(st: &AppState, c: &Arc<Conv>, session: &Session) -> Result<Arc<dyn LiveConversation>, ApiError> {
    let mut slot = c.live.lock().await;
    if let Some((l, _)) = slot.as_ref().filter(|(l, _)| l.is_alive()) {
        return Ok(l.clone());
    }
    let adapter = adapter_for(st, session.agent).await?;
    let creds = crate::sessions::credentials_for(st, &session.project_id).await?;
    let has_history = c.state().has_history;
    let row = st.store.get_session(&session.id).await?.unwrap_or_else(|| session.clone());
    let (program, resume) = match session.agent {
        // Codex resumes its thread whenever it has one.
        Some(AgentKind::Codex) => (st.platform.codex_program.clone(), row.external_session_id.is_some()),
        // Claude saves a conversation only after its first turn; resuming one that was never saved fails.
        _ => {
            let saved = st
                .platform
                .claude_config_dir
                .as_deref()
                .is_none_or(|dir| pdt_agents::launch::claude_conversation_exists(dir, &session.id));
            (st.platform.claude_program.clone(), has_history && saved)
        }
    };
    let spec = OpenSpec {
        session_id: session.id.clone(),
        external_id: row.external_session_id.clone(),
        resume,
        cwd: PathBuf::from(&session.cwd),
        program,
        search_path: st.platform.login_path.clone(),
        model: row.model.clone(),
        permission_mode: row.permission_mode.unwrap_or_default(),
        env: login_path_env(st),
        env_remove: scrubbed(creds),
        instructions: crate::sessions::agent_instructions(st, &row).await?,
    };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let live: Arc<dyn LiveConversation> = Arc::from(adapter.open(spec, tx).await.map_err(adapter_error)?);
    let generation = st.conversations.generation.fetch_add(1, Ordering::SeqCst);
    *slot = Some((live.clone(), generation));
    {
        let mut s = c.state();
        s.live = true;
        s.generation = generation;
        s.stopped = None;
        s.last_active = Instant::now();
    }
    if session.status != SessionStatus::Running || row.status != SessionStatus::Running {
        st.store.set_session_running(&session.id).await?;
    }
    let agent = session.agent.unwrap_or(AgentKind::Claude);
    tokio::spawn(pump(st.clone(), c.clone(), rx, generation, creds, agent));
    drop(slot);
    c.push_state();
    Ok(live)
}

/// Applies one live process's outputs, in order.
async fn pump(
    st: AppState,
    c: Arc<Conv>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<AdapterOutput>,
    generation: u64,
    creds: CredentialPolicy,
    agent: AgentKind,
) {
    while let Some(out) = rx.recv().await {
        match out {
            AdapterOutput::Event { item_id, turn_id, event, raw } => {
                append(&st, &c, item_id.as_deref(), turn_id.as_deref(), event, raw.as_deref()).await;
            }
            AdapterOutput::Delta { item_id, field, text } => {
                c.state().partial.entry(item_id.clone()).or_insert((field, String::new())).1.push_str(&text);
                let _ = c.tx.send(ConversationServerMessage::Delta { item_id, field, text });
            }
            AdapterOutput::Info(info) => {
                // The plan login is the only billing PDT uses unless the project opts into API keys (spec §14).
                let billing_ok = creds != CredentialPolicy::Subscription
                    || info.api_key_source.as_deref().is_none_or(|s| s == "none");
                if !billing_ok {
                    let source = info.api_key_source.clone().unwrap_or_default();
                    let text = format!(
                        "Claude is not using your Claude plan login (its key source is {source}). PDT stopped it; \
                         check `claude` in a terminal, or allow API keys for this project."
                    );
                    append(&st, &c, None, None, AgentEvent::Error { text, retryable: false }, None).await;
                    let st2 = st.clone();
                    let c2 = c.clone();
                    tokio::spawn(async move { park(&st2, &c2).await });
                }
                let mode_changed = {
                    let mut s = c.state();
                    if info.model.is_some() {
                        s.model = info.model.clone();
                    }
                    if info.version.is_some() {
                        s.agent_version = info.version.clone();
                    }
                    // Claude says how it bills each turn; Codex once, from its account.
                    let billing = info.billing.clone().or_else(|| {
                        info.api_key_source
                            .as_deref()
                            .map(|k| if k == "none" { "Claude plan" } else { "API key" }.to_string())
                    });
                    if billing.is_some() {
                        s.billing = billing;
                    }
                    match info.permission_mode {
                        Some(m) if m != s.permission_mode => {
                            s.permission_mode = m;
                            Some(m)
                        }
                        _ => None,
                    }
                };
                if let Some(m) = mode_changed {
                    let _ = st.store.set_session_permission_mode(&c.id, m).await;
                }
                c.push_state();
            }
            AdapterOutput::Capabilities(caps) => {
                st.conversations.caps.lock().unwrap_or_else(|e| e.into_inner()).insert(agent, caps);
            }
            AdapterOutput::ExternalId(id) => {
                if let Err(e) = st.store.set_external_session_id(&c.id, &id).await {
                    tracing::warn!(session = %c.id, error = %e, "could not record the agent's conversation id");
                }
            }
            AdapterOutput::History(turns) => import_history(&st, &c, &turns).await,
            AdapterOutput::Exited { code, stderr_tail, expected } => {
                let current = c.state().generation == generation;
                if !current {
                    c.exits.send_modify(|g| *g = (*g).max(generation));
                    continue;
                }
                {
                    let mut slot = c.live.lock().await;
                    if slot.as_ref().is_some_and(|(_, g)| *g == generation) {
                        *slot = None;
                    }
                }
                {
                    let mut s = c.state();
                    s.live = false;
                    s.turn_running = false;
                    s.open_requests.clear();
                    s.partial.clear();
                    if !expected {
                        let code = code.map_or("no exit code".to_string(), |c| format!("exit code {c}"));
                        let mut why = format!("Claude stopped unexpectedly ({code}).");
                        if !stderr_tail.is_empty() {
                            why.push('\n');
                            why.push_str(&stderr_tail);
                        }
                        s.stopped = Some(why);
                        s.pending.clear();
                    }
                }
                if !expected {
                    let text = c.state().stopped.clone().unwrap_or_default();
                    append(&st, &c, None, None, AgentEvent::Error { text, retryable: true }, None).await;
                }
                c.push_state();
                refresh_activity(&st, &c).await;
                c.exits.send_modify(|g| *g = (*g).max(generation));
            }
        }
    }
}

/// Stores an event (updating its item in place), applies it to the runtime state and sends it to viewers.
async fn append(
    st: &AppState,
    c: &Arc<Conv>,
    item_id: Option<&str>,
    turn_id: Option<&str>,
    event: AgentEvent,
    raw: Option<&str>,
) {
    // The agent echoes a message without PDT's attachment ids (the stored row names them, for the thumbnails) and
    // without saying that PDT sent it on its own.
    let mut event = event;
    if let AgentEvent::UserMessage { text, images, source } = &mut event
        && *source != pdt_core::conversation::MessageSource::Terminal
    {
        let s = c.state();
        if let Some(p) = pending_match(&s.pending, text) {
            if !p.images.is_empty() {
                *images = p.images.clone();
            }
            if p.notice && p.text.trim() == text.trim() {
                *source = pdt_core::conversation::MessageSource::Notice;
            }
        }
    }
    let row = match st
        .store
        .upsert_session_event(&c.id, NewEvent { ts: now_ms(), item_id, turn_id, event: &event, raw })
        .await
    {
        Ok(row) => row,
        Err(e) => {
            tracing::warn!(session = %c.id, error = %e, "could not store a conversation event");
            return;
        }
    };
    let state_changed = {
        let mut s = c.state();
        s.has_history = true;
        s.last_active = Instant::now();
        if let Some(item) = item_id {
            s.partial.remove(item);
        }
        match &event {
            AgentEvent::Turn { phase: TurnPhase::Started, .. } => {
                s.turn_running = true;
                s.errored = false;
                true
            }
            AgentEvent::Turn { phase: TurnPhase::Ended, .. } => {
                s.turn_running = false;
                true
            }
            AgentEvent::Request { id, state: RequestState::Open, .. } => {
                s.open_requests.insert(id.clone());
                true
            }
            AgentEvent::Request { id, .. } => s.open_requests.remove(id),
            AgentEvent::Error { .. } => {
                s.errored = true;
                true
            }
            AgentEvent::Usage { rate_limits, .. } if !rate_limits.is_empty() => {
                s.limited_until = pdt_core::conversation::limited_until(rate_limits, now_ms());
                false
            }
            AgentEvent::UserMessage { text, source, .. }
                if *source != pdt_core::conversation::MessageSource::Terminal =>
            {
                // The agent took a message: it is no longer pending.
                if let Some(i) = pending_index(&s.pending, text) {
                    s.pending.remove(i);
                }
                true
            }
            _ => false,
        }
    };
    let _ = c.tx.send(ConversationServerMessage::Event { event: row });
    if state_changed {
        c.push_state();
        refresh_activity(st, c).await;
    }
}

/// The pending message an echoed one is: the same text, else the oldest.
fn pending_index(pending: &[PendingMessage], text: &str) -> Option<usize> {
    pending.iter().position(|p| p.text.trim() == text.trim()).or((!pending.is_empty()).then_some(0))
}

fn pending_match<'a>(pending: &'a [PendingMessage], text: &str) -> Option<&'a PendingMessage> {
    pending_index(pending, text).map(|i| &pending[i])
}

/// Recomputes the session's activity and records it when it changed (with attention when it now needs the user).
async fn refresh_activity(st: &AppState, c: &Arc<Conv>) {
    let _order = c.activity_write.lock().await;
    let (prev, next) = {
        let mut s = c.state();
        let next = conversation_activity(s.facts());
        let prev = s.activity.replace(next);
        (prev, next)
    };
    if prev == Some(next) {
        return;
    }
    let attention = needs_attention(prev.unwrap_or(Activity::Unknown), next).then(now_ms);
    if let Err(e) = st.store.update_activity(&c.id, Some(next), attention).await {
        tracing::warn!(session = %c.id, error = %e, "could not record conversation activity");
    }
    c.push_state();
}

/// Closes a live process, keeping the conversation (the next message reopens it). Returns once its last events (an
/// interrupted turn, expired requests) are stored.
async fn park(_st: &AppState, c: &Arc<Conv>) {
    let taken = c.live.lock().await.take();
    if let Some((live, generation)) = taken {
        live.close().await;
        let mut exits = c.exits.subscribe();
        let _ = tokio::time::timeout(Duration::from_secs(5), exits.wait_for(|g| *g >= generation)).await;
    }
}

// ---- operations used by the routes and by sessions.rs ----

/// Opens a new conversation session's process and sends its first message, if any.
pub(crate) async fn start(st: &AppState, session: &Session, first: Option<String>) -> Result<(), ApiError> {
    let c = conv(st, session).await?;
    ensure_live(st, &c, session).await?;
    if let Some(text) = first {
        send_message(st, session, &text, &[], Delivery::Next, false).await?;
    }
    Ok(())
}

async fn send_message(
    st: &AppState,
    session: &Session,
    text: &str,
    image_ids: &[String],
    delivery: Delivery,
    notice: bool,
) -> Result<PendingMessage, ApiError> {
    if !valid_message(text, image_ids.len()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_message",
            "a message must have text or an image, and not be over 64 KB",
        ));
    }
    let images = crate::attachments::resolve(st, &session.id, image_ids)?;
    let c = conv(st, session).await?;
    let live = ensure_live(st, &c, session).await?;
    let pending = PendingMessage {
        id: uuid::Uuid::now_v7().to_string(),
        text: text.to_string(),
        delivery,
        images: image_ids.to_vec(),
        notice,
    };
    {
        let mut s = c.state();
        s.pending.push(pending.clone());
        s.errored = false;
        s.stopped = None;
        s.last_active = Instant::now();
    }
    c.push_state();
    refresh_activity(st, &c).await;
    if let Err(e) = live.send(&Outgoing { text: text.to_string(), images }, delivery).await {
        c.state().pending.retain(|p| p.id != pending.id);
        c.push_state();
        refresh_activity(st, &c).await;
        return Err(adapter_error(e));
    }
    Ok(pending)
}

/// Whether PDT may send a message of its own now (a PR notice, spec §14): no turn running, nothing asked or queued,
/// no rate limit hit, its terminal not open beside it, and nothing happening for `notice_quiet`. A conversation not
/// loaded since pdtd started is idle.
pub(crate) fn ready_for_notice(st: &AppState, id: &str) -> bool {
    let Some(c) = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned() else {
        return true;
    };
    let s = c.state();
    !s.turn_running
        && s.open_requests.is_empty()
        && s.pending.is_empty()
        && !s.terminal_open
        && s.limited_until.is_none_or(|t| t <= now_ms())
        && s.last_active.elapsed() >= st.conversations.notice_quiet
}

/// Sends PDT's own message (a PR notice) as the next message; it shows as PDT's, not the user's.
pub(crate) async fn send_notice(st: &AppState, session: &Session, text: &str) -> Result<(), ApiError> {
    send_message(st, session, text, &[], Delivery::Next, true).await.map(|_| ())
}

/// Stops a conversation: closes its process and marks the session finished. Sending again reopens it.
pub(crate) async fn stop(st: &AppState, session: &Session) -> Result<(), ApiError> {
    let c = conv(st, session).await?;
    if st.ptys.is_running(&session.id) {
        let _ = st.kill_pty(&session.id).await;
    }
    park(st, &c).await;
    st.store.end_session(&session.id, SessionStatus::Finished, None, now_ms()).await?;
    {
        let mut s = c.state();
        s.pending.clear();
        s.activity = None;
    }
    c.push_state();
    Ok(())
}

/// Reopens a stopped or interrupted conversation (Restart).
pub(crate) async fn reopen(st: &AppState, session: &Session) -> Result<(), ApiError> {
    let c = conv(st, session).await?;
    ensure_live(st, &c, session).await.map(|_| ())
}

/// Applies a model or permission-mode change: stored, then applied to the live process. When the agent refuses it
/// live, the process is parked so the next message starts it with the new settings.
pub(crate) async fn apply_settings(st: &AppState, session: &Session, change: SessionChange) -> Result<(), ApiError> {
    if let Some(model) = &change.model {
        st.store.set_session_model(&session.id, model.as_deref()).await?;
    }
    if let Some(mode) = change.permission_mode {
        st.store.set_session_permission_mode(&session.id, mode).await?;
    }
    let c = conv(st, session).await?;
    if let Some(mode) = change.permission_mode {
        c.state().permission_mode = mode;
    }
    if let Some(live) = c.current_live().await
        && let Err(e) = live.set(&change).await
    {
        tracing::info!(session = %session.id, error = %e, "the agent refused a live settings change; parking it");
        park(st, &c).await;
    }
    c.push_state();
    Ok(())
}

/// Forgets a removed session's runtime, closing its process.
pub(crate) async fn forget(st: &AppState, id: &str) {
    let c = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    if let Some(c) = c {
        park(st, &c).await;
    }
}

/// Parks every live conversation that has been idle (no turn, request or pending message) for `idle_park`. Returns how
/// many it parked.
pub async fn park_idle_conversations(st: &AppState) -> usize {
    let mut parked = 0;
    let convs: Vec<Arc<Conv>> =
        st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
    for c in convs {
        let idle = {
            let s = c.state();
            s.live
                && !s.turn_running
                && s.open_requests.is_empty()
                && s.pending.is_empty()
                && s.last_active.elapsed() >= st.conversations.idle_park
        };
        if idle {
            tracing::info!(session = %c.id, "parking an idle conversation");
            park(st, &c).await;
            parked += 1;
        }
    }
    parked
}

/// Parks idle conversations every 30 s, until shutdown.
pub async fn run_conversation_ticker(st: AppState) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        if st.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        park_idle_conversations(&st).await;
    }
}

/// Closes every conversation process (pdtd is stopping).
pub async fn shutdown_conversations(st: &AppState) {
    let convs: Vec<Arc<Conv>> =
        st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
    futures_util::future::join_all(convs.iter().map(|c| park(st, c))).await;
    if let Some(server) = st.conversations.codex.get() {
        server.shutdown().await;
    }
}

/// Appends the agent's turns that PDT has not logged (taken in the TUI while PDT was not listening).
async fn import_history(st: &AppState, c: &Arc<Conv>, turns: &[serde_json::Value]) {
    let known = match st.store.session_turn_ids(&c.id).await {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!(session = %c.id, error = %e, "could not read the logged turns");
            return;
        }
    };
    for out in pdt_core::conversation::codex::history_events(turns, &known) {
        if let pdt_core::conversation::codex::CodexOut::Event { item_id, turn_id, event } = out {
            append(st, c, item_id.as_deref(), turn_id.as_deref(), event, None).await;
        }
    }
}

// ---- the live shared terminal (Codex, spec §8.1) ----

#[derive(serde::Deserialize)]
pub(crate) struct TerminalSizeBody {
    cols: u16,
    rows: u16,
}

/// `POST /api/sessions/{id}/terminal`: opens the real Codex TUI on this conversation's thread, attached live to PDT's
/// app-server, in a terminal keyed by the session id (`/api/sessions/{id}/pty`). Nothing happens when it is open.
pub(crate) async fn open_terminal(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TerminalSizeBody>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(size) = payload?;
    let size = pdt_core::session::TerminalSize { cols: size.cols, rows: size.rows }.validate()?;
    let session = conversation_session(&st, &id).await?;
    if session.agent != Some(AgentKind::Codex) {
        return Err(conflict("terminal_unsupported", "only Codex conversations open in a live terminal so far"));
    }
    let _guard = st.lock_session(&id).await;
    if st.ptys.is_running(&id) {
        return Ok(StatusCode::NO_CONTENT);
    }
    // Codex saves a thread only once it has taken a turn; the TUI cannot resume one before that.
    if st.store.session_turn_ids(&id).await?.is_empty() {
        return Err(conflict("no_turn_yet", "send a first message, then open the terminal"));
    }
    let c = conv(&st, &session).await?;
    ensure_live(&st, &c, &session).await?;
    // The pump records a new thread's id just after it opens.
    let mut thread = None;
    for _ in 0..100 {
        thread = st.store.get_session(&id).await?.and_then(|s| s.external_session_id);
        if thread.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let thread =
        thread.ok_or_else(|| conflict("agent_not_running", "Codex has not started this conversation's thread"))?;
    let server = codex_server(&st).await?;
    let endpoint = server.endpoint().await.ok_or_else(|| conflict("agent_not_running", "Codex is not running"))?;
    let args = vec![
        "resume".to_string(),
        thread,
        "--remote".into(),
        endpoint.url,
        "--remote-auth-token-env".into(),
        TOKEN_ENV.into(),
    ];
    let resolved =
        pdt_agents::launch::resolve_launch(&st.platform.codex_program, &args, st.platform.login_path.as_deref())?;
    let mut env = pdt_core::session::terminal_env();
    env.extend(login_path_env(&st));
    env.push((TOKEN_ENV.to_string(), endpoint.token));
    let creds = st.store.get_host_settings().await?.agent_credentials;
    st.remove_pty(&id).await;
    let exit = st.ptys.spawn(pdt_agents::SpawnSpec {
        id: id.clone(),
        program: resolved.program,
        args: resolved.args,
        cwd: PathBuf::from(&session.cwd),
        env_remove: scrubbed(creds),
        env,
        cols: size.cols,
        rows: size.rows,
    })?;
    c.state().terminal_open = true;
    c.push_state();
    let c2 = c.clone();
    tokio::spawn(async move {
        let _ = exit.await;
        // Closing the TUI leaves the conversation as it was.
        c2.state().terminal_open = false;
        c2.push_state();
    });
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/sessions/{id}/terminal`: closes the live TUI (the conversation goes on).
pub(crate) async fn close_terminal(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let session = conversation_session(&st, &id).await?;
    let _guard = st.lock_session(&id).await;
    if st.ptys.is_running(&id) {
        st.kill_pty(&id).await?;
    }
    let c = conv(&st, &session).await?;
    c.state().terminal_open = false;
    c.push_state();
    Ok(StatusCode::NO_CONTENT)
}

// ---- Claude: one at a time, conversation or terminal (spec §8.2) ----

/// How long the TUI gets to leave after `/exit` before it is killed.
const TUI_EXIT_GRACE: Duration = Duration::from_secs(5);

/// `POST /api/sessions/{id}/mode`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SwitchMode {
    mode: SessionMode,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
    /// Interrupt a running turn instead of refusing (`turn_running`).
    #[serde(default)]
    interrupt: bool,
}

/// `POST /api/sessions/{id}/mode`: hands a Claude session between the conversation and Claude's own TUI. Codex
/// conversations keep their live shared terminal (`…/terminal`) instead.
pub(crate) async fn switch_mode(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<SwitchMode>, JsonRejection>,
) -> Result<Json<Session>, ApiError> {
    let Json(body) = payload?;
    let _guard = st.lock_session(&id).await;
    let session = st.store.get_session(&id).await?.ok_or_else(|| ApiError::not_found("session"))?;
    if session.kind != SessionKind::AgentTui || session.agent != Some(AgentKind::Claude) {
        return Err(conflict(
            "switch_unsupported",
            "only Claude sessions switch between conversation and terminal (Codex conversations open a live terminal)",
        ));
    }
    if session.mode != body.mode {
        match body.mode {
            SessionMode::Terminal => {
                let size =
                    pdt_core::session::TerminalSize { cols: body.cols.unwrap_or(120), rows: body.rows.unwrap_or(30) }
                        .validate()?;
                to_terminal(&st, &session, size, body.interrupt).await?;
            }
            SessionMode::Conversation => to_conversation(&st, &session).await?,
        }
    }
    let session = st.store.get_session(&id).await?.ok_or_else(|| ApiError::not_found("session"))?;
    Ok(Json(session))
}

/// The session's transcript, read off the async runtime (None: Claude has not saved one, or no config dir known).
async fn read_transcript(st: &AppState, id: &str) -> Option<String> {
    let dir = st.platform.claude_config_dir.clone()?;
    let id = id.to_string();
    tokio::task::spawn_blocking(move || {
        pdt_agents::launch::claude_transcript(&dir, &id).and_then(|p| std::fs::read_to_string(p).ok())
    })
    .await
    .ok()
    .flatten()
}

/// Conversation → terminal: closes the `-p` process (interrupting a turn only when asked), remembers where PDT's log
/// ends in Claude's transcript, and starts the TUI on the same conversation as a restart would (hooks, `--resume`).
async fn to_terminal(
    st: &AppState,
    session: &Session,
    size: pdt_core::session::TerminalSize,
    interrupt: bool,
) -> Result<(), ApiError> {
    let c = conv(st, session).await?;
    let busy = {
        let s = c.state();
        s.turn_running || !s.pending.is_empty() || !s.open_requests.is_empty()
    };
    if busy && !interrupt {
        return Err(conflict("turn_running", "Claude is working; interrupt it to open the terminal"));
    }
    park(st, &c).await;
    let notice =
        AgentEvent::Notice { level: NoticeLevel::Info, text: "Continued in the terminal.".into(), until: None };
    append(st, &c, None, None, notice, None).await;
    // "" means "from the start": Claude had not saved this conversation yet.
    let anchor = read_transcript(st, &session.id)
        .await
        .and_then(|t| pdt_core::conversation::claude::last_uuid(&t))
        .unwrap_or_default();
    st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).remove(&session.id);
    st.store.switch_session_mode(&session.id, SessionMode::Terminal, Some(&anchor)).await?;
    if let Err(e) = crate::sessions::restart_locked(st, &session.id, size, None).await {
        st.store.switch_session_mode(&session.id, SessionMode::Conversation, None).await?;
        return Err(e);
    }
    Ok(())
}

/// Terminal → conversation: asks the TUI to `/exit` (killing it after a grace period), imports the turns it took
/// (spec §8.3; all of them for a session that was always a terminal) and makes the session a conversation again. The
/// next message resumes it headless.
async fn to_conversation(st: &AppState, session: &Session) -> Result<(), ApiError> {
    let id = session.id.as_str();
    if st.ptys.is_running(id) {
        // Typed, then Enter on its own: a single write would look like a paste to the TUI.
        let _ = st.ptys.write(id, b"/exit");
        tokio::time::sleep(Duration::from_millis(150)).await;
        let _ = st.ptys.write(id, b"\r");
        let deadline = Instant::now() + TUI_EXIT_GRACE;
        while st.ptys.is_running(id) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if st.ptys.is_running(id) {
            st.kill_pty(id).await?;
        }
    }
    crate::sessions::settle_exit(st, id).await;
    st.remove_pty(id).await;
    let anchor = st.store.history_from(id).await?;
    let logged = st.store.last_session_event_seq(id).await? > 0;
    let after = match anchor.as_deref() {
        Some("") => Some(None),
        Some(uuid) => Some(Some(uuid.to_string())),
        // Never handed over by PDT: import everything, unless PDT already has a log it cannot line up.
        None if logged => None,
        None => Some(None),
    };
    st.store.switch_session_mode(id, SessionMode::Conversation, None).await?;
    st.store.set_session_running(id).await?;
    let session = st.store.get_session(id).await?.ok_or_else(|| ApiError::not_found("session"))?;
    let c = conv(st, &session).await?;
    if let Some(after) = after
        && let Some(transcript) = read_transcript(st, id).await
    {
        match pdt_core::conversation::claude::history_events(&transcript, after.as_deref(), Some(&session.cwd)) {
            Ok(events) => {
                for out in events {
                    if let pdt_core::conversation::claude::MapOut::Event { item_id, turn_id, event } = out {
                        append(st, &c, item_id.as_deref(), turn_id.as_deref(), event, None).await;
                    }
                }
            }
            Err(e) => {
                let text = format!("{e}: turns taken in the terminal are not shown here.");
                let notice = AgentEvent::Notice { level: NoticeLevel::Warning, text, until: None };
                append(st, &c, None, None, notice, None).await;
            }
        }
    }
    refresh_activity(st, &c).await;
    c.push_state();
    Ok(())
}

/// After a pdtd restart: a turn that was running ends as interrupted and open requests expire (spec §9). Run before
/// serving.
pub async fn recover_conversations(st: &AppState) {
    let sessions = match st.store.list_sessions_by_status(SessionStatus::Interrupted).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "could not list sessions to recover conversations");
            return;
        }
    };
    for s in sessions.into_iter().filter(|s| s.mode == SessionMode::Conversation) {
        if let Err(e) = recover_one(st, &s.id).await {
            tracing::warn!(session = %s.id, error = %e, "could not recover a conversation");
        }
    }
}

async fn recover_one(st: &AppState, id: &str) -> Result<(), pdt_store::StoreError> {
    for r in st.store.session_events_of_kind(id, "request").await? {
        if let AgentEvent::Request { state: RequestState::Open, .. } = &r.event {
            let mut e = r.event.clone();
            if let AgentEvent::Request { state, .. } = &mut e {
                *state = RequestState::Expired;
            }
            let ne = NewEvent {
                ts: now_ms(),
                item_id: r.item_id.as_deref(),
                turn_id: r.turn_id.as_deref(),
                event: &e,
                raw: None,
            };
            st.store.upsert_session_event(id, ne).await?;
        }
    }
    let turns = st.store.session_events_of_kind(id, "turn").await?;
    if let Some(last) = turns.last()
        && matches!(last.event, AgentEvent::Turn { phase: TurnPhase::Started, .. })
    {
        let turn = last.turn_id.clone().unwrap_or_default();
        let e = AgentEvent::Turn {
            phase: TurnPhase::Ended,
            status: Some(TurnStatus::Interrupted),
            duration_ms: None,
            reason: Some("PDT restarted".into()),
        };
        let item = format!("turn-end:{turn}");
        let ne = NewEvent { ts: now_ms(), item_id: Some(&item), turn_id: Some(&turn), event: &e, raw: None };
        // The web offers Continue on an interrupted last turn; nothing is sent without the user (spec §14).
        st.store.upsert_session_event(id, ne).await?;
    }
    Ok(())
}

// ---- routes ----

#[derive(serde::Deserialize)]
pub(crate) struct EventsQuery {
    #[serde(default)]
    after: Option<i64>,
    #[serde(default)]
    limit: Option<i64>,
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
        (s.snapshot(), s.partial.iter().map(|(k, (f, t))| (k.clone(), *f, t.clone())).collect::<Vec<_>>())
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
    let pending = send_message(&st, &session, &body.text, &images, body.delivery.unwrap_or_default(), false).await?;
    Ok((StatusCode::ACCEPTED, Json(pending)))
}

/// `POST /api/sessions/{id}/requests/{requestId}`.
pub(crate) async fn post_answer(
    State(st): State<AppState>,
    Path((id, request_id)): Path<(String, String)>,
    payload: Result<Json<AnswerRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(body) = payload?;
    let session = conversation_session(&st, &id).await?;
    let c = conv(&st, &session).await?;
    let live = c.current_live().await.ok_or_else(|| adapter_error(AdapterError::RequestClosed))?;
    live.answer(&request_id, &body.answer).await.map_err(adapter_error)?;
    c.state().last_active = Instant::now();
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/sessions/{id}/ticket-draft`: Make ticket's draft (spec §10). A hidden one-turn side request forks the
/// saved conversation (Claude: `--fork-session --no-session-persistence`; Codex: an ephemeral `thread/fork`), asks
/// for a title and description with no tools, and saves nothing; the conversation and its process are untouched.
/// Only the user's click sends it, and nothing retries.
pub(crate) async fn ticket_draft(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TicketDraft>, ApiError> {
    let session = conversation_session(&st, &id).await?;
    let adapter = adapter_for(&st, session.agent).await?;
    let creds = crate::sessions::credentials_for(&st, &session.project_id).await?;
    let (program, saved) = match session.agent {
        Some(AgentKind::Codex) => (st.platform.codex_program.clone(), session.external_session_id.is_some()),
        _ => {
            let on_disk = st
                .platform
                .claude_config_dir
                .as_deref()
                .is_none_or(|dir| pdt_agents::launch::claude_conversation_exists(dir, &session.id));
            (st.platform.claude_program.clone(), on_disk && st.store.last_session_event_seq(&session.id).await? > 0)
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
        model: session.model.clone(),
        permission_mode: PermissionMode::Ask,
        env: login_path_env(&st),
        env_remove: scrubbed(creds),
        // The fork carries the conversation as it is; the side request adds nothing to it.
        instructions: None,
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
    let agent = AgentKind::parse(&agent).ok_or_else(|| ApiError::not_found("agent"))?;
    let known = st.conversations.caps.lock().unwrap_or_else(|e| e.into_inner()).get(&agent).cloned();
    let known = known.or_else(|| match agent {
        AgentKind::Codex => st.conversations.codex.get().and_then(|s| s.capabilities()),
        AgentKind::Claude => None,
        AgentKind::Devin => Some(AgentCapabilities { conversation: false, ..Default::default() }),
    });
    Ok(Json(known.unwrap_or_else(|| AgentCapabilities {
        conversation: true,
        permission_modes: PermissionMode::ALL.to_vec(),
        ..Default::default()
    })))
}
