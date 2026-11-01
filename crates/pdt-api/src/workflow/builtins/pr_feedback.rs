//! Pull request feedback to the agent.

use super::*;

pub(super) fn pr_feedback() -> WorkflowDoc {
    // Outsiders' comments are never sent to an agent (PDT-202; `session.send` refuses them too): the ticket's
    // timeline shows them.
    let people = json!([
        {"left": "{{trigger.byOwner}}", "op": "falsy"},
        {"left": "{{trigger.isBot}}", "op": "falsy"},
        {"left": "{{trigger.outside}}", "op": "falsy"},
    ]);
    let mut reviewed = people.clone();
    reviewed.as_array_mut().unwrap().extend([
        json!({"left": "{{trigger.state}}", "op": "ne", "right": "approved"}),
        json!({"left": "{{trigger.body}}", "op": "truthy"}),
    ]);
    doc(json!({
        "name": "PR comments to the agent",
        "description": "Comments and reviews on a ticket's pull request from other people (not yours, not bots') go \
                        to the attempt's agent once it is idle. A review's line comments come one by one; approvals \
                        and empty reviews are left out. Those from people who aren't collaborators on the \
                        repository never go to the agent: the ticket's timeline shows them.",
        "nodes": [
            {"id": "commented", "kind": "pr.commented", "version": 1, "position": {"x": 0, "y": 0},
             "config": {"where": people}},
            {"id": "reviewed", "kind": "pr.reviewed", "version": 1, "position": {"x": 0, "y": 140},
             "config": {"where": reviewed}},
            {"id": "tell", "kind": "session.send", "version": 1, "position": {"x": 320, "y": 70}, "config": {
                "attempt": "{{trigger.attemptId}}", "text": "{{trigger.notice.text}}",
                "dedupe": "{{trigger.notice.key}}", "timeline": "pr"}},
        ],
        "edges": [{"from": "commented", "to": "tell"}, {"from": "reviewed", "to": "tell"}],
    }))
}
