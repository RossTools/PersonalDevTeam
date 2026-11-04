//! When CI is cancelled (PDT-510): CI on a pull request's head cancelled with nothing else running is re-run once,
//! then the ticket goes to the Inbox as stalled.

use super::*;

/// How many times it re-runs one head's cancelled CI on its own before it asks the user.
pub const RERUNS: u32 = 1;

pub(super) fn ci_cancelled() -> WorkflowDoc {
    let head = json!({"attempt": "{{trigger.attemptId}}", "head": "{{trigger.headSha}}"});
    doc(json!({
        "name": "When CI is cancelled",
        "description": "When CI on the head commit of a ticket's pull request is cancelled and nothing else runs, is \
                        queued or failed there, PDT re-runs that head's CI once and notes it on the ticket. Cancelled \
                        again, or when it can't be re-run, the ticket goes to the Inbox as stalled, with Re-run CI; \
                        the item closes by itself once the pull request moves on to another commit or its CI runs \
                        again. Only the pull request's current head counts: a cancelled run on an older commit is \
                        left alone.",
        "nodes": [
            {"id": "cancelled", "kind": "ci.run_cancelled", "version": 1, "position": {"x": 0, "y": 0},
             "config": {"where": [{"left": "{{trigger.pr}}", "op": "exists"}]}},
            {"id": "count", "kind": "state.set", "version": 1, "position": {"x": 280, "y": 0},
             "title": "Count the cancels",
             "config": {"key": "cancels.{{trigger.attemptId}}.{{trigger.headSha}}", "add": 1}},
            {"id": "first", "kind": "condition", "version": 1, "position": {"x": 560, "y": 0},
             "title": "Re-run it?", "config": {"conditions": [
                {"left": "{{steps.count.value}}", "op": "lt", "right": RERUNS + 1}]}},
            {"id": "rerun", "kind": "ci.rerun_head", "version": 1, "position": {"x": 840, "y": -120},
             "title": "Re-run the head's CI", "config": head},
            {"id": "note", "kind": "ticket.comment", "version": 1, "position": {"x": 1120, "y": -200},
             "title": "Note it on the ticket", "config": {"ticket": "{{trigger.ticketId}}",
                "body": "CI on {{steps.rerun.short}} was cancelled with nothing else running; PDT re-ran it."}},
            {"id": "stalled", "kind": "ci.stalled", "version": 1, "position": {"x": 840, "y": 120},
             "title": "Inbox: stalled", "config": head},
            {"id": "again", "kind": "ci.rerun_head", "version": 1, "position": {"x": 1120, "y": 60},
             "title": "Re-run CI", "config": head},
            {"id": "not_rerun", "kind": "ticket.comment", "version": 1, "position": {"x": 1400, "y": 60},
             "title": "Note the failure", "config": {"ticket": "{{trigger.ticketId}}",
                "body": "PDT could not re-run the cancelled CI on {{trigger.headSha}}: {{steps.again.error}}"}},
            {"id": "done", "kind": "end", "version": 1, "position": {"x": 1400, "y": -120}},
            {"id": "moved_on", "kind": "end", "version": 1, "position": {"x": 1400, "y": 260},
             "title": "Moved on", "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "cancelled", "to": "count"},
            {"from": "count", "to": "first"},
            {"from": "first", "port": "true", "to": "rerun"},
            {"from": "first", "port": "false", "to": "stalled"},
            {"from": "rerun", "port": "rerun", "to": "note"},
            {"from": "rerun", "port": "moved_on", "to": "moved_on"},
            {"from": "rerun", "port": "error", "to": "stalled"},
            {"from": "note", "to": "done"},
            {"from": "stalled", "port": "rerun", "to": "again"},
            {"from": "stalled", "port": "dismiss", "to": "moved_on"},
            {"from": "stalled", "port": "moved_on", "to": "moved_on"},
            {"from": "again", "port": "rerun", "to": "done"},
            {"from": "again", "port": "moved_on", "to": "moved_on"},
            {"from": "again", "port": "error", "to": "not_rerun"},
            {"from": "not_rerun", "to": "done"},
        ],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::validate::{Severity, validate};

    #[test]
    fn every_port_leads_somewhere() {
        let d = ci_cancelled();
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        assert!(!issues.iter().any(|i| i.code == "unreachable" || i.code == "open_port"), "{issues:?}");
        assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
    }
}
