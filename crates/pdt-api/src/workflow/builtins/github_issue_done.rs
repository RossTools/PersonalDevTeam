//! The GitHub plugin's built-in: a ticket's issue closed when it is done.

use super::*;

/// A ticket linked to a GitHub issue that finishes closes it (PDT-154): Done as completed, Won't do as not planned,
/// each with a comment, once per ticket (the run key). Only an issue Done closes (`closing`, PDT-481): one PDT filed or
/// imported, or one linked by hand with "Close it when this ticket is done" on.
pub(super) fn github_issue_done() -> WorkflowDoc {
    let trigger = |to: &str| {
        json!({"runKey": "{{trigger.ticketId}}", "where": [
            {"left": "{{trigger.to}}", "op": "eq", "right": to},
            {"left": "{{trigger.closing.github}}", "op": "exists"}]})
    };
    doc(json!({
        "name": "Done closes the GitHub issue",
        "description": "Moving a ticket linked to a GitHub issue to Done closes the issue as completed, with a comment \
                        naming the ticket; Won't do closes it as not planned. Once per ticket: moving it again \
                        changes nothing on GitHub. An issue closed already is left as it is, and one linked by hand only \
                        closes when its \"Close it when this ticket is done\" is on.",
        "nodes": [
            {"id": "done", "kind": "ticket.stage_changed", "version": 1, "position": {"x": 0, "y": 0},
             "config": trigger("done")},
            {"id": "wont_do", "kind": "ticket.stage_changed", "version": 1, "position": {"x": 0, "y": 160},
             "config": trigger("wont_do")},
            {"id": "fixed", "kind": "github.close", "version": 1, "position": {"x": 320, "y": 0},
             "title": "Close as completed", "config": {"ticket": "{{trigger.ticketId}}", "reason": "completed",
                "comment": "Fixed in {{trigger.ticketKey}}; it ships in the next PDT release."}},
            {"id": "dropped", "kind": "github.close", "version": 1, "position": {"x": 320, "y": 160},
             "title": "Close as not planned", "config": {"ticket": "{{trigger.ticketId}}", "reason": "not_planned",
                "comment": "{{trigger.ticketKey}} won't be done, so this is closed as not planned."}},
            {"id": "end", "kind": "end", "version": 1, "position": {"x": 640, "y": 80}, "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "done", "to": "fixed"},
            {"from": "wont_do", "to": "dropped"},
            {"from": "fixed", "port": "unlinked", "to": "end"},
            {"from": "dropped", "port": "unlinked", "to": "end"},
        ],
    }))
}
