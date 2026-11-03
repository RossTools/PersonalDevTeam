//! The event log's types and payloads. Each type is a trigger kind in the catalog, with its payload
//! as the trigger's output schema. CI types are provider-neutral (`provider`).

use schemars::{JsonSchema, Schema};

mod agents;
mod attempts;
mod catalog;
mod ci;
mod documents;
mod github;
mod inbox;
mod outside;
mod pull_requests;
mod releases;
mod runs;
mod schedules;
mod sessions;
mod system;
mod tickets;

pub use agents::*;
pub use attempts::*;
pub use catalog::*;
pub use ci::*;
pub use documents::*;
pub use github::*;
pub use inbox::*;
pub use outside::*;
pub use pull_requests::*;
pub use releases::*;
pub use runs::*;
pub use schedules::*;
pub use sessions::*;
pub use system::*;
pub use tickets::*;

pub const CI_RUN_FAILED: &str = "ci.run_failed";
pub const CI_RUN_PASSED: &str = "ci.run_passed";
pub const PR_CONFLICT: &str = "pr.conflict";
pub const PR_QUEUE_LEFT: &str = "pr.queue_left";
pub const PR_MERGED: &str = "pr.merged";
pub const PR_BEHIND: &str = "pr.behind";
pub const ATTEMPT_STARTED: &str = "attempt.started";
pub const ATTEMPT_MERGED: &str = "attempt.merged";
pub const ATTEMPT_ABANDONED: &str = "attempt.abandoned";
pub const ATTEMPT_LATE_COMMITS: &str = "attempt.late_commits";
pub const TICKET_CREATED: &str = "ticket.created";
pub const TICKET_STAGE_CHANGED: &str = "ticket.stage_changed";
pub const TICKET_UNBLOCKED: &str = "ticket.unblocked";
pub const TICKET_LABELS_CHANGED: &str = "ticket.labels_changed";
pub const TICKET_PRIORITY_CHANGED: &str = "ticket.priority_changed";
pub const TICKET_PARENT_CHANGED: &str = "ticket.parent_changed";
pub const TICKET_COMMENTED: &str = "ticket.commented";
pub const TICKET_OVERLAP_FOUND: &str = "ticket.overlap_found";
pub const TICKET_OVERLAP_CLEARED: &str = "ticket.overlap_cleared";
pub const PR_COMMENTED: &str = "pr.commented";
pub const PR_REVIEWED: &str = "pr.reviewed";
pub const GITHUB_ISSUE_OPENED: &str = "github.issue.opened";
pub const GITHUB_PR_OPENED: &str = "github.pull_request.opened";
pub const GITHUB_PR_PUSHED: &str = "github.pull_request.pushed";
pub const GITHUB_PR_CLOSED: &str = "github.pull_request.closed";
pub const GITHUB_WORKFLOW_FAILED: &str = "github.workflow.failed";
pub const SCHEDULE_FIRED: &str = "schedule.fired";
pub const WEBHOOK_RECEIVED: &str = "webhook.received";
pub const DOCUMENT_CHANGED: &str = "document.changed";
pub const DOCUMENT_COMMENT_ADDED: &str = "document.comment_added";
pub const INBOX_CREATED: &str = "inbox.created";
pub const INBOX_ANSWERED: &str = "inbox.answered";
pub const SESSION_NEEDS_YOU: &str = "session.needs_you";
pub const SESSION_TURN_ENDED: &str = "session.turn_ended";
pub const SESSION_EXITED: &str = "session.exited";
pub const RUN_FAILED: &str = "run.failed";
pub const RUN_FINISHED: &str = "run.finished";
pub const RELEASE_PUBLISHED: &str = "release.published";
pub const UPDATE_AVAILABLE: &str = "update.available";
pub const UPDATE_INSTALLED: &str = "update.installed";
pub const UPDATE_FAILED: &str = "update.failed";
pub const DISK_LOW: &str = "disk.low";
pub const DISK_RECOVERED: &str = "disk.recovered";
pub const CONNECTION_FAILED: &str = "connection.failed";
pub const TODO_DUE: &str = "todo.due";

pub const AGENT_OUTCOME: &str = "agent.outcome";
pub const AGENT_STALLED: &str = "agent.stalled";
pub const AGENT_LOOP_LIMIT: &str = "agent.loop_limit";
pub const AGENT_TIMED_OUT: &str = "agent.timed_out";
pub const OUTSIDE_INPUT: &str = "session.outside_input";
pub const TRIAGE_SKIPPED: &str = "triage.skipped";
pub const TICKET_REFINE_REQUESTED: &str = "ticket.refine_requested";
pub const TICKET_STARTABLE: &str = "ticket.startable";
pub const SLOT_FREED: &str = "slot.freed";

/// The event types that carry outside input (PDT-201): what someone outside PDT wrote. A run they start triages.
pub const UNTRUSTED_EVENTS: &[&str] =
    &[GITHUB_ISSUE_OPENED, GITHUB_PR_OPENED, GITHUB_PR_PUSHED, GITHUB_PR_CLOSED, WEBHOOK_RECEIVED];

/// The pull request events (PDT-268): pull requests on a project's repository that PDT didn't open.
pub const GITHUB_PR_EVENTS: &[&str] = &[GITHUB_PR_OPENED, GITHUB_PR_PUSHED, GITHUB_PR_CLOSED];

/// Whether an event of type `kind` carries outside input.
pub fn untrusted(kind: &str) -> bool {
    UNTRUSTED_EVENTS.contains(&kind)
}

/// Whether an event's payload (a trigger) was written by someone outside the repository (PDT-202: `outside` on
/// `pr.commented` and `pr.reviewed`): it is never sent into an agent session.
pub fn by_outsider(payload: &serde_json::Value) -> bool {
    payload.get("outside").and_then(serde_json::Value::as_bool) == Some(true)
}

/// Whether an event's payload (a trigger) says the user wrote its outside fields (`byUser` on
/// `document.comment_added`, PDT-318): they are not fenced, and no outside input is recorded.
pub fn by_user(payload: &serde_json::Value) -> bool {
    payload.get("byUser").and_then(serde_json::Value::as_bool) == Some(true)
}

/// The event types a workflow's own runs can cause (PDT-364): their payload's `workflowId` names the workflow whose run
/// caused them (the run itself, its Inbox item, its agent step's session). A trigger skips its own workflow's unless
/// it sets `includeOwn`, so a workflow that reports run failures never reports its own.
pub const OWN_RUN_EVENTS: &[&str] = &[
    INBOX_CREATED,
    INBOX_ANSWERED,
    SESSION_NEEDS_YOU,
    SESSION_TURN_ENDED,
    SESSION_EXITED,
    RUN_FAILED,
    RUN_FINISHED,
    AGENT_STALLED,
    AGENT_LOOP_LIMIT,
    AGENT_TIMED_OUT,
];

/// The events of an agent step that is stuck (PDT-367): "When an agent is stuck" handles them.
pub const AGENT_STUCK_EVENTS: &[&str] = &[AGENT_STALLED, AGENT_LOOP_LIMIT, AGENT_TIMED_OUT];

/// Whether an event of type `kind` with `payload` came from a run of workflow `workflow_id` (see [`OWN_RUN_EVENTS`]).
pub fn from_own_run(kind: &str, payload: &serde_json::Value, workflow_id: &str) -> bool {
    OWN_RUN_EVENTS.contains(&kind) && payload.get("workflowId").and_then(serde_json::Value::as_str) == Some(workflow_id)
}

/// How many runs in a row `run.*` events may start one another (PDT-364): workflow A reports B's runs and B reports
/// A's would go on forever. A run started by a `run.*` event is one further down the chain than that event's run;
/// a run this far down logs no `run.*` events.
pub const MAX_RUN_CHAIN: u32 = 3;

/// A run's place in a chain of runs started by `run.*` events, from its trigger: 0 unless a `run.*` event started it.
pub fn run_chain(trigger: &serde_json::Value) -> u32 {
    match trigger.get("type").and_then(serde_json::Value::as_str) {
        Some(RUN_FAILED | RUN_FINISHED) => {
            let up = trigger.get("chain").and_then(serde_json::Value::as_u64).unwrap_or(0);
            u32::try_from(up).unwrap_or(MAX_RUN_CHAIN).saturating_add(1)
        }
        _ => 0,
    }
}

/// A short tail of an agent's last message for a payload (`lastMessage`): its last `max` characters, on a word
/// boundary when one is near, with `…` before it when it was cut.
pub fn message_tail(text: &str, max: usize) -> String {
    let text = text.trim();
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let start = text.char_indices().nth(count - max).map_or(0, |(i, _)| i);
    let tail = &text[start..];
    // Start at the next word when one begins soon.
    let tail = match tail.find(char::is_whitespace) {
        Some(i) if i < max / 5 => tail[i..].trim_start(),
        _ => tail,
    };
    format!("…{tail}")
}

/// The CI provider of GitHub Actions runs.
pub use crate::pr::GITHUB_ACTIONS;

/// One event type: its id, what it means and its payload's schema.
pub struct EventType {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Schema,
    /// The payload's fields that hold what someone outside PDT wrote (PDT-202): fenced wherever they go to an agent
    /// (an agent step's prompt, `session.send`'s text). Paths as in references (`issue.body`).
    pub outside: &'static [&'static str],
}

/// `T`'s JSON Schema, for an [`EventType`]'s `schema`.
pub fn schema_of<T: JsonSchema>() -> Schema {
    schemars::schema_for!(T)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_is_unique_and_has_a_schema() {
        let mut ids = std::collections::HashSet::new();
        for t in EVENT_TYPES.iter().chain(GITHUB_EVENT_TYPES).chain(DOCUMENT_EVENT_TYPES) {
            assert!(ids.insert(t.id), "{} twice", t.id);
            assert!(t.id.contains('.'));
            assert!((t.schema)().as_object().is_some_and(|o| o.contains_key("properties")), "{}", t.id);
        }
        assert!(event_type(PR_CONFLICT).is_some() && event_type("nope").is_none());
    }

    #[test]
    fn outside_pull_requests_are_untrusted_and_their_text_fenced() {
        for id in GITHUB_PR_EVENTS {
            assert!(untrusted(id), "{id}");
            assert_eq!(event_type(id).unwrap().outside, ["title", "body"], "{id}");
        }
        let schema = serde_json::to_value((event_type(GITHUB_PR_OPENED).unwrap().schema)()).unwrap();
        for f in ["kind", "fork", "headRepo", "updateType", "changedFiles", "previousHead", "merged"] {
            assert!(schema["properties"].get(f).is_some(), "{f}");
        }
    }

    #[test]
    fn own_runs_are_told_by_the_payloads_workflow() {
        let p = serde_json::json!({"workflowId": "w1"});
        assert!(from_own_run(RUN_FAILED, &p, "w1"));
        assert!(!from_own_run(RUN_FAILED, &p, "w2"));
        assert!(!from_own_run(RUN_FINISHED, &serde_json::json!({}), "w1"));
        assert!(from_own_run(SESSION_TURN_ENDED, &p, "w1") && from_own_run(INBOX_CREATED, &p, "w1"));
        // Older events carry a workflowId too (an agent outcome is its own run's step): not guarded.
        assert!(!from_own_run(AGENT_OUTCOME, &p, "w1"));
        for id in OWN_RUN_EVENTS {
            let schema = serde_json::to_value((event_type(id).unwrap().schema)()).unwrap();
            assert!(schema["properties"].get("workflowId").is_some(), "{id}");
            assert!(event_type(id).unwrap().description.contains("includeOwn"), "{id}");
        }
    }

    #[test]
    fn a_chain_of_runs_started_by_run_events_counts_up() {
        assert_eq!(run_chain(&serde_json::json!({"type": "ticket.stage_changed"})), 0);
        assert_eq!(run_chain(&serde_json::json!({})), 0);
        assert_eq!(run_chain(&serde_json::json!({"type": RUN_FAILED, "chain": 0})), 1);
        assert_eq!(run_chain(&serde_json::json!({"type": RUN_FINISHED, "chain": 2})), 3);
    }

    #[test]
    fn a_message_tail_keeps_the_end() {
        assert_eq!(message_tail("  short  ", 20), "short");
        let t = message_tail("The first part goes away and the last words stay", 20);
        assert!(t.starts_with('…') && t.ends_with("words stay"), "{t}");
        assert!(t.chars().count() <= 21, "{t}");
        assert_eq!(message_tail("ééééé", 3), "…ééé");
    }

    #[test]
    fn payloads_are_flat_camel_case() {
        let e = TicketCommented {
            project_id: "p".into(),
            ticket_id: "t".into(),
            ticket_key: "PDT-1".into(),
            body: "b".into(),
            actor: "user".into(),
            attempt_id: None,
            workflow_id: None,
            by_session: None,
            author: "user".into(),
            attachments: vec![],
            attachment_list: String::new(),
        };
        assert_eq!(serde_json::to_value(&e).unwrap()["ticketKey"], "PDT-1");
        let a = AttemptEvent {
            attempt: AttemptRef { attempt_id: "a".into(), ..Default::default() },
            actor: "system".into(),
            via: Some(MergeVia::MergedHere),
            pr: None,
            finish_on_merge: true,
            finish_delete_branch: false,
            session_id: None,
            notice: None,
            labels: vec!["automerge".into()],
            ticket_title: "Faster boards".into(),
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["labels"], serde_json::json!(["automerge"]));
        assert_eq!((v["attemptId"].as_str(), v["via"].as_str()), (Some("a"), Some("merged_here")));
        assert_eq!(serde_json::from_value::<AttemptEvent>(v).unwrap(), a);
    }
}
