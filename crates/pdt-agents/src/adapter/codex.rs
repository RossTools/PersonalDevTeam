//! Codex through `codex app-server` (spec §3.2, plan 5b): one server per pdtd on a loopback WebSocket with a
//! capability token (the real TUI can attach to it with `--remote`), one client connection multiplexing every thread,
//! and a `CodexLive` per conversation. The frame mapping is pdt-core's `CodexMapper`.

use super::{
    AdapterError, AdapterOutput, AgentAdapter, AgentInfo, EventSink, LiveConversation, OpenSpec, Outgoing,
    SessionChange,
};
use futures_util::{SinkExt, StreamExt};
use pdt_core::conversation::codex::{CodexMapper, CodexOut, SideTurn, codex_policy, sandbox_policy};
use pdt_core::conversation::{
    AgentCapabilities, AgentEvent, AgentModel, Answer, Delivery, NoticeLevel, PermissionMode,
};
use pdt_core::session::AgentKind;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// How long a request to the server may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a new server gets to start listening.
const START_TIMEOUT: Duration = Duration::from_secs(30);
/// The env var the TUI reads the token from (`--remote-auth-token-env`).
pub const TOKEN_ENV: &str = "PDT_CODEX_TOKEN";

/// How to start the server.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexServerConfig {
    pub program: String,
    pub search_path: Option<String>,
    /// PDT's folder for the server: the token file and the server's working folder.
    pub dir: PathBuf,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
}

/// Where a running server listens, for attaching the TUI.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexEndpoint {
    pub url: String,
    pub token: String,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;
type SideRoutes = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<(String, Value)>>>>;

struct Route {
    mapper: Mutex<CodexMapper>,
    sink: EventSink,
}

struct Conn {
    tx: mpsc::UnboundedSender<String>,
    pending: Pending,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
    endpoint: CodexEndpoint,
    pid: Option<u32>,
    _child: Arc<tokio::sync::Mutex<tokio::process::Child>>,
}

impl Conn {
    fn write(&self, v: &Value) -> Result<(), AdapterError> {
        self.tx.send(v.to_string()).map_err(|_| AdapterError::NotRunning)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, AdapterError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        if let Err(e) = self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})) {
            self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(AdapterError::Refused(e)),
            Ok(Err(_)) => Err(AdapterError::NotRunning),
            Err(_) => {
                self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                Err(AdapterError::Timeout)
            }
        }
    }

    fn kill(&self) {
        let Some(pid) = self.pid else { return };
        #[cfg(unix)]
        if let Ok(pid) = i32::try_from(pid) {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1500));
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            });
        }
        #[cfg(windows)]
        std::thread::spawn(move || crate::pty::kill_tree(pid));
    }
}

/// PDT's own `codex app-server`, started on first use and again after it stops.
pub struct CodexServer {
    config: CodexServerConfig,
    conn: tokio::sync::Mutex<Option<Arc<Conn>>>,
    routes: Arc<Mutex<HashMap<String, Arc<Route>>>>,
    /// Hidden side threads (Make ticket's draft): their notifications go here, never to a conversation.
    side: SideRoutes,
    caps: Mutex<Option<AgentCapabilities>>,
    billing: Mutex<Option<String>>,
}

fn random_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn write_token(path: &std::path::Path, token: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        f.write_all(token.as_bytes())
    }
    #[cfg(not(unix))]
    std::fs::write(path, token)
}

/// The models `model/list` reports.
pub fn capabilities_from_models(v: &Value) -> AgentCapabilities {
    let models = v
        .get("data")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|m| !m.get("hidden").and_then(Value::as_bool).unwrap_or(false))
                .filter_map(|m| {
                    let value = m.get("id").or(m.get("model")).and_then(Value::as_str)?.to_string();
                    Some(AgentModel {
                        display_name: m.get("displayName").and_then(Value::as_str).unwrap_or(&value).to_string(),
                        description: m
                            .get("description")
                            .and_then(Value::as_str)
                            .map(String::from)
                            .filter(|d| !d.is_empty()),
                        value,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    AgentCapabilities {
        conversation: true,
        permission_modes: PermissionMode::ALL.to_vec(),
        models,
        commands: pdt_core::conversation::codex::commands(),
        terminal_commands: vec![],
        version: None,
    }
}

fn emit(route: &Route, outs: Vec<CodexOut>, raw: Option<&str>, conn: &Conn) {
    for o in outs {
        match o {
            CodexOut::Event { item_id, turn_id, event } => {
                let _ = route.sink.send(AdapterOutput::Event { item_id, turn_id, event, raw: raw.map(String::from) });
            }
            CodexOut::Delta { item_id, field, text } => {
                let _ = route.sink.send(AdapterOutput::Delta { item_id, field, text });
            }
            CodexOut::RespondError { id, message } => {
                let _ = conn.write(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": message}}));
            }
        }
    }
}

impl CodexServer {
    pub fn new(config: CodexServerConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            conn: Default::default(),
            routes: Default::default(),
            side: Default::default(),
            caps: Mutex::new(None),
            billing: Mutex::new(None),
        })
    }

    /// The running server's endpoint, if it runs.
    pub async fn endpoint(&self) -> Option<CodexEndpoint> {
        self.conn.lock().await.as_ref().filter(|c| c.alive.load(Ordering::SeqCst)).map(|c| c.endpoint.clone())
    }

    pub fn capabilities(&self) -> Option<AgentCapabilities> {
        self.caps.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Stops the server (pdtd is stopping).
    pub async fn shutdown(&self) {
        if let Some(c) = self.conn.lock().await.take() {
            c.alive.store(false, Ordering::SeqCst);
            c.kill();
        }
    }

    async fn ensure(self: &Arc<Self>) -> Result<Arc<Conn>, AdapterError> {
        let mut slot = self.conn.lock().await;
        if let Some(c) = slot.as_ref().filter(|c| c.alive.load(Ordering::SeqCst)) {
            return Ok(c.clone());
        }
        // The free port is found before the server binds it, so another program can take it first: try again.
        let mut last = None;
        for _ in 0..3 {
            match self.start().await {
                Ok(conn) => {
                    *slot = Some(conn.clone());
                    return Ok(conn);
                }
                Err(e @ AdapterError::Spawn(_)) => {
                    tracing::warn!(error = %e, "codex app-server did not start; trying again");
                    last = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or(AdapterError::NotRunning))
    }

    async fn start(self: &Arc<Self>) -> Result<Arc<Conn>, AdapterError> {
        let token = random_token();
        let token_file = self.config.dir.join("token");
        write_token(&token_file, &token).map_err(|e| AdapterError::Spawn(format!("writing the Codex token: {e}")))?;
        let port = free_port().map_err(|e| AdapterError::Spawn(format!("finding a free port: {e}")))?;
        let url = format!("ws://127.0.0.1:{port}");
        let args: Vec<String> = vec![
            "app-server".into(),
            "--listen".into(),
            url.clone(),
            "--ws-auth".into(),
            "capability-token".into(),
            "--ws-token-file".into(),
            token_file.to_string_lossy().into_owned(),
        ];
        let resolved = crate::launch::resolve_launch(&self.config.program, &args, self.config.search_path.as_deref())
            .map_err(|_| AdapterError::ProgramNotFound(self.config.program.clone()))?;
        let mut cmd = tokio::process::Command::new(&resolved.program);
        cmd.args(&resolved.args)
            .current_dir(&self.config.dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        for k in &self.config.env_remove {
            cmd.env_remove(k);
        }
        cmd.envs(self.config.env.iter().map(|(k, v)| (k, v)));
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().map_err(|e| AdapterError::Spawn(e.to_string()))?;
        let pid = child.id();
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        if let Some(mut err) = child.stderr.take() {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = [0u8; 4096];
                while let Ok(n) = err.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                    t.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if t.len() > 4096 {
                        let mut cut = t.len() - 4096;
                        while !t.is_char_boundary(cut) {
                            cut += 1;
                        }
                        t.drain(..cut);
                    }
                }
            });
        }
        let child = Arc::new(tokio::sync::Mutex::new(child));

        // Connect once it listens.
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        let ws = loop {
            let mut req = url.as_str().into_client_request().map_err(|e| AdapterError::Spawn(e.to_string()))?;
            req.headers_mut().insert(
                "Authorization",
                format!("Bearer {token}").parse().map_err(|_| AdapterError::Spawn("bad token".into()))?,
            );
            match tokio_tungstenite::connect_async(req).await {
                Ok((ws, _)) => break ws,
                Err(_) if tokio::time::Instant::now() < deadline => {
                    if let Ok(Some(status)) = child.lock().await.try_wait() {
                        let tail = stderr_tail.lock().unwrap_or_else(|e| e.into_inner()).trim().to_string();
                        return Err(AdapterError::Spawn(format!("codex app-server exited ({status}): {tail}")));
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                Err(e) => {
                    let _ = child.lock().await.start_kill();
                    return Err(AdapterError::Spawn(format!("could not connect to codex app-server: {e}")));
                }
            }
        };
        let (mut write, mut read) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                if write.send(Message::Text(m.into())).await.is_err() {
                    break;
                }
            }
            let _ = write.close().await;
        });
        let alive = Arc::new(AtomicBool::new(true));
        let conn = Arc::new(Conn {
            tx,
            pending: Default::default(),
            next_id: AtomicU64::new(1),
            alive: alive.clone(),
            endpoint: CodexEndpoint { url, token },
            pid,
            _child: child.clone(),
        });

        // Reader: responses to their requests, everything else to the thread it names.
        {
            let conn = conn.clone();
            let routes = self.routes.clone();
            let side = self.side.clone();
            tokio::spawn(async move {
                while let Some(Ok(msg)) = read.next().await {
                    let text = match msg {
                        Message::Text(t) => t.to_string(),
                        Message::Close(_) => break,
                        _ => continue,
                    };
                    let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                    let method = v.get("method").and_then(Value::as_str);
                    match (method, v.get("id")) {
                        (None, Some(id)) => {
                            let Some(id) = id.as_u64() else { continue };
                            let tx = conn.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                            if let Some(tx) = tx {
                                let r = match v.get("error") {
                                    Some(e) => {
                                        Err(e.get("message").and_then(Value::as_str).unwrap_or("error").to_string())
                                    }
                                    None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                                };
                                let _ = tx.send(r);
                            }
                        }
                        (Some(method), id) => {
                            let params = v.get("params").cloned().unwrap_or(Value::Null);
                            let thread = params.get("threadId").and_then(Value::as_str);
                            let side_tx =
                                thread.and_then(|t| side.lock().unwrap_or_else(|e| e.into_inner()).get(t).cloned());
                            if let Some(tx) = side_tx {
                                match id {
                                    // A side thread never gets to ask (no tools, approval `never`): refuse.
                                    Some(id) => {
                                        let _ = conn.write(&json!({"jsonrpc": "2.0", "id": id,
                                            "error": {"code": -32601, "message": "not allowed in a side request"}}));
                                    }
                                    None => {
                                        let _ = tx.send((method.to_string(), params));
                                    }
                                }
                                continue;
                            }
                            let targets: Vec<Arc<Route>> = {
                                let map = routes.lock().unwrap_or_else(|e| e.into_inner());
                                match thread {
                                    Some(t) => map.get(t).cloned().into_iter().collect(),
                                    // Global notifications (rate limits) concern every conversation.
                                    None if id.is_none() => map.values().cloned().collect(),
                                    None => vec![],
                                }
                            };
                            if let Some(id) = id {
                                match targets.first() {
                                    Some(r) => {
                                        let outs = r
                                            .mapper
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner())
                                            .on_server_request(id.clone(), method, &params);
                                        emit(r, outs, Some(&text), &conn);
                                    }
                                    None => {
                                        let _ = conn.write(&json!({"jsonrpc": "2.0", "id": id,
                                            "error": {"code": -32601, "message": "no PDT conversation for this thread"}}));
                                    }
                                }
                            } else {
                                for r in targets {
                                    let outs = r
                                        .mapper
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .on_notification(method, &params);
                                    emit(&r, outs, Some(&text), &conn);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                // The server or the connection went away: every conversation on it stops.
                alive.store(false, Ordering::SeqCst);
                conn.pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
                let code = {
                    let mut c = child.lock().await;
                    match tokio::time::timeout(Duration::from_secs(2), c.wait()).await {
                        Ok(Ok(s)) => s.code(),
                        _ => {
                            let _ = c.start_kill();
                            None
                        }
                    }
                };
                let tail = stderr_tail.lock().unwrap_or_else(|e| e.into_inner()).trim().to_string();
                side.lock().unwrap_or_else(|e| e.into_inner()).clear();
                let all: Vec<Arc<Route>> =
                    routes.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, r)| r).collect();
                for r in all {
                    let outs = r.mapper.lock().unwrap_or_else(|e| e.into_inner()).on_exit("Codex stopped");
                    emit(&r, outs, None, &conn);
                    let _ = r.sink.send(AdapterOutput::Exited { code, stderr_tail: tail.clone(), expected: false });
                }
            });
        }

        conn.request(
            "initialize",
            json!({"clientInfo": {"name": pdt_core::brand::SLUG, "version": pdt_core::BUILD_VERSION},
                   "capabilities": {"experimentalApi": true}}),
        )
        .await?;
        conn.write(&json!({"jsonrpc": "2.0", "method": "initialized"}))?;
        if let Ok(v) = conn.request("model/list", json!({})).await {
            *self.caps.lock().unwrap_or_else(|e| e.into_inner()) = Some(capabilities_from_models(&v));
        }
        // Only the account's type is read (the composer says who pays); never its tokens.
        if let Ok(v) = conn.request("account/read", json!({})).await {
            *self.billing.lock().unwrap_or_else(|e| e.into_inner()) = pdt_core::conversation::codex::billing_label(&v);
        }
        Ok(conn)
    }
}

#[derive(Debug, Clone, Default)]
struct Settings {
    model: Option<String>,
    mode: PermissionMode,
}

pub struct CodexAdapter {
    pub server: Arc<CodexServer>,
}

pub struct CodexLive {
    server: Arc<CodexServer>,
    conn: Arc<Conn>,
    thread: String,
    route: Arc<Route>,
    settings: Mutex<Settings>,
    open: AtomicBool,
}

fn input(msg: &Outgoing) -> Value {
    let paths: Vec<String> = msg.images.iter().map(|i| i.path.to_string_lossy().into_owned()).collect();
    pdt_core::conversation::codex::user_input(&msg.text, &paths)
}

#[async_trait::async_trait]
impl AgentAdapter for CodexAdapter {
    fn agent(&self) -> AgentKind {
        AgentKind::Codex
    }

    async fn open(&self, spec: OpenSpec, sink: EventSink) -> Result<Box<dyn LiveConversation>, AdapterError> {
        let conn = self.server.ensure().await?;
        if let Some(caps) = self.server.capabilities() {
            let _ = sink.send(AdapterOutput::Capabilities(caps));
        }
        let billing = self.server.billing.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if billing.is_some() {
            let _ = sink.send(AdapterOutput::Info(AgentInfo { billing, ..AgentInfo::default() }));
        }
        let (approval, sandbox) = codex_policy(spec.permission_mode);
        let cwd = spec.cwd.to_string_lossy().into_owned();
        let route = Arc::new(Route { mapper: Mutex::new(CodexMapper::new()), sink: sink.clone() });
        let mut resumed = None;
        if let Some(thread) = spec.external_id.clone().filter(|_| spec.resume) {
            // Listen before resuming, so nothing the thread does meanwhile is missed.
            self.server.routes.lock().unwrap_or_else(|e| e.into_inner()).insert(thread.clone(), route.clone());
            let params = json!({"threadId": thread, "excludeTurns": true, "cwd": cwd, "model": spec.model,
                                "approvalPolicy": approval, "sandbox": sandbox});
            match conn.request("thread/resume", params).await {
                Ok(_) => resumed = Some(thread),
                Err(e) => {
                    self.server.routes.lock().unwrap_or_else(|e| e.into_inner()).remove(&thread);
                    tracing::info!(thread = %thread, error = %e, "could not resume the Codex thread; starting a new one");
                    let text = "Codex could not resume this conversation's thread, so it starts a new one.".to_string();
                    let _ = sink.send(AdapterOutput::Event {
                        item_id: None,
                        turn_id: None,
                        event: AgentEvent::Notice { level: NoticeLevel::Warning, text, until: None },
                        raw: None,
                    });
                }
            }
        }
        let thread = match resumed {
            Some(t) => {
                // Turns taken while PDT was not listening (e.g. in the TUI) are imported from the history.
                let mut turns = Vec::new();
                let mut cursor: Option<String> = None;
                loop {
                    let params = json!({"threadId": t, "itemsView": "full", "sortDirection": "asc", "cursor": cursor});
                    let Ok(page) = conn.request("thread/turns/list", params).await else { break };
                    turns.extend(page.get("data").and_then(Value::as_array).cloned().unwrap_or_default());
                    cursor = page.get("nextCursor").and_then(Value::as_str).map(String::from);
                    if cursor.is_none() {
                        break;
                    }
                }
                if !turns.is_empty() {
                    let _ = sink.send(AdapterOutput::History(turns));
                }
                t
            }
            None => {
                let params = json!({"cwd": cwd, "model": spec.model, "approvalPolicy": approval, "sandbox": sandbox});
                let r = conn.request("thread/start", params).await?;
                let id = r
                    .pointer("/thread/id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| AdapterError::Refused("thread/start returned no thread".into()))?
                    .to_string();
                self.server.routes.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), route.clone());
                let _ = sink.send(AdapterOutput::ExternalId(id.clone()));
                id
            }
        };
        Ok(Box::new(CodexLive {
            server: self.server.clone(),
            conn,
            thread,
            route,
            settings: Mutex::new(Settings { model: spec.model, mode: spec.permission_mode }),
            open: AtomicBool::new(true),
        }))
    }

    async fn draft(&self, spec: OpenSpec, prompt: &str, schema: &Value) -> Result<Value, AdapterError> {
        let thread = spec.external_id.clone().ok_or_else(|| AdapterError::Refused("no Codex thread yet".into()))?;
        let conn = self.server.ensure().await?;
        let cwd = spec.cwd.to_string_lossy().into_owned();
        // An ephemeral fork: never written to disk, read-only, never asks.
        let params = json!({"threadId": thread, "ephemeral": true, "excludeTurns": true, "cwd": cwd, "model": spec.model,
                            "approvalPolicy": "never", "sandbox": "read-only"});
        let r = conn.request("thread/fork", params).await?;
        let fork = r
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| AdapterError::Refused("thread/fork returned no thread".into()))?
            .to_string();
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.server.side.lock().unwrap_or_else(|e| e.into_inner()).insert(fork.clone(), tx);
        // Also when the request is dropped midway (the browser gave up): the fork is let go.
        let _guard = SideGuard { side: self.server.side.clone(), conn: conn.clone(), fork: fork.clone() };
        let mut params = json!({"threadId": fork, "input": pdt_core::conversation::codex::user_input(prompt, &[]),
                                "approvalPolicy": "never", "sandboxPolicy": sandbox_policy("read-only"),
                                "outputSchema": schema});
        if let Some(m) = &spec.model {
            params["model"] = json!(m);
        }
        self.side_turn(&conn, &fork, &mut rx, params).await
    }
}

/// Stops routing a side thread and unsubscribes from it when its request ends, however it ends.
struct SideGuard {
    side: SideRoutes,
    conn: Arc<Conn>,
    fork: String,
}

impl Drop for SideGuard {
    fn drop(&mut self) {
        self.side.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.fork);
        let id = self.conn.next_id.fetch_add(1, Ordering::SeqCst);
        let _ = self.conn.write(&json!({"jsonrpc": "2.0", "id": id, "method": "thread/unsubscribe",
                                        "params": {"threadId": self.fork}}));
    }
}

impl CodexAdapter {
    async fn side_turn(
        &self,
        conn: &Conn,
        thread: &str,
        rx: &mut mpsc::UnboundedReceiver<(String, Value)>,
        params: Value,
    ) -> Result<Value, AdapterError> {
        let started = conn.request("turn/start", params).await?;
        let turn_id = started.pointer("/turn/id").and_then(Value::as_str).map(String::from);
        let mut turn = SideTurn::default();
        let wait = async {
            while let Some((method, p)) = rx.recv().await {
                if let Some(r) = turn.on_notification(&method, &p) {
                    return r.map(Value::String).map_err(AdapterError::Refused);
                }
            }
            Err(AdapterError::NotRunning)
        };
        match tokio::time::timeout(super::DRAFT_TIMEOUT, wait).await {
            Ok(r) => r,
            Err(_) => {
                if let Some(t) = turn_id {
                    let _ = conn.request("turn/interrupt", json!({"threadId": thread, "turnId": t})).await;
                }
                Err(AdapterError::Timeout)
            }
        }
    }
}

impl CodexLive {
    fn mapper(&self) -> std::sync::MutexGuard<'_, CodexMapper> {
        self.route.mapper.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn start_turn(&self, msg: &Outgoing) -> Result<(), AdapterError> {
        let s = self.settings.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (approval, sandbox) = codex_policy(s.mode);
        let mut params = json!({"threadId": self.thread, "input": input(msg), "approvalPolicy": approval,
                                "sandboxPolicy": sandbox_policy(sandbox)});
        if let Some(m) = s.model {
            params["model"] = json!(m);
        }
        self.mapper().note_sent();
        if let Err(e) = self.conn.request("turn/start", params).await {
            let mut m = self.mapper();
            m.note_unsent();
            return Err(e);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl LiveConversation for CodexLive {
    async fn send(&self, msg: &Outgoing, delivery: Delivery) -> Result<(), AdapterError> {
        // `/compact` and `/review` go to their RPCs; the conversation shows the command as sent.
        if let Some((method, mut params)) =
            pdt_core::conversation::codex::slash_command(&msg.text).filter(|_| msg.images.is_empty())
        {
            params["threadId"] = json!(self.thread);
            // Shown first: the command's turn may start before the request returns.
            let _ = self.route.sink.send(AdapterOutput::Event {
                item_id: Some(format!("u:cmd-{}", uuid::Uuid::now_v7())),
                turn_id: None,
                event: AgentEvent::UserMessage {
                    text: msg.text.trim().to_string(),
                    images: vec![],
                    source: pdt_core::conversation::MessageSource::Pdt,
                },
                raw: None,
            });
            self.conn.request(method, params).await?;
            return Ok(());
        }
        let running = self.mapper().turn().map(String::from);
        match (running, delivery) {
            (Some(turn), Delivery::Next) => {
                self.mapper().note_sent();
                let params = json!({"threadId": self.thread, "expectedTurnId": turn, "input": input(msg)});
                match self.conn.request("turn/steer", params).await {
                    Ok(_) => Ok(()),
                    // The turn ended meanwhile: run it as a new turn.
                    Err(AdapterError::Refused(_)) => {
                        self.mapper().note_unsent();
                        self.start_turn(msg).await
                    }
                    Err(e) => Err(e),
                }
            }
            (Some(turn), Delivery::Now) => {
                self.mapper().note_interrupt();
                let _ = self.conn.request("turn/interrupt", json!({"threadId": self.thread, "turnId": turn})).await;
                self.start_turn(msg).await
            }
            (None, _) => self.start_turn(msg).await,
        }
    }

    async fn answer(&self, request_id: &str, answer: &Answer) -> Result<(), AdapterError> {
        let a = self.mapper().answer(request_id, answer).map_err(|e| match e {
            pdt_core::conversation::claude::AnswerError::Closed => AdapterError::RequestClosed,
            pdt_core::conversation::claude::AnswerError::WrongAnswer => AdapterError::WrongAnswer,
        })?;
        self.conn.write(&json!({"jsonrpc": "2.0", "id": a.rpc_id, "result": a.result}))?;
        emit(&self.route, a.events, None, &self.conn);
        if let Some(note) = a.follow_up {
            // Codex's decline has no message: the note goes to it as the next message.
            self.send(&Outgoing { text: note, images: vec![] }, Delivery::Next).await?;
        }
        Ok(())
    }

    async fn interrupt(&self) -> Result<(), AdapterError> {
        let Some(turn) = self.mapper().turn().map(String::from) else { return Ok(()) };
        self.mapper().note_interrupt();
        self.conn.request("turn/interrupt", json!({"threadId": self.thread, "turnId": turn})).await.map(|_| ())
    }

    async fn set(&self, change: &SessionChange) -> Result<(), AdapterError> {
        // Applied as overrides on the next turn/start.
        let mut s = self.settings.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = &change.model {
            s.model = m.clone();
        }
        if let Some(mode) = change.permission_mode {
            s.mode = mode;
        }
        Ok(())
    }

    async fn close(&self) {
        if !self.open.swap(false, Ordering::SeqCst) {
            return;
        }
        let _ = self.conn.request("thread/unsubscribe", json!({"threadId": self.thread})).await;
        self.server.routes.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.thread);
        let outs = self.mapper().on_exit("Stopped");
        emit(&self.route, outs, None, &self.conn);
        let _ = self.route.sink.send(AdapterOutput::Exited { code: None, stderr_tail: String::new(), expected: true });
    }

    fn is_alive(&self) -> bool {
        self.open.load(Ordering::SeqCst)
            && self.conn.alive.load(Ordering::SeqCst)
            && self.server.routes.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&self.thread)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_skip_hidden_ones() {
        let v = json!({"data": [{"id": "gpt-5", "displayName": "GPT-5", "description": "", "hidden": false},
                                {"id": "secret", "hidden": true}]});
        let c = capabilities_from_models(&v);
        assert_eq!(c.models.len(), 1);
        assert_eq!((c.models[0].value.as_str(), c.models[0].description.as_ref()), ("gpt-5", None));
    }

    #[test]
    fn the_token_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("codex").join("token");
        write_token(&f, "abc").unwrap();
        write_token(&f, "def").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "def");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(random_token().len(), 64);
    }
}
