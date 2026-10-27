//! Integrations (PDT-171): each one declares, in one [`Integration`], what it adds to PDT: connection kinds, workflow
//! node kinds, event types (each a trigger), pdt-mcp tools and actions (each both a node kind and a tool, PDT-172), and
//! how the PM treats its tools (PDT-175: what an asking call acts on, the approval's title, credentials to redact, a
//! prompt fragment). The [`Registry`] gathers every registered integration's into the lists the rest of PDT reads: the
//! node catalog, `tools/list` and `tools/call`, the personas' tools, the event types, `GET /api/connections/kinds`, the
//! project settings form (`GET /api/integrations/project-settings`) and the PM's prompt, approvals and transcripts.
//! PDT's own pieces register the same way ([`core::Core`]).
//!
//! Adding an integration: a module here implementing [`Integration`], and a line in [`all`].

pub(crate) mod action;
mod arr;
mod bug_reports;
mod buildkite;
mod core;
mod discord;
mod forge;
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
use action::AnyAction;
use pdt_core::integration::{ConnectionKind, IntegrationSettings, ProjectSettingsSchema, ToolDecl, ToolTable};
use pdt_core::mcp::ToolInfo;
use pdt_core::pm::AskTarget;
use pdt_core::workflow::events::EventType;
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};

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
        tool_info(&self.decl, self.title, self.description, (self.schema)())
    }
}

/// A tool as `tools/list` gives it, with its annotations from its flags.
pub(crate) fn tool_info(d: &ToolDecl, title: &str, description: &str, input_schema: Value) -> ToolInfo {
    ToolInfo {
        name: d.name.into(),
        title: title.into(),
        description: description.into(),
        input_schema,
        annotations: json!({"title": title, "readOnlyHint": d.read_only, "destructiveHint": d.destructive,
                            "openWorldHint": d.open_world}),
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

    /// Its actions: each one is a pdt-mcp tool (before [`Integration::tools`]) and, when it has a node face, a node
    /// kind (after [`Integration::node_kinds`]).
    fn actions(&self) -> Vec<Arc<dyn AnyAction>> {
        vec![]
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

    /// What a call to one of its asking [`Integration::tools`] acts on (`tool` is the bare name), whose policy says
    /// whether the PM acts without asking. Its actions answer for themselves ([`action::Action::ask_target`]).
    fn ask_target(&self, _tool: &str, _input: &Value) -> Option<AskTarget> {
        None
    }

    /// The title of the user's approval of a call to one of its tools or actions (Claude's approval card, the Inbox,
    /// the transcript): what they say yes to. None keeps the agent's.
    fn approval_title(&self, _tool: &str, _input: &Value) -> Option<String> {
        None
    }

    /// Takes the credentials an agent wrote into a call to one of its tools or actions out of `input` (each becomes
    /// [`pdt_core::secret::REDACTED`]) and gives them back, for PDT to take out of the call's output and its raw
    /// transcript line too. The call itself still gets them.
    fn redact_call(&self, _tool: &str, _input: &mut Value) -> Vec<String> {
        vec![]
    }

    /// What the PM's prompt says about its tools, as Markdown list items. A PM session gets it when it has any of
    /// them.
    fn pm_prompt(&self) -> Option<&'static str> {
        None
    }
}

/// Every integration PDT has, PDT's own first. The order is the catalog's, `tools/list`'s and the Connections page's.
fn all() -> Vec<Box<dyn Integration>> {
    vec![
        Box::new(core::Core),
        Box::new(forge::ForgeTools),
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

/// What runs a tool.
enum Runner {
    /// The integration with this index ([`Integration::call_tool`]).
    Integration(usize),
    Action(Arc<dyn AnyAction>),
}

/// A registered tool.
struct Entry {
    /// As `tools/list` gives it.
    info: ToolInfo,
    runner: Runner,
    /// The index of the integration it is from.
    owner: usize,
}

/// The registered integrations, and what they add, gathered.
pub struct Registry {
    integrations: Vec<Box<dyn Integration>>,
    /// Every tool, in `tools/list` order.
    tools: Vec<Entry>,
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
        let mut tools: Vec<Entry> = vec![];
        let mut decls: Vec<ToolDecl> = vec![];
        let mut events: Vec<&'static EventType> = vec![];
        let mut kinds: Vec<ConnectionKind> = vec![];
        let mut project_settings: Vec<(usize, ProjectSettingsSchema)> = vec![];
        for (i, int) in integrations.iter().enumerate() {
            if let Some(s) = int.project_settings() {
                debug_assert_eq!(s.integration, int.id(), "an integration's project settings are under its id");
                project_settings.push((i, s));
            }
            debug_assert!(integrations[..i].iter().all(|x| x.id() != int.id()), "{} is registered twice", int.id());
            let own = int
                .actions()
                .into_iter()
                .map(|a| (a.decl(), a.info(), Runner::Action(a)))
                .chain(int.tools().into_iter().map(|t| (t.decl, t.info(), Runner::Integration(i))));
            for (decl, info, runner) in own {
                if !decls.iter().any(|x| x.name == decl.name) {
                    decls.push(decl);
                    tools.push(Entry { info, runner, owner: i });
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
        let table = ToolTable::new(decls);
        Self { integrations, tools, table, events, kinds, project_settings }
    }

    /// Every pdt-mcp tool's declaration, in `tools/list` order.
    pub fn tool_table(&self) -> &ToolTable {
        &self.table
    }

    /// Every pdt-mcp tool as `tools/list` gives them.
    pub fn tool_infos(&self) -> Vec<ToolInfo> {
        self.tools.iter().map(|t| t.info.clone()).collect()
    }

    /// A tool by its bare name.
    fn entry(&self, name: &str) -> Option<&Entry> {
        self.tools.iter().find(|t| t.info.name == name)
    }

    /// A tool by the name an agent gives it ([`pdt_core::mcp::pdt_tool`]): its bare name, entry and integration.
    fn agent_tool<'a>(&'a self, tool: &'a str) -> Option<(&'a str, &'a Entry, &'a dyn Integration)> {
        let name = pdt_core::mcp::pdt_tool(tool);
        let e = self.entry(name)?;
        Some((name, e, self.integrations[e.owner].as_ref()))
    }

    /// Runs a tool by name. Err is a message for the agent.
    pub async fn call_tool(&self, cx: &Cx<'_>, name: &str, args: Value) -> Result<Value, String> {
        match self.entry(name).map(|e| &e.runner) {
            Some(Runner::Integration(i)) => self.integrations[*i].call_tool(cx, name, args).await,
            Some(Runner::Action(a)) => a.call(cx, args).await,
            None => Err(format!("no tool is called {name}")),
        }
    }

    /// What a call to an asking tool acts on, from its name as the agent gives it and its arguments; None for a tool
    /// that doesn't ask, or a call that names nothing.
    pub fn ask_target(&self, tool: &str, input: &Value) -> Option<AskTarget> {
        let (name, e, int) = self.agent_tool(tool)?;
        if !self.table.asks(name) {
            return None;
        }
        match &e.runner {
            Runner::Action(a) => Some(a.ask_target(input)),
            Runner::Integration(_) => int.ask_target(name, input),
        }
    }

    /// The title of the user's approval of a tool call, when its integration gives one ([`Integration::approval_title`]).
    pub fn approval_title(&self, tool: &str, input: &Value) -> Option<String> {
        let (name, _, int) = self.agent_tool(tool)?;
        int.approval_title(name, input).map(|t| pdt_core::conversation::claude::title_line(&t))
    }

    /// Takes the credentials out of a tool call's input ([`Integration::redact_call`]); the ones it took.
    pub fn redact_call(&self, tool: &str, input: &mut Value) -> Vec<String> {
        match self.agent_tool(tool) {
            Some((name, _, int)) => int.redact_call(name, input),
            None => vec![],
        }
    }

    /// The prompt fragments ([`Integration::pm_prompt`]) of the integrations a PM session with `tools` has any tool
    /// of, in registry order.
    pub fn pm_prompt(&self, tools: &[String]) -> Vec<&'static str> {
        self.integrations
            .iter()
            .enumerate()
            .filter(|(i, _)| self.tools.iter().any(|e| e.owner == *i && tools.contains(&e.info.name)))
            .filter_map(|(_, int)| int.pm_prompt())
            .collect()
    }

    /// A connection kind's label (`Discord bot`); the id for a kind no integration has.
    pub fn connection_label<'a>(&'a self, kind: &'a str) -> &'a str {
        self.kinds.iter().find(|k| k.id == kind).map_or(kind, |k| k.label.as_str())
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

    /// Every node kind: a plain trigger for each event type no kind claims, then each integration's kinds and its
    /// actions' kinds.
    pub fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        let own: Vec<Box<dyn NodeKind>> = self
            .integrations
            .iter()
            .flat_map(|i| {
                let actions = i.actions().into_iter().filter_map(|a| a.node_kind());
                i.node_kinds().into_iter().chain(actions).collect::<Vec<_>>()
            })
            .collect();
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
