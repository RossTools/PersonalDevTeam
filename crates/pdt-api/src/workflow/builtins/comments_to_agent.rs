//! Ticket comments to the agent.

use super::*;

pub(super) fn comments_to_agent() -> WorkflowDoc {
    doc(json!({
        "name": "Ticket comments to the agent",
        "description": "A comment on a ticket that is being worked on goes to the attempt's agent session, once it is \
                        idle (a stopped or interrupted conversation reopens to take it).",
        "nodes": [
            {"id": "commented", "kind": "ticket.commented", "version": 1, "position": {"x": 0, "y": 0}, "config": {"where": [
                {"left": "{{trigger.attemptId}}", "op": "exists"},
                {"left": "{{trigger.actor}}", "op": "eq", "right": "user"},
                {"left": "{{trigger.workflowId}}", "op": "missing"}]}},
            {"id": "tell", "kind": "session.send", "version": 1, "position": {"x": 320, "y": 0}, "config": {
                "attempt": "{{trigger.attemptId}}", "reopen": true,
                "text": "[PDT] New comment on {{trigger.ticketKey}} from the user:\n\n{{trigger.body}}"}},
        ],
        "edges": [{"from": "commented", "to": "tell"}],
    }))
}
