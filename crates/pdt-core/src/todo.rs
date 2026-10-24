//! The user's todo list (PDT-125): short items in the right panel, each optionally tied to a project or a ticket. The
//! PM reads them (`list_todos`) but never changes them. A todo may have a due day (PDT-140), the user's local day,
//! with an optional time; when it comes, pdtd makes one `todo.due` notification. A dated todo may repeat (PDT-141):
//! checking it off makes the next one, due on the next day its [`Recurrence`] gives. Pure; pdt-store keeps the rows,
//! pdt-api serves them.

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveTime, TimeZone, Weekday};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const MAX_TITLE: usize = 500;

/// One todo, with the keys its chips show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Todo {
    pub id: String,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub title: String,
    pub done: bool,
    /// Ascending: the list's order.
    pub position: f64,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[ts(type = "number | null")]
    pub done_at: Option<i64>,
    /// The project's key (`PDT`), for its chip.
    pub project_key: Option<String>,
    /// The linked ticket's key (`PDT-12`) and title.
    pub ticket_key: Option<String>,
    pub ticket_title: Option<String>,
    /// The day it is due (`YYYY-MM-DD`, the user's local day).
    pub due_on: Option<String>,
    /// The time on that day (`HH:MM`, local); without one it is due from the day's start.
    pub due_time: Option<String>,
    /// How it repeats: checking it off makes the next one. Needs a due date.
    pub recurrence: Option<Recurrence>,
}

/// How a todo repeats (PDT-141): a rule, and whether the next due date counts from this one's due date or from the
/// day it is checked off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Recurrence {
    pub rule: RepeatRule,
    #[serde(default)]
    pub from: RepeatFrom,
}

/// The days a todo repeats on. Weekdays are 1 (Monday) to 7 (Sunday).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "every", rename_all = "camelCase")]
#[ts(export)]
pub enum RepeatRule {
    /// Every `interval` days (1: daily).
    Day { interval: u32 },
    /// Monday to Friday.
    Weekday,
    /// Every `interval` weeks (1: weekly) on these days.
    Week { interval: u32, days: Vec<u8> },
    /// Each month on this day (1–31; a shorter month uses its last day).
    Month { day: u8 },
}

/// What the next due date counts from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RepeatFrom {
    /// The due date: the series keeps its schedule (the default).
    #[default]
    Due,
    /// The day it was checked off.
    Done,
}

/// `POST /api/todos`.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewTodo {
    pub title: String,
    /// A project id or key.
    #[serde(default)]
    #[ts(optional)]
    pub project: Option<String>,
    /// A ticket id or key; its project too unless `project` is given.
    #[serde(default)]
    #[ts(optional)]
    pub ticket: Option<String>,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    #[ts(optional)]
    pub due_on: Option<String>,
    /// `HH:MM`; needs `dueOn`.
    #[serde(default)]
    #[ts(optional)]
    pub due_time: Option<String>,
    /// Needs `dueOn`.
    #[serde(default)]
    #[ts(optional)]
    pub recurrence: Option<Recurrence>,
}

/// `PATCH /api/todos/{id}`: absent fields are left alone; `project: ""` or `ticket: ""` removes the link, `dueOn: ""`
/// clears the due date (and its time and recurrence), `dueTime: ""` only the time, `recurrence: null` stops it
/// repeating.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TodoPatch {
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub done: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub position: Option<f64>,
    #[serde(default)]
    #[ts(optional)]
    pub project: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub ticket: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub due_on: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub due_time: Option<String>,
    /// Some(None): stop repeating.
    #[serde(default, deserialize_with = "present")]
    #[ts(optional)]
    pub recurrence: Option<Option<Recurrence>>,
}

/// Tells an absent field (None) from an explicit null (Some(None)).
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<Recurrence>>, D::Error> {
    Ok(Some(Option::<Recurrence>::deserialize(d)?))
}

/// A todo's title, trimmed, or why not.
pub fn valid_title(t: &str) -> Result<String, String> {
    let t = t.trim();
    if t.is_empty() {
        return Err("a todo needs a title".into());
    }
    if t.contains('\n') || t.chars().count() > MAX_TITLE {
        return Err(format!("a todo is one line of at most {MAX_TITLE} characters"));
    }
    Ok(t.to_string())
}

/// A position the client sent, or why not (NaN and infinities would break the order).
pub fn valid_position(p: f64) -> Result<f64, String> {
    if p.is_finite() { Ok(p) } else { Err("a todo's position is a finite number".into()) }
}

/// When a todo is done: kept if it already was, now if it just became done, none when it is open.
pub fn done_at(was: Option<i64>, done: bool, now: i64) -> Option<i64> {
    if done { Some(was.unwrap_or(now)) } else { None }
}

/// A due day the client sent (`YYYY-MM-DD`; `""` for none), or why not.
pub fn valid_due_on(s: &str) -> Result<Option<String>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    match NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        Ok(d) if s.len() == 10 => Ok(Some(d.format("%Y-%m-%d").to_string())),
        _ => Err(format!("{s:?} is not a date (YYYY-MM-DD)")),
    }
}

/// A due time the client sent (`HH:MM`, 24-hour; `""` for none), or why not.
pub fn valid_due_time(s: &str) -> Result<Option<String>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    match NaiveTime::parse_from_str(s, "%H:%M") {
        Ok(t) if s.len() == 5 => Ok(Some(t.format("%H:%M").to_string())),
        _ => Err(format!("{s:?} is not a time (HH:MM)")),
    }
}

/// A todo's due day and time together: a time needs a day.
pub fn valid_due(due_on: Option<String>, due_time: Option<String>) -> Result<(Option<String>, Option<String>), String> {
    if due_on.is_none() && due_time.is_some() {
        return Err("a due time needs a due date".into());
    }
    Ok((due_on, due_time))
}

/// Longest interval a rule takes (days or weeks).
const MAX_INTERVAL: u32 = 365;

/// A recurrence the client sent, normalised (weekly days sorted, no repeats), or why not. A repeating todo needs a due
/// date.
pub fn valid_recurrence(r: Option<Recurrence>, due_on: Option<&str>) -> Result<Option<Recurrence>, String> {
    let Some(mut r) = r else { return Ok(None) };
    if due_on.is_none() {
        return Err("a repeating todo needs a due date".into());
    }
    match &mut r.rule {
        RepeatRule::Day { interval } | RepeatRule::Week { interval, .. } if !(1..=MAX_INTERVAL).contains(interval) => {
            return Err(format!("a todo repeats every 1 to {MAX_INTERVAL} days or weeks"));
        }
        RepeatRule::Week { days, .. } => {
            days.sort_unstable();
            days.dedup();
            if days.is_empty() || days.iter().any(|d| !(1..=7).contains(d)) {
                return Err("a weekly todo repeats on one or more weekdays (1 = Monday to 7 = Sunday)".into());
            }
        }
        RepeatRule::Month { day } if !(1..=31).contains(day) => {
            return Err("a monthly todo repeats on a day from 1 to 31".into());
        }
        _ => {}
    }
    Ok(Some(r))
}

/// `day` of `y`-`m`, or the month's last day when it is shorter.
fn day_in_month(y: i32, m: u32, day: u8) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(y, m, 1).expect("a month");
    let last = (first + Months::new(1)).pred_opt().expect("a day").day();
    first.with_day(u32::from(day).min(last)).expect("a day in the month")
}

/// The rule's first day after `after`. `anchor` is where the series counts from (every N days or weeks).
fn following(rule: &RepeatRule, anchor: NaiveDate, after: NaiveDate) -> NaiveDate {
    match rule {
        RepeatRule::Day { interval } => {
            let n = i64::from((*interval).max(1));
            let k = (after - anchor).num_days().div_euclid(n) + 1;
            anchor + Duration::days(k * n)
        }
        RepeatRule::Weekday => {
            let mut d = after.succ_opt().expect("a day");
            while matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
                d = d.succ_opt().expect("a day");
            }
            d
        }
        RepeatRule::Week { interval, days } => {
            let n = i64::from((*interval).max(1));
            let monday = |d: NaiveDate| d - Duration::days(i64::from(d.weekday().num_days_from_monday()));
            let start = monday(anchor);
            let mut d = after.succ_opt().expect("a day");
            // At most n weeks (plus one) to look through before a chosen day in a week of the series comes up.
            for _ in 0..(7 * (n + 1)) {
                let week = (monday(d) - start).num_days() / 7;
                if week.rem_euclid(n) == 0 && days.contains(&(d.weekday().number_from_monday() as u8)) {
                    return d;
                }
                d = d.succ_opt().expect("a day");
            }
            d
        }
        RepeatRule::Month { day } => {
            let this = day_in_month(after.year(), after.month(), *day);
            if this > after {
                return this;
            }
            let next = after.with_day(1).expect("a day") + Months::new(1);
            day_in_month(next.year(), next.month(), *day)
        }
    }
}

/// The next todo's due day (`YYYY-MM-DD`) when one due on `due_on` is checked off on `today` (the user's local day).
/// From the due date the series keeps its schedule, but skips days that have already passed: the next one is never
/// due today or earlier. From when it's done, it counts from `today`. None when `due_on` isn't a date.
pub fn next_due_on(r: &Recurrence, due_on: &str, today: NaiveDate) -> Option<String> {
    let due = NaiveDate::parse_from_str(due_on, "%Y-%m-%d").ok()?;
    let next = match r.from {
        RepeatFrom::Due => following(&r.rule, due, due.max(today)),
        RepeatFrom::Done => following(&r.rule, today, today),
    };
    Some(next.format("%Y-%m-%d").to_string())
}

/// [`next_due_on`] with today in the host's own time zone.
pub fn next_due_on_local(r: &Recurrence, due_on: &str) -> Option<String> {
    next_due_on(r, due_on, chrono::Local::now().date_naive())
}

/// The moment (ms) a todo comes due in `zone`: its time on its day, or the day's start (local midnight). None when it
/// has no (valid) due day. A local time skipped by a clock change falls an hour later.
pub fn due_at<Z: TimeZone>(due_on: Option<&str>, due_time: Option<&str>, zone: &Z) -> Option<i64> {
    let day = NaiveDate::parse_from_str(due_on?, "%Y-%m-%d").ok()?;
    let time = match due_time {
        Some(t) => NaiveTime::parse_from_str(t, "%H:%M").ok()?,
        None => NaiveTime::MIN,
    };
    let local = day.and_time(time);
    let at = zone
        .from_local_datetime(&local)
        .earliest()
        .or_else(|| zone.from_local_datetime(&(local + Duration::hours(1))).earliest())?;
    Some(at.timestamp_millis())
}

/// [`due_at`] in the host's own time zone (PDT is personal: the user's).
pub fn due_at_local(due_on: Option<&str>, due_time: Option<&str>) -> Option<i64> {
    due_at(due_on, due_time, &chrono::Local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_one_trimmed_line() {
        assert_eq!(valid_title("  Call Sam "), Ok("Call Sam".into()));
        assert!(valid_title("   ").is_err());
        assert!(valid_title("a\nb").is_err());
        assert!(valid_title(&"x".repeat(MAX_TITLE + 1)).is_err());
        assert!(valid_title(&"x".repeat(MAX_TITLE)).is_ok());
    }

    #[test]
    fn positions_are_finite() {
        assert_eq!(valid_position(-1.5), Ok(-1.5));
        assert!(valid_position(f64::NAN).is_err());
        assert!(valid_position(f64::INFINITY).is_err());
    }

    #[test]
    fn done_at_is_set_once_and_cleared_on_reopen() {
        assert_eq!(done_at(None, true, 5), Some(5));
        assert_eq!(done_at(Some(3), true, 5), Some(3));
        assert_eq!(done_at(Some(3), false, 5), None);
        assert_eq!(done_at(None, false, 5), None);
    }

    #[test]
    fn due_dates_and_times_are_validated() {
        assert_eq!(valid_due_on(" 2026-10-23 "), Ok(Some("2026-10-23".into())));
        assert_eq!(valid_due_on(""), Ok(None), "clears it");
        for bad in ["2026-02-30", "2026-1-5", "23/10/2026", "tomorrow", "2026-10-23T10:00"] {
            assert!(valid_due_on(bad).is_err(), "{bad}");
        }
        assert_eq!(valid_due_time("09:30"), Ok(Some("09:30".into())));
        assert_eq!(valid_due_time(""), Ok(None));
        for bad in ["9:30", "24:00", "09:60", "noon"] {
            assert!(valid_due_time(bad).is_err(), "{bad}");
        }
        assert!(valid_due(None, Some("09:30".into())).is_err());
        assert!(valid_due(Some("2026-10-23".into()), None).is_ok());
    }

    #[test]
    fn a_todo_is_due_at_local_midnight_or_its_time() {
        let zone: chrono_tz::Tz = "America/New_York".parse().unwrap();
        let midnight = zone.with_ymd_and_hms(2026, 10, 23, 0, 0, 0).unwrap().timestamp_millis();
        assert_eq!(due_at(Some("2026-10-23"), None, &zone), Some(midnight));
        assert_eq!(due_at(Some("2026-10-23"), Some("14:30"), &zone), Some(midnight + (14 * 60 + 30) * 60_000));
        assert_eq!(due_at(None, Some("14:30"), &zone), None);
        assert_eq!(due_at(Some("nope"), None, &zone), None);
        // 02:30 doesn't exist on 2026-03-08 in New York: it falls at 03:30.
        let skipped = zone.with_ymd_and_hms(2026, 3, 8, 3, 30, 0).unwrap().timestamp_millis();
        assert_eq!(due_at(Some("2026-03-08"), Some("02:30"), &zone), Some(skipped));
    }

    fn rec(rule: RepeatRule, from: RepeatFrom) -> Recurrence {
        Recurrence { rule, from }
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// The next due day from `due`, checked off on `today`.
    fn next(rule: RepeatRule, due: &str, today: &str) -> String {
        next_due_on(&rec(rule, RepeatFrom::Due), due, day(today)).unwrap()
    }

    #[test]
    fn recurrences_are_validated_and_need_a_due_date() {
        let weekly = |days: Vec<u8>| rec(RepeatRule::Week { interval: 1, days }, RepeatFrom::Due);
        let due = Some("2026-10-23");
        assert_eq!(valid_recurrence(None, None), Ok(None));
        assert!(valid_recurrence(Some(weekly(vec![1])), None).is_err(), "needs a due date");
        assert_eq!(valid_recurrence(Some(weekly(vec![3, 1, 3])), due), Ok(Some(weekly(vec![1, 3]))));
        for bad in [
            weekly(vec![]),
            weekly(vec![0]),
            weekly(vec![8]),
            rec(RepeatRule::Week { interval: 0, days: vec![1] }, RepeatFrom::Due),
            rec(RepeatRule::Day { interval: 0 }, RepeatFrom::Due),
            rec(RepeatRule::Day { interval: MAX_INTERVAL + 1 }, RepeatFrom::Due),
            rec(RepeatRule::Month { day: 0 }, RepeatFrom::Due),
            rec(RepeatRule::Month { day: 32 }, RepeatFrom::Due),
        ] {
            assert!(valid_recurrence(Some(bad.clone()), due).is_err(), "{bad:?}");
        }
        assert!(valid_recurrence(Some(rec(RepeatRule::Weekday, RepeatFrom::Done)), due).is_ok());
    }

    #[test]
    fn recurrences_read_and_write_as_json() {
        let r: Recurrence = serde_json::from_str(r#"{"rule":{"every":"week","interval":2,"days":[1,3]}}"#).unwrap();
        assert_eq!(r, rec(RepeatRule::Week { interval: 2, days: vec![1, 3] }, RepeatFrom::Due), "from defaults to due");
        let back = serde_json::to_value(rec(RepeatRule::Month { day: 31 }, RepeatFrom::Done)).unwrap();
        assert_eq!(back, serde_json::json!({"rule": {"every": "month", "day": 31}, "from": "done"}));
        let p: TodoPatch = serde_json::from_str(r#"{"recurrence":null}"#).unwrap();
        assert_eq!(p.recurrence, Some(None), "null stops it");
        assert_eq!(serde_json::from_str::<TodoPatch>("{}").unwrap().recurrence, None, "absent leaves it");
    }

    #[test]
    fn daily_and_every_n_days() {
        assert_eq!(next(RepeatRule::Day { interval: 1 }, "2026-10-23", "2026-10-23"), "2026-10-24");
        assert_eq!(next(RepeatRule::Day { interval: 1 }, "2026-12-31", "2026-12-31"), "2027-01-01");
        assert_eq!(next(RepeatRule::Day { interval: 3 }, "2026-10-23", "2026-10-20"), "2026-10-26", "done early");
        // Done late: the series keeps its 3-day grid, past today.
        assert_eq!(next(RepeatRule::Day { interval: 3 }, "2026-10-01", "2026-10-08"), "2026-10-10");
        assert_eq!(next(RepeatRule::Day { interval: 3 }, "2026-10-01", "2026-10-07"), "2026-10-10");
        assert_eq!(next(RepeatRule::Day { interval: 1 }, "2026-10-01", "2026-10-08"), "2026-10-09");
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        // 2026-10-23 is a Friday.
        assert_eq!(next(RepeatRule::Weekday, "2026-10-23", "2026-10-23"), "2026-10-26");
        assert_eq!(next(RepeatRule::Weekday, "2026-10-26", "2026-10-26"), "2026-10-27");
        assert_eq!(next(RepeatRule::Weekday, "2026-10-24", "2026-10-24"), "2026-10-26", "from a Saturday");
    }

    #[test]
    fn weekly_on_chosen_days_and_every_n_weeks() {
        let mon_wed = || RepeatRule::Week { interval: 1, days: vec![1, 3] };
        // 2026-10-26 is a Monday.
        assert_eq!(next(mon_wed(), "2026-10-26", "2026-10-26"), "2026-10-28");
        assert_eq!(next(mon_wed(), "2026-10-28", "2026-10-28"), "2026-11-02");
        assert_eq!(next(mon_wed(), "2026-10-28", "2026-10-29"), "2026-11-02", "done a day late");
        let fortnightly = || RepeatRule::Week { interval: 2, days: vec![1, 3] };
        assert_eq!(next(fortnightly(), "2026-10-26", "2026-10-26"), "2026-10-28", "the same week");
        assert_eq!(next(fortnightly(), "2026-10-28", "2026-10-28"), "2026-11-09", "skips a week");
        assert_eq!(next(fortnightly(), "2026-10-28", "2026-11-10"), "2026-11-11", "late: the next on the grid");
        let sundays = RepeatRule::Week { interval: 1, days: vec![7] };
        assert_eq!(next(sundays, "2026-10-25", "2026-10-25"), "2026-11-01");
    }

    #[test]
    fn monthly_falls_back_to_the_months_last_day() {
        let on = |day| RepeatRule::Month { day };
        assert_eq!(next(on(15), "2026-10-15", "2026-10-15"), "2026-11-15");
        assert_eq!(next(on(31), "2026-10-31", "2026-10-31"), "2026-11-30", "31st → 30th");
        assert_eq!(next(on(31), "2026-11-30", "2026-11-30"), "2026-12-31", "and back to the 31st");
        assert_eq!(next(on(31), "2027-01-31", "2027-01-31"), "2027-02-28", "31st → 28th");
        assert_eq!(next(on(31), "2028-01-31", "2028-01-31"), "2028-02-29", "a leap year: 29th");
        assert_eq!(next(on(29), "2027-01-29", "2027-01-29"), "2027-02-28");
        assert_eq!(next(on(29), "2028-02-29", "2028-02-29"), "2028-03-29");
        assert_eq!(next(on(15), "2026-12-15", "2026-12-15"), "2027-01-15", "across the year");
        assert_eq!(next(on(20), "2026-10-15", "2026-10-15"), "2026-10-20", "a rule changed to a later day");
        assert_eq!(next(on(15), "2026-08-15", "2026-10-20"), "2026-11-15", "late: past today");
    }

    #[test]
    fn from_when_its_done_counts_from_today() {
        let done = |rule, due: &str, today: &str| next_due_on(&rec(rule, RepeatFrom::Done), due, day(today)).unwrap();
        assert_eq!(done(RepeatRule::Day { interval: 3 }, "2026-10-01", "2026-10-08"), "2026-10-11");
        assert_eq!(done(RepeatRule::Day { interval: 7 }, "2026-10-23", "2026-10-20"), "2026-10-27", "done early");
        assert_eq!(done(RepeatRule::Month { day: 31 }, "2026-09-30", "2026-10-02"), "2026-10-31");
        let fortnightly = RepeatRule::Week { interval: 2, days: vec![1] };
        // Done on Wednesday 2026-10-28: its week counts as the first, so the Monday after next.
        assert_eq!(done(fortnightly, "2026-10-26", "2026-10-28"), "2026-11-09");
        assert_eq!(next_due_on(&rec(RepeatRule::Weekday, RepeatFrom::Done), "nope", day("2026-10-28")), None);
    }

    #[test]
    fn the_next_one_keeps_its_local_time_across_a_clock_change() {
        let zone: chrono_tz::Tz = "Europe/London".parse().unwrap();
        // British Summer Time ends on 2026-10-25: a daily 09:00 is still 09:00 local, 25 hours later.
        let next_day = next(RepeatRule::Day { interval: 1 }, "2026-10-24", "2026-10-24");
        assert_eq!(next_day, "2026-10-25");
        let before = due_at(Some("2026-10-24"), Some("09:00"), &zone).unwrap();
        let after = due_at(Some(&next_day), Some("09:00"), &zone).unwrap();
        assert_eq!(after - before, 25 * 3_600_000);
        assert_eq!(zone.timestamp_millis_opt(after).unwrap().format("%H:%M").to_string(), "09:00");
        // And into summer time (2027-03-28): 23 hours.
        let spring = next(RepeatRule::Day { interval: 1 }, "2027-03-27", "2027-03-27");
        let gap = due_at(Some(&spring), Some("09:00"), &zone).unwrap()
            - due_at(Some("2027-03-27"), Some("09:00"), &zone).unwrap();
        assert_eq!(gap, 23 * 3_600_000);
    }
}
