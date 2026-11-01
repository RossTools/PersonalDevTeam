//! Commits pushed after a merge.

use super::*;

const FOLLOW_UP: &str = "{{trigger.summary}}.\n\nPR #{{trigger.pr}} ({{trigger.url}}) merged {{trigger.ticketKey}} \
into {{trigger.baseBranch}} at {{trigger.mergedHead}}, and these commits on {{trigger.branch}} came after it, so \
they are not in {{trigger.baseBranch}}:\n\n{{trigger.list}}\n\nCherry-pick them onto this ticket's branch (`git \
cherry-pick {{trigger.mergedHead}}..{{trigger.head}}`; the branch {{trigger.branch}} is kept for this), check that \
they still make sense on {{trigger.baseBranch}}, run the checks and open a pull request.\n\n## Done when\n- What \
those commits change is in {{trigger.baseBranch}}, or found not to be needed there.";

/// Commits pushed to an attempt's branch after its pull request merged (PDT-151): the attempt's agent hears that its
/// push came too late, and the Inbox asks the user what becomes of them: a follow-up ticket that cherry-picks them
/// (the ticket finishes, keeping the branch), discard them and finish (after confirming), or keep the ticket open.
pub(super) fn late_commits() -> WorkflowDoc {
    let ended = json!({"type": "attempt.merged", "also": ["attempt.abandoned"],
        "where": [{"left": "{{event.attemptId}}", "op": "eq", "right": "{{trigger.attemptId}}"}]});
    let comment = |body: &str| json!({"ticket": "{{trigger.ticketId}}", "body": body});
    doc(json!({
        "name": "Commits pushed after a merge",
        "description": "When commits reach a ticket's branch after its pull request merged, the ticket stays open: \
                        this tells the attempt's agent, and asks you in the Inbox whether to open a follow-up ticket \
                        that cherry-picks them (finishing this one and keeping its branch), discard them and finish \
                        (after you confirm), or keep the ticket open.",
        "nodes": [
            {"id": "late", "kind": "attempt.late_commits", "version": 1, "position": {"x": 0, "y": 0}},
            {"id": "tell", "kind": "session.send", "version": 1, "position": {"x": 280, "y": -140},
             "title": "Tell the agent", "config": {
                "attempt": "{{trigger.attemptId}}", "text": "{{trigger.notice.text}}",
                "dedupe": "{{trigger.notice.key}}", "timeline": "pr"}},
            {"id": "ask", "kind": "gate", "version": 1, "position": {"x": 280, "y": 0}, "title": "What becomes of them?",
             "config": {
                "question": "{{trigger.summary}}. The pull request merged without them, so the ticket is still open. \
                             Open a follow-up ticket that cherry-picks them, discard them and finish the ticket, or \
                             keep it open.",
                "choices": ["open_follow_up", "discard_and_finish", "keep_open"], "event": ended}},
            {"id": "follow_up", "kind": "ticket.create", "version": 1, "position": {"x": 560, "y": -140},
             "title": "Open a follow-up", "config": {
                "project": "{{trigger.projectId}}", "stage": "todo", "description": FOLLOW_UP,
                "title": "Follow-up to {{trigger.ticketKey}}: commits pushed after PR #{{trigger.pr}} merged"}},
            {"id": "follow_note", "kind": "ticket.comment", "version": 1, "position": {"x": 840, "y": -140},
             "title": "Note the follow-up", "config": comment(
                "{{steps.follow_up.key}} takes the commits pushed after PR #{{trigger.pr}} merged; the branch \
                 {{trigger.branch}} stays for it.")},
            {"id": "finish_keep", "kind": "ticket.finish", "version": 1, "position": {"x": 1120, "y": -140},
             "title": "Finish, keeping the branch", "config": {
                "ticket": "{{trigger.ticketId}}", "stopRunning": true, "deleteBranch": false}},
            {"id": "confirm", "kind": "gate", "version": 1, "position": {"x": 560, "y": 0}, "title": "Really discard?",
             "config": {
                "question": "Discard {{trigger.summary}}? They are not in {{trigger.baseBranch}}: finishing \
                             {{trigger.ticketKey}} deletes its branch {{trigger.branch}} and its worktree.",
                "choices": ["discard", "back"], "event": ended}},
            {"id": "finish_drop", "kind": "ticket.finish", "version": 1, "position": {"x": 840, "y": 0},
             "title": "Finish, deleting the branch", "config": {
                "ticket": "{{trigger.ticketId}}", "stopRunning": true, "deleteBranch": true}},
            {"id": "blocked", "kind": "ticket.comment", "version": 1, "position": {"x": 1400, "y": 0},
             "title": "Note what blocks it", "config": comment(
                "Couldn't finish {{trigger.ticketKey}}: something in the way stops it (uncommitted changes in its \
                 worktree, or a session or run that didn't stop). Clear it, then move the ticket to Done.")},
            {"id": "done", "kind": "end", "version": 1, "position": {"x": 1400, "y": -140}, "title": "Finished"},
            {"id": "kept", "kind": "end", "version": 1, "position": {"x": 560, "y": 140}, "title": "Kept open",
             "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "late", "to": "tell"},
            {"from": "late", "to": "ask"},
            {"from": "ask", "port": "open_follow_up", "to": "follow_up"},
            {"from": "ask", "port": "discard_and_finish", "to": "confirm"},
            {"from": "ask", "port": "keep_open", "to": "kept"},
            {"from": "ask", "port": "event", "to": "kept"},
            {"from": "ask", "port": "unanswered", "to": "kept"},
            {"from": "ask", "port": "timeout", "to": "kept"},
            {"from": "follow_up", "to": "follow_note"},
            {"from": "follow_note", "to": "finish_keep"},
            {"from": "finish_keep", "port": "finished", "to": "done"},
            {"from": "finish_keep", "port": "blocked", "to": "blocked"},
            {"from": "confirm", "port": "discard", "to": "finish_drop"},
            {"from": "confirm", "port": "back", "to": "ask"},
            {"from": "confirm", "port": "event", "to": "kept"},
            {"from": "confirm", "port": "unanswered", "to": "kept"},
            {"from": "confirm", "port": "timeout", "to": "kept"},
            {"from": "finish_drop", "port": "finished", "to": "done"},
            {"from": "finish_drop", "port": "blocked", "to": "blocked"},
        ],
        "settings": {"maxRuntimeMs": 30_i64 * 24 * 60 * 60 * 1000},
    }))
}
