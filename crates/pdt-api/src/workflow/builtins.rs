//! Built-in workflows (Phase 7a spec §8): the reactions that used to be hard-coded, installed host-wide when pdtd
//! starts. A built-in the user never edited follows newer templates; one the user edited is left alone. They can be
//! disabled, not deleted.

use crate::routes::{AppState, now_ms};
use pdt_core::brand::RELEASE_REPO;
use pdt_core::workflow::api::{Author, Scope, Source, Workflow};
use pdt_core::workflow::doc::WorkflowDoc;
use serde_json::json;

#[derive(Clone, Copy)]
pub struct Builtin {
    pub id: &'static str,
    /// Enabled when first installed.
    pub enabled: bool,
    pub doc: fn() -> WorkflowDoc,
}

pub const PR_NOTICES: &str = "pr-notices";
pub const FINISH_MERGED: &str = "finish-merged";
pub const COMMENTS_TO_AGENT: &str = "comments-to-agent";
pub const BASE_MOVED: &str = "base-moved";
pub const PR_FEEDBACK: &str = "pr-feedback";
pub const LATE_COMMITS: &str = "late-commits";
pub const PDT_BUG_TRIAGE: &str = "pdt-bug-triage";
pub const GITHUB_ISSUE_DONE: &str = "github-issue-done";

pub const BUILTINS: &[Builtin] = &[
    Builtin { id: PR_NOTICES, enabled: true, doc: pr_notices },
    Builtin { id: FINISH_MERGED, enabled: true, doc: finish_merged },
    Builtin { id: LATE_COMMITS, enabled: true, doc: late_commits },
    Builtin { id: COMMENTS_TO_AGENT, enabled: true, doc: comments_to_agent },
    Builtin { id: BASE_MOVED, enabled: false, doc: base_moved },
    Builtin { id: PR_FEEDBACK, enabled: false, doc: pr_feedback },
    // Enabled when installed only on a host with a project whose repository is the release repository (PDT's own).
    Builtin { id: PDT_BUG_TRIAGE, enabled: false, doc: pdt_bug_triage },
    Builtin { id: GITHUB_ISSUE_DONE, enabled: true, doc: github_issue_done },
];

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

const FIX_PROMPT: &str = "{{trigger.notice.text}}\n\nThis pull request was set to merge (auto-merge or the merge \
queue), so get it merging again: fix the cause, run the checks, commit and push. Once you push, PDT turns auto-merge \
back on if it is off: don't turn it on yourself, whatever the notice above says.\n\nMost fixes are small (a CI fix, resolving a conflict, merging the base \
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
    let people = json!([
        {"left": "{{trigger.byOwner}}", "op": "falsy"},
        {"left": "{{trigger.isBot}}", "op": "falsy"},
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
                        and empty reviews are left out.",
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

/// What the triage step asks of the PM (PDT-154).
const BUG_TRIAGE_PROMPT: &str = "A bug report from another PDT instance arrived as issue #{{trigger.number}} on \
{{trigger.repo}}: \"{{trigger.title}}\" by {{trigger.author}} ({{trigger.url}}).\n\n{{trigger.body}}\n\nTriage it \
against this project: read the code it touches (your working folder is the project's checkout), its tickets \
(`search_tickets`, open and closed) and the repository's open and closed issues (`gh issue list --repo \
{{trigger.repo}} --state all --search \"<words>\"`, `gh issue view <n> --repo {{trigger.repo}}`). Change nothing: make \
no ticket, comment or issue change yourself; the workflow does that from your outcome.\n\nThen call `submit_outcome` \
with one of:\n- `accept`: a real bug no ticket covers yet. `summary`: your triage notes (where in the code, how bad, \
how to reproduce). `data`: `{\"priority\": \"low|medium|high|urgent\", \"labels\": [\"bug\"]}`.\n- `duplicate`: a \
ticket already covers it. `summary`: why it is the same bug. `data`: `{\"ticket\": \"<that ticket's key>\", \
\"duplicateOf\": <issue number>}`, where the issue is the one linked to that ticket (`get_ticket` shows its links), \
else the earliest issue reporting the same bug; leave `duplicateOf` out when there is none.\n- `reject`: not a bug or \
nothing to do (a question, spam, works as intended, can't be reproduced). `summary`: the reason, written for the \
reporter: it is posted on the issue, which is then closed as not planned.";

/// Bug reports from other PDT instances (PDT-153 files them as issues labelled `from-pdt` on the release repository)
/// come in through triage (PDT-154): accepted ones become linked Draft tickets, duplicates are closed as GitHub
/// duplicates of the canonical issue, rejected ones are closed as not planned with a notice in the Inbox.
fn pdt_bug_triage() -> WorkflowDoc {
    use pdt_core::github_issue::FROM_PDT;
    let triage = "{{steps.triage.summary}}";
    doc(json!({
        "name": "PDT bug triage",
        "description": format!("Each new issue labelled {FROM_PDT} on {RELEASE_REPO} (a bug report from another PDT \
                        instance) is triaged by the PM against the code, the tickets and the other issues. Accepted: \
                        a Draft ticket linked to the issue, with the triage notes, and a comment with its key on the \
                        issue. A duplicate: the issue is closed at once as a duplicate of the canonical issue (or as \
                        not planned when there is none), and the existing ticket notes the extra report. Rejected: \
                        closed as not planned with the reason, and a notice in your Inbox so you can overrule it. \
                        Enabled where a project's repository is {RELEASE_REPO}.", RELEASE_REPO = RELEASE_REPO),
        "nodes": [
            {"id": "opened", "kind": "github.issue.opened", "version": 1, "position": {"x": 0, "y": 160}, "config": {
                "repos": [RELEASE_REPO],
                "where": [
                    {"left": "{{trigger.labels}}", "op": "contains", "right": FROM_PDT},
                    {"left": "{{trigger.projectId}}", "op": "exists"}]}},
            {"id": "triage", "kind": "agent", "version": 1, "position": {"x": 300, "y": 160}, "title": "Triage",
             "config": {
                "persona": "PM", "session": "new", "ticket": "", "attempt": "", "prompt": BUG_TRIAGE_PROMPT,
                "outcomes": [
                    {"id": "accept", "description": "a real bug no ticket covers: it becomes a Draft ticket"},
                    {"id": "duplicate", "description": "a ticket already covers it (data: ticket, duplicateOf)"},
                    {"id": "reject", "description": "not a bug or nothing to do: closed as not planned"},
                ]}},
            // accept
            {"id": "import", "kind": "github.import", "version": 1, "position": {"x": 600, "y": 0}, "config": {
                "repo": "{{trigger.repo}}", "number": "{{trigger.number}}", "project": "{{trigger.projectId}}",
                "priority": "{{steps.triage.data.priority}}", "labels": "{{steps.triage.data.labels}}"}},
            {"id": "notes", "kind": "ticket.comment", "version": 1, "position": {"x": 900, "y": 0},
             "title": "Triage notes", "config": {"ticket": "{{steps.import.ticketId}}",
                "body": format!("Triage of {{{{trigger.url}}}} (from-pdt, by {{{{trigger.author}}}}):\n\n{triage}")}},
            {"id": "tell", "kind": "github.comment", "version": 1, "position": {"x": 1200, "y": 0},
             "title": "Comment the ticket key", "config": {
                "repo": "{{trigger.repo}}", "number": "{{trigger.number}}",
                "body": "Thanks for the report. It is tracked as {{steps.import.ticketKey}}; this issue is closed \
                         when that ticket is done."}},
            {"id": "imported", "kind": "end", "version": 1, "position": {"x": 1500, "y": 0}, "title": "Imported"},
            {"id": "known", "kind": "end", "version": 1, "position": {"x": 900, "y": -140},
             "title": "Already a ticket", "config": {"quiet": true}},
            // duplicate
            {"id": "canonical", "kind": "condition", "version": 1, "position": {"x": 600, "y": 160},
             "title": "Duplicates an issue?", "config": {"conditions": [
                {"left": "{{steps.triage.data.duplicateOf}}", "op": "truthy"}]}},
            {"id": "close_dup", "kind": "github.close", "version": 1, "position": {"x": 900, "y": 160},
             "title": "Close as duplicate", "config": {
                "repo": "{{trigger.repo}}", "number": "{{trigger.number}}", "reason": "duplicate",
                "duplicateOf": "{{steps.triage.data.duplicateOf}}",
                "comment": "Thanks for the report. This is a duplicate of #{{steps.triage.data.duplicateOf}}, \
                            tracked as {{steps.triage.data.ticket}}."}},
            {"id": "close_known", "kind": "github.close", "version": 1, "position": {"x": 900, "y": 300},
             "title": "Close as not planned", "config": {
                "repo": "{{trigger.repo}}", "number": "{{trigger.number}}", "reason": "not_planned",
                "comment": "Thanks for the report. This is already tracked as {{steps.triage.data.ticket}}."}},
            {"id": "extra", "kind": "ticket.comment", "version": 1, "position": {"x": 1200, "y": 230},
             "title": "Note the extra report", "config": {"ticket": "{{steps.triage.data.ticket}}",
                "body": format!("Another report of this bug: {{{{trigger.url}}}} (by {{{{trigger.author}}}}), \
                                 closed as a duplicate.\n\n{triage}")}},
            {"id": "duplicate", "kind": "end", "version": 1, "position": {"x": 1500, "y": 230}, "title": "Duplicate"},
            // reject
            {"id": "close_rejected", "kind": "github.close", "version": 1, "position": {"x": 600, "y": 460},
             "title": "Close as not planned", "config": {
                "repo": "{{trigger.repo}}", "number": "{{trigger.number}}", "reason": "not_planned",
                "comment": format!("Thanks for the report. Closing it: {triage}")}},
            {"id": "notice", "kind": "inbox.notice", "version": 1, "position": {"x": 900, "y": 460},
             "title": "Tell me", "config": {
                "key": "{{trigger.repo}}#{{trigger.number}}",
                "title": "Triage closed {{trigger.repo}}#{{trigger.number}} as not planned",
                "text": format!("PDT bug triage closed [{{{{trigger.repo}}}}#{{{{trigger.number}}}}]({{{{trigger.url}}}}) \
                                 \"{{{{trigger.title}}}}\" as not planned:\n\n{triage}\n\nTo overrule it, reopen the \
                                 issue on GitHub and import it.")}},
            {"id": "rejected", "kind": "end", "version": 1, "position": {"x": 1200, "y": 460}, "title": "Rejected"},
            {"id": "untriaged", "kind": "end", "version": 1, "position": {"x": 600, "y": 620},
             "title": "Not triaged", "config": {"output": {"issue": "{{trigger.url}}"}}},
        ],
        "edges": [
            {"from": "opened", "to": "triage"},
            {"from": "triage", "port": "accept", "to": "import"},
            {"from": "triage", "port": "duplicate", "to": "canonical"},
            {"from": "triage", "port": "reject", "to": "close_rejected"},
            {"from": "triage", "port": "timeout", "to": "untriaged"},
            {"from": "triage", "port": "stopped", "to": "untriaged"},
            {"from": "import", "port": "imported", "to": "notes"},
            {"from": "import", "port": "exists", "to": "known"},
            {"from": "notes", "to": "tell"},
            {"from": "tell", "to": "imported"},
            {"from": "canonical", "port": "true", "to": "close_dup"},
            {"from": "canonical", "port": "false", "to": "close_known"},
            {"from": "close_dup", "to": "extra"},
            {"from": "close_known", "to": "extra"},
            {"from": "extra", "to": "duplicate"},
            {"from": "close_rejected", "to": "notice"},
            {"from": "notice", "port": "added", "to": "rejected"},
            {"from": "notice", "port": "skipped", "to": "rejected"},
        ],
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

/// Whether a project on this host is PDT's own repository (the release repository): where bug triage runs.
async fn builds_pdt(st: &AppState) -> bool {
    for p in st.store.list_projects().await.unwrap_or_default() {
        if crate::github::github_repo(&p.repo_path).await.is_some_and(|r| r.eq_ignore_ascii_case(RELEASE_REPO)) {
            return true;
        }
    }
    false
}

/// Installs or updates the built-ins (once per pdtd; callers hold the engine lock).
pub(crate) async fn ensure_installed(st: &AppState) {
    use std::sync::atomic::Ordering;
    if st.builtins_installed.load(Ordering::SeqCst) {
        return;
    }
    let mut ok = true;
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

async fn install(st: &AppState, b: &Builtin) -> Result<(), pdt_store::StoreError> {
    let template = (b.doc)();
    let now = now_ms();
    match st.store.workflow_by_builtin(b.id).await? {
        None => {
            let enabled = b.enabled || (b.id == PDT_BUG_TRIAGE && builds_pdt(st).await);
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
        for b in BUILTINS {
            let d = (b.doc)();
            let issues = validate(&d, crate::integrations::test_registry().catalog());
            let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
            assert!(errors.is_empty(), "{}: {errors:?}", b.id);
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
