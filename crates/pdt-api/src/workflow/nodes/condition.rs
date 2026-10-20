//! `condition`: true or false from a list of conditions.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::cond::{Condition, all, any};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Match {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ConditionConfig {
    pub conditions: Vec<Condition>,
    #[serde(default, rename = "match")]
    #[ts(rename = "match")]
    pub matching: Match,
}

pub struct ConditionKind;

impl NodeKind for ConditionKind {
    fn spec(&self) -> KindSpec {
        spec::<ConditionConfig, Value>(
            "condition",
            1,
            Category::Condition,
            "If",
            "Takes `true` when all (or any) of its conditions hold, else `false`.",
            &[("true", "True"), ("false", "False")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<ConditionConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: ConditionConfig = parse(&input)?;
            // The input is resolved already; conditions still see the context for anything left.
            let ok = match c.matching {
                Match::All => all(&c.conditions, cx.ctx),
                Match::Any => any(&c.conditions, cx.ctx),
            };
            Ok(Outcome::next(if ok { "true" } else { "false" }, json!({"result": ok})))
        })
    }
}
