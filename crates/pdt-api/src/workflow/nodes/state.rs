//! `state.set`: a value in the workflow's own key-value state (`{{state.<key>}}`), e.g. a baseline or "last seen".

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::tpl::Tpl;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct StateSetConfig {
    pub key: String,
    /// The value (not used with `add`).
    #[serde(default)]
    #[ts(type = "unknown")]
    pub value: Value,
    /// Only when the key has no value yet (a first run's baseline).
    #[serde(default)]
    #[ts(type = "boolean | string")]
    pub if_missing: Tpl<bool>,
    /// A counter instead: adds this to the number the key holds (none counts as 0); the output's `value` is the sum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub add: Option<f64>,
}

pub struct StateSet;

/// A whole number as an integer (`3`, not `3.0`).
fn number(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 9e15 { json!(f as i64) } else { json!(f) }
}

impl NodeKind for StateSet {
    fn spec(&self) -> KindSpec {
        spec::<StateSetConfig, Value>(
            "state.set",
            1,
            Category::Action,
            "Remember",
            "Stores a value in the workflow's state, read back as {{state.<key>}} in later runs (and as the output's \
             `value` in this one). With `add`, counts \
             instead: the key's number plus `add`, also in the output's `value` (a key per attempt, \
             `tries.{{trigger.attemptId}}`, counts per attempt).",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: StateSetConfig = parse(config)?;
        if c.key.trim().is_empty() {
            return Err("key is empty".into());
        }
        if c.key.starts_with(super::schedule::RESERVED_STATE) {
            return Err(format!("keys starting {} are PDT's own", super::schedule::RESERVED_STATE));
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: StateSetConfig = parse(&input)?;
            if let Some(by) = c.add {
                let value = if cx.dry_run {
                    let state = cx.st.store.workflow_state(&cx.workflow.id).await.map_err(|e| e.to_string())?;
                    state.get(&c.key).and_then(Value::as_f64).unwrap_or(0.0) + by
                } else {
                    cx.st
                        .store
                        .add_workflow_state(&cx.workflow.id, &c.key, by, now_ms())
                        .await
                        .map_err(|e| e.to_string())?
                };
                let value = number(value);
                let mut out = json!({"value": value, "quiet": true});
                if cx.dry_run {
                    out["would"] = json!(format!("count {} up to {value}", c.key));
                }
                return Ok(Outcome::next("out", out));
            }
            if cx.dry_run {
                let would = format!("set state {} to {}", c.key, c.value);
                return Ok(Outcome::next("out", json!({"would": would, "value": c.value})));
            }
            let written = cx
                .st
                .store
                .set_workflow_state(&cx.workflow.id, &c.key, &c.value, c.if_missing.or(false), now_ms())
                .await
                .map_err(|e| e.to_string())?;
            Ok(Outcome::next("out", json!({"written": written, "value": c.value, "quiet": true})))
        })
    }
}
