//! Ticket comments to the agent.

use super::*;

pub(super) fn comments_to_agent() -> WorkflowDoc {
    doc(json!({
        "name": "Ticket comments to the agent",
        "description": "Your comments and the PM's on a ticket that is being worked on go to the attempt's agent \
                        session, once it is idle (a stopped or interrupted conversation reopens to take it), with the \
                        local paths of the files they attach. Other agents' comments and workflows' notes stay on \
                        the ticket.",
        "nodes": [
            // PDT-341: the PM's comments too (its persona is the author), which the user confirmed before they were
            // made (PDT-537); the ticket's own agents' never, so an agent doesn't hear back what it wrote.
            {"id": "commented", "kind": "ticket.commented", "version": 1, "position": {"x": 0, "y": 0}, "config": {"where": [
                {"left": "{{trigger.attemptId}}", "op": "exists"},
                {"left": "{{trigger.author}}", "op": "in", "right": ["user", "PM"]},
                {"left": "{{trigger.workflowId}}", "op": "missing"}]}},
            {"id": "tell", "kind": "session.send", "version": 1, "position": {"x": 320, "y": 0}, "config": {
                "attempt": "{{trigger.attemptId}}", "reopen": true,
                "text": "[PDT] New comment on {{trigger.ticketKey}} from the {{trigger.author}}:\n\n{{trigger.body}}\
                         {{trigger.attachmentList}}"}},
        ],
        "edges": [{"from": "commented", "to": "tell"}],
    }))
}
