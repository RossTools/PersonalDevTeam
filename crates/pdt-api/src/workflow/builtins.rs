//! Built-in workflows (Phase 7a spec §8): the reactions that used to be hard-coded, installed host-wide when pdtd
//! starts. A built-in the user never edited follows newer templates; one the user edited is left alone. They can be
//! disabled, not deleted.

use crate::routes::{AppState, now_ms};
use pdt_core::workflow::api::{Author, Scope, Source, Workflow};
use pdt_core::workflow::doc::WorkflowDoc;
use serde_json::json;

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
pub const JIRA_IMPORT: &str = "jira-import";
pub const JIRA_DONE: &str = "jira-done";

pub const BUILTINS: &[Builtin] = &[
    Builtin { id: PR_NOTICES, enabled: true, doc: pr_notices },
    Builtin { id: FINISH_MERGED, enabled: true, doc: finish_merged },
    Builtin { id: COMMENTS_TO_AGENT, enabled: true, doc: comments_to_agent },
    Builtin { id: BASE_MOVED, enabled: false, doc: base_moved },
    Builtin { id: PR_FEEDBACK, enabled: false, doc: pr_feedback },
    Builtin { id: JIRA_IMPORT, enabled: true, doc: jira_import },
    Builtin { id: JIRA_DONE, enabled: true, doc: jira_done },
];

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
        node("ci", "ci.run_failed", 0, 0, "", null.clone()),
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
            "persona": "Reviewer", "prompt": FIX_REVIEW_PROMPT, "session": "new",
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
        node(
            "refused_note",
            "ticket.comment",
            2520,
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
        e("remerge", "refused", "refused_note"),
        e("tell", "untold", "unheard"),
        e("unheard", "true", "inbox"),
        e("unheard", "false", "heard"),
        e("inbox", "added", "unheard_note"),
    ]);
    doc(json!({
        "name": "PR notices",
        "description": "Tells the attempt's agent when CI fails, its pull request conflicts with the base, the merge \
                        queue drops it, or it merges. Each notice once; it waits until the agent is idle and nobody \
                        is typing, and lands on the ticket's timeline. A pull request that was set to merge gets an \
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
        "description": "When a ticket's branch is found merged and Merge… asked to finish it then, moves it to Done \
                        and cleans up (sessions, runs, worktree, workspace) and pulls the base. Uncommitted changes \
                        leave it for the merged banner.",
        "nodes": [
            {"id": "merged", "kind": "attempt.merged", "version": 1, "position": {"x": 0, "y": 0},
             "config": {"where": [{"left": "{{trigger.via}}", "op": "eq", "right": "detected"}]}},
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

fn jira_import() -> WorkflowDoc {
    let mine = json!([
        {"left": "{{trigger.assignedToMe}}", "op": "truthy"},
        {"left": "{{trigger.ticketId}}", "op": "missing"},
    ]);
    doc(json!({
        "name": "Import assigned Jira issues",
        "description": "An issue in a linked project's Jira scope that is assigned to you and has no ticket yet becomes \
                        a Draft ticket, linked to it. Does nothing until a project links Jira in its settings.",
        "nodes": [
            {"id": "created", "kind": "jira.issue.created", "version": 1, "position": {"x": 0, "y": 0},
             "config": {"where": mine}},
            {"id": "updated", "kind": "jira.issue.updated", "version": 1, "position": {"x": 0, "y": 140},
             "config": {"where": mine}},
            {"id": "import", "kind": "jira.import", "version": 1, "position": {"x": 320, "y": 70}, "config": {
                "key": "{{trigger.key}}", "project": "{{trigger.projectId}}"}},
        ],
        "edges": [{"from": "created", "to": "import"}, {"from": "updated", "to": "import"}],
    }))
}

fn jira_done() -> WorkflowDoc {
    doc(json!({
        "name": "Done moves the Jira issue",
        "description": "Moving a ticket linked to a Jira issue to Done moves the issue to Done too (the project's done \
                        transition, in its settings). Nothing else goes to Jira unless a workflow says so.",
        "nodes": [
            {"id": "done", "kind": "ticket.stage_changed", "version": 1, "position": {"x": 0, "y": 0}, "config": {"where": [
                {"left": "{{trigger.to}}", "op": "eq", "right": "done"},
                {"left": "{{trigger.jiraKey}}", "op": "exists"}]}},
            {"id": "move", "kind": "jira.transition", "version": 1, "position": {"x": 320, "y": 0}, "config": {
                "ticket": "{{trigger.ticketId}}"}},
        ],
        "edges": [{"from": "done", "to": "move"}],
    }))
}

/// Installs or updates the built-ins (once per pdtd; callers hold the engine lock).
pub(crate) async fn ensure_installed(st: &AppState) {
    use std::sync::atomic::Ordering;
    if st.builtins_installed.load(Ordering::SeqCst) {
        return;
    }
    let mut ok = true;
    for b in BUILTINS {
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
            let w = Workflow {
                id: uuid::Uuid::now_v7().to_string(),
                name: template.name.clone(),
                description: template.description.clone(),
                scope: Scope::Host,
                project_id: None,
                enabled: b.enabled,
                builtin: Some(b.id.into()),
                source: Source::Host,
                repo_file: None,
                version: 1,
                proposed: None,
                created_at: now,
                updated_at: now,
            };
            st.store.insert_workflow(&w, &template, Author::Builtin, "installed").await?;
            tracing::info!(builtin = b.id, enabled = b.enabled, "installed a built-in workflow");
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
            let issues = validate(&d, crate::workflow::catalog::catalog());
            let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
            assert!(errors.is_empty(), "{}: {errors:?}", b.id);
        }
    }

    /// PDT-139: a notice nobody could be told goes on, to the Inbox and the ticket.
    #[test]
    fn pr_notices_leaves_no_notice_untold() {
        let issues = validate(&pr_notices(), crate::workflow::catalog::catalog());
        let open: Vec<_> =
            issues.iter().filter(|i| i.code == "open_port" && i.node.as_deref() == Some("tell")).collect();
        assert!(open.iter().all(|i| !i.message.contains("untold")), "{open:?}");
        assert!(!issues.iter().any(|i| i.code == "unreachable"), "{issues:?}");
    }
}
