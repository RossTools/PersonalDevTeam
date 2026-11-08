//! Waiting in tests: poll for what a test expects instead of sleeping a fixed time and then asserting. Shared by test
//! crates with `#[path = ".../test-support/wait.rs"] mod wait;` (pdt-api's, the integrations' and pdtd's tests:
//! `common::wait`; pdt-agents' tests: `wait`).
//!
//! - [`wait_until`] (async) and [`wait_until_blocking`] poll until a check is ready, and panic after [`timeout`].
//! - [`still_after`] (async) and [`still_after_blocking`] check that something does not happen within a window, and
//!   panic as soon as it does.
//!
//! A check returns `bool`, `Option<T>` (ready with `T`) or `Result<T, E>` (ready with `T`; the last `E` is shown when
//! the wait times out, so it can carry what the test saw). `PDT_TEST_WAIT_SCALE` multiplies every timeout (a slow
//! machine: `PDT_TEST_WAIT_SCALE=3`).
#![allow(dead_code)]

use std::fmt::Debug;
use std::time::{Duration, Instant};

/// How often a check runs.
const EVERY: Duration = Duration::from_millis(25);

/// How long [`wait_until`] waits: 30 s, times `PDT_TEST_WAIT_SCALE`. Long, because only a failing test waits it out.
pub fn timeout() -> Duration {
    scaled(Duration::from_secs(30))
}

/// `d` times `PDT_TEST_WAIT_SCALE` (a number, default 1).
pub fn scaled(d: Duration) -> Duration {
    let scale = std::env::var("PDT_TEST_WAIT_SCALE").ok().and_then(|s| s.parse::<f64>().ok()).filter(|s| *s > 0.0);
    d.mul_f64(scale.unwrap_or(1.0))
}

/// What a check returns: ready with a value, or not yet (with what it saw, for the timeout's message).
pub trait Ready {
    type Out;
    fn ready(self) -> Result<Self::Out, String>;
}

impl Ready for bool {
    type Out = ();
    fn ready(self) -> Result<(), String> {
        if self { Ok(()) } else { Err(String::new()) }
    }
}

impl<T> Ready for Option<T> {
    type Out = T;
    fn ready(self) -> Result<T, String> {
        self.ok_or_else(String::new)
    }
}

impl<T, E: Debug> Ready for Result<T, E> {
    type Out = T;
    fn ready(self) -> Result<T, String> {
        self.map_err(|e| format!("{e:#?}"))
    }
}

fn timed_out(what: &str, waited: Duration, last: &str) -> ! {
    if last.is_empty() {
        panic!("waited {waited:?} for {what}");
    }
    panic!("waited {waited:?} for {what}; last saw: {last}");
}

/// Polls `check` until it is ready and returns its value; panics after [`timeout`].
pub async fn wait_until<R: Ready>(what: &str, check: impl AsyncFnMut() -> R) -> R::Out {
    wait_within(what, timeout(), check).await
}

/// [`wait_until`] with its own timeout (scaled like the default), for a wait that is part of what the test checks.
pub async fn wait_within<R: Ready>(what: &str, within: Duration, mut check: impl AsyncFnMut() -> R) -> R::Out {
    let within = scaled(within);
    let start = Instant::now();
    loop {
        match check().await.ready() {
            Ok(v) => return v,
            Err(last) if start.elapsed() >= within => timed_out(what, start.elapsed(), &last),
            Err(_) => tokio::time::sleep(EVERY).await,
        }
    }
}

/// Checks that `happened` stays false for `window`, polling it; panics as soon as it is true.
pub async fn still_after(what: &str, window: Duration, mut happened: impl AsyncFnMut() -> bool) {
    let start = Instant::now();
    loop {
        assert!(!happened().await, "{what} happened after {:?}", start.elapsed());
        if start.elapsed() >= window {
            return;
        }
        tokio::time::sleep(EVERY).await;
    }
}

/// [`wait_until`] for synchronous tests.
pub fn wait_until_blocking<R: Ready>(what: &str, check: impl FnMut() -> R) -> R::Out {
    wait_within_blocking(what, timeout(), check)
}

/// [`wait_within`] for synchronous tests.
pub fn wait_within_blocking<R: Ready>(what: &str, within: Duration, mut check: impl FnMut() -> R) -> R::Out {
    let within = scaled(within);
    let start = Instant::now();
    loop {
        match check().ready() {
            Ok(v) => return v,
            Err(last) if start.elapsed() >= within => timed_out(what, start.elapsed(), &last),
            Err(_) => std::thread::sleep(EVERY),
        }
    }
}

/// [`still_after`] for synchronous tests.
pub fn still_after_blocking(what: &str, window: Duration, mut happened: impl FnMut() -> bool) {
    let start = Instant::now();
    loop {
        assert!(!happened(), "{what} happened after {:?}", start.elapsed());
        if start.elapsed() >= window {
            return;
        }
        std::thread::sleep(EVERY);
    }
}
