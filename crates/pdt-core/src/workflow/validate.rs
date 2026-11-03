//! Validating a workflow document against what the catalog says about each kind.

use super::doc::{ERROR_PORT, WorkflowDoc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Severity {
    /// The document can't be saved or run.
    Error,
    /// It runs, but probably not as meant.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Issue {
    pub severity: Severity,
    /// `unknown_kind`, `invalid_config`, `unreachable`, …
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    pub message: String,
}

/// What validation needs to know about one node's kind, with that node's config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindFacts {
    /// The catalog's version of the kind.
    pub version: u32,
    pub trigger: bool,
    /// The ports this config gives the node (an `end` node has none).
    pub ports: Vec<String>,
    /// Why the config doesn't fit the kind (after migrating an older version).
    pub config_error: Option<String>,
}

/// The catalog, as validation sees it: facts for a node (None for an unknown kind).
pub trait Kinds {
    fn facts(&self, node: &super::doc::Node) -> Option<KindFacts>;

    /// The issue's code and message for a node of a kind it doesn't have (`unknown_kind`; pdt-api names the
    /// integration a kind belongs to when that is turned off).
    fn unknown(&self, kind: &str) -> (String, String) {
        ("unknown_kind".into(), format!("no node kind is called {kind}"))
    }
}

fn issue(severity: Severity, code: &str, node: Option<&str>, message: String) -> Issue {
    Issue { severity, code: code.into(), node: node.map(String::from), message }
}

/// Every problem with `doc`, errors first.
pub fn validate(doc: &WorkflowDoc, kinds: &dyn Kinds) -> Vec<Issue> {
    use Severity::{Error, Warning};
    let mut out = vec![];
    if doc.name.trim().is_empty() {
        out.push(issue(Error, "no_name", None, "the workflow needs a name".into()));
    }
    let mut seen = HashSet::new();
    let mut facts: HashMap<&str, KindFacts> = HashMap::new();
    for n in &doc.nodes {
        if n.id.trim().is_empty() {
            out.push(issue(Error, "no_id", None, format!("a {} node has no id", n.kind)));
            continue;
        }
        if !seen.insert(n.id.as_str()) {
            out.push(issue(Error, "duplicate_id", Some(&n.id), format!("two nodes are called {}", n.id)));
            continue;
        }
        let Some(f) = kinds.facts(n) else {
            let (code, message) = kinds.unknown(&n.kind);
            out.push(issue(Error, &code, Some(&n.id), message));
            continue;
        };
        if n.version > f.version {
            out.push(issue(
                Error,
                "newer_version",
                Some(&n.id),
                format!(
                    "{} {} was written for version {}; this PDT has version {}",
                    n.kind, n.id, n.version, f.version
                ),
            ));
        } else if n.version < f.version {
            out.push(issue(
                Warning,
                "older_version",
                Some(&n.id),
                format!(
                    "{} was written for {} version {}; it is migrated to version {} when it runs",
                    n.id, n.kind, n.version, f.version
                ),
            ));
        }
        if let Some(e) = &f.config_error {
            out.push(issue(Error, "invalid_config", Some(&n.id), format!("{}: {e}", n.id)));
        }
        facts.insert(n.id.as_str(), f);
    }
    let triggers: Vec<&str> = doc
        .nodes
        .iter()
        .filter(|n| facts.get(n.id.as_str()).is_some_and(|f| f.trigger))
        .map(|n| n.id.as_str())
        .collect();
    if triggers.is_empty() {
        out.push(issue(Error, "no_trigger", None, "the workflow has no trigger".into()));
    }
    for e in &doc.edges {
        let Some(from) = facts.get(e.from.as_str()) else {
            if !seen.contains(e.from.as_str()) {
                out.push(issue(Error, "unknown_node", None, format!("an edge leaves {}, which is not a node", e.from)));
            }
            continue;
        };
        if !seen.contains(e.to.as_str()) {
            out.push(issue(
                Error,
                "unknown_node",
                Some(&e.from),
                format!("an edge from {} goes to {}, which is not a node", e.from, e.to),
            ));
            continue;
        }
        if facts.get(e.to.as_str()).is_some_and(|f| f.trigger) {
            out.push(issue(
                Error,
                "edge_to_trigger",
                Some(&e.to),
                format!("{} is a trigger: nothing can lead into it", e.to),
            ));
        }
        if e.port != ERROR_PORT && !from.ports.contains(&e.port) {
            out.push(issue(
                Error,
                "unknown_port",
                Some(&e.from),
                format!(
                    "{} has no port {} (it has {})",
                    e.from,
                    e.port,
                    if from.ports.is_empty() { "none".to_string() } else { from.ports.join(", ") }
                ),
            ));
        }
    }
    // Reachable from a trigger.
    let mut reached: HashSet<&str> = triggers.iter().copied().collect();
    let mut queue: VecDeque<&str> = triggers.iter().copied().collect();
    while let Some(id) = queue.pop_front() {
        for e in doc.edges.iter().filter(|e| e.from == id) {
            if reached.insert(e.to.as_str()) {
                queue.push_back(e.to.as_str());
            }
        }
    }
    for n in &doc.nodes {
        if facts.contains_key(n.id.as_str()) && !reached.contains(n.id.as_str()) {
            out.push(issue(Warning, "unreachable", Some(&n.id), format!("no trigger leads to {}", n.id)));
        }
        if let Some(f) = facts.get(n.id.as_str()) {
            for p in f.ports.iter().filter(|p| !doc.has_edge_from(&n.id, p)) {
                out.push(issue(
                    Warning,
                    "open_port",
                    Some(&n.id),
                    format!("{}'s {p} leads nowhere: a run that takes it ends there", n.id),
                ));
            }
        }
    }
    out.sort_by_key(|i| i.severity != Error);
    out
}

/// Whether the issues allow saving and running.
pub fn is_valid(issues: &[Issue]) -> bool {
    !issues.iter().any(|i| i.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::doc::Node;
    use serde_json::json;

    struct Fake;
    impl Kinds for Fake {
        fn facts(&self, n: &Node) -> Option<KindFacts> {
            let f = |trigger: bool, ports: &[&str]| KindFacts {
                version: 2,
                trigger,
                ports: ports.iter().map(|p| p.to_string()).collect(),
                config_error: n.config.get("bad").map(|_| "bad is not allowed".to_string()),
            };
            match n.kind.as_str() {
                "t" => Some(f(true, &["out"])),
                "if" => Some(f(false, &["true", "false"])),
                "end" => Some(f(false, &[])),
                _ => None,
            }
        }
    }

    fn doc(v: serde_json::Value) -> WorkflowDoc {
        serde_json::from_value(v).unwrap()
    }

    fn codes(issues: &[Issue]) -> Vec<(&str, Option<&str>)> {
        issues.iter().map(|i| (i.code.as_str(), i.node.as_deref())).collect()
    }

    #[test]
    fn a_good_document_has_only_open_port_warnings() {
        let d = doc(json!({"name": "w", "nodes": [
            {"id": "a", "kind": "t", "version": 2}, {"id": "b", "kind": "if", "version": 2},
            {"id": "c", "kind": "end", "version": 2}],
            "edges": [{"from": "a", "to": "b"}, {"from": "b", "port": "true", "to": "c"}]}));
        let issues = validate(&d, &Fake);
        assert_eq!(codes(&issues), vec![("open_port", Some("b"))]);
        assert!(is_valid(&issues));
    }

    #[test]
    fn finds_each_problem() {
        let d = doc(json!({"name": " ", "nodes": [
            {"id": "a", "kind": "if", "version": 3},
            {"id": "a", "kind": "if", "version": 2},
            {"id": "x", "kind": "nope", "version": 1},
            {"id": "c", "kind": "end", "version": 1, "config": {"bad": 1}}],
            "edges": [{"from": "a", "port": "maybe", "to": "c"}, {"from": "c", "to": "ghost"}, {"from": "ghost", "to": "a"}]}));
        let issues = validate(&d, &Fake);
        let got = codes(&issues);
        for want in [
            ("no_name", None),
            ("duplicate_id", Some("a")),
            ("unknown_kind", Some("x")),
            ("newer_version", Some("a")),
            ("older_version", Some("c")),
            ("invalid_config", Some("c")),
            ("no_trigger", None),
            ("unknown_port", Some("a")),
            ("unknown_node", Some("c")),
            ("unknown_node", None),
            ("unreachable", Some("a")),
        ] {
            assert!(got.contains(&want), "{want:?} in {got:?}");
        }
        assert!(!is_valid(&issues));
        assert_eq!(issues[0].severity, Severity::Error, "errors first");
    }

    #[test]
    fn nothing_leads_into_a_trigger_and_error_ports_are_always_there() {
        let d = doc(json!({"name": "w", "nodes": [
            {"id": "a", "kind": "t", "version": 2}, {"id": "b", "kind": "end", "version": 2}],
            "edges": [{"from": "a", "to": "b"}, {"from": "a", "port": "error", "to": "b"}, {"from": "b", "to": "a"}]}));
        let issues = validate(&d, &Fake);
        let got = codes(&issues).into_iter().map(|(c, _)| c).collect::<Vec<_>>();
        assert!(got.contains(&"edge_to_trigger"));
        assert!(got.contains(&"unknown_port"), "end has no out port");
        assert_eq!(got.iter().filter(|c| **c == "unknown_port").count(), 1, "the error port is fine");
    }
}
