//! `wait`: until a time, for a while, or for an event (with a timeout). An event wait's conditions use
//! `{{event.…}}` for the awaited event; other references are filled in when the wait starts.

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::cond::Condition;
use pdt_core::workflow::tpl::Tpl;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct EventWait {
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub kind: String,
    /// More event types that end the wait too (PDT-151: CI passing or failing).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub also: Vec<String>,
    #[serde(default, rename = "where")]
    #[ts(rename = "where")]
    pub conditions: Vec<Condition>,
    /// With `where`, at least one of these holds too (when given).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<Condition>>", optional)]
    pub any: Vec<Condition>,
}

impl EventWait {
    /// Every type is a known event type: PDT's own, or an integration's (enabled or not: a wait for a turned-off
    /// integration's event just never ends with it).
    pub fn check(&self) -> Result<(), String> {
        match std::iter::once(&self.kind).chain(&self.also).find(|k| {
            pdt_core::workflow::events::event_type(k).is_none() && pdt_core::integration::owner_of_kind(k).is_none()
        }) {
            Some(k) => Err(format!("no event type is called {k}")),
            None => Ok(()),
        }
    }

    /// The step's `wait` for it.
    pub fn wait(&self) -> Value {
        let mut w = json!({"type": self.kind, "where": self.conditions});
        if !self.also.is_empty() {
            w["also"] = json!(self.also);
        }
        if !self.any.is_empty() {
            w["any"] = json!(self.any);
        }
        w
    }
}

/// Whether `event` (its trigger value) ends a step's `wait.event`: every `where` holds, and one of `any` if given.
pub fn wait_matches(w: &Value, event: &Value) -> bool {
    let conds = |k: &str| -> Vec<Condition> {
        w.get(k).cloned().and_then(|c| serde_json::from_value(c).ok()).unwrap_or_default()
    };
    let ctx = json!({"event": event});
    let any = conds("any");
    conds("where").iter().all(|c| c.holds(&ctx)) && (any.is_empty() || any.iter().any(|c| c.holds(&ctx)))
}

/// The event types a step's `wait.event` takes: its `type` and `also`.
pub fn wait_types(w: &Value) -> Vec<&str> {
    let also = w.get("also").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
    w.get("type").and_then(Value::as_str).into_iter().chain(also).collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct WaitConfig {
    /// Wait this long.
    #[ts(type = "number | string | null")]
    pub ms: Option<Tpl<u64>>,
    /// Wait until this time (ms since the epoch).
    #[ts(type = "number | string | null")]
    pub until: Option<Tpl<i64>>,
    /// Wait for an event; `timeoutMs` bounds it.
    pub event: Option<EventWait>,
    #[ts(type = "number | null")]
    pub timeout_ms: Option<u64>,
    /// The ticket's pipeline stage while it waits (the Board's column).
    #[ts(optional)]
    pub pipeline_stage: Option<pdt_core::ticket::PipelineStage>,
}

pub struct Wait;

impl NodeKind for Wait {
    fn spec(&self) -> KindSpec {
        spec::<WaitConfig, Value>(
            "wait",
            1,
            Category::Wait,
            "Wait",
            "Waits for a while, until a time, or for an event (then `event`, or `timeout`).",
            &[("done", "Time is up"), ("event", "The event came"), ("timeout", "No event in time")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: WaitConfig = parse(config)?;
        match (&c.ms, &c.until, &c.event) {
            (None, None, None) => Err("say how long (ms), until when (until) or for what (event)".into()),
            (_, _, Some(e)) => e.check(),
            _ => Ok(()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: WaitConfig = parse(&input)?;
            if cx.dry_run {
                let port = if c.event.is_some() { "event" } else { "done" };
                return Ok(Outcome::next(port, json!({"would": "wait"})));
            }
            crate::workflow::nodes::set_stage(cx, c.pipeline_stage).await;
            let now = now_ms();
            if let Some(e) = c.event {
                let until = c.timeout_ms.map(|ms| now + i64::try_from(ms).unwrap_or(i64::MAX / 2));
                return Ok(Outcome::Wait {
                    wait: json!({"event": e.wait()}),
                    until,
                    timeout_port: Some("timeout".into()),
                    output: Value::Null,
                });
            }
            let until = match (
                c.until.as_ref().and_then(|t| t.value().copied()),
                c.ms.as_ref().and_then(|m| m.value().copied()),
            ) {
                (Some(t), _) => t,
                (None, Some(ms)) => now + i64::try_from(ms).unwrap_or(i64::MAX / 2),
                (None, None) => now,
            };
            Ok(Outcome::Wait {
                wait: json!({"timer": true}),
                until: Some(until),
                timeout_port: Some("done".into()),
                output: Value::Null,
            })
        })
    }
}
