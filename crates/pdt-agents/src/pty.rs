//! PTY-backed terminal sessions owned by pdtd. Each session keeps a scrollback buffer and broadcasts its output
//! to any number of attached clients, so terminals outlive the windows that show them.

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};

pub const SCROLLBACK_BYTES: usize = 512 * 1024;
/// After the child exits, give the reader a moment to flush the last output before announcing the exit.
const FLUSH_GRACE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub id: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Removed from the inherited environment before `env` is applied.
    pub env_remove: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone)]
pub enum PtyEvent {
    Output(Arc<[u8]>),
    Exit(i32),
}

/// A consistent view for a new client: everything so far, then every later event.
pub struct Attachment {
    pub scrollback: Vec<u8>,
    pub exited: Option<i32>,
    pub events: broadcast::Receiver<PtyEvent>,
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("{0} was not found on PATH")]
    ProgramNotFound(String),
    #[error("failed to start the terminal: {0}")]
    Spawn(String),
    #[error("terminal session {0} does not exist")]
    NotFound(String),
    #[error("terminal session {0} already exists")]
    AlreadyExists(String),
    #[error("terminal session {0} is not running")]
    NotRunning(String),
    #[error("terminal I/O failed: {0}")]
    Io(String),
}

pub(crate) struct Scrollback {
    buf: VecDeque<u8>,
    cap: usize,
}

impl Scrollback {
    pub(crate) fn new(cap: usize) -> Self {
        Self { buf: VecDeque::with_capacity(cap.min(64 * 1024)), cap }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        let bytes = if bytes.len() > self.cap { &bytes[bytes.len() - self.cap..] } else { bytes };
        let overflow = (self.buf.len() + bytes.len()).saturating_sub(self.cap);
        self.buf.drain(..overflow);
        self.buf.extend(bytes);
    }

    pub(crate) fn snapshot(&self) -> Vec<u8> {
        self.buf.iter().copied().collect()
    }
}

/// Device Status Report: "where is the cursor?".
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
/// Our answer to the query: row 1, column 1.
const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

/// ConPTY (opened by portable-pty with `PSEUDOCONSOLE_INHERIT_CURSOR`) starts every session with a cursor query
/// and holds back all output until it is answered. A session must run with no client attached, and a query
/// replayed from scrollback would make a later client answer it a second time, so the manager answers it itself
/// and keeps it out of the output. Returns the rest of `chunk` if it starts with the query.
fn strip_cursor_query(chunk: &[u8]) -> Option<&[u8]> {
    chunk.strip_prefix(CURSOR_QUERY)
}

struct State {
    scrollback: Scrollback,
    exited: Option<i32>,
}

struct Handle {
    state: Mutex<State>,
    events: broadcast::Sender<PtyEvent>,
    input: std_mpsc::Sender<Vec<u8>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

impl Handle {
    fn exited(&self) -> bool {
        self.state.lock().unwrap().exited.is_some()
    }

    /// Asks the OS to end the process; the waiter thread reports the exit.
    fn kill(&self) -> std::io::Result<()> {
        let result = self.killer.lock().unwrap().kill();
        if cfg!(windows) {
            // portable-pty 0.9.0's Windows killer inverts TerminateProcess's result (Err on success, Ok on
            // failure), so the result says nothing. Killing our own child only fails if it already exited.
            return Ok(());
        }
        result
    }
}

#[derive(Clone, Default)]
pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<String, Arc<Handle>>>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts the process. The receiver resolves with the exit code once it has ended.
    pub fn spawn(&self, spec: SpawnSpec) -> Result<oneshot::Receiver<i32>, PtyError> {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.contains_key(&spec.id) {
            return Err(PtyError::AlreadyExists(spec.id));
        }
        if !spec.cwd.is_dir() {
            return Err(PtyError::Spawn(format!("working directory does not exist: {}", spec.cwd.display())));
        }
        let spawn_err = |e: &dyn std::fmt::Display| PtyError::Spawn(e.to_string());
        let size = PtySize { rows: spec.rows, cols: spec.cols, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|e| spawn_err(&e))?;
        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        for key in &spec.env_remove {
            cmd.env_remove(key);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| spawn_err(&e))?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let io = pair.master.try_clone_reader().and_then(|r| Ok((r, pair.master.take_writer()?)));
        let (mut reader, mut writer) = match io {
            Ok(rw) => rw,
            Err(e) => {
                let _ = child.kill();
                return Err(spawn_err(&e));
            }
        };

        let (events, _) = broadcast::channel(1024);
        let (input_tx, input_rx) = std_mpsc::channel::<Vec<u8>>();
        let handle = Arc::new(Handle {
            state: Mutex::new(State { scrollback: Scrollback::new(SCROLLBACK_BYTES), exited: None }),
            events,
            input: input_tx,
            master: Mutex::new(Some(pair.master)),
            killer: Mutex::new(killer),
        });

        let h = handle.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut first = cfg!(windows);
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut chunk = &buf[..n];
                        if std::mem::take(&mut first)
                            && let Some(rest) = strip_cursor_query(chunk)
                        {
                            let _ = h.input.send(CURSOR_REPLY.to_vec());
                            chunk = rest;
                        }
                        if chunk.is_empty() {
                            continue;
                        }
                        // Append and broadcast under one lock so `attach` sees each byte exactly once.
                        let mut st = h.state.lock().unwrap();
                        st.scrollback.push(chunk);
                        let _ = h.events.send(PtyEvent::Output(Arc::from(chunk)));
                    }
                }
            }
        });

        std::thread::spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
        });

        let (exit_tx, exit_rx) = oneshot::channel();
        let h = handle.clone();
        let id = spec.id.clone();
        std::thread::spawn(move || {
            let code = child.wait().map(|s| s.exit_code() as i32).unwrap_or(-1);
            std::thread::sleep(FLUSH_GRACE);
            {
                let mut st = h.state.lock().unwrap();
                st.exited = Some(code);
                let _ = h.events.send(PtyEvent::Exit(code));
            }
            // Closing the master ends the reader (on Windows, ConPTY keeps the pipe open until then).
            h.master.lock().unwrap().take();
            tracing::debug!(session = %id, code, "terminal exited");
            let _ = exit_tx.send(code);
        });

        sessions.insert(spec.id, handle);
        Ok(exit_rx)
    }

    fn get(&self, id: &str) -> Result<Arc<Handle>, PtyError> {
        self.sessions.lock().unwrap().get(id).cloned().ok_or_else(|| PtyError::NotFound(id.to_string()))
    }

    pub fn attach(&self, id: &str) -> Result<Attachment, PtyError> {
        let h = self.get(id)?;
        let st = h.state.lock().unwrap();
        Ok(Attachment { scrollback: st.scrollback.snapshot(), exited: st.exited, events: h.events.subscribe() })
    }

    pub fn write(&self, id: &str, bytes: &[u8]) -> Result<(), PtyError> {
        let h = self.get(id)?;
        if h.exited() {
            return Err(PtyError::NotRunning(id.to_string()));
        }
        h.input.send(bytes.to_vec()).map_err(|_| PtyError::NotRunning(id.to_string()))
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), PtyError> {
        let h = self.get(id)?;
        let master = h.master.lock().unwrap();
        let master = master.as_ref().ok_or_else(|| PtyError::NotRunning(id.to_string()))?;
        master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| PtyError::Io(e.to_string()))
    }

    pub fn kill(&self, id: &str) -> Result<(), PtyError> {
        let h = self.get(id)?;
        if h.exited() {
            return Err(PtyError::NotRunning(id.to_string()));
        }
        h.kill().map_err(|e| PtyError::Io(e.to_string()))
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.get(id).is_ok_and(|h| !h.exited())
    }

    /// Forgets a session, killing it first if it is still running.
    pub fn remove(&self, id: &str) {
        if let Some(h) = self.sessions.lock().unwrap().remove(id)
            && !h.exited()
        {
            let _ = h.kill();
        }
    }

    /// Kills every running session and waits (up to 5 s) for them to report their exit.
    pub async fn shutdown(&self) {
        let handles: Vec<Arc<Handle>> = self.sessions.lock().unwrap().values().cloned().collect();
        for h in &handles {
            if !h.exited() {
                let _ = h.kill();
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while handles.iter().any(|h| !h.exited()) && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Scrollback, strip_cursor_query};

    #[test]
    fn leading_cursor_query_is_stripped() {
        assert_eq!(strip_cursor_query(b"\x1b[6nhello"), Some(&b"hello"[..]));
        assert_eq!(strip_cursor_query(b"\x1b[6n"), Some(&b""[..]));
        assert_eq!(strip_cursor_query(b"hello\x1b[6n"), None);
        assert_eq!(strip_cursor_query(b"\x1b[6"), None);
    }

    #[test]
    fn scrollback_keeps_the_tail() {
        let mut s = Scrollback::new(8);
        s.push(b"abc");
        s.push(b"defgh");
        assert_eq!(s.snapshot(), b"abcdefgh");
        s.push(b"ij");
        assert_eq!(s.snapshot(), b"cdefghij");
        s.push(b"0123456789");
        assert_eq!(s.snapshot(), b"23456789");
    }
}
