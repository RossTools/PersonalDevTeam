//! The node catalog (Phase 7a spec §4.5): every node kind is one module implementing [`NodeKind`], registered once in
//! [`catalog`]. The catalog serves `GET /api/workflows/catalog`, checks configs for validation, and runs nodes.

use crate::routes::AppState;
use pdt_core::workflow::api::{Run, Step, Workflow};
use pdt_core::workflow::doc::{ERROR_PORT, Node};
use pdt_core::workflow::validate::{KindFacts, Kinds};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use ts_rs::TS;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Category {
    Trigger,
    Condition,
    Action,
    Gate,
    Wait,
    Flow,
    /// An agent step (Phase 7b).
    Agent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PortSpec {
    pub id: String,
    pub title: String,
}

/// One kind as the catalog lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KindSpec {
    /// `session.send`; a trigger's id is its event type.
    pub id: String,
    pub version: u32,
    pub category: Category,
    pub title: String,
    pub description: String,
    /// JSON Schema of the node's config.
    #[ts(type = "Record<string, unknown>")]
    pub config_schema: Value,
    /// JSON Schema of what the node outputs (a trigger: the event payload).
    #[ts(type = "Record<string, unknown>")]
    pub output_schema: Value,
    /// The ports every node of the kind has; with `dynamicPorts`, more come from its config (cases, choices).
    pub ports: Vec<PortSpec>,
    pub dynamic_ports: bool,
}

/// What a node's execution came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Leave by `port` with `output`.
    Next { port: String, output: Value },
    /// Wait for what `wait` names (`{event: …}`, `{answer: …}`, `{delivery: …}`, `{run: …}`, `{timer: true}`), or until
    /// `until`, then take `timeout_port`.
    Wait { wait: Value, until: Option<i64>, timeout_port: Option<String>, output: Value },
    /// End the run (`quiet`: nothing to report).
    End { quiet: bool, output: Value },
}

impl Outcome {
    pub fn next(port: &str, output: impl Serialize) -> Self {
        Outcome::Next { port: port.into(), output: serde_json::to_value(output).unwrap_or(Value::Null) }
    }
}

/// What a running node can see and use.
pub struct StepCx<'a> {
    pub st: &'a AppState,
    pub workflow: &'a Workflow,
    pub run: &'a Run,
    pub step: &'a Step,
    /// The node being run (its title and settings), for kinds that need more than their config.
    pub node: &'a Node,
    /// The run's context (`trigger`, `steps`, `state`, `run`), for kinds that resolve more later.
    pub ctx: &'a Value,
    /// A dry run: report what would happen, change nothing.
    pub dry_run: bool,
}

impl StepCx<'_> {
    /// The node's title, else its id.
    pub fn node_title(&self) -> &str {
        if self.node.title.is_empty() { &self.node.id } else { &self.node.title }
    }

    /// The workflow and run, for records of what a workflow did (vision principle 5).
    pub fn by(&self) -> crate::finish::By {
        crate::finish::By {
            workflow_id: self.workflow.id.clone(),
            workflow_name: self.workflow.name.clone(),
            run_id: self.run.id.clone(),
        }
    }
}

/// One node kind.
pub trait NodeKind: Send + Sync {
    fn spec(&self) -> KindSpec;

    /// The ports a node with this (unresolved) config has.
    fn ports(&self, _config: &Value) -> Vec<String> {
        self.spec().ports.into_iter().map(|p| p.id).collect()
    }

    /// Whether `config` (references allowed) fits the kind.
    fn check(&self, config: &Value) -> Result<(), String>;

    /// A config written for an older version, brought up to this one.
    fn migrate(&self, from: u32, config: Value) -> Result<Value, String> {
        let v = self.spec().version;
        if from == v { Ok(config) } else { Err(format!("no migration from version {from} to {v}")) }
    }

    /// The event type a trigger kind fires on (None for other kinds).
    fn event_type(&self) -> Option<&str> {
        None
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>>;

    /// What the node would do, changing nothing. Kinds without side effects run as usual.
    fn dry_run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        self.run(cx, input)
    }

    /// How long one execution may take unless the node says.
    fn default_timeout_ms(&self) -> u64 {
        5 * 60 * 1000
    }
}

/// A config of type `C`, or why not.
pub fn parse<C: DeserializeOwned>(config: &Value) -> Result<C, String> {
    let v = if config.is_null() { Value::Object(Default::default()) } else { config.clone() };
    serde_json::from_value(v).map_err(|e| e.to_string())
}

/// The JSON Schema of `T`.
pub fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
}

pub fn ports(list: &[(&str, &str)]) -> Vec<PortSpec> {
    list.iter().map(|(id, title)| PortSpec { id: id.to_string(), title: title.to_string() }).collect()
}

/// A kind's spec from its config and output types.
#[allow(clippy::too_many_arguments)]
pub fn spec<C: JsonSchema, O: JsonSchema>(
    id: &str,
    version: u32,
    category: Category,
    title: &str,
    description: &str,
    port_list: &[(&str, &str)],
    dynamic_ports: bool,
) -> KindSpec {
    KindSpec {
        id: id.into(),
        version,
        category,
        title: title.into(),
        description: description.into(),
        config_schema: schema_of::<C>(),
        output_schema: schema_of::<O>(),
        ports: ports(port_list),
        dynamic_ports,
    }
}

pub struct Catalog {
    kinds: Vec<(KindSpec, Box<dyn NodeKind>)>,
}

impl Catalog {
    fn new(kinds: Vec<Box<dyn NodeKind>>) -> Self {
        Self { kinds: kinds.into_iter().map(|k| (k.spec(), k)).collect() }
    }

    pub fn get(&self, id: &str) -> Option<&dyn NodeKind> {
        self.kinds.iter().find(|(s, _)| s.id == id).map(|(_, k)| k.as_ref())
    }

    pub fn spec(&self, id: &str) -> Option<&KindSpec> {
        self.kinds.iter().find(|(s, _)| s.id == id).map(|(s, _)| s)
    }

    pub fn specs(&self) -> Vec<KindSpec> {
        self.kinds.iter().map(|(s, _)| s.clone()).collect()
    }

    #[cfg(test)]
    pub fn is_trigger(&self, id: &str) -> bool {
        self.spec(id).is_some_and(|s| s.category == Category::Trigger)
    }

    /// A node's config at the kind's current version.
    pub fn current_config(&self, node: &Node) -> Result<Value, String> {
        let (kind, spec) = self
            .get(&node.kind)
            .zip(self.spec(&node.kind))
            .ok_or_else(|| format!("no node kind is called {}", node.kind))?;
        if node.version > spec.version {
            return Err(format!(
                "{} was written for {} version {}; this PDT has {}",
                node.id, node.kind, node.version, spec.version
            ));
        }
        kind.migrate(node.version, node.config.clone())
    }
}

impl Kinds for Catalog {
    fn facts(&self, node: &Node) -> Option<KindFacts> {
        let kind = self.get(&node.kind)?;
        let spec = self.spec(&node.kind)?;
        let config = if node.version < spec.version {
            kind.migrate(node.version, node.config.clone())
        } else {
            Ok(node.config.clone())
        };
        let config_error = match &config {
            Ok(c) => kind.check(c).err(),
            Err(e) => Some(e.clone()),
        };
        let mut ports = kind.ports(config.as_ref().unwrap_or(&node.config));
        ports.retain(|p| p != ERROR_PORT);
        Some(KindFacts { version: spec.version, trigger: spec.category == Category::Trigger, ports, config_error })
    }
}

/// Every kind PDT knows. Adding one: a module in `nodes/` and a line here.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        use super::nodes::*;
        let mut kinds: Vec<Box<dyn NodeKind>> = vec![];
        kinds.extend(trigger::event_triggers());
        kinds.push(Box::new(trigger::Manual));
        kinds.push(Box::new(condition::ConditionKind));
        kinds.push(Box::new(switch::Switch));
        kinds.push(Box::new(gate::Gate));
        kinds.push(Box::new(wait::Wait));
        kinds.push(Box::new(call::Call));
        kinds.push(Box::new(end::End));
        kinds.push(Box::new(state::StateSet));
        kinds.push(Box::new(session_send::SessionSend));
        kinds.push(Box::new(ticket::Finish));
        kinds.push(Box::new(ticket::Move));
        kinds.push(Box::new(ticket::Comment));
        kinds.push(Box::new(agent::Agent));
        Catalog::new(kinds)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_unique_id_and_schemas() {
        let mut seen = std::collections::HashSet::new();
        for s in catalog().specs() {
            assert!(seen.insert(s.id.clone()), "{} twice", s.id);
            assert!(s.version >= 1);
            assert!(s.config_schema.is_object(), "{}", s.id);
            assert!(!s.title.is_empty() && !s.description.is_empty(), "{}", s.id);
        }
        for t in pdt_core::workflow::events::EVENT_TYPES {
            assert!(catalog().is_trigger(t.id), "{} has a trigger", t.id);
        }
    }
}
