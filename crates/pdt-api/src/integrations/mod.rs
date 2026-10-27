//! Integrations (PDT-171): each one declares, in one [`Integration`], what it adds to PDT: connection kinds, workflow
//! node kinds, event types (each a trigger) and pdt-mcp tools. The [`Registry`] gathers every registered integration's
//! into the lists the rest of PDT reads: the node catalog, `tools/list` and `tools/call`, the personas' tools, the
//! event types, `GET /api/connections/kinds` and the project settings form (`GET /api/integrations/project-settings`).
//! PDT's own pieces register the same way ([`core::Core`]).
//!
//! Adding an integration: a module here implementing [`Integration`], and a line in [`all`].

mod arr;
mod bug_reports;
mod buildkite;
mod core;
mod discord;
mod github;
mod http;
mod jira;
mod mcp;
mod plex;
mod scryer;
mod webhook;

use crate::mcp::tools::Cx;
use crate::workflow::catalog::{BoxFut, NodeKind};
use crate::workflow::nodes::trigger::EventTrigger;
use pdt_core::integration::{ConnectionKind, IntegrationSettings, ProjectSettingsSchema, ToolDecl, ToolTable};
use pdt_core::mcp::ToolInfo;
use pdt_core::workflow::events::EventType;
use serde_json::{Value, json};
use std::sync::OnceLock;

/// One pdt-mcp tool: what it is ([`ToolDecl`]) and what an agent sees of it.
pub struct Tool {
    pub decl: ToolDecl,
    pub title: &'static str,
    pub description: &'static str,
    /// Its input's JSON Schema.
    pub schema: fn() -> Value,
}

impl Tool {
    /// The tool as `tools/list` gives it.
    pub fn info(&self) -> ToolInfo {
        let d = &self.decl;
        ToolInfo {
            name: d.name.into(),
            title: self.title.into(),
            description: self.description.into(),
            input_schema: (self.schema)(),
            annotations: json!({"title": self.title, "readOnlyHint": d.read_only, "destructiveHint": d.destructive,
                                "openWorldHint": d.open_world}),
        }
    }
}

/// What an integration adds to PDT. Everything is optional: an integration declares only what it has.
pub trait Integration: Send + Sync {
    /// A short id (`discord`), unique among integrations.
    fn id(&self) -> &'static str;

    /// The kinds of connection it uses, in the order the Connections page shows them.
    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![]
    }

    /// The event types it logs. Each is a trigger in the catalog: the plain event trigger, unless one of its node
    /// kinds claims the type ([`NodeKind::event_type`]).
    fn event_types(&self) -> &'static [EventType] {
        &[]
    }

    /// Its workflow node kinds, in catalog order.
    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![]
    }

    /// Its pdt-mcp tools, in `tools/list` order. A tool is the PM's only with [`ToolDecl::pm`], and in no other
    /// persona's set unless the user picks it.
    fn tools(&self) -> Vec<Tool> {
        vec![]
    }

    /// What a project sets for it on this host (its connection and fields), if anything. The schema's `integration`
    /// is the integration's id.
    fn project_settings(&self) -> Option<ProjectSettingsSchema> {
        None
    }

    /// Checks a project's settings for it, once [`ProjectSettingsSchema::apply`] has tidied them. Err is a message for
    /// the user.
    fn check_project_settings(&self, _settings: &IntegrationSettings) -> Result<(), String> {
        Ok(())
    }

    /// Runs one of its tools (`name` is one of [`Integration::tools`]). Err is a message for the agent.
    fn call_tool<'a>(&'a self, _cx: &'a Cx<'a>, name: &'a str, _args: Value) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move { Err(format!("no tool is called {name}")) })
    }
}

/// Every integration PDT has, PDT's own first. The order is the catalog's, `tools/list`'s and the Connections page's.
fn all() -> Vec<Box<dyn Integration>> {
    vec![
        Box::new(core::Core),
        Box::new(discord::Discord),
        Box::new(webhook::Webhook),
        Box::new(github::Github),
        Box::new(jira::Jira),
        Box::new(buildkite::Buildkite),
        Box::new(http::Http),
        Box::new(arr::Arr),
        Box::new(scryer::Scryer),
        Box::new(plex::Plex),
        Box::new(mcp::Mcp),
        Box::new(bug_reports::BugReports),
    ]
}

/// The registered integrations, and what they add, gathered.
pub struct Registry {
    integrations: Vec<Box<dyn Integration>>,
    /// Every tool, with the index of its integration.
    tools: Vec<(usize, Tool)>,
    table: ToolTable,
    events: Vec<&'static EventType>,
    kinds: Vec<ConnectionKind>,
    /// Every project settings schema, with the index of its integration.
    project_settings: Vec<(usize, ProjectSettingsSchema)>,
}

impl Registry {
    /// The registry of `integrations`. A tool, event type or connection kind another integration already has is left
    /// out (the first keeps it).
    pub fn new(integrations: Vec<Box<dyn Integration>>) -> Self {
        let mut tools: Vec<(usize, Tool)> = vec![];
        let mut events: Vec<&'static EventType> = vec![];
        let mut kinds: Vec<ConnectionKind> = vec![];
        let mut project_settings: Vec<(usize, ProjectSettingsSchema)> = vec![];
        for (i, int) in integrations.iter().enumerate() {
            if let Some(s) = int.project_settings() {
                debug_assert_eq!(s.integration, int.id(), "an integration's project settings are under its id");
                project_settings.push((i, s));
            }
            debug_assert!(integrations[..i].iter().all(|x| x.id() != int.id()), "{} is registered twice", int.id());
            for t in int.tools() {
                if !tools.iter().any(|(_, x)| x.decl.name == t.decl.name) {
                    tools.push((i, t));
                }
            }
            for e in int.event_types() {
                if !events.iter().any(|x| x.id == e.id) {
                    events.push(e);
                }
            }
            for k in int.connection_kinds() {
                if !kinds.iter().any(|x| x.id == k.id) {
                    kinds.push(k);
                }
            }
        }
        let table = ToolTable::new(tools.iter().map(|(_, t)| t.decl));
        Self { integrations, tools, table, events, kinds, project_settings }
    }

    /// Every pdt-mcp tool's declaration, in `tools/list` order.
    pub fn tool_table(&self) -> &ToolTable {
        &self.table
    }

    /// Every pdt-mcp tool as `tools/list` gives them.
    pub fn tool_infos(&self) -> Vec<ToolInfo> {
        self.tools.iter().map(|(_, t)| t.info()).collect()
    }

    /// Runs a tool by name. Err is a message for the agent.
    pub async fn call_tool(&self, cx: &Cx<'_>, name: &str, args: Value) -> Result<Value, String> {
        match self.tools.iter().find(|(_, t)| t.decl.name == name) {
            Some((i, _)) => self.integrations[*i].call_tool(cx, name, args).await,
            None => Err(format!("no tool is called {name}")),
        }
    }

    /// Every event type, in catalog order.
    pub fn events(&self) -> &[&'static EventType] {
        &self.events
    }

    pub fn event_type(&self, id: &str) -> Option<&'static EventType> {
        self.events.iter().find(|t| t.id == id).copied()
    }

    /// Every connection kind, in the Connections page's order.
    pub fn connection_kinds(&self) -> &[ConnectionKind] {
        &self.kinds
    }

    /// Every integration's project settings, in the settings page's order.
    pub fn project_settings(&self) -> impl Iterator<Item = &ProjectSettingsSchema> {
        self.project_settings.iter().map(|(_, s)| s)
    }

    /// The settings `input` makes over `current` for integration `id` ([`ProjectSettingsSchema::apply`], then the
    /// integration's own check). None: unlinked. Err is a message for the user.
    pub fn apply_project_settings(
        &self,
        id: &str,
        input: &IntegrationSettings,
        current: Option<&IntegrationSettings>,
    ) -> Result<Option<(&ProjectSettingsSchema, IntegrationSettings)>, String> {
        let Some((i, schema)) = self.project_settings.iter().find(|(_, s)| s.integration == id) else {
            return Err(format!("no integration {id} has project settings"));
        };
        let Some(out) = schema.apply(input, current)? else { return Ok(None) };
        self.integrations[*i].check_project_settings(&out)?;
        Ok(Some((schema, out)))
    }

    /// Every node kind: a plain trigger for each event type no kind claims, then each integration's kinds.
    pub fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        let own: Vec<Box<dyn NodeKind>> = self.integrations.iter().flat_map(|i| i.node_kinds()).collect();
        let claimed: Vec<String> = own.iter().filter_map(|k| k.event_type().map(String::from)).collect();
        let mut kinds: Vec<Box<dyn NodeKind>> = self
            .events
            .iter()
            .filter(|t| !claimed.iter().any(|c| c == t.id))
            .map(|t| Box::new(EventTrigger(t)) as Box<dyn NodeKind>)
            .collect();
        kinds.extend(own);
        kinds
    }
}

/// The registry of every integration PDT has.
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry::new(all()))
}

/// Every pdt-mcp tool's name, in `tools/list` order.
pub fn tool_names() -> Vec<&'static str> {
    registry().tool_table().names()
}

/// `GET /api/connections/kinds`: every kind of connection, for the Connections page.
pub(crate) async fn connection_kinds() -> axum::Json<Vec<ConnectionKind>> {
    axum::Json(registry().connection_kinds().to_vec())
}

/// `GET /api/integrations/project-settings`: what a project sets for each integration, for its settings form.
pub(crate) async fn project_settings() -> axum::Json<Vec<ProjectSettingsSchema>> {
    axum::Json(registry().project_settings().cloned().collect())
}

#[cfg(test)]
mod tests;
