//! Runs and tests on a remote instance (PDT-634): an owner (`studio`) and a remote instance (`win-box`) on loopback. A
//! `windows` attempt's worktree is on the remote, so its runs and tests run there: the owner offers only those for
//! the remote's system, approves their commands, and shows their state and output; a web run's link is the remote's
//! own, mapped through its `tailscale serve` to its owner-checked proxy.

use crate::common::wait::wait_until;
use crate::common::{approve_commands, call, call_as_ui};
use crate::port_blocks;
use crate::remote_attempts::{Pair, paired_with, ticket};
use crate::runs::{http_get_as, proxied, wait_state};
use axum::http::StatusCode;
use pdt_core::run::{RunStep, pty_id};
use pdt_tailscale::testing::RecordingTailscale;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

/// A web server in node (CI has node for pnpm) that answers with the run's branch; a test that prints and fails; and
/// a test for another system than this one.
const CONFIG: &str = r#"
[[run]]
name = "web"
command = "node -e \"require('http').createServer((q, s) => s.end(process.env.PDT_RUN_BRANCH)).listen(Number(process.env.PORT), '127.0.0.1')\""
web = true

[[test]]
name = "hello"
setup = "node -e \"console.log('setting up')\""
command = "node -e \"console.log('hello from ' + process.env.PDT_RUN_BRANCH); process.exit(3)\""

[[test]]
name = "elsewhere"
command = "node -e \"0\""
os = "OTHER_OS"
"#;

fn other_os() -> &'static str {
    if std::env::consts::OS == "windows" { "linux" } else { "windows" }
}

/// The remote's Tailscale: remote access works there, as `win-box.t.ts.net`.
fn tailscale() -> Arc<RecordingTailscale> {
    Arc::new(RecordingTailscale::new(Ok(pdt_tailscale::TsStatus {
        dns_name: "win-box.t.ts.net".into(),
        self_login: None,
        https_enabled: true,
    })))
}

/// A `windows` ticket's attempt, on the remote, with the run configuration in its worktree there: (worktree id,
/// branch, its folder there).
async fn remote_attempt(p: &Pair) -> (String, String, PathBuf) {
    let t = ticket(p, "Client", &["windows"]).await;
    let start = json!({"agent": "claude", "noSession": true});
    let (s, started) = call(&p.owner.app, "POST", &format!("/api/tickets/{t}/start"), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let branch = started["attempt"]["branch"].as_str().unwrap().to_string();
    let (_, list) = call(&p.owner.app, "GET", &format!("/api/projects/{}/worktrees", p.project), None).await;
    let wt = list.as_array().unwrap().iter().find(|w| w["branch"] == branch.as_str()).unwrap().clone();
    assert_eq!(wt["instance"]["name"], "win-box", "{wt}");
    let path = PathBuf::from(wt["path"].as_str().unwrap());
    assert!(path.starts_with(&p.checkout), "{wt}");
    std::fs::create_dir_all(path.join(".pdt")).unwrap();
    std::fs::write(path.join(".pdt").join("project.toml"), CONFIG.replace("OTHER_OS", other_os())).unwrap();
    (wt["id"].as_str().unwrap().to_string(), branch, path)
}

async fn start_run(p: &Pair, worktree: &str, name: &str, kind: &str) -> (StatusCode, Value) {
    let body = json!({"worktreeId": worktree, "name": name, "kind": kind});
    call_as_ui(&p.owner.app, "POST", &format!("/api/projects/{}/runs", p.project), Some(body)).await
}

/// What the owner's terminal for a run's step shows.
async fn shows(p: &Pair, run: &str, step: RunStep, text: &str) {
    wait_until(&format!("the run's {} terminal to show {text:?}", step.as_str()), async || {
        let shown = match p.owner.state.ptys.attach(&pty_id(run, step)) {
            Ok(a) => String::from_utf8_lossy(&a.scrollback).into_owned(),
            Err(e) => e.to_string(),
        };
        if shown.contains(text) { Ok(()) } else { Err(shown) }
    })
    .await;
}

#[tokio::test]
async fn a_remote_attempts_runs_and_tests_run_on_the_remote_instance() {
    let fake = tailscale();
    let p = paired_with(Some(fake.clone())).await;
    let ports = port_blocks::claim(10000, 250, 40);
    let settings = json!({
        "runPorts": {"start": ports, "end": ports + 39},
        "remoteAccess": true,
        "ownerLogin": "me@example.com",
    });
    let (s, v) = call(&p.remote.app, "PUT", "/api/host/settings", Some(settings)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (wt, branch, path) = remote_attempt(&p).await;

    // The owner offers what the worktree there configures for the remote's system.
    let (s, v) =
        call(&p.owner.app, "GET", &format!("/api/projects/{}/run-config?worktreeId={wt}", p.project), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["file"].as_str().unwrap(), CONFIG.replace("OTHER_OS", other_os()));
    let names = |k: &str| v["effective"][k].as_array().unwrap().iter().map(|d| d["name"].clone()).collect::<Vec<_>>();
    assert_eq!((names("runs"), names("tests")), (vec![json!("web")], vec![json!("hello")]));
    let (s, v) = start_run(&p, &wt, "elsewhere", "test").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["message"].as_str().unwrap().contains(&format!("runs only on {}", other_os())), "{v}");

    // Its commands wait on the user's approval, like any.
    let (s, v) = start_run(&p, &wt, "hello", "test").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("untrusted_commands")), "{v}");
    approve_commands(&p.owner.app, &p.project, Some(&wt)).await;

    // A test runs there, and fails there: its state, exit code and output are the owner's to show.
    let (s, run) = start_run(&p, &wt, "hello", "test").await;
    assert_eq!(s, StatusCode::CREATED, "{run}");
    assert_eq!(run["instance"]["name"], "win-box", "{run}");
    assert_eq!(run["cwd"].as_str().map(PathBuf::from), Some(path.clone()));
    let id = run["id"].as_str().unwrap().to_string();
    let done = wait_state(&p.owner.app, &id, &["failed", "passed"]).await;
    assert_eq!((done["state"].as_str(), done["exitCode"].as_i64()), (Some("failed"), Some(3)), "{done}");
    assert_eq!(done["error"], "tests failed (exit code 3)");
    shows(&p, &id, RunStep::Setup, "setting up").await;
    shows(&p, &id, RunStep::Command, &format!("hello from {branch}")).await;
    let (_, mine) = call(&p.remote.app, "GET", "/api/runs", None).await;
    assert_eq!(mine, json!([]), "the remote keeps no run of its own");

    // Started again, it is the same run, in its tab.
    let (s, again) = start_run(&p, &wt, "hello", "test").await;
    assert_eq!((s, again["id"].as_str()), (StatusCode::OK, Some(id.as_str())), "{again}");
    wait_state(&p.owner.app, &id, &["failed"]).await;

    // A web run's link is the remote's: its port is mapped there, to a proxy that lets only the owner in.
    let (s, run) = start_run(&p, &wt, "web", "run").await;
    assert_eq!(s, StatusCode::CREATED, "{run}");
    let web = run["id"].as_str().unwrap().to_string();
    let ready = wait_state(&p.owner.app, &web, &["ready", "failed"]).await;
    assert_eq!(ready["state"], "ready", "{ready}");
    let url = ready["url"].as_str().unwrap();
    let port: u16 = url
        .strip_prefix("https://win-box.t.ts.net:")
        .and_then(|r| r.strip_suffix('/'))
        .unwrap_or_else(|| panic!("not the remote's link: {url}"))
        .parse()
        .unwrap();
    assert!(ready["port"].is_null(), "the port is the remote's, not the owner's: {ready}");
    let proxy = proxied(&fake.calls(), port);
    let owner = http_get_as(proxy, "me@example.com").await;
    assert!(owner.contains(" 200 ") && owner.ends_with(branch.as_str()), "{owner}");
    assert!(http_get_as(proxy, "eve@example.com").await.contains(" 403 "));

    // Stopping it on the owner stops it there, and its mapping goes.
    let (s, v) = call(&p.owner.app, "POST", &format!("/api/runs/{web}/stop"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (_, r) = call(&p.owner.app, "GET", &format!("/api/runs/{web}"), None).await;
    assert_eq!(r["state"], "stopped", "{r}");
    assert!(fake.calls().contains(&format!("serve --https={port} off")), "{:?}", fake.calls());
    wait_until("the remote's server to stop", async || {
        tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err()
    })
    .await;

    // Its data stays there.
    let (s, v) = call(&p.owner.app, "POST", &format!("/api/runs/{web}/reset-data"), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("remote_project")), "{v}");
    p.owner.state.ptys.shutdown().await;
    p.remote.state.ptys.shutdown().await;
}
