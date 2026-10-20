//! `switch`: one port per case, `default` when none matches.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::cond::{Condition, Op};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Case {
    pub port: String,
    #[ts(type = "unknown")]
    pub equals: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct SwitchConfig {
    /// Usually a reference (`{{trigger.to}}`).
    #[ts(type = "unknown")]
    pub value: Value,
    pub cases: Vec<Case>,
}

pub struct Switch;

pub const DEFAULT: &str = "default";

impl NodeKind for Switch {
    fn spec(&self) -> KindSpec {
        spec::<SwitchConfig, Value>(
            "switch",
            1,
            Category::Condition,
            "Switch",
            "Takes the port of the first case whose value equals `value`, else `default`.",
            &[(DEFAULT, "Default")],
            true,
        )
    }

    fn ports(&self, config: &Value) -> Vec<String> {
        let mut out: Vec<String> =
            parse::<SwitchConfig>(config).map(|c| c.cases.into_iter().map(|c| c.port).collect()).unwrap_or_default();
        out.push(DEFAULT.into());
        out
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: SwitchConfig = parse(config)?;
        if c.cases.iter().any(|c| c.port.trim().is_empty() || c.port == DEFAULT || c.port == "error") {
            return Err("a case needs a port name other than default and error".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, _cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: SwitchConfig = parse(&input)?;
            let hit = c.cases.iter().find(|k| {
                Condition { left: c.value.clone(), op: Op::Eq, right: Some(k.equals.clone()) }.holds(&Value::Null)
            });
            Ok(Outcome::next(hit.map(|k| k.port.as_str()).unwrap_or(DEFAULT), json!({"value": c.value})))
        })
    }
}
