//! Built-in workflows (Phase 7a spec §8): the reactions that used to be hard-coded, installed host-wide when pdtd
//! starts. A built-in the user never edited follows newer templates; one the user edited is left alone. They can be
//! disabled, not deleted.

use crate::routes::{AppState, now_ms};
use pdt_core::workflow::api::{Author, Scope, Source, Workflow};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::template::Category;
use serde_json::json;

#[derive(Clone, Copy)]
pub struct Builtin {
    pub id: &'static str,
    /// Enabled when first installed.
    pub enabled: bool,
    /// Its group in the New workflow gallery (PDT-229).
    pub category: Category,
    pub doc: fn() -> WorkflowDoc,
}

pub const PR_NOTICES: &str = "pr-notices";
pub const FINISH_MERGED: &str = "finish-merged";
pub const COMMENTS_TO_AGENT: &str = "comments-to-agent";
pub const BASE_MOVED: &str = "base-moved";
pub const PR_FEEDBACK: &str = "pr-feedback";
pub const LATE_COMMITS: &str = "late-commits";
pub const GITHUB_ISSUE_DONE: &str = "github-issue-done";
pub const PICK_UP_TICKETS: &str = "pick-up-tickets";
/// The Cut a release template (without `builtin:`); a built-in workflow until PDT-301 ([`RETIRED`]).
pub const CUT_RELEASE: &str = "cut-release";

pub const BUILTINS: &[Builtin] = &[
    Builtin { id: PICK_UP_TICKETS, enabled: true, category: Category::Tickets, doc: pick_up_tickets },
    Builtin { id: PR_NOTICES, enabled: true, category: Category::CiPr, doc: pr_notices },
    Builtin { id: FINISH_MERGED, enabled: true, category: Category::Tickets, doc: finish_merged },
    Builtin { id: LATE_COMMITS, enabled: true, category: Category::CiPr, doc: late_commits },
    Builtin { id: COMMENTS_TO_AGENT, enabled: true, category: Category::Tickets, doc: comments_to_agent },
    Builtin { id: BASE_MOVED, enabled: false, category: Category::CiPr, doc: base_moved },
    Builtin { id: PR_FEEDBACK, enabled: false, category: Category::CiPr, doc: pr_feedback },
];

/// The GitHub plugin's built-ins ([`crate::integrations::Integration::builtin_workflows`]): left out while it is off
/// (PDT-251).
pub const GITHUB_BUILTINS: &[Builtin] =
    &[Builtin { id: GITHUB_ISSUE_DONE, enabled: true, category: Category::Issues, doc: github_issue_done }];

/// PDT's built-ins, then the enabled integrations' ([`crate::integrations::Integration::builtin_workflows`]).
pub fn all(reg: &crate::integrations::Registry) -> Vec<Builtin> {
    BUILTINS.iter().copied().chain(reg.builtin_workflows()).collect()
}

/// Whether a stored workflow is the built-in of an integration that is turned off (or not in this build; PDT-173): an
/// install from before integrations could be turned off has Jira's, enabled. It can't run (its kinds aren't
/// registered) nor be deleted, so it is left out of the workflow list and the engine until its integration is on
/// again, when it comes back as it was.
pub fn hidden(reg: &crate::integrations::Registry, w: &Workflow) -> bool {
    w.builtin.as_deref().is_some_and(|id| !all(reg).iter().any(|b| b.id == id))
}

fn doc(v: serde_json::Value) -> WorkflowDoc {
    serde_json::from_value(v).expect("a built-in workflow is a valid document")
}

/// How many times PR notices puts an agent on one attempt's failed merge before it stops and comments (PDT-104).
pub const FIX_TRIES: u32 = 3;

const FIX_PROMPT: &str = "{{trigger.notice.text}}\n\nThis pull request was set to merge (auto-merge, a merge queue, \
or PDT once CI passes), so get it merging again: fix the cause, run the checks, commit and push. Once you push, PDT \
sets it to merge again if it isn't: don't turn on auto-merge or merge it yourself, whatever the notice above says.\n\nMost fixes are small (a CI fix, resolving a conflict, merging the base \
in): push and submit `submitted`. If the fix is a large change to the work (new behaviour, a redesign, a big rewrite), \
commit it but don't push, and submit `review`: a Reviewer looks at it first, then you are asked to push.";

const FIX_REVIEW_PROMPT: &str = "This attempt's pull request was set to merge and failed (CI, a conflict or the merge \
queue), and the Coder's fix is large enough to want a review before it merges. Review the fix: the commits not pushed \
yet (`git log @{u}..`) and the branch's diff against its base, against the ticket. Put each finding on its line \
(`comments` in `submit_outcome`).";

const PUSH_PROMPT: &str = "The Reviewer approved your fix. Push it (run the checks first if anything changed since); \
PDT then gets the pull request merging again.";

/// The built-in PR notices (PDT-25, Phase 7a; PDT-104): each notice once to the attempt's agent; for a pull request
/// that was set to merge (or whose workflow merge was refused for a conflict, PDT-139), an agent step that fixes it (a
/// Reviewer first when the agent calls the fix large), then auto-merge on again, up to [`FIX_TRIES`] times per
/// attempt. A notice with no agent session to tell goes to the Inbox and the ticket's comments, once (PDT-139).
fn pr_notices() -> WorkflowDoc {
    let node = |id: &str, kind: &str, x: i32, y: i32, title: &str, config: serde_json::Value| {
        let mut n = json!({"id": id, "kind": kind, "version": 1, "position": {"x": x, "y": y}});
        if !title.is_empty() {
            n["title"] = json!(title);
        }
        if !config.is_null() {
            n["config"] = config;
        }
        n
    };
    let cond = |left: &str, op: &str, right: Option<serde_json::Value>| match right {
        Some(r) => json!({"left": left, "op": op, "right": r}),
        None => json!({"left": left, "op": op}),
    };
    let comment = |body: &str| json!({"ticket": "{{trigger.ticketId}}", "body": body});
    let null = serde_json::Value::Null;
    let nodes = vec![
        // A workflow's merge waiting for this CI handles its failure itself (PDT-151).
        node("ci", "ci.run_failed", 0, 0, "", json!({"where": [cond("{{trigger.mergeWaiting}}", "falsy", None)]})),
        node("conflict", "pr.conflict", 0, 100, "", null.clone()),
        node("queue", "pr.queue_left", 0, 200, "", null.clone()),
        node("behind", "pr.behind", 0, 300, "", null.clone()),
        node("pr_merged", "pr.merged", 0, 400, "", null.clone()),
        node(
            "merged",
            "attempt.merged",
            0,
            500,
            "",
            json!({"where": [cond("{{trigger.via}}", "ne", Some(json!("closed")))]}),
        ),
        node(
            "passed",
            "ci.run_passed",
            0,
            700,
            "",
            json!({"where": [
            cond("{{trigger.pr}}", "exists", None),
            cond("{{trigger.mergeRequested}}", "truthy", None),
            cond("{{trigger.autoMerge}}", "falsy", None),
            cond("{{trigger.inMergeQueue}}", "falsy", None),
            cond("{{trigger.agentWorking}}", "falsy", None)]}),
        ),
        node(
            "asked",
            "condition",
            280,
            150,
            "Set to merge?",
            json!({"match": "any", "conditions": [
            cond("{{trigger.mergeRequested}}", "truthy", None),
            cond("{{trigger.mergeRefused}}", "truthy", None)]}),
        ),
        node(
            "busy",
            "attempt.agent_working",
            560,
            150,
            "An agent on it already?",
            json!({"attempt": "{{trigger.attemptId}}", "known": "{{trigger.agentWorking}}"}),
        ),
        node("count", "state.set", 840, 150, "Count the try", json!({"key": "tries.{{trigger.attemptId}}", "add": 1})),
        node(
            "over",
            "condition",
            1120,
            150,
            "Out of tries?",
            json!({"conditions": [
            cond("{{steps.count.value}}", "gt", Some(json!(FIX_TRIES)))]}),
        ),
        node(
            "first_over",
            "condition",
            1400,
            300,
            "Just now?",
            json!({"conditions": [
            cond("{{steps.count.value}}", "eq", Some(json!(FIX_TRIES + 1)))]}),
        ),
        node(
            "limit_note",
            "ticket.comment",
            1680,
            300,
            "Note the limit",
            comment(&format!(
                "PR notices stopped getting PR #{{{{trigger.pr}}}} merging again after {FIX_TRIES} tries: it failed again \
             ({{{{trigger.notice.text}}}}). Fix it and merge it yourself; its later notices still go to the agent."
            )),
        ),
        node(
            "fix",
            "agent",
            1400,
            0,
            "Fix it",
            json!({
            "persona": "Coder", "prompt": FIX_PROMPT, "session": "reuse",
            "outcomes": [
                {"id": "submitted", "description": "the fix is pushed (a small fix: CI, a conflict, the base merged in)"},
                {"id": "review", "description": "the fix is large: committed but not pushed, for a Reviewer first"},
            ]}),
        ),
        node(
            "review",
            "agent",
            1680,
            -140,
            "Review the fix",
            json!({
            "persona": "Reviewer", "prompt": FIX_REVIEW_PROMPT, "session": "own",
            "outcomes": [
                {"id": "approved", "description": "the fix can merge as is"},
                {"id": "changes_requested", "description": "something must change first"},
            ]}),
        ),
        node(
            "push",
            "agent",
            1960,
            -140,
            "Push it",
            json!({
            "persona": "Coder", "prompt": PUSH_PROMPT, "session": "reuse",
            "outcomes": [{"id": "submitted", "description": "the fix is pushed"}]}),
        ),
        node(
            "remerge",
            "attempt.merge",
            2240,
            0,
            "Merge again",
            json!({"attempt": "{{trigger.attemptId}}", "again": true}),
        ),
        // Merged again before the fix's CI passed (PDT-151): set to merge, so CI passing merges it.
        node(
            "ci_wait",
            "condition",
            2520,
            140,
            "Waiting for CI?",
            json!({"match": "any", "conditions": [
            cond("{{steps.remerge.code}}", "eq", Some(json!("ci_pending"))),
            cond("{{steps.remerge.code}}", "eq", Some(json!("ci_failed")))]}),
        ),
        node("after_ci", "end", 2800, 280, "Merges once CI passes", json!({"quiet": true})),
        node(
            "refused_note",
            "ticket.comment",
            2800,
            140,
            "Note the refusal",
            comment("PR notices could not turn auto-merge back on for PR #{{trigger.pr}}: {{steps.remerge.reason}}"),
        ),
        node("merging", "end", 2520, 0, "Merging again", null.clone()),
        node(
            "stuck",
            "ticket.comment",
            1960,
            140,
            "Note where it stopped",
            comment(
                "PR notices stopped fixing PR #{{trigger.pr}} for its merge: a step ran out of time or was stopped at its \
             loop limit. Its run is in Workflows → PR notices → Runs.",
            ),
        ),
        node(
            "tell",
            "session.send",
            1680,
            450,
            "Tell the agent",
            json!({
            "attempt": "{{trigger.attemptId}}", "text": "{{trigger.notice.text}}",
            "dedupe": "{{trigger.notice.key}}", "timeline": "pr"}),
        ),
        // Nobody to tell (PDT-139): a conflict, CI failure, queue drop or base move goes to the user instead.
        node(
            "unheard",
            "condition",
            1960,
            450,
            "Needs someone?",
            json!({"conditions": [
            cond("{{trigger.notice.kind}}", "exists", None),
            cond("{{trigger.notice.kind}}", "ne", Some(json!("merged")))]}),
        ),
        node(
            "inbox",
            "inbox.notice",
            2240,
            450,
            "Put it in the Inbox",
            json!({"attempt": "{{trigger.attemptId}}", "text": "{{trigger.notice.text}}", "key": "{{trigger.notice.key}}"}),
        ),
        node(
            "unheard_note",
            "ticket.comment",
            2520,
            450,
            "Note it on the ticket",
            comment(
                "PR notices had no agent session to tell, so this waits for you (it is in the Inbox):\n\n\
                 {{trigger.notice.text}}",
            ),
        ),
        node("heard", "end", 2240, 600, "Nothing to do", json!({"quiet": true})),
    ];
    let e = |from: &str, port: &str, to: &str| {
        if port.is_empty() { json!({"from": from, "to": to}) } else { json!({"from": from, "port": port, "to": to}) }
    };
    let mut edges: Vec<_> = ["ci", "conflict", "queue", "behind"].iter().map(|f| e(f, "", "asked")).collect();
    edges.extend([
        e("pr_merged", "", "tell"),
        e("merged", "", "tell"),
        e("passed", "", "remerge"),
        e("asked", "true", "busy"),
        e("asked", "false", "tell"),
        e("busy", "yes", "tell"),
        e("busy", "no", "count"),
        e("count", "", "over"),
        e("over", "true", "first_over"),
        e("over", "false", "fix"),
        e("first_over", "true", "limit_note"),
        e("first_over", "false", "tell"),
        e("limit_note", "", "tell"),
        e("fix", "submitted", "remerge"),
        e("fix", "review", "review"),
        e("fix", "timeout", "stuck"),
        e("fix", "stopped", "stuck"),
        e("review", "approved", "push"),
        e("review", "changes_requested", "fix"),
        e("review", "timeout", "stuck"),
        e("review", "stopped", "stuck"),
        e("push", "submitted", "remerge"),
        e("push", "timeout", "stuck"),
        e("push", "stopped", "stuck"),
        e("remerge", "queued", "merging"),
        e("remerge", "merged", "merging"),
        e("remerge", "refused", "ci_wait"),
        e("ci_wait", "true", "after_ci"),
        e("ci_wait", "false", "refused_note"),
        e("tell", "untold", "unheard"),
        e("unheard", "true", "inbox"),
        e("unheard", "false", "heard"),
        e("inbox", "added", "unheard_note"),
    ]);
    doc(json!({
        "name": "PR notices",
        "description": "Tells the attempt's agent when CI fails, its pull request conflicts with the base, the merge \
                        queue drops it, or it merges. Each notice once; it waits until the agent is idle and nobody \
                        is typing, and lands on the ticket's timeline; one that comes while a Coder's pass sent \
                        back by review or QA waits goes in with it, after the findings. A pull request that was set to merge gets an \
                        agent step instead (the attempt's Coder session, or a new one): it fixes the cause and pushes, \
                        with a Reviewer first when it calls the fix large, and auto-merge goes back on, also when CI \
                        passes again with it off; a workflow's merge refused for a conflict counts too. After 3 tries \
                        on one attempt it stops and comments on the ticket. A notice with no agent session to tell \
                        goes to the Inbox and the ticket instead, once; the Inbox item closes when its cause is gone.",
        "nodes": nodes,
        "edges": edges,
    }))
}

/// Auto-start as a workflow (PDT-327): when a ticket can start, room frees up, or every minute, the candidates of the
/// project (every project, for the sweep) are filtered and ordered, choose keeps to the caps and the start delay, and
/// Start takes the first of each project still free. Every field it leaves out is the project's auto-start setting
/// (Settings), so it does nothing in a project whose auto-start is off.
fn pick_up_tickets() -> WorkflowDoc {
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

fn finish_merged() -> WorkflowDoc {
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

const FOLLOW_UP: &str = "{{trigger.summary}}.\n\nPR #{{trigger.pr}} ({{trigger.url}}) merged {{trigger.ticketKey}} \
into {{trigger.baseBranch}} at {{trigger.mergedHead}}, and these commits on {{trigger.branch}} came after it, so \
they are not in {{trigger.baseBranch}}:\n\n{{trigger.list}}\n\nCherry-pick them onto this ticket's branch (`git \
cherry-pick {{trigger.mergedHead}}..{{trigger.head}}`; the branch {{trigger.branch}} is kept for this), check that \
they still make sense on {{trigger.baseBranch}}, run the checks and open a pull request.\n\n## Done when\n- What \
those commits change is in {{trigger.baseBranch}}, or found not to be needed there.";

/// Commits pushed to an attempt's branch after its pull request merged (PDT-151): the attempt's agent hears that its
/// push came too late, and the Inbox asks the user what becomes of them: a follow-up ticket that cherry-picks them
/// (the ticket finishes, keeping the branch), discard them and finish (after confirming), or keep the ticket open.
fn late_commits() -> WorkflowDoc {
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

fn comments_to_agent() -> WorkflowDoc {
    doc(json!({
        "name": "Ticket comments to the agent",
        "description": "A comment on a ticket that is being worked on goes to the attempt's agent session, once it is \
                        idle (a stopped or interrupted conversation reopens to take it).",
        "nodes": [
            {"id": "commented", "kind": "ticket.commented", "version": 1, "position": {"x": 0, "y": 0}, "config": {"where": [
                {"left": "{{trigger.attemptId}}", "op": "exists"},
                {"left": "{{trigger.actor}}", "op": "eq", "right": "user"},
                {"left": "{{trigger.workflowId}}", "op": "missing"}]}},
            {"id": "tell", "kind": "session.send", "version": 1, "position": {"x": 320, "y": 0}, "config": {
                "attempt": "{{trigger.attemptId}}", "reopen": true,
                "text": "[PDT] New comment on {{trigger.ticketKey}} from the user:\n\n{{trigger.body}}"}},
        ],
        "edges": [{"from": "commented", "to": "tell"}],
    }))
}

fn base_moved() -> WorkflowDoc {
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

fn pr_feedback() -> WorkflowDoc {
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

/// A ticket linked to a GitHub issue that finishes closes it (PDT-154): Done as completed, Won't do as not planned,
/// each with a comment, once per ticket (the run key).
fn github_issue_done() -> WorkflowDoc {
    let trigger = |to: &str| {
        json!({"runKey": "{{trigger.ticketId}}", "where": [
            {"left": "{{trigger.to}}", "op": "eq", "right": to},
            {"left": "{{trigger.githubIssue}}", "op": "exists"}]})
    };
    doc(json!({
        "name": "Done closes the GitHub issue",
        "description": "Moving a ticket linked to a GitHub issue to Done closes the issue as completed, with a comment \
                        naming the ticket; Won't do closes it as not planned. Once per ticket: moving it again \
                        changes nothing on GitHub. An issue closed already is left as it is.",
        "nodes": [
            {"id": "done", "kind": "ticket.stage_changed", "version": 1, "position": {"x": 0, "y": 0},
             "config": trigger("done")},
            {"id": "wont_do", "kind": "ticket.stage_changed", "version": 1, "position": {"x": 0, "y": 160},
             "config": trigger("wont_do")},
            {"id": "fixed", "kind": "github.close", "version": 1, "position": {"x": 320, "y": 0},
             "title": "Close as completed", "config": {"ticket": "{{trigger.ticketId}}", "reason": "completed",
                "comment": "Fixed in {{trigger.ticketKey}}; it ships in the next PDT release."}},
            {"id": "dropped", "kind": "github.close", "version": 1, "position": {"x": 320, "y": 160},
             "title": "Close as not planned", "config": {"ticket": "{{trigger.ticketId}}", "reason": "not_planned",
                "comment": "{{trigger.ticketKey}} won't be done, so this is closed as not planned."}},
            {"id": "end", "kind": "end", "version": 1, "position": {"x": 640, "y": 80}, "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "done", "to": "fixed"},
            {"from": "wont_do", "to": "dropped"},
            {"from": "fixed", "port": "unlinked", "to": "end"},
            {"from": "dropped", "port": "unlinked", "to": "end"},
        ],
    }))
}

/// How often "Cut a release" looks, and for how long: the release PR (opened within a minute or two), its merge (it
/// may merge itself once its checks and a merge queue pass: hours; a failed check waits for the user's fix, up to two
/// weeks) and the published release (bundles may build for an hour or more; allowed four).
const PR_POLL_MS: u64 = 30_000;
const PR_POLLS: u32 = 40;
const MERGE_POLL_MS: u64 = 5 * 60_000;
const MERGE_POLLS: u32 = 14 * 24 * 12;
const RELEASE_POLL_MS: u64 = 2 * 60_000;
const RELEASE_POLLS: u32 = 4 * 30;

/// Cut a release (PDT-194, PDT-214, PDT-294; a template since PDT-301): made for a project from its release workflow
/// (`workflow`, run on `ref`), it asks Run now for the kind of release and follows what the workflow makes:
/// - a stable release runs the workflow with `stableInputs` (`{{trigger.bump}}` or `{{trigger.version}}` in them).
///   With `releaseBranch` (the branch the workflow opens its release PR from), an open one stops a new release, and the
///   run says when the PR is open, puts a failed check in the Inbox and waits for the merge; without it, there is no PR
///   stage. `tagPattern` is its tag (none: the latest release);
/// - the other kind (named `preName`, e.g. Nightly) runs it with `preInputs` and has no PR; `preTagPattern` is its tag.
///
/// It says when the release is published. PDT merges and tags nothing: the workflow and the forge do.
pub(crate) fn cut_release_template() -> pdt_core::workflow::template::Template {
    use super::templates::{builtin, params};
    // A GitHub read that fails (the network, a rate limit) is tried again before the run fails.
    let retry = json!({"max": 5, "backoffMs": 60_000});
    let wait = |id: &str, x: i32, y: i32, title: &str, ms: u64, visits: u32| {
        json!({"id": id, "kind": "wait", "version": 1, "position": {"x": x, "y": y}, "title": title,
               "maxVisits": visits, "config": {"ms": ms}})
    };
    let run_workflow = |id: &str, x: i32, y: i32, inputs: &str| {
        json!({"id": id, "kind": "github.workflow.run", "version": 1, "position": {"x": x, "y": y},
               "title": "Run the release workflow", "config": {"workflow": "{{params.workflow}}",
               "ref": "{{params.ref}}", "inputs": inputs}})
    };
    let release = |id: &str, x: i32, y: i32, title: &str, tag: &str, extra: serde_json::Value| {
        let mut config = json!({"tagPattern": tag});
        config.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        json!({"id": id, "kind": "github.release", "version": 1, "position": {"x": x, "y": y}, "title": title,
               "retry": retry, "config": config})
    };
    // A release without a PR (a stable one without releaseBranch, or the other kind): the workflow publishes it.
    let direct = |p: &str, y: i32, inputs: &str, tag: &str| {
        let id = |s: &str| format!("{p}{s}");
        let mut published = release(
            &id("published"),
            1040,
            y,
            "Release published?",
            tag,
            json!({"newerThan": format!("{{{{steps.{p}before.tag}}}}")}),
        );
        published["maxVisits"] = json!(RELEASE_POLLS);
        let out = |field: &str| format!("{{{{steps.{p}published.{field}}}}}");
        let nodes = vec![
            release(&id("before"), 260, y, "Newest release before", tag, json!({})),
            run_workflow(&id("dispatch"), 520, y, inputs),
            wait(&id("wait"), 780, y, "Wait for the release", RELEASE_POLL_MS, RELEASE_POLLS),
            published,
            json!({"id": id("notify"), "kind": "notify", "version": 1, "position": {"x": 1300, "y": y},
             "title": "Tell me it is out", "config": {
                "title": format!("Release {} is published", out("tag")),
                "body": format!("{} is out.", out("name")),
                "link": out("url")}}),
            json!({"id": id("tell"), "kind": "inbox.notice", "version": 1, "position": {"x": 1560, "y": y},
             "title": "Inbox: published", "config": {
                "key": format!("released-{}", out("tag")),
                "title": format!("Release {} is published", out("tag")),
                "text": format!("[{}]({}) is published.", out("name"), out("url"))}}),
            json!({"id": id("released"), "kind": "end", "version": 1, "position": {"x": 1820, "y": y},
             "title": "Released", "config": {"output": {"tag": out("tag"), "url": out("url")}}}),
        ];
        let edges = vec![
            json!({"from": id("before"), "port": "found", "to": id("dispatch")}),
            json!({"from": id("before"), "port": "none", "to": id("dispatch")}),
            json!({"from": id("dispatch"), "to": id("wait")}),
            json!({"from": id("wait"), "port": "done", "to": id("published")}),
            json!({"from": id("published"), "port": "found", "to": id("notify")}),
            json!({"from": id("published"), "port": "none", "to": id("wait")}),
            json!({"from": id("notify"), "to": id("tell")}),
            json!({"from": id("tell"), "port": "added", "to": id("released")}),
            json!({"from": id("tell"), "port": "skipped", "to": id("released")}),
        ];
        (nodes, edges)
    };
    let (direct_nodes, direct_edges) = direct("direct_", -260, "{{params.stableInputs}}", "{{params.tagPattern}}");
    let (pre_nodes, pre_edges) = direct("pre_", -520, "{{params.preInputs}}", "{{params.preTagPattern}}");
    let mut published = release(
        "published",
        2340,
        0,
        "Release published?",
        "{{params.tagPattern}}",
        json!({"newerThan": "{{steps.before.tag}}"}),
    );
    published["maxVisits"] = json!(RELEASE_POLLS);
    let start = json!({"id": "start", "kind": "manual", "version": 1, "position": {"x": -520, "y": 0},
        "title": "Cut a release", "config": {
        "inputs": [
            {"id": "type", "label": "Type", "type": "choice", "required": true, "default": "{{params.defaultType}}",
             "choices": [{"value": "prerelease", "label": "{{params.preName}}"}, {"value": "stable", "label": "Stable"}]},
            {"id": "version", "label": "Exact version", "type": "text", "showIf": {"field": "type", "equals": "stable"},
             "help": "X.Y.Z. Leave it empty to bump the version instead."},
            {"id": "bump", "label": "Bump", "type": "choice", "default": "patch",
             "showIf": {"field": "version", "equals": ""},
             "choices": [{"value": "patch", "label": "Patch"}, {"value": "minor", "label": "Minor"},
                         {"value": "major", "label": "Major"}]},
        ],
        "confirm": {"text": "This publishes a stable release ({{trigger.bump}}{{trigger.version}}). Continue?",
                    "showIf": {"field": "type", "equals": "stable"}}}});
    let mut nodes = vec![
        start,
        json!({"id": "kind", "kind": "condition", "version": 1, "position": {"x": -260, "y": 0},
         "title": "Stable?", "config": {"conditions": [{"left": "{{trigger.type}}", "op": "eq", "right": "stable"}]}}),
        json!({"id": "has_pr", "kind": "condition", "version": 1, "position": {"x": 0, "y": 0},
         "title": "A release PR?", "config": {"conditions": [{"left": "{{params.releaseBranch}}", "op": "truthy"}]}}),
    ];
    nodes.extend(direct_nodes);
    nodes.extend(pre_nodes);
    // A release PR: the workflow opens it, and it merges (by itself or by hand), which publishes the release. Its
    // branch comes through a step, so a project without one (these nodes never run) still validates.
    nodes.extend([
        json!({"id": "pr_branch", "kind": "state.set", "version": 1, "position": {"x": 130, "y": 100},
         "title": "The release PR branch", "config": {"key": "releaseBranch", "value": "{{params.releaseBranch}}"}}),
        release("before", 260, 0, "Latest release before", "{{params.tagPattern}}", json!({})),
        // A workflow that opens one release PR at a time refuses while one is open, and that PR isn't the release
        // asked for: say so instead.
        json!({"id": "open_already", "kind": "github.change_request", "version": 1, "position": {"x": 390, "y": 200},
         "title": "A release PR still open?", "retry": retry,
         "config": {"branch": "{{steps.pr_branch.value}}", "dryRunPort": "none"}}),
        json!({"id": "still_open", "kind": "inbox.notice", "version": 1, "position": {"x": 650, "y": 200},
         "title": "Inbox: one is still open", "config": {
            "key": "release-pr-open-{{steps.open_already.number}}",
            "title": "Release PR #{{steps.open_already.number}} is still open",
            "text": "No new release was started: [{{steps.open_already.title}}]({{steps.open_already.url}}) is \
                     still open on {{steps.pr_branch.value}}. Merge it (that release ships) or close it, then \
                     run Cut a release again."}}),
        json!({"id": "not_started", "kind": "end", "version": 1, "position": {"x": 910, "y": 200},
         "title": "Not started"}),
        run_workflow("dispatch", 520, 0, "{{params.stableInputs}}"),
        wait("wait_pr", 780, 0, "Wait for the release PR", PR_POLL_MS, PR_POLLS),
        json!({"id": "find_pr", "kind": "github.change_request", "version": 1, "position": {"x": 1040, "y": 0},
         "title": "Release PR opened?", "retry": retry, "maxVisits": PR_POLLS,
         "config": {"branch": "{{steps.pr_branch.value}}"}}),
        json!({"id": "tell_pr", "kind": "notify", "version": 1, "position": {"x": 1300, "y": 0},
         "title": "Tell me it is open", "config": {
            "title": "Release PR #{{steps.find_pr.number}} is open",
            "body": "{{steps.find_pr.title}}: the release is published once it merges.",
            "link": "{{steps.find_pr.url}}"}}),
        wait("wait_merge", 1560, 0, "Wait for the merge", MERGE_POLL_MS, MERGE_POLLS),
        json!({"id": "check_pr", "kind": "github.change_request", "version": 1, "position": {"x": 1820, "y": 0},
         "title": "Release PR merged?", "retry": retry, "maxVisits": MERGE_POLLS,
         "config": {"branch": "{{steps.pr_branch.value}}", "number": "{{steps.find_pr.number}}", "ci": true,
                    "dryRunPort": "merged"}}),
        // Once per head commit and kind of failure: a fix pushed that fails again is news again.
        json!({"id": "tell_failed", "kind": "inbox.notice", "version": 1, "position": {"x": 1820, "y": -200},
         "title": "Inbox: a check failed", "maxVisits": MERGE_POLLS, "config": {
            "key": "release-pr-{{steps.check_pr.number}}-{{steps.check_pr.failure}}-{{steps.check_pr.headSha}}",
            "title": "Release PR #{{steps.check_pr.number}} failed its checks",
            "text": "[{{steps.check_pr.title}}]({{steps.check_pr.url}}) didn't merge: [a check failed]\
                     ({{steps.check_pr.ciUrl}}).\n\n{{steps.check_pr.failedJobs}}\n\nIt stays open, and nothing \
                     is released. Push a fix to {{steps.pr_branch.value}} (after a merge queue removed it, set it \
                     to merge again), or close it, which ends this release."}}),
        json!({"id": "notify_failed", "kind": "notify", "version": 1, "position": {"x": 1560, "y": -200},
         "title": "Tell me it failed", "maxVisits": MERGE_POLLS, "config": {
            "title": "Release PR #{{steps.check_pr.number}} failed its checks",
            "body": "It stays open and nothing is released: fix it or close it.",
            "level": "warning", "link": "{{steps.check_pr.ciUrl}}"}}),
        wait("wait_release", 2080, 0, "Wait for the release", RELEASE_POLL_MS, RELEASE_POLLS),
        published,
        json!({"id": "notify", "kind": "notify", "version": 1, "position": {"x": 2600, "y": 0},
         "title": "Tell me it is out", "config": {
            "title": "Release {{steps.published.tag}} is published",
            "body": "{{steps.published.name}} is out.",
            "link": "{{steps.published.url}}"}}),
        json!({"id": "tell_released", "kind": "inbox.notice", "version": 1, "position": {"x": 2860, "y": 0},
         "title": "Inbox: published", "config": {
            "key": "released-{{steps.published.tag}}",
            "title": "Release {{steps.published.tag}} is published",
            "text": "[{{steps.published.name}}]({{steps.published.url}}) is published."}}),
        json!({"id": "released", "kind": "end", "version": 1, "position": {"x": 3120, "y": 0}, "title": "Released",
         "config": {"output": {"tag": "{{steps.published.tag}}", "url": "{{steps.published.url}}"}}}),
        json!({"id": "dropped", "kind": "notify", "version": 1, "position": {"x": 2080, "y": 200},
         "title": "Tell me it was closed", "config": {
            "title": "Release PR #{{steps.find_pr.number}} was closed without merging",
            "body": "Nothing was released. Run Cut a release again for a new one.",
            "level": "warning", "link": "{{steps.find_pr.url}}"}}),
        json!({"id": "not_released", "kind": "end", "version": 1, "position": {"x": 2340, "y": 200},
         "title": "Not released"}),
    ]);
    let mut edges = vec![
        json!({"from": "start", "to": "kind"}),
        json!({"from": "kind", "port": "true", "to": "has_pr"}),
        json!({"from": "kind", "port": "false", "to": "pre_before"}),
        json!({"from": "has_pr", "port": "true", "to": "pr_branch"}),
        json!({"from": "pr_branch", "to": "before"}),
        json!({"from": "has_pr", "port": "false", "to": "direct_before"}),
    ];
    edges.extend(direct_edges);
    edges.extend(pre_edges);
    edges.extend([
        json!({"from": "before", "port": "found", "to": "open_already"}),
        json!({"from": "before", "port": "none", "to": "open_already"}),
        json!({"from": "open_already", "port": "open", "to": "still_open"}),
        json!({"from": "open_already", "port": "none", "to": "dispatch"}),
        json!({"from": "open_already", "port": "merged", "to": "dispatch"}),
        json!({"from": "open_already", "port": "closed", "to": "dispatch"}),
        json!({"from": "still_open", "port": "added", "to": "not_started"}),
        json!({"from": "still_open", "port": "skipped", "to": "not_started"}),
        json!({"from": "dispatch", "to": "wait_pr"}),
        json!({"from": "wait_pr", "port": "done", "to": "find_pr"}),
        json!({"from": "find_pr", "port": "open", "to": "tell_pr"}),
        // Until the new one is open, the branch's newest PR may be the last release's (merged).
        json!({"from": "find_pr", "port": "none", "to": "wait_pr"}),
        json!({"from": "find_pr", "port": "merged", "to": "wait_pr"}),
        json!({"from": "find_pr", "port": "closed", "to": "wait_pr"}),
        json!({"from": "tell_pr", "to": "wait_merge"}),
        json!({"from": "wait_merge", "port": "done", "to": "check_pr"}),
        json!({"from": "check_pr", "port": "open", "to": "wait_merge"}),
        json!({"from": "check_pr", "port": "failed", "to": "tell_failed"}),
        json!({"from": "tell_failed", "port": "added", "to": "notify_failed"}),
        json!({"from": "tell_failed", "port": "skipped", "to": "wait_merge"}),
        json!({"from": "notify_failed", "to": "wait_merge"}),
        json!({"from": "check_pr", "port": "merged", "to": "wait_release"}),
        json!({"from": "check_pr", "port": "closed", "to": "dropped"}),
        json!({"from": "check_pr", "port": "none", "to": "dropped"}),
        json!({"from": "dropped", "to": "not_released"}),
        json!({"from": "wait_release", "port": "done", "to": "published"}),
        json!({"from": "published", "port": "found", "to": "notify"}),
        json!({"from": "published", "port": "none", "to": "wait_release"}),
        json!({"from": "notify", "to": "tell_released"}),
        json!({"from": "tell_released", "port": "added", "to": "released"}),
        json!({"from": "tell_released", "port": "skipped", "to": "released"}),
    ]);
    builtin(
        CUT_RELEASE,
        Category::Releases,
        params(json!([
            {"id": "workflow", "title": "Release workflow", "type": "string", "required": true,
             "description": "The release workflow's file (publish.yml), started by hand (workflow_dispatch)."},
            {"id": "ref", "title": "Branch it runs on", "type": "string", "default": "main"},
            {"id": "stableInputs", "title": "Stable release inputs", "type": "text",
             "default": "{\"bump\": \"{{trigger.bump}}\", \"version\": \"{{trigger.version}}\"}",
             "description": "The workflow's inputs for a stable release, as JSON. {{trigger.bump}} is the bump \
                             asked for (patch, minor or major), {{trigger.version}} the exact version; empty ones are \
                             left out."},
            {"id": "releaseBranch", "title": "Release PR branch", "type": "string", "default": "",
             "description": "When the workflow opens a release PR for a stable release, the branch it opens it from: \
                             you are told when it is open, a failed check goes to the Inbox, and an open one stops a \
                             new release. Empty: no release PR."},
            {"id": "tagPattern", "title": "Stable tag pattern", "type": "string", "default": "",
             "description": "The stable release's tag (* is any text), e.g. v*. Empty: the latest release."},
            {"id": "preName", "title": "Other release's name", "type": "string", "default": "Pre-release",
             "description": "What Run now calls the other kind of release, e.g. Beta."},
            {"id": "preInputs", "title": "Other release's inputs", "type": "text", "default": "{}",
             "description": "The workflow's inputs for it, as JSON: {\"stage\": \"beta\"}. It has no release PR."},
            {"id": "preTagPattern", "title": "Other release's tag pattern", "type": "string", "default": "",
             "description": "Its tag (* is any text), e.g. v*-beta.*; pre-releases count. Empty: the newest release."},
            {"id": "defaultType", "title": "Run now offers first", "type": "choice", "default": "prerelease",
             "choices": ["prerelease", "stable"],
             "description": "The kind Run now's form starts on. A stable release always asks you to confirm."},
        ])),
        super::templates::doc(json!({
            "name": "Cut a release",
            "description": "Runs a project's release workflow and tells you when the release is published. Run now \
                            asks which kind: a stable release (an exact version, or a patch, minor or major bump; \
                            you confirm it) or the other kind the workflow makes (a pre-release). With a release PR \
                            branch, an open release PR stops a new stable release, you are told when the PR is open, \
                            and a failed check goes to the Inbox while it waits for the merge. Run it (the PM does, \
                            when you ask for a release) with input {\"type\": \"stable\", \"bump\": \"minor\"}, \
                            {\"type\": \"stable\", \"version\": \"2.0.0\"} or {\"type\": \"prerelease\"}.",
            "nodes": nodes,
            "edges": edges,
            "settings": {"maxSteps": 2 * PR_POLLS + 3 * MERGE_POLLS + 2 * RELEASE_POLLS + 20,
                         "maxRuntimeMs": 16_i64 * 24 * 60 * 60 * 1000},
        })),
    )
}

/// The id of the Report a missing scheduled release template (without `builtin:`).
pub const MISSING_RELEASE: &str = "missing-scheduled-release";

/// Report a missing scheduled release (PDT-294, after PDT-288's watch for PDT's nightlies): a release workflow on
/// a schedule (GitHub's are best-effort, and some days never start) should have published a release whose tag matches
/// a pattern by an hour. Each day then it looks; a missing one goes to the Inbox with Start it (the workflow, with the
/// inputs given), and the item closes by itself once the release is published.
pub(crate) fn missing_release_template() -> pdt_core::workflow::template::Template {
    use super::templates::{builtin, params};
    builtin(
        MISSING_RELEASE,
        Category::Releases,
        params(json!([
            {"id": "repo", "title": "Repository", "type": "string", "required": true,
             "description": "owner/name on GitHub."},
            {"id": "workflow", "title": "Release workflow", "type": "string", "required": true,
             "description": "The scheduled release workflow's file (publish.yml): its runs are linked, and Start it \
                             runs it."},
            {"id": "branch", "title": "Branch", "type": "string", "default": "main",
             "description": "The branch the release is built from (and Start it runs on)."},
            {"id": "tagPattern", "title": "Tag pattern", "type": "string", "required": true,
             "description": "The scheduled release's tag: * is any text, ? one character and {date} the day \
                             (YYYYMMDD or YYYY-MM-DD), e.g. v*-beta.{date}* or snapshot-{date}. Without {date}, a \
                             matching release published that day counts."},
            {"id": "hour", "title": "Expected by (hour, UTC)", "type": "number", "default": 10,
             "description": "When the day's release should be out (0 to 23): it looks then."},
            {"id": "onlyIfChanged", "title": "Only if the branch changed", "type": "boolean", "default": true,
             "description": "Say nothing on a day the branch has nothing the newest matching release lacks."},
            {"id": "inputs", "title": "Start it inputs", "type": "text", "default": "{}",
             "description": "The workflow's inputs for Start it, as JSON: {\"channel\": \"beta\"}."},
            {"id": "title", "title": "Inbox title", "type": "string", "default": "A scheduled release is missing"},
        ])),
        doc(json!({
            "name": "Report a missing scheduled release",
            "description": "Every day at the hour you give, looks for the day's release from a scheduled release \
                            workflow (by its tag pattern). When none is published (and the branch changed since \
                            the last one), the Inbox says so, with Start it, which runs the workflow; the item \
                            closes by itself once the release is published.",
            "nodes": [
                {"id": "daily", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 0}, "title": "Every day",
                 "config": {"cron": "0 {{params.hour}} * * *", "timezone": "UTC", "catchUp": true}},
                {"id": "watch", "kind": "github.nightly", "version": 2, "position": {"x": 260, "y": 0},
                 "title": "The day's release", "retry": {"max": 5, "backoffMs": 60_000},
                 "config": {"repo": "{{params.repo}}", "workflow": "{{params.workflow}}", "branch": "{{params.branch}}",
                            "tagPattern": "{{params.tagPattern}}", "day": "{{trigger.date}}",
                            "dueBy": "{{params.hour}}:00", "onlyIfChanged": "{{params.onlyIfChanged}}",
                            "title": "{{params.title}}"}},
                {"id": "start", "kind": "github.workflow.run", "version": 1, "position": {"x": 520, "y": -160},
                 "title": "Run the release workflow", "config": {"repo": "{{params.repo}}",
                    "workflow": "{{params.workflow}}", "ref": "{{params.branch}}", "inputs": "{{params.inputs}}"}},
                {"id": "started", "kind": "notify", "version": 1, "position": {"x": 780, "y": -160},
                 "title": "Tell me it started", "config": {
                    "title": "The release workflow is starting",
                    "body": "{{params.workflow}} runs on {{params.branch}}.",
                    "link": "{{steps.watch.runsUrl}}"}},
                {"id": "start_failed", "kind": "inbox.notice", "version": 1, "position": {"x": 780, "y": -320},
                 "title": "Inbox: it would not start", "config": {
                    "key": "release-start-failed-{{steps.watch.day}}",
                    "title": "The release workflow didn't start",
                    "text": "PDT could not run {{params.workflow}}: {{steps.start.error}}\n\nStart it on GitHub \
                             instead: [the release workflow]({{steps.watch.runsUrl}}), Run workflow on \
                             {{params.branch}}."}},
                {"id": "done", "kind": "end", "version": 1, "position": {"x": 1040, "y": -160}, "title": "Started"},
                {"id": "fine", "kind": "end", "version": 1, "position": {"x": 520, "y": 160}, "title": "Fine",
                 "config": {"quiet": true}},
            ],
            "edges": [
                {"from": "daily", "to": "watch"},
                {"from": "watch", "port": "start", "to": "start"},
                {"from": "start", "to": "started"},
                {"from": "start", "port": "error", "to": "start_failed"},
                {"from": "started", "to": "done"},
                {"from": "start_failed", "port": "added", "to": "done"},
                {"from": "start_failed", "port": "skipped", "to": "done"},
                {"from": "watch", "port": "published", "to": "fine"},
                {"from": "watch", "port": "early", "to": "fine"},
                {"from": "watch", "port": "unchanged", "to": "fine"},
                {"from": "watch", "port": "ignore", "to": "fine"},
                {"from": "watch", "port": "already", "to": "fine"},
                {"from": "watch", "port": "expired", "to": "fine"},
            ],
        })),
    )
}

/// Installs or updates the built-ins (once per pdtd; callers hold the engine lock).
pub(crate) async fn ensure_installed(st: &AppState) {
    use std::sync::atomic::Ordering;
    if st.builtins_installed.load(Ordering::SeqCst) {
        return;
    }
    let mut ok = true;
    if let Err(e) = detach_retired(st).await {
        ok = false;
        tracing::warn!(error = %e, "could not keep retired built-in workflows as the user's own");
    }
    for b in &all(&st.registry()) {
        if let Err(e) = install(st, b).await {
            ok = false;
            tracing::warn!(builtin = b.id, error = %e, "could not install a built-in workflow");
        }
    }
    if let Err(e) = super::templates::follow_templates(st).await {
        ok = false;
        tracing::warn!(error = %e, "could not move workflows to their templates' newer versions");
    }
    // Tried again on the next tick when something failed.
    st.builtins_installed.store(ok, Ordering::SeqCst);
}

/// Built-ins PDT no longer ships (PDT-295): they automated PDT's own development (bug triage, outside pull request
/// review, nightly release watches), and PDT never special-cases its own repository; Cut a release is a template
/// since PDT-301 (made per project with its release workflow). A host that has one keeps it as its own workflow
/// ([`detach_retired`]); no host gets one installed.
pub(crate) const RETIRED: &[&str] =
    &["pdt-bug-triage", "pdt-pr-review", "pdt-nightly", "pdt-nightly-missing", CUT_RELEASE];

/// Makes each installed [`RETIRED`] built-in an ordinary workflow of the user's: the same document, enabled state,
/// runs and state, no longer updated by PDT. One the user never edited gets its nodes moved to their kinds' current
/// versions first, as PDT's last update of it (a missing nightly's version 1 watch becomes version 2 with PDT's tag
/// scheme written out); an edited one is left as the user saved it (it runs the same). Repeating it after a failure is
/// harmless.
async fn detach_retired(st: &AppState) -> Result<(), pdt_store::StoreError> {
    let everything = st.integrations.everything();
    let catalog = everything.catalog();
    for id in RETIRED {
        let Some(w) = st.store.workflow_by_builtin(id).await? else { continue };
        let untouched = st.store.current_version_author(&w.id).await? == Some(Author::Builtin);
        // Cut a release came off; one never turned on, edited or run was only ever PDT's: it goes (PDT-301).
        if *id == CUT_RELEASE && untouched && !w.enabled && !ran(st, &w.id).await? {
            st.store.delete_workflow(&w.id).await?;
            tracing::info!(builtin = id, "removed a retired built-in workflow that was never used");
            continue;
        }
        if let Some(mut doc) = st.store.workflow_doc(&w.id, w.version).await?.filter(|_| untouched) {
            let mut moved = false;
            for n in &mut doc.nodes {
                let Some(version) = catalog.spec(&n.kind).map(|s| s.version).filter(|v| *v > n.version) else {
                    continue;
                };
                match catalog.current_config(n) {
                    Ok(config) => {
                        (n.config, n.version, moved) = (config, version, true);
                    }
                    Err(e) => tracing::warn!(workflow = %w.name, node = %n.id, error = %e, "left a node as it was"),
                }
            }
            if moved {
                let note = "kept as your own workflow (PDT no longer ships it)";
                st.store.save_workflow_version(&w.id, &doc, Author::Builtin, note, now_ms()).await?;
            }
        }
        st.store.detach_workflow_builtin(&w.id, now_ms()).await?;
        tracing::info!(builtin = id, workflow = %w.name, "kept a retired built-in workflow as the user's own");
    }
    Ok(())
}

/// Whether a workflow has any run.
async fn ran(st: &AppState, id: &str) -> Result<bool, pdt_store::StoreError> {
    let f = pdt_store::RunFilter { workflow_id: Some(id.into()), limit: 1, ..Default::default() };
    Ok(!st.store.list_workflow_runs(&f).await?.is_empty())
}

async fn install(st: &AppState, b: &Builtin) -> Result<(), pdt_store::StoreError> {
    let template = (b.doc)();
    let now = now_ms();
    match st.store.workflow_by_builtin(b.id).await? {
        None => {
            let enabled = b.enabled;
            let w = Workflow {
                id: uuid::Uuid::now_v7().to_string(),
                name: template.name.clone(),
                description: template.description.clone(),
                scope: Scope::Host,
                project_id: None,
                enabled,
                builtin: Some(b.id.into()),
                source: Source::Host,
                repo_file: None,
                version: 1,
                proposed: None,
                created_at: now,
                updated_at: now,
            };
            st.store.insert_workflow(&w, &template, Author::Builtin, "installed").await?;
            tracing::info!(builtin = b.id, enabled, "installed a built-in workflow");
        }
        Some(w) => {
            let current = st.store.workflow_doc(&w.id, w.version).await?;
            let untouched = st.store.current_version_author(&w.id).await? == Some(Author::Builtin);
            if untouched && current.as_ref() != Some(&template) {
                st.store.save_workflow_version(&w.id, &template, Author::Builtin, "updated by PDT", now).await?;
                tracing::info!(builtin = b.id, "updated a built-in workflow");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::validate::{Severity, validate};

    #[test]
    fn every_builtin_is_valid() {
        for b in &all(crate::integrations::test_registry()) {
            let d = (b.doc)();
            let issues = validate(&d, crate::integrations::test_registry().catalog());
            let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
            assert!(errors.is_empty(), "{}: {errors:?}", b.id);
        }
    }

    /// Cut a release made for a project: a beta channel, a release PR branch.
    fn cut_release_for(given: serde_json::Value) -> WorkflowDoc {
        use pdt_core::workflow::template::{fill, param_values};
        let t = cut_release_template();
        fill(&t.doc, &param_values(&t.params, given.as_object().unwrap()).unwrap()).unwrap()
    }

    /// PDT-194: every port of Cut a release leads somewhere (but a timed wait's event and timeout, which it never
    /// takes), and every node is reached; since PDT-301 it is a template, no built-in.
    #[test]
    fn cut_release_has_no_open_ports() {
        let d = cut_release_for(json!({"workflow": "publish.yml", "releaseBranch": "cut/release", "preName": "Beta",
                                       "preInputs": "{\"channel\": \"beta\"}"}));
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        let open = |i: &&pdt_core::workflow::validate::Issue| {
            i.code == "unreachable" || (i.code == "open_port" && !i.node.as_deref().is_some_and(|n| n.contains("wait")))
        };
        assert!(!issues.iter().any(|i| open(&i)), "{issues:?}");
        assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
        assert!(!all(crate::integrations::test_registry()).iter().any(|b| b.id == CUT_RELEASE), "a template only");
        assert!(RETIRED.contains(&CUT_RELEASE));
    }

    /// PDT-301: Run now asks the kind of release, the other kind (the safe one) first, and a stable one confirms.
    #[test]
    fn cut_release_asks_the_kind_of_release() {
        use crate::workflow::nodes::trigger::ManualConfig;
        let d = cut_release_for(json!({"workflow": "publish.yml", "preName": "Beta"}));
        let manual: ManualConfig = serde_json::from_value(d.node("start").unwrap().config.clone()).unwrap();
        pdt_core::workflow::inputs::check_fields(&manual.inputs, manual.confirm.as_ref()).unwrap();
        assert_eq!(manual.inputs[0].choices[0].label, "Beta");
        let input = |v: serde_json::Value| manual.input(&v).unwrap();
        assert_eq!(input(json!({})), json!({"type": "prerelease"}));
        let stable = input(json!({"type": "stable", "bump": "minor"}));
        assert_eq!(stable, json!({"type": "stable", "bump": "minor"}));
        let ask = |v: &serde_json::Value| pdt_core::workflow::inputs::confirm_text(manual.confirm.as_ref(), v);
        assert_eq!(ask(&stable).as_deref(), Some("This publishes a stable release (minor). Continue?"));
        assert_eq!(ask(&input(json!({}))), None);
        let exact = input(json!({"type": "stable", "version": "2.0.0", "bump": "major"}));
        assert_eq!(exact, json!({"type": "stable", "version": "2.0.0"}), "an exact version, no bump");
        assert!(manual.input(&json!({"channel": "beta"})).unwrap_err().contains("there is no input channel"));
        let stable_first = cut_release_for(json!({"workflow": "publish.yml", "defaultType": "stable"}));
        let m: ManualConfig = serde_json::from_value(stable_first.node("start").unwrap().config.clone()).unwrap();
        assert_eq!(m.input(&json!({})).unwrap()["type"], "stable");
    }

    /// PDT-295: no built-in is a retired one, so none comes back.
    #[test]
    fn no_retired_builtin_is_shipped() {
        for b in &all(crate::integrations::test_registry()) {
            assert!(!RETIRED.contains(&b.id), "{}", b.id);
        }
    }

    /// PDT-294: Report a missing scheduled release, filled in, leaves no port open, and names nothing of PDT's.
    #[test]
    fn the_missing_release_template_fills_in() {
        use pdt_core::workflow::template::{fill, param_values};
        let t = missing_release_template();
        let given = json!({"repo": "acme/app", "workflow": "publish.yml", "tagPattern": "v*-beta.{date}", "hour": 7,
                           "inputs": "{\"channel\": \"beta\"}"});
        let d = fill(&t.doc, &param_values(&t.params, given.as_object().unwrap()).unwrap()).unwrap();
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        assert!(!issues.iter().any(|i| i.code == "unreachable" || i.code == "open_port"), "{issues:?}");
        assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
        let watch = &d.node("watch").unwrap().config;
        assert_eq!(watch["dueBy"], "7:00");
        assert_eq!(watch["onlyIfChanged"], true);
        assert_eq!(d.node("daily").unwrap().config["cron"], "0 7 * * *");
        // The node kind keeps its version 1 id.
        let text = serde_json::to_string(&t).unwrap().to_lowercase().replace("\"github.nightly\"", "");
        for pdt in ["release.yml", "nightly", pdt_core::brand::RELEASE_REPO] {
            assert!(!text.contains(&pdt.to_lowercase()), "names {pdt}");
        }
    }

    /// PDT-294: Cut a release assumes no workflow file, branch, inputs or tag scheme.
    #[test]
    fn cut_release_has_no_pdt_defaults() {
        let text = serde_json::to_string(&cut_release_template()).unwrap().to_lowercase();
        for pdt in ["release.yml", "nightly", "experimental", "channel", "\"release\"", pdt_core::brand::RELEASE_REPO] {
            assert!(!text.contains(&pdt.to_lowercase()), "names {pdt}");
        }
    }

    /// PDT-139: a notice nobody could be told goes on, to the Inbox and the ticket.
    #[test]
    fn pr_notices_leaves_no_notice_untold() {
        let issues = validate(&pr_notices(), crate::integrations::test_registry().catalog());
        let open: Vec<_> =
            issues.iter().filter(|i| i.code == "open_port" && i.node.as_deref() == Some("tell")).collect();
        assert!(open.iter().all(|i| !i.message.contains("untold")), "{open:?}");
        assert!(!issues.iter().any(|i| i.code == "unreachable"), "{issues:?}");
    }
}
