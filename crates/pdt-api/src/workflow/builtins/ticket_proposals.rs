//! Ticket proposals (PDT-366).

use super::*;

/// A ticket an agent session makes (not the PM's, triage's or Make into parts') waits in the Inbox for the user's
/// decision. While this is on, PDT also makes such tickets in Draft without the `agentic` label
/// (`nodes::ticket_proposal::proposals_on`).
pub(super) fn ticket_proposals() -> WorkflowDoc {
    doc(json!({
        "name": "Ticket proposals",
        "description": "A ticket an agent makes while it works (a follow-up a Coder or QA files) waits in your Inbox as \
                        a proposal, in Draft and off the board, with who proposed it, from which ticket and why. \
                        Accept it to To do or to the backlog, mark it a duplicate of another ticket, or reject it; \
                        the agent hears your decision. The PM's tickets, yours and the parts Make into parts makes \
                        skip it. Turn this off and agents' tickets go straight to the board again.",
        "nodes": [
            {"id": "created", "kind": "ticket.created", "version": 1, "position": {"x": 0, "y": 0}, "config": {
                "runKey": "{{trigger.ticketId}}", "where": [
                    {"left": "{{trigger.actor}}", "op": "eq", "right": "agent"},
                    {"left": "{{trigger.viaParts}}", "op": "falsy"}]}},
            {"id": "propose", "kind": "inbox.ticket_proposal", "version": 1, "position": {"x": 320, "y": 0}, "config": {
                "ticket": "{{trigger.ticketId}}", "reason": "{{trigger.reason}}", "persona": "{{trigger.persona}}",
                "session": "{{trigger.sessionId}}", "fromTicket": "{{trigger.fromTicketId}}"}},
            {"id": "end", "kind": "end", "version": 1, "position": {"x": 640, "y": 80}, "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "created", "to": "propose"},
            {"from": "propose", "port": "skipped", "to": "end"},
        ],
    }))
}
