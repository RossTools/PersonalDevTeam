//! Helpers shared by the integration tests of pdt-api and the integrations (`mod common;` in their `tests/main.rs`).
//! Each uses a subset.
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// Sends one request through the router and returns the status and the JSON body (`Null` when it is not JSON).
pub async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    call_with(app, method, uri, body, &[]).await
}

/// [`call`] as PDT's web client makes it (PDT-204): with the UI token it fetches as a same-origin browser fetch.
pub async fn call_as_ui(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().uri("/api/ui/token").header("sec-fetch-site", "same-origin");
    let res = app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let token = serde_json::from_slice::<Value>(&bytes).unwrap()["token"].as_str().unwrap().to_string();
    call_with(app, method, uri, body, &[("x-ui-token", &token)]).await
}

/// [`call`] with extra headers.
pub async fn call_with(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Sends raw bytes (e.g. an image upload) and returns the status, the content type and the body bytes.
pub async fn call_raw(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Vec<u8>,
) -> (StatusCode, Option<String>, Vec<u8>) {
    let req = Request::builder().method(method).uri(uri).body(Body::from(body)).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let ct = res.headers().get("content-type").and_then(|v| v.to_str().ok()).map(String::from);
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, ct, bytes)
}

/// pdt-api's `fake-agent`: cargo's path to it in pdt-api's own tests; elsewhere (an integration crate's tests, which
/// cargo builds no pdt-api binary for) the one in this build's target folder, built first when it is missing.
pub fn fake_agent() -> String {
    if let Some(p) = option_env!("CARGO_BIN_EXE_fake-agent") {
        return p.into();
    }
    static BUILT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            // target/<profile>/deps/<this test> → target/<profile>/fake-agent
            let exe = std::env::current_exe().unwrap();
            let dir = exe.parent().and_then(|d| d.parent()).unwrap().to_path_buf();
            let path = dir.join(format!("fake-agent{}", std::env::consts::EXE_SUFFIX));
            if !path.exists() {
                let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
                let release = dir.file_name().is_some_and(|n| n == "release");
                let mut c = std::process::Command::new(cargo);
                c.args(["build", "-q", "-p", "pdt-api", "--bin", "fake-agent"]);
                if release {
                    c.arg("--release");
                }
                assert!(c.status().unwrap().success(), "could not build fake-agent");
            }
            path.to_string_lossy().into_owned()
        })
        .clone()
}

/// The smallest bytes PDT takes for a PNG (it reads the signature, not the pixels).
pub const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDRfake";

/// Runs `git` in `dir` (with a throwaway identity, and no line-ending conversion so its warnings stay out of the
/// test output) and asserts it succeeds.
pub fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "core.autocrlf=false"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// Records the checkout's branch as pushed to `origin` at HEAD (a remote-tracking ref, no remote needed), as an agent
/// that pushed would leave it: `submitted` is refused for a branch that isn't on `origin` (PDT-161).
pub fn mark_pushed(worktree: &std::path::Path) {
    let out = std::process::Command::new("git")
        .current_dir(worktree)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .unwrap();
    let branch = String::from_utf8(out.stdout).unwrap().trim().to_string();
    git(worktree, &["update-ref", &format!("refs/remotes/origin/{branch}"), "HEAD"]);
}

/// A stand-in agent program that records each launch's arguments in `<dir>/<name>.args` and then waits for input.
/// On Unix every argument is followed by a `<<END>>` line and every launch by a `<<LAUNCH>>` line (see
/// [`recorded_launches`]); on Windows (a `.cmd` shim, run through cmd.exe) each launch is one `echo %*` line.
pub fn argv_agent(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let args_file = dir.join(format!("{name}.args"));
    let (file, body) = if cfg!(windows) {
        (
            format!("{name}.cmd"),
            format!(
                "@echo off\r\n:again\r\n(echo(%* >> \"{}\") 2>nul || goto again\r\nset /p x=\r\n",
                args_file.display()
            ),
        )
    } else {
        (
            name.to_string(),
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n<<END>>\\n' \"$a\"; done >> \"{f}\"\nprintf '<<LAUNCH>>\\n' >> \"{f}\"\nread x\n",
                f = args_file.display()
            ),
        )
    };
    let path = dir.join(file);
    std::fs::write(&path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// The launches [`argv_agent`] recorded, each as its arguments (on Windows: one string, the raw `%*` line). Waits up
/// to 10 s for `want` of them. Only complete launches count: the fake writes one argument at a time, so a launch
/// still being written (no end marker yet) is left out rather than returned with only its first arguments.
pub async fn recorded_launches(dir: &std::path::Path, name: &str, want: usize) -> Vec<Vec<String>> {
    let mut found = vec![];
    for _ in 0..100 {
        let text = std::fs::read_to_string(dir.join(format!("{name}.args"))).unwrap_or_default();
        found = if cfg!(windows) {
            text.split_inclusive('\n').filter(|l| l.ends_with('\n')).map(|l| vec![l.trim().to_string()]).collect()
        } else {
            let mut launches: Vec<&str> = text.split("<<LAUNCH>>\n").collect();
            launches.pop(); // what follows the last end marker: nothing, or a launch still being written
            launches.into_iter().map(|l| l.split_terminator("\n<<END>>\n").map(String::from).collect()).collect()
        };
        if found.len() >= want {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    found
}

/// A launch [`recorded_launches`] gave, as its arguments on every OS: on Windows the raw `%*` line is split back
/// into them ([`split_command_line`]).
pub fn launch_args(launch: Vec<String>) -> Vec<String> {
    if cfg!(windows) { launch.first().map(|l| split_command_line(l)).unwrap_or_default() } else { launch }
}

/// A Windows command line split into its arguments, as a program's C runtime splits it: whitespace separates them
/// outside quotes; inside quotes `""` is a literal quote; `n` backslashes before a quote are `n / 2` backslashes,
/// and an odd one escapes the quote.
pub fn split_command_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut started) = (false, false);
    let chars: Vec<char> = line.trim_end_matches(['\r', '\n']).chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                let n = chars[i..].iter().take_while(|c| **c == '\\').count();
                i += n;
                if chars.get(i) == Some(&'"') {
                    cur.extend(std::iter::repeat_n('\\', n / 2));
                    if n % 2 == 1 {
                        cur.push('"');
                        i += 1;
                    }
                } else {
                    cur.extend(std::iter::repeat_n('\\', n));
                }
                started = true;
                continue;
            }
            '"' if quoted && chars.get(i + 1) == Some(&'"') => {
                cur.push('"');
                i += 1;
            }
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
        i += 1;
    }
    if started {
        out.push(cur);
    }
    out
}

/// Parks the test's idle conversation, retrying for up to 10 s: a turn's end is stored before the conversation's
/// runtime state says the turn is over, so a test that saw the stored end can call park a moment too early.
pub async fn park_once_idle(st: &pdt_api::AppState) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while pdt_api::park_idle_conversations(st).await == 0 {
        assert!(std::time::Instant::now() < deadline, "the conversation never became idle enough to park");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// Makes every insert of an event of type `kind` into the store at `db` fail, as a full disk would; None: inserts work
/// again.
pub async fn fail_events(db: &std::path::Path, kind: Option<&str>) {
    use sqlx::Connection;
    let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(db);
    let mut c = sqlx::SqliteConnection::connect_with(&opts).await.unwrap();
    sqlx::query("DROP TRIGGER IF EXISTS test_fail_events").execute(&mut c).await.unwrap();
    if let Some(k) = kind {
        let sql = format!(
            "CREATE TRIGGER test_fail_events BEFORE INSERT ON events WHEN NEW.type = '{k}' \
             BEGIN SELECT RAISE(ABORT, 'disk full'); END"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql)).execute(&mut c).await.unwrap();
    }
    c.close().await.unwrap();
}
pub mod issues;
pub mod pm;
pub mod workflows;

/// One pass of auto-start: the pickup watch tells what can start, then the engine runs "Pick up tickets".
pub async fn pick_up(st: &pdt_api::AppState) {
    pdt_api::watch_pickup(st).await;
    pdt_api::run_workflows(st).await;
}
