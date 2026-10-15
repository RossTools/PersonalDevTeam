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
