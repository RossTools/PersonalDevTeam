//! Conditions (Phase 7a spec §3.2): trigger filters, `condition` and `switch`. Data, not an expression language.

use super::refs::resolve;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Op {
    Eq,
    Ne,
    /// `right` is a list holding `left`.
    In,
    NotIn,
    Exists,
    Missing,
    Truthy,
    Falsy,
    Gt,
    Lt,
    /// A string holding `right`, or a list with it.
    Contains,
    /// A list sharing a value with the list `right` (or a string holding one of them): `{{trigger.labels}}` with
    /// any of a set of labels.
    ContainsAny,
    StartsWith,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub struct Condition {
    /// Usually a reference (`{{trigger.via}}`).
    #[ts(type = "unknown")]
    pub left: Value,
    pub op: Op,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "unknown")]
    pub right: Option<Value>,
}

/// JavaScript-like truthiness: null, false, 0, "" and empty lists and objects are false.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

/// Equal, with numbers compared by value and a number equal to its string form (`"12"` = `12`, as a reference into
/// text and a literal may differ in type).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Number(n), Value::String(s)) | (Value::String(s), Value::Number(n)) => n.to_string() == *s,
        _ => a == b,
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

impl Condition {
    /// Whether the condition holds, its references resolved from `ctx`.
    pub fn holds(&self, ctx: &Value) -> bool {
        let left = resolve(&self.left, ctx);
        let right = self.right.as_ref().map(|r| resolve(r, ctx)).unwrap_or(Value::Null);
        match self.op {
            Op::Eq => same(&left, &right),
            Op::Ne => !same(&left, &right),
            Op::In => right.as_array().is_some_and(|a| a.iter().any(|x| same(x, &left))),
            Op::NotIn => !right.as_array().is_some_and(|a| a.iter().any(|x| same(x, &left))),
            Op::Exists => !left.is_null(),
            Op::Missing => left.is_null(),
            Op::Truthy => truthy(&left),
            Op::Falsy => !truthy(&left),
            Op::Gt => matches!((number(&left), number(&right)), (Some(a), Some(b)) if a > b),
            Op::Lt => matches!((number(&left), number(&right)), (Some(a), Some(b)) if a < b),
            Op::Contains => match (&left, &right) {
                (Value::String(s), Value::String(r)) => s.contains(r.as_str()),
                (Value::Array(a), r) => a.iter().any(|x| same(x, r)),
                _ => false,
            },
            Op::ContainsAny => right.as_array().is_some_and(|rs| {
                rs.iter().any(|r| match (&left, r) {
                    (Value::String(s), Value::String(r)) => s.contains(r.as_str()),
                    (Value::Array(a), r) => a.iter().any(|x| same(x, r)),
                    _ => false,
                })
            }),
            Op::StartsWith => {
                matches!((&left, &right), (Value::String(s), Value::String(r)) if s.starts_with(r.as_str()))
            }
        }
    }
}

/// Every condition holds (an empty list does).
pub fn all(conds: &[Condition], ctx: &Value) -> bool {
    conds.iter().all(|c| c.holds(ctx))
}

/// Some condition holds (an empty list does not).
pub fn any(conds: &[Condition], ctx: &Value) -> bool {
    conds.iter().any(|c| c.holds(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn c(left: &str, op: Op, right: Option<Value>) -> Condition {
        Condition { left: json!(left), op, right }
    }

    #[test]
    fn evaluates_each_op() {
        let ctx = json!({"t": {"via": "detected", "pr": 12, "jobs": ["lint"], "none": null, "s": "pdt-PDT-1"}});
        assert!(c("{{t.via}}", Op::Eq, Some(json!("detected"))).holds(&ctx));
        assert!(c("{{t.via}}", Op::Ne, Some(json!("closed"))).holds(&ctx));
        assert!(c("{{t.pr}}", Op::Eq, Some(json!("12"))).holds(&ctx), "a number equals its text");
        assert!(c("{{t.via}}", Op::In, Some(json!(["detected", "merged_here"]))).holds(&ctx));
        assert!(c("{{t.via}}", Op::NotIn, Some(json!(["closed"]))).holds(&ctx));
        assert!(c("{{t.pr}}", Op::Exists, None).holds(&ctx));
        assert!(c("{{t.none}}", Op::Missing, None).holds(&ctx));
        assert!(c("{{t.nope}}", Op::Missing, None).holds(&ctx));
        assert!(c("{{t.jobs}}", Op::Truthy, None).holds(&ctx));
        assert!(c("{{t.none}}", Op::Falsy, None).holds(&ctx));
        assert!(c("{{t.pr}}", Op::Gt, Some(json!(3))).holds(&ctx));
        assert!(!c("{{t.pr}}", Op::Lt, Some(json!(3))).holds(&ctx));
        assert!(c("{{t.jobs}}", Op::Contains, Some(json!("lint"))).holds(&ctx));
        assert!(c("{{t.s}}", Op::Contains, Some(json!("PDT"))).holds(&ctx));
        assert!(c("{{t.jobs}}", Op::ContainsAny, Some(json!(["test", "lint"]))).holds(&ctx));
        assert!(!c("{{t.jobs}}", Op::ContainsAny, Some(json!(["test"]))).holds(&ctx));
        assert!(!c("{{t.jobs}}", Op::ContainsAny, Some(json!([]))).holds(&ctx), "none of nothing");
        assert!(!c("{{t.jobs}}", Op::ContainsAny, Some(json!("lint"))).holds(&ctx), "right is a list");
        assert!(c("{{t.s}}", Op::ContainsAny, Some(json!(["x", "PDT"]))).holds(&ctx));
        assert!(c("{{t.s}}", Op::StartsWith, Some(json!("pdt-"))).holds(&ctx));
        assert!(all(&[], &ctx) && !any(&[], &ctx));
    }
}
