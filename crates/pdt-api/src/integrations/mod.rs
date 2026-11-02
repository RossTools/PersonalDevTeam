//! Integrations (PDT-171): each one declares, in one [`Integration`], what it adds to PDT: connection kinds, workflow
//! node kinds, event types (each a trigger), pdt-mcp tools and actions (each both a node kind and a tool, PDT-172), and
//! how the PM treats its tools (PDT-175: what an asking call acts on, the approval's title, credentials to redact, a
//! prompt fragment), and the rest of what it hooks into (its part of the poll, routes, built-in workflows and
//! templates, CI). The [`Registry`] gathers every enabled integration's into the lists the rest of PDT reads: the node
//! catalog, `tools/list` and `tools/call`, the personas' tools, the event types, `GET /api/connections/kinds`, the
//! project settings form (`GET /api/integrations/project-settings`) and the PM's prompt, approvals and transcripts.
//! PDT's own pieces register the same way ([`core::Core`]) and are always on, but for those that can be turned off
//! ([`Integration::can_turn_off`]: GitHub, while no project uses its forge, PDT-251).
//!
//! The optional integrations live in crates of their own (`crates/pdt-int-<id>`, PDT-173), each behind a pdtd cargo
//! feature; pdtd hands the ones it was built with to [`Integrations::new`], and an instance enables those it uses
//! (Settings → Plugins, [`Integrations::enable`]). A disabled one registers nothing. The UI calls an integration a
//! plugin (PDT-228): those with a [`Integration::category`] are listed in Settings → Plugins, each with a page.

pub mod action;
mod bug_reports;
mod core;
mod forge;
mod github;
mod http;
mod mcp;
mod webhook;

use crate::mcp::tools::Cx;
use crate::routes::AppState;
use crate::workflow::builtins::Builtin;
use crate::workflow::catalog::{BoxFut, Catalog, NodeKind};
use crate::workflow::nodes::trigger::EventTrigger;
use action::AnyAction;
use axum::extract::State;
use pdt_core::integration::{
    ConnectionKind, ConnectionStatus, Icon, IntegrationSettings, PluginCategory, ProjectSettingsSchema, ToolDecl,
    ToolTable,
};
use pdt_core::mcp::ToolInfo;
use pdt_core::pm::AskTarget;
use pdt_core::workflow::events::EventType;
use pdt_core::workflow::template::Template;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, RwLock};
use ts_rs::TS;

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
pub fn tool_info(d: &ToolDecl, title: &str, description: &str, input_schema: Value) -> ToolInfo {
    ToolInfo {
        name: d.name.into(),
        title: title.into(),
        description: description.into(),
        input_schema,
        annotations: json!({"title": title, "readOnlyHint": d.read_only, "destructiveHint": d.destructive,
                            "openWorldHint": d.open_world}),
    }
}

/// A member of the 60 s poll an integration adds: its name in the logs (`the tracker check`) and the pass.
pub struct Poller {
    pub name: &'static str,
    pub run: fn(AppState) -> BoxFut<'static, ()>,
}

/// A task an integration runs for as long as pdtd does (PDT-304: the Discord bot's Gateway connection), started once
/// when pdtd starts whether the integration is enabled or not: it checks [`Integrations::is_enabled`] itself and
/// idles while it is off. Its name is for the logs.
pub struct Service {
    pub name: &'static str,
    pub run: fn(AppState) -> BoxFut<'static, ()>,
}

/// Starts every built integration's [`Integration::services`]. One that panics is logged and started again a minute
/// later, until pdtd stops.
pub fn start_services(st: &AppState) {
    for int in st.integrations.built() {
        for svc in int.services() {
            let st = st.clone();
            tokio::spawn(async move {
                loop {
                    let ended = crate::loops::contain(svc.name, None, (svc.run)(st.clone())).await;
                    if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    if !matches!(ended, crate::loops::Ended::Panicked) {
                        tracing::warn!(service = svc.name, "an integration's service ended; starting it again");
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
            });
        }
    }
}

/// What an integration adds to PDT. Everything but its id and label is optional: an integration declares only what it
/// has.
pub trait Integration: Send + Sync {
    /// A short id (`chat`), unique among integrations: its entry in [`pdt_core::integration::KNOWN`].
    fn id(&self) -> &'static str;

    /// Its name in Settings → Plugins.
    fn label(&self) -> &'static str {
        pdt_core::integration::known(self.id()).map_or(self.id(), |k| k.label)
    }

    /// What it adds, in a line, in Settings → Plugins.
    fn description(&self) -> &'static str {
        pdt_core::integration::known(self.id()).map_or("", |k| k.description)
    }

    /// Its group in Settings → Plugins; None: not a plugin of its own (PDT's core, listed nowhere).
    fn category(&self) -> Option<PluginCategory> {
        pdt_core::integration::known(self.id()).map(|k| k.category)
    }

    /// Its icon in Settings → Plugins.
    fn icon(&self) -> Icon {
        pdt_core::integration::known(self.id()).map_or(Icon { letters: "?", color: "#6b7080" }, |k| k.icon)
    }

    /// The kinds of connection it uses, in the order the Connections page shows them.
    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![]
    }

    /// Checks a connection's config for one of its kinds. Err is a message for the user.
    fn check_connection_config(&self, _kind: &str, _config: &Value) -> Result<(), String> {
        Ok(())
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

    /// Runs one of its tools (`name` is one of [`Integration::tools`]). Err is a message for the agent.
    fn call_tool<'a>(&'a self, _cx: &'a Cx<'a>, name: &'a str, _args: Value) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move { Err(format!("no tool is called {name}")) })
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

    /// Imports issue `key` into project `project_id` as a Draft ticket linked to it, or finds the ticket that already
    /// links it, for its [`ProjectSettingsSchema::import`] (`POST /api/projects/{id}/import/{integration}`).
    fn import_by_key<'a>(
        &'a self,
        _st: &'a AppState,
        _project_id: &'a str,
        _key: &'a str,
    ) -> BoxFut<'a, Result<Imported, crate::error::ApiError>> {
        Box::pin(async { Err(crate::error::ApiError::not_found("import")) })
    }

    /// How a connection of one of its kinds with [`ConnectionKind::status`] is doing, beyond OK or Failing
    /// (`GET /api/connections/{id}/status`). None: nothing to say.
    fn connection_status<'a>(
        &'a self,
        _st: &'a AppState,
        _connection: &'a pdt_core::workflow::api::Connection,
    ) -> BoxFut<'a, Option<ConnectionStatus>> {
        Box::pin(async { None })
    }

    /// The CI provider it is for project `project_id` with these settings, if it is one and the project has it set
    /// up (PDT-242): the PR poller reads its runs with the forge's own CI's, the badge and an automated merge combine
    /// them ([`crate::ci`]).
    fn ci_provider(
        &self,
        _st: &AppState,
        _project_id: &str,
        _settings: &pdt_store::ProjectSettings,
    ) -> Option<Arc<dyn pdt_forge::CiProvider>> {
        None
    }

    /// A connection of one of its kinds is being deleted (PDT-316): forget, and where it can take back, what it keeps
    /// for it apart from the connection's own secret (Google's sign-in). Runs whether the integration is enabled or
    /// not, before the connection goes; problems are only logged, the delete goes on.
    fn connection_deleted<'a>(
        &'a self,
        _st: &'a AppState,
        _connection: &'a pdt_core::workflow::api::Connection,
    ) -> BoxFut<'a, ()> {
        Box::pin(async {})
    }

    /// The document providers it provides (PDT-314: Google's Docs and Sheets), used while it is enabled.
    fn document_providers(&self, _st: &AppState) -> Vec<Arc<dyn crate::documents::DocumentProvider>> {
        Vec::new()
    }

    /// The forge it provides (PDT-241), if it is one: the forge plugin of this kind in [`AppState::forges`] is used
    /// while it is enabled, and Settings → Plugins shows its login's state.
    fn forge_kind(&self) -> Option<pdt_core::forge::ForgeKind> {
        None
    }

    /// Its accounts' sign-in in the browser (PDT-378), if they sign in on the service's consent page: its page in
    /// Settings → Plugins shows it.
    fn sign_in(&self) -> Option<pdt_core::integration::SignIn> {
        None
    }

    /// One of PDT's own that the user can turn off (GitHub, PDT-251). A forge plugin ([`Integration::forge_kind`])
    /// can't be turned off while a project uses its forge.
    fn can_turn_off(&self) -> bool {
        false
    }

    /// Its members of the 60 s poll, run after PDT's own checks.
    fn pollers(&self) -> Vec<Poller> {
        vec![]
    }

    /// What it runs for as long as pdtd does ([`Service`]).
    fn services(&self) -> Vec<Service> {
        vec![]
    }

    /// Its routes, under `/api`; they answer 404 while it is disabled.
    fn routes(&self) -> Option<axum::Router<AppState>> {
        None
    }

    /// Its built-in workflows, installed when it is enabled.
    fn builtin_workflows(&self) -> Vec<Builtin> {
        vec![]
    }

    /// Its workflow templates (ids without `builtin:`).
    fn templates(&self) -> Vec<Template> {
        vec![]
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

/// PDT's own integrations, in the catalog's order: always on, but for those that can be turned off.
fn own() -> Vec<Arc<dyn Integration>> {
    vec![
        Arc::new(core::Core),
        Arc::new(forge::ForgeTools),
        Arc::new(crate::documents::tools::DocumentTools),
        Arc::new(webhook::Webhook),
        Arc::new(github::Github),
        Arc::new(http::Http),
        Arc::new(mcp::Mcp),
        Arc::new(bug_reports::BugReports),
    ]
}

/// The registry of PDT's own integrations alone, for unit tests.
#[cfg(test)]
pub(crate) fn test_registry() -> &'static Registry {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| Registry::new(own(), vec![]))
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

/// The enabled integrations, and what they add, gathered.
pub struct Registry {
    integrations: Vec<Arc<dyn Integration>>,
    /// Every tool, in `tools/list` order.
    tools: Vec<Entry>,
    table: ToolTable,
    events: Vec<&'static EventType>,
    /// Every connection kind, with the index of its integration.
    kinds: Vec<(ConnectionKind, usize)>,
    /// Every project settings schema, with the index of its integration.
    project_settings: Vec<(usize, ProjectSettingsSchema)>,
    catalog: Catalog,
}

impl Registry {
    /// The registry of `integrations`; `built` are the ids of every integration this build has (enabled or not), so
    /// the catalog can tell a node of a turned-off integration from one this build lacks. A tool, event type or
    /// connection kind another integration already has is left out (the first keeps it).
    pub fn new(integrations: Vec<Arc<dyn Integration>>, built: Vec<&'static str>) -> Self {
        let mut tools: Vec<Entry> = vec![];
        let mut decls: Vec<ToolDecl> = vec![];
        let mut events: Vec<&'static EventType> = vec![];
        let mut kinds: Vec<(ConnectionKind, usize)> = vec![];
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
                if !kinds.iter().any(|(x, _)| x.id == k.id) {
                    kinds.push((k, i));
                }
            }
        }
        let table = ToolTable::new(decls);
        let catalog = Catalog::new(Self::kinds_of(&integrations, &events), built);
        Self { integrations, tools, table, events, kinds, project_settings, catalog }
    }

    /// Every node kind: a plain trigger for each event type no kind claims, then each integration's kinds and its
    /// actions' kinds.
    fn kinds_of(integrations: &[Arc<dyn Integration>], events: &[&'static EventType]) -> Vec<Box<dyn NodeKind>> {
        let own: Vec<Box<dyn NodeKind>> = integrations
            .iter()
            .flat_map(|i| {
                let actions = i.actions().into_iter().filter_map(|a| a.node_kind());
                i.node_kinds().into_iter().chain(actions).collect::<Vec<_>>()
            })
            .collect();
        let claimed: Vec<String> = own.iter().filter_map(|k| k.event_type().map(String::from)).collect();
        let mut kinds: Vec<Box<dyn NodeKind>> = events
            .iter()
            .filter(|t| !claimed.iter().any(|c| c == t.id))
            .map(|t| Box::new(EventTrigger(t)) as Box<dyn NodeKind>)
            .collect();
        kinds.extend(own);
        kinds
    }

    /// The registered integrations, PDT's own first.
    pub fn integrations(&self) -> &[Arc<dyn Integration>] {
        &self.integrations
    }

    /// Every node kind of the enabled integrations.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
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

    /// Every event type, in catalog order.
    pub fn events(&self) -> &[&'static EventType] {
        &self.events
    }

    pub fn event_type(&self, id: &str) -> Option<&'static EventType> {
        self.events.iter().find(|t| t.id == id).copied()
    }

    /// Every connection kind, in the Connections page's order.
    pub fn connection_kinds(&self) -> Vec<ConnectionKind> {
        self.kinds.iter().map(|(k, _)| k.clone()).collect()
    }

    /// A connection kind's label (`Chat bot`), or its id when no enabled integration has it.
    pub fn connection_label(&self, kind: &str) -> String {
        self.kinds.iter().find(|(k, _)| k.id == kind).map_or_else(|| kind.to_string(), |(k, _)| k.label.clone())
    }

    /// Checks a connection's config with the integration of its kind. Err is a message for the user.
    pub fn check_connection_config(&self, kind: &str, config: &Value) -> Result<(), String> {
        match self.kinds.iter().find(|(k, _)| k.id == kind) {
            Some((_, i)) => self.integrations[*i].check_connection_config(kind, config),
            None => Ok(()),
        }
    }

    /// The enabled integration a kind of connection is from.
    pub fn owner_of_connection(&self, kind: &str) -> Option<&Arc<dyn Integration>> {
        self.kinds.iter().find(|(k, _)| k.id == kind).map(|(_, i)| &self.integrations[*i])
    }

    /// The enabled integration with project settings `id`, and its schema.
    pub fn project_settings_of(&self, id: &str) -> Option<(&Arc<dyn Integration>, &ProjectSettingsSchema)> {
        self.project_settings.iter().find(|(_, s)| s.integration == id).map(|(i, s)| (&self.integrations[*i], s))
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

    /// The enabled integrations' members of the poll.
    pub fn pollers(&self) -> Vec<Poller> {
        self.integrations.iter().flat_map(|i| i.pollers()).collect()
    }

    /// The enabled integrations' built-in workflows.
    pub fn builtin_workflows(&self) -> Vec<Builtin> {
        self.integrations.iter().flat_map(|i| i.builtin_workflows()).collect()
    }

    /// The enabled integrations' templates.
    pub fn templates(&self) -> Vec<Template> {
        self.integrations.iter().flat_map(|i| i.templates()).collect()
    }
}

/// The integrations this build has, and the registry of the ones this instance enabled (PDT-173).
pub struct Integrations {
    /// The optional integrations this build has (pdtd's features), in their order.
    built: Vec<Arc<dyn Integration>>,
    /// What is on now, and its registry.
    current: RwLock<Current>,
    /// The registry of every integration this build has, enabled or not, made when first asked for.
    everything: std::sync::OnceLock<Arc<Registry>>,
}

/// The integrations an instance has on.
struct Current {
    /// The ids of the enabled optional ones, in the build's order.
    enabled: Vec<&'static str>,
    /// The ids of PDT's own that are turned off ([`Integration::can_turn_off`]).
    off: Vec<&'static str>,
    /// The registry of PDT's own that are on and the enabled ones.
    registry: Arc<Registry>,
}

impl Default for Integrations {
    fn default() -> Self {
        Self::new(vec![])
    }
}

impl Integrations {
    /// `built`, none of them enabled yet.
    pub fn new(built: Vec<Arc<dyn Integration>>) -> Self {
        let registry = Arc::new(Registry::new(own(), built.iter().map(|i| i.id()).collect()));
        Self {
            built,
            current: RwLock::new(Current { enabled: vec![], off: vec![], registry }),
            everything: Default::default(),
        }
    }

    /// The registry of PDT's own integrations and every one this build has, enabled or not: what the New workflow
    /// gallery names a turned-off integration's nodes with (PDT-229). Nothing runs from it.
    pub fn everything(&self) -> Arc<Registry> {
        self.everything
            .get_or_init(|| {
                let ids = self.built.iter().map(|i| i.id()).collect();
                Arc::new(Registry::new(own().into_iter().chain(self.built.iter().cloned()).collect(), ids))
            })
            .clone()
    }

    /// The optional integrations this build has.
    pub fn built(&self) -> &[Arc<dyn Integration>] {
        &self.built
    }

    fn current(&self) -> std::sync::RwLockReadGuard<'_, Current> {
        self.current.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The registry of PDT's own integrations that are on and the enabled ones.
    pub fn registry(&self) -> Arc<Registry> {
        self.current().registry.clone()
    }

    /// The ids of the enabled optional integrations, in the build's order.
    pub fn enabled(&self) -> Vec<&'static str> {
        self.current().enabled.clone()
    }

    /// The ids of PDT's own integrations that are turned off (PDT-251).
    pub fn own_off(&self) -> Vec<&'static str> {
        self.current().off.clone()
    }

    /// Whether integration `id` is on: an enabled optional one, or one of PDT's own not turned off.
    pub fn is_enabled(&self, id: &str) -> bool {
        let c = self.current();
        c.enabled.contains(&id) || (own().iter().any(|i| i.id() == id) && !c.off.contains(&id))
    }

    /// The integrations this build has that are off: the optional ones not enabled and PDT's own turned off.
    pub fn off(&self) -> Vec<Arc<dyn Integration>> {
        let c = self.current();
        let own_off = own().into_iter().filter(|i| c.off.contains(&i.id()));
        own_off.chain(self.built.iter().filter(|i| !c.enabled.contains(&i.id())).cloned()).collect()
    }

    /// Enables exactly the optional integrations in `ids` this build has (the rest are ignored), keeping PDT's own as
    /// they are, and makes their registry.
    pub fn enable(&self, ids: &[String]) {
        let off: Vec<String> = self.own_off().into_iter().map(String::from).collect();
        self.set(ids, &off);
    }

    /// Enables exactly the optional integrations in `ids` this build has and turns off those of PDT's own in `off`
    /// that can be turned off (the rest of both are ignored), and makes their registry.
    pub fn set(&self, ids: &[String], off: &[String]) {
        let on: Vec<Arc<dyn Integration>> =
            self.built.iter().filter(|i| ids.iter().any(|x| x == i.id())).cloned().collect();
        let enabled: Vec<&'static str> = on.iter().map(|i| i.id()).collect();
        let (dropped, own_on): (Vec<_>, Vec<_>) =
            own().into_iter().partition(|i| i.can_turn_off() && off.iter().any(|x| x == i.id()));
        let off: Vec<&'static str> = dropped.iter().map(|i| i.id()).collect();
        let registry =
            Registry::new(own_on.into_iter().chain(on).collect(), self.built.iter().map(|i| i.id()).collect());
        *self.current.write().unwrap_or_else(|e| e.into_inner()) =
            Current { enabled, off, registry: Arc::new(registry) };
    }
}

impl AppState {
    /// The registry of the integrations this instance has enabled (and PDT's own).
    pub fn registry(&self) -> Arc<Registry> {
        self.integrations.registry()
    }
}

/// Enables the integrations the store says this instance enabled, and turns off PDT's own it turned off (pdtd's
/// start, and after a change).
pub async fn load_enabled(st: &AppState) -> Result<(), pdt_store::StoreError> {
    let ids = st.store.enabled_integrations().await?;
    let off = st.store.own_integrations_off().await?;
    st.integrations.set(&ids, &off);
    Ok(())
}

/// A plugin's state in Settings → Plugins (PDT-228).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PluginStatus {
    On,
    Off,
    /// On, but it has no connection of any of its kinds (GitHub: PDT needs the login and has none).
    NeedsSetup,
    /// One of its connections is failing ([`IntegrationInfo::error`] says which and why).
    Error,
    /// This build lacks it.
    NotBuilt,
}

/// A plugin's icon: a letter or two on its colour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginIcon {
    pub letters: String,
    pub color: String,
}

/// Something a plugin adds, by id and name: a workflow step or trigger, or a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginPart {
    pub id: String,
    pub title: String,
}

/// A project linked to a plugin (its project settings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginProject {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// An integration as Settings → Plugins lists it: a card in the catalog, and its page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IntegrationInfo {
    pub id: String,
    pub label: String,
    pub description: String,
    pub category: PluginCategory,
    pub icon: PluginIcon,
    pub enabled: bool,
    /// This build has it (pdtd's cargo feature); one it lacks can't be enabled.
    pub built: bool,
    /// Part of PDT (GitHub, incoming webhooks, HTTP services, MCP servers): always on, no switch, unless
    /// [`IntegrationInfo::can_turn_off`].
    pub core: bool,
    /// One of PDT's own the user can turn off (GitHub, PDT-251), while no project uses it
    /// ([`IntegrationInfo::forge_projects`]).
    pub can_turn_off: bool,
    /// The projects whose forge it is, while it is on (PDT-251): a forge plugin can't be turned off while any does.
    pub forge_projects: Vec<PluginProject>,
    /// The forge it provides (`github`, `gitlab`), whose logins its page shows (`/api/forges/{kind}/login`).
    pub forge_kind: Option<String>,
    /// How its accounts sign in in the browser ([`Integration::sign_in`]), while this build has it.
    pub sign_in: Option<pdt_core::integration::SignIn>,
    pub status: PluginStatus,
    /// What is failing, for [`PluginStatus::Error`]: the connection's name and its error.
    pub error: Option<String>,
    /// The connection with the error, for Edit connection.
    pub error_connection: Option<String>,
    /// It has checks of its own in the poll, which "Test again" runs now (`POST /api/integrations/{id}/check`).
    pub checkable: bool,
    /// The kinds of connection it adds, enabled or not.
    pub connection_kinds: Vec<ConnectionKind>,
    /// Its workflow steps and triggers.
    pub node_kinds: Vec<PluginPart>,
    /// Its workflow templates.
    pub templates: Vec<PluginPart>,
    /// The built-in workflows it installs (by their id).
    pub workflows: Vec<PluginPart>,
    /// The pdt-mcp tools it adds (its actions' and its own), enabled or not; none when this build lacks it.
    pub tools: Vec<String>,
    /// Those of [`IntegrationInfo::tools`] the PM has ([`ToolDecl::pm`]).
    pub pm_tools: Vec<String>,
    /// What a project sets for it (its project settings' description), if anything.
    pub project_settings: Option<String>,
    /// The projects linked to it.
    pub projects: Vec<PluginProject>,
}

/// A forge plugin's login, for its integration's status.
pub(crate) struct ForgeLogin {
    pub kind: pdt_core::forge::ForgeKind,
    pub label: &'static str,
    /// PDT needs the login and has none ([`pdt_forge::Forge::login_needed`]).
    pub needed: bool,
    /// The forge refuses the login PDT has.
    pub rejected: bool,
}

/// What a plugin's status is computed from.
#[derive(Default)]
pub(crate) struct PluginFacts {
    pub connections: Vec<pdt_core::workflow::api::Connection>,
    /// Each forge plugin's login ([`Integration::forge_kind`]).
    pub forges: Vec<ForgeLogin>,
    /// Each project, with the ids of the integrations it has settings for.
    pub projects: Vec<(PluginProject, Vec<String>)>,
    /// Each project with a forge from a forge plugin in use, with its kind.
    pub forge_projects: Vec<(pdt_core::forge::ForgeKind, PluginProject)>,
}

impl PluginFacts {
    pub(crate) async fn load(st: &AppState) -> Result<Self, pdt_store::StoreError> {
        let mut projects = vec![];
        let mut forge_projects = vec![];
        // Each project's forge costs a run config read and `git remote`: only worth it while a forge plugin is on.
        let forges_on = !st.forge_plugins().is_empty();
        for p in st.store.list_projects().await? {
            let linked = st.store.get_project_settings(&p.id).await?.integrations.into_keys().collect();
            let plugin = PluginProject { id: p.id.clone(), key: p.key, name: p.name };
            if forges_on
                && let Some(f) = crate::forge::project_forge(st, &p.id, &p.repo_path).await
                && st.forge_plugin(&f.kind).is_some()
            {
                forge_projects.push((f.kind, plugin.clone()));
            }
            projects.push((plugin, linked));
        }
        Ok(Self {
            connections: st.store.list_connections().await?,
            forges: st
                .forges
                .plugins()
                .iter()
                .map(|p| {
                    // Any of its accounts (PDT-244).
                    let fs = crate::forge::signed_forges(p.as_ref());
                    ForgeLogin {
                        kind: p.kind(),
                        label: p.label(),
                        needed: fs.iter().any(|(_, f)| f.login_needed()),
                        rejected: fs.iter().any(|(_, f)| f.login_rejected_at().is_some()),
                    }
                })
                .collect(),
            projects,
            forge_projects,
        })
    }
}

/// What `PUT /api/integrations` takes: the ids of the integrations to enable (the rest are disabled).
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnableIntegrations {
    pub enabled: Vec<String>,
    /// The ids of PDT's own to turn off (the rest of those that can be are turned on, PDT-251); left out: as they
    /// are.
    #[serde(default)]
    #[ts(optional)]
    pub off: Option<Vec<String>>,
}

fn icon_of(i: Icon) -> PluginIcon {
    PluginIcon { letters: i.letters.into(), color: i.color.into() }
}

/// A plugin this build has, with its status from `facts`.
fn info(
    i: &dyn Integration,
    category: PluginCategory,
    enabled: bool,
    core: bool,
    facts: &PluginFacts,
) -> IntegrationInfo {
    let decls: Vec<ToolDecl> =
        i.actions().iter().map(|a| a.decl()).chain(i.tools().into_iter().map(|t| t.decl)).collect();
    let kinds = i.connection_kinds();
    let own: Vec<_> = facts.connections.iter().filter(|c| kinds.iter().any(|k| k.id == c.kind)).collect();
    let failing = own.iter().find(|c| c.last_error.is_some());
    let login = i.forge_kind().and_then(|k| facts.forges.iter().find(|f| f.kind == k));
    let (status, error, error_connection) = if !enabled {
        (PluginStatus::Off, None, None)
    } else if let Some(l) = login.filter(|l| l.rejected) {
        (
            PluginStatus::Error,
            Some(format!("{} no longer accepts PDT's login: the token was revoked or expired.", l.label)),
            None,
        )
    } else if let Some(c) = failing {
        let e = c.last_error.clone().unwrap_or_default();
        (PluginStatus::Error, Some(format!("{}: {e}", c.name)), Some(c.id.clone()))
    } else if login.is_some_and(|l| l.needed) || (!core && !kinds.is_empty() && own.is_empty()) {
        (PluginStatus::NeedsSetup, None, None)
    } else {
        (PluginStatus::On, None, None)
    };
    let mut steps: Vec<PluginPart> = vec![];
    let nodes = i.node_kinds().into_iter().chain(i.actions().into_iter().filter_map(|a| a.node_kind()));
    let mut claimed: Vec<String> = vec![];
    for k in nodes {
        let spec = k.spec();
        claimed.extend(k.event_type().map(String::from));
        steps.push(PluginPart { id: spec.id, title: spec.title });
    }
    let triggers = i
        .event_types()
        .iter()
        .filter(|t| !claimed.iter().any(|c| c == t.id))
        .map(|t| PluginPart { id: t.id.into(), title: t.title.into() });
    let schema = i.project_settings();
    let forge_projects = match i.forge_kind() {
        Some(k) if enabled => facts.forge_projects.iter().filter(|(x, _)| *x == k).map(|(_, p)| p.clone()).collect(),
        _ => vec![],
    };
    IntegrationInfo {
        id: i.id().into(),
        label: i.label().into(),
        description: i.description().into(),
        category,
        icon: icon_of(i.icon()),
        enabled,
        built: true,
        core,
        can_turn_off: core && i.can_turn_off(),
        forge_projects,
        forge_kind: i.forge_kind().map(|k| k.to_string()),
        sign_in: i.sign_in(),
        status,
        error,
        error_connection,
        checkable: !i.pollers().is_empty(),
        connection_kinds: kinds,
        node_kinds: triggers.chain(steps).collect(),
        templates: i.templates().into_iter().map(|t| PluginPart { id: t.id, title: t.name }).collect(),
        workflows: i
            .builtin_workflows()
            .into_iter()
            .map(|w| PluginPart { id: w.id.into(), title: (w.doc)().name })
            .collect(),
        tools: decls.iter().map(|d| d.name.to_string()).collect(),
        pm_tools: decls.iter().filter(|d| d.pm).map(|d| d.name.to_string()).collect(),
        projects: match &schema {
            Some(_) => facts
                .projects
                .iter()
                .filter(|(_, ids)| ids.iter().any(|x| x == i.id()))
                .map(|(p, _)| p.clone())
                .collect(),
            None => vec![],
        },
        project_settings: schema.map(|s| s.description),
    }
}

/// Every plugin: PDT's own (on unless turned off), the ones this build has, enabled or not, then the known ones it lacks.
fn infos(ints: &Integrations, facts: &PluginFacts) -> Vec<IntegrationInfo> {
    let built = ints.built();
    let own = own();
    let mut out: Vec<IntegrationInfo> = own
        .iter()
        .filter_map(|i| Some(info(i.as_ref(), i.category()?, ints.is_enabled(i.id()), true, facts)))
        .chain(
            built.iter().filter_map(|i| Some(info(i.as_ref(), i.category()?, ints.is_enabled(i.id()), false, facts))),
        )
        .collect();
    for k in pdt_core::integration::KNOWN.iter().filter(|k| !built.iter().any(|i| i.id() == k.id)) {
        out.push(IntegrationInfo {
            id: k.id.into(),
            label: k.label.into(),
            description: k.description.into(),
            category: k.category,
            icon: icon_of(k.icon),
            enabled: false,
            built: false,
            core: false,
            can_turn_off: false,
            forge_projects: vec![],
            forge_kind: None,
            sign_in: None,
            status: PluginStatus::NotBuilt,
            error: None,
            error_connection: None,
            checkable: false,
            connection_kinds: vec![],
            node_kinds: vec![],
            templates: vec![],
            workflows: vec![],
            tools: vec![],
            pm_tools: vec![],
            project_settings: None,
            projects: vec![],
        });
    }
    out
}

/// Every plugin as a fresh instance lists it (no connections, no projects): the web tests' fixture.
pub fn plugin_infos(ints: &Integrations) -> Vec<IntegrationInfo> {
    infos(ints, &PluginFacts::default())
}

/// `GET /api/integrations`: every plugin, PDT's own first, then those this build has, enabled or not, then the known
/// ones it lacks.
pub(crate) async fn list(
    State(st): State<AppState>,
) -> Result<axum::Json<Vec<IntegrationInfo>>, crate::error::ApiError> {
    Ok(axum::Json(infos(&st.integrations, &PluginFacts::load(&st).await?)))
}

/// `PUT /api/integrations`: enables exactly the given integrations. Turning one on installs its built-in workflows
/// and gives the PM its tools; turning one off leaves its connections, settings and workflows where they are (a
/// workflow using it then fails validation, naming it).
pub(crate) async fn put(
    State(st): State<AppState>,
    payload: Result<axum::Json<EnableIntegrations>, axum::extract::rejection::JsonRejection>,
) -> Result<axum::Json<Vec<IntegrationInfo>>, crate::error::ApiError> {
    let axum::Json(input) = payload?;
    let invalid = |m: String| crate::error::ApiError::new(axum::http::StatusCode::UNPROCESSABLE_ENTITY, "invalid", m);
    let mut ids: Vec<String> = vec![];
    for id in input.enabled {
        // PDT's own are on unless `off` names them.
        if own().iter().any(|i| i.id() == id) {
            continue;
        }
        if !st.integrations.built().iter().any(|i| i.id() == id) {
            let what = match pdt_core::integration::known(&id) {
                Some(k) => {
                    format!("this {} was built without the {} integration", pdt_core::brand::SHORT_NAME, k.label)
                }
                None => format!("there is no integration {id}"),
            };
            return Err(invalid(what));
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    // An optional forge plugin (GitLab) turned off: not while a project's forge is its, as for GitHub (PDT-251).
    for i in st.integrations.built() {
        if st.integrations.is_enabled(i.id()) && !ids.iter().any(|id| id == i.id()) {
            turn_off_check(&st, i.as_ref()).await?;
        }
    }
    let off: Vec<String> = match input.off {
        None => st.integrations.own_off().into_iter().map(String::from).collect(),
        Some(off) => {
            let mut out: Vec<String> = vec![];
            for id in off {
                match own().into_iter().find(|i| i.id() == id) {
                    Some(i) if i.can_turn_off() => {
                        if st.integrations.is_enabled(i.id()) {
                            turn_off_check(&st, i.as_ref()).await?;
                        }
                        if !out.contains(&id) {
                            out.push(id);
                        }
                    }
                    Some(i) => return Err(invalid(format!("{} is part of PDT: it can't be turned off", i.label()))),
                    None => return Err(invalid(format!("{id} is not one of PDT's own plugins"))),
                }
            }
            out
        }
    };
    st.store.set_enabled_integrations(&ids).await?;
    st.store.set_own_integrations_off(&off).await?;
    st.integrations.set(&ids, &off);
    // Built-in workflows and personas follow what is enabled on the engine's next tick.
    st.builtins_installed.store(false, std::sync::atomic::Ordering::SeqCst);
    st.personas_installed.store(false, std::sync::atomic::Ordering::SeqCst);
    tracing::info!(enabled = ?st.integrations.enabled(), off = ?st.integrations.own_off(), "changed the enabled integrations");
    Ok(axum::Json(infos(&st.integrations, &PluginFacts::load(&st).await?)))
}

/// Refuses to turn off a forge plugin while a project's forge is its (PDT-251): the project's `forge` in
/// `.pdt/project.toml`, else its origin.
async fn turn_off_check(st: &AppState, i: &dyn Integration) -> Result<(), crate::error::ApiError> {
    let Some(kind) = i.forge_kind() else { return Ok(()) };
    let facts = PluginFacts::load(st).await?;
    let using: Vec<String> =
        facts.forge_projects.iter().filter(|(k, _)| *k == kind).map(|(_, p)| p.name.clone()).collect();
    if using.is_empty() {
        return Ok(());
    }
    let label = i.label();
    Err(crate::error::ApiError::new(
        axum::http::StatusCode::CONFLICT,
        "in_use",
        format!(
            "{label} is the forge of {}: give {} another forge (`forge` in .pdt/project.toml: \"local\" or \"none\") \
             before turning {label} off",
            using.join(", "),
            if using.len() == 1 { "it" } else { "them" },
        ),
    ))
}

/// `POST /api/integrations/{id}/check`: runs an enabled integration's checks in the poll now ("Test again"), which
/// record how its connections' uses went; then lists the plugins.
pub(crate) async fn check(
    State(st): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<axum::Json<Vec<IntegrationInfo>>, crate::error::ApiError> {
    let registry = st.registry();
    let Some(int) = registry.integrations().iter().find(|i| i.id() == id) else {
        return Err(crate::error::ApiError::not_found("integration"));
    };
    for p in int.pollers() {
        crate::loops::part(&st, p.name, p.run).await;
    }
    Ok(axum::Json(infos(&st.integrations, &PluginFacts::load(&st).await?)))
}

/// Each integration's routes this build has, answering 404 while it is disabled.
pub(crate) fn routes(st: &AppState) -> axum::Router<AppState> {
    let mut out = axum::Router::new();
    for int in st.integrations.built() {
        let Some(r) = int.routes() else { continue };
        let id = int.id();
        let guard = move |State(st): State<AppState>, req: axum::extract::Request, next: axum::middleware::Next| async move {
            if st.integrations.is_enabled(id) {
                next.run(req).await
            } else {
                use axum::response::IntoResponse;
                crate::error::ApiError::not_found("route").into_response()
            }
        };
        out = out.merge(r.route_layer(axum::middleware::from_fn_with_state(st.clone(), guard)));
    }
    out
}

/// `GET /api/connections/kinds`: every kind of connection, for the Connections page.
pub(crate) async fn connection_kinds(State(st): State<AppState>) -> axum::Json<Vec<ConnectionKind>> {
    axum::Json(st.registry().connection_kinds())
}

/// An issue an integration imported ([`Integration::import_by_key`]).
pub struct Imported {
    pub ticket_id: String,
    /// A ticket already linked the issue (nothing new was made).
    pub existed: bool,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct TicketImportInput {
    /// The issue's key (`ABC-123`).
    pub key: String,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketImportResult {
    pub ticket: pdt_core::ticket::Ticket,
    /// A ticket already linked the issue (nothing new was made).
    pub existed: bool,
}

/// `POST /api/projects/{id}/import/{integration} {key}`: one of an enabled integration's issues as a ticket
/// ([`ProjectSettingsSchema::import`]); 201 when it made one, 200 when a ticket already links it.
pub(crate) async fn import_by_key(
    State(st): State<AppState>,
    axum::extract::Path((project_id, integration)): axum::extract::Path<(String, String)>,
    payload: Result<axum::Json<TicketImportInput>, axum::extract::rejection::JsonRejection>,
) -> Result<(axum::http::StatusCode, axum::Json<TicketImportResult>), crate::error::ApiError> {
    use crate::error::ApiError;
    let axum::Json(input) = payload?;
    st.store.get_project(&project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let registry = st.registry();
    let Some((int, _)) = registry.project_settings_of(&integration).filter(|(_, s)| s.import.is_some()) else {
        return Err(ApiError::not_found("import"));
    };
    let r = int.import_by_key(&st, &project_id, input.key.trim()).await?;
    let ticket = st.store.get_ticket(&r.ticket_id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
    let ticket = crate::overlap::decorate(&st, ticket).await?;
    let status = if r.existed { axum::http::StatusCode::OK } else { axum::http::StatusCode::CREATED };
    Ok((status, axum::Json(TicketImportResult { ticket, existed: r.existed })))
}

/// `GET /api/connections/{id}/status`: how a connection is doing, from its kind's integration; null when it has
/// nothing to say (or its integration is off).
pub(crate) async fn connection_status(
    State(st): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<axum::Json<Option<ConnectionStatus>>, crate::error::ApiError> {
    let c = st.store.get_connection(&id).await?.ok_or_else(|| crate::error::ApiError::not_found("connection"))?;
    let registry = st.registry();
    let status = match registry.owner_of_connection(&c.kind) {
        Some(int) => int.connection_status(&st, &c).await,
        None => None,
    };
    Ok(axum::Json(status))
}

/// `GET /api/integrations/project-settings`: what a project sets for each integration, for its settings form.
pub(crate) async fn project_settings(State(st): State<AppState>) -> axum::Json<Vec<ProjectSettingsSchema>> {
    axum::Json(st.registry().project_settings().cloned().collect())
}

#[cfg(test)]
mod tests;
