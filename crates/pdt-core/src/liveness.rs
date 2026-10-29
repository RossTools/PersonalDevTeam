//! pdtd's background loops' liveness (PDT-127): when each loop last finished a pass, and when that is too long ago.
//! pdt-api records the passes; `/api/health` reports them and the notifier raises `host.loop_stale` for a stale one.

use serde::Serialize;
use ts_rs::TS;

/// The least time a loop may go without finishing a pass before it counts as stale, whatever its period: a slow
/// pass (many attempts' PR checks, a busy engine tick) is not a dead loop.
pub const MIN_STALE_AFTER_MS: i64 = 5 * 60_000;

/// How long a loop that runs every `every_ms` may go without finishing a pass: five periods, at least
/// [`MIN_STALE_AFTER_MS`].
pub fn stale_after(every_ms: i64) -> i64 {
    (every_ms.saturating_mul(5)).max(MIN_STALE_AFTER_MS)
}

/// Whether a loop is stale at `now`: its last finished pass (or, before its first, when it started) is more than
/// `stale_after_ms` ago, not counting `slept_ms` the computer spent asleep since then (no loop runs while it sleeps,
/// so a wake is not a stall; PDT-255).
pub fn is_stale(started_at: i64, last_pass_at: Option<i64>, stale_after_ms: i64, now: i64, slept_ms: i64) -> bool {
    now - last_pass_at.unwrap_or(started_at) - slept_ms.max(0) > stale_after_ms
}

/// One background loop as `/api/health` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LoopHealth {
    /// The loop's name (`poll`, `engine`, `overlaps`, …).
    pub name: String,
    /// How often it runs a pass.
    #[ts(type = "number")]
    pub every_ms: i64,
    /// When it started (this daemon run).
    #[ts(type = "number")]
    pub started_at: i64,
    /// When its last pass finished (or ended in a panic); null before the first.
    #[ts(type = "number | null")]
    pub last_pass_at: Option<i64>,
    /// How long it may go without a finished pass before it is stale.
    #[ts(type = "number")]
    pub stale_after_ms: i64,
    /// Passes that panicked in this daemon run (each was logged; the loop went on).
    #[ts(type = "number")]
    pub panics: u64,
    pub stale: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_after_is_five_periods_but_never_under_the_minimum() {
        assert_eq!(stale_after(2_000), MIN_STALE_AFTER_MS);
        assert_eq!(stale_after(60_000), MIN_STALE_AFTER_MS);
        assert_eq!(stale_after(120_000), 600_000);
        assert_eq!(stale_after(i64::MAX), i64::MAX);
    }

    #[test]
    fn a_loop_is_stale_once_its_last_pass_is_too_old() {
        assert!(!is_stale(0, Some(1_000), 500, 1_400, 0));
        assert!(!is_stale(0, Some(1_000), 500, 1_500, 0), "exactly at the limit is not yet stale");
        assert!(is_stale(0, Some(1_000), 500, 1_501, 0));
    }

    #[test]
    fn before_its_first_pass_a_loop_counts_from_its_start() {
        assert!(!is_stale(1_000, None, 500, 1_200, 0));
        assert!(is_stale(1_000, None, 500, 1_600, 0), "a first pass that never ends is stale too");
    }

    #[test]
    fn time_asleep_does_not_count() {
        assert!(!is_stale(0, Some(1_000), 500, 20_000, 19_000), "woken just now, after a long sleep");
        assert!(is_stale(0, Some(1_000), 500, 20_000, 18_000), "awake too long without a pass is still a stall");
        assert!(is_stale(0, Some(1_000), 500, 1_501, -5_000), "a clock set back never makes it fresher");
    }
}
