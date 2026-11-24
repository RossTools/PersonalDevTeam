//! Watches (PDT-319): a ticket, a workflow run or a document (PDT-616) that a PM conversation (or the user) wants to
//! hear about. What happens to it (from the event log, its Inbox items and the run's status) is noted on the watch, a
//! burst of notes is coalesced into one message, and the message goes to the conversation through the outbox, or to
//! the user as a `watch.event` notification. A document's message carries what changed (a text diff), fenced: whoever
//! can edit the document wrote it. Pure types and rules; pdt-store keeps the watches, pdt-api's `watches` ticks them.

use crate::fence::Fence;
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
    /// A document's content changed.
    Changed,
}

impl WatchEvent {
    pub const ALL: [WatchEvent; 6] = [
        WatchEvent::Finished,
        WatchEvent::Outcome,
        WatchEvent::Comment,
        WatchEvent::NeedsYou,
        WatchEvent::Failed,
        WatchEvent::Changed,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            WatchEvent::Finished => "finished",
            WatchEvent::Outcome => "outcome",
            WatchEvent::Comment => "comment",
            WatchEvent::NeedsYou => "needs_you",
            WatchEvent::Failed => "failed",
            WatchEvent::Changed => "changed",
        }
    }
}

/// What a watch of a ticket or run tells about when none are named.
pub const DEFAULT_EVENTS: [WatchEvent; 3] = [WatchEvent::Finished, WatchEvent::Failed, WatchEvent::NeedsYou];
/// What a watch of a document tells about when none are named.
pub const DOCUMENT_EVENTS: [WatchEvent; 2] = [WatchEvent::Changed, WatchEvent::Comment];

/// What is watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WatchTarget {
    Ticket,
    /// A workflow run.
    Run,
    /// A document PDT has (imported, linked or made).
    Document,
}

impl WatchTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            WatchTarget::Ticket => "ticket",
            WatchTarget::Run => "run",
            WatchTarget::Document => "document",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [WatchTarget::Ticket, WatchTarget::Run, WatchTarget::Document].into_iter().find(|t| t.as_str() == s)
    }
    /// What a watch of it tells about when none are named.
    fn default_events(self) -> &'static [WatchEvent] {
        match self {
            WatchTarget::Document => &DOCUMENT_EVENTS,
            _ => &DEFAULT_EVENTS,
        }
    }
    /// Whether it can tell about `e`.
    pub fn has(self, e: WatchEvent) -> bool {
        match self {
            WatchTarget::Document => matches!(e, WatchEvent::Changed | WatchEvent::Comment),
            _ => e != WatchEvent::Changed,
        }
    }
    /// Whether a watch of it ends when it finishes unless told otherwise: a document never finishes.
    pub fn once_by_default(self) -> bool {
        self != WatchTarget::Document
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
    /// What someone outside PDT wrote that goes with it (a document's comment, who edited it): told fenced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside: Option<String>,
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
    /// Whether it keeps a document's content as its watcher was last told (what the next change is diffed against;
    /// the store reads it only to tell a change).
    pub has_snapshot: bool,
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
    /// `ABC-12` for a ticket, the workflow's name for a run, the title for a document.
    pub target_label: String,
    /// The ticket's title; null for a run or a document (or a target that is gone).
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

/// A watch's events: the ones named, in their order, without repeats; the target's defaults when none are. Err names
/// an event the target can't tell about.
pub fn events_or_default(target: WatchTarget, given: Option<&[WatchEvent]>) -> Result<Vec<WatchEvent>, String> {
    let mut out: Vec<WatchEvent> = vec![];
    for e in given.unwrap_or(&[]) {
        if !target.has(*e) {
            let can: Vec<&str> = WatchEvent::ALL.iter().filter(|e| target.has(**e)).map(|e| e.as_str()).collect();
            return Err(format!("a {} can't tell `{}`: give {}", target.as_str(), e.as_str(), can.join(", ")));
        }
        if !out.contains(e) {
            out.push(*e);
        }
    }
    Ok(if out.is_empty() { target.default_events().to_vec() } else { out })
}

/// How long a note waits for others after the newest one, so a burst is told in one message.
pub const QUIET_MS: i64 = 15_000;
/// The longest a note waits to be told.
pub const MAX_WAIT_MS: i64 = 60_000;
/// A document's quiet: its version is looked at every minute, so someone typing makes a change a minute; a burst of
/// edits is told once they have stopped for three.
pub const DOCUMENT_QUIET_MS: i64 = 180_000;
/// The longest a document's note waits to be told.
pub const DOCUMENT_MAX_WAIT_MS: i64 = 600_000;

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

/// What a document's logged event (`document.changed`, `document.comment_added`) means to a watch: (event, clause,
/// outside text), or None when it is no news to this watcher (the user's own comment, to the user).
pub fn document_note(kind: &str, payload: &Value, to_user: bool) -> Option<(WatchEvent, String, Option<String>)> {
    let s = |k: &str| payload.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    match kind {
        ev::DOCUMENT_CHANGED => Some((WatchEvent::Changed, "it changed".into(), s("modifiedBy").map(String::from))),
        ev::DOCUMENT_COMMENT_ADDED => {
            let by_user = payload.get("byUser").and_then(Value::as_bool) == Some(true);
            if to_user && by_user {
                return None;
            }
            let reply = payload.get("reply").and_then(Value::as_bool) == Some(true);
            let line = if reply { "a reply in a comment thread" } else { "a new comment" };
            let mut outside = format!(
                "{} by {}: {}",
                if reply { "Reply" } else { "Comment" },
                s("author").unwrap_or("someone"),
                s("body").unwrap_or_default()
            );
            if let Some(q) = s("quote") {
                outside.push_str(&format!(" (on “{}”)", clip(q, 160)));
            }
            Some((WatchEvent::Comment, line.into(), Some(outside)))
        }
        _ => None,
    }
}

/// A document's clauses, in one line: "it changed; 2 new comments".
fn document_summary(notes: &[WatchNote]) -> String {
    let mut parts: Vec<String> = vec![];
    if notes.iter().any(|n| n.event == WatchEvent::Changed) {
        parts.push("it changed".into());
    }
    let comments: Vec<&WatchNote> = notes.iter().filter(|n| n.event == WatchEvent::Comment).collect();
    match comments.as_slice() {
        [] => {}
        [one] => parts.push(one.line.clone()),
        many => parts.push(format!("{} new comments or replies", many.len())),
    }
    parts.extend(
        notes.iter().filter(|n| !matches!(n.event, WatchEvent::Changed | WatchEvent::Comment)).map(|n| n.line.clone()),
    );
    parts.join("; ")
}

/// What outside text a document's notes carry, for the fenced block: its title, who edited it, the comments, and
/// `diff` (what changed).
fn document_outside(title: &str, notes: &[WatchNote], diff: Option<&str>) -> String {
    let mut out = format!("Title: {title}");
    let mut editors: Vec<&str> = vec![];
    for n in notes.iter().filter(|n| n.event == WatchEvent::Changed) {
        if let Some(who) = n.outside.as_deref().filter(|w| !editors.contains(w)) {
            editors.push(who);
        }
    }
    if !editors.is_empty() {
        out.push_str(&format!("\nEdited by: {}", editors.join(", ")));
    }
    for n in notes.iter().filter(|n| n.event == WatchEvent::Comment) {
        if let Some(o) = &n.outside {
            out.push_str(&format!("\n{o}"));
        }
    }
    if notes.iter().any(|n| n.event == WatchEvent::Changed) {
        match diff {
            Some(d) => out.push_str(&format!("\n\nWhat changed:\n{d}")),
            None => out.push_str("\n\nWhat changed: no change to its text PDT can show (formatting, or edited back)."),
        }
    }
    out
}

/// The message for a PM conversation about a watched document: `[PDT] Watched the document <id>: it changed.`, then
/// the fence's note and a block with its title, who edited it, the comments and what changed (`diff`), since anyone
/// with edit rights wrote those. `ended` says why the watch is over, if it is.
pub fn document_message(
    id: &str,
    title: &str,
    notes: &[WatchNote],
    diff: Option<&str>,
    ended: Option<&str>,
    fence: &Fence,
) -> String {
    let summary = document_summary(notes);
    let mut out = format!("[PDT] Watched the document {id} (`doc_read` reads it)");
    match (summary.is_empty(), ended) {
        (false, _) => out.push_str(&format!(": {summary}.")),
        (true, Some(e)) => out.push_str(&format!(": {e}.")),
        (true, None) => out.push('.'),
    }
    out.push_str(&format!("\n\n{}\n\n{}", fence.note(), fence.wrap(&document_outside(title, notes, diff))));
    if let Some(e) = ended {
        if !summary.is_empty() {
            out.push_str(&format!("\n\nThen {e}."));
        }
        out.push_str("\n\nThe watch has ended.");
    }
    out
}

/// The user's notification about a watched document: (title, body). The Inbox shows it as text, so nothing is fenced.
pub fn document_notice(title: &str, notes: &[WatchNote], diff: Option<&str>, ended: Option<&str>) -> (String, String) {
    let mut summary = document_summary(notes);
    if let Some(e) = ended {
        summary = if summary.is_empty() { e.to_string() } else { format!("{summary}; then {e}") };
    }
    let body = document_outside(title, notes, diff);
    let body = body.split_once('\n').map_or("", |(_, rest)| rest).trim_start().to_string();
    (format!("“{title}”: {summary}"), body)
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
        WatchNote { event: WatchEvent::Outcome, at, line: "x".into(), ends: false, outside: None }
    }

    #[test]
    fn events_default_and_lose_repeats() {
        let t = WatchTarget::Ticket;
        assert_eq!(events_or_default(t, None).unwrap(), DEFAULT_EVENTS.to_vec());
        assert_eq!(events_or_default(t, Some(&[])).unwrap(), DEFAULT_EVENTS.to_vec());
        let given = [WatchEvent::Outcome, WatchEvent::Outcome, WatchEvent::Comment];
        assert_eq!(events_or_default(t, Some(&given)).unwrap(), vec![WatchEvent::Outcome, WatchEvent::Comment]);
        assert!(events_or_default(t, Some(&[WatchEvent::Changed])).is_err(), "a ticket has no `changed`");
        let d = WatchTarget::Document;
        assert_eq!(events_or_default(d, None).unwrap(), DOCUMENT_EVENTS.to_vec());
        let err = events_or_default(d, Some(&[WatchEvent::Outcome])).unwrap_err();
        assert_eq!(err, "a document can't tell `outcome`: give comment, changed");
        assert!(!d.once_by_default() && t.once_by_default());
    }

    fn doc_note(event: WatchEvent, line: &str, outside: Option<&str>) -> WatchNote {
        WatchNote { event, at: 0, line: line.into(), ends: false, outside: outside.map(String::from) }
    }

    #[test]
    fn document_events_become_notes() {
        let changed = json!({"documentId": "d1", "modifiedBy": "Ann"});
        assert_eq!(
            document_note(ev::DOCUMENT_CHANGED, &changed, false),
            Some((WatchEvent::Changed, "it changed".into(), Some("Ann".into())))
        );
        let c = json!({"author": "Sam", "body": "Why?", "quote": "Ship it", "reply": false, "byUser": true});
        let (e, line, outside) = document_note(ev::DOCUMENT_COMMENT_ADDED, &c, false).unwrap();
        assert_eq!((e, line.as_str()), (WatchEvent::Comment, "a new comment"));
        assert_eq!(outside.unwrap(), "Comment by Sam: Why? (on “Ship it”)");
        assert_eq!(document_note(ev::DOCUMENT_COMMENT_ADDED, &c, true), None, "the user's own, to the user");
        assert_eq!(document_note(ev::TICKET_COMMENTED, &c, false), None);
    }

    #[test]
    fn a_document_message_fences_what_others_wrote() {
        let fence = Fence::new("f1");
        let notes = [
            doc_note(WatchEvent::Changed, "it changed", Some("Ann")),
            doc_note(WatchEvent::Changed, "it changed", Some("Ann")),
            doc_note(WatchEvent::Comment, "a new comment", Some("Comment by Sam: Why?")),
        ];
        let m = document_message("d1", "Plan", &notes, Some("+ New idea."), None, &fence);
        assert!(
            m.starts_with("[PDT] Watched the document d1 (`doc_read` reads it): it changed; a new comment.\n\n"),
            "{m}"
        );
        assert!(m.contains(&fence.note()));
        assert!(m.ends_with(
            "<untrusted id=\"f1\">\nTitle: Plan\nEdited by: Ann\nComment by Sam: Why?\n\nWhat changed:\n+ New idea.\n</untrusted id=\"f1\">"
        ), "{m}");
        let gone = document_message("d1", "Plan", &[], None, Some("it was removed from PDT"), &fence);
        assert!(gone.starts_with("[PDT] Watched the document d1 (`doc_read` reads it): it was removed from PDT."));
        assert!(gone.ends_with("The watch has ended."));
        let two = [notes[2].clone(), notes[2].clone()];
        assert!(document_message("d1", "Plan", &two, None, None, &fence).contains(": 2 new comments or replies."));
        let (title, body) = document_notice("Plan", &notes[..1], None, None);
        assert_eq!(title, "“Plan”: it changed");
        assert_eq!(
            body,
            "Edited by: Ann\n\nWhat changed: no change to its text PDT can show (formatting, or edited back)."
        );
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
        let a = WatchNote {
            event: WatchEvent::Outcome,
            at: 0,
            line: "the Coder reported `submitted`".into(),
            ends: false,
            outside: None,
        };
        let b =
            WatchNote { event: WatchEvent::Finished, at: 1, line: "moved to Done".into(), ends: true, outside: None };
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
