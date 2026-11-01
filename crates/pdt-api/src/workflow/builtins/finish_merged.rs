//! Finish merged tickets.

use super::*;

pub(super) fn finish_merged() -> WorkflowDoc {
    doc(json!({
        "name": "Finish merged tickets",
        "description": "When a ticket's branch is found merged, or a workflow or an agent merged it, and the merge \
                        asked to finish it then, moves it to Done and cleans up (sessions, runs, worktree, workspace) \
                        and pulls the base. Uncommitted changes leave it for the merged banner.",
        "nodes": [
            {"id": "merged", "kind": "attempt.merged", "version": 1, "position": {"x": 0, "y": 0},
             "config": {"where": [{"left": "{{trigger.via}}", "op": "in", "right": ["detected", "automated"]}]}},
            {"id": "ask", "kind": "gate", "version": 1, "position": {"x": 280, "y": 0}, "config": {
                "question": "Move {{trigger.ticketKey}} to Done and remove its worktree?",
                "choices": ["finish"],
                "ahead": [{"when": [{"left": "{{trigger.finishOnMerge}}", "op": "truthy"}], "choice": "finish"}],
                "ask": "skip"}},
            {"id": "finish", "kind": "ticket.finish", "version": 1, "position": {"x": 560, "y": 0}, "config": {
                "ticket": "{{trigger.ticketId}}", "stopRunning": true, "deleteBranch": "{{trigger.finishDeleteBranch}}"}},
            {"id": "banner", "kind": "end", "version": 1, "position": {"x": 560, "y": 140}, "title": "Left for the merged banner",
             "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "merged", "to": "ask"},
            {"from": "ask", "port": "finish", "to": "finish"},
            {"from": "ask", "port": "unanswered", "to": "banner"},
            {"from": "ask", "port": "timeout", "to": "banner"},
            {"from": "finish", "port": "blocked", "to": "banner"},
        ],
    }))
}
