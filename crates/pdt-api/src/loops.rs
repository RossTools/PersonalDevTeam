//! pdtd's background loops (PDT-127): each pass runs in its own task, so a panic ends that pass only (logged at error
//! level; the loop goes on), and each loop's last finished pass is recorded for `/api/health` and the notifier's
//! `host.loop_stale`.

use crate::routes::{AppState, now_ms};
use pdt_core::liveness::{LoopHealth, is_stale, stale_after};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// The longest one member of the 60 s poll may take (each sensor call has its own, shorter timeout): an overrun is
/// aborted and logged, and the rest of the poll runs.
pub const POLL_PART_LIMIT: Duration = Duration::from_secs(120);

/// How long the 60 s poll may go without a finished pass: its seven members may each run to [`POLL_PART_LIMIT`].
pub const POLL_STALE_AFTER: Duration = Duration::from_secs(20 * 60);

struct Entry {
    every_ms: i64,
    started_at: i64,
    last_pass_at: Option<i64>,
    stale_after_ms: i64,
    panics: u64,
    /// [`Loops::asleep_ms`] at its start or last pass.
    asleep_at_mark: i64,
}

/// Every registered loop's liveness, by name.
pub struct Loops {
    map: Mutex<BTreeMap<&'static str, Entry>>,
    /// The wall clock and the monotonic clock when tracking began: see [`Loops::asleep_ms`].
    wall_base: i64,
    mono_base: Instant,
}

impl Default for Loops {
    fn default() -> Self {
        Self { map: Mutex::default(), wall_base: now_ms(), mono_base: Instant::now() }
    }
}

impl Loops {
    /// How long the computer has spent asleep since tracking began (PDT-255): the wall clock's time since then less
    /// the monotonic clock's, which stands still while macOS or Linux sleeps. (Where it counts sleep too, as on
    /// Windows, this stays 0 and a long sleep can still read as a stall.) A wall clock set forward counts as sleep
    /// too, which is right: no loop missed a pass in that time.
    fn asleep_ms(&self) -> i64 {
        let mono = i64::try_from(self.mono_base.elapsed().as_millis()).unwrap_or(i64::MAX);
        (now_ms() - self.wall_base).saturating_sub(mono).max(0)
    }

    /// Starts tracking a loop that runs every `every` (again: a loop already tracked keeps its start). Stale after
    /// `stale_after` without a finished pass, by default [`pdt_core::liveness::stale_after`] of its period.
    pub fn register(&self, name: &'static str, every: Duration, stale_after_ms: Option<i64>) {
        let asleep = self.asleep_ms();
        let every_ms = i64::try_from(every.as_millis()).unwrap_or(i64::MAX);
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(name).or_insert_with(|| Entry {
            every_ms,
            started_at: now_ms(),
            last_pass_at: None,
            stale_after_ms: stale_after_ms.unwrap_or_else(|| stale_after(every_ms)),
            panics: 0,
            asleep_at_mark: asleep,
        });
    }

    /// A pass of `name` ended at `now` (finished, or ended in a panic).
    pub fn passed(&self, name: &str, now: i64, panicked: bool) {
        self.passed_at(name, now, panicked, self.asleep_ms());
    }

    fn passed_at(&self, name: &str, now: i64, panicked: bool, asleep: i64) {
        if let Some(e) = self.map.lock().unwrap_or_else(|e| e.into_inner()).get_mut(name) {
            e.last_pass_at = Some(now);
            e.panics += u64::from(panicked);
            e.asleep_at_mark = asleep;
        }
    }

    /// Every tracked loop at `now`, by name. Time asleep since a loop's last pass does not count towards its being
    /// stale: after a wake, the notifier can look before the engine's first pass.
    pub fn health(&self, now: i64) -> Vec<LoopHealth> {
        self.health_at(now, self.asleep_ms())
    }

    fn health_at(&self, now: i64, asleep: i64) -> Vec<LoopHealth> {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.iter()
            .map(|(name, e)| LoopHealth {
                name: name.to_string(),
                every_ms: e.every_ms,
                started_at: e.started_at,
                last_pass_at: e.last_pass_at,
                stale_after_ms: e.stale_after_ms,
                panics: e.panics,
                stale: is_stale(e.started_at, e.last_pass_at, e.stale_after_ms, now, asleep - e.asleep_at_mark),
            })
            .collect()
    }
}

/// How a contained piece of work ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Finished,
    Panicked,
    TimedOut,
}

/// Runs `work` in its own task. A panic ends only it and is logged at error level with `what` and its message; a
/// `limit` it overruns aborts it, logged as a warning.
pub async fn contain<F>(what: &str, limit: Option<Duration>, work: F) -> Ended
where
    F: Future<Output = ()> + Send + 'static,
{
    let mut task = tokio::spawn(work);
    let joined = match limit {
        Some(limit) => match tokio::time::timeout(limit, &mut task).await {
            Ok(r) => r,
            Err(_) => {
                task.abort();
                tracing::warn!(task = what, "{what} took longer than {limit:?} and was stopped");
                return Ended::TimedOut;
            }
        },
        None => task.await,
    };
    match joined {
        Ok(()) => Ended::Finished,
        Err(e) if e.is_panic() => {
            let payload = e.into_panic();
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "(no message)".into());
            tracing::error!(task = what, panic = %msg, "{what} panicked; it goes on with its next pass");
            Ended::Panicked
        }
        // Cancelled: the runtime is shutting down.
        Err(_) => Ended::Finished,
    }
}

/// When a loop's first pass runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    Now,
    AfterOnePeriod,
}

/// Runs `pass` every `every` until shutdown, each pass contained (see [`contain`]) and recorded as `name`'s.
pub async fn run_every<F, Fut>(st: AppState, name: &'static str, every: Duration, start: Start, mut pass: F)
where
    F: FnMut(AppState) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    st.loops.register(name, every, None);
    let first = match start {
        Start::Now => tokio::time::Instant::now(),
        Start::AfterOnePeriod => tokio::time::Instant::now() + every,
    };
    let mut tick = tokio::time::interval_at(first, every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if st.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let ended = contain(name, None, pass(st.clone())).await;
        st.loops.passed(name, now_ms(), ended == Ended::Panicked);
    }
}

/// One member of a loop's pass, contained and limited to [`POLL_PART_LIMIT`]: the members after it run whatever
/// happens to it.
pub async fn part<F, Fut>(st: &AppState, what: &'static str, work: F) -> Ended
where
    F: FnOnce(AppState) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    contain(what, Some(st.poll_part_limit), work(st.clone())).await
}

/// One pass of the 60 s poll, in order: runs whose checkout is gone; attempts' pull requests and CI (PDT-25), then
/// attempts whose branch was merged (tickets spec §6); new issues in repos workflows watch (Phase 7e) and new releases of projects they watch (PDT-365), and pull requests PDT didn't open (PDT-268); failed runs of watched CI workflows (PDT-292); the enabled
/// integrations' checks (their CI, their trackers' issues; PDT-173); workflows kept in repos, and pruning old events
/// and runs (Phase 7a). Each member is contained and limited, so a hung or panicking one never holds up the rest.
pub async fn poll_pass(st: AppState) {
    part(&st, "the gone-runs sweep", |s| async move { crate::sweep_gone_runs(&s).await }).await;
    part(&st, "the PR check", |s| async move { crate::refresh_prs(&s).await }).await;
    part(&st, "the merge check", |s| async move { crate::check_merges(&s).await }).await;
    part(&st, "the issue check", |s| async move { crate::refresh_issues(&s).await }).await;
    part(&st, "the release check", |s| async move { crate::refresh_releases(&s).await }).await;
    part(&st, "the pull request check", |s| async move { crate::refresh_pulls(&s).await }).await;
    part(&st, "the workflow run check", |s| async move { crate::refresh_workflow_runs(&s).await }).await;
    for p in st.registry().pollers() {
        part(&st, p.name, p.run).await;
    }
    part(&st, "workflow upkeep", |s| async move { crate::maintain_workflows(&s).await }).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    /// A log writer the test reads back.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    async fn state() -> (AppState, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        (AppState::new(store, tmp.path().join("data")), tmp)
    }

    #[tokio::test]
    async fn a_pass_that_panics_is_logged_and_the_next_pass_still_runs() {
        let log = Captured::default();
        let w = log.clone();
        let subscriber = tracing_subscriber::fmt().with_writer(move || w.clone()).with_ansi(false).finish();
        // The test's runtime is single-threaded, so the passes' tasks log through this subscriber too.
        let _guard = tracing::subscriber::set_default(subscriber);
        let (st, _tmp) = state().await;
        let passes = Arc::new(AtomicUsize::new(0));
        let counter = passes.clone();
        let looping = tokio::spawn(run_every(st.clone(), "test", Duration::from_millis(10), Start::Now, move |_| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    panic!("the first pass breaks");
                }
            }
        }));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while passes.load(Ordering::SeqCst) < 3 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        st.shutting_down.store(true, Ordering::SeqCst);
        looping.await.unwrap();
        assert!(passes.load(Ordering::SeqCst) >= 3, "the loop went on after the panic");
        let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
        let line = text.lines().find(|l| l.contains("panicked")).unwrap_or_else(|| panic!("no panic line in {text}"));
        assert!(line.contains("ERROR") && line.contains("test") && line.contains("the first pass breaks"), "{line}");
        let h = st.loops.health(now_ms());
        assert_eq!((h[0].name.as_str(), h[0].panics, h[0].stale), ("test", 1, false));
        assert!(h[0].last_pass_at.is_some());
    }

    #[tokio::test]
    async fn a_member_that_hangs_is_stopped_and_the_members_after_it_run() {
        let (mut st, _tmp) = state().await;
        st.poll_part_limit = Duration::from_millis(50);
        let after = Arc::new(AtomicUsize::new(0));
        let a = after.clone();
        let started = std::time::Instant::now();
        let hung = part(&st, "hangs", |_| std::future::pending::<()>()).await;
        let broke = part(&st, "panics", |_| async { panic!("boom") }).await;
        let ran = part(&st, "after", |_| async move {
            a.fetch_add(1, Ordering::SeqCst);
        })
        .await;
        assert_eq!((hung, broke, ran), (Ended::TimedOut, Ended::Panicked, Ended::Finished));
        assert_eq!(after.load(Ordering::SeqCst), 1);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_loop_without_a_pass_goes_stale() {
        let (st, _tmp) = state().await;
        // Explicit times, not the clock: a slow runner can take longer than the stale window between two calls.
        let before = now_ms();
        st.loops.register("slow", Duration::from_secs(1), Some(20));
        st.loops.register("slow", Duration::from_secs(9), None);
        let after = now_ms();
        assert!(!st.loops.health(before)[0].stale);
        assert_eq!(st.loops.health(before)[0].every_ms, 1_000, "registering again changes nothing");
        assert!(st.loops.health(after + 40)[0].stale);
        st.loops.passed("slow", after + 40, false);
        assert!(!st.loops.health(after + 40)[0].stale);
    }

    #[tokio::test]
    async fn time_asleep_is_not_a_stall_but_a_stall_after_waking_still_is() {
        let loops = Loops::default();
        loops.register("engine", Duration::from_secs(2), Some(1_000));
        let at = now_ms();
        loops.passed_at("engine", at, false, 0);
        // Asleep for ten minutes; the notifier looks the moment the computer wakes, before the engine's next pass.
        let woke = at + 600_000;
        assert!(!loops.health_at(woke, 600_000)[0].stale, "the sleep is not counted");
        // Awake again for longer than the stale window with no pass: a real stall.
        assert!(loops.health_at(woke + 1_001, 600_000)[0].stale);
        // A pass after waking marks the sleep as seen; a later sleep counts from there.
        loops.passed_at("engine", woke + 500, false, 600_000);
        assert!(!loops.health_at(woke + 1_400, 600_000)[0].stale);
        assert!(loops.health_at(woke + 1_501, 600_000)[0].stale);
        assert!(!loops.health_at(woke + 300_000, 900_000)[0].stale);
    }

    #[test]
    fn awake_the_clocks_agree() {
        let loops = Loops::default();
        assert!(loops.asleep_ms() < 1_000, "{}", loops.asleep_ms());
    }
}
