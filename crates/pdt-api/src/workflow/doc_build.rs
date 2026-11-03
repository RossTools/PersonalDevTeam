//! Building blocks for the built-in workflows' documents, written as JSON.

use serde_json::{Value, json};

/// A node at (`x`, `y`), with its title and config when given (a blank title, a null config: left out).
pub(crate) fn node(id: &str, kind: &str, x: i32, y: i32, title: &str, config: Value) -> Value {
    let mut n = json!({"id": id, "kind": kind, "version": 1, "position": {"x": x, "y": y}});
    if !title.is_empty() {
        n["title"] = json!(title);
    }
    if !config.is_null() {
        n["config"] = config;
    }
    n
}

/// An edge from `from`'s `port` (blank: its only way out) to `to`.
pub(crate) fn edge(from: &str, port: &str, to: &str) -> Value {
    if port.is_empty() { json!({"from": from, "to": to}) } else { json!({"from": from, "port": port, "to": to}) }
}
