//! The documents' workflow templates (PDT-318): an agent answering new comments on a project's documents, and a sheet
//! that logs each merged ticket.

use crate::workflow::templates::{builtin, doc, params};
use pdt_core::workflow::template::{Category, Template};
use serde_json::json;

/// What the agent answering a comment is asked.
const REPLY_PROMPT: &str = "{{trigger.author}} commented on the document \"{{trigger.title}}\" (id \
{{trigger.documentId}}, {{trigger.url}}). The comment is on:\n\n{{trigger.quote}}\n\nIt says:\n\n{{trigger.body}}\n\n\
Read the document (doc_read or sheet_read, and comments_list for the rest of the thread) and write a short, helpful \
reply. Don't change the document or reply yourself: PDT posts your reply in the comment's thread. Finish with \
submit_outcome: `reply` with the reply's text as the summary, or `skip` when it needs no reply (say why in the \
summary).";

pub(crate) fn all() -> Vec<Template> {
    vec![reply_to_comments(), log_merged()]
}

fn reply_to_comments() -> Template {
    builtin(
        "document-comment-reply",
        Category::Documents,
        params(json!([
            {"id": "persona", "title": "Persona", "type": "string", "default": "Reviewer",
             "description": "Who answers: a persona with the document tools (the default set has them)."},
        ])),
        doc(json!({
            "name": "Reply to new comments on a project's documents with an agent",
            "description": "When someone comments on a document linked to the project (a ticket's design doc, a plan), \
                            an agent reads the document and the comment and replies in its thread. What others wrote \
                            reaches the agent fenced as outside input. Agents' and workflows' own comments never start \
                            it. The first look at a document only records the comments already there.",
            "nodes": [
                {"id": "commented", "kind": "document.comment_added", "version": 1, "position": {"x": 0, "y": 80}},
                {"id": "agent", "kind": "agent", "version": 1, "position": {"x": 320, "y": 80}, "title": "Write a reply",
                 "config": {
                    "persona": "{{params.persona}}", "session": "new", "ticket": "", "attempt": "",
                    "prompt": REPLY_PROMPT,
                    "outcomes": [
                        {"id": "reply", "description": "the summary is your reply to post in the comment's thread"},
                        {"id": "skip", "description": "it needs no reply (say why)"},
                    ]}},
                {"id": "reply", "kind": "document.comment", "version": 1, "position": {"x": 640, "y": 0},
                 "title": "Post the reply", "config": {
                    "document": "{{trigger.documentId}}", "comment": "{{trigger.thread}}",
                    "body": "{{steps.agent.summary}}"}},
                {"id": "skipped", "kind": "end", "version": 1, "position": {"x": 640, "y": 160}, "title": "No reply",
                 "config": {"quiet": true}},
            ],
            "edges": [
                {"from": "commented", "to": "agent"},
                {"from": "agent", "port": "reply", "to": "reply"},
                {"from": "agent", "port": "skip", "to": "skipped"},
            ],
        })),
    )
}

fn log_merged() -> Template {
    builtin(
        "merged-tickets-sheet",
        Category::Documents,
        params(json!([
            {"id": "sheet", "title": "Spreadsheet", "type": "string", "required": true,
             "description": "The spreadsheet's id or exact title (Documents lists them)."},
        ])),
        doc(json!({
            "name": "Log each merged ticket to a sheet",
            "description": "When a ticket's work merges, adds a row to a spreadsheet: the ticket's key and title, its \
                            branch, its pull request and how it merged.",
            "nodes": [
                {"id": "merged", "kind": "attempt.merged", "version": 1, "position": {"x": 0, "y": 0}},
                {"id": "log", "kind": "sheet.append_rows", "version": 1, "position": {"x": 320, "y": 0},
                 "title": "Add a row", "config": {
                    "document": "{{params.sheet}}",
                    "rows": [["{{trigger.ticketKey}}", "{{trigger.ticketTitle}}", "{{trigger.branch}}",
                              "{{trigger.pr}}", "{{trigger.via}}"]]}},
            ],
            "edges": [{"from": "merged", "to": "log"}],
        })),
    )
}
