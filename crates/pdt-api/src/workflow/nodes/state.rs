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
    #[ts(type = "unknown")]
    pub value: Value,
    /// Only when the key has no value yet (a first run's baseline).
    #[serde(default)]
    #[ts(type = "boolean | string")]
    pub if_missing: Tpl<bool>,
}

pub struct StateSet;

impl NodeKind for StateSet {
    fn spec(&self) -> KindSpec {
        spec::<StateSetConfig, Value>(
            "state.set",
            1,
            Category::Action,
            "Remember",
            "Stores a value in the workflow's state, read back as {{state.<key>}} in later runs.",
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
            if cx.dry_run {
                return Ok(Outcome::next("out", json!({"would": format!("set state {} to {}", c.key, c.value)})));
            }
            let written = cx
                .st
                .store
                .set_workflow_state(&cx.workflow.id, &c.key, &c.value, c.if_missing.or(false), now_ms())
                .await
                .map_err(|e| e.to_string())?;
            Ok(Outcome::next("out", json!({"written": written, "quiet": true})))
        })
    }
}
