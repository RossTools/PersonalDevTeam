//! The built-in Pipeline template (Phase 7c spec §3): Draft → Refine → Ready → Code → Review → QA → Accept → Done as
//! agent steps and gates. Its parameters pick the personas, which stages a person does, where a refined ticket goes,
//! whether a person accepts, and whether the run or a person merges; the project's policies stay the guard.

use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::template::TemplateParam;
use serde_json::{Value, json};

pub const ID: &str = "pipeline";

const REFINE_PROMPT: &str = "Refine this ticket so a coding agent can build it without guessing: a clear scope, \
what is out of scope, and a \"Done when\" list someone can check. Rewrite the ticket with `update_ticket`. Ask the \
user (`ask_user`) about anything only they can decide.";

const CODE_PROMPT: &str = "Build this ticket on its branch. On a later pass, address every point in the earlier \
review and QA outcomes and in the user's newest comments, e.g. why they rejected it at Accept (`get_ticket` lists \
them, with their line comments). Run the project's checks, commit, push and open a pull request against the base \
branch when the project works with them.";

const REVIEW_PROMPT: &str = "Review this attempt's changes against the ticket: the branch's diff against its base. \
Put each finding on its line (`comments` in `submit_outcome`).";

const FIX_PROMPT: &str = "QA passed, but the merge was refused: {{steps.merge.reason}}. Clear it on the branch: \
for a conflict, merge the base branch in and resolve it; for uncommitted changes, commit them; for failing CI, read \
the failure and fix its cause. Run the project's checks, commit and push. Change nothing else. A Reviewer looks at \
the fix before PDT merges again, once CI passes.";

const FIX_REVIEW_PROMPT: &str = "QA passed on this attempt, then its merge was refused (a conflict, uncommitted \
changes or failing CI) and the Coder fixed that. Review the fix: the commits since your earlier review, and the branch's diff \
against its base, against the ticket. Put each finding on its line (`comments` in `submit_outcome`).";

/// How long a merge may wait (for a person, or for someone to clear what blocks it) before the Inbox asks again.
const MERGE_WAIT_MS: i64 = 3 * 24 * 60 * 60 * 1000;

/// How long a merge waits for CI on the pull request's head to finish before the Inbox asks (PDT-151).
const CI_WAIT_MS: i64 = 6 * 60 * 60 * 1000;

const QA_PROMPT: &str = "Check that this attempt's change works: run the project's tests and try each point of the \
ticket's \"Done when\".";

pub fn params() -> Vec<TemplateParam> {
    let who = |id: &str, title: &str| {
        json!({"id": id, "title": title, "type": "choice", "choices": ["agent", "user"], "default": "agent",
               "description": "user: a manual stage. The Inbox asks you to do it and pick its outcome."})
    };
    let persona = |id: &str, title: &str, default: &str| json!({"id": id, "title": title, "type": "string", "default": default, "required": true});
    serde_json::from_value(json!([
        persona("refiner", "Refiner persona", "Refiner"),
        persona("coder", "Coder persona", "Coder"),
        persona("reviewer", "Reviewer persona", "Reviewer"),
        persona("qa", "QA persona", "QA"),
        who("refineBy", "Who refines"),
        who("codeBy", "Who codes"),
        who("reviewBy", "Who reviews"),
        who("qaBy", "Who does QA"),
        {"id": "afterRefine", "title": "A refined ticket goes to", "type": "choice", "choices": ["todo", "ready"],
         "default": "todo",
         "description": "todo: on the board (auto-start takes it when the project's policy says so); ready: the backlog."},
        {"id": "accept", "title": "Who accepts", "type": "choice", "choices": ["user", "auto"], "default": "user",
         "description": "user: you accept from the Inbox before the merge; auto: it merges once QA passes (unless the \
                         project needs a person's approval)."},
        {"id": "autoAcceptLabels", "title": "Labels accepted automatically", "type": "list", "default": [],
         "description": "With user accepting: a ticket with any of these labels (lower case, as it had them when it \
                         started) skips Accept and merges once QA passes; every other ticket waits for you at Accept."},
        {"id": "merge", "title": "Who merges", "type": "choice", "choices": ["workflow", "person"],
         "default": "workflow",
         "description": "workflow: the run merges after Accept (auto-merge of the pull request, or a local merge); \
                         person: it waits until someone merges, in PDT or on GitHub."},
        {"id": "loopLimit", "title": "Passes before asking", "type": "number", "default": 3,
         "description": "How many times Code may run again before the Inbox asks."},
    ]))
    .expect("the pipeline's params")
}

fn node(id: &str, kind: &str, x: i32, y: i32, title: &str, config: Value) -> Value {
    let mut n = json!({"id": id, "kind": kind, "version": 1, "position": {"x": x, "y": y}});
    if !title.is_empty() {
        n["title"] = json!(title);
    }
    if !config.is_null() {
        n["config"] = config;
    }
    n
}

fn edge(from: &str, port: &str, to: &str) -> Value {
    if port.is_empty() { json!({"from": from, "to": to}) } else { json!({"from": from, "port": port, "to": to}) }
}

/// This run's attempt merging (by anyone, anywhere).
fn merged_event() -> Value {
    json!({"type": "attempt.merged", "where": [{"left": "{{event.attemptId}}", "op": "eq", "right": "{{trigger.attemptId}}"}]})
}

/// CI on this run's attempt passing or failing: on the head the merge was refused at (also one logged before the wait
/// began, or while GitHub couldn't be read), or on a newer head while the merge waits (`mergeWaiting`).
fn ci_event() -> Value {
    json!({"type": "ci.run_passed", "also": ["ci.run_failed"], "where": [
        {"left": "{{event.attemptId}}", "op": "eq", "right": "{{trigger.attemptId}}"}],
        "any": [
        {"left": "{{event.headSha}}", "op": "eq", "right": "{{steps.merge.headSha}}"},
        {"left": "{{event.mergeWaiting}}", "op": "truthy"}]})
}

pub fn doc() -> WorkflowDoc {
    let mut nodes = vec![
        // Refine: Draft → Ready.
        node("refine_requested", "ticket.refine_requested", 0, 0, "", Value::Null),
        node(
            "refine",
            "agent",
            300,
            0,
            "Refine",
            json!({
            "persona": "{{params.refiner}}", "prompt": REFINE_PROMPT, "session": "new",
            "owner": "{{params.refineBy}}", "pipelineStage": "refine",
            "outcomes": [
                {"id": "ready", "description": "the ticket can be built as written"},
                {"id": "needs_clarification", "description": "it can't be built until the user decides something you could not ask"},
            ]}),
        ),
        node(
            "queue",
            "ticket.move",
            600,
            0,
            "Move to the queue",
            json!({"ticket": "{{trigger.ticketId}}", "stage": "{{params.afterRefine}}"}),
        ),
        node(
            "clarify",
            "gate",
            600,
            140,
            "Clarify",
            json!({
            "question": "The Refiner needs you on {{trigger.ticketKey}}: {{steps.refine.summary}} Answer in the \
                         ticket (comments or its description), then refine again.",
            "choices": ["refine_again", "stop"], "pipelineStage": "refine"}),
        ),
        node("refined", "end", 900, 0, "Refined", Value::Null),
        node("left", "end", 900, 140, "Left as it is", json!({"quiet": true})),
        // Code → Review → QA → Accept → merge.
        node("started", "attempt.started", 0, 400, "", Value::Null),
        node(
            "code",
            "agent",
            300,
            400,
            "Code",
            json!({
            "persona": "{{params.coder}}", "prompt": CODE_PROMPT, "session": "reuse",
            "owner": "{{params.codeBy}}", "pipelineStage": "code", "loopLimit": "{{params.loopLimit}}",
            "maxRuntimeMs": 8 * 60 * 60 * 1000,
            "outcomes": [{"id": "submitted", "description": "the change is built, checked and committed"}]}),
        ),
        node(
            "review",
            "agent",
            600,
            400,
            "Review",
            json!({
            "persona": "{{params.reviewer}}", "prompt": REVIEW_PROMPT, "session": "new",
            "owner": "{{params.reviewBy}}", "pipelineStage": "review",
            "outcomes": [
                {"id": "approved", "description": "it can merge as is"},
                {"id": "changes_requested", "description": "something must change first"},
            ]}),
        ),
        node(
            "qa",
            "agent",
            900,
            400,
            "QA",
            json!({
            "persona": "{{params.qa}}", "prompt": QA_PROMPT, "session": "new",
            "owner": "{{params.qaBy}}", "pipelineStage": "qa",
            "outcomes": [
                {"id": "pass", "description": "it works as the ticket asks"},
                {"id": "fail", "description": "something doesn't work: say what and how to reproduce it"},
            ]}),
        ),
        node(
            "accept_mode",
            "switch",
            1200,
            400,
            "Who accepts",
            json!({
            "value": "{{params.accept}}", "cases": [{"port": "user", "equals": "user"}, {"port": "auto", "equals": "auto"}]}),
        ),
        node(
            "auto_accept",
            "condition",
            1350,
            250,
            "Accepted by label?",
            json!({"conditions": [
                {"left": "{{trigger.labels}}", "op": "contains_any", "right": "{{params.autoAcceptLabels}}"}]}),
        ),
        node(
            "accept",
            "gate",
            1500,
            400,
            "Accept",
            json!({
            "question": "Accept {{trigger.ticketKey}} and merge it?", "choices": ["merge", "reject"],
            "inbox": "accept", "pipelineStage": "accept", "approve": "merge"}),
        ),
        node(
            "merge_mode",
            "switch",
            1800,
            400,
            "Who merges",
            json!({
            "value": "{{params.merge}}",
            "cases": [{"port": "workflow", "equals": "workflow"}, {"port": "person", "equals": "person"}]}),
        ),
        node(
            "merge",
            "attempt.merge",
            2100,
            400,
            "Merge",
            json!({"attempt": "{{trigger.attemptId}}", "finishOnMerge": true, "pipelineStage": "merge"}),
        ),
        // A refused merge: CI still running waits for it (PDT-151); the Coder fixes what an agent can (a conflict,
        // failing CI; then a Reviewer checks the fix); anything else asks you.
        node(
            "merge_why",
            "switch",
            2100,
            100,
            "Waiting for CI?",
            json!({"value": "{{steps.merge.code}}", "cases": [{"port": "ci_pending", "equals": "ci_pending"}]}),
        ),
        node(
            "wait_ci",
            "wait",
            2400,
            -60,
            "Wait for CI",
            json!({"event": ci_event(), "timeoutMs": CI_WAIT_MS, "pipelineStage": "merge"}),
        ),
        node(
            "fixable",
            "condition",
            2100,
            250,
            "Can the Coder fix it?",
            json!({"conditions": [{"left": "{{steps.merge.fixable}}", "op": "truthy"}]}),
        ),
        node(
            "fix",
            "agent",
            2400,
            100,
            "Fix the merge",
            json!({
            "persona": "{{params.coder}}", "prompt": FIX_PROMPT, "session": "reuse",
            "owner": "{{params.codeBy}}", "pipelineStage": "code", "loopLimit": "{{params.loopLimit}}",
            "outcomes": [{"id": "submitted", "description": "the fix is committed and pushed"}]}),
        ),
        node(
            "fix_review",
            "agent",
            2700,
            100,
            "Review the fix",
            json!({
            "persona": "{{params.reviewer}}", "prompt": FIX_REVIEW_PROMPT, "session": "new",
            "owner": "{{params.reviewBy}}", "pipelineStage": "review",
            "outcomes": [
                {"id": "approved", "description": "the fix can merge as is"},
                {"id": "changes_requested", "description": "something must change first"},
            ]}),
        ),
        node(
            "blocked",
            "gate",
            2400,
            250,
            "Merge blocked",
            json!({
            "question": "Merge blocked on {{trigger.ticketKey}}: {{steps.merge.reason}}. Clear it (or merge it \
                         yourself), then merge again.",
            "choices": ["merge_again", "stop"], "approve": "merge_again", "pipelineStage": "merge_blocked",
            "timeoutMs": MERGE_WAIT_MS, "event": merged_event()}),
        ),
        node(
            "finish",
            "ticket.finish",
            2400,
            400,
            "Finish",
            json!({"ticket": "{{trigger.ticketId}}", "stopRunning": true}),
        ),
        node(
            "wait_merge",
            "wait",
            2100,
            560,
            "Wait for the merge",
            json!({"event": merged_event(), "timeoutMs": MERGE_WAIT_MS, "pipelineStage": "merge"}),
        ),
        node(
            "not_merged",
            "gate",
            2400,
            560,
            "Still not merged",
            json!({
            "question": "{{trigger.ticketKey}} passed and is waiting for someone to merge it.",
            "choices": ["keep_waiting", "merge", "stop"], "approve": "merge", "pipelineStage": "merge",
            "event": merged_event()}),
        ),
        node("done", "end", 2700, 400, "Done", Value::Null),
        // An agent step out of time or stopped at its loop limit.
        node(
            "stopped",
            "ticket.comment",
            1200,
            700,
            "Note where it stopped",
            json!({
            "ticket": "{{trigger.ticketId}}",
            "body": "The pipeline stopped here: a step ran out of time or was stopped at its loop limit. Its run is \
                     in Workflows → Pipeline → Runs."}),
        ),
        node("stopped_end", "end", 1500, 700, "Stopped", Value::Null),
    ];
    // GitHub not answering (`merge_failed`, `ci_unreadable`, PDT-151): a few more tries before the run fails.
    if let Some(m) = nodes.iter_mut().find(|n| n["id"] == "merge") {
        m["retry"] = json!({"max": 3, "backoffMs": 2 * 60 * 1000});
    }
    let edges = vec![
        edge("refine_requested", "", "refine"),
        edge("refine", "ready", "queue"),
        edge("refine", "needs_clarification", "clarify"),
        edge("refine", "timeout", "stopped"),
        edge("refine", "stopped", "stopped"),
        edge("queue", "", "refined"),
        edge("clarify", "refine_again", "refine"),
        edge("clarify", "stop", "left"),
        edge("clarify", "timeout", "left"),
        edge("clarify", "unanswered", "left"),
        edge("started", "", "code"),
        edge("code", "submitted", "review"),
        edge("code", "timeout", "stopped"),
        edge("code", "stopped", "stopped"),
        edge("review", "approved", "qa"),
        edge("review", "changes_requested", "code"),
        edge("review", "timeout", "stopped"),
        edge("review", "stopped", "stopped"),
        edge("qa", "pass", "accept_mode"),
        edge("qa", "fail", "code"),
        edge("qa", "timeout", "stopped"),
        edge("qa", "stopped", "stopped"),
        edge("accept_mode", "user", "auto_accept"),
        edge("auto_accept", "true", "merge_mode"),
        edge("auto_accept", "false", "accept"),
        edge("accept_mode", "auto", "merge_mode"),
        edge("accept_mode", "default", "accept"),
        edge("accept", "merge", "merge_mode"),
        edge("accept", "reject", "code"),
        edge("accept", "timeout", "stopped"),
        edge("accept", "unanswered", "stopped"),
        edge("merge_mode", "workflow", "merge"),
        edge("merge_mode", "person", "wait_merge"),
        edge("merge_mode", "default", "wait_merge"),
        edge("merge", "merged", "finish"),
        edge("merge", "queued", "done"),
        edge("merge", "refused", "merge_why"),
        edge("merge_why", "ci_pending", "wait_ci"),
        edge("merge_why", "default", "fixable"),
        edge("wait_ci", "event", "merge"),
        edge("wait_ci", "timeout", "blocked"),
        edge("fixable", "true", "fix"),
        edge("fixable", "false", "blocked"),
        edge("fix", "submitted", "fix_review"),
        edge("fix", "timeout", "stopped"),
        edge("fix", "stopped", "stopped"),
        edge("fix_review", "approved", "merge"),
        edge("fix_review", "changes_requested", "fix"),
        edge("fix_review", "timeout", "stopped"),
        edge("fix_review", "stopped", "stopped"),
        edge("blocked", "merge_again", "merge"),
        edge("blocked", "stop", "left"),
        edge("blocked", "event", "done"),
        edge("blocked", "timeout", "stopped"),
        edge("blocked", "unanswered", "stopped"),
        edge("finish", "finished", "done"),
        edge("finish", "blocked", "done"),
        edge("wait_merge", "event", "done"),
        edge("wait_merge", "timeout", "not_merged"),
        edge("not_merged", "keep_waiting", "wait_merge"),
        edge("not_merged", "merge", "merge"),
        edge("not_merged", "stop", "left"),
        edge("not_merged", "event", "done"),
        edge("not_merged", "timeout", "stopped"),
        edge("not_merged", "unanswered", "stopped"),
        edge("stopped", "", "stopped_end"),
    ];
    serde_json::from_value(json!({
        "name": "Pipeline",
        "description": "Draft → Refine → Ready → Code → Review → QA → Accept → Done. Refine (on a ticket) runs the \
                        Refiner; starting a ticket runs the Coder, then a fresh Reviewer and QA, sending it back to the \
                        Coder on changes or a failure (findings first, PR notices after them; it submits once its \
                        work is committed and pushed); then you accept and the run merges once CI on the pull \
                        request passes. A merge refused for a conflict or failing CI goes back to the Coder and a \
                        Reviewer; one refused for anything else asks you in the Inbox. Any stage can be yours instead of an agent's.",
        "nodes": nodes,
        "edges": edges,
        "settings": {"maxSteps": 200, "maxRuntimeMs": 30_i64 * 24 * 60 * 60 * 1000},
    }))
    .expect("the pipeline is a valid document")
}
