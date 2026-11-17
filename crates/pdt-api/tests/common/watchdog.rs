//! A watchdog for a test that has hung in CI without saying where (PDT-620): it names the step the test last reached
//! and ends the test's process when the test runs past its limit, before nextest's own timeout kills it silently.
//!
//! It runs on a thread of its own, so it fires even when the test's runtime thread is blocked (a held lock, a stuck
//! file call), where a `tokio::time::timeout` would never be polled. Under `cargo test` (many tests in one process)
//! firing ends the whole run, which only a hang does.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Watchdog {
    step: Arc<Mutex<&'static str>>,
    done: Arc<AtomicBool>,
}

impl Watchdog {
    /// Watches the test from now: past `limit` (times `PDT_TEST_WAIT_SCALE`), it prints the last step and exits.
    pub fn start(test: &'static str, limit: Duration) -> Watchdog {
        let limit = super::wait::scaled(limit);
        let step = Arc::new(Mutex::new("start"));
        let done = Arc::new(AtomicBool::new(false));
        let (s, d) = (step.clone(), done.clone());
        let started = Instant::now();
        std::thread::spawn(move || {
            while started.elapsed() < limit {
                if d.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if d.load(Ordering::SeqCst) {
                return;
            }
            let at = *s.lock().unwrap_or_else(|e| e.into_inner());
            // Straight to stderr: libtest's capture would drop an `eprintln!` when the process exits.
            let _ = writeln!(std::io::stderr(), "{test} hung: still at step \"{at}\" after {limit:?}");
            std::process::exit(101);
        });
        Watchdog { step, done }
    }

    /// Records the step the test is starting.
    pub fn step(&self, name: &'static str) {
        *self.step.lock().unwrap_or_else(|e| e.into_inner()) = name;
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
    }
}
