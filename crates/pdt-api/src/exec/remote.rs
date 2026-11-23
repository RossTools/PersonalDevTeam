//! The remote instance's end of the executor: it runs the agents its owner starts (each adapter's outputs numbered in
//! a buffer the owner reads), keeps the project's checkout and worktrees, and passes its agents' pdt-mcp calls on to
//! the owner. Every command is checked with `link::from_owner`; nothing here acts on its own.

use super::*;
use crate::error::{ApiError, codes};
use crate::link::{self, PeerError};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use pdt_agents::adapter::{
    EVENT_QUEUE, EventStream, ImageFile, LiveConversation, OpenSpec, Outgoing, SessionChange, SessionInstructions,
    event_channel,
};
use pdt_core::link::OwnerLink;
use pdt_core::worktree::{WORKTREES_DIR, worktree_path};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ts_rs::TS;

/// The most events kept for owners that haven't read them; past it the oldest go (logged).
const OUTBOX_CAP: usize = 200_000;
/// The most events one read returns.
const BATCH: usize = 1_000;
/// The most run output (base64 bytes) kept for owners that haven't read it; past it the oldest output goes first, so
/// a noisy test never pushes out the sessions' events.
const OUTPUT_CAP: usize = 32 * 1024 * 1024;
/// The most run output one read returns (it returns at least one event).
const BATCH_OUTPUT: usize = 4 * 1024 * 1024;
/// The longest an owner's read waits for events.
const MAX_WAIT_MS: u64 = 25_000;
/// How long an agent's pdt-mcp call waits for the owner while the link is down, before it fails.
const RELAY_PATIENCE: Duration = Duration::from_secs(30 * 60);
/// How long one relayed call may take: a tool that asks the user waits for their answer.
const RELAY_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

struct Running {
    live: Arc<dyn LiveConversation>,
    proc: String,
    link_id: String,
}

struct Outbox {
    next: i64,
    /// By link: the events, oldest first.
    items: VecDeque<(String, Item)>,
    /// The run output in `items`, in bytes.
    output: usize,
}

impl Outbox {
    fn push(&mut self, link_id: &str, item: Item) {
        self.output += item.output_bytes();
        self.items.push_back((link_id.to_string(), item));
        if self.output > OUTPUT_CAP {
            tracing::warn!(link = link_id, "an owner hasn't read its runs' output: the oldest is dropped");
            while self.output > OUTPUT_CAP && self.drop_oldest_output() {}
        }
        if self.items.len() > OUTBOX_CAP && !self.drop_oldest_output() {
            self.items.pop_front();
            tracing::warn!(link = link_id, "an owner hasn't read its sessions' events: the oldest are dropped");
        }
    }

    /// Drops the oldest run output; false when there is none.
    fn drop_oldest_output(&mut self) -> bool {
        let Some(at) = self.items.iter().position(|(_, e)| e.output_bytes() > 0) else { return false };
        if let Some((_, e)) = self.items.remove(at) {
            self.output -= e.output_bytes();
        }
        true
    }

    /// Forgets the owner on `link_id`'s events up to `after` (it has them).
    fn forget(&mut self, link_id: &str, after: i64) {
        let mut freed = 0;
        self.items.retain(|(l, e)| {
            let keep = !(l == link_id && e.seq() <= after);
            if !keep {
                freed += e.output_bytes();
            }
            keep
        });
        self.output -= freed;
    }

    /// The owner on `link_id`'s events after `after`: up to [`BATCH`] of them, and [`BATCH_OUTPUT`] of run output.
    fn read(&self, link_id: &str, after: i64) -> Vec<Item> {
        let mut out = vec![];
        let mut output = 0;
        for (_, e) in self.items.iter().filter(|(l, e)| l == link_id && e.seq() > after) {
            output += e.output_bytes();
            if out.len() == BATCH || (!out.is_empty() && output > BATCH_OUTPUT) {
                break;
            }
            out.push(e.clone());
        }
        out
    }
}

/// One numbered report for an owner: a session's output, or a run's (`super::runs`).
// Most are sessions' events: boxing them would only add an allocation each.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
enum Item {
    Session(WireEvent),
    Run(super::runs::WireRunEvent),
}

impl Item {
    fn seq(&self) -> i64 {
        match self {
            Item::Session(e) => e.seq,
            Item::Run(e) => e.seq,
        }
    }

    /// The bytes of run output it carries.
    fn output_bytes(&self) -> usize {
        match self {
            Item::Run(super::runs::WireRunEvent { out: super::runs::RunOut::Output { data, .. }, .. }) => data.len(),
            _ => 0,
        }
    }
}

pub(super) struct RemoteSide {
    running: Mutex<HashMap<String, Running>>,
    outbox: Mutex<Outbox>,
    added: tokio::sync::Notify,
    /// This process: an owner that read another one's events starts again from the first.
    boot: String,
    /// Opens one at a time, so two opens of one session never start two agents.
    opening: tokio::sync::Mutex<()>,
}

impl Default for RemoteSide {
    fn default() -> Self {
        Self {
            running: Default::default(),
            outbox: Mutex::new(Outbox { next: 0, items: VecDeque::new(), output: 0 }),
            added: Default::default(),
            boot: uuid::Uuid::new_v4().to_string(),
            opening: Default::default(),
        }
    }
}

impl RemoteSide {
    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<String, Running>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(&self, link_id: &str, session_id: &str, proc: &str, out: WireOut) {
        let (session_id, proc) = (session_id.to_string(), proc.to_string());
        self.push_item(link_id, |seq| Item::Session(WireEvent { seq, session_id, proc, out }));
    }

    /// Numbers a run's report for the owner on `link_id`.
    pub(super) fn push_run(&self, link_id: &str, run_id: &str, proc: &str, out: super::runs::RunOut) {
        let (run_id, proc) = (run_id.to_string(), proc.to_string());
        self.push_item(link_id, |seq| Item::Run(super::runs::WireRunEvent { seq, run_id, proc, out }));
    }

    fn push_item(&self, link_id: &str, item: impl FnOnce(i64) -> Item) {
        {
            let mut o = self.outbox.lock().unwrap_or_else(|e| e.into_inner());
            o.next += 1;
            let seq = o.next;
            o.push(link_id, item(seq));
        }
        self.added.notify_waiters();
    }
}

/// A remote project's request from its owner, checked, with its body.
pub(super) async fn checked<T: serde::de::DeserializeOwned>(
    st: &AppState,
    method: &Method,
    uri: &OriginalUri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(OwnerLink, T), ApiError> {
    let link = link::from_owner(st, method, uri, headers, body).await?;
    let req = serde_json::from_slice(body).map_err(|e| ApiError::invalid(e.to_string()))?;
    Ok((link, req))
}

/// Where this instance keeps a remote project's checkout.
pub(crate) fn checkout_dir(st: &AppState, link: &OwnerLink) -> PathBuf {
    match link.checkout_path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => PathBuf::from(crate::host::expand_home(p, st.home_dir.get().as_deref())),
        None => st.data_dir.join("checkouts").join(&link.link_id),
    }
}

/// The live process of a session the owner on `link` runs here.
fn live_of(st: &AppState, link: &OwnerLink, session: &str) -> Result<Arc<dyn LiveConversation>, ApiError> {
    st.exec
        .remote
        .running()
        .get(session)
        .filter(|r| r.link_id == link.link_id && r.live.is_alive())
        .map(|r| r.live.clone())
        .ok_or_else(|| ApiError::conflict(codes::AGENT_NOT_RUNNING, "the agent is not running"))
}

/// `/link/exec/open`: starts the session's agent here (or, running already, tells the owner which process it is).
pub(crate) async fn open_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Opened>, ApiError> {
    let (link, req): (OwnerLink, OpenReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let checkout = checkout_dir(&st, &link);
    let cwd = match &req.branch {
        Some(b) => worktree_path(&checkout, b),
        None => checkout,
    };
    let cwd_text = cwd.to_string_lossy().into_owned();
    let _one = st.exec.remote.opening.lock().await;
    if let Some(r) = st.exec.remote.running().get(&req.session_id).filter(|r| r.live.is_alive())
        && r.link_id == link.link_id
    {
        return Ok(Json(Opened { proc: r.proc.clone(), cwd: cwd_text }));
    }
    if link.paused {
        let me = link::instance_name(&st).await?;
        return Err(ApiError::conflict(codes::REMOTE_PROJECT, format!("this project is paused on {me}")));
    }
    if !cwd.is_dir() {
        return Err(ApiError::conflict(
            codes::REMOTE_PROJECT,
            format!("{} has no worktree at {cwd_text}", link::instance_name(&st).await?),
        ));
    }
    let dir = st.data_dir.join("sessions").join(&req.session_id);
    let instructions = match &req.instructions {
        Some(i) => {
            let file = write_instructions(&dir, &i.file_name, &i.text)
                .map_err(|e| ApiError::io("writing the session's instructions", e))?;
            Some(SessionInstructions { text: i.text.clone(), file })
        }
        None => None,
    };
    let platform = st.agent_platform().await;
    let agent = req.agent;
    let program = match agent {
        AgentKind::Codex => platform.codex_program.clone(),
        AgentKind::Devin => platform.devin_program.clone(),
        AgentKind::Acp(_) => {
            let def = crate::conversations::acp_agent(&st, agent).await?;
            crate::host::expand_home(&def.command, st.home_dir.get().as_deref())
        }
        _ => platform.claude_program.clone(),
    };
    let resume = match agent {
        AgentKind::Claude => {
            let id = req.external_id.as_deref().unwrap_or(&req.session_id);
            req.has_history
                && pdt_agents::launch::claude_conversation_saved(
                    st.platform.claude_config_dir.as_deref(),
                    &cwd_text,
                    id,
                )
                .await
        }
        _ => req.external_id.is_some(),
    };
    let mcp = match &req.mcp {
        Some(m) => Some(mcp_server(&st, &dir, agent, m)?),
        None => None,
    };
    let spec = OpenSpec {
        session_id: req.session_id.clone(),
        external_id: req.external_id.clone(),
        resume,
        cwd: cwd.clone(),
        program,
        search_path: st.platform.login_path.clone(),
        model: req.model.clone(),
        permission_mode: req.permission_mode,
        env: crate::conversations::login_path_env(&st),
        env_remove: crate::conversations::scrubbed(req.credentials),
        instructions,
        config_file: None,
        import_after: None,
        mcp,
    };
    let adapter = crate::conversations::adapter_for(&st, Some(agent)).await?;
    let (tx, rx) = event_channel(EVENT_QUEUE);
    let live: Arc<dyn LiveConversation> =
        Arc::from(adapter.open(spec, tx).await.map_err(crate::conversations::adapter_error)?);
    let proc = uuid::Uuid::now_v7().to_string();
    st.exec
        .remote
        .running()
        .insert(req.session_id.clone(), Running { live, proc: proc.clone(), link_id: link.link_id.clone() });
    let row = pdt_store::ExecSession {
        session_id: req.session_id.clone(),
        link_id: link.link_id.clone(),
        agent: agent.as_str().to_string(),
        title: req.title.clone(),
        ticket_key: req.ticket_key.clone(),
        cwd: cwd_text.clone(),
        token: req.mcp.as_ref().map(|m| m.token.clone()).unwrap_or_default(),
        created_at: now_ms(),
        ended_at: None,
    };
    st.store.put_exec_session(&row).await?;
    tokio::spawn(forward(st.clone(), req.session_id.clone(), proc.clone(), link.link_id.clone(), rx));
    tracing::info!(session = %req.session_id, owner = %link.owner_name, "started an agent for the owner");
    Ok(Json(Opened { proc, cwd: cwd_text }))
}

fn write_instructions(dir: &Path, name: &str, text: &str) -> std::io::Result<PathBuf> {
    let name = Path::new(name).file_name().map(|n| n.to_owned()).unwrap_or_else(|| "instructions.md".into());
    std::fs::create_dir_all(dir)?;
    let path = dir.join(name);
    std::fs::write(&path, text)?;
    Ok(path)
}

/// pdt-mcp as this instance serves it to the owner's session: at its own address, with the owner's token and tools,
/// and none of this instance's own MCP servers.
fn mcp_server(
    st: &AppState,
    dir: &Path,
    agent: AgentKind,
    m: &WireMcp,
) -> Result<pdt_agents::mcp::McpServer, ApiError> {
    let codex_home = st.platform.codex_home.as_deref();
    let mut server = pdt_agents::mcp::McpServer {
        url: format!("http://127.0.0.1:{}/api/mcp", st.listen_port),
        token: m.token.clone(),
        tools: m.tools.clone(),
        asks: m.asks.clone(),
        pm: m.pm,
        locked: m.locked,
        claude_config: None,
        claude_settings: None,
        extra: vec![],
        only_pdt: m.only_pdt,
        codex_user_servers: match (agent, codex_home) {
            (AgentKind::Codex, Some(home)) => pdt_agents::mcp::codex_config_servers(home),
            _ => vec![],
        },
        codex_shell_exclude: match (agent, codex_home) {
            (AgentKind::Codex, Some(home)) => pdt_agents::mcp::codex_config_shell_exclude(home),
            _ => vec![],
        },
    };
    if agent == AgentKind::Claude {
        let io = |e| ApiError::io("writing the session's MCP config", e);
        server.claude_config = Some(pdt_agents::mcp::write_claude_config(dir, &server).map_err(io)?);
        server.claude_settings = pdt_agents::mcp::write_claude_settings(dir, &server).map_err(io)?;
    }
    Ok(server)
}

/// Numbers one process's outputs into the outbox, until it ends.
async fn forward(st: AppState, session: String, proc: String, link_id: String, mut rx: EventStream) {
    while let Some(out) = rx.recv().await {
        let exited = matches!(out, pdt_agents::adapter::AdapterOutput::Exited { .. });
        st.exec.remote.push(&link_id, &session, &proc, out.into());
        if exited {
            let mine = {
                let mut running = st.exec.remote.running();
                let mine = running.get(&session).is_some_and(|r| r.proc == proc);
                if mine {
                    running.remove(&session);
                }
                mine
            };
            if mine && let Err(e) = st.store.end_exec_session(&session, now_ms()).await {
                tracing::warn!(session = %session, error = %e, "could not record a remote session's end");
            }
        }
    }
}

pub(crate) async fn send_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let (link, req): (OwnerLink, SendReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let live = live_of(&st, &link, &req.session_id)?;
    let dir = st.data_dir.join("sessions").join(&req.session_id).join("attachments");
    let mut images = vec![];
    for img in req.images {
        let data = base64::engine::general_purpose::STANDARD
            .decode(img.data.as_bytes())
            .map_err(|e| ApiError::invalid(format!("an image that can't be read: {e}")))?;
        let name = Path::new(&img.name).file_name().map(|n| n.to_owned()).unwrap_or_else(|| "image".into());
        let path = dir.join(name);
        tokio::fs::create_dir_all(&dir).await.map_err(|e| ApiError::io("saving an image", e))?;
        tokio::fs::write(&path, data).await.map_err(|e| ApiError::io("saving an image", e))?;
        images.push(ImageFile { path, media_type: img.media_type });
    }
    live.send(&Outgoing { text: req.text, images }, req.delivery).await.map_err(crate::conversations::adapter_error)?;
    Ok(Json(Value::Null))
}

pub(crate) async fn answer_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let (link, req): (OwnerLink, AnswerReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let live = live_of(&st, &link, &req.session_id)?;
    live.answer(&req.request_id, &req.answer).await.map_err(crate::conversations::adapter_error)?;
    Ok(Json(Value::Null))
}

pub(crate) async fn interrupt_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let (link, req): (OwnerLink, SessionReq) = checked(&st, &method, &uri, &headers, &body).await?;
    live_of(&st, &link, &req.session_id)?.interrupt().await.map_err(crate::conversations::adapter_error)?;
    Ok(Json(Value::Null))
}

pub(crate) async fn set_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let (link, req): (OwnerLink, SetReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let change = SessionChange { model: req.set_model.then_some(req.model), permission_mode: req.permission_mode };
    live_of(&st, &link, &req.session_id)?.set(&change).await.map_err(crate::conversations::adapter_error)?;
    Ok(Json(Value::Null))
}

/// `/link/exec/close`: stops the session's agent (its exit goes to the owner with its events). Not running: nothing.
pub(crate) async fn close_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let (link, req): (OwnerLink, SessionReq) = checked(&st, &method, &uri, &headers, &body).await?;
    if let Ok(live) = live_of(&st, &link, &req.session_id) {
        live.close().await;
    }
    Ok(Json(Value::Null))
}

/// Stops every agent this instance runs for the owner on `link_id` (it removed this instance, or this one left).
pub(crate) async fn close_link(st: &AppState, link_id: &str) {
    let lives: Vec<Arc<dyn LiveConversation>> =
        st.exec.remote.running().values().filter(|r| r.link_id == link_id).map(|r| r.live.clone()).collect();
    futures_util::future::join_all(lives.iter().map(|l| l.close())).await;
    super::runs::stop_link(st, link_id).await;
}

/// `/link/exec/events`: the owner's long poll.
pub(crate) async fn events_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Batch>, ApiError> {
    let (link, req): (OwnerLink, PollReq) = checked(&st, &method, &uri, &headers, &body).await?;
    st.store.owner_contact(&link.link_id, now_ms(), None).await?;
    let side = &st.exec.remote;
    let same_boot = req.boot.as_deref() == Some(side.boot.as_str());
    let after = if same_boot { req.after } else { 0 };
    if same_boot {
        let mut o = side.outbox.lock().unwrap_or_else(|e| e.into_inner());
        o.forget(&link.link_id, after);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_millis(req.wait_ms.min(MAX_WAIT_MS));
    let items = loop {
        let added = side.added.notified();
        tokio::pin!(added);
        added.as_mut().enable();
        let items: Vec<Item> = { side.outbox.lock().unwrap_or_else(|e| e.into_inner()).read(&link.link_id, after) };
        if !items.is_empty() || tokio::time::Instant::now() >= deadline {
            break items;
        }
        let _ = tokio::time::timeout_at(deadline, added).await;
    };
    let (mut events, mut runs) = (vec![], vec![]);
    for item in items {
        match item {
            Item::Session(e) => events.push(e),
            Item::Run(e) => runs.push(e),
        }
    }
    let alive = side
        .running()
        .iter()
        .filter(|(_, r)| r.link_id == link.link_id && r.live.is_alive())
        .map(|(id, _)| id.clone())
        .collect();
    let runs_alive = super::runs::alive(&st, &link.link_id);
    Ok(Json(Batch { boot: side.boot.clone(), events, alive, runs, runs_alive }))
}

/// The remote project's checkout, cloned from the owner's `origin` on first use, else fetched (best effort).
async fn ensure_checkout(st: &AppState, link: &OwnerLink, base: &str) -> Result<PathBuf, ApiError> {
    let dir = checkout_dir(st, link);
    let (d, url, base) = (dir.clone(), link.repo_url.clone(), base.to_string());
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        if d.join(".git").exists() {
            if pdt_git::remote_url(&d, "origin").is_some()
                && let Err(e) = pdt_git::fetch(&d, "origin", &base, pdt_git::QUICK_FETCH_TIMEOUT)
            {
                tracing::info!(error = %e, "could not fetch the base branch; using the checkout's refs");
            }
            return Ok(());
        }
        if d.read_dir().is_ok_and(|mut r| r.next().is_some()) {
            return Err(ApiError::conflict(
                codes::REMOTE_PROJECT,
                format!("{} isn't empty and isn't a checkout of the project: pick another folder", d.display()),
            ));
        }
        let _ = std::fs::remove_dir(&d);
        let url = url.ok_or_else(|| {
            ApiError::conflict(codes::REMOTE_PROJECT, "the owner's repository has no origin to clone from")
        })?;
        pdt_git::clone(&url, &d)?;
        Ok(())
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(dir)
}

pub(crate) async fn worktree_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WorktreeMade>, ApiError> {
    let (link, req): (OwnerLink, WorktreeReq) = checked(&st, &method, &uri, &headers, &body).await?;
    pdt_core::worktree::NewWorktree { branch: req.branch.clone(), base: None }
        .validate()
        .map_err(|e| ApiError::invalid(e.to_string()))?;
    let repo = ensure_checkout(&st, &link, &req.base).await?;
    let target = worktree_path(&repo, &req.branch);
    let (r, t, b, base) = (repo.clone(), target.clone(), req.branch.clone(), req.base.clone());
    let (base_commit, _) = tokio::task::spawn_blocking(move || {
        pdt_git::ensure_excluded(&r, &format!("/{WORKTREES_DIR}/"))?;
        let from = if pdt_git::remote_url(&r, "origin").is_some() { pdt_git::start_point(&r, &base) } else { base };
        crate::worktrees::add_on_disk(&r, &t, &b, &from, req.existing_branch)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(WorktreeMade { path: target.to_string_lossy().into_owned(), base_commit }))
}

pub(crate) async fn remove_worktree_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Removed>, ApiError> {
    let (link, req): (OwnerLink, RemoveWorktreeReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let repo = checkout_dir(&st, &link);
    let path = worktree_path(&repo, &req.branch);
    let rows = st.store.list_exec_sessions(Some(&link.link_id)).await?;
    let busy = {
        let running = st.exec.remote.running();
        rows.iter().any(|e| Path::new(&e.cwd) == path && running.get(&e.session_id).is_some_and(|r| r.live.is_alive()))
    };
    if busy {
        return Err(ApiError::conflict(codes::WORKTREE_BUSY, "stop this branch's sessions first"));
    }
    let branch_error = tokio::task::spawn_blocking(move || {
        crate::worktrees::remove_on_disk(&repo, &path, &req.branch, req.force, req.delete_branch)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(Removed { branch_error }))
}

pub(crate) async fn git_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let (link, req): (OwnerLink, GitReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let repo = checkout_dir(&st, &link);
    let worktree = worktree_path(&repo, &req.branch);
    Ok(Json(match req.op {
        GitOp::HasChanges { base, base_commit } => {
            let branch = req.branch.clone();
            let changes = tokio::task::spawn_blocking(move || {
                let dirty = worktree.is_dir() && pdt_git::is_dirty(&worktree).unwrap_or(true);
                let origin = format!("origin/{base}");
                let own = |head: String| pdt_git::has_own_commits(&repo, &head, &base_commit, &[&base, &origin]);
                dirty || pdt_git::rev_parse(&repo, &format!("refs/heads/{branch}")).is_none_or(own)
            })
            .await
            .map_err(ApiError::internal)?;
            serde_json::json!({ "changes": changes })
        }
        GitOp::Head => {
            let branch = req.branch.clone();
            let head = tokio::task::spawn_blocking(move || pdt_git::rev_parse(&repo, &format!("refs/heads/{branch}")))
                .await
                .map_err(ApiError::internal)?;
            serde_json::json!({ "head": head })
        }
        GitOp::UpdateBase { base, remote_pr } => {
            let (port, output) = crate::workflow::nodes::conflicts::update_without_rules(
                &repo,
                &worktree,
                &base,
                &req.branch,
                remote_pr,
            )
            .await;
            serde_json::json!({ "port": port, "output": output })
        }
    }))
}

/// The session a pdt-mcp request's token names, when it is one this instance runs for an owner (with that token).
async fn relayed(st: &AppState, headers: &HeaderMap) -> Option<pdt_store::ExecSession> {
    let auth = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (id, mac) = pdt_core::mcp::parse_token(auth)?;
    let e = st.store.get_exec_session(id).await.ok()??;
    (!e.token.is_empty() && e.token == pdt_core::mcp::token(id, mac)).then_some(e)
}

/// Whether a pdt-mcp request is an owner's session's, which this instance passes on.
pub(crate) async fn is_relayed(st: &AppState, headers: &HeaderMap) -> bool {
    relayed(st, headers).await.is_some()
}

/// Passes an agent's pdt-mcp request on to its session's owner, retrying while the owner can't be reached: None when
/// the token isn't a relayed session's.
pub(crate) async fn relay_mcp(st: &AppState, headers: &HeaderMap, body: &Bytes) -> Option<Response> {
    let e = relayed(st, headers).await?;
    Some(match relay(st, &e, headers, body).await {
        Ok(r) => r,
        Err(err) => err.into_response(),
    })
}

async fn relay(
    st: &AppState,
    e: &pdt_store::ExecSession,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Response, ApiError> {
    let link = st
        .store
        .owner_link_by_id(&e.link_id)
        .await?
        .ok_or_else(|| ApiError::conflict(codes::REMOTE_PROJECT, "this instance left that project"))?;
    let key = link::load_key(st, &link.link_id).await?;
    let req = McpReq {
        token: e.token.clone(),
        accept: headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).map(String::from),
        body: String::from_utf8_lossy(body).into_owned(),
    };
    let started = tokio::time::Instant::now();
    let mut wait = Duration::from_millis(250);
    let reply: McpReply = loop {
        let signed = Some((&key, link.link_id.as_str(), link.owner_project_id.as_str()));
        match link::post_within(st, RELAY_TIMEOUT, &link.owner_url, MCP_PATH, signed, &req).await {
            Ok(r) => break r,
            Err(PeerError::Unreachable(why)) if started.elapsed() < RELAY_PATIENCE => {
                tracing::debug!(session = %e.session_id, error = %why, "the owner can't be reached: the tool call waits");
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(5));
            }
            Err(err) => return Err(err.into_api(&link.owner_name)),
        }
    };
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut res = (status, reply.body).into_response();
    if let Some(ct) = reply.content_type.and_then(|c| axum::http::HeaderValue::from_str(&c).ok()) {
        res.headers_mut().insert(header::CONTENT_TYPE, ct);
    }
    Ok(res)
}

/// A session this instance runs for an owner, as its remote project's page lists it (read-only).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunningView {
    pub session_id: String,
    pub title: String,
    pub agent: String,
    pub ticket_key: Option<String>,
    #[ts(type = "number")]
    pub started_at: i64,
    /// Its agent's process runs now (else it is parked, and starts again when the owner sends it something).
    pub live: bool,
}

/// What this instance runs for the owner on `link_id`.
pub(crate) async fn running_view(st: &AppState, link_id: &str) -> Vec<RunningView> {
    let rows = st.store.list_exec_sessions(Some(link_id)).await.unwrap_or_default();
    let running = st.exec.remote.running();
    rows.into_iter()
        .filter(|e| e.ended_at.is_none() || running.contains_key(&e.session_id))
        .map(|e| RunningView {
            live: running.get(&e.session_id).is_some_and(|r| r.live.is_alive()),
            session_id: e.session_id,
            title: e.title,
            agent: e.agent,
            ticket_key: e.ticket_key,
            started_at: e.created_at,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::runs::{RunOut, WireRunEvent};
    use super::*;

    fn output(seq: i64, bytes: usize) -> Item {
        let out = RunOut::Output { step: "command".into(), data: "x".repeat(bytes) };
        Item::Run(WireRunEvent { seq, run_id: "r".into(), proc: "p".into(), out })
    }

    fn session(seq: i64) -> Item {
        let out = WireOut::ExternalId { id: "t".into() };
        Item::Session(WireEvent { seq, session_id: "s".into(), proc: "p".into(), out })
    }

    /// Run output is kept by size, oldest dropped first, and never pushes out the sessions' events; a read takes a
    /// bounded share of it.
    #[test]
    fn run_output_is_bounded_by_size_and_never_drops_session_events() {
        let mut o = Outbox { next: 0, items: VecDeque::new(), output: 0 };
        let chunk = 1024 * 1024;
        o.push("l", session(1));
        for seq in 2..(2 + 40) {
            o.push("l", output(seq, chunk));
        }
        o.push("l", session(42));
        assert!(o.output <= OUTPUT_CAP, "{}", o.output);
        let seqs: Vec<i64> = o.items.iter().map(|(_, e)| e.seq()).collect();
        assert_eq!((seqs[0], *seqs.last().unwrap()), (1, 42), "the sessions' events stay");
        assert_eq!(seqs[1], 2 + 40 - (OUTPUT_CAP / chunk) as i64, "the oldest output went: {seqs:?}");

        let read = o.read("l", 0);
        let read_bytes: usize = read.iter().map(Item::output_bytes).sum();
        assert!(
            read_bytes <= BATCH_OUTPUT && read.len() == 1 + BATCH_OUTPUT / chunk,
            "{} events, {read_bytes} bytes",
            read.len()
        );
        assert!(o.read("other", 0).is_empty());

        let last = read.last().unwrap().seq();
        o.forget("l", last);
        assert_eq!(o.output, o.items.iter().map(|(_, e)| e.output_bytes()).sum::<usize>());
        assert_eq!(o.items.front().map(|(_, e)| e.seq()), Some(last + 1));
        // One event bigger than a read's share still goes out alone.
        let mut big = Outbox { next: 0, items: VecDeque::new(), output: 0 };
        big.push("l", output(1, BATCH_OUTPUT + 1));
        assert_eq!(big.read("l", 0).len(), 1);
    }
}
