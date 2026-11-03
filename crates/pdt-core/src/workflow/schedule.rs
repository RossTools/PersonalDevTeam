//! Schedules: `every` intervals aligned to the Unix epoch and 5-field cron expressions in a zone,
//! the latest slot at or before a time, and the scheduler's rule for when a slot fires (no backlog).

use chrono::{DateTime, Local, TimeZone, Utc};
use chrono_tz::Tz;
use croner::Cron;
use croner::parser::{CronParser, Seconds};
use serde::{Deserialize, Serialize};

/// The shortest interval a schedule may have.
pub const MIN_EVERY_MS: u64 = 60_000;

/// A slot older than this when the scheduler first sees it was missed (pdtd was down), not just come due.
pub const MISSED_AFTER_MS: i64 = 2 * 60_000;

#[derive(Debug, Clone)]
pub enum Schedule {
    Every(u64),
    Cron { cron: Box<Cron>, zone: Option<Tz> },
}

/// `"90s"`, `"15m"`, `"1h"`, `"1d"` in ms.
pub fn parse_every(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).ok_or_else(|| format!("{s:?} has no unit (s, m, h or d)"))?;
    let (n, unit) = s.split_at(split);
    let n: u64 = n.parse().map_err(|_| format!("{s:?} does not start with a number"))?;
    let unit_ms = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return Err(format!("{s:?}: the unit is s, m, h or d")),
    };
    let ms = n.checked_mul(unit_ms).ok_or("the interval is too long")?;
    if ms < MIN_EVERY_MS {
        return Err(format!("{s:?} is shorter than a minute"));
    }
    Ok(ms)
}

impl Schedule {
    /// Exactly one of `every` and `cron`; `timezone` (IANA) only with `cron`.
    pub fn parse(every: Option<&str>, cron: Option<&str>, timezone: Option<&str>) -> Result<Self, String> {
        let zone = match timezone.map(str::trim).filter(|z| !z.is_empty()) {
            Some(z) => Some(z.parse::<Tz>().map_err(|_| format!("{z:?} is not a time zone"))?),
            None => None,
        };
        match (every.map(str::trim).filter(|s| !s.is_empty()), cron.map(str::trim).filter(|s| !s.is_empty())) {
            (Some(e), None) => {
                if zone.is_some() {
                    return Err("timezone only applies to cron".into());
                }
                Ok(Schedule::Every(parse_every(e)?))
            }
            (None, Some(c)) => {
                if c.split_whitespace().count() != 5 {
                    return Err(format!("{c:?} is not a 5-field cron expression (minute hour day month weekday)"));
                }
                let cron = CronParser::builder()
                    .seconds(Seconds::Disallowed)
                    .build()
                    .parse(c)
                    .map_err(|e| format!("{c:?}: {e}"))?;
                Ok(Schedule::Cron { cron: Box::new(cron), zone })
            }
            (Some(_), Some(_)) => Err("give either every or cron, not both".into()),
            (None, None) => Err("give every (\"15m\") or cron (\"0 7 * * 1-5\")".into()),
        }
    }

    /// The latest slot at or before `now` (ms), if any.
    pub fn latest_slot(&self, now: i64) -> Option<i64> {
        match self {
            Schedule::Every(ms) => {
                let ms = i64::try_from(*ms).ok()?;
                Some(now - now.rem_euclid(ms))
            }
            Schedule::Cron { cron, zone } => {
                let at = Utc.timestamp_millis_opt(now).single()?;
                match zone {
                    Some(z) => previous(cron, &at.with_timezone(z)),
                    None => previous(cron, &at.with_timezone(&Local)),
                }
            }
        }
    }

    /// A slot's date (`YYYY-MM-DD`) and RFC 3339 time in the schedule's zone (the host's for `every`).
    pub fn describe_slot(&self, slot: i64) -> (String, String) {
        let at = Utc.timestamp_millis_opt(slot).single().unwrap_or_default();
        match self {
            Schedule::Cron { zone: Some(z), .. } => fmt(&at.with_timezone(z)),
            _ => fmt(&at.with_timezone(&Local)),
        }
    }
}

fn previous<T: TimeZone>(cron: &Cron, at: &DateTime<T>) -> Option<i64>
where
    DateTime<T>: croner::time::CronDateTime,
{
    cron.find_previous_occurrence(at, true).ok().map(|t| t.timestamp_millis())
}

fn fmt<T: TimeZone>(t: &DateTime<T>) -> (String, String)
where
    T::Offset: std::fmt::Display,
{
    (t.format("%Y-%m-%d").to_string(), t.to_rfc3339())
}

/// What the scheduler last did for a schedule node (kept in workflow state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    /// The last slot handled (fired, skipped or taken as the baseline).
    pub slot: i64,
    /// When this mark was written.
    pub at: i64,
}

/// The scheduler's rule. `latest` is the latest slot ≤ `now`; `changed_at` when the workflow was last saved or enabled.
/// Returns the slot to fire (if any) and the mark to write (None: keep the old one).
pub fn decide(
    prev: Option<Mark>,
    latest: Option<i64>,
    changed_at: i64,
    catch_up: bool,
    now: i64,
) -> (Option<i64>, Option<Mark>) {
    let Some(latest) = latest else { return (None, None) };
    let mark = Some(Mark { slot: latest, at: now });
    match prev {
        // New, or changed since: the current slot is the baseline; the next one fires.
        None => (None, mark),
        Some(p) if changed_at > p.at => (None, mark),
        Some(p) if latest <= p.slot => (None, None),
        Some(_) if now - latest > MISSED_AFTER_MS && !catch_up => (None, mark),
        Some(_) => (Some(latest), mark),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn intervals_parse_with_units_and_a_minimum() {
        assert_eq!(parse_every("15m"), Ok(15 * 60_000));
        assert_eq!(parse_every("90s"), Ok(90_000));
        assert_eq!(parse_every("1h"), Ok(3_600_000));
        assert_eq!(parse_every("2d"), Ok(2 * 86_400_000));
        assert!(parse_every("30s").is_err());
        assert!(parse_every("5").is_err());
        assert!(parse_every("5w").is_err());
        assert!(parse_every("m").is_err());
    }

    #[test]
    fn exactly_one_of_every_and_cron() {
        assert!(Schedule::parse(Some("1m"), None, None).is_ok());
        assert!(Schedule::parse(None, Some("0 7 * * 1-5"), Some("Europe/London")).is_ok());
        assert!(Schedule::parse(Some("1m"), Some("* * * * *"), None).is_err());
        assert!(Schedule::parse(None, None, None).is_err());
        assert!(Schedule::parse(None, Some("0 0 7 * * 1"), None).is_err(), "seconds are not allowed");
        assert!(Schedule::parse(None, Some("not cron"), None).is_err());
        assert!(Schedule::parse(None, Some("0 7 * * *"), Some("Mars/Base")).is_err());
        assert!(Schedule::parse(Some("1m"), None, Some("UTC")).is_err());
    }

    #[test]
    fn intervals_align_to_the_epoch() {
        let s = Schedule::parse(Some("15m"), None, None).unwrap();
        let now = 1_000 * 15 * MIN + 7 * MIN + 1234;
        assert_eq!(s.latest_slot(now), Some(1_000 * 15 * MIN));
        assert_eq!(s.latest_slot(1_000 * 15 * MIN), Some(1_000 * 15 * MIN), "a slot is its own latest");
    }

    #[test]
    fn cron_slots_follow_the_zone() {
        // 2026-01-05 is a Monday. 07:00 in London (GMT in January) is 07:00 UTC; in New York 12:00 UTC.
        let monday_noon_utc = Utc.with_ymd_and_hms(2026, 1, 5, 12, 30, 0).unwrap().timestamp_millis();
        let london = Schedule::parse(None, Some("0 7 * * 1-5"), Some("Europe/London")).unwrap();
        let seven_utc = Utc.with_ymd_and_hms(2026, 1, 5, 7, 0, 0).unwrap().timestamp_millis();
        assert_eq!(london.latest_slot(monday_noon_utc), Some(seven_utc));
        let ny = Schedule::parse(None, Some("0 7 * * 1-5"), Some("America/New_York")).unwrap();
        let twelve_utc = Utc.with_ymd_and_hms(2026, 1, 5, 12, 0, 0).unwrap().timestamp_millis();
        assert_eq!(ny.latest_slot(monday_noon_utc), Some(twelve_utc));
        // Before Monday's slot in New York, the latest is Friday's.
        let friday = Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap().timestamp_millis();
        assert_eq!(ny.latest_slot(twelve_utc - 1), Some(friday));
        assert_eq!(ny.describe_slot(twelve_utc).0, "2026-01-05");
        assert!(ny.describe_slot(twelve_utc).1.starts_with("2026-01-05T07:00:00-05:00"));
    }

    #[test]
    fn cron_across_a_dst_change() {
        // London springs forward on 2026-03-29: 07:00 is 06:00 UTC that day, 07:00 UTC the day before.
        let s = Schedule::parse(None, Some("0 7 * * *"), Some("Europe/London")).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 3, 29, 9, 0, 0).unwrap().timestamp_millis();
        let slot = Utc.with_ymd_and_hms(2026, 3, 29, 6, 0, 0).unwrap().timestamp_millis();
        assert_eq!(s.latest_slot(after), Some(slot));
    }

    #[test]
    fn a_new_schedule_takes_a_baseline_then_fires_at_the_next_slot() {
        let (fire, mark) = decide(None, Some(100 * MIN), 0, true, 100 * MIN + 5);
        assert_eq!(fire, None);
        let mark = mark.unwrap();
        assert_eq!(mark.slot, 100 * MIN);
        // Same slot again: nothing, nothing written.
        assert_eq!(decide(Some(mark), Some(100 * MIN), 0, true, 100 * MIN + 2_000), (None, None));
        // The next slot fires.
        let (fire, next) = decide(Some(mark), Some(101 * MIN), 0, true, 101 * MIN + 1_000);
        assert_eq!(fire, Some(101 * MIN));
        assert_eq!(next.unwrap().slot, 101 * MIN);
    }

    #[test]
    fn missed_slots_fire_once_with_catch_up_only() {
        let mark = Mark { slot: 100 * MIN, at: 100 * MIN };
        // pdtd was down from 100 to 160 minutes (a 5-minute schedule): only the latest slot fires.
        assert_eq!(decide(Some(mark), Some(155 * MIN), 0, true, 160 * MIN).0, Some(155 * MIN));
        let (fire, m) = decide(Some(mark), Some(155 * MIN), 0, false, 160 * MIN);
        assert_eq!(fire, None);
        assert_eq!(m.unwrap().slot, 155 * MIN, "skipped, so it doesn't fire later either");
        // A slot that just came due fires even without catch-up.
        assert_eq!(decide(Some(mark), Some(155 * MIN), 0, false, 155 * MIN + 2_000).0, Some(155 * MIN));
    }

    #[test]
    fn a_changed_workflow_takes_a_new_baseline() {
        let mark = Mark { slot: 100 * MIN, at: 100 * MIN };
        // Enabled again at 150 after being off: no catch-up.
        let (fire, m) = decide(Some(mark), Some(159 * MIN), 150 * MIN, true, 160 * MIN);
        assert_eq!(fire, None);
        assert_eq!(m.unwrap(), Mark { slot: 159 * MIN, at: 160 * MIN });
    }
}
