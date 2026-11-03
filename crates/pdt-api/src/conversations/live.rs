//! A conversation's runtime: its live agent process, the events it sends (stored in order, fanned out to viewers) and the
//! activity derived from them.

use super::terminal::{import_anchor, read_transcript};
use super::{Conv, ConvState, acp_agent, adapter_error, adapter_for, login_path_env, scrubbed};
use crate::error::ApiError;
use crate::log_err::LogErr;
use crate::routes::{AppState, now_ms};
use pdt_agents::adapter::{AdapterOutput, EventStream, LiveConversation, OpenSpec};
use pdt_core::activity::{Activity, needs_attention};
use pdt_core::conversation::{
    AgentEvent, ConversationServerMessage, DeltaField, PendingMessage, RequestState, SessionMode, TurnPhase,
    conversation_activity,
};
use pdt_core::host::CredentialPolicy;
use pdt_core::session::{AgentKind, Session, SessionStatus};
use pdt_store::NewEvent;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// The runtime for a conversation session, made from the stored row on first use.
pub(crate) async fn conv(st: &AppState, session: &Session) -> Result<Arc<Conv>, ApiError> {
    if let Some(c) = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner()).get(&session.id) {
        return Ok(c.clone());
    }
    let has_history = st.store.last_session_event_seq(&session.id).await? > 0;
    if has_history {
        repair_gaps(st, session).await;
    }
    let stopped_background = st.store.stopped_background(&session.id).await?;
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
            background_tasks: Vec::new(),
            stopped_background,
        }),
        activity_write: tokio::sync::Mutex::new(()),
        sending: tokio::sync::RwLock::new(()),
        exits: tokio::sync::watch::channel(0).0,
        unstored: tokio::sync::Mutex::new(std::collections::VecDeque::new()),
        retrying: std::sync::atomic::AtomicBool::new(false),
    });
    let mut map = st.conversations.map.lock().unwrap_or_else(|e| e.into_inner());
    Ok(map.entry(session.id.clone()).or_insert(c).clone())
}

/// Fills in the gaps in a Claude conversation's log from Claude's transcript, once per pdtd, as it loads (PDT-331):
/// events PDT received but could not store (a full disk) before pdtd stopped. Loads of other conversations wait
/// meanwhile, and a second load of this one finds it done.
async fn repair_gaps(st: &AppState, session: &Session) {
    if !matches!(session.agent, None | Some(AgentKind::Claude)) {
        return;
    }
    let mut checked = st.conversations.repaired.lock().await;
    if !checked.insert(session.id.clone()) {
        return;
    }
    // A terminal's turns waiting to be imported are not a gap.
    if !matches!(st.store.history_from(&session.id).await, Ok(None)) {
        return;
    }
    let Some(transcript) = read_transcript(st, session.claude_conversation_id()).await else { return };
    let cwd = session.cwd.clone();
    let history = tokio::task::spawn_blocking(move || {
        pdt_core::conversation::claude::history_events(&transcript, None, Some(&cwd)).map(|events| {
            events
                .into_iter()
                .filter_map(|out| match out {
                    pdt_core::conversation::claude::MapOut::Event { item_id, turn_id, event } => {
                        Some((item_id, turn_id, event))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
    })
    .await;
    let Ok(Ok(history)) = history else { return };
    let stored = match st.store.session_item_ids(&session.id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(session = %session.id, error = %e, "could not read the conversation's items");
            return;
        }
    };
    for gap in pdt_core::conversation::repair::gaps(&history, &stored) {
        let events: Vec<NewEvent> = gap
            .events
            .iter()
            .map(|(item, turn, event)| NewEvent {
                ts: 0,
                item_id: Some(item),
                turn_id: turn.as_deref(),
                event,
                raw: None,
            })
            .collect();
        match st.store.insert_session_events_after(&session.id, &gap.after, &events).await {
            Ok(n) => {
                tracing::info!(session = %session.id, after = %gap.after, events = n, "filled in a gap in the conversation")
            }
            Err(e) => tracing::warn!(session = %session.id, error = %e, "could not fill in a gap in the conversation"),
        }
    }
}

/// Starts the agent process for a conversation if it is not running, resuming its saved conversation. Marks the
/// session running.
pub(super) async fn ensure_live(
    st: &AppState,
    c: &Arc<Conv>,
    session: &Session,
) -> Result<Arc<dyn LiveConversation>, ApiError> {
    let mut slot = c.live.lock().await;
    if let Some((l, _)) = slot.as_ref().filter(|(l, _)| l.is_alive()) {
        return Ok(l.clone());
    }
    let adapter = adapter_for(st, session.agent).await?;
    let creds = crate::sessions::credentials_for(st, &session.project_id).await?;
    let has_history = c.state().has_history;
    let row = st.store.get_session(&session.id).await?.unwrap_or_else(|| session.clone());
    let (program, resume) = match session.agent {
        // Codex resumes its thread whenever it has one; Devin its session (it starts a new one when that was never
        // saved).
        Some(AgentKind::Codex) => (st.agent_platform().await.codex_program, row.external_session_id.is_some()),
        Some(AgentKind::Devin) => (st.agent_platform().await.devin_program, row.external_session_id.is_some()),
        // An ACP agent resumes its session when it has one (and can load sessions).
        Some(agent @ AgentKind::Acp(_)) => {
            let def = acp_agent(st, agent).await?;
            (crate::host::expand_home(&def.command, st.home_dir.as_deref()), row.external_session_id.is_some())
        }
        // Claude saves a conversation only after its first turn; resuming one that was never saved fails.
        _ => {
            let saved =
                st.platform.claude_config_dir.as_deref().is_none_or(|dir| {
                    pdt_agents::launch::claude_conversation_exists(dir, row.claude_conversation_id())
                });
            (st.agent_platform().await.claude_program, has_history && saved)
        }
    };
    // Devin: turns taken in its own terminal since the switch (or, for a session that was always a terminal, all of
    // them) come in with `session/load`'s replay.
    let import_after = match session.agent {
        Some(AgentKind::Devin) if row.external_session_id.is_some() => match import_anchor(st, &session.id).await? {
            Some(None) => Some(0),
            Some(Some(at)) => at.parse::<i64>().ok(),
            None => None,
        },
        _ => None,
    };
    let instructions = crate::sessions::agent_instructions(st, &row).await?;
    let config_file = match session.agent {
        Some(AgentKind::Devin) => {
            crate::sessions::devin_config(st, &session.id, instructions.as_ref().map(|i| i.file.as_path()))
                .map_err(|e| ApiError::io("writing the session's Devin config", e))?
        }
        _ => None,
    };
    let spec = OpenSpec {
        session_id: session.id.clone(),
        external_id: row.external_session_id.clone(),
        resume,
        cwd: PathBuf::from(&session.cwd),
        program,
        search_path: st.platform.login_path.clone(),
        // Its own model (or its persona's), else the agent's default model in Settings → Agents.
        model: st.store.get_host_settings().await?.model_for(row.agent, row.model.as_deref()),
        permission_mode: row.permission_mode.unwrap_or_default(),
        env: login_path_env(st),
        env_remove: scrubbed(creds),
        instructions,
        config_file,
        import_after,
        mcp: crate::mcp::server_for(st, &row).await,
    };
    // Bounded: a store that lags pauses reading the agent's output (PDT-441).
    let (tx, rx) = pdt_agents::adapter::event_channel(pdt_agents::adapter::EVENT_QUEUE);
    let live: Arc<dyn LiveConversation> = Arc::from(adapter.open(spec, tx).await.map_err(adapter_error)?);
    if import_after.is_some() {
        st.store.switch_session_mode(&session.id, SessionMode::Conversation, None).await?;
    }
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
    mut rx: EventStream,
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
                // Shown live, never stored: a secret's value in one chunk is redacted here, its whole message once it
                // is stored (PDT-158).
                let text = st.store.redactor().string(&text);
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
                    st.store.set_session_permission_mode(&c.id, m).await.log_warn("save the session's permission mode");
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
            AdapterOutput::BackgroundTasks(tasks) => {
                // Kept with the session, so a restart knows what it stopped (PDT-311).
                if let Err(e) = st.store.set_session_background(&c.id, &tasks, now_ms()).await {
                    tracing::warn!(session = %c.id, error = %e, "could not record the background tasks");
                }
                {
                    let mut s = c.state();
                    s.background_tasks = tasks;
                    s.last_active = Instant::now();
                }
                c.push_state();
            }
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
                // Ended with its process; one PDT's shutdown ended stays recorded, for the agent to be told after
                // the restart (PDT-311).
                let had_background = !c.state().background_tasks.is_empty();
                if had_background
                    && !st.shutting_down.load(std::sync::atomic::Ordering::SeqCst)
                    && let Err(e) = st.store.set_session_background(&c.id, &[], now_ms()).await
                {
                    tracing::warn!(session = %c.id, error = %e, "could not clear the background tasks");
                }
                {
                    let mut s = c.state();
                    s.live = false;
                    s.turn_running = false;
                    s.open_requests.clear();
                    s.partial.clear();
                    s.background_tasks.clear();
                    if !expected {
                        let code = code.map_or("no exit code".to_string(), |c| format!("exit code {c}"));
                        let mut why = format!("{} stopped unexpectedly ({code}).", agent.label());
                        if !stderr_tail.is_empty() {
                            why.push('\n');
                            why.push_str(&stderr_tail);
                        }
                        s.stopped = Some(why);
                    }
                }
                if !expected {
                    crate::workflow::sensors::session_exited(&st, &c.id, code, false).await;
                    // What it had not taken yet is lost with it: say so (PDT-129).
                    let lost: Vec<String> = c.state().pending.drain(..).map(|p| p.id).collect();
                    let why = format!("{} stopped before it took it.", agent.label());
                    crate::workflow::outbox::lost(&st, &lost, Some(&why));
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
/// A call to an integration's tool gets the title its integration gives it (`Integration::approval_title`: what the
/// user says yes to), and a credential the agent wrote into it (PDT-150) is redacted from the event (the transcript and
/// the approval card, the call's input and output) and its raw line (`Integration::redact_call`); the call itself
/// still gets it. The raw line without them, when there were any.
fn integration_hooks(st: &AppState, event: &mut AgentEvent, raw: Option<&str>) -> Option<String> {
    let registry = st.registry();
    let (AgentEvent::ToolCall { tool, title, input, .. } | AgentEvent::Request { tool, title, input, .. }) = event
    else {
        return None;
    };
    if let Some(t) = registry.approval_title(tool, input) {
        *title = t;
    }
    let secrets = registry.redact_call(tool, input);
    if secrets.is_empty() {
        return None;
    }
    use pdt_core::secret::REDACTED;
    let scrub = |text: &str| {
        secrets.iter().fold(text.to_string(), |t, s| {
            let escaped = serde_json::to_string(s).unwrap_or_default();
            t.replace(escaped.trim_matches('"'), REDACTED).replace(s.as_str(), REDACTED)
        })
    };
    if let AgentEvent::ToolCall { output: Some(output), .. } = event {
        *output = scrub(output);
    }
    raw.map(scrub)
}

pub(super) async fn append(
    st: &AppState,
    c: &Arc<Conv>,
    item_id: Option<&str>,
    turn_id: Option<&str>,
    event: AgentEvent,
    raw: Option<&str>,
) {
    // When it happened: a write that has to wait keeps it (PDT-331).
    let ts = now_ms();
    // The agent echoes a message without PDT's attachment ids (the stored row names them, for the thumbnails) and
    // without saying that PDT sent it on its own.
    let mut event = event;
    if let AgentEvent::UserMessage { text, images, source, pdt } = &mut event
        && *source != pdt_core::conversation::MessageSource::Terminal
    {
        let s = c.state();
        if let Some(p) = pending_match(&s.pending, text) {
            if !p.images.is_empty() {
                *images = p.images.clone();
            }
            if p.notice && p.text.trim() == text.trim() {
                *source = pdt_core::conversation::MessageSource::Notice;
                *pdt = p.pdt;
            }
        }
    }
    let scrubbed = integration_hooks(st, &mut event, raw);
    let raw = scrubbed.as_deref().or(raw);
    // The runtime state follows the event as it comes, whether or not it can be stored yet: a message the agent took
    // leaves the queue even while the disk is full (PDT-331). What reads the log waits until it is stored.
    let (state_changed, then) = apply_event(c, item_id, &event);
    let unstored = Unstored {
        ts,
        item_id: item_id.map(String::from),
        turn_id: turn_id.map(String::from),
        bytes: Unstored::size(&event, raw),
        event,
        raw: raw.map(String::from),
        then,
    };
    store_in_order(st, c, unstored).await;
    if state_changed {
        c.push_state();
        refresh_activity(st, c).await;
    }
}

/// What an event sets off once it is stored (never when it can't be: its session was removed).
enum FollowUp {
    /// An agent step's session ended a turn: did it submit an outcome (Phase 7b spec §4.4)? A PM session's may be due
    /// its daily update (PDT-167), or end once the user closed it.
    TurnEnded,
    /// One of PDT's asking tools in a project that lets the PM act is answered for the user (PDT-81).
    PdtRequest { id: String, tool: String, input: serde_json::Value },
    /// A PM conversation writes a journal checkpoint and carries on (PDT-167).
    Compacted,
    /// Whatever a message the agent took carried was told (PDT-129).
    Echoed(String),
}

impl FollowUp {
    fn spawn(self, st: &AppState, session: &str) {
        let (st, sid) = (st.clone(), session.to_string());
        match self {
            FollowUp::TurnEnded => tokio::spawn(async move {
                crate::workflow::agent::turn_ended(&st, &sid).await;
                crate::pm::turn_ended(&st, &sid).await;
            }),
            FollowUp::PdtRequest { id, tool, input } => {
                tokio::spawn(async move { crate::pm::request_opened(&st, &sid, &id, &tool, &input).await })
            }
            FollowUp::Compacted => tokio::spawn(async move { crate::pm::compacted(&st, &sid).await }),
            FollowUp::Echoed(pending) => {
                tokio::spawn(async move { crate::workflow::outbox::echoed(&st, &pending).await })
            }
        };
    }
}

/// Applies an event to the conversation's runtime state: whether the state viewers see changed, and what it sets off.
fn apply_event(c: &Conv, item_id: Option<&str>, event: &AgentEvent) -> (bool, Vec<FollowUp>) {
    let mut then = Vec::new();
    let mut s = c.state();
    s.has_history = true;
    s.last_active = Instant::now();
    if let Some(item) = item_id {
        s.partial.remove(item);
    }
    let changed = match event {
        AgentEvent::Turn { phase: TurnPhase::Started, .. } => {
            s.turn_running = true;
            s.errored = false;
            true
        }
        AgentEvent::Turn { phase: TurnPhase::Ended, .. } => {
            s.turn_running = false;
            // A message streamed in a turn that ended without it is not coming (PDT-331); a tool's output can still
            // grow (a background task).
            s.partial.retain(|_, (field, _)| *field != DeltaField::Markdown);
            then.push(FollowUp::TurnEnded);
            true
        }
        AgentEvent::Request { id, state: RequestState::Open, tool, input, .. } => {
            s.open_requests.insert(id.clone());
            if tool.starts_with(&format!("mcp__{}__", pdt_core::mcp::SERVER_NAME)) {
                then.push(FollowUp::PdtRequest { id: id.clone(), tool: tool.clone(), input: input.clone() });
            }
            true
        }
        AgentEvent::Request { id, .. } => s.open_requests.remove(id),
        AgentEvent::Compaction { .. } => {
            then.push(FollowUp::Compacted);
            false
        }
        AgentEvent::Error { .. } => {
            s.errored = true;
            true
        }
        AgentEvent::Usage { rate_limits, .. } if !rate_limits.is_empty() => {
            s.limited_until = pdt_core::conversation::limited_until(rate_limits, now_ms());
            false
        }
        AgentEvent::UserMessage { text, source, .. } if *source != pdt_core::conversation::MessageSource::Terminal => {
            // The agent took a message: it is no longer pending.
            if let Some(i) = pending_index(&s.pending, text) {
                let p = s.pending.remove(i);
                then.push(FollowUp::Echoed(p.id));
            }
            true
        }
        _ => false,
    };
    (changed, then)
}

/// An event received and not stored yet.
pub(super) struct Unstored {
    ts: i64,
    item_id: Option<String>,
    turn_id: Option<String>,
    event: AgentEvent,
    raw: Option<String>,
    /// About how much memory it holds: its event and raw line.
    bytes: usize,
    /// What it sets off once stored.
    then: Vec<FollowUp>,
}

impl Unstored {
    fn size(event: &AgentEvent, raw: Option<&str>) -> usize {
        serde_json::to_vec(event).map_or(0, |v| v.len()) + raw.map_or(0, str::len)
    }
}

/// At most this many events, and about this many bytes of them, wait for a write in one conversation; past either the
/// oldest is given up (and logged).
const MAX_UNSTORED: usize = 10_000;
const MAX_UNSTORED_BYTES: usize = 64 * 1024 * 1024;

/// Stores an event after those still waiting for a write, in the order they came (PDT-331), and sends each one stored
/// to the viewers. One that can't be stored yet (a full disk) waits in memory, with the ones after it, and is retried
/// with a backoff until it is. A later version of a waiting item replaces it there, keeping its place.
async fn store_in_order(st: &AppState, c: &Arc<Conv>, e: Unstored) {
    let mut q = c.unstored.lock().await;
    let waiting = e.item_id.as_ref().and_then(|item| q.iter_mut().find(|w| w.item_id.as_ref() == Some(item)));
    if let Some(w) = waiting {
        w.ts = e.ts;
        w.event = e.event;
        w.bytes = e.bytes;
        if e.turn_id.is_some() {
            w.turn_id = e.turn_id;
        }
        if e.raw.is_some() {
            w.raw = e.raw;
        }
        w.then.extend(e.then);
    } else {
        q.push_back(e);
    }
    for lost in give_up_past(&mut q, MAX_UNSTORED, MAX_UNSTORED_BYTES) {
        tracing::error!(session = %c.id, kind = lost.event.kind(), "gave up on a conversation event PDT could not store");
    }
    // While a retry waits and writes still fail, this one waits with the rest instead of failing again.
    if c.retrying.load(Ordering::SeqCst) && st.store.write_failure().is_some() {
        return;
    }
    if !flush_unstored(st, c, &mut q).await && !c.retrying.swap(true, Ordering::SeqCst) {
        tracing::warn!(session = %c.id, waiting = q.len(), "conversation events wait to be stored");
        let (st, c) = (st.clone(), c.clone());
        tokio::spawn(async move { retry_unstored(&st, &c).await });
    }
}

/// Takes the oldest waiting events off while more than `max` wait or they hold more than `max_bytes` (never the
/// newest).
fn give_up_past(q: &mut std::collections::VecDeque<Unstored>, max: usize, max_bytes: usize) -> Vec<Unstored> {
    let mut bytes: usize = q.iter().map(|w| w.bytes).sum();
    let mut lost = vec![];
    while q.len() > 1
        && (q.len() > max || bytes > max_bytes)
        && let Some(e) = q.pop_front()
    {
        bytes -= e.bytes;
        lost.push(e);
    }
    lost
}

/// Stores the waiting events in order, sending each to the viewers, until one fails to write: whether all were.
pub(super) async fn flush_unstored(st: &AppState, c: &Arc<Conv>, q: &mut std::collections::VecDeque<Unstored>) -> bool {
    while let Some(e) = q.front() {
        let new = NewEvent {
            ts: e.ts,
            item_id: e.item_id.as_deref(),
            turn_id: e.turn_id.as_deref(),
            event: &e.event,
            raw: e.raw.as_deref(),
        };
        match st.store.upsert_session_event(&c.id, new).await {
            Ok(row) => {
                if let Some(done) = q.pop_front() {
                    for f in done.then {
                        f.spawn(st, &c.id);
                    }
                }
                let _ = st.conversations.all.send((c.id.clone(), row.clone()));
                let _ = c.tx.send(ConversationServerMessage::Event { event: row });
            }
            Err(pdt_store::StoreError::Write(e)) => {
                tracing::debug!(session = %c.id, error = %e, "could not store a conversation event yet");
                return false;
            }
            // Not a write that may work later: retrying would hold up every event after it.
            Err(err) => {
                tracing::warn!(session = %c.id, error = %err, "could not store a conversation event");
                q.pop_front();
            }
        }
    }
    true
}

/// Retries a conversation's waiting events with a backoff until they are all stored.
async fn retry_unstored(st: &AppState, c: &Arc<Conv>) {
    let mut tries = 0;
    loop {
        let wait = u64::try_from(pdt_store::write_backoff_ms(tries)).unwrap_or(30_000);
        tokio::time::sleep(Duration::from_millis(wait)).await;
        tries += 1;
        let mut q = c.unstored.lock().await;
        if flush_unstored(st, c, &mut q).await {
            c.retrying.store(false, Ordering::SeqCst);
            tracing::info!(session = %c.id, "stored the conversation events that were waiting");
            return;
        }
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
pub(super) async fn refresh_activity(st: &AppState, c: &Arc<Conv>) {
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
    crate::notifications::kick(st);
    crate::workflow::sensors::activity_changed(st, &c.id, prev, next);
    c.push_state();
}

/// Closes a live process, keeping the conversation (the next message reopens it). Returns once its last events (an
/// interrupted turn, expired requests) are stored.
pub(super) async fn park(_st: &AppState, c: &Arc<Conv>) {
    let taken = c.live.lock().await.take();
    if let Some((live, generation)) = taken {
        live.close().await;
        let mut exits = c.exits.subscribe();
        let _ = tokio::time::timeout(Duration::from_secs(5), exits.wait_for(|g| *g >= generation)).await;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting(text: &str, raw: Option<&str>) -> Unstored {
        let event = AgentEvent::Error { text: text.into(), retryable: false };
        Unstored {
            ts: 0,
            item_id: None,
            turn_id: None,
            bytes: Unstored::size(&event, raw),
            event,
            raw: raw.map(String::from),
            then: vec![],
        }
    }

    fn texts(q: &std::collections::VecDeque<Unstored>) -> Vec<String> {
        q.iter()
            .map(|w| match &w.event {
                AgentEvent::Error { text, .. } => text.clone(),
                _ => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn waiting_events_are_bounded_by_count_and_by_bytes() {
        let mut q: std::collections::VecDeque<_> = ["a", "b", "c"].iter().map(|t| waiting(t, None)).collect();
        assert_eq!(give_up_past(&mut q, 2, usize::MAX).len(), 1);
        assert_eq!(texts(&q), ["b", "c"], "the oldest goes first");

        let big = "x".repeat(10_000);
        let mut q: std::collections::VecDeque<_> =
            ["a", "b", "c"].iter().map(|t| waiting(t, Some(big.as_str()))).collect();
        assert_eq!(give_up_past(&mut q, 100, 25_000).len(), 1, "two of them fit");
        assert_eq!(texts(&q), ["b", "c"]);

        let mut q: std::collections::VecDeque<_> = [waiting("huge", Some(big.as_str()))].into();
        assert!(give_up_past(&mut q, 100, 10).is_empty(), "the newest is kept, however large");
    }
}
