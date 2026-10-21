//! Devin through `devin acp`, an Agent Client Protocol server on stdio (notes: docs/superpowers/notes/devin-cli.md):
//! one process per live conversation, JSON-RPC 2.0 one message per line. The frame mapping is pdt-core's
//! `DevinMapper`; this module owns the process, the request ids and the replies.

use super::{
    AdapterError, AdapterOutput, AgentAdapter, AgentInfo, EventSink, LiveConversation, OpenSpec, Outgoing,
    SessionChange,
};
use base64::Engine as _;
use pdt_core::conversation::devin::{
    DEVIN_MODES, DevinMapper, DevinOut, SESSION_NOT_FOUND, devin_mode, from_devin_mode, prompt_blocks,
};
use pdt_core::conversation::{AgentCapabilities, AgentEvent, Answer, Delivery, NoticeLevel};
use pdt_core::session::AgentKind;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};

/// How much of the agent's error output is kept to explain a crash.
const STDERR_TAIL: usize = 4096;
/// How long `initialize` and opening the session may take.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a settings request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a closing process gets after its stdin closes before it is killed.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
/// How long "Send now" waits for the cancelled turn to end before sending.
const CANCEL_WAIT: Duration = Duration::from_secs(10);

/// The `devin` arguments: `--config <file>` (the user's config plus PDT's SessionStart hook, which hands Devin PDT's
/// instructions) when there is one, then `acp`.
pub fn devin_acp_args(spec: &OpenSpec) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(file) = &spec.config_file {
        a.push("--config".to_string());
        a.push(file.to_string_lossy().into_owned());
    }
    a.push("acp".into());
    a
}

/// `initialize`'s parameters: no file system or terminal for Devin to borrow (it uses its own tools), and form
/// elicitation, which carries its `ask_user_question`.
pub fn initialize_params() -> Value {
    json!({
        "protocolVersion": 1,
        "clientCapabilities": {
            "fs": {"readTextFile": false, "writeTextFile": false},
            "terminal": false,
            "elicitation": {"form": {}},
        },
        "clientInfo": {"name": pdt_core::brand::SLUG, "title": pdt_core::brand::PRODUCT_NAME,
                       "version": env!("CARGO_PKG_VERSION")},
    })
}

#[derive(Debug, Clone)]
struct RpcError {
    code: i64,
    message: String,
    retryable: bool,
}

impl RpcError {
    fn of(v: &Value) -> Self {
        RpcError {
            code: v.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: v.get("message").and_then(Value::as_str).unwrap_or("error").to_string(),
            retryable: v
                .get("data")
                .and_then(|d| d.get("cognition.ai/retryable"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
}

type Reply = Result<Value, RpcError>;

pub struct DevinAdapter;

struct Shared {
    mapper: Mutex<DevinMapper>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
    next_id: AtomicU64,
    stdin: Mutex<Option<mpsc::UnboundedSender<String>>>,
    alive: AtomicBool,
    closing: AtomicBool,
    pid: Option<u32>,
    sink: EventSink,
    /// Devin's id for the session (empty until `session/new` or `session/load` answers).
    session: Mutex<String>,
    caps: Mutex<AgentCapabilities>,
    /// Devin's mode id as it last reported it.
    mode: Mutex<Option<String>>,
    /// Whether no turn is running (for "Send now").
    idle: watch::Sender<bool>,
}

impl Shared {
    fn mapper(&self) -> std::sync::MutexGuard<'_, DevinMapper> {
        self.mapper.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn session(&self) -> String {
        self.session.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn write(&self, v: &Value) -> Result<(), AdapterError> {
        let tx = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        let tx = tx.as_ref().ok_or(AdapterError::NotRunning)?;
        tx.send(v.to_string()).map_err(|_| AdapterError::NotRunning)
    }

    /// Sends a request; the reply arrives on the returned channel.
    fn call(&self, method: &str, params: Value) -> Result<oneshot::Receiver<Reply>, AdapterError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        if let Err(e) = self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})) {
            self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            return Err(e);
        }
        Ok(rx)
    }

    /// Sends a request and waits for its reply (`Ok(Err)`: Devin answered with an error).
    async fn request(&self, method: &str, params: Value, limit: Duration) -> Result<Reply, AdapterError> {
        let rx = self.call(method, params)?;
        match tokio::time::timeout(limit, rx).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => Err(AdapterError::NotRunning),
            Err(_) => Err(AdapterError::Timeout),
        }
    }

    fn publish_caps(&self) {
        let caps = self.caps.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let _ = self.sink.send(AdapterOutput::Capabilities(caps));
    }

    fn emit(&self, outs: Vec<DevinOut>, raw: Option<&str>) {
        for o in outs {
            match o {
                DevinOut::Event { item_id, turn_id, event } => {
                    let _ =
                        self.sink.send(AdapterOutput::Event { item_id, turn_id, event, raw: raw.map(String::from) });
                }
                DevinOut::Delta { item_id, field, text } => {
                    let _ = self.sink.send(AdapterOutput::Delta { item_id, field, text });
                }
                DevinOut::Settings { mode, model } => {
                    if mode.is_some() {
                        *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = mode.clone();
                    }
                    let _ = self.sink.send(AdapterOutput::Info(AgentInfo {
                        model,
                        permission_mode: mode.as_deref().and_then(from_devin_mode),
                        ..Default::default()
                    }));
                }
                DevinOut::Models(models) => {
                    self.caps.lock().unwrap_or_else(|e| e.into_inner()).models = models;
                    self.publish_caps();
                }
                DevinOut::Commands(commands) => {
                    self.caps.lock().unwrap_or_else(|e| e.into_inner()).commands = commands;
                    self.publish_caps();
                }
                DevinOut::RespondError { id, message } => {
                    let _ =
                        self.write(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": message}}));
                }
            }
        }
        let idle = self.mapper().turn().is_none();
        self.idle.send_if_modified(|v| std::mem::replace(v, idle) != idle);
    }

    /// One line from Devin's stdout.
    fn on_line(&self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return };
        match (v.get("method").and_then(Value::as_str), v.get("id")) {
            (Some(method), Some(id)) => {
                let outs = self.mapper().on_request(id.clone(), method, v.get("params").unwrap_or(&Value::Null));
                self.emit(outs, Some(line));
            }
            (Some("session/update"), None) => {
                let p = v.get("params").cloned().unwrap_or_default();
                let session = self.session();
                let for_us = session.is_empty() || p.get("sessionId").and_then(Value::as_str) == Some(session.as_str());
                if for_us && let Some(u) = p.get("update") {
                    let outs = self.mapper().on_update(u);
                    self.emit(outs, Some(line));
                }
            }
            (Some(_), None) => {}
            (None, Some(id)) => {
                let Some(id) = id.as_u64() else { return };
                if let Some(tx) = self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
                    let reply = match v.get("error") {
                        Some(e) => Err(RpcError::of(e)),
                        None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = tx.send(reply);
                }
            }
            (None, None) => {}
        }
    }
}

pub struct DevinLive {
    shared: Arc<Shared>,
    exited: watch::Receiver<bool>,
}

#[async_trait::async_trait]
impl AgentAdapter for DevinAdapter {
    fn agent(&self) -> AgentKind {
        AgentKind::Devin
    }

    async fn open(&self, spec: OpenSpec, sink: EventSink) -> Result<Box<dyn LiveConversation>, AdapterError> {
        let live = DevinLive::spawn(&spec, sink).await?;
        if let Err(e) = live.start_session(&spec).await {
            live.close().await;
            return Err(e);
        }
        Ok(Box::new(live))
    }

    async fn draft(&self, _spec: OpenSpec, _prompt: &str, _schema: &Value) -> Result<Value, AdapterError> {
        // ACP has no fork: a side request would land in the conversation itself.
        Err(AdapterError::Refused("Make ticket is not available for Devin yet".into()))
    }
}

impl DevinLive {
    async fn spawn(spec: &OpenSpec, sink: EventSink) -> Result<Self, AdapterError> {
        let args = devin_acp_args(spec);
        let resolved = crate::launch::resolve_launch(&spec.program, &args, spec.search_path.as_deref())
            .map_err(|_| AdapterError::ProgramNotFound(spec.program.clone()))?;
        let mut cmd = tokio::process::Command::new(&resolved.program);
        cmd.args(&resolved.args)
            .current_dir(&spec.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        for k in &spec.env_remove {
            cmd.env_remove(k);
        }
        cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().map_err(|e| AdapterError::Spawn(e.to_string()))?;
        let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(AdapterError::Spawn("the agent's pipes were not available".into()));
        };
        let canonical = std::fs::canonicalize(&spec.cwd).ok().map(|p| p.to_string_lossy().into_owned());
        let prefix = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let shared = Arc::new(Shared {
            mapper: Mutex::new(DevinMapper::new(spec.cwd.to_string_lossy(), canonical, prefix)),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            stdin: Mutex::new(Some(tx)),
            alive: AtomicBool::new(true),
            closing: AtomicBool::new(false),
            pid: child.id(),
            sink,
            session: Mutex::new(String::new()),
            caps: Mutex::new(AgentCapabilities {
                conversation: true,
                permission_modes: DEVIN_MODES.to_vec(),
                ..Default::default()
            }),
            mode: Mutex::new(None),
            idle: watch::channel(true).0,
        });

        // Writer: one message per line; stdin closes when every sender is gone.
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err()
                    || stdin.write_all(b"\n").await.is_err()
                    || stdin.flush().await.is_err()
                {
                    break;
                }
            }
        });

        let tail = Arc::new(Mutex::new(String::new()));
        let err_task = {
            let tail = tail.clone();
            tokio::spawn(async move {
                let mut stderr = stderr;
                let mut buf = [0u8; 4096];
                while let Ok(n) = stderr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                    t.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if t.len() > STDERR_TAIL {
                        let mut cut = t.len() - STDERR_TAIL;
                        while !t.is_char_boundary(cut) {
                            cut += 1;
                        }
                        t.drain(..cut);
                    }
                }
            })
        };

        let (exit_tx, exit_rx) = watch::channel(false);
        {
            let shared = shared.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    shared.on_line(&line);
                }
                let code = child.wait().await.ok().and_then(|s| s.code());
                let _ = err_task.await;
                shared.alive.store(false, Ordering::SeqCst);
                shared.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
                let expected = shared.closing.load(Ordering::SeqCst);
                let reason = if expected { "Stopped" } else { "Devin stopped" };
                let outs = shared.mapper().on_exit(reason);
                shared.emit(outs, None);
                // After the turn has ended: the prompts still waiting then map to nothing.
                shared.pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
                let stderr_tail = tail.lock().unwrap_or_else(|e| e.into_inner()).trim().to_string();
                let _ = shared.sink.send(AdapterOutput::Exited { code, stderr_tail, expected });
                let _ = exit_tx.send(true);
            });
        }
        Ok(DevinLive { shared, exited: exit_rx })
    }

    /// `initialize`, then the saved session (`session/load`, whose replay of history PDT has logged already) or a new
    /// one, then the session's mode and model.
    async fn start_session(&self, spec: &OpenSpec) -> Result<(), AdapterError> {
        let sh = &self.shared;
        let refused = |e: RpcError| AdapterError::Refused(e.message);
        sh.request("initialize", initialize_params(), START_TIMEOUT).await?.map_err(refused)?;
        let cwd = spec.cwd.to_string_lossy().into_owned();
        let mcp = crate::mcp::acp_servers(spec.mcp.as_ref());
        let mut loaded = false;
        if let Some(id) = spec.external_id.clone().filter(|_| spec.resume) {
            *sh.session.lock().unwrap_or_else(|e| e.into_inner()) = id.clone();
            sh.mapper().begin_load(spec.import_after);
            let r = sh
                .request("session/load", json!({"sessionId": id, "cwd": cwd, "mcpServers": mcp}), START_TIMEOUT)
                .await;
            let outs = sh.mapper().end_load();
            sh.emit(outs, None);
            match r? {
                Ok(_) => loaded = true,
                // Devin saves a session only at its first prompt.
                Err(e) if e.code == SESSION_NOT_FOUND => {
                    sh.session.lock().unwrap_or_else(|e| e.into_inner()).clear();
                    let text = "Devin had not saved this conversation (it saves one at the first message), so this is \
                                a new one."
                        .to_string();
                    let _ = sh.sink.send(AdapterOutput::Event {
                        item_id: None,
                        turn_id: None,
                        event: AgentEvent::Notice { level: NoticeLevel::Info, text, until: None },
                        raw: None,
                    });
                }
                Err(e) => return Err(refused(e)),
            }
        }
        if !loaded {
            let v = sh
                .request("session/new", json!({"cwd": cwd, "mcpServers": mcp}), START_TIMEOUT)
                .await?
                .map_err(refused)?;
            let id = v
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| AdapterError::Refused("Devin did not name the new session".into()))?
                .to_string();
            *sh.session.lock().unwrap_or_else(|e| e.into_inner()) = id.clone();
            let _ = sh.sink.send(AdapterOutput::ExternalId(id));
        }
        let want = devin_mode(spec.permission_mode);
        let current = sh.mode.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if current.as_deref() != Some(want) {
            self.set_mode(want).await?;
        }
        if let Some(m) = &spec.model {
            self.set_model(m).await?;
        }
        sh.publish_caps();
        Ok(())
    }

    async fn set_mode(&self, mode: &str) -> Result<(), AdapterError> {
        let params = json!({"sessionId": self.shared.session(), "modeId": mode});
        self.shared
            .request("session/set_mode", params, REQUEST_TIMEOUT)
            .await?
            .map_err(|e| AdapterError::Refused(e.message))?;
        Ok(())
    }

    async fn set_model(&self, model: &str) -> Result<(), AdapterError> {
        let params = json!({"sessionId": self.shared.session(), "configId": "model", "value": model});
        self.shared
            .request("session/set_config_option", params, REQUEST_TIMEOUT)
            .await?
            .map_err(|e| AdapterError::Refused(e.message))?;
        Ok(())
    }

    fn kill(&self) {
        let Some(pid) = self.shared.pid else { return };
        #[cfg(unix)]
        if let Ok(pid) = i32::try_from(pid) {
            // The child leads its own process group: end its tools too.
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

#[async_trait::async_trait]
impl LiveConversation for DevinLive {
    async fn send(&self, msg: &Outgoing, delivery: Delivery) -> Result<(), AdapterError> {
        let sh = &self.shared;
        if !self.is_alive() {
            return Err(AdapterError::NotRunning);
        }
        if delivery == Delivery::Now && sh.mapper().turn().is_some() {
            // A prompt sent mid-turn joins the turn: "now" ends it first.
            self.interrupt().await?;
            let mut idle = sh.idle.subscribe();
            let _ = tokio::time::timeout(CANCEL_WAIT, idle.wait_for(|i| *i)).await;
        }
        let mut images = Vec::new();
        for img in &msg.images {
            let bytes = tokio::fs::read(&img.path)
                .await
                .map_err(|e| AdapterError::Spawn(format!("reading an attached image: {e}")))?;
            images.push((img.media_type.clone(), base64::engine::general_purpose::STANDARD.encode(bytes)));
        }
        let params = json!({"sessionId": sh.session(), "prompt": prompt_blocks(&msg.text, &images)});
        let outs = sh.mapper().begin_prompt(&msg.text);
        sh.emit(outs, None);
        let rx = match sh.call("session/prompt", params) {
            Ok(rx) => rx,
            Err(e) => {
                let outs = sh.mapper().end_prompt(Err(("Devin is not running".into(), true)));
                sh.emit(outs, None);
                return Err(e);
            }
        };
        // A turn takes as long as it takes: its answer ends it.
        let shared = sh.clone();
        tokio::spawn(async move {
            let outs = match rx.await {
                Ok(Ok(v)) => shared.mapper().end_prompt(Ok(&v)),
                Ok(Err(e)) => shared.mapper().end_prompt(Err((e.message, e.retryable))),
                // The process ended: its exit already ended the turn.
                Err(_) => return,
            };
            shared.emit(outs, None);
        });
        Ok(())
    }

    async fn answer(&self, request_id: &str, answer: &Answer) -> Result<(), AdapterError> {
        let a = self.shared.mapper().answer(request_id, answer).map_err(|e| match e {
            pdt_core::conversation::claude::AnswerError::Closed => AdapterError::RequestClosed,
            pdt_core::conversation::claude::AnswerError::WrongAnswer => AdapterError::WrongAnswer,
        })?;
        self.shared.write(&json!({"jsonrpc": "2.0", "id": a.rpc_id, "result": a.result}))?;
        self.shared.emit(a.events, None);
        if let Some(note) = a.follow_up {
            // Devin's reject has no message: the note goes to it as the next message (taken into the running turn).
            self.send(&Outgoing { text: note, images: vec![] }, Delivery::Next).await?;
        }
        Ok(())
    }

    async fn interrupt(&self) -> Result<(), AdapterError> {
        let sh = &self.shared;
        let (replies, outs) = {
            let mut m = sh.mapper();
            if m.turn().is_none() {
                return Ok(());
            }
            m.note_interrupt();
            m.cancel_requests()
        };
        for (id, result) in replies {
            let _ = sh.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
        }
        sh.emit(outs, None);
        sh.write(&json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": sh.session()}}))
    }

    async fn set(&self, change: &SessionChange) -> Result<(), AdapterError> {
        if let Some(model) = &change.model {
            match model {
                Some(m) => self.set_model(m).await?,
                // Devin has no "default" to pick: the next process starts on the saved session's model.
                None => return Err(AdapterError::Refused("Devin has no default model to switch back to".into())),
            }
        }
        if let Some(mode) = change.permission_mode {
            self.set_mode(devin_mode(mode)).await?;
        }
        Ok(())
    }

    async fn close(&self) {
        self.shared.closing.store(true, Ordering::SeqCst);
        self.shared.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
        let mut exited = self.exited.clone();
        if tokio::time::timeout(CLOSE_GRACE, exited.wait_for(|e| *e)).await.is_err() {
            self.kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), exited.wait_for(|e| *e)).await;
        }
    }

    fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }
}

impl Drop for DevinLive {
    fn drop(&mut self) {
        if self.is_alive() {
            self.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::conversation::PermissionMode;

    #[test]
    fn args_name_the_config_before_acp() {
        let mut spec = OpenSpec {
            session_id: "sid".into(),
            external_id: None,
            resume: false,
            cwd: std::env::temp_dir(),
            program: "devin".into(),
            search_path: None,
            model: None,
            permission_mode: PermissionMode::AcceptEdits,
            env: vec![],
            env_remove: vec![],
            instructions: None,
            config_file: None,
            import_after: None,
            mcp: None,
        };
        assert_eq!(devin_acp_args(&spec), vec!["acp"]);
        let file = std::env::temp_dir().join("devin-config.json");
        spec.config_file = Some(file.clone());
        assert_eq!(
            devin_acp_args(&spec),
            vec!["--config".to_string(), file.to_string_lossy().into_owned(), "acp".into()]
        );
        let p = initialize_params();
        assert_eq!(p["clientCapabilities"]["fs"]["writeTextFile"], false);
        assert!(p["clientCapabilities"]["elicitation"]["form"].is_object());
    }
}
