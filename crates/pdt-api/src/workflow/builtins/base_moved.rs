//! The base branch moved.

use super::*;

pub(super) fn base_moved() -> WorkflowDoc {
    doc(json!({
        "name": "Base moved while a PR waits",
        "description": "When an open pull request with auto-merge on (and no merge queue) falls behind its base, asks \
                        the agent to merge the base in and push (PDT-24).",
        "nodes": [
            {"id": "behind", "kind": "pr.behind", "version": 1, "position": {"x": 0, "y": 0}},
            {"id": "tell", "kind": "session.send", "version": 1, "position": {"x": 320, "y": 0}, "config": {
                "attempt": "{{trigger.attemptId}}", "text": "{{trigger.notice.text}}", "dedupe": "{{trigger.notice.key}}"}},
        ],
        "edges": [{"from": "behind", "to": "tell"}],
    }))
}
