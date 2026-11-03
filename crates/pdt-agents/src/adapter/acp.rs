//! Agents that speak the Agent Client Protocol on stdio: one process per live conversation, JSON-RPC 2.0 one message
//! per line. The frame mapping is pdt-core's `AcpMapper`; this module owns the process, the request ids and the
//! replies. What one agent does its own way (its program and arguments, its modes, how it resumes) comes from its
//! [`AcpProfile`] (Devin's: `super::devin`).

use super::lines::{CappedLines, MAX_LINE};
use super::{
    AdapterError, AdapterOutput, AgentInfo, EventSink, LiveConversation, OpenSpec, Outgoing, REQUEST_TIMEOUT,
    START_TIMEOUT, SessionChange,
};
use base64::Engine as _;
use pdt_core::conversation::acp::{AcpExtensions, AcpMapper, AcpOut, prompt_blocks};
use pdt_core::conversation::{AgentCapabilities, AgentEvent, Answer, Delivery, NoticeLevel, PermissionMode};
use pdt_core::session::AgentKind;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};

/// How much of the agent's error output is kept to explain a crash.
const STDERR_TAIL: usize = 4096;
/// How long "Send now" waits for the cancelled turn to end before sending.
const CANCEL_WAIT: Duration = Duration::from_secs(10);

/// What one ACP agent does its own way. Each method's default is plain ACP's.
pub trait AcpProfile: Send + Sync + 'static {
    /// What the agent adds to ACP's frames, for the mapper.
    type Extensions: AcpExtensions + 'static;

    fn agent(&self) -> AgentKind;

    /// The agent's name, in what PDT says about it ("Devin stopped").
    fn name(&self) -> &str;

    fn extensions(&self) -> Self::Extensions;

    /// The program's arguments, including any config file the session's launch wrote (`spec.config_file`: Devin's
    /// TUI and hook config).
    fn args(&self, spec: &OpenSpec) -> Vec<String>;

    /// The environment the program gets on top of PDT's (`spec.env`).
    fn env(&self, _spec: &OpenSpec) -> Vec<(String, String)> {
        Vec::new()
    }

    /// The PDT permission modes the agent has a session mode for. Empty: the ones the `availableModes` it
    /// advertises fit (`pdt_core::acp_agent::advertised_modes`), and when none do, PDT keeps the session's mode by
    /// answering its permission requests itself.
    fn modes(&self) -> Vec<PermissionMode> {
        Vec::new()
    }

    /// The agent's session mode id for a PDT mode (None: no modes).
    fn mode_id(&self, _mode: PermissionMode) -> Option<&str> {
        None
    }

    /// The PDT mode for one of the agent's session mode ids.
    fn pdt_mode(&self, _id: &str) -> Option<PermissionMode> {
        None
    }

    /// `session/load` failed with `code` and `message` (with the error's details): when that means the agent has no
    /// saved session to resume, a new one is started instead and this notice says why.
    fn resume_fallback(&self, _code: i64, _message: &str) -> Option<String> {
        None
    }

    /// Opening the session failed with `code` and `message`: what to tell the user instead, when the agent's error
    /// means more than it says (Antigravity's "Authentication required": it isn't signed in).
    fn refusal_hint(&self, _code: i64, _message: &str) -> Option<String> {
        None
    }

    /// The agent's session mode PDT keeps the modes this profile's map leaves out in (Cursor's `agent`): the
    /// session offers all of PDT's modes, and PDT answers permission requests for the rest (`AcpLive::apply_mode`).
    /// None: the map covers every mode it offers.
    fn base_mode(&self) -> Option<&str> {
        None
    }

    /// The agent asks before editing files, so PDT can keep Ask. False (Cursor's `agent` edits without asking): the
    /// session offers no Ask, and one started in Ask runs as Accept edits, as Devin's does.
    fn asks_before_edits(&self) -> bool {
        true
    }

    /// PDT's instructions go in front of a new session's first prompt: the agent has no other way to get them (Devin
    /// gets them from its SessionStart hook).
    fn instructions_in_prompt(&self) -> bool {
        false
    }
}

/// The PDT modes a session keeping the ones its profile has no mode for offers: all of them, but Ask for an agent that
/// edits without asking.
fn offered(profile: &impl AcpProfile) -> Vec<PermissionMode> {
    let ask = profile.asks_before_edits();
    PermissionMode::ALL.into_iter().filter(|m| ask || *m != PermissionMode::Ask).collect()
}

/// `initialize`'s parameters: no file system or terminal for the agent to borrow (it uses its own tools), and form
/// elicitation, which carries its questions (Devin's `ask_user_question`).
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
    /// The error's message, with its `data.details` or `data.message` when it gives some (Cursor's "Invalid params"
    /// says what was wrong only there: `Session "…" not found`).
    fn of(v: &Value, ext: &impl AcpExtensions) -> Self {
        let message = v.get("message").and_then(Value::as_str).unwrap_or("error");
        let details = ["/data/details", "/data/message"]
            .iter()
            .find_map(|p| v.pointer(p).and_then(Value::as_str).map(str::trim).filter(|d| !d.is_empty()));
        RpcError {
            code: v.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: match details {
                Some(d) => format!("{message}: {d}"),
                None => message.to_string(),
            },
            retryable: v.get("data").is_some_and(|d| ext.retryable(d)),
        }
    }
}

type Reply = Result<Value, RpcError>;

struct Shared<P: AcpProfile> {
    profile: P,
    mapper: Mutex<AcpMapper<P::Extensions>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
    next_id: AtomicU64,
    stdin: Mutex<Option<mpsc::UnboundedSender<String>>>,
    alive: AtomicBool,
    closing: AtomicBool,
    pid: Option<u32>,
    sink: EventSink,
    /// The agent's id for the session (empty until `session/new` or `session/load` answers).
    session: Mutex<String>,
    caps: Mutex<AgentCapabilities>,
    /// The agent's mode id as it last reported it.
    mode: Mutex<Option<String>>,
    /// The mode map the agent advertised (`availableModes`), for a profile with none of its own.
    learned_modes: Mutex<Vec<(PermissionMode, String)>>,
    /// The mode the agent's session started in, as it named it.
    start_mode: Mutex<Option<String>>,
    /// PDT's instructions, to go in front of the next prompt (a new session's first; `instructions_in_prompt`).
    instructions: Mutex<Option<String>>,
    /// Whether no turn is running (for "Send now").
    idle: watch::Sender<bool>,
}

impl<P: AcpProfile> Shared<P> {
    fn mapper(&self) -> std::sync::MutexGuard<'_, AcpMapper<P::Extensions>> {
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

    /// Sends a request and waits for its reply (`Ok(Err)`: the agent answered with an error).
    async fn request(&self, method: &str, params: Value, limit: Duration) -> Result<Reply, AdapterError> {
        let rx = self.call(method, params)?;
        match tokio::time::timeout(limit, rx).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => Err(AdapterError::NotRunning),
            Err(_) => Err(AdapterError::Timeout),
        }
    }

    /// The agent's session mode id for a PDT mode: the profile's, else the advertised one.
    fn mode_id(&self, mode: PermissionMode) -> Option<String> {
        if let Some(id) = self.profile.mode_id(mode) {
            return Some(id.to_string());
        }
        let learned = self.learned_modes.lock().unwrap_or_else(|e| e.into_inner());
        learned.iter().find(|(m, _)| *m == mode).map(|(_, id)| id.clone())
    }

    /// The PDT mode for one of the agent's session mode ids.
    fn pdt_mode(&self, id: &str) -> Option<PermissionMode> {
        self.profile.pdt_mode(id).or_else(|| {
            let learned = self.learned_modes.lock().unwrap_or_else(|e| e.into_inner());
            learned.iter().find(|(_, i)| i == id).map(|(m, _)| *m)
        })
    }

    /// Learns the agent's modes from a `session/new` or `session/load` answer (a profile with none of its own, or
    /// one that covers only some of PDT's). The session offers all of PDT's modes: those the agent has no mode for,
    /// PDT keeps itself (`AcpLive::apply_mode`).
    fn settle_modes(&self, answer: &Value) {
        let own = !self.profile.modes().is_empty();
        if own && self.profile.base_mode().is_none() {
            return;
        }
        if !own {
            let learned =
                pdt_core::acp_agent::advertised_modes(answer.pointer("/modes/availableModes").unwrap_or(&Value::Null));
            *self.learned_modes.lock().unwrap_or_else(|e| e.into_inner()) = learned;
        }
        if let Some(current) = answer.pointer("/modes/currentModeId").and_then(Value::as_str) {
            *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = Some(current.to_string());
            self.start_mode.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(|| current.to_string());
        }
        self.caps.lock().unwrap_or_else(|e| e.into_inner()).permission_modes = offered(&self.profile);
    }

    fn publish_caps(&self) {
        let caps = self.caps.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let _ = self.sink.send(AdapterOutput::Capabilities(caps));
    }

    fn emit(&self, outs: Vec<AcpOut>, raw: Option<&str>) {
        for o in outs {
            match o {
                AcpOut::Event { item_id, turn_id, event } => {
                    let _ =
                        self.sink.send(AdapterOutput::Event { item_id, turn_id, event, raw: raw.map(String::from) });
                }
                AcpOut::Delta { item_id, field, text } => {
                    let _ = self.sink.send(AdapterOutput::Delta { item_id, field, text });
                }
                AcpOut::Settings { mode, model } => {
                    if mode.is_some() {
                        *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = mode.clone();
                    }
                    let _ = self.sink.send(AdapterOutput::Info(AgentInfo {
                        model,
                        permission_mode: mode.as_deref().and_then(|m| self.pdt_mode(m)),
                        ..Default::default()
                    }));
                }
                AcpOut::Models(models) => {
                    self.caps.lock().unwrap_or_else(|e| e.into_inner()).models = models;
                    self.publish_caps();
                }
                AcpOut::Commands(commands) => {
                    self.caps.lock().unwrap_or_else(|e| e.into_inner()).commands = commands;
                    self.publish_caps();
                }
                AcpOut::RespondError { id, message } => {
                    let _ =
                        self.write(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": message}}));
                }
                AcpOut::Respond { id, result } => {
                    let _ = self.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
                }
            }
        }
        let idle = self.mapper().turn().is_none();
        self.idle.send_if_modified(|v| std::mem::replace(v, idle) != idle);
    }

    /// One line from the agent's stdout.
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
                let reply = match v.get("error") {
                    Some(e) => Err(RpcError::of(e, self.mapper().extensions())),
                    None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                };
                if let Some(tx) = self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
                    let _ = tx.send(reply);
                }
            }
            (None, None) => {}
        }
    }
}

pub struct AcpLive<P: AcpProfile> {
    shared: Arc<Shared<P>>,
    exited: watch::Receiver<bool>,
}

/// Starts or resumes an ACP agent's conversation (`AgentAdapter::open` for an ACP agent).
pub async fn open<P: AcpProfile>(
    profile: P,
    spec: OpenSpec,
    sink: EventSink,
) -> Result<Box<dyn LiveConversation>, AdapterError> {
    let live = AcpLive::spawn(profile, &spec, sink).await?;
    if let Err(e) = live.start_session(&spec).await {
        live.close().await;
        return Err(e);
    }
    Ok(Box::new(live))
}

/// `AgentAdapter::draft` for an ACP agent: ACP has no fork, so a side request would land in the conversation itself.
pub fn draft(profile: &impl AcpProfile) -> Result<Value, AdapterError> {
    Err(AdapterError::Refused(format!("Make ticket is not available for {} yet", profile.name())))
}

impl<P: AcpProfile> AcpLive<P> {
    async fn spawn(profile: P, spec: &OpenSpec, sink: EventSink) -> Result<Self, AdapterError> {
        let args = profile.args(spec);
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
        cmd.envs(profile.env(spec));
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
        let mapper = AcpMapper::with_extensions(profile.extensions(), spec.cwd.to_string_lossy(), canonical, prefix);
        // A profile that keeps the modes its map leaves out offers them all from the start.
        let permission_modes = if profile.base_mode().is_some() { offered(&profile) } else { profile.modes() };
        let shared = Arc::new(Shared {
            profile,
            mapper: Mutex::new(mapper),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            stdin: Mutex::new(Some(tx)),
            alive: AtomicBool::new(true),
            closing: AtomicBool::new(false),
            pid: child.id(),
            sink,
            session: Mutex::new(String::new()),
            caps: Mutex::new(AgentCapabilities { conversation: true, permission_modes, ..Default::default() }),
            mode: Mutex::new(None),
            learned_modes: Mutex::new(Vec::new()),
            start_mode: Mutex::new(None),
            instructions: Mutex::new(None),
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
                let mut lines = CappedLines::new(BufReader::new(stdout), MAX_LINE);
                while let Some(line) = lines.next_line().await {
                    shared.on_line(&line);
                    shared.sink.ready().await;
                }
                let code = child.wait().await.ok().and_then(|s| s.code());
                let _ = err_task.await;
                shared.alive.store(false, Ordering::SeqCst);
                shared.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
                let expected = shared.closing.load(Ordering::SeqCst);
                let reason =
                    if expected { "Stopped".to_string() } else { format!("{} stopped", shared.profile.name()) };
                let outs = shared.mapper().on_exit(&reason);
                shared.emit(outs, None);
                // After the turn has ended: the prompts still waiting then map to nothing.
                shared.pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
                let stderr_tail = tail.lock().unwrap_or_else(|e| e.into_inner()).trim().to_string();
                let _ = shared.sink.send(AdapterOutput::Exited { code, stderr_tail, expected });
                let _ = exit_tx.send(true);
            });
        }
        Ok(AcpLive { shared, exited: exit_rx })
    }

    /// `initialize`, then the saved session (`session/load`, whose replay of history PDT has logged already) or a new
    /// one, then the session's mode and model.
    async fn start_session(&self, spec: &OpenSpec) -> Result<(), AdapterError> {
        let sh = &self.shared;
        let refused =
            |e: RpcError| AdapterError::Refused(sh.profile.refusal_hint(e.code, &e.message).unwrap_or(e.message));
        let init = sh.request("initialize", initialize_params(), START_TIMEOUT).await?.map_err(refused)?;
        let can_load = init.pointer("/agentCapabilities/loadSession").and_then(Value::as_bool).unwrap_or(false);
        let cwd = spec.cwd.to_string_lossy().into_owned();
        let (mcp, skipped) = crate::mcp::acp_servers(spec.mcp.as_ref(), crate::mcp::AcpMcpCaps::from_init(&init));
        let mut notes = vec![];
        if !skipped.is_empty() {
            notes.push(format!(
                "{} can't connect to remote MCP servers of this kind, so it doesn't get {}.",
                sh.profile.name(),
                skipped.join(", ")
            ));
        }
        if let Some(m) = spec
            .mcp
            .as_ref()
            .filter(|m| m.extra.iter().any(|x| !x.plan.with(pdt_core::mcp_servers::ToolMode::Block).is_empty()))
        {
            let names: Vec<&str> = m
                .extra
                .iter()
                .filter(|x| !x.plan.with(pdt_core::mcp_servers::ToolMode::Block).is_empty())
                .map(|x| x.name.as_str())
                .collect();
            notes.push(format!(
                "{} can't block single MCP tools: the tools blocked on {} are still available to it.",
                sh.profile.name(),
                names.join(", ")
            ));
        }
        for text in notes {
            let _ = sh.sink.send(AdapterOutput::Event {
                item_id: None,
                turn_id: None,
                event: AgentEvent::Notice { level: NoticeLevel::Info, text, until: None },
                raw: None,
            });
        }
        let mut loaded = false;
        let resume = spec.external_id.clone().filter(|_| spec.resume);
        if resume.is_some() && !can_load {
            let text = format!("{} can't resume a conversation, so this is a new one.", sh.profile.name());
            let _ = sh.sink.send(AdapterOutput::Event {
                item_id: None,
                turn_id: None,
                event: AgentEvent::Notice { level: NoticeLevel::Info, text, until: None },
                raw: None,
            });
        }
        if let Some(id) = resume.filter(|_| can_load) {
            *sh.session.lock().unwrap_or_else(|e| e.into_inner()) = id.clone();
            sh.mapper().begin_load(spec.import_after);
            let r = sh
                .request("session/load", json!({"sessionId": id, "cwd": cwd, "mcpServers": mcp}), START_TIMEOUT)
                .await;
            let outs = sh.mapper().end_load();
            sh.emit(outs, None);
            match r? {
                Ok(v) => {
                    loaded = true;
                    sh.settle_modes(&v);
                    let outs = sh.mapper().on_session(&v);
                    sh.emit(outs, None);
                }
                Err(e) => {
                    let Some(text) = sh.profile.resume_fallback(e.code, &e.message) else { return Err(refused(e)) };
                    sh.session.lock().unwrap_or_else(|e| e.into_inner()).clear();
                    let _ = sh.sink.send(AdapterOutput::Event {
                        item_id: None,
                        turn_id: None,
                        event: AgentEvent::Notice { level: NoticeLevel::Info, text, until: None },
                        raw: None,
                    });
                }
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
                .ok_or_else(|| AdapterError::Refused(format!("{} did not name the new session", sh.profile.name())))?
                .to_string();
            *sh.session.lock().unwrap_or_else(|e| e.into_inner()) = id.clone();
            let _ = sh.sink.send(AdapterOutput::ExternalId(id));
            sh.settle_modes(&v);
            let outs = sh.mapper().on_session(&v);
            sh.emit(outs, None);
            if sh.profile.instructions_in_prompt() {
                *sh.instructions.lock().unwrap_or_else(|e| e.into_inner()) =
                    spec.instructions.as_ref().map(|i| i.text.clone());
            }
        }
        self.apply_mode(spec.permission_mode).await?;
        if let Some(m) = &spec.model {
            self.set_model(m).await?;
        }
        sh.publish_caps();
        Ok(())
    }

    /// Puts the session in `mode`: the agent's own mode for it when it has one; otherwise PDT keeps the mode by
    /// answering its permission requests, with the agent in the profile's base mode, else back in the mode it started
    /// in when that is none of PDT's (an agent left in its Plan would refuse what Bypass allows).
    /// Returns the mode the session runs in (Accept edits for Ask on an agent that edits without asking).
    async fn apply_mode(&self, mode: PermissionMode) -> Result<PermissionMode, AdapterError> {
        let sh = &self.shared;
        let mode = if mode == PermissionMode::Ask && !sh.profile.asks_before_edits() {
            // It would edit without asking: say what the session really runs in.
            let _ = sh.sink.send(AdapterOutput::Info(AgentInfo {
                permission_mode: Some(PermissionMode::AcceptEdits),
                ..Default::default()
            }));
            PermissionMode::AcceptEdits
        } else {
            mode
        };
        let current = sh.mode.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let want = match sh.mode_id(mode) {
            Some(id) => {
                sh.mapper().enforce_mode(None);
                Some(id)
            }
            None => {
                sh.mapper().enforce_mode(Some(mode));
                let start = sh.start_mode.lock().unwrap_or_else(|e| e.into_inner()).clone();
                sh.profile.base_mode().map(String::from).or_else(|| start.filter(|id| sh.pdt_mode(id).is_none()))
            }
        };
        if let Some(id) = want.filter(|id| current.as_deref() != Some(id.as_str())) {
            self.set_mode(&id).await?;
        }
        Ok(mode)
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
        // The child leads its own process group: end its tools too.
        if let Some(pid) = self.shared.pid {
            crate::proc::terminate_group(pid, crate::proc::KILL_GRACE);
        }
    }
}

#[async_trait::async_trait]
impl<P: AcpProfile> LiveConversation for AcpLive<P> {
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
        let mut blocks = prompt_blocks(&msg.text, &images);
        let instructions = sh.instructions.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let (Some(text), Some(blocks)) = (instructions, blocks.as_array_mut()) {
            blocks.insert(0, json!({"type": "text", "text": text}));
        }
        let params = json!({"sessionId": sh.session(), "prompt": blocks});
        let outs = sh.mapper().begin_prompt(&msg.text);
        sh.emit(outs, None);
        let rx = match sh.call("session/prompt", params) {
            Ok(rx) => rx,
            Err(e) => {
                let outs = sh.mapper().end_prompt(Err((format!("{} is not running", sh.profile.name()), true)));
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
        if let Some(mode) = a.mode {
            // An approved plan of the agent's own plan request: it leaves plan mode only when told to.
            let mode = self.apply_mode(mode).await?;
            let _ = self
                .shared
                .sink
                .send(AdapterOutput::Info(AgentInfo { permission_mode: Some(mode), ..Default::default() }));
        }
        if let Some(note) = a.follow_up {
            // ACP's reject has no message: the note goes to the agent as the next message (taken into the running
            // turn).
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
                // ACP has no "default" to pick: the next process starts on the saved session's model.
                None => {
                    let name = self.shared.profile.name();
                    return Err(AdapterError::Refused(format!("{name} has no default model to switch back to")));
                }
            }
        }
        if let Some(mode) = change.permission_mode {
            self.apply_mode(mode).await?;
        }
        Ok(())
    }

    async fn close(&self) {
        self.shared.closing.store(true, Ordering::SeqCst);
        self.shared.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
        super::await_close(self.exited.clone(), || self.kill()).await;
    }

    fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }
}

impl<P: AcpProfile> Drop for AcpLive<P> {
    fn drop(&mut self) {
        if self.is_alive() {
            self.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{EventStream, event_channel};
    use pdt_core::conversation::acp::PlainAcp;
    use pdt_core::conversation::{RequestKind, RequestState, TurnPhase, TurnStatus};

    /// Frames from a plain ACP agent's stdout, with a stray session's update and a line that is not JSON.
    const SESSION: &str = include_str!("../../tests/fixtures/acp/session.jsonl");

    /// A plain ACP agent with no modes of its own; `edits_freely`: it edits without asking (Cursor's `agent`).
    struct Probe {
        edits_freely: bool,
    }

    impl AcpProfile for Probe {
        type Extensions = PlainAcp;
        fn agent(&self) -> AgentKind {
            AgentKind::Devin
        }
        fn name(&self) -> &str {
            "Probe"
        }
        fn extensions(&self) -> PlainAcp {
            PlainAcp
        }
        fn args(&self, _spec: &OpenSpec) -> Vec<String> {
            vec![]
        }
        fn asks_before_edits(&self) -> bool {
            !self.edits_freely
        }
    }

    struct Rig {
        live: AcpLive<Probe>,
        stream: EventStream,
        stdin: mpsc::UnboundedReceiver<String>,
        idle: watch::Receiver<bool>,
    }

    /// A live conversation with no process: what PDT writes to the agent arrives on `stdin`.
    fn rig(profile: Probe) -> Rig {
        let (sink, stream) = event_channel(256);
        let (tx, stdin) = mpsc::unbounded_channel();
        let (idle_tx, idle) = watch::channel(true);
        let mapper = AcpMapper::with_extensions(profile.extensions(), "/work/repo", None, "p");
        let shared = Arc::new(Shared {
            profile,
            mapper: Mutex::new(mapper),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            stdin: Mutex::new(Some(tx)),
            alive: AtomicBool::new(true),
            closing: AtomicBool::new(false),
            pid: None,
            sink,
            session: Mutex::new("s1".into()),
            caps: Mutex::new(AgentCapabilities::default()),
            mode: Mutex::new(None),
            learned_modes: Mutex::new(Vec::new()),
            start_mode: Mutex::new(None),
            instructions: Mutex::new(None),
            idle: idle_tx,
        });
        let (_exited_tx, exited) = watch::channel(false);
        Rig { live: AcpLive { shared, exited }, stream, stdin, idle }
    }

    fn drain(stream: &mut EventStream) -> Vec<AdapterOutput> {
        std::iter::from_fn(|| stream.try_recv().ok()).collect()
    }

    fn written(stdin: &mut mpsc::UnboundedReceiver<String>) -> Vec<Value> {
        std::iter::from_fn(|| stdin.try_recv().ok()).map(|l| serde_json::from_str(&l).unwrap()).collect()
    }

    fn events(outs: &[AdapterOutput]) -> Vec<&AgentEvent> {
        outs.iter()
            .filter_map(|o| match o {
                AdapterOutput::Event { event, .. } => Some(event),
                _ => None,
            })
            .collect()
    }

    /// `session/new`'s answer for an agent that advertises an Ask and a Plan mode and starts in its own `custom`.
    fn session_answer() -> Value {
        json!({"sessionId": "s1", "modes": {"currentModeId": "custom", "availableModes": [
            {"id": "default", "name": "Default"}, {"id": "architect", "name": "Architect"}, {"id": "custom", "name": "Custom"}]}})
    }

    #[test]
    fn stdout_lines_become_outputs_for_this_session_and_refusals_for_the_agent() {
        let mut r = rig(Probe { edits_freely: false });
        let sh = &r.live.shared;
        sh.settle_modes(&session_answer());
        let lines: Vec<&str> = SESSION.lines().collect();
        for l in &lines[..5] {
            sh.on_line(l);
        }
        let outs = drain(&mut r.stream);
        // Another session's commands never arrive; this one's are published with the capabilities.
        let AdapterOutput::Capabilities(caps) = &outs[0] else { panic!("{outs:?}") };
        assert_eq!(caps.commands.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["init"]);
        assert_eq!(caps.permission_modes, PermissionMode::ALL.to_vec(), "PDT keeps the modes the agent has none for");
        // The agent's mode, by its name for it, is PDT's mode it advertised for it.
        let AdapterOutput::Info(info) = &outs[1] else { panic!("{outs:?}") };
        assert_eq!(info.permission_mode, Some(PermissionMode::Plan));
        assert_eq!(sh.mode.lock().unwrap().as_deref(), Some("architect"));
        let AdapterOutput::Info(info) = &outs[2] else { panic!("{outs:?}") };
        assert_eq!((info.model.as_deref(), info.permission_mode), (Some("m1"), None));
        let AdapterOutput::Capabilities(caps) = &outs[3] else { panic!("{outs:?}") };
        assert_eq!(caps.models[0].display_name, "Model One");
        assert_eq!(outs.len(), 4, "the line that is not JSON is skipped");

        // A turn running: the conversation is busy, and its frames arrive with the line they came from.
        let outs = sh.mapper().begin_prompt("go");
        sh.emit(outs, None);
        assert!(!*r.idle.borrow());
        for l in &lines[5..] {
            sh.on_line(l);
        }
        let outs = drain(&mut r.stream);
        assert!(outs.iter().any(|o| matches!(o, AdapterOutput::Delta { text, .. } if text == "Hello")));
        let raw = outs.iter().find_map(|o| match o {
            AdapterOutput::Event {
                event: AgentEvent::Request { request_kind: RequestKind::Approval, .. },
                raw,
                ..
            } => raw.clone(),
            _ => None,
        });
        assert_eq!(raw.as_deref(), Some(lines[7]));
        // The client method PDT does not offer is refused at once, with JSON-RPC's "method not found".
        assert_eq!(
            written(&mut r.stdin),
            [
                json!({"jsonrpc": "2.0", "id": 8, "error": {"code": -32601, "message": "PDT does not handle terminal/create"}})
            ]
        );
    }

    #[test]
    fn replies_reach_the_call_that_asked_with_the_errors_details() {
        let mut r = rig(Probe { edits_freely: false });
        let sh = &r.live.shared;
        let mut ok = sh.call("session/new", json!({"cwd": "/work/repo"})).unwrap();
        let mut failed = sh.call("session/load", json!({"sessionId": "gone"})).unwrap();
        let sent = written(&mut r.stdin);
        assert_eq!(
            sent[0],
            json!({"jsonrpc": "2.0", "id": 0, "method": "session/new", "params": {"cwd": "/work/repo"}})
        );
        assert_eq!(sent[1]["id"], 1);
        // Replies with ids PDT never used, or that are not numbers, are dropped.
        sh.on_line(r#"{"jsonrpc":"2.0","id":99,"result":{}}"#);
        sh.on_line(r#"{"jsonrpc":"2.0","id":"x","result":{}}"#);
        sh.on_line(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Invalid params","data":{"details":" Session \"gone\" not found "}}}"#);
        sh.on_line(r#"{"jsonrpc":"2.0","id":0,"result":{"sessionId":"s2"}}"#);
        assert_eq!(ok.try_recv().unwrap().unwrap(), json!({"sessionId": "s2"}));
        let e = failed.try_recv().unwrap().unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str(), e.retryable),
            (-32602, "Invalid params: Session \"gone\" not found", false)
        );
        assert!(sh.pending.lock().unwrap().is_empty());
        // `data.message` explains an error too; with neither, the message stands alone.
        let e = RpcError::of(
            &json!({"code": -32603, "message": "Internal error", "data": {"message": "disk full"}}),
            &PlainAcp,
        );
        assert_eq!(e.message, "Internal error: disk full");
        let e = RpcError::of(&json!({"data": {"details": "  "}}), &PlainAcp);
        assert_eq!((e.code, e.message.as_str()), (0, "error"));
        // Once stdin is closed, a call fails and is not left waiting.
        sh.stdin.lock().unwrap().take();
        assert!(matches!(sh.call("session/prompt", json!({})), Err(AdapterError::NotRunning)));
        assert!(sh.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_prompt_carries_the_instructions_once_and_its_answer_ends_the_turn() {
        let mut r = rig(Probe { edits_freely: false });
        let sh = r.live.shared.clone();
        *sh.instructions.lock().unwrap() = Some("PDT says hello".into());
        let msg = |t: &str| Outgoing { text: t.into(), images: vec![] };
        r.live.send(&msg("first"), Delivery::Next).await.unwrap();
        let sent = written(&mut r.stdin);
        assert_eq!(sent[0]["method"], "session/prompt");
        assert_eq!(
            sent[0]["params"],
            json!({"sessionId": "s1", "prompt": [{"type": "text", "text": "PDT says hello"}, {"type": "text", "text": "first"}]})
        );
        let ev = drain(&mut r.stream);
        assert!(matches!(
            events(&ev)[..],
            [AgentEvent::Turn { phase: TurnPhase::Started, .. }, AgentEvent::UserMessage { .. }]
        ));
        // A prompt sent mid-turn joins it, without the instructions again.
        r.live.send(&msg("second"), Delivery::Next).await.unwrap();
        let sent = written(&mut r.stdin);
        assert_eq!(sent[0]["params"]["prompt"], json!([{"type": "text", "text": "second"}]));
        sh.on_line(r#"{"jsonrpc":"2.0","id":0,"result":{"stopReason":"end_turn"}}"#);
        sh.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}"#);
        let mut idle = r.idle.clone();
        tokio::time::timeout(Duration::from_secs(5), idle.wait_for(|i| *i)).await.unwrap().unwrap();
        let ev = drain(&mut r.stream);
        assert!(matches!(events(&ev).last(), Some(AgentEvent::Turn { status: Some(TurnStatus::Completed), .. })));
        // A stopped agent takes nothing.
        sh.alive.store(false, Ordering::SeqCst);
        assert!(matches!(r.live.send(&msg("x"), Delivery::Next).await, Err(AdapterError::NotRunning)));
    }

    #[tokio::test]
    async fn interrupting_cancels_the_open_requests_then_the_turn() {
        let mut r = rig(Probe { edits_freely: false });
        let sh = r.live.shared.clone();
        // Nothing running: nothing to cancel.
        r.live.interrupt().await.unwrap();
        assert!(written(&mut r.stdin).is_empty());
        let outs = sh.mapper().begin_prompt("go");
        sh.emit(outs, None);
        sh.on_line(r#"{"jsonrpc":"2.0","id":3,"method":"elicitation/create","params":{"requestedSchema":{"properties":{"x":{"type":"string"}}}}}"#);
        drain(&mut r.stream);
        r.live.interrupt().await.unwrap();
        assert_eq!(
            written(&mut r.stdin),
            [
                json!({"jsonrpc": "2.0", "id": 3, "result": {"action": "cancel"}}),
                json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": "s1"}}),
            ]
        );
        let ev = drain(&mut r.stream);
        assert!(matches!(events(&ev)[..], [AgentEvent::Request { state: RequestState::Expired, .. }]));
        // The agent's `end_turn` after a cancel is the user's interrupt.
        let outs = sh.mapper().end_prompt(Ok(&json!({"stopReason": "end_turn"})));
        assert!(matches!(
            outs.last(),
            Some(AcpOut::Event { event: AgentEvent::Turn { status: Some(TurnStatus::Interrupted), .. }, .. })
        ));
    }

    #[tokio::test]
    async fn answers_go_back_on_stdin_and_a_refusals_note_follows_as_a_prompt() {
        let mut r = rig(Probe { edits_freely: false });
        let sh = r.live.shared.clone();
        let outs = sh.mapper().begin_prompt("go");
        sh.emit(outs, None);
        let lines: Vec<&str> = SESSION.lines().collect();
        sh.on_line(lines[6]);
        sh.on_line(lines[7]);
        drain(&mut r.stream);
        assert!(matches!(
            r.live.answer("nope", &Answer::Approve { always: false }).await,
            Err(AdapterError::RequestClosed)
        ));
        assert!(matches!(
            r.live.answer("7", &Answer::Question { answers: Default::default() }).await,
            Err(AdapterError::WrongAnswer)
        ));
        r.live.answer("7", &Answer::Deny { message: Some("use ls -la".into()) }).await.unwrap();
        let sent = written(&mut r.stdin);
        assert_eq!(
            sent[0],
            json!({"jsonrpc": "2.0", "id": 7, "result": {"outcome": {"outcome": "selected", "optionId": "no"}}})
        );
        assert_eq!(sent[1]["method"], "session/prompt");
        assert_eq!(sent[1]["params"]["prompt"], json!([{"type": "text", "text": "use ls -la"}]));
        let ev = drain(&mut r.stream);
        assert!(events(&ev).iter().any(|e| matches!(e, AgentEvent::Request { state: RequestState::Answered, .. })));
        assert!(events(&ev).iter().any(|e| matches!(e, AgentEvent::UserMessage { text, .. } if text == "use ls -la")));
    }

    #[tokio::test]
    async fn modes_the_agent_lacks_are_kept_by_pdt_and_ask_runs_as_accept_edits_when_it_edits_freely() {
        let mut r = rig(Probe { edits_freely: true });
        let sh = r.live.shared.clone();
        sh.settle_modes(&session_answer());
        assert_eq!(sh.mode_id(PermissionMode::Plan).as_deref(), Some("architect"));
        assert_eq!(sh.pdt_mode("default"), Some(PermissionMode::Ask));
        assert_eq!(sh.pdt_mode("custom"), None);
        assert!(!sh.caps.lock().unwrap().permission_modes.contains(&PermissionMode::Ask));
        assert_eq!(*sh.start_mode.lock().unwrap(), Some("custom".to_string()));
        // Plan is the agent's own: PDT switches it there.
        let apply = r.live.apply_mode(PermissionMode::Plan);
        let reply = async {
            loop {
                if let Ok(l) = r.stdin.try_recv() {
                    let v: Value = serde_json::from_str(&l).unwrap();
                    assert_eq!(v["params"], json!({"sessionId": "s1", "modeId": "architect"}));
                    sh.on_line(&json!({"jsonrpc": "2.0", "id": v["id"], "result": {}}).to_string());
                    break;
                }
                tokio::task::yield_now().await;
            }
        };
        let (mode, ()) = tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(apply, reply) })
            .await
            .expect("PDT asked to set the mode");
        assert_eq!(mode.unwrap(), PermissionMode::Plan);
        sh.on_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"current_mode_update","currentModeId":"architect"}}}"#);
        // Ask, for an agent that edits without asking, runs as Accept edits; PDT keeps that itself, back in the mode
        // the agent started in.
        let apply = r.live.apply_mode(PermissionMode::Ask);
        let reply = async {
            loop {
                if let Ok(l) = r.stdin.try_recv() {
                    let v: Value = serde_json::from_str(&l).unwrap();
                    assert_eq!(v["params"]["modeId"], "custom");
                    sh.on_line(
                        &json!({"jsonrpc": "2.0", "id": v["id"], "error": {"code": 1, "message": "no"}}).to_string(),
                    );
                    break;
                }
                tokio::task::yield_now().await;
            }
        };
        let (mode, ()) = tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(apply, reply) })
            .await
            .expect("PDT asked to set the mode");
        assert!(matches!(mode, Err(AdapterError::Refused(m)) if m == "no"));
        let outs = drain(&mut r.stream);
        assert!(
            outs.iter()
                .any(|o| matches!(o, AdapterOutput::Info(i) if i.permission_mode == Some(PermissionMode::AcceptEdits)))
        );
        // ACP has no default model to go back to.
        let change = SessionChange { model: Some(None), permission_mode: None };
        assert!(
            matches!(r.live.set(&change).await, Err(AdapterError::Refused(m)) if m == "Probe has no default model to switch back to")
        );
    }

    #[test]
    fn initialize_offers_no_file_system_or_terminal_and_drafts_are_refused() {
        let p = initialize_params();
        assert_eq!(p["protocolVersion"], 1);
        assert_eq!(p["clientCapabilities"]["terminal"], false);
        assert_eq!(p["clientCapabilities"]["fs"], json!({"readTextFile": false, "writeTextFile": false}));
        assert_eq!(p["clientCapabilities"]["elicitation"], json!({"form": {}}));
        assert_eq!(p["clientInfo"]["name"], pdt_core::brand::SLUG);
        let e = draft(&Probe { edits_freely: false }).unwrap_err();
        assert!(matches!(e, AdapterError::Refused(m) if m == "Make ticket is not available for Probe yet"));
        assert_eq!(offered(&Probe { edits_freely: false }), PermissionMode::ALL.to_vec());
        assert!(!offered(&Probe { edits_freely: true }).contains(&PermissionMode::Ask));
    }
}
