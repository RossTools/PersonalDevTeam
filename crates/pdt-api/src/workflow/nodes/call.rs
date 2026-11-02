//! `workflow.call`: runs another workflow (its `manual` trigger) with an input and waits for it; the child's output
//! is this node's.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CallConfig {
    /// The workflow's id.
    pub workflow: String,
    #[serde(default)]
    #[ts(type = "unknown")]
    #[schemars(extend("x-format" = "code", "x-language" = "json"))]
    pub input: Value,
}

pub const KIND: &str = "workflow.call";

pub struct Call;

impl NodeKind for Call {
    fn spec(&self) -> KindSpec {
        spec::<CallConfig, Value>(
            KIND,
            1,
            Category::Flow,
            "Run a workflow",
            "Runs another workflow (from its Run now trigger) with an input and waits for it to end.",
            &[("done", "It succeeded"), ("failed", "It failed")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<CallConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CallConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next(
                    "done",
                    json!({"would": format!("run workflow {} with {}", c.workflow, c.input)}),
                ));
            }
            let child = crate::workflow::engine::start_child(cx, &c.workflow, c.input).await?;
            Ok(Outcome::Wait {
                wait: json!({"run": child}),
                until: None,
                timeout_port: None,
                output: json!({"runId": child}),
            })
        })
    }
}
