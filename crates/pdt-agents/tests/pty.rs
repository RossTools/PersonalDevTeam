#[path = "../../../test-support/port_blocks.rs"]
mod port_blocks;

use pdt_agents::{Attachment, PtyError, PtyEvent, PtyManager, SpawnSpec};
use std::path::PathBuf;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(20);

/// Runs a one-off command: `cmd.exe /d /c <win>` on Windows, `/bin/sh -c <unix>` elsewhere.
fn oneshot_cmd(id: &str, win: &str, unix: &str) -> SpawnSpec {
    let (program, args) = if cfg!(windows) {
        (PathBuf::from("cmd.exe"), vec!["/d".into(), "/c".into(), win.into()])
    } else {
        (PathBuf::from("/bin/sh"), vec!["-c".into(), unix.into()])
    };
    spec(id, program, args)
}

/// An interactive shell that waits for input.
fn interactive(id: &str) -> SpawnSpec {
    if cfg!(windows) {
        spec(id, "cmd.exe".into(), vec!["/d".into(), "/q".into()])
    } else {
        spec(id, "/bin/sh".into(), vec![])
    }
}

fn spec(id: &str, program: PathBuf, args: Vec<String>) -> SpawnSpec {
    SpawnSpec {
        id: id.into(),
        program,
        args,
        cwd: std::env::temp_dir(),
        env_remove: vec![],
        env: vec![("PDT_TEST_VAR".into(), "from-spec".into())],
        cols: 80,
        rows: 24,
    }
}

/// Collects output until the exit event (or the timeout).
async fn drain(att: Attachment) -> (String, Option<i32>) {
    let mut out = att.scrollback;
    if let Some(code) = att.exited {
        return (String::from_utf8_lossy(&out).into_owned(), Some(code));
    }
    let mut events = att.events;
    let result = tokio::time::timeout(WAIT, async {
        loop {
            match events.recv().await {
                Ok(PtyEvent::Output(b)) => out.extend_from_slice(&b),
                Ok(PtyEvent::Exit(code)) => return Some(code),
                Err(_) => return None,
            }
        }
    })
    .await
    .unwrap_or(None);
    (String::from_utf8_lossy(&out).into_owned(), result)
}

#[tokio::test]
async fn output_and_exit_code() {
    let m = PtyManager::new();
    let exit = m.spawn(oneshot_cmd("a", "echo hello-pty& exit /b 3", "echo hello-pty; exit 3")).unwrap();
    let att = m.attach("a").unwrap();
    let (out, code) = drain(att).await;
    assert!(out.contains("hello-pty"), "{out:?}");
    assert_eq!(code, Some(3));
    assert_eq!(tokio::time::timeout(WAIT, exit).await.unwrap().unwrap(), 3);
    assert!(!m.is_running("a"));
}

#[tokio::test]
async fn env_from_spec_is_applied() {
    let m = PtyManager::new();
    m.spawn(oneshot_cmd("a", "echo %PDT_TEST_VAR%", "echo \"$PDT_TEST_VAR\"")).unwrap();
    let (out, _) = drain(m.attach("a").unwrap()).await;
    assert!(out.contains("from-spec"), "{out:?}");
}

#[tokio::test]
async fn input_reaches_the_process() {
    let m = PtyManager::new();
    m.spawn(interactive("a")).unwrap();
    let att = m.attach("a").unwrap();
    // The typed text contains "4000+2000+21"; only the evaluated output contains "6021".
    let line: &[u8] = if cfg!(windows) { b"set /a 4000+2000+21 & exit\r" } else { b"echo $((4000+2000+21)); exit\r" };
    m.write("a", line).unwrap();
    let (out, code) = drain(att).await;
    assert!(out.contains("6021"), "{out:?}");
    assert!(code.is_some());
}

#[tokio::test]
async fn late_attach_gets_scrollback_and_exit() {
    let m = PtyManager::new();
    let exit = m.spawn(oneshot_cmd("a", "echo late-attach-ok", "echo late-attach-ok")).unwrap();
    tokio::time::timeout(WAIT, exit).await.unwrap().unwrap();
    let att = m.attach("a").unwrap();
    assert_eq!(att.exited, Some(0));
    assert!(String::from_utf8_lossy(&att.scrollback).contains("late-attach-ok"));
}

#[tokio::test]
async fn kill_ends_the_process() {
    let m = PtyManager::new();
    let exit = m.spawn(interactive("a")).unwrap();
    assert!(m.is_running("a"));
    m.resize("a", 100, 30).unwrap();
    m.kill("a").unwrap();
    tokio::time::timeout(WAIT, exit).await.expect("exit after kill").unwrap();
    assert!(matches!(m.write("a", b"x"), Err(PtyError::NotRunning(_))));
    assert!(matches!(m.kill("a"), Err(PtyError::NotRunning(_))));
}

/// A run's server is a grandchild (`cmd.exe` → node, `sh` → node): killing the terminal must end it too, or it keeps
/// its port (Review Focus 2).
#[tokio::test]
async fn kill_ends_the_whole_process_tree() {
    // A claimed port, not a random free one: another test's socket could take that before node listens on it.
    let port = port_blocks::claim(31000, 100, 1);
    // node starts the server as its own child, like dev servers do. On Windows that child is detached from the
    // console, so closing the pseudo console does not end it: only a tree kill does.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("server.js"),
        "if (process.argv[2] === 'child') {\n\
         require('net').createServer().listen(Number(process.env.P), '127.0.0.1');\n\
         } else {\n\
         require('child_process').spawn(process.execPath, [__filename, 'child'], \
         { detached: process.platform === 'win32', stdio: 'ignore', windowsHide: true });\n\
         setInterval(() => {}, 1000);\n\
         }\n",
    )
    .unwrap();
    let m = PtyManager::new();
    // The trailing echo keeps the shell from exec'ing node in its place.
    let mut s = oneshot_cmd("a", "node server.js & echo after", "node server.js; echo after");
    s.cwd = dir.path().to_path_buf();
    s.env.push(("P".into(), port.to_string()));
    let exit = m.spawn(s).unwrap();
    let deadline = tokio::time::Instant::now() + WAIT;
    while tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err() {
        assert!(tokio::time::Instant::now() < deadline, "the server never listened");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    m.kill("a").unwrap();
    tokio::time::timeout(WAIT, exit).await.expect("exit after kill").unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
        assert!(tokio::time::Instant::now() < deadline, "the grandchild server still holds port {port}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn ids_are_checked() {
    let m = PtyManager::new();
    m.spawn(interactive("a")).unwrap();
    assert!(matches!(m.spawn(interactive("a")), Err(PtyError::AlreadyExists(_))));
    assert!(matches!(m.attach("nope"), Err(PtyError::NotFound(_))));
    assert!(matches!(m.write("nope", b"x"), Err(PtyError::NotFound(_))));
    m.remove("a");
    assert!(matches!(m.attach("a"), Err(PtyError::NotFound(_))));
}

#[tokio::test]
async fn shutdown_kills_everything() {
    let m = PtyManager::new();
    let e1 = m.spawn(interactive("a")).unwrap();
    let e2 = m.spawn(interactive("b")).unwrap();
    tokio::time::timeout(WAIT, m.shutdown()).await.unwrap();
    assert!(!m.is_running("a") && !m.is_running("b"));
    e1.await.unwrap();
    e2.await.unwrap();
}

#[tokio::test]
async fn missing_working_directory_is_a_spawn_error() {
    let m = PtyManager::new();
    let mut s = interactive("a");
    s.cwd = std::env::temp_dir().join("pdt-no-such-dir-xyz");
    assert!(matches!(m.spawn(s), Err(PtyError::Spawn(_))));
    assert!(matches!(m.attach("a"), Err(PtyError::NotFound(_))));
}

#[tokio::test]
async fn spawn_after_shutdown_is_refused_and_starts_no_process() {
    let m = PtyManager::new();
    tokio::time::timeout(WAIT, m.shutdown()).await.unwrap();
    let marker = std::env::temp_dir().join(format!("pdt-no-spawn-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let (win, unix) = (format!("echo x> \"{}\"", marker.display()), format!("echo x > '{}'", marker.display()));
    let r = m.spawn(oneshot_cmd("late", &win, &unix));
    assert!(matches!(r, Err(PtyError::ShuttingDown)), "{r:?}");
    assert!(matches!(m.attach("late"), Err(PtyError::NotFound(_))), "nothing was registered");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!marker.exists(), "the command must not have run");
}

/// Input to a process that has gone is refused, not queued for a writer that leads nowhere (PDT-129): as soon as its
/// side of the terminal closes, before its exit is reported.
#[tokio::test]
async fn input_is_refused_once_the_process_is_gone() {
    let m = PtyManager::new();
    let exit = m.spawn(interactive("a")).unwrap();
    assert!(m.accepts_input("a"));
    m.write("a", b"exit\r").unwrap();
    tokio::time::timeout(WAIT, async {
        while m.accepts_input("a") {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("its input closes");
    assert!(matches!(m.write("a", b"x"), Err(PtyError::NotRunning(_))));
    tokio::time::timeout(WAIT, exit).await.unwrap().unwrap();
    assert!(!m.accepts_input("a") && !m.accepts_input("nobody"));
}
