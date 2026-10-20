//! What enabled workflows tell an attempt's agent about (PDT-83), so its instructions can say so: the event types
//! whose triggers lead to a `session.send` to the attempt, directly or through `workflow.call`. A trigger's
//! conditions are not read, so this can name an event a condition then skips.

use crate::workflow::api::{Scope, Workflow};
use crate::workflow::doc::WorkflowDoc;
use crate::workflow::events::EVENT_TYPES;
use std::collections::{BTreeSet, HashSet};

const SESSION_SEND: &str = "session.send";
const WORKFLOW_CALL: &str = "workflow.call";

/// The event types (ids from [`EVENT_TYPES`]) some enabled workflow that applies to `project_id` turns into a message
/// to an attempt's agent. `all` is every workflow with its current document; a called workflow need not be enabled.
pub fn agent_news(all: &[(Workflow, WorkflowDoc)], project_id: &str) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for (wf, doc) in all.iter().filter(|(wf, _)| wf.enabled) {
        let applies = match wf.scope {
            Scope::Project => wf.project_id.as_deref() == Some(project_id),
            Scope::Host => true,
            // Not about a code project, so never about an attempt.
            Scope::Personal => false,
        };
        if !applies {
            continue;
        }
        for node in &doc.nodes {
            let Some(event) = EVENT_TYPES.iter().find(|e| e.id == node.kind) else { continue };
            if !for_project(&node.config, project_id) {
                continue;
            }
            if reaches_send(all, doc, &node.id, &mut HashSet::new()) {
                out.insert(event.id);
            }
        }
    }
    out
}

/// A trigger's `projects` (empty: every project).
fn for_project(config: &serde_json::Value, project_id: &str) -> bool {
    match config.get("projects").and_then(|p| p.as_array()) {
        Some(ps) if !ps.is_empty() => ps.iter().any(|p| p.as_str() == Some(project_id)),
        _ => true,
    }
}

/// Whether some path of edges from `from` gets to a `session.send` to an attempt. `seen` holds the (document, node)
/// pairs visited, so loops and calls that recurse end.
fn reaches_send<'a>(
    all: &'a [(Workflow, WorkflowDoc)],
    doc: &'a WorkflowDoc,
    from: &'a str,
    seen: &mut HashSet<(usize, &'a str)>,
) -> bool {
    let mut stack = vec![from];
    while let Some(id) = stack.pop() {
        if !seen.insert((std::ptr::from_ref(doc) as usize, id)) {
            continue;
        }
        let Some(node) = doc.node(id) else { continue };
        if node.kind == SESSION_SEND && sends_to_attempt(&node.config) {
            return true;
        }
        if node.kind == WORKFLOW_CALL
            && let Some(called) = node.config.get("workflow").and_then(|w| w.as_str())
            && let Some((_, child)) = all.iter().find(|(w, _)| w.id == called)
            && child.nodes.iter().filter(|n| n.kind == "manual").any(|n| reaches_send(all, child, &n.id, seen))
        {
            return true;
        }
        stack.extend(doc.edges.iter().filter(|e| e.from == id).map(|e| e.to.as_str()));
    }
    false
}

fn sends_to_attempt(config: &serde_json::Value) -> bool {
    config.get("attempt").and_then(|a| a.as_str()).is_some_and(|a| !a.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::api::Source;
    use serde_json::json;

    fn wf(
        id: &str,
        scope: Scope,
        project: Option<&str>,
        enabled: bool,
        doc: serde_json::Value,
    ) -> (Workflow, WorkflowDoc) {
        let doc: WorkflowDoc = serde_json::from_value(doc).unwrap();
        let w = Workflow {
            id: id.into(),
            name: doc.name.clone(),
            description: String::new(),
            scope,
            project_id: project.map(String::from),
            enabled,
            builtin: None,
            source: Source::Host,
            repo_file: None,
            version: 1,
            created_at: 0,
            updated_at: 0,
        };
        (w, doc)
    }

    fn node(id: &str, kind: &str, config: serde_json::Value) -> serde_json::Value {
        json!({"id": id, "kind": kind, "version": 1, "config": config})
    }

    fn tell() -> serde_json::Value {
        node("tell", SESSION_SEND, json!({"attempt": "{{trigger.attemptId}}", "text": "x"}))
    }

    fn notices(name: &str) -> serde_json::Value {
        json!({"name": name, "nodes": [node("ci", "ci.run_failed", json!({})), node("m", "attempt.merged", json!({})),
            node("c", "condition", json!({})), tell()],
            "edges": [{"from": "ci", "to": "c"}, {"from": "c", "port": "true", "to": "tell"}, {"from": "m", "to": "tell"}]})
    }

    #[test]
    fn names_the_events_whose_triggers_lead_to_a_message() {
        let all = [wf("a", Scope::Host, None, true, notices("PR notices"))];
        assert_eq!(agent_news(&all, "p1"), BTreeSet::from(["attempt.merged", "ci.run_failed"]));
    }

    #[test]
    fn leaves_out_disabled_other_project_and_personal_workflows() {
        let all = [
            wf("a", Scope::Host, None, false, notices("a")),
            wf("b", Scope::Project, Some("p2"), true, notices("b")),
            wf("c", Scope::Personal, None, true, notices("c")),
        ];
        assert!(agent_news(&all, "p1").is_empty());
        assert_eq!(agent_news(&all, "p2").len(), 2);
    }

    #[test]
    fn honours_a_triggers_projects() {
        let doc = json!({"name": "x", "nodes": [node("t", "pr.conflict", json!({"projects": ["p2"]})), tell()],
            "edges": [{"from": "t", "to": "tell"}]});
        let all = [wf("a", Scope::Host, None, true, doc)];
        assert!(agent_news(&all, "p1").is_empty());
        assert_eq!(agent_news(&all, "p2"), BTreeSet::from(["pr.conflict"]));
    }

    #[test]
    fn needs_a_send_to_the_attempt_on_the_path() {
        let doc = json!({"name": "x", "nodes": [
                node("t", "pr.conflict", json!({})), node("move", "ticket.move", json!({})),
                node("s", "ci.run_passed", json!({})),
                node("other", SESSION_SEND, json!({"session": "s9", "text": "x"})), tell()],
            "edges": [{"from": "t", "to": "move"}, {"from": "s", "to": "other"}]});
        assert!(agent_news(&[wf("a", Scope::Host, None, true, doc)], "p1").is_empty());
    }

    #[test]
    fn follows_called_workflows_and_survives_loops() {
        let parent = json!({"name": "parent", "nodes": [node("t", "pr.queue_left", json!({})),
                node("call", WORKFLOW_CALL, json!({"workflow": "child"})), node("loop", "condition", json!({}))],
            "edges": [{"from": "t", "to": "loop"}, {"from": "loop", "to": "loop"}, {"from": "loop", "to": "call"}]});
        let child = json!({"name": "child", "nodes": [node("m", "manual", json!({})), tell()],
            "edges": [{"from": "m", "to": "tell"}]});
        let all = [wf("p", Scope::Host, None, true, parent), wf("child", Scope::Host, None, false, child)];
        assert_eq!(agent_news(&all, "p1"), BTreeSet::from(["pr.queue_left"]));
    }
}
