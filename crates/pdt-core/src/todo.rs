//! The user's todo list (PDT-125): short items in the right panel, each optionally tied to a project or a ticket. The
//! PM reads them (`list_todos`) but never changes them. A todo may have a due day (PDT-140), the user's local day,
//! with an optional time; when it comes, pdtd makes one `todo.due` notification. Pure; pdt-store keeps the rows,
//! pdt-api serves them.

use chrono::{Duration, NaiveDate, NaiveTime, TimeZone};
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
}

/// `PATCH /api/todos/{id}`: absent fields are left alone; `project: ""` or `ticket: ""` removes the link, `dueOn: ""`
/// clears the due date (and its time), `dueTime: ""` only the time.
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
}
