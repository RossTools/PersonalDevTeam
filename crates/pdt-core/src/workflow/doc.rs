//! A workflow document (Phase 7a spec §3.1): nodes, edges, settings and layout. Stored as JSON, one version per save.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The port every node without named outcomes leaves by.
pub const OUT: &str = "out";
/// The port a node takes, once its retries are spent, when it has an edge from it (else the run fails).
pub const ERROR_PORT: &str = "error";
/// How often a run may visit one node unless the node says otherwise (a loop limit).
pub const DEFAULT_MAX_VISITS: u32 = 25;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowDoc {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub settings: RunSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Node {
    /// Unique in the document; `{{steps.<id>}}` refers to its output.
    pub id: String,
    /// The catalog kind (`session.send`).
    pub kind: String,
    /// The kind's version this config was written for.
    pub version: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub config: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Retry>,
    /// The longest one execution may take (actions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub timeout_ms: Option<u64>,
    /// Loop limit: more visits in one run fail the step. Default [`DEFAULT_MAX_VISITS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_visits: Option<u32>,
}

impl Node {
    pub fn max_visits(&self) -> u32 {
        self.max_visits.unwrap_or(DEFAULT_MAX_VISITS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[ts(export)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// Retry a failed execution up to `max` more times, waiting `backoff_ms`, doubled each time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Retry {
    pub max: u32,
    #[serde(default)]
    #[ts(type = "number")]
    pub backoff_ms: u64,
}

impl Retry {
    /// The wait before retry number `n` (1-based).
    pub fn delay_ms(&self, n: u32) -> u64 {
        self.backoff_ms.saturating_mul(1u64 << n.saturating_sub(1).min(16))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Edge {
    pub from: String,
    #[serde(default = "out_port")]
    pub port: String,
    pub to: String,
}

fn out_port() -> String {
    OUT.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct RunSettings {
    /// Steps one run may execute before it fails.
    pub max_steps: u32,
    /// How long one run may last before it fails (waits included).
    #[ts(type = "number")]
    pub max_runtime_ms: u64,
}

impl Default for RunSettings {
    fn default() -> Self {
        Self { max_steps: 200, max_runtime_ms: 7 * 24 * 60 * 60 * 1000 }
    }
}

impl WorkflowDoc {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// The nodes an edge from `from`'s `port` leads to, in document order.
    pub fn next(&self, from: &str, port: &str) -> Vec<&str> {
        self.edges.iter().filter(|e| e.from == from && e.port == port).map(|e| e.to.as_str()).collect()
    }

    pub fn has_edge_from(&self, from: &str, port: &str) -> bool {
        self.edges.iter().any(|e| e.from == from && e.port == port)
    }

    /// Whether a node of `kind` is `from` or lies on some path of edges from it.
    pub fn reaches(&self, from: &str, kind: &str) -> bool {
        let mut seen = std::collections::HashSet::new();
        let mut todo = vec![from];
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            if self.node(id).is_some_and(|n| n.kind == kind) {
                return true;
            }
            todo.extend(self.edges.iter().filter(|e| e.from == id).map(|e| e.to.as_str()));
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_minimal_document_with_defaults() {
        let d: WorkflowDoc = serde_json::from_value(serde_json::json!({
            "name": "n",
            "nodes": [{"id": "a", "kind": "manual", "version": 1}, {"id": "b", "kind": "end", "version": 1}],
            "edges": [{"from": "a", "to": "b"}],
        }))
        .unwrap();
        assert_eq!(d.edges[0].port, OUT);
        assert_eq!(d.next("a", OUT), vec!["b"]);
        assert!(d.next("a", "x").is_empty());
        assert_eq!(d.node("b").unwrap().max_visits(), DEFAULT_MAX_VISITS);
        assert_eq!(d.settings, RunSettings::default());
        assert!(d.has_edge_from("a", OUT));
    }

    #[test]
    fn reaches_follows_every_port_through_loops() {
        let d: WorkflowDoc = serde_json::from_value(serde_json::json!({
            "name": "n",
            "nodes": [{"id": "a", "kind": "gate", "version": 1}, {"id": "b", "kind": "wait", "version": 1},
                      {"id": "f", "kind": "ticket.finish", "version": 1}, {"id": "e", "kind": "end", "version": 1}],
            "edges": [{"from": "a", "port": "no", "to": "e"}, {"from": "a", "port": "yes", "to": "b"},
                      {"from": "b", "to": "a"}, {"from": "b", "port": "event", "to": "f"}],
        }))
        .unwrap();
        assert!(d.reaches("a", "ticket.finish"));
        assert!(d.reaches("f", "ticket.finish"), "a node reaches itself");
        assert!(!d.reaches("e", "ticket.finish"));
        assert!(!d.reaches("gone", "ticket.finish"));
    }

    #[test]
    fn retry_backs_off_doubling() {
        let r = Retry { max: 3, backoff_ms: 100 };
        assert_eq!((r.delay_ms(1), r.delay_ms(2), r.delay_ms(3)), (100, 200, 400));
    }
}
