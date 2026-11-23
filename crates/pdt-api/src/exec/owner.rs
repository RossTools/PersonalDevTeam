//! The owner's end of the executor: remote sessions' live processes ([`RemoteLive`]), reading each joined instance's
//! events into its conversations ([`poll_loop`]), the git calls an attempt on a remote instance needs, and the agents'
//! pdt-mcp calls the remote instance passes on ([`mcp_route`]).

use super::*;
use crate::error::{ApiError, codes};
use crate::link::{self, PeerError};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::response::IntoResponse;
use base64::Engine;
use pdt_agents::adapter::{
    AdapterError, EVENT_QUEUE, EventSink, EventStream, LiveConversation, Outgoing, SessionChange, event_channel,
};
use pdt_core::link::RemoteInstance;
use pdt_core::project::Project;
use pdt_core::session::{Session, SessionStatus};
use pdt_core::worktree::Worktree;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long starting an agent there may take (it may be slow to answer, like a local one).
const OPEN_TIMEOUT: Duration = Duration::from_secs(120);
/// How long making a worktree may take: the first one clones the repository.
const WORKTREE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long a git look or update there may take (a fetch, a merge, a push).
const GIT_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Where a remote session's outputs go on the owner: its conversation's sink.
struct Route {
    sink: EventSink,
    /// The process its outputs come from; None while it is being opened.
    proc: Option<String>,
    alive: Arc<AtomicBool>,
    link_id: String,
    /// When the open came back (ms): a poll sent after it lists the process as alive while it runs.
    opened_at: i64,
}

pub(super) struct OwnerSide {
    routes: Mutex<HashMap<String, Route>>,
    /// The links an event poller runs for.
    pollers: Mutex<HashSet<String>>,
    wait_ms: AtomicU64,
    retry_ms: AtomicU64,
}

impl Default for OwnerSide {
    fn default() -> Self {
        Self {
            routes: Default::default(),
            pollers: Default::default(),
            wait_ms: AtomicU64::new(5_000),
            retry_ms: AtomicU64::new(2_000),
        }
    }
}

impl OwnerSide {
    pub(super) fn set_timing(&self, wait: Duration, retry: Duration) {
        self.wait_ms.store(wait.as_millis() as u64, Ordering::SeqCst);
        self.retry_ms.store(retry.as_millis() as u64, Ordering::SeqCst);
    }

    fn routes(&self) -> std::sync::MutexGuard<'_, HashMap<String, Route>> {
        self.routes.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Whether `session`'s live process runs on a remote instance (a shutdown leaves it running there).
pub(crate) fn is_routed(st: &AppState, session: &str) -> bool {
    st.exec.owner.routes().contains_key(session)
}

/// The joined instance `instance_id` of `project_id`, or why there is none.
pub(super) async fn instance(st: &AppState, project_id: &str, instance_id: &str) -> Result<RemoteInstance, ApiError> {
    st.store
        .list_remote_instances(Some(project_id))
        .await?
        .into_iter()
        .find(|i| i.instance_id == instance_id)
        .ok_or_else(|| ApiError::conflict(codes::REMOTE_PROJECT, "that remote instance has left the project"))
}

/// The instance a worktree is on, when it is a remote one.
pub(crate) async fn worktree_instance(st: &AppState, wt: &Worktree) -> Result<Option<RemoteInstance>, ApiError> {
    match &wt.instance {
        Some(i) => Ok(Some(instance(st, &wt.project_id, &i.id).await?)),
        None => Ok(None),
    }
}

/// A signed call to the remote instance `i`.
pub(super) async fn call<T: serde::de::DeserializeOwned>(
    st: &AppState,
    i: &RemoteInstance,
    path: &str,
    body: &impl Serialize,
    timeout: Duration,
) -> Result<T, PeerError> {
    let key = link::load_key(st, &i.link_id).await.map_err(|e| PeerError::Unreachable(e.message().to_string()))?;
    link::post_within(st, timeout, &i.url, path, Some((&key, &i.link_id, &i.project_id)), body).await
}

/// Opens (or attaches to) a remote session's agent on its instance: the live process and its outputs, for
/// `conversations::ensure_live`.
pub(crate) async fn open(
    st: &AppState,
    session: &Session,
    row: &Session,
) -> Result<(Arc<dyn LiveConversation>, EventStream), ApiError> {
    let on = session.instance.as_ref().ok_or_else(|| ApiError::internal("not a remote session"))?;
    let i = instance(st, &session.project_id, &on.id).await?;
    let branch = match &row.worktree_id {
        Some(w) => st.store.get_worktree(w).await?.map(|w| w.branch),
        None => None,
    };
    let instructions = crate::sessions::agent_instructions(st, row).await?.map(|ins| WireInstructions {
        file_name: ins.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or("instructions.md".into()),
        text: ins.text,
    });
    let mcp = crate::mcp::server_for(st, row).await.map(|m| WireMcp {
        token: m.token,
        tools: m.tools,
        asks: m.asks,
        pm: m.pm,
        locked: m.locked,
        only_pdt: m.only_pdt,
    });
    let req = OpenReq {
        session_id: row.id.clone(),
        agent: row.agent.unwrap_or(AgentKind::Claude),
        title: row.title.clone(),
        ticket_key: row.ticket.as_ref().map(|t| t.key.clone()),
        branch,
        external_id: row.external_session_id.clone(),
        has_history: st.store.last_session_event_seq(&row.id).await? > 0,
        model: st.store.get_host_settings().await?.model_for(row.agent, row.model.as_deref()),
        permission_mode: row.permission_mode.unwrap_or_default(),
        credentials: crate::sessions::credentials_for(st, &row.project_id).await?,
        instructions,
        mcp,
    };
    let (sink, stream) = event_channel(EVENT_QUEUE);
    let alive = Arc::new(AtomicBool::new(true));
    st.exec.owner.routes().insert(
        row.id.clone(),
        Route { sink, proc: None, alive: alive.clone(), link_id: i.link_id.clone(), opened_at: 0 },
    );
    ensure_poller(st, &i.link_id);
    let opened: Opened = match call(st, &i, OPEN_PATH, &req, OPEN_TIMEOUT).await {
        Ok(o) => o,
        Err(e) => {
            st.exec.owner.routes().remove(&row.id);
            return Err(e.into_api(&i.name));
        }
    };
    if let Some(r) = st.exec.owner.routes().get_mut(&row.id) {
        r.proc = Some(opened.proc);
        r.opened_at = now_ms();
    }
    let live = RemoteLive { st: st.clone(), instance: i, session_id: row.id.clone(), alive };
    Ok((Arc::new(live), stream))
}

/// A remote session's live process, as its conversation on the owner sees it.
pub(crate) struct RemoteLive {
    st: AppState,
    instance: RemoteInstance,
    session_id: String,
    alive: Arc<AtomicBool>,
}

fn adapter_error(e: PeerError, who: &str) -> AdapterError {
    match e {
        PeerError::Refused { code, .. } if code == codes::REQUEST_CLOSED => AdapterError::RequestClosed,
        PeerError::Refused { code, .. } if code == codes::WRONG_ANSWER => AdapterError::WrongAnswer,
        PeerError::Refused { code, .. } if code == codes::AGENT_NOT_RUNNING => AdapterError::NotRunning,
        PeerError::Refused { message, .. } => AdapterError::Refused(format!("{who}: {message}")),
        PeerError::Unreachable(e) => AdapterError::Refused(format!("couldn't reach {who}: {e}")),
    }
}

impl RemoteLive {
    async fn call(&self, path: &str, body: &impl Serialize) -> Result<(), AdapterError> {
        let _: Value = call(&self.st, &self.instance, path, body, OPEN_TIMEOUT)
            .await
            .map_err(|e| adapter_error(e, &self.instance.name))?;
        Ok(())
    }

    fn of(&self) -> SessionReq {
        SessionReq { session_id: self.session_id.clone() }
    }
}

#[async_trait::async_trait]
impl LiveConversation for RemoteLive {
    async fn send(&self, msg: &Outgoing, delivery: Delivery) -> Result<(), AdapterError> {
        let mut images = vec![];
        for img in &msg.images {
            let data = tokio::fs::read(&img.path).await.map_err(|e| AdapterError::Refused(e.to_string()))?;
            images.push(WireImage {
                name: img.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                media_type: img.media_type.clone(),
                data: base64::engine::general_purpose::STANDARD.encode(data),
            });
        }
        let req = SendReq { session_id: self.session_id.clone(), text: msg.text.clone(), images, delivery };
        self.call(SEND_PATH, &req).await
    }

    async fn answer(&self, request_id: &str, answer: &Answer) -> Result<(), AdapterError> {
        let req =
            AnswerReq { session_id: self.session_id.clone(), request_id: request_id.into(), answer: answer.clone() };
        self.call(ANSWER_PATH, &req).await
    }

    async fn interrupt(&self) -> Result<(), AdapterError> {
        self.call(INTERRUPT_PATH, &self.of()).await
    }

    async fn set(&self, change: &SessionChange) -> Result<(), AdapterError> {
        let req = SetReq {
            session_id: self.session_id.clone(),
            set_model: change.model.is_some(),
            model: change.model.clone().flatten(),
            permission_mode: change.permission_mode,
        };
        self.call(SET_PATH, &req).await
    }

    /// Its exit comes back with its events. When the remote instance can't be reached, it is closed here (the
    /// process there is left to its next open).
    async fn close(&self) {
        if let Err(e) = self.call(CLOSE_PATH, &self.of()).await {
            tracing::warn!(session = %self.session_id, instance = %self.instance.name, error = %e, "could not close a remote session");
            let route = st_remove(&self.st, &self.session_id);
            self.alive.store(false, Ordering::SeqCst);
            if let Some(r) = route {
                let _ = r.sink.send(AdapterOutput::Exited { code: None, stderr_tail: String::new(), expected: true });
            }
        }
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

fn st_remove(st: &AppState, session: &str) -> Option<Route> {
    st.exec.owner.routes().remove(session)
}

/// Makes sure a poller reads `link_id`'s events.
pub(super) fn ensure_poller(st: &AppState, link_id: &str) {
    if !st.exec.owner.pollers.lock().unwrap_or_else(|e| e.into_inner()).insert(link_id.to_string()) {
        return;
    }
    let (st, link) = (st.clone(), link_id.to_string());
    tokio::spawn(async move {
        poll_loop(&st, &link).await;
        st.exec.owner.pollers.lock().unwrap_or_else(|e| e.into_inner()).remove(&link);
        // A session opened as it stopped gets a poller of its own.
        if st.exec.owner.routes().values().any(|r| r.link_id == link) {
            ensure_poller(&st, &link);
        }
    });
}

/// After a restart: pollers for the instances that run sessions of this owner's, whose events waited for it.
pub async fn resume_pollers(st: &AppState) {
    match st.store.list_remote_instances(None).await {
        Ok(list) => {
            for i in list {
                if running_on(st, &i).await > 0 {
                    ensure_poller(st, &i.link_id);
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "could not list the remote instances"),
    }
}

/// How many sessions and active runs are on the instance, as the owner's store says.
async fn running_on(st: &AppState, i: &RemoteInstance) -> usize {
    let sessions = match st.store.list_sessions(&i.project_id).await {
        Ok(list) => list
            .iter()
            .filter(|s| {
                s.status == SessionStatus::Running && s.instance.as_ref().is_some_and(|r| r.id == i.instance_id)
            })
            .count(),
        Err(_) => 1,
    };
    sessions + super::runs::active_on(st, i).await.len()
}

/// Reads a joined instance's events into their conversations until nothing on it is followed any more.
async fn poll_loop(st: &AppState, link_id: &str) {
    loop {
        if st.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let Ok(Some(i)) = st.store.get_remote_instance(link_id).await else { return };
        let (boot, after) = st.store.events_cursor(link_id).await.unwrap_or((None, 0));
        let wait_ms = st.exec.owner.wait_ms.load(Ordering::SeqCst);
        let sent_at = now_ms();
        let req = PollReq { boot: boot.clone(), after, wait_ms };
        let timeout = Duration::from_millis(wait_ms) + Duration::from_secs(20);
        match call::<Batch>(st, &i, EVENTS_PATH, &req, timeout).await {
            Ok(batch) => apply(st, &i, batch, boot.as_deref(), after, sent_at).await,
            Err(e) => {
                tracing::debug!(instance = %i.name, error = %e.text(), "could not read a remote instance's events");
                tokio::time::sleep(Duration::from_millis(st.exec.owner.retry_ms.load(Ordering::SeqCst))).await;
            }
        }
        let routed = st.exec.owner.routes().values().any(|r| r.link_id == link_id);
        if !routed && running_on(st, &i).await == 0 {
            return;
        }
    }
}

async fn apply(st: &AppState, i: &RemoteInstance, batch: Batch, boot: Option<&str>, after: i64, sent_at: i64) {
    let start = if boot == Some(batch.boot.as_str()) { after } else { 0 };
    let mut last = start;
    for e in batch.events {
        if e.seq <= start {
            continue;
        }
        last = last.max(e.seq);
        route(st, i, e).await;
    }
    last = last.max(super::runs::apply(st, i, batch.runs, start).await);
    super::runs::reconcile(st, i, &batch.runs_alive, sent_at).await;
    if let Err(e) = st.store.set_events_cursor(&i.link_id, &batch.boot, last).await {
        tracing::warn!(instance = %i.name, error = %e, "could not record how far a remote instance's events were read");
    }
    // Processes the remote instance no longer runs, with no exit to say so (it restarted): they ended.
    let alive: HashSet<&str> = batch.alive.iter().map(String::as_str).collect();
    let gone: Vec<Route> = {
        let mut routes = st.exec.owner.routes();
        let ids: Vec<String> = routes
            .iter()
            .filter(|(id, r)| {
                r.link_id == i.link_id && r.opened_at > 0 && r.opened_at < sent_at && !alive.contains(id.as_str())
            })
            .map(|(id, _)| id.clone())
            .collect();
        ids.iter().filter_map(|id| routes.remove(id)).collect()
    };
    for r in gone {
        r.alive.store(false, Ordering::SeqCst);
        let why = format!("{} no longer runs it (it restarted?)", i.name);
        let _ = r.sink.send(AdapterOutput::Exited { code: None, stderr_tail: why, expected: false });
    }
}

/// Hands one event to its conversation. One for a session the owner has no process for (the owner restarted while it
/// ran there) attaches to it first.
async fn route(st: &AppState, i: &RemoteInstance, e: WireEvent) {
    let exited = matches!(e.out, WireOut::Exited { .. });
    for attempt in 0..2 {
        let delivered = {
            let mut routes = st.exec.owner.routes();
            match routes.get(&e.session_id) {
                Some(r) if r.proc.as_deref().is_none_or(|p| p == e.proc) => {
                    let _ = r.sink.send(e.out.clone().into());
                    if exited && r.proc.as_deref() == Some(e.proc.as_str()) {
                        r.alive.store(false, Ordering::SeqCst);
                        routes.remove(&e.session_id);
                    }
                    Some(true)
                }
                // Another process's (one that was replaced).
                Some(_) => Some(false),
                None => None,
            }
        };
        if delivered.is_some() || exited || attempt == 1 {
            return;
        }
        let Ok(Some(s)) = st.store.get_session(&e.session_id).await else { return };
        if s.status == SessionStatus::Finished || s.instance.as_ref().is_none_or(|r| r.id != i.instance_id) {
            return;
        }
        if let Err(err) = crate::conversations::attach(st, &s).await {
            tracing::warn!(session = %s.id, instance = %i.name, error = %err, "could not attach to a remote session");
            return;
        }
    }
}

/// Makes a worktree for `branch` on the remote instance `i` and records it here.
pub(crate) async fn make_worktree(
    st: &AppState,
    project: &Project,
    i: &RemoteInstance,
    branch: &str,
    base: &str,
    existing_branch: bool,
) -> Result<Worktree, ApiError> {
    let req = WorktreeReq { branch: branch.into(), base: base.into(), existing_branch };
    let made: WorktreeMade =
        call(st, i, WORKTREE_PATH, &req, WORKTREE_TIMEOUT).await.map_err(|e| e.into_api(&i.name))?;
    let wt = Worktree {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: project.id.clone(),
        branch: branch.to_string(),
        path: made.path,
        base_commit: made.base_commit,
        created_at: now_ms(),
        missing: false,
        instance: Some(pdt_core::placement::InstanceRef { id: i.instance_id.clone(), name: i.name.clone() }),
    };
    st.store.insert_worktree(&wt).await?;
    Ok(wt)
}

/// Removes a remote worktree there: why its branch stayed, when it was to go and didn't.
pub(crate) async fn remove_worktree(
    st: &AppState,
    i: &RemoteInstance,
    branch: &str,
    force: bool,
    delete_branch: bool,
) -> Result<Option<String>, ApiError> {
    let req = RemoveWorktreeReq { branch: branch.into(), force, delete_branch };
    let r: Removed = call(st, i, REMOVE_WORKTREE_PATH, &req, GIT_TIMEOUT).await.map_err(|e| e.into_api(&i.name))?;
    Ok(r.branch_error)
}

/// `attempt.has_changes` for a remote worktree.
pub(crate) async fn has_changes(
    st: &AppState,
    i: &RemoteInstance,
    branch: &str,
    base: &str,
    base_commit: &str,
) -> Result<bool, String> {
    let req =
        GitReq { branch: branch.into(), op: GitOp::HasChanges { base: base.into(), base_commit: base_commit.into() } };
    let v: Value = call(st, i, GIT_PATH, &req, GIT_TIMEOUT).await.map_err(|e| format!("{}: {}", i.name, e.text()))?;
    Ok(v["changes"].as_bool().unwrap_or(true))
}

/// A remote branch's head there (None when it has none).
pub(crate) async fn branch_head(st: &AppState, i: &RemoteInstance, branch: &str) -> Result<Option<String>, String> {
    let req = GitReq { branch: branch.into(), op: GitOp::Head };
    let v: Value = call(st, i, GIT_PATH, &req, GIT_TIMEOUT).await.map_err(|e| format!("{}: {}", i.name, e.text()))?;
    Ok(v["head"].as_str().map(String::from))
}

/// `attempt.update_base` for a remote worktree: the port it leaves by and its output.
pub(crate) async fn update_base(
    st: &AppState,
    i: &RemoteInstance,
    branch: &str,
    base: &str,
    remote_pr: bool,
) -> Result<(String, Value), String> {
    let req = GitReq { branch: branch.into(), op: GitOp::UpdateBase { base: base.into(), remote_pr } };
    let v: Value = call(st, i, GIT_PATH, &req, GIT_TIMEOUT).await.map_err(|e| format!("{}: {}", i.name, e.text()))?;
    Ok((v["port"].as_str().unwrap_or("skipped").to_string(), v["output"].clone()))
}

/// `/link/exec/mcp`: a remote instance passes on its agent's pdt-mcp call. The token's session must run there; it is
/// then answered as any session's.
pub(crate) async fn mcp_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<McpReply>, ApiError> {
    let i = link::from_remote(&st, &method, &uri, &headers, &body).await?;
    let req: McpReq = serde_json::from_slice(&body).map_err(|e| ApiError::invalid(e.to_string()))?;
    let bearer = format!("Bearer {}", req.token);
    let id = pdt_core::mcp::parse_token(&bearer).map(|(id, _)| id.to_string()).unwrap_or_default();
    let ours = matches!(
        st.store.get_session(&id).await?,
        Some(s) if s.project_id == i.project_id && s.instance.as_ref().is_some_and(|r| r.id == i.instance_id)
    );
    if !ours {
        return Err(ApiError::forbidden(codes::LINK_REFUSED, "that session doesn't run on this instance"));
    }
    let mut h = HeaderMap::new();
    h.insert(header::AUTHORIZATION, HeaderValue::from_str(&bearer).map_err(|e| ApiError::invalid(e.to_string()))?);
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(a) = req.accept.as_deref().and_then(|a| HeaderValue::from_str(a).ok()) {
        h.insert(header::ACCEPT, a);
    }
    let res = crate::mcp::post(State(st), h, Bytes::from(req.body)).await.into_response();
    let status = res.status().as_u16();
    let content_type = res.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(String::from);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.map_err(ApiError::internal)?;
    Ok(Json(McpReply { status, content_type, body: String::from_utf8_lossy(&bytes).into_owned() }))
}
