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

pub const BUILTINS: &[Builtin] = &[
    Builtin { id: PR_NOTICES, enabled: true, doc: pr_notices },
    Builtin { id: FINISH_MERGED, enabled: true, doc: finish_merged },
    Builtin { id: COMMENTS_TO_AGENT, enabled: true, doc: comments_to_agent },
    Builtin { id: BASE_MOVED, enabled: false, doc: base_moved },
];

fn doc(v: serde_json::Value) -> WorkflowDoc {
    serde_json::from_value(v).expect("a built-in workflow is a valid document")
}

fn pr_notices() -> WorkflowDoc {
    let trigger =
        |id: &str, kind: &str, y: i32| json!({"id": id, "kind": kind, "version": 1, "position": {"x": 0, "y": y}});
    let mut merged = trigger("merged", "attempt.merged", 400);
    merged["config"] = json!({"where": [{"left": "{{trigger.via}}", "op": "ne", "right": "closed"}]});
    let edges: Vec<_> =
        ["ci", "conflict", "queue", "pr_merged", "merged"].iter().map(|f| json!({"from": f, "to": "tell"})).collect();
    doc(json!({
        "name": "PR notices",
        "description": "Tells the attempt's agent when CI fails, its pull request conflicts with the base, the merge \
                        queue drops it, or it merges. Each notice once; it waits until the agent is idle and nobody \
                        is typing, and lands on the ticket's timeline.",
        "nodes": [
            trigger("ci", "ci.run_failed", 0),
            trigger("conflict", "pr.conflict", 100),
            trigger("queue", "pr.queue_left", 200),
            trigger("pr_merged", "pr.merged", 300),
            merged,
            {"id": "tell", "kind": "session.send", "version": 1, "position": {"x": 320, "y": 200}, "config": {
                "attempt": "{{trigger.attemptId}}", "text": "{{trigger.notice.text}}",
                "dedupe": "{{trigger.notice.key}}", "timeline": "pr"}},
        ],
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
                        idle.",
        "nodes": [
            {"id": "commented", "kind": "ticket.commented", "version": 1, "position": {"x": 0, "y": 0}, "config": {"where": [
                {"left": "{{trigger.attemptId}}", "op": "exists"},
                {"left": "{{trigger.actor}}", "op": "eq", "right": "user"},
                {"left": "{{trigger.workflowId}}", "op": "missing"}]}},
            {"id": "tell", "kind": "session.send", "version": 1, "position": {"x": 320, "y": 0}, "config": {
                "attempt": "{{trigger.attemptId}}",
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
}
