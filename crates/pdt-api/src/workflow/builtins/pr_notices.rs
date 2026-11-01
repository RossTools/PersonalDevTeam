//! PR notices: each pull request notice to its agent, and a failed merge fixed and set to merge again.

use super::*;

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

const RESOLVE_PROMPT: &str = "{{steps.update.message}}\n\nThis pull request was set to merge. Once you push, PDT sets \
it to merge again: don't turn on auto-merge or merge it yourself.";

const PUSH_PROMPT: &str = "The Reviewer approved your fix. Push it (run the checks first if anything changed since); \
PDT then gets the pull request merging again.";

/// The built-in PR notices (PDT-25, Phase 7a; PDT-104): each notice once to the attempt's agent; for a pull request
/// that was set to merge (or whose workflow merge was refused for a conflict, PDT-139), an agent step that fixes it (a
/// Reviewer first when the agent calls the fix large), then auto-merge on again, up to [`FIX_TRIES`] times per
/// attempt. A notice with no agent session to tell goes to the Inbox and the ticket's comments, once (PDT-139).
pub(super) fn pr_notices() -> WorkflowDoc {
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
        // Conflicts are fixed once, at merge time (PDT-328): before the merge step they are recorded, not sent.
        node(
            "defer",
            "attempt.defer_conflict",
            140,
            0,
            "Resolve at merge time?",
            json!({"attempt": "{{trigger.attemptId}}", "notice": "{{trigger.notice.kind}}"}),
        ),
        node("later", "end", 280, 0, "Resolved at merge time", json!({"quiet": true})),
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
        // The base first, with no agent (PDT-328): a conflict or a queue drop may need only the base merged in.
        node(
            "stale",
            "condition",
            1400,
            -280,
            "About the base?",
            json!({"conditions": [
            cond("{{trigger.notice.kind}}", "in", Some(json!(["conflict", "queue_left", "behind"])))]}),
        ),
        node(
            "update",
            "attempt.update_base",
            1680,
            -420,
            "Update with the base",
            json!({"attempt": "{{trigger.attemptId}}"}),
        ),
        node(
            "resolve",
            "agent",
            1960,
            -420,
            "Resolve the conflicts",
            json!({
            "persona": "Coder", "prompt": RESOLVE_PROMPT, "session": "reuse",
            "outcomes": [{"id": "submitted", "description": "the conflicts are resolved, checked and pushed"}]}),
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
    let mut edges: Vec<_> = ["ci", "queue"].iter().map(|f| e(f, "", "asked")).collect();
    edges.extend([
        e("conflict", "", "defer"),
        e("behind", "", "defer"),
        e("defer", "deferred", "later"),
        e("defer", "now", "asked"),
        e("pr_merged", "", "tell"),
        e("merged", "", "tell"),
        e("passed", "", "remerge"),
        e("asked", "true", "busy"),
        e("asked", "false", "tell"),
        e("busy", "yes", "tell"),
        e("busy", "no", "count"),
        e("count", "", "over"),
        e("over", "true", "first_over"),
        e("over", "false", "stale"),
        e("stale", "true", "update"),
        e("stale", "false", "fix"),
        e("update", "updated", "remerge"),
        e("update", "conflict", "resolve"),
        e("update", "current", "fix"),
        e("update", "skipped", "fix"),
        e("resolve", "submitted", "remerge"),
        e("resolve", "timeout", "stuck"),
        e("resolve", "stopped", "stuck"),
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
                        queue drops it, or it merges. A conflict while the ticket is in code, review, QA or accept \
                        waits for merge time (unless the project resolves conflicts immediately): it is recorded on \
                        the ticket, not sent. Each notice once; it waits until the agent is idle and nobody \
                        is typing, and lands on the ticket's timeline; one that comes while a Coder's pass sent \
                        back by review or QA waits goes in with it, after the findings. A pull request that was set to merge gets an \
                        agent step instead (the attempt's Coder session, or a new one), after PDT merges the base in itself \
                        for a conflict or a queue drop (an agent gets only the conflicts PDT can't resolve): it fixes the cause and pushes, \
                        with a Reviewer first when it calls the fix large, and auto-merge goes back on, also when CI \
                        passes again with it off; a workflow's merge refused for a conflict counts too. After 3 tries \
                        on one attempt it stops and comments on the ticket. A notice with no agent session to tell \
                        goes to the Inbox and the ticket instead, once; the Inbox item closes when its cause is gone.",
        "nodes": nodes,
        "edges": edges,
    }))
}
