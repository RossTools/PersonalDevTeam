//! The PM's daily update and journal checkpoint.

use super::*;

/// The PM daily checkpoint (PDT-372; hard-coded at the end of a turn before): every hour, each open PM conversation
/// you wrote to since its last update, a day or more ago, gets what happened since (and writes a journal checkpoint);
/// one with nothing new is asked only for the checkpoint. The schedule, the wait and who gets one are the user's to
/// change; journaling when a conversation is reset or closed stays PDT's own.
pub(super) fn pm_daily_checkpoint() -> WorkflowDoc {
    doc(json!({
        "name": "PM daily checkpoint",
        "description": "Keeps the PM's open conversations current: each hour, a conversation you wrote to since its \
                        last update, a day or more ago, gets what happened since (tickets, the Inbox, what other PM \
                        conversations left open) and writes a journal checkpoint; on a quiet day it is asked only for \
                        the checkpoint. Change the schedule to send them at a set time or skip weekends.",
        "nodes": [
            {"id": "hourly", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 0}, "title": "Every hour",
             "config": {"every": "1h", "catchUp": true}},
            {"id": "update", "kind": "pm.update", "version": 1, "position": {"x": 260, "y": 0},
             "title": "Send the daily updates", "config": {"after": "1d", "onlyInUse": true}},
            {"id": "checkpoint", "kind": "pm.checkpoint", "version": 1, "position": {"x": 520, "y": 0},
             "title": "Ask the quiet ones for a checkpoint",
             "config": {"conversations": "{{steps.update.nothingNew}}"}},
            {"id": "done", "kind": "end", "version": 1, "position": {"x": 780, "y": 0}, "title": "Done"},
        ],
        "edges": [
            {"from": "hourly", "to": "update"},
            {"from": "update", "port": "updated", "to": "checkpoint"},
            {"from": "update", "port": "none", "to": "checkpoint"},
            {"from": "checkpoint", "to": "done"},
        ],
    }))
}
