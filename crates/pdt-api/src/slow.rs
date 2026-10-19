//! Logs slow HTTP requests (PDT-42): a warning while one is still running after [`SLOW`], and how long it took when
//! it ends, or when the client gave up on it. A request that hangs shows up in the log even if it never finishes.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::time::{Duration, Instant};

pub(crate) const SLOW: Duration = Duration::from_secs(2);

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
            run.await
        }
    };
    pending.done = true;
    let took = pending.started.elapsed();
    if took >= SLOW {
        tracing::warn!(method = %pending.method, path = %pending.path, status = res.status().as_u16(), ?took, "slow request");
    }
    res
}
