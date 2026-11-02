//! Triggers: one kind per event type (its id is the type), and `manual` ("Run now").

use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, in_scope, parse, ports, schema_of,
};
use pdt_core::workflow::api::{LogEvent, Workflow};
use pdt_core::workflow::cond::Condition;
use pdt_core::workflow::doc::Node;
use pdt_core::workflow::events::{EventType, OWN_RUN_EVENTS, from_own_run};
use pdt_core::workflow::inputs::{InputField, RunConfirm, apply, check_fields};
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

/// The config of a trigger on an event a workflow's own runs can cause (`run.*`, `session.*`, `inbox.*`; PDT-364).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct OwnRunTriggerConfig {
    /// Only these projects' events (a host-wide workflow); empty: every project.
    pub projects: Vec<String>,
    /// Only events for which every condition holds (`{{trigger.via}}` …).
    #[serde(rename = "where")]
    #[ts(rename = "where")]
    pub conditions: Vec<Condition>,
    /// The run's idempotency key: a second run with the same key is skipped. Default: the event.
    pub run_key: Option<String>,
    /// Also events this workflow's own runs caused (its run failing, its agent step's session, its Inbox item). Off by
    /// default, so a workflow never reacts to itself.
    pub include_own: bool,
}

/// The fields every trigger kind has, read leniently (a kind's own config adds more): what the engine checks.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SharedTrigger {
    pub projects: Vec<String>,
    #[serde(rename = "where")]
    pub conditions: Vec<Condition>,
    pub run_key: Option<String>,
}

/// A trigger on one event type: the registry makes one for each type no node kind claims
/// (`crate::integrations::Registry::node_kinds`).
pub struct EventTrigger(pub &'static EventType);

impl EventTrigger {
    /// Its events can come from the workflow's own runs: it has `includeOwn`.
    fn guarded(&self) -> bool {
        OWN_RUN_EVENTS.contains(&self.0.id)
    }
}

impl NodeKind for EventTrigger {
    fn spec(&self) -> KindSpec {
        KindSpec {
            id: self.0.id.into(),
            version: 1,
            category: Category::Trigger,
            title: self.0.title.into(),
            description: self.0.description.into(),
            config_schema: if self.guarded() {
                schema_of::<OwnRunTriggerConfig>()
            } else {
                schema_of::<TriggerConfig>()
            },
            output_schema: serde_json::to_value((self.0.schema)()).unwrap_or(Value::Null),
            ports: ports(&[("out", "Then")]),
            dynamic_ports: false,
            preset: None,
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        if self.guarded() {
            return parse::<OwnRunTriggerConfig>(config).map(|_| ());
        }
        parse::<TriggerConfig>(config).map(|_| ())
    }

    fn event_type(&self) -> Option<&str> {
        Some(self.0.id)
    }

    /// In scope, and not from one of this workflow's own runs unless the trigger includes them (PDT-364).
    fn accepts(&self, config: &Value, wf: &Workflow, _node: &Node, event: &LogEvent) -> bool {
        if !in_scope(wf, event) {
            return false;
        }
        let include_own = config.get("includeOwn").and_then(Value::as_bool) == Some(true);
        include_own || !from_own_run(&event.kind, &event.payload, &wf.id)
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
    /// The fields Run now asks for (PDT-301); every manual start's input is checked against them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<InputField>>", optional)]
    pub inputs: Vec<InputField>,
    /// What Run now asks before it starts, e.g. for a stable release.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub confirm: Option<RunConfirm>,
}

impl ManualConfig {
    /// The input checked against the declared fields (defaults filled in).
    pub fn input(&self, input: &Value) -> Result<Value, String> {
        apply(&self.inputs, input)
    }
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
            "Started by hand (Run now, the API) or by another workflow; its input is the trigger. `inputs` declares the \
             fields Run now asks for (each `{{trigger.<id>}}`): a start's input is checked against them, defaults \
             filled in; `confirm` is asked before Run now starts.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: ManualConfig = parse(config)?;
        check_fields(&c.inputs, c.confirm.as_ref())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move { Ok(Outcome::Next { port: "out".into(), output: cx.run.trigger.clone() }) })
    }
}
