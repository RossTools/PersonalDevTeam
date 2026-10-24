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
    #[error("the terminal manager is shutting down")]
    ShuttingDown,
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
    /// A process in a PTY, or None for a followed log ([`PtyManager::follow_log`]).
    proc: Option<Proc>,
    follow: Follow,
}

struct Proc {
    input: std_mpsc::Sender<Vec<u8>>,
    /// The child's side is gone (its output ended, or a write failed), though its exit may not be reported yet: input
    /// sent now would go nowhere (PDT-129).
    closed: std::sync::atomic::AtomicBool,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: Option<u32>,
}

/// A followed log's controls.
#[derive(Default)]
struct Follow {
    /// Stop following (the entry is forgotten, or pdtd is shutting down).
    stop: std::sync::atomic::AtomicBool,
    /// The process behind the log ended: read what is left, then report this exit.
    end: Mutex<Option<i32>>,
}

/// How long a process gets to act on SIGHUP before it is killed outright.
#[cfg(unix)]
const HANGUP_GRACE: Duration = Duration::from_millis(1500);

impl Handle {
    fn exited(&self) -> bool {
        self.state.lock().unwrap().exited.is_some()
    }

    /// Asks the OS to end the process and everything it started; the waiter thread reports the exit.
    ///
    /// - Windows: `taskkill /T /F` ends the whole tree first, while the root is still alive (taskkill finds the tree
    ///   through it), then portable-pty's killer ends the root if it is somehow still there. A run's
    ///   `cmd.exe /c script.cmd` (or an npm shim) otherwise leaves its server running, holding ports and files.
    /// - unix: SIGHUP to the child's process group, which a shell can sit on (macOS's /bin/sh handles it only once
    ///   its read returns), so after `HANGUP_GRACE` the group (and the child) get SIGKILL. The group is killed even
    ///   when the shell has already exited, since its children may still be in it.
    ///
    /// A followed log has no process of its own: killing it does nothing.
    fn kill(self: &Arc<Self>) -> std::io::Result<()> {
        let Some(p) = &self.proc else { return Ok(()) };
        #[cfg(windows)]
        if let Some(pid) = p.pid {
            kill_tree(pid);
        }
        let result = p.killer.lock().unwrap().kill();
        #[cfg(unix)]
        if let Some(pid) = p.pid.and_then(|p| i32::try_from(p).ok()) {
            // The child is a session leader, so its pid is also its process group id.
            unsafe {
                libc::kill(-pid, libc::SIGHUP);
            }
            let h = self.clone();
            std::thread::spawn(move || {
                std::thread::sleep(HANGUP_GRACE);
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                    if !h.exited() {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
            });
        }
        if cfg!(windows) {
            // portable-pty 0.9.0's Windows killer inverts TerminateProcess's result (Err on success, Ok on
            // failure), so the result says nothing (and after the tree kill the root is usually gone already).
            return Ok(());
        }
        result
    }
}

/// Ends `pid` and every process it started (`taskkill /T /F`), waiting for taskkill to finish.
#[cfg(windows)]
pub(crate) fn kill_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // The system copy, not whatever `taskkill` the PATH would find first.
    let program = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("taskkill.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("taskkill.exe"));
    let status = std::process::Command::new(program)
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    if let Err(e) = status {
        tracing::warn!(pid, error = %e, "could not run taskkill");
    }
}

#[derive(Clone, Default)]
pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<String, Arc<Handle>>>>,
    /// Set by `shutdown`, under the `sessions` lock, so no process can start after the snapshot it kills.
    shut_down: Arc<std::sync::atomic::AtomicBool>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts the process. The receiver resolves with the exit code once it has ended.
    pub fn spawn(&self, spec: SpawnSpec) -> Result<oneshot::Receiver<i32>, PtyError> {
        let mut sessions = self.sessions.lock().unwrap();
        if self.shut_down.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(PtyError::ShuttingDown);
        }
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
        let pid = child.process_id();
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
            proc: Some(Proc {
                input: input_tx,
                closed: Default::default(),
                master: Mutex::new(Some(pair.master)),
                killer: Mutex::new(killer),
                pid,
            }),
            follow: Follow::default(),
        });

        let h = handle.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut first = cfg!(windows);
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        // unix: every slave fd closed (EIO), so the child exited; Windows: the master closed.
                        if let Some(p) = &h.proc {
                            p.closed.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        break;
                    }
                    Ok(n) => {
                        let mut chunk = &buf[..n];
                        if std::mem::take(&mut first)
                            && let Some(rest) = strip_cursor_query(chunk)
                        {
                            if let Some(p) = &h.proc {
                                let _ = p.input.send(CURSOR_REPLY.to_vec());
                            }
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

        let h = handle.clone();
        std::thread::spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
                    if let Some(p) = &h.proc {
                        p.closed.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
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
            if let Some(p) = &h.proc {
                p.master.lock().unwrap().take();
            }
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

    /// Fails once the process is gone, even before its exit is reported (its output ended or a write failed).
    pub fn write(&self, id: &str, bytes: &[u8]) -> Result<(), PtyError> {
        let h = self.get(id)?;
        let Some(p) = h.proc.as_ref().filter(|p| !h.exited() && !p.closed.load(std::sync::atomic::Ordering::SeqCst))
        else {
            return Err(PtyError::NotRunning(id.to_string()));
        };
        p.input.send(bytes.to_vec()).map_err(|_| PtyError::NotRunning(id.to_string()))
    }

    /// A followed log has no size: resizing it does nothing.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), PtyError> {
        let h = self.get(id)?;
        let Some(p) = &h.proc else { return Ok(()) };
        let master = p.master.lock().unwrap();
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

    /// Whether input written now can reach the process: it runs and its side of the terminal is still open.
    pub fn accepts_input(&self, id: &str) -> bool {
        self.get(id).is_ok_and(|h| {
            !h.exited() && h.proc.as_ref().is_some_and(|p| !p.closed.load(std::sync::atomic::Ordering::SeqCst))
        })
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.get(id).is_ok_and(|h| !h.exited())
    }

    /// Forgets a session, killing it first if it is still running (or stops following its log).
    pub fn remove(&self, id: &str) {
        if let Some(h) = self.sessions.lock().unwrap().remove(id) {
            h.follow.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            if !h.exited() {
                let _ = h.kill();
            }
        }
    }

    /// Shows the file at `log` as terminal `id`, read-only: its last `SCROLLBACK_BYTES` (from a line start), then
    /// whatever is appended, until [`PtyManager::end_log`]. The file may not exist yet. Bare line feeds become CR LF,
    /// as a program writing to a file (not a terminal) leaves them bare.
    pub fn follow_log(&self, id: &str, log: PathBuf) -> Result<(), PtyError> {
        let mut sessions = self.sessions.lock().unwrap();
        if self.shut_down.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(PtyError::ShuttingDown);
        }
        if sessions.contains_key(id) {
            return Err(PtyError::AlreadyExists(id.to_string()));
        }
        let (events, _) = broadcast::channel(1024);
        let handle = Arc::new(Handle {
            state: Mutex::new(State { scrollback: Scrollback::new(SCROLLBACK_BYTES), exited: None }),
            events,
            proc: None,
            follow: Follow::default(),
        });
        let h = handle.clone();
        std::thread::spawn(move || follow(&h, &log));
        sessions.insert(id.to_string(), handle);
        Ok(())
    }

    /// The process behind followed log `id` ended with `code` (-1 when unknown): the log's last output is shown,
    /// then the exit.
    pub fn end_log(&self, id: &str, code: Option<i32>) {
        if let Ok(h) = self.get(id) {
            *h.follow.end.lock().unwrap() = Some(code.unwrap_or(-1));
        }
    }

    /// Kills every running session and waits (up to 5 s) for them to report their exit. Afterwards `spawn` refuses
    /// to start anything. Followed logs just stop: the processes behind them carry on.
    pub async fn shutdown(&self) {
        let handles: Vec<Arc<Handle>> = {
            let sessions = self.sessions.lock().unwrap();
            self.shut_down.store(true, std::sync::atomic::Ordering::SeqCst);
            sessions.values().cloned().collect()
        };
        for h in &handles {
            h.follow.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let handles: Vec<Arc<Handle>> = handles.into_iter().filter(|h| h.proc.is_some()).collect();
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

/// How often a followed log is checked for new output.
const FOLLOW_EVERY: Duration = Duration::from_millis(200);

/// `chunk` with every line feed not preceded by a carriage return made CR LF. `last_cr` carries the previous
/// chunk's last byte across calls.
fn crlf(chunk: &[u8], last_cr: &mut bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(chunk.len() + chunk.len() / 16);
    for &b in chunk {
        if b == b'\n' && !*last_cr {
            out.push(b'\r');
        }
        out.push(b);
        *last_cr = b == b'\r';
    }
    out
}

/// The follower thread of [`PtyManager::follow_log`].
fn follow(h: &Handle, path: &std::path::Path) {
    use std::io::{Seek, SeekFrom};
    use std::sync::atomic::Ordering;
    let mut file: Option<std::fs::File> = None;
    let mut pos: u64 = 0;
    let mut last_cr = false;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if h.follow.stop.load(Ordering::SeqCst) {
            return;
        }
        // Read before draining, so everything written before the end is shown.
        let end = *h.follow.end.lock().unwrap();
        if file.is_none()
            && let Ok(mut f) = std::fs::File::open(path)
        {
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            if len > SCROLLBACK_BYTES as u64 {
                pos = len - SCROLLBACK_BYTES as u64;
                let _ = f.seek(SeekFrom::Start(pos));
                // Start at a line, not part way through one (or through a UTF-8 sequence).
                let mut skip = [0u8; 1];
                while let Ok(1) = f.read(&mut skip) {
                    pos += 1;
                    if skip[0] == b'\n' {
                        break;
                    }
                }
            }
            file = Some(f);
        }
        if let Some(f) = file.as_mut() {
            // A log truncated under us (the command started again) is read from its start.
            if f.metadata().is_ok_and(|m| m.len() < pos) {
                pos = 0;
                let _ = f.seek(SeekFrom::Start(0));
            }
            loop {
                match f.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        pos += n as u64;
                        let bytes = crlf(&buf[..n], &mut last_cr);
                        let mut st = h.state.lock().unwrap();
                        st.scrollback.push(&bytes);
                        let _ = h.events.send(PtyEvent::Output(Arc::from(bytes)));
                    }
                }
            }
        }
        if let Some(code) = end {
            let mut st = h.state.lock().unwrap();
            st.exited = Some(code);
            let _ = h.events.send(PtyEvent::Exit(code));
            return;
        }
        std::thread::sleep(FOLLOW_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::{PtyEvent, PtyManager, Scrollback, crlf, strip_cursor_query};
    use std::time::Duration;

    #[test]
    fn bare_line_feeds_become_crlf() {
        let mut cr = false;
        assert_eq!(crlf(b"a\nb\r\nc", &mut cr), b"a\r\nb\r\nc");
        assert_eq!(crlf(b"x\r", &mut cr), b"x\r");
        assert_eq!(crlf(b"\ny\n", &mut cr), b"\ny\r\n", "a CR at the end of the last chunk counts");
    }

    #[tokio::test]
    async fn a_followed_log_shows_its_tail_then_new_output_then_the_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("command.log");
        let m = PtyManager::new();
        m.follow_log("l", log.clone()).unwrap();
        // The file appears after following began.
        std::fs::write(&log, "one\n").unwrap();
        let mut att = loop {
            let att = m.attach("l").unwrap();
            if !att.scrollback.is_empty() {
                break att;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert_eq!(att.scrollback, b"one\r\n");
        assert!(m.write("l", b"x").is_err(), "no input");
        assert!(m.resize("l", 80, 24).is_ok() && m.kill("l").is_ok(), "no size, nothing to kill");
        assert!(m.attach("l").unwrap().exited.is_none(), "kill leaves it following");
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"two\n").unwrap();
        m.end_log("l", Some(4));
        let mut seen = Vec::new();
        let code = loop {
            match tokio::time::timeout(Duration::from_secs(5), att.events.recv()).await.unwrap().unwrap() {
                PtyEvent::Output(b) => seen.extend_from_slice(&b),
                PtyEvent::Exit(c) => break c,
            }
        };
        assert_eq!((seen.as_slice(), code), (&b"two\r\n"[..], 4), "what was written before the end, then the exit");
        m.shutdown().await;
    }

    #[tokio::test]
    async fn a_long_log_starts_at_a_line_within_the_scrollback() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("command.log");
        let line = "x".repeat(99) + "\n";
        std::fs::write(&log, format!("{}last\n", line.repeat(super::SCROLLBACK_BYTES / 100 + 50))).unwrap();
        let m = PtyManager::new();
        m.follow_log("l", log).unwrap();
        let att = loop {
            let att = m.attach("l").unwrap();
            if att.scrollback.ends_with(b"last\r\n") {
                break att;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert!(att.scrollback.starts_with(b"xxx"), "starts at a line");
        assert!(att.scrollback.len() <= super::SCROLLBACK_BYTES);
        m.shutdown().await;
    }

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
