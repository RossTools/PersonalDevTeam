//! Conversations: one runtime per conversation session that owns its live agent
//! process (opened on demand, parked when idle), appends the adapter's events to the store, fans them out to
//! WebSocket viewers and derives the session's activity from them.

use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use pdt_agents::adapter::acp_agent::DefinedAdapter;
use pdt_agents::adapter::claude::ClaudeAdapter;
use pdt_agents::adapter::codex::{CodexAdapter, CodexServer, CodexServerConfig};
use pdt_agents::adapter::devin::DevinAdapter;
use pdt_agents::adapter::{AdapterError, AgentAdapter, LiveConversation, Outgoing, SessionChange};
use pdt_core::acp_agent::AcpAgentDef;
use pdt_core::activity::Activity;
use pdt_core::conversation::{
    ActivityFacts, AgentCapabilities, AgentEvent, BackgroundTask, ConversationEvent, ConversationServerMessage,
    ConversationState, Delivery, DeltaField, PendingMessage, PermissionMode, RequestState, SessionMode, TurnPhase,
    TurnStatus, valid_message,
};
use pdt_core::host::CredentialPolicy;
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus, is_scrubbed};
use pdt_store::NewEvent;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

mod live;
mod routes;
mod state;
mod terminal;

use live::{Unstored, conv, ensure_live, flush_unstored, park, refresh_activity};
pub(crate) use routes::{answer_request, router};
pub(crate) use state::{
    Busy, background, busy_ids, busy_with, is_live_or_waiting, is_loaded, open_requests, ready_for_notice,
};
pub use state::{answer_as_user, conversation_live, is_working, subscribe_all};
pub(crate) use terminal::terminal_conversation_moved;

/// How long an idle conversation's process lives before it is parked.
pub const IDLE_PARK: Duration = Duration::from_secs(15 * 60);
/// How long a message may sit unechoed while nothing happens in its conversation (`Conversations::pending_timeout`).
pub const PENDING_TIMEOUT: Duration = Duration::from_secs(5 * 60);
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
    /// How long a message may wait to go in with no turn running, nothing asked and nothing heard from the agent
    /// before PDT gives up on it and says so (PDT-129; tests shorten it).
    pub pending_timeout: Duration,
    /// How long a conversation whose background tasks all finished must stay quiet (no turn) before an agent step
    /// waiting on them counts its turn as ended (PDT-142; tests shorten it).
    pub background_grace: Duration,
    /// A pause between a step's session being found idle and its close (PDT-188): none, but tests lengthen it to send
    /// a message in between.
    pub close_pause: Duration,
    /// PDT's `codex app-server`, made on first use.
    codex: Arc<std::sync::OnceLock<Arc<CodexServer>>>,
    /// Every conversation's stored events as they happen, with their session's id ([`subscribe_all`]).
    all: broadcast::Sender<(String, ConversationEvent)>,
    /// The conversations whose log was checked for gaps since pdtd started ([`repair_gaps`]).
    repaired: Arc<tokio::sync::Mutex<HashSet<String>>>,
}

impl Default for Conversations {
    fn default() -> Self {
        Self {
            map: Default::default(),
            caps: Default::default(),
            generation: Arc::new(AtomicU64::new(1)),
            idle_park: IDLE_PARK,
            notice_quiet: Duration::from_secs(20),
            pending_timeout: PENDING_TIMEOUT,
            background_grace: Duration::from_secs(10),
            close_pause: Duration::ZERO,
            codex: Default::default(),
            all: broadcast::channel(1024).0,
            repaired: Default::default(),
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
        program: st.agent_platform().await.codex_program,
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
    /// Sends hold it shared until their message is pending; closing an idle conversation holds it alone, so a message
    /// sent as a step's session is closed either lands first (and keeps it open) or reopens it after (PDT-188).
    sending: tokio::sync::RwLock<()>,
    /// The newest process generation whose exit has been fully applied (its last events stored).
    exits: tokio::sync::watch::Sender<u64>,
    /// Events received that are not stored yet (a write failed), oldest first; held while storing, so events are
    /// stored in the order they came (PDT-331).
    unstored: tokio::sync::Mutex<std::collections::VecDeque<Unstored>>,
    /// `unstored` is being retried.
    retrying: std::sync::atomic::AtomicBool,
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
    /// A rate limit is hit until then (ms): nothing is sent on PDT's own.
    limited_until: Option<i64>,
    /// The agent's background tasks still running: the process is not parked while there are any.
    background_tasks: Vec<BackgroundTask>,
    /// Background tasks a restart of PDT stopped, until the agent is told with the next message (PDT-311).
    stopped_background: Vec<BackgroundTask>,
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
            background_tasks: self.background_tasks.clone(),
            stopped_background: self.stopped_background.clone(),
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
        Some(AgentKind::Devin) => Ok(Box::new(DevinAdapter)),
        Some(agent @ AgentKind::Acp(_)) => {
            let def = acp_agent(st, agent).await?;
            Ok(Box::new(DefinedAdapter::new(def).ok_or_else(|| unknown_agent(agent))?))
        }
        None => Err(ApiError::conflict(codes::NOT_CONVERSATION, "only agent sessions are conversations")),
    }
}

fn unknown_agent(agent: AgentKind) -> ApiError {
    ApiError::bad_request(
        codes::UNKNOWN_AGENT,
        format!("this host has no ACP agent `{}`: add it in Settings → Agents", agent.as_str()),
    )
}

/// The ACP agent an agent kind names (a built-in profile or one the user added), or 400 when this host has none.
/// The ACP agent as it launches on this host (a profile PDT installs runs the program it installed, or the path set
/// over it).
pub(crate) async fn acp_agent(st: &AppState, agent: AgentKind) -> Result<AcpAgentDef, ApiError> {
    let s = st.store.get_host_settings().await?;
    let def = s.acp_agent(agent).ok_or_else(|| unknown_agent(agent))?;
    Ok(crate::toolchains::launch_def(&st.data_dir, st.home_dir.as_deref(), &s, def))
}

fn adapter_error(e: AdapterError) -> ApiError {
    match e {
        AdapterError::ProgramNotFound(p) => {
            ApiError::conflict(codes::AGENT_NOT_FOUND, format!("{p} was not found; install it or set its path"))
        }
        AdapterError::RequestClosed => {
            ApiError::conflict(codes::REQUEST_CLOSED, "this request was already answered or has expired")
        }
        AdapterError::WrongAnswer => {
            ApiError::bad_request(codes::WRONG_ANSWER, "that answer does not fit this request")
        }
        AdapterError::NotRunning => ApiError::conflict(codes::AGENT_NOT_RUNNING, "the agent is not running"),
        e => ApiError::upstream(codes::AGENT_ERROR, e.to_string()),
    }
}

/// The session as a conversation, or 404 / 409.
async fn conversation_session(st: &AppState, id: &str) -> Result<Session, ApiError> {
    let s = st.session(id).await?;
    if s.mode != SessionMode::Conversation {
        return Err(ApiError::conflict(codes::NOT_CONVERSATION, "this session is a terminal, not a conversation"));
    }
    Ok(s)
}

// ---- operations used by the routes and by sessions.rs ----

/// Opens a new conversation session's process and sends its first message, if any.
pub(crate) async fn start(st: &AppState, session: &Session, first: Option<String>) -> Result<(), ApiError> {
    let c = conv(st, session).await?;
    ensure_live(st, &c, session).await?;
    if let Some(text) = first {
        send_message(st, session, &text, &[], Delivery::Next, Sender::User, None).await?;
    }
    Ok(())
}

/// Who a message is from: the user, or PDT on its own (a notice; which of its PM messages, PDT-211).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sender {
    User,
    Pdt(Option<pdt_core::conversation::PdtMessage>),
}

/// Sends a message: pending until the agent echoes it. `outbox` names the pending id the outbox's messages already
/// wait under; any other message gets an outbox row of its own (status `sending`), so a message lost on the way (the
/// agent stops, pdtd restarts) is reported instead of vanishing (PDT-129).
async fn send_message(
    st: &AppState,
    session: &Session,
    text: &str,
    image_ids: &[String],
    delivery: Delivery,
    from: Sender,
    outbox: Option<&str>,
) -> Result<PendingMessage, ApiError> {
    let (notice, pdt) = match from {
        Sender::User => (false, None),
        Sender::Pdt(pdt) => (true, pdt),
    };
    if !valid_message(text, image_ids.len()) {
        return Err(ApiError::bad_request(
            codes::INVALID_MESSAGE,
            "a message must have text or an image, and not be over 64 KB",
        ));
    }
    let images = crate::attachments::resolve(st, &session.id, image_ids)?;
    let c = conv(st, session).await?;
    let _sending = c.sending.read().await;
    let live = ensure_live(st, &c, session).await?;
    tell_stopped_background(st, &c, live.as_ref()).await;
    let pending = PendingMessage {
        id: outbox.map_or_else(|| uuid::Uuid::now_v7().to_string(), String::from),
        text: text.to_string(),
        delivery,
        images: image_ids.to_vec(),
        notice,
        pdt,
    };
    if outbox.is_none() {
        crate::workflow::outbox::track(st, session, &pending.id, text, !notice).await?;
    }
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
        // The sender hears of it (an error, or the outbox trying again): nothing to report.
        if outbox.is_none() {
            crate::workflow::outbox::lost(st, std::slice::from_ref(&pending.id), None);
        }
        return Err(adapter_error(e));
    }
    Ok(pending)
}

/// Background tasks a restart stopped (PDT-311): the agent hears of it just before the message that reopened it, so it
/// does not wait on them for ever. Never sent on its own: it goes along with a message the user or a workflow sends.
async fn tell_stopped_background(st: &AppState, c: &Arc<Conv>, live: &dyn LiveConversation) {
    let stopped = std::mem::take(&mut c.state().stopped_background);
    if stopped.is_empty() {
        return;
    }
    let names: Vec<String> = stopped.iter().map(|t| t.description.clone()).collect();
    let text = pdt_core::conversation::restart_notice(&names);
    let pending = PendingMessage {
        id: uuid::Uuid::now_v7().to_string(),
        text: text.clone(),
        delivery: Delivery::Next,
        images: Vec::new(),
        notice: true,
        pdt: None,
    };
    c.state().pending.push(pending.clone());
    match live.send(&Outgoing { text, images: Vec::new() }, Delivery::Next).await {
        Ok(()) => {
            if let Err(e) = st.store.set_stopped_background(&c.id, &[]).await {
                tracing::warn!(session = %c.id, error = %e, "could not record the stopped tasks as told");
            }
        }
        Err(e) => {
            tracing::warn!(session = %c.id, error = %e, "could not tell the agent its background tasks stopped");
            let mut s = c.state();
            s.pending.retain(|p| p.id != pending.id);
            s.stopped_background = stopped;
        }
    }
    c.push_state();
}

/// Takes the background tasks a restart stopped that the session's agent was not told about yet, so the caller tells
/// it in its own words (a workflow step's notice, PDT-311).
pub(crate) async fn take_stopped_background(st: &AppState, id: &str) -> Vec<BackgroundTask> {
    let loaded = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned();
    let mut tasks = match &loaded {
        Some(c) => std::mem::take(&mut c.state().stopped_background),
        None => Vec::new(),
    };
    if tasks.is_empty() {
        tasks = st.store.stopped_background(id).await.unwrap_or_default();
    }
    if let Err(e) = st.store.set_stopped_background(id, &[]).await {
        tracing::warn!(session = %id, error = %e, "could not record the stopped tasks as told");
    }
    if let Some(c) = loaded {
        c.push_state();
    }
    tasks
}

/// Sends the outbox's messages (`pending_id` names them) as the next message; it shows as PDT's, not the user's (and
/// collapsed, when `pdt` says which of its PM messages it is). They are told once the agent echoes it.
pub(crate) async fn send_notice(
    st: &AppState,
    session: &Session,
    text: &str,
    pending_id: &str,
    pdt: Option<pdt_core::conversation::PdtMessage>,
) -> Result<(), ApiError> {
    send_message(st, session, text, &[], Delivery::Next, Sender::Pdt(pdt), Some(pending_id)).await.map(|_| ())
}

/// Sends a message PDT composed for the user (review feedback) as the user's next message: queued while a turn runs;
/// a parked or stopped conversation reopens to take it.
pub(crate) async fn send_user_message(st: &AppState, session: &Session, text: &str) -> Result<(), ApiError> {
    send_message(st, session, text, &[], Delivery::Next, Sender::User, None).await.map(|_| ())
}

/// Stops a conversation: closes its process and marks the session finished. Sending again reopens it.
pub(crate) async fn stop(st: &AppState, session: &Session) -> Result<(), ApiError> {
    let c = conv(st, session).await?;
    stop_conv(st, session, &c).await
}

/// Stops a conversation if it is idle (no turn, request or pending message), checked so that no message can go in
/// between the look and the stop (PDT-188). False: it was at work, and is left alone.
pub(crate) async fn stop_if_idle(st: &AppState, session: &Session) -> Result<bool, ApiError> {
    let c = conv(st, session).await?;
    let _closing = c.sending.write().await;
    if !idle(&c) {
        return Ok(false);
    }
    stop_conv(st, session, &c).await.map(|()| true)
}

/// No turn runs, nothing is asked and no message waits to go in.
fn idle(c: &Conv) -> bool {
    let s = c.state();
    !s.turn_running && s.open_requests.is_empty() && s.pending.is_empty()
}

async fn stop_conv(st: &AppState, session: &Session, c: &Arc<Conv>) -> Result<(), ApiError> {
    // A terminal's exit is told where it ends (`session.exited`); a conversation's process, here.
    let terminal = st.ptys.is_running(&session.id);
    if terminal {
        let _ = st.kill_pty(&session.id).await;
    }
    park(st, c).await;
    st.store.end_session(&session.id, SessionStatus::Finished, None, now_ms()).await?;
    if !terminal && session.status == SessionStatus::Running {
        crate::workflow::sensors::session_exited(st, &session.id, None, true).await;
    }
    let dropped: Vec<String> = {
        let mut s = c.state();
        s.activity = None;
        s.pending.drain(..).map(|p| p.id).collect()
    };
    // The user stopped it: what had not gone in is not told, and they know why.
    crate::workflow::outbox::lost(st, &dropped, None);
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
    // Back to "Default" is the agent's default model in Settings → Agents when it has one.
    let mut change = change;
    if let Some(None) = change.model {
        change.model = Some(st.store.get_host_settings().await?.model_for(session.agent, None));
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

/// Parks a loaded conversation's process now if it is idle (no turn, request or pending message), instead of after
/// `idle_park` (an `own` step's session between passes, PDT-183). False: it was at work (a message went in, PDT-188).
pub(crate) async fn park_idle(st: &AppState, id: &str) -> bool {
    let Some(c) = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned() else { return true };
    let _closing = c.sending.write().await;
    if !idle(&c) {
        return false;
    }
    if c.state().live {
        tracing::info!(session = %id, "parking a step's conversation after its outcome");
        park(st, &c).await;
        c.push_state();
    }
    true
}

/// Forgets a removed session's runtime, closing its process.
pub(crate) async fn forget(st: &AppState, id: &str) {
    let c = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    if let Some(c) = c {
        park(st, &c).await;
        let dropped: Vec<String> = c.state().pending.drain(..).map(|p| p.id).collect();
        crate::workflow::outbox::lost(st, &dropped, None);
    }
}

/// Parks every live conversation that has been idle (no turn, request, pending message or background task) for
/// `idle_park`: parking would stop its background tasks before they report back. Returns how many it parked.
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
                && s.background_tasks.is_empty()
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

/// Gives up on messages that have waited `pending_timeout` to go in while nothing happened in their conversation (no
/// turn, nothing asked, no event): they are dropped and reported, so they no longer hold the conversation busy
/// (PDT-129). Returns how many it dropped.
pub async fn expire_pending_messages(st: &AppState) -> usize {
    let convs: Vec<Arc<Conv>> =
        st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
    let mut dropped = 0;
    for c in convs {
        let stuck: Vec<String> = {
            let mut s = c.state();
            if s.pending.is_empty()
                || s.turn_running
                || !s.open_requests.is_empty()
                || s.last_active.elapsed() < st.conversations.pending_timeout
            {
                continue;
            }
            s.pending.drain(..).map(|p| p.id).collect()
        };
        tracing::warn!(session = %c.id, messages = stuck.len(), "a conversation never took its messages; giving up");
        dropped += stuck.len();
        let mins = st.conversations.pending_timeout.as_secs().div_ceil(60);
        let why = format!("The agent did not take it within {mins} min.");
        crate::workflow::outbox::lost(st, &stuck, Some(&why));
        c.push_state();
        refresh_activity(st, &c).await;
    }
    dropped
}

/// Parks idle conversations and gives up on stuck messages every 30 s, until shutdown.
pub async fn run_conversation_ticker(st: AppState) {
    let every = Duration::from_secs(30);
    crate::loops::run_every(st, "conversations", every, crate::loops::Start::AfterOnePeriod, |st| async move {
        expire_pending_messages(&st).await;
        park_idle_conversations(&st).await;
    })
    .await
}

/// Closes every conversation process (pdtd is stopping).
pub async fn shutdown_conversations(st: &AppState) {
    let convs: Vec<Arc<Conv>> =
        st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
    futures_util::future::join_all(convs.iter().map(|c| park(st, c))).await;
    // A last try at events still waiting for a write: what is not stored now is lost with this process (PDT-331).
    for c in &convs {
        let mut q = c.unstored.lock().await;
        if !q.is_empty() && !flush_unstored(st, c, &mut q).await {
            tracing::error!(session = %c.id, events = q.len(), "conversation events could not be stored before PDT stopped");
        }
    }
    if let Some(server) = st.conversations.codex.get() {
        server.shutdown().await;
    }
}

/// After a pdtd restart: a turn that was running ends as interrupted and open requests expire. Run before
/// serving.
pub async fn recover_conversations(st: &AppState) {
    // Every background task the previous pdtd's agents ran ended with them (PDT-311).
    match st.store.retire_session_background().await {
        Ok(ids) if !ids.is_empty() => tracing::info!(sessions = ids.len(), "a restart stopped background tasks"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "could not record the background tasks a restart stopped"),
    }
    // Messages a conversation had not echoed when the previous pdtd stopped never reached it (PDT-129).
    crate::workflow::outbox::recover(st).await;
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
        // The web offers Continue on an interrupted last turn; nothing is sent without the user.
        st.store.upsert_session_event(id, ne).await?;
    }
    Ok(())
}
