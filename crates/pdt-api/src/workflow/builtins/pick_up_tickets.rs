//! Pick up tickets: auto-start as a workflow.

use super::*;

/// Auto-start as a workflow (PDT-327): when a ticket can start, room frees up, or every minute, the candidates of the
/// project (every project, for the sweep) are filtered and ordered, choose keeps to the caps and the start delay, and
/// Start takes the first of each project still free. Every field it leaves out is the project's auto-start setting
/// (Settings), so it does nothing in a project whose auto-start is off.
pub(super) fn pick_up_tickets() -> WorkflowDoc {
    doc(json!({
        "name": "Pick up tickets",
        "description": "Auto-start: in each project whose auto-start is on, starts the next To do (and, if the \
                        project says, Ready) ticket while fewer than its cap are in flight, once it has gone the start \
                        delay unchanged; blocked, conflicting and parent tickets wait. Runs when a ticket can start, \
                        when a ticket in flight ends or an agent slot frees up, and every minute. Its settings are the \
                        project's auto-start settings unless you set them here.",
        "nodes": [
            {"id": "startable", "kind": "ticket.startable", "version": 1, "position": {"x": 0, "y": 0}},
            {"id": "freed", "kind": "slot.freed", "version": 1, "position": {"x": 0, "y": 140}},
            {"id": "sweep", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 280}, "title": "Every minute",
             "config": {"every": "1m", "catchUp": false}},
            {"id": "candidates", "kind": "tickets.candidates", "version": 1, "position": {"x": 300, "y": 140},
             "config": {"project": "{{trigger.projectId}}"}},
            {"id": "choose", "kind": "tickets.choose", "version": 1, "position": {"x": 600, "y": 140},
             "config": {"candidates": "{{steps.candidates}}"}},
            {"id": "start", "kind": "ticket.start", "version": 1, "position": {"x": 900, "y": 0},
             "config": {"ticket": "{{steps.choose.tickets}}"}},
            {"id": "nothing", "kind": "end", "version": 1, "position": {"x": 1200, "y": 280},
             "title": "Nothing to start", "config": {"quiet": true}},
            {"id": "started", "kind": "end", "version": 1, "position": {"x": 1200, "y": 0}, "title": "Started"},
            {"id": "failed", "kind": "end", "version": 1, "position": {"x": 1200, "y": 140},
             "title": "Couldn't start it", "config": {"output": {"failed": "{{steps.start.failed}}"}}},
        ],
        "edges": [
            {"from": "startable", "to": "candidates"},
            {"from": "freed", "to": "candidates"},
            {"from": "sweep", "to": "candidates"},
            {"from": "candidates", "to": "choose"},
            {"from": "choose", "port": "chosen", "to": "start"},
            {"from": "choose", "port": "none", "to": "nothing"},
            {"from": "start", "port": "started", "to": "started"},
            {"from": "start", "port": "skipped", "to": "nothing"},
            {"from": "start", "port": "failed", "to": "failed"},
        ],
    }))
}
