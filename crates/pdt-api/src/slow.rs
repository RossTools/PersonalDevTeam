//! Logs slow HTTP requests (PDT-42): a warning while one is still running after [`SLOW`], and how long it took when
//! it ends, or when the client gave up on it. A request that hangs shows up in the log even if it never finishes.
//!
//! With `PDT_SLOW_REQUESTS_LOG` set to a file, each warning is also appended to it as a line (PDT-358): the tests
//! install no tracing subscriber and nextest hides a passing test's output, so CI sets it to see slow requests.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub(crate) const SLOW: Duration = Duration::from_secs(2);

/// The file `PDT_SLOW_REQUESTS_LOG` names, read once.
fn log_file() -> Option<&'static Path> {
    static FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
    FILE.get_or_init(|| std::env::var_os("PDT_SLOW_REQUESTS_LOG").filter(|v| !v.is_empty()).map(PathBuf::from))
        .as_deref()
}

/// Who made the request: the test nextest is running, else the thread's name (libtest names it after the test).
fn origin() -> String {
    std::env::var("NEXTEST_TEST_NAME")
        .ok()
        .or_else(|| std::thread::current().name().map(String::from))
        .unwrap_or_default()
}

/// Appends one line to `file`: the time, who made the request and what happened. Several test processes append to
/// the same file, so the line goes in one write; a failure to write is ignored (it is a diagnostic).
fn append(file: &Path, method: &axum::http::Method, path: &str, what: &str) {
    let line = format!("{} {} {method} {path} {what}\n", jiff::Timestamp::now(), origin());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = f.write_all(line.as_bytes());
    }
}

fn record(method: &axum::http::Method, path: &str, what: &str) {
    if let Some(file) = log_file() {
        append(file, method, path, what);
    }
}

/// Logs, when dropped before `done`, that the client went away from a slow request.
struct Pending {
    method: axum::http::Method,
    path: String,
    started: Instant,
    done: bool,
}

impl Drop for Pending {
    fn drop(&mut self) {
        let took = self.started.elapsed();
        if !self.done && took >= SLOW {
            tracing::warn!(method = %self.method, path = %self.path, ?took, "slow request dropped before it finished");
            record(&self.method, &self.path, &format!("dropped before it finished after {took:?}"));
        }
    }
}

pub(crate) async fn log_slow(req: Request, next: Next) -> Response {
    let mut pending = Pending {
        method: req.method().clone(),
        path: req.uri().path().to_owned(),
        started: Instant::now(),
        done: false,
    };
    let run = next.run(req);
    tokio::pin!(run);
    let res = tokio::select! {
        res = &mut run => res,
        _ = tokio::time::sleep(SLOW) => {
            tracing::warn!(method = %pending.method, path = %pending.path, "request still running after {SLOW:?}");
            record(&pending.method, &pending.path, &format!("still running after {SLOW:?}"));
            run.await
        }
    };
    pending.done = true;
    let took = pending.started.elapsed();
    if took >= SLOW {
        let status = res.status().as_u16();
        tracing::warn!(method = %pending.method, path = %pending.path, status, ?took, "slow request");
        record(&pending.method, &pending.path, &format!("took {took:?} ({status})"));
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_record_is_one_line_appended_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("slow.log");
        append(&file, &axum::http::Method::GET, "/api/tickets", "took 2.5s (200)");
        append(&file, &axum::http::Method::POST, "/api/runs", "still running after 2s");
        let text = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with(" GET /api/tickets took 2.5s (200)"), "{}", lines[0]);
        assert!(lines[1].ends_with(" POST /api/runs still running after 2s"), "{}", lines[1]);
        assert!(lines[0].contains("each_record_is_one_line_appended_to_the_file"), "names the test: {}", lines[0]);
    }

    #[test]
    fn a_file_that_cannot_be_written_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        append(&dir.path().join("missing").join("slow.log"), &axum::http::Method::GET, "/", "took 3s (200)");
    }
}
