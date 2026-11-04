//! Watches (PDT-319): a ticket or a workflow run that a PM conversation (or the user) wants to hear about. What
//! happens to it (from the event log, its Inbox items and the run's status) is noted on the watch, a burst of notes
//! is coalesced into one message, and the message goes to the conversation through the outbox, or to the user as a
//! `watch.event` notification. Pure types and rules; pdt-store keeps the watches, pdt-api's `watches` ticks them.

use crate::workflow::events as ev;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// What a watch tells about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WatchEvent {
    /// The ticket is Done or Won't do, or its work merged; the run succeeded, failed or was cancelled.
    Finished,
    /// An agent step on the ticket (or in the run) reported its outcome.
    Outcome,
    /// Someone other than the watcher commented on the ticket.
    Comment,
    /// A question, gate or stalled step about it waits on the user.
    NeedsYou,
    /// CI failed, the merge queue dropped its pull request, or the run failed.
    Failed,
}

impl WatchEvent {
    pub const ALL: [WatchEvent; 5] =
        [WatchEvent::Finished, WatchEvent::Outcome, WatchEvent::Comment, WatchEvent::NeedsYou, WatchEvent::Failed];
    pub fn as_str(self) -> &'static str {
        match self {
            WatchEvent::Finished => "finished",
            WatchEvent::Outcome => "outcome",
            WatchEvent::Comment => "comment",
            WatchEvent::NeedsYou => "needs_you",
            WatchEvent::Failed => "failed",
        }
    }
}

/// What a watch tells about when none are named.
pub const DEFAULT_EVENTS: [WatchEvent; 3] = [WatchEvent::Finished, WatchEvent::Failed, WatchEvent::NeedsYou];

/// What is watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WatchTarget {
    Ticket,
    /// A workflow run.
    Run,
}

impl WatchTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            WatchTarget::Ticket => "ticket",
            WatchTarget::Run => "run",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [WatchTarget::Ticket, WatchTarget::Run].into_iter().find(|t| t.as_str() == s)
    }
}

/// One thing that happened to a watched target, waiting to be told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchNote {
    pub event: WatchEvent,
    pub at: i64,
    /// What happened, as a clause: "the Coder reported `submitted`: Built the page".
    pub line: String,
    /// The target finished with it (a `once` watch ends after telling it).
    #[serde(default)]
    pub ends: bool,
}

/// A stored watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    pub id: String,
    pub target: WatchTarget,
    pub target_id: String,
    /// The PM conversation told; None: the user.
    pub session_id: Option<String>,
    pub events: Vec<WatchEvent>,
    pub once: bool,
    /// The last event log `seq` looked at.
    pub cursor: i64,
    /// Inbox items made after this time (ms) are news.
    pub inbox_since: i64,
    /// A run's status when last looked at.
    pub run_status: Option<String>,
    /// Notes not told yet, oldest first.
    pub pending: Vec<WatchNote>,
    pub created_at: i64,
    pub ended_at: Option<i64>,
}

/// A watch as the API and the tools show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WatchInfo {
    pub id: String,
    pub target: WatchTarget,
    pub target_id: String,
    /// `ABC-12` for a ticket, the workflow's name for a run.
    pub target_label: String,
    /// The ticket's title; null for a run (or a target that is gone).
    pub target_title: Option<String>,
    /// The ticket's stage (`in_progress`) or the run's status (`running`); null for a target that is gone.
    pub target_state: Option<String>,
    /// A run's workflow (its page is under it); null for a ticket.
    pub workflow_id: Option<String>,
    /// The PM conversation told; null: the user.
    pub session_id: Option<String>,
    /// "you", or "PM “Docs and website”".
    pub watcher: String,
    pub events: Vec<WatchEvent>,
    pub once: bool,
    #[ts(type = "number")]
    pub created_at: i64,
}

/// A watch's events: the ones named, in their order, without repeats; the defaults when none are.
pub fn events_or_default(given: Option<&[WatchEvent]>) -> Vec<WatchEvent> {
    let mut out: Vec<WatchEvent> = vec![];
    for e in given.unwrap_or(&[]) {
        if !out.contains(e) {
            out.push(*e);
        }
    }
    if out.is_empty() { DEFAULT_EVENTS.to_vec() } else { out }
}

/// How long a note waits for others after the newest one, so a burst is told in one message.
pub const QUIET_MS: i64 = 15_000;
/// The longest a note waits to be told.
pub const MAX_WAIT_MS: i64 = 60_000;

/// Whether a watch's pending notes are told now: the newest is `quiet_ms` old (nothing more came), or the oldest has
/// waited `max_ms`.
pub fn due(pending: &[WatchNote], now: i64, quiet_ms: i64, max_ms: i64) -> bool {
    match (pending.first(), pending.last()) {
        (Some(first), Some(last)) => now - last.at >= quiet_ms || now - first.at >= max_ms,
        _ => false,
    }
}

/// What a ticket's logged event means to a watch: its note, or None when it is no news to this watcher.
/// `watcher_session` is the PM conversation watching (None: the user), whose own comments are no news.
pub fn ticket_note(kind: &str, payload: &Value, watcher_session: Option<&str>) -> Option<(WatchEvent, String, bool)> {
    let s = |k: &str| payload.get(k).and_then(Value::as_str).unwrap_or_default();
    match kind {
        ev::TICKET_STAGE_CHANGED => match s("to") {
            "done" => Some((WatchEvent::Finished, "moved to Done".into(), true)),
            "wont_do" => Some((WatchEvent::Finished, "was closed as Won't do".into(), true)),
            _ => None,
        },
        ev::ATTEMPT_MERGED => {
            let line = match payload.get("pr").and_then(Value::as_u64) {
                Some(n) => format!("its pull request #{n} merged"),
                None => "its work merged".into(),
            };
            Some((WatchEvent::Finished, line, true))
        }
        ev::AGENT_OUTCOME => Some((WatchEvent::Outcome, outcome_line(payload), false)),
        ev::TICKET_COMMENTED => {
            let actor = s("actor");
            let by_session = payload.get("bySession").and_then(Value::as_str);
            let own = match watcher_session {
                None => actor == "user",
                Some(w) => by_session == Some(w),
            };
            if own {
                return None;
            }
            let who = match actor {
                "user" => "the user",
                "agent" => "an agent",
                _ => "a workflow",
            };
            Some((WatchEvent::Comment, format!("{who} commented: {}", clip(s("body"), 160)), false))
        }
        ev::CI_RUN_FAILED => {
            let jobs = payload
                .get("jobs")
                .and_then(Value::as_array)
                .map(|j| j.iter().filter_map(Value::as_str).take(3).collect::<Vec<_>>().join(", "));
            let line = match jobs.filter(|j| !j.is_empty()) {
                Some(j) => format!("CI failed ({j})"),
                None => "CI failed".into(),
            };
            Some((WatchEvent::Failed, line, false))
        }
        ev::PR_QUEUE_LEFT => {
            let n = payload.get("pr").and_then(Value::as_u64).unwrap_or(0);
            Some((WatchEvent::Failed, format!("the merge queue dropped pull request #{n}"), false))
        }
        _ => None,
    }
}

/// An `agent.outcome`'s clause: "the Coder reported `submitted`: Built the page".
pub fn outcome_line(payload: &Value) -> String {
    let s = |k: &str| payload.get(k).and_then(Value::as_str).unwrap_or_default();
    let persona = if s("persona").is_empty() { "agent" } else { s("persona") };
    let summary = clip(s("summary"), 200);
    let head = if s("by") == "user" {
        format!("the user took over the {persona} step with `{}`", s("outcome"))
    } else {
        format!("the {persona} reported `{}`", s("outcome"))
    };
    if summary.is_empty() { head } else { format!("{head}: {summary}") }
}

/// What a run's new status means to a watch: (event, clause). Every finished status ends the watch.
pub fn run_note(status: &str, error: Option<&str>) -> Option<(WatchEvent, String)> {
    match status {
        "succeeded" | "quiet" => Some((WatchEvent::Finished, "succeeded".into())),
        "cancelled" => Some((WatchEvent::Finished, "was cancelled".into())),
        "failed" => {
            let line = match error.map(|e| clip(e, 200)).filter(|e| !e.is_empty()) {
                Some(e) => format!("failed: {e}"),
                None => "failed".into(),
            };
            Some((WatchEvent::Failed, line))
        }
        _ => None,
    }
}

/// Whether a run status is a finished one.
pub fn run_finished(status: &str) -> bool {
    matches!(status, "succeeded" | "quiet" | "failed" | "cancelled")
}

/// An Inbox item's clause: "waits on you: Which colour?".
pub fn needs_you_line(title: &str) -> String {
    format!("waits on you: {}", clip(title, 160))
}

/// The message for a PM conversation: `[PDT] Watched ABC-12 “Title”: the Coder reported …; moved to Done.` One note
/// is one line; several are a list. `ended` says the watch is over.
pub fn message(label: &str, notes: &[WatchNote], ended: bool) -> String {
    let end = if ended { " The watch has ended." } else { "" };
    match notes {
        // A clause that ends a sentence of its own keeps one full stop.
        [one] => format!("[PDT] Watched {label}: {}.{end}", one.line.trim_end_matches('.')),
        many => {
            let lines: Vec<String> = many.iter().map(|n| format!("- {}", n.line)).collect();
            format!(
                "[PDT] Watched {label}:\n{}{}",
                lines.join("\n"),
                if ended { "\n\nThe watch has ended." } else { "" }
            )
        }
    }
}

/// The first line of `text`, cut to `max` characters with "…".
fn clip(text: &str, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() <= max {
        return line.to_string();
    }
    let cut: String = line.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn note(at: i64) -> WatchNote {
        WatchNote { event: WatchEvent::Outcome, at, line: "x".into(), ends: false }
    }

    #[test]
    fn events_default_and_lose_repeats() {
        assert_eq!(events_or_default(None), DEFAULT_EVENTS.to_vec());
        assert_eq!(events_or_default(Some(&[])), DEFAULT_EVENTS.to_vec());
        let given = [WatchEvent::Outcome, WatchEvent::Outcome, WatchEvent::Comment];
        assert_eq!(events_or_default(Some(&given)), vec![WatchEvent::Outcome, WatchEvent::Comment]);
    }

    #[test]
    fn a_burst_waits_for_quiet_or_the_longest_wait() {
        assert!(!due(&[], 1_000_000, QUIET_MS, MAX_WAIT_MS));
        let burst = [note(0), note(10_000), note(20_000)];
        assert!(!due(&burst, 30_000, QUIET_MS, MAX_WAIT_MS), "the newest is 10 s old");
        assert!(due(&burst, 35_000, QUIET_MS, MAX_WAIT_MS), "15 s of quiet");
        let steady: Vec<WatchNote> = (0..7).map(|i| note(i * 10_000)).collect();
        assert!(due(&steady, 61_000, QUIET_MS, MAX_WAIT_MS), "the oldest waited a minute");
        assert!(due(&[note(5)], 5, 0, 0), "no wait at all");
    }

    #[test]
    fn ticket_events_become_notes() {
        let moved = |to: &str| ticket_note(ev::TICKET_STAGE_CHANGED, &json!({"to": to}), None);
        assert_eq!(moved("done"), Some((WatchEvent::Finished, "moved to Done".into(), true)));
        assert_eq!(moved("wont_do").map(|n| n.2), Some(true));
        assert_eq!(moved("in_progress"), None);
        assert_eq!(
            ticket_note(ev::ATTEMPT_MERGED, &json!({"pr": 12}), None),
            Some((WatchEvent::Finished, "its pull request #12 merged".into(), true))
        );
        let outcome = json!({"persona": "Coder", "outcome": "submitted", "summary": "Built it.\nMore.", "by": "agent"});
        assert_eq!(
            ticket_note(ev::AGENT_OUTCOME, &outcome, None),
            Some((WatchEvent::Outcome, "the Coder reported `submitted`: Built it.".into(), false))
        );
        let ci = json!({"jobs": ["rust", "web"]});
        assert_eq!(ticket_note(ev::CI_RUN_FAILED, &ci, None).unwrap().1, "CI failed (rust, web)");
        assert_eq!(ticket_note(ev::PR_QUEUE_LEFT, &json!({"pr": 3}), None).unwrap().0, WatchEvent::Failed);
        assert_eq!(ticket_note(ev::PR_COMMENTED, &json!({}), None), None);
    }

    #[test]
    fn the_watchers_own_comments_are_no_news() {
        let by_user = json!({"actor": "user", "body": "hi"});
        assert_eq!(ticket_note(ev::TICKET_COMMENTED, &by_user, None), None, "the user's own");
        let n = ticket_note(ev::TICKET_COMMENTED, &by_user, Some("pm")).unwrap();
        assert_eq!(n.1, "the user commented: hi");
        let by_pm = json!({"actor": "agent", "body": "noted", "bySession": "pm"});
        assert_eq!(ticket_note(ev::TICKET_COMMENTED, &by_pm, Some("pm")), None, "the PM's own");
        assert!(ticket_note(ev::TICKET_COMMENTED, &by_pm, Some("other")).is_some());
        assert!(ticket_note(ev::TICKET_COMMENTED, &by_pm, None).is_some());
    }

    #[test]
    fn run_statuses_become_notes() {
        assert_eq!(run_note("running", None), None);
        assert_eq!(run_note("succeeded", None).unwrap().0, WatchEvent::Finished);
        assert_eq!(run_note("failed", Some("boom")), Some((WatchEvent::Failed, "failed: boom".into())));
        assert!(run_finished("cancelled") && !run_finished("waiting"));
    }

    #[test]
    fn messages_are_one_line_or_a_list() {
        let a =
            WatchNote { event: WatchEvent::Outcome, at: 0, line: "the Coder reported `submitted`".into(), ends: false };
        let b = WatchNote { event: WatchEvent::Finished, at: 1, line: "moved to Done".into(), ends: true };
        assert_eq!(
            message("ABC-12", std::slice::from_ref(&a), false),
            "[PDT] Watched ABC-12: the Coder reported `submitted`."
        );
        let dotted = WatchNote { line: "the Coder reported `submitted`: Built it.".into(), ..a.clone() };
        assert_eq!(
            message("ABC-12", &[dotted], false),
            "[PDT] Watched ABC-12: the Coder reported `submitted`: Built it."
        );
        assert_eq!(
            message("ABC-12", &[a, b], true),
            "[PDT] Watched ABC-12:\n- the Coder reported `submitted`\n- moved to Done\n\nThe watch has ended."
        );
    }
}
