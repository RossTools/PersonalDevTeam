//! Close finished parents (PDT-377).

use super::*;

/// A parent closes when its last open part does, with a comment and a notification; one with work of its own stays
/// open with a notification instead; a part opening again moves a closed parent back to In progress.
pub(super) fn close_parents() -> WorkflowDoc {
    doc(json!({
        "name": "Close finished parents",
        "description": "When the last open part of a ticket reaches Done or Won't do, the parent moves to Done (Won't \
                        do if every part was Won't do), with a comment and a notification. A parent with an active \
                        attempt or a running session stays open and you are told instead. Moving a part out of Done \
                        or Won't do moves its closed parent back to In progress.",
        "nodes": [
            {"id": "moved", "kind": "ticket.stage_changed", "version": 1, "position": {"x": 0, "y": 140},
             "title": "A part moved", "config": {"where": [{"left": "{{trigger.parentId}}", "op": "exists"}]}},
            {"id": "follow", "kind": "ticket.close_parent", "version": 1, "position": {"x": 300, "y": 140},
             "config": {"ticket": "{{trigger.ticketId}}"}},
            {"id": "closed_comment", "kind": "ticket.comment", "version": 1, "position": {"x": 600, "y": 0},
             "title": "Say why it closed", "config": {"ticket": "{{steps.follow.parentId}}",
                "body": "All parts closed: moved to {{steps.follow.stageName}} when {{trigger.ticketKey}} closed."}},
            {"id": "closed_notify", "kind": "notify", "version": 1, "position": {"x": 900, "y": 0}, "config": {
                "title": "{{steps.follow.parentKey}} closed: all its parts are closed",
                "body": "{{steps.follow.parentTitle}}", "link": "{{steps.follow.parentKey}}"}},
            {"id": "held_notify", "kind": "notify", "version": 1, "position": {"x": 600, "y": 140}, "config": {
                "title": "All parts of {{steps.follow.parentKey}} are closed",
                "body": "It stays open: it has an active attempt or a running session of its own.",
                "level": "warning", "link": "{{steps.follow.parentKey}}"}},
            {"id": "reopened_comment", "kind": "ticket.comment", "version": 1, "position": {"x": 600, "y": 280},
             "title": "Say why it reopened", "config": {"ticket": "{{steps.follow.parentId}}",
                "body": "{{trigger.ticketKey}} opened again: back to In progress."}},
            {"id": "end", "kind": "end", "version": 1, "position": {"x": 1200, "y": 140}, "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "moved", "to": "follow"},
            {"from": "follow", "port": "closed", "to": "closed_comment"},
            {"from": "closed_comment", "to": "closed_notify"},
            {"from": "follow", "port": "held", "to": "held_notify"},
            {"from": "follow", "port": "reopened", "to": "reopened_comment"},
            {"from": "follow", "port": "skipped", "to": "end"},
        ],
    }))
}
