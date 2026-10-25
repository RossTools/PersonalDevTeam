//! Helpers shared by the pdt-api integration tests (`mod common;`). Each test binary uses a subset.
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// Sends one request through the router and returns the status and the JSON body (`Null` when it is not JSON).
pub async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
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
pub mod scryer;
