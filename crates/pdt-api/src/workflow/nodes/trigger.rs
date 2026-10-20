//! Triggers: one kind per event type (its id is the type), and `manual` ("Run now").

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, ports, schema_of};
use pdt_core::workflow::cond::Condition;
use pdt_core::workflow::events::{EVENT_TYPES, EventType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// Every trigger's config.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct TriggerConfig {
    /// Only these projects' events (a host-wide workflow); empty: every project.
    pub projects: Vec<String>,
    /// Only events for which every condition holds (`{{trigger.via}}` …).
    #[serde(rename = "where")]
    #[ts(rename = "where")]
    pub conditions: Vec<Condition>,
    /// The run's idempotency key: a second run with the same key is skipped. Default: the event.
    pub run_key: Option<String>,
}

/// A trigger on one event type.
pub struct EventTrigger(pub &'static EventType);

pub fn event_triggers() -> Vec<Box<dyn NodeKind>> {
    EVENT_TYPES.iter().map(|t| Box::new(EventTrigger(t)) as Box<dyn NodeKind>).collect()
}

impl NodeKind for EventTrigger {
    fn spec(&self) -> KindSpec {
        KindSpec {
            id: self.0.id.into(),
            version: 1,
            category: Category::Trigger,
            title: self.0.title.into(),
            description: self.0.description.into(),
            config_schema: schema_of::<TriggerConfig>(),
            output_schema: serde_json::to_value((self.0.schema)()).unwrap_or(Value::Null),
            ports: ports(&[("out", "Then")]),
            dynamic_ports: false,
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<TriggerConfig>(config).map(|_| ())
    }

    fn event_type(&self) -> Option<&str> {
        Some(self.0.id)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move { Ok(Outcome::Next { port: "out".into(), output: cx.run.trigger.clone() }) })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct ManualConfig {
    /// The run's idempotency key (`{{trigger.date}}`); default: every run is new.
    pub run_key: Option<String>,
}

/// "Run now" (`POST /api/workflows/{id}/run`) and sub-workflow calls: the input is the trigger.
pub struct Manual;

pub const MANUAL: &str = "manual";

impl NodeKind for Manual {
    fn spec(&self) -> KindSpec {
        crate::workflow::catalog::spec::<ManualConfig, Value>(
            MANUAL,
            1,
            Category::Trigger,
            "Run now",
            "Started by hand (Run now, the API) or by another workflow; its input is the trigger.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<ManualConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move { Ok(Outcome::Next { port: "out".into(), output: cx.run.trigger.clone() }) })
    }
}
