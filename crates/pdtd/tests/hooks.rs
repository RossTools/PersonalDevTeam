//! `pdtd hook …`: the agent hook subcommand, alone and against a real daemon. Never touches the stable instance.

mod common;
use common::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hook(args: &[&str], env: &[(&str, &str)], stdin: &str) -> (std::process::Output, Duration) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pdtd"));
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    let mut child = cmd
        .arg("hook")
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let t = Instant::now();
    // The hook may exit without reading its input.
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    let out = child.wait_with_output().unwrap();
    (out, t.elapsed())
}

#[test]
fn hook_is_silent_and_exits_0_when_pdtd_is_down() {
    let dead = free_port().to_string();
    for (args, env) in [
        (vec!["claude", "UserPromptSubmit"], vec![("PDT_SESSION_ID", "s"), ("PDT_HOOK_PORT", dead.as_str())]),
        (vec!["claude", "Stop"], vec![]),
        (
            vec!["codex-notify", r#"{"type":"agent-turn-complete"}"#],
            vec![("PDT_SESSION_ID", "s"), ("PDT_HOOK_PORT", dead.as_str())],
        ),
        (vec!["nonsense"], vec![]),
        (vec![], vec![]),
    ] {
        let (out, took) = hook(&args, &env, r#"{"prompt":"hello"}"#);
        assert!(out.status.success(), "{args:?}: {out:?}");
        assert!(out.stdout.is_empty(), "{args:?} printed {:?}", String::from_utf8_lossy(&out.stdout));
        assert!(took < Duration::from_secs(5), "{args:?} took {took:?}");
    }
}

#[test]
fn codex_notify_runs_the_users_chained_notifier() {
    let tmp = tempfile::tempdir().unwrap();
    let out_file = tmp.path().join("chained.txt");
    let script = "require('fs').writeFileSync(process.argv[1], process.argv[2])";
    let file = out_file.to_string_lossy();
    let payload = r#"{"type":"agent-turn-complete","thread-id":"t"}"#;
    let (out, _) = hook(&["codex-notify", "--chain", "node", "-e", script, &file, "--", payload], &[], "");
    assert!(out.status.success());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !out_file.exists() {
        assert!(Instant::now() < deadline, "the chained notifier never ran");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(std::fs::read_to_string(&out_file).unwrap(), payload);
}

/// A stand-in for `claude` that records its arguments and PDT_SESSION_ID, then waits.
fn fake_claude(dir: &Path) -> PathBuf {
    if cfg!(windows) {
        let p = dir.join("claude.cmd");
        std::fs::write(
            &p,
            "@echo off\r\necho %*> \"%~dp0args.txt\"\r\necho %PDT_SESSION_ID%> \"%~dp0session.txt\"\r\nping -n 120 127.0.0.1 >nul\r\n",
        )
        .unwrap();
        p
    } else {
        let p = dir.join("claude");
        std::fs::write(
            &p,
            "#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/args.txt\"\necho \"$PDT_SESSION_ID\" > \"$(dirname \"$0\")/session.txt\"\nexec sleep 120\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }
}

#[test]
fn hook_events_change_a_sessions_activity() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = fake_claude(&bin);
    let data = tmp.path().join("data");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success());
    let port = free_port();
    let _d = start_with(port, &data, &[("PDT_CLAUDE_PROGRAM", &claude.to_string_lossy())]);
    let (_, p) = http(
        port,
        "POST",
        "/api/projects",
        Some(&serde_json::json!({"name": "P", "repoPath": repo.to_string_lossy()}).to_string()),
    );
    let body =
        serde_json::json!({"projectId": p["id"], "kind": "agent_tui", "agent": "claude", "cols": 80, "rows": 24});
    let (s, sess) = http(port, "POST", "/api/sessions", Some(&body.to_string()));
    assert_eq!(s, 201, "{sess}");
    let id = sess["id"].as_str().unwrap().to_string();

    let deadline = Instant::now() + Duration::from_secs(20);
    while !bin.join("session.txt").exists() {
        assert!(Instant::now() < deadline, "the fake claude never started");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(std::fs::read_to_string(bin.join("session.txt")).unwrap().trim(), id, "PDT_SESSION_ID is set");
    let args = std::fs::read_to_string(bin.join("args.txt")).unwrap();
    assert!(args.contains("--settings"), "{args}");
    let settings_file = data.join("sessions").join(&id).join("claude-settings.json");
    let settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&settings_file).unwrap()).unwrap();
    let command = settings["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
    assert!(Path::new(command).is_file(), "the hook runs this pdtd: {command}");

    let port_s = port.to_string();
    let env = [("PDT_SESSION_ID", id.as_str()), ("PDT_HOOK_PORT", port_s.as_str())];
    let activity = || http(port, "GET", &format!("/api/sessions/{id}"), None).1;
    assert_eq!(activity()["activity"], "idle");
    assert!(hook(&["claude", "UserPromptSubmit"], &env, r#"{"prompt":"x"}"#).0.status.success());
    assert_eq!(activity()["activity"], "working");
    hook(&["claude", "Stop"], &env, "{}");
    let s = activity();
    assert_eq!((s["activity"].as_str(), s["unseen"].as_bool()), (Some("idle"), Some(true)));
    assert_eq!(http(port, "POST", &format!("/api/sessions/{id}/view"), None).0, 204);
    assert_eq!(activity()["unseen"], false);
    hook(&["claude", "Notification"], &env, r#"{"notification_type":"permission_prompt","message":"m"}"#);
    assert_eq!(activity()["activity"], "needs_you");
}
