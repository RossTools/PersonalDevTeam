//! `gate`: a human gate. It takes the port of the user's choice: one made ahead of time (`ahead`, e.g. Merge…'s
//! "finish once it merges", PDT-75) passes at once; otherwise it waits for an answer through the API (the Inbox in
//! 7b) or takes `unanswered` at once.

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::cond::{Condition, all};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const UNANSWERED: &str = "unanswered";
pub const TIMEOUT: &str = "timeout";

/// A choice made ahead of time: taken when every condition holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Ahead {
    pub when: Vec<Condition>,
    pub choice: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Ask {
    /// Wait for an answer (`POST /api/workflow-runs/{run}/steps/{step}/answer`).
    #[default]
    Wait,
    /// Don't ask: take `unanswered` (something else asks, e.g. the merged banner).
    Skip,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GateConfig {
    pub question: String,
    /// One port each.
    pub choices: Vec<String>,
    #[serde(default)]
    pub ahead: Vec<Ahead>,
    #[serde(default)]
    pub ask: Ask,
    /// How long to wait for an answer before taking `timeout`.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub timeout_ms: Option<u64>,
}

pub struct Gate;

impl NodeKind for Gate {
    fn spec(&self) -> KindSpec {
        spec::<GateConfig, Value>(
            "gate",
            1,
            Category::Gate,
            "Ask the user",
            "A human gate: one port per choice. A choice made ahead of time passes at once.",
            &[(UNANSWERED, "Not asked"), (TIMEOUT, "No answer in time")],
            true,
        )
    }

    fn ports(&self, config: &Value) -> Vec<String> {
        let mut out: Vec<String> = parse::<GateConfig>(config).map(|c| c.choices).unwrap_or_default();
        out.extend([UNANSWERED.to_string(), TIMEOUT.to_string()]);
        out
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GateConfig = parse(config)?;
        if c.choices.is_empty() {
            return Err("a gate needs at least one choice".into());
        }
        if c.choices.iter().any(|x| [UNANSWERED, TIMEOUT, "error"].contains(&x.as_str()) || x.trim().is_empty()) {
            return Err("choices can't be empty, unanswered, timeout or error".into());
        }
        if let Some(a) = c.ahead.iter().find(|a| !c.choices.contains(&a.choice)) {
            return Err(format!("ahead names {}, which is not a choice", a.choice));
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GateConfig = parse(&input)?;
            if let Some(a) = c.ahead.iter().find(|a| all(&a.when, cx.ctx)) {
                return Ok(Outcome::next(&a.choice, json!({"choice": a.choice, "answeredAhead": true})));
            }
            if c.ask == Ask::Skip {
                return Ok(Outcome::next(UNANSWERED, json!({"quiet": true})));
            }
            if cx.dry_run {
                return Ok(Outcome::next(
                    UNANSWERED,
                    json!({"would": format!("ask: {} ({})", c.question, c.choices.join(", "))}),
                ));
            }
            Ok(Outcome::Wait {
                wait: json!({"answer": {"question": c.question, "choices": c.choices}}),
                until: c.timeout_ms.map(|ms| now_ms() + i64::try_from(ms).unwrap_or(i64::MAX / 2)),
                timeout_port: Some(TIMEOUT.into()),
                output: Value::Null,
            })
        })
    }
}
