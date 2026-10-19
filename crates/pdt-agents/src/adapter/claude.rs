//! Claude Code through `claude -p` stream-json: one process per live conversation (spec §3.2). The frame mapping is
//! pdt-core's `ClaudeMapper`; this module owns the process, its pipes and control-request replies.

use super::{
    AdapterError, AdapterOutput, AgentAdapter, AgentInfo, EventSink, LiveConversation, OpenSpec, Outgoing,
    SessionChange,
};
use base64::Engine as _;
use pdt_core::conversation::claude::{AnswerError, ClaudeMapper, MapOut, user_line};
use pdt_core::conversation::{AgentCapabilities, AgentCommandInfo, AgentModel, Answer, Delivery, PermissionMode};
use pdt_core::session::AgentKind;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};

/// How much of the agent's error output is kept to explain a crash.
const STDERR_TAIL: usize = 4096;
/// How long a control request may take.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a closing process gets after its stdin closes before it is killed.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

/// The `claude -p` arguments for a session. Never `--bare` (it would stop using the plan login), never PDT's hook
/// settings (activity comes from the protocol).
pub fn claude_args(spec: &OpenSpec) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--replay-user-messages",
        "--permission-prompts",
        "host",
        // Hidden but required: without it every prompt is silently denied (notes file).
        "--permission-prompt-tool",
        "stdio",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    a.push(if spec.resume { "--resume" } else { "--session-id" }.into());
    a.push(spec.session_id.clone());
    a.push("--permission-mode".into());
    a.push(spec.permission_mode.claude().into());
    if let Some(m) = &spec.model {
        a.push("--model".into());
        a.push(m.clone());
    }
    a
}

/// The arguments of a side request (Make ticket): a fork of the saved conversation that is never saved, with no
/// tools, answering in JSON shaped by `schema`. The prompt goes in on stdin (a Windows `.cmd` shim mangles free text
/// in arguments; the schema is plain enough to pass). Never `--bare`.
pub fn claude_draft_args(spec: &OpenSpec, schema: &Value) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-p".into(),
        "--resume".into(),
        spec.session_id.clone(),
        "--fork-session".into(),
        "--no-session-persistence".into(),
        "--tools".into(),
        String::new(),
        "--output-format".into(),
        "json".into(),
        "--json-schema".into(),
        schema.to_string(),
    ];
    if let Some(m) = &spec.model {
        a.push("--model".into());
        a.push(m.clone());
    }
    a
}

/// The answer in a side request's `--output-format json` result: its `structured_output`, else its text.
pub fn draft_answer(stdout: &str) -> Result<Value, AdapterError> {
    let result = stdout
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .find(|v| v.get("type").and_then(Value::as_str) == Some("result"))
        .ok_or_else(|| AdapterError::Refused("Claude gave no answer".into()))?;
    if result.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        let why = result
            .get("result")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .or(result.get("subtype").and_then(Value::as_str))
            .unwrap_or("error");
        return Err(AdapterError::Refused(why.to_string()));
    }
    match result.get("structured_output").filter(|v| !v.is_null()) {
        Some(v) => Ok(v.clone()),
        None => result.get("result").cloned().ok_or_else(|| AdapterError::Refused("Claude gave no answer".into())),
    }
}

/// The capabilities in Claude's `initialize` reply.
pub fn capabilities_from_initialize(v: &Value, version: Option<String>) -> AgentCapabilities {
    let models = v
        .get("models")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let value = m.get("value")?.as_str()?.to_string();
                    let display_name = m.get("displayName").and_then(Value::as_str).unwrap_or(&value).to_string();
                    let description = m.get("description").and_then(Value::as_str).map(String::from);
                    Some(AgentModel { value, display_name, description })
                })
                .collect()
        })
        .unwrap_or_default();
    let commands = v
        .get("commands")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|c| serde_json::from_value::<AgentCommandInfo>(c.clone()).ok()).collect())
        .unwrap_or_default();
    AgentCapabilities {
        conversation: true,
        permission_modes: PermissionMode::ALL.to_vec(),
        models,
        commands,
        terminal_commands: vec![],
        version,
    }
}

pub struct ClaudeAdapter;

struct Shared {
    session_id: String,
    mapper: Mutex<ClaudeMapper>,
    replies: Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>,
    stdin: Mutex<Option<mpsc::UnboundedSender<String>>>,
    alive: AtomicBool,
    closing: AtomicBool,
    pid: Option<u32>,
    sink: EventSink,
    version: Mutex<Option<String>>,
    /// What `initialize` reported, completed by the TUI-only commands that each turn's `init` lists.
    caps: Mutex<Option<AgentCapabilities>>,
    terminal_commands: Mutex<Vec<String>>,
}

impl Shared {
    /// Sends the capabilities once both halves may have arrived (in either order).
    fn publish_caps(&self) {
        let caps = self.caps.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(mut c) = caps {
            c.terminal_commands = self.terminal_commands.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let _ = self.sink.send(AdapterOutput::Capabilities(c));
        }
    }
}

impl Shared {
    fn write(&self, line: &Value) -> Result<(), AdapterError> {
        let tx = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        let tx = tx.as_ref().ok_or(AdapterError::NotRunning)?;
        tx.send(line.to_string()).map_err(|_| AdapterError::NotRunning)
    }

    fn emit(&self, outs: Vec<MapOut>, raw: Option<&str>) {
        for o in outs {
            let out = match o {
                MapOut::Event { item_id, turn_id, event } => {
                    AdapterOutput::Event { item_id, turn_id, event, raw: raw.map(String::from) }
                }
                MapOut::Delta { item_id, field, text } => AdapterOutput::Delta { item_id, field, text },
                MapOut::Init(i) => {
                    if !i.terminal_commands.is_empty() {
                        let changed = {
                            let mut t = self.terminal_commands.lock().unwrap_or_else(|e| e.into_inner());
                            let changed = *t != i.terminal_commands;
                            *t = i.terminal_commands.clone();
                            changed
                        };
                        if changed {
                            self.publish_caps();
                        }
                    }
                    if i.version.is_some() {
                        *self.version.lock().unwrap_or_else(|e| e.into_inner()) = i.version.clone();
                    }
                    AdapterOutput::Info(AgentInfo {
                        model: i.model,
                        permission_mode: i.permission_mode.as_deref().and_then(PermissionMode::from_claude),
                        version: i.version,
                        api_key_source: i.api_key_source,
                        billing: None,
                    })
                }
                MapOut::Reply { request_id, result } => {
                    if let Some(tx) = self.replies.lock().unwrap_or_else(|e| e.into_inner()).remove(&request_id) {
                        let _ = tx.send(result);
                    }
                    continue;
                }
                MapOut::Send(v) => {
                    let _ = self.write(&v);
                    continue;
                }
            };
            let _ = self.sink.send(out);
        }
    }

    /// Sends one of PDT's own control requests and waits for Claude's reply.
    async fn control(&self, subtype: &str, extra: Value) -> Result<Value, AdapterError> {
        let (id, line) = self.mapper.lock().unwrap_or_else(|e| e.into_inner()).control_line(subtype, extra);
        let (tx, rx) = oneshot::channel();
        self.replies.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), tx);
        if let Err(e) = self.write(&line) {
            self.replies.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(CONTROL_TIMEOUT, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(AdapterError::Refused(e)),
            Ok(Err(_)) => Err(AdapterError::NotRunning),
            Err(_) => {
                self.replies.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                Err(AdapterError::Timeout)
            }
        }
    }
}

pub struct ClaudeLive {
    shared: Arc<Shared>,
    exited: watch::Receiver<bool>,
}

#[async_trait::async_trait]
impl AgentAdapter for ClaudeAdapter {
    fn agent(&self) -> AgentKind {
        AgentKind::Claude
    }

    async fn open(&self, spec: OpenSpec, sink: EventSink) -> Result<Box<dyn LiveConversation>, AdapterError> {
        Ok(Box::new(ClaudeLive::spawn(spec, sink).await?))
    }

    async fn draft(&self, spec: OpenSpec, prompt: &str, schema: &Value) -> Result<Value, AdapterError> {
        let args = claude_draft_args(&spec, schema);
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
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().map_err(|e| AdapterError::Spawn(e.to_string()))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.map_err(|e| AdapterError::Spawn(e.to_string()))?;
            // Dropped here: stdin closes, so Claude reads the whole prompt.
        }
        // Dropping the future on a timeout kills the process (kill_on_drop).
        let out = tokio::time::timeout(super::DRAFT_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| AdapterError::Timeout)?
            .map_err(|e| AdapterError::Spawn(e.to_string()))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        match draft_answer(&stdout) {
            Err(AdapterError::Refused(why)) if why == "Claude gave no answer" && !out.status.success() => {
                let err = String::from_utf8_lossy(&out.stderr);
                let tail = err.trim().lines().last().unwrap_or("").to_string();
                Err(AdapterError::Refused(if tail.is_empty() {
                    format!("Claude exited ({})", out.status)
                } else {
                    tail
                }))
            }
            r => r,
        }
    }
}

impl ClaudeLive {
    pub async fn spawn(spec: OpenSpec, sink: EventSink) -> Result<Self, AdapterError> {
        let args = claude_args(&spec);
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
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let shared = Arc::new(Shared {
            session_id: spec.session_id.clone(),
            mapper: Mutex::new(ClaudeMapper::with_cwd(spec.cwd.to_string_lossy())),
            replies: Mutex::new(HashMap::new()),
            stdin: Mutex::new(Some(tx)),
            alive: AtomicBool::new(true),
            closing: AtomicBool::new(false),
            pid: child.id(),
            sink,
            version: Mutex::new(None),
            caps: Mutex::new(None),
            terminal_commands: Mutex::new(Vec::new()),
        });

        // Writer: one line per message; stdin closes when every sender is gone.
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
                    let outs = shared.mapper.lock().unwrap_or_else(|e| e.into_inner()).on_line(&line);
                    shared.emit(outs, Some(&line));
                }
                let code = child.wait().await.ok().and_then(|s| s.code());
                let _ = err_task.await;
                shared.alive.store(false, Ordering::SeqCst);
                shared.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
                shared.replies.lock().unwrap_or_else(|e| e.into_inner()).clear();
                let expected = shared.closing.load(Ordering::SeqCst);
                let reason = if expected { "Stopped" } else { "Claude stopped" };
                let outs = shared.mapper.lock().unwrap_or_else(|e| e.into_inner()).on_exit(reason);
                shared.emit(outs, None);
                let stderr_tail = tail.lock().unwrap_or_else(|e| e.into_inner()).trim().to_string();
                let _ = shared.sink.send(AdapterOutput::Exited { code, stderr_tail, expected });
                let _ = exit_tx.send(true);
            });
        }

        // Ask for models and commands once; nothing waits on it.
        {
            let shared = shared.clone();
            tokio::spawn(async move {
                if let Ok(v) = shared.control("initialize", json!({})).await {
                    let version = shared.version.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    *shared.caps.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(capabilities_from_initialize(&v, version));
                    shared.publish_caps();
                }
            });
        }
        Ok(ClaudeLive { shared, exited: exit_rx })
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
impl LiveConversation for ClaudeLive {
    async fn send(&self, msg: &Outgoing, delivery: Delivery) -> Result<(), AdapterError> {
        let now = delivery == Delivery::Now;
        let mut images = Vec::new();
        for img in &msg.images {
            let bytes = tokio::fs::read(&img.path)
                .await
                .map_err(|e| AdapterError::Spawn(format!("reading an attached image: {e}")))?;
            images.push((img.media_type.clone(), base64::engine::general_purpose::STANDARD.encode(bytes)));
        }
        {
            let mut m = self.shared.mapper.lock().unwrap_or_else(|e| e.into_inner());
            if now && m.turn().is_some() {
                m.note_interrupt();
            }
            if msg.text.trim_start().starts_with('/') && images.is_empty() {
                m.note_sent_command(&msg.text);
            }
        }
        self.shared.write(&user_line(&self.shared.session_id, &msg.text, &images, now))
    }

    async fn answer(&self, request_id: &str, answer: &Answer) -> Result<(), AdapterError> {
        let (line, outs) = {
            let mut m = self.shared.mapper.lock().unwrap_or_else(|e| e.into_inner());
            m.answer(request_id, answer).map_err(|e| match e {
                AnswerError::Closed => AdapterError::RequestClosed,
                AnswerError::WrongAnswer => AdapterError::WrongAnswer,
            })?
        };
        self.shared.write(&line)?;
        self.shared.emit(outs, None);
        Ok(())
    }

    async fn interrupt(&self) -> Result<(), AdapterError> {
        self.shared.mapper.lock().unwrap_or_else(|e| e.into_inner()).note_interrupt();
        self.shared.control("interrupt", json!({})).await.map(|_| ())
    }

    async fn set(&self, change: &SessionChange) -> Result<(), AdapterError> {
        if let Some(model) = &change.model {
            let m = model.clone().unwrap_or_else(|| "default".into());
            self.shared.control("set_model", json!({"model": m})).await?;
        }
        if let Some(mode) = change.permission_mode {
            self.shared.control("set_permission_mode", json!({"mode": mode.claude()})).await?;
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

impl Drop for ClaudeLive {
    fn drop(&mut self) {
        if self.is_alive() {
            self.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(resume: bool, model: Option<&str>) -> OpenSpec {
        OpenSpec {
            session_id: "sid".into(),
            external_id: None,
            resume,
            cwd: std::env::temp_dir(),
            program: "claude".into(),
            search_path: None,
            model: model.map(String::from),
            permission_mode: PermissionMode::AcceptEdits,
            env: vec![],
            env_remove: vec![],
        }
    }

    #[test]
    fn args_start_or_resume_with_the_mode_and_model() {
        let a = claude_args(&spec(false, None));
        assert!(a.windows(2).any(|w| w == ["--session-id", "sid"]));
        assert!(a.windows(2).any(|w| w == ["--permission-prompt-tool", "stdio"]));
        assert!(a.windows(2).any(|w| w == ["--permission-mode", "acceptEdits"]));
        assert!(!a.iter().any(|x| x == "--bare" || x == "--settings" || x == "--model"));
        let a = claude_args(&spec(true, Some("opus")));
        assert!(a.windows(2).any(|w| w == ["--resume", "sid"]));
        assert!(a.windows(2).any(|w| w == ["--model", "opus"]));
    }

    #[test]
    fn side_requests_fork_without_saving_or_tools() {
        let schema = pdt_core::conversation::draft::draft_schema();
        let a = claude_draft_args(&spec(true, Some("opus")), &schema);
        assert!(a.windows(2).any(|w| w == ["--resume", "sid"]));
        assert!(a.iter().any(|x| x == "--fork-session") && a.iter().any(|x| x == "--no-session-persistence"));
        assert!(a.windows(2).any(|w| w[0] == "--tools" && w[1].is_empty()));
        assert!(a.windows(2).any(|w| w == ["--output-format", "json"]));
        assert!(a.windows(2).any(|w| w == ["--model", "opus"]));
        assert!(!a.iter().any(|x| x == "--bare" || x == "--settings" || x == "--dangerously-skip-permissions"));
        // The schema passes through cmd.exe on Windows: nothing it would interpret.
        let s = &a[a.iter().position(|x| x == "--json-schema").unwrap() + 1];
        assert!(!s.chars().any(|c| "&|<>^%!\r\n".contains(c)), "{s}");
    }

    #[test]
    fn side_answers_are_the_structured_output() {
        let ok =
            r#"{"type":"result","subtype":"success","is_error":false,"result":"{}","structured_output":{"title":"t"}}"#;
        assert_eq!(draft_answer(ok).unwrap(), json!({"title": "t"}));
        let text = r#"{"type":"result","is_error":false,"result":"{\"title\":\"t\"}"}"#;
        assert_eq!(draft_answer(text).unwrap(), json!("{\"title\":\"t\"}"));
        let limited = r#"{"type":"result","is_error":true,"result":"You've hit your limit"}"#;
        assert!(matches!(draft_answer(limited), Err(AdapterError::Refused(w)) if w.contains("limit")));
        assert!(matches!(draft_answer("not json\n"), Err(AdapterError::Refused(_))));
    }

    #[test]
    fn capabilities_come_from_initialize() {
        let v = json!({"commands": [{"name": "compact", "description": "d", "argumentHint": ""}, {"bad": 1}],
                       "models": [{"value": "opus", "displayName": "Opus"}, {"value": "x"}]});
        let c = capabilities_from_initialize(&v, Some("2.1".into()));
        assert_eq!(c.commands.len(), 1);
        assert_eq!(c.models.len(), 2);
        assert_eq!(c.models[1].display_name, "x");
        assert_eq!(c.permission_modes.len(), 4);
    }
}
