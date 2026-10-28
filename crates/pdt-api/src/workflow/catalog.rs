//! The node catalog (Phase 7a spec §4.5): every node kind is one module implementing [`NodeKind`], registered once in
//! [`catalog`]. The catalog serves `GET /api/workflows/catalog`, checks configs for validation, and runs nodes.

use crate::routes::AppState;
use pdt_core::brand::SHORT_NAME;
use pdt_core::workflow::api::{LogEvent, Run, Scope, Step, Workflow};
use pdt_core::workflow::doc::{ERROR_PORT, Node};
use pdt_core::workflow::validate::{KindFacts, Kinds};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
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

/// What adding a catalog entry makes: a node of `kind` with `config` (an MCP tool's entry is an `mcp.tool` node).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KindPreset {
    pub kind: String,
    #[ts(type = "Record<string, unknown>")]
    pub config: Value,
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
    /// An entry that is a preset of another kind (`mcp:<server>/<tool>`): adding it adds that kind's node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub preset: Option<KindPreset>,
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
    /// The step ran before and was cut short (PDT stopped while it ran, or it timed out): it may have done some or all
    /// of its work. `step.output` holds the progress it recorded ([`StepCx::save_progress`]).
    pub rerun: bool,
}

impl StepCx<'_> {
    /// The node's title, else its id.
    pub fn node_title(&self) -> &str {
        if self.node.title.is_empty() { &self.node.id } else { &self.node.title }
    }

    /// Records what the step has done so far (as its output), so running it again after a restart or a retry picks up
    /// from there (`cx.step.output`) instead of doing it twice.
    pub async fn save_progress(&self, progress: &Value) -> Result<(), String> {
        self.st
            .store
            .set_step_progress(&self.step.id, self.step.attempts, progress)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
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

    /// Whether this trigger node (its current `config`) fires for `event`, before the shared `projects` and `where`
    /// checks. Default: the event is in the workflow's scope. Schedules and webhooks fire for their own events only.
    fn accepts(&self, _config: &Value, wf: &Workflow, _node: &Node, event: &LogEvent) -> bool {
        in_scope(wf, event)
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

    /// The config fields whose text goes to an agent (an agent step's prompt, a message): the trigger's outside text
    /// in them is fenced (PDT-202).
    fn agent_text(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether an action's body runs under the engine lock (PDT-128): for a quick one whose wait the engine's own
    /// passes end (the outbox's delivery), so it is waiting before they look. Other actions and agent steps run
    /// outside it; flow, gate, wait and condition kinds always run under it.
    fn in_engine_lock(&self) -> bool {
        false
    }

    /// Whether running a node with this (resolved) `input` again after it was cut short (PDT stopped while it ran, or
    /// it timed out) is safe: it does nothing twice, because what it does can be repeated or it checks what it did
    /// (`StepCx::rerun`, its saved progress). An unsafe one is not run again on its own (no restart re-run, no retry
    /// after a timeout): its step fails, and Retry on the run is the user's choice to run it once more (PDT-131).
    fn rerun_safe(&self, _input: &Value) -> bool {
        true
    }
}

/// Whether an event is in a workflow's scope: a project workflow's own project, any for a host one, none for a
/// personal one.
pub fn in_scope(wf: &Workflow, event: &LogEvent) -> bool {
    match wf.scope {
        Scope::Project => event.project_id.is_some() && event.project_id == wf.project_id,
        Scope::Host => true,
        Scope::Personal => event.project_id.is_none(),
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
        preset: None,
    }
}

pub struct Catalog {
    kinds: Vec<(KindSpec, std::sync::Arc<dyn NodeKind>)>,
    /// The ids of the integrations this build has, enabled or not.
    built: Vec<&'static str>,
}

impl Catalog {
    pub(crate) fn new(kinds: Vec<Box<dyn NodeKind>>, built: Vec<&'static str>) -> Self {
        Self { kinds: kinds.into_iter().map(|k| (k.spec(), std::sync::Arc::from(k))).collect(), built }
    }

    /// Why there is no kind `kind`: its integration is turned off, or this build lacks it, or there is no such kind.
    pub fn missing(&self, kind: &str) -> (&'static str, String) {
        match pdt_core::integration::owner_of_kind(kind) {
            Some(i) if self.built.contains(&i.id) => (
                "integration_disabled",
                format!("{kind} is part of the {} integration, which is turned off (Settings → Integrations)", i.label),
            ),
            Some(i) => (
                "integration_missing",
                format!("{kind} is part of the {} integration, which this {SHORT_NAME} was built without", i.label),
            ),
            None => ("unknown_kind", format!("no node kind is called {kind}")),
        }
    }

    pub fn get(&self, id: &str) -> Option<&dyn NodeKind> {
        self.kinds.iter().find(|(s, _)| s.id == id).map(|(_, k)| k.as_ref())
    }

    /// The kind, to keep beyond the catalog (a running step's).
    pub fn shared(&self, id: &str) -> Option<std::sync::Arc<dyn NodeKind>> {
        self.kinds.iter().find(|(s, _)| s.id == id).map(|(_, k)| k.clone())
    }

    pub fn spec(&self, id: &str) -> Option<&KindSpec> {
        self.kinds.iter().find(|(s, _)| s.id == id).map(|(s, _)| s)
    }

    pub fn specs(&self) -> Vec<KindSpec> {
        self.kinds.iter().map(|(s, _)| s.clone()).collect()
    }

    /// The ports a node has with its config (the kind's titles where it names them; `error` left out). None for an
    /// unknown kind.
    pub fn node_ports(&self, node: &Node) -> Option<Vec<PortSpec>> {
        let facts = self.facts(node)?;
        let spec = self.spec(&node.kind)?;
        Some(
            facts
                .ports
                .into_iter()
                .map(|id| {
                    let title = spec.ports.iter().find(|p| p.id == id).map_or_else(|| id.clone(), |p| p.title.clone());
                    PortSpec { id, title }
                })
                .collect(),
        )
    }

    /// A node's config at the kind's current version.
    pub fn current_config(&self, node: &Node) -> Result<Value, String> {
        let (kind, spec) = self.get(&node.kind).zip(self.spec(&node.kind)).ok_or_else(|| self.missing(&node.kind).1)?;
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
    fn unknown(&self, kind: &str) -> (String, String) {
        let (code, message) = self.missing(kind);
        (code.into(), message)
    }

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

/// The catalog as the API lists it: every kind, then an entry per cached MCP tool (`mcp:<server>/<tool>`).
pub fn listed(st: &AppState) -> Vec<KindSpec> {
    let reg = st.registry();
    let mut all = reg.catalog().specs();
    if let Some(base) = reg.catalog().spec(super::nodes::mcp_tool::KIND) {
        all.extend(super::nodes::mcp_tool::tool_entries(base, &st.mcp.all_cached()));
    }
    all
}

/// A kind or MCP tool entry by id, as [`listed`] has it.
pub fn listed_spec(st: &AppState, id: &str) -> Option<KindSpec> {
    st.registry().catalog().spec(id).cloned().or_else(|| listed(st).into_iter().find(|k| k.id == id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> &'static Catalog {
        crate::integrations::test_registry().catalog()
    }

    #[test]
    fn every_kind_has_a_unique_id_and_schemas() {
        let mut seen = std::collections::HashSet::new();
        for s in catalog().specs() {
            assert!(seen.insert(s.id.clone()), "{} twice", s.id);
            assert!(s.version >= 1);
            assert!(s.config_schema.is_object(), "{}", s.id);
            assert!(!s.title.is_empty() && !s.description.is_empty(), "{}", s.id);
        }
        for t in crate::integrations::test_registry().events() {
            let kinds: Vec<_> = catalog()
                .specs()
                .into_iter()
                .filter(|s| catalog().get(&s.id).unwrap().event_type() == Some(t.id))
                .collect();
            assert_eq!(kinds.len(), 1, "{} has one trigger", t.id);
            assert_eq!(kinds[0].category, Category::Trigger);
        }
    }

    #[test]
    fn connection_fields_say_their_kind() {
        let prop = |kind: &str, field: &str| catalog().spec(kind).unwrap().config_schema["properties"][field].clone();
        assert_eq!(prop("webhook", "hook")["x-connection-kind"], "webhook");
        assert_eq!(prop("mcp.tool", "server")["x-connection-kind"], "mcp");
        assert_eq!(
            prop("http.request", "connection")["x-connection-kind"],
            "any",
            "{}",
            prop("http.request", "connection")
        );
    }

    #[test]
    fn node_ports_follow_the_config() {
        let node: Node = serde_json::from_value(serde_json::json!({"id": "g", "kind": "gate", "version": 1,
            "config": {"question": "q", "choices": ["yes", "no"]}}))
        .unwrap();
        let ports = catalog().node_ports(&node).unwrap();
        let ids: Vec<_> = ports.iter().map(|p| (p.id.as_str(), p.title.as_str())).collect();
        assert_eq!(ids, [("yes", "yes"), ("no", "no"), ("unanswered", "Not asked"), ("timeout", "No answer in time")]);
        let unknown: Node =
            serde_json::from_value(serde_json::json!({"id": "x", "kind": "nope", "version": 1})).unwrap();
        assert!(catalog().node_ports(&unknown).is_none());
    }
}
