//! The difference between two versions of a workflow (Phase 7a spec §3.3): which nodes and edges changed, and a
//! unified diff of the two documents as pretty JSON.

use super::doc::{Edge, WorkflowDoc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocDiff {
    pub nodes_added: Vec<String>,
    pub nodes_removed: Vec<String>,
    /// Same id, something else changed (kind, version, config, retry, …; not only its position).
    pub nodes_changed: Vec<String>,
    pub edges_added: Vec<Edge>,
    pub edges_removed: Vec<Edge>,
    /// Name, description or settings changed.
    pub meta_changed: bool,
    /// `diff -u` of the two documents.
    pub unified: String,
}

/// What changed from `a` to `b`.
pub fn diff(a: &WorkflowDoc, b: &WorkflowDoc, a_label: &str, b_label: &str) -> DocDiff {
    let mut d = DocDiff::default();
    for n in &b.nodes {
        match a.node(&n.id) {
            None => d.nodes_added.push(n.id.clone()),
            Some(old) => {
                let mut x = old.clone();
                x.position = n.position;
                if x != *n {
                    d.nodes_changed.push(n.id.clone());
                }
            }
        }
    }
    d.nodes_removed = a.nodes.iter().filter(|n| b.node(&n.id).is_none()).map(|n| n.id.clone()).collect();
    d.edges_added = b.edges.iter().filter(|e| !a.edges.contains(e)).cloned().collect();
    d.edges_removed = a.edges.iter().filter(|e| !b.edges.contains(e)).cloned().collect();
    d.meta_changed = a.name != b.name || a.description != b.description || a.settings != b.settings;
    let pretty = |doc: &WorkflowDoc| serde_json::to_string_pretty(doc).unwrap_or_default() + "\n";
    let (ta, tb) = (pretty(a), pretty(b));
    d.unified = similar::TextDiff::from_lines(&ta, &tb).unified_diff().header(a_label, b_label).to_string();
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_what_changed() {
        let a: WorkflowDoc = serde_json::from_value(json!({"name": "w", "nodes": [
            {"id": "t", "kind": "manual", "version": 1, "position": {"x": 0, "y": 0}},
            {"id": "s", "kind": "end", "version": 1}, {"id": "gone", "kind": "end", "version": 1}],
            "edges": [{"from": "t", "to": "gone"}]}))
        .unwrap();
        let b: WorkflowDoc = serde_json::from_value(json!({"name": "w2", "nodes": [
            {"id": "t", "kind": "manual", "version": 1, "position": {"x": 5, "y": 9}},
            {"id": "s", "kind": "end", "version": 1, "config": {"quiet": true}}, {"id": "new", "kind": "end", "version": 1}],
            "edges": [{"from": "t", "to": "s"}]}))
        .unwrap();
        let d = diff(&a, &b, "v1", "v2");
        assert_eq!(d.nodes_added, vec!["new"]);
        assert_eq!(d.nodes_removed, vec!["gone"]);
        assert_eq!(d.nodes_changed, vec!["s"], "a move alone is no change");
        assert_eq!(d.edges_added.len(), 1);
        assert_eq!(d.edges_removed[0].to, "gone");
        assert!(d.meta_changed);
        assert!(d.unified.starts_with("--- v1\n+++ v2\n"), "{}", d.unified);
        assert!(d.unified.contains("+  \"name\": \"w2\""));
        assert!(diff(&a, &a, "a", "a").unified.is_empty());
    }
}
