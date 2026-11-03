//! `end`: ends the run, quietly ("nothing to report") or with an output.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, parses, spec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct EndConfig {
    /// Nothing to report: nobody is told; the run still shows in the runs list.
    pub quiet: bool,
    /// The run's output (a sub-workflow's result).
    #[ts(type = "unknown")]
    #[schemars(extend("x-format" = "code", "x-language" = "json"))]
    pub output: Value,
}

pub struct End;

impl NodeKind for End {
    fn spec(&self) -> KindSpec {
        spec::<EndConfig, Value>(
            "end",
            1,
            Category::Flow,
            "End",
            "Ends the run, quietly or with an output.",
            &[],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<EndConfig>(config)
    }

    fn run<'a>(&'a self, _cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: EndConfig = parse(&input)?;
            Ok(Outcome::End { quiet: c.quiet, output: c.output })
        })
    }
}
