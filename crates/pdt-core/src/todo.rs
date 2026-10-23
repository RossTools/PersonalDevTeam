//! The user's todo list (PDT-125): short items in the right panel, each optionally tied to a project or a ticket. The
//! PM reads them (`list_todos`) but never changes them. Pure; pdt-store keeps the rows, pdt-api serves them.

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
}

/// `PATCH /api/todos/{id}`: absent fields are left alone; `project: ""` or `ticket: ""` removes the link.
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
}
