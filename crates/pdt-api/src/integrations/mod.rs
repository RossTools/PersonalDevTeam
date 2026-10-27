//! Integrations (PDT-171): each one declares, in one [`Integration`], what it adds to PDT: connection kinds, workflow
//! node kinds, event types (each a trigger), pdt-mcp tools and actions (each both a node kind and a tool, PDT-172), and
//! the rest of what it hooks into (its part of the poll, routes, built-in workflows and templates, CI, the PM's
//! instructions). The [`Registry`] gathers every enabled integration's into the lists the rest of PDT reads: the node
//! catalog, `tools/list` and `tools/call`, the personas' tools, the event types, `GET /api/connections/kinds` and the
//! project settings form (`GET /api/integrations/project-settings`). PDT's own pieces register the same way
//! ([`core::Core`]) and are always on.
//!
//! The optional integrations live in crates of their own (`crates/pdt-int-<id>`, PDT-173), each behind a pdtd cargo
//! feature; pdtd hands the ones it was built with to [`Integrations::new`], and an instance enables those it uses
//! (Settings → Integrations, [`Integrations::enable`]). A disabled one registers nothing.

pub mod action;
mod bug_reports;
mod core;
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
use pdt_core::integration::{ConnectionKind, IntegrationSettings, ProjectSettingsSchema, ToolDecl, ToolTable};
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

/// What an integration adds to PDT. Everything but its id and label is optional: an integration declares only what it
/// has.
pub trait Integration: Send + Sync {
    /// A short id (`chat`), unique among integrations: its entry in [`pdt_core::integration::KNOWN`].
    fn id(&self) -> &'static str;

    /// Its name on the Settings → Integrations page.
    fn label(&self) -> &'static str {
        pdt_core::integration::known(self.id()).map_or(self.id(), |k| k.label)
    }

    /// What it adds, in a line, on the Settings → Integrations page.
    fn description(&self) -> &'static str {
        ""
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

    /// What a call of one of its asking tools acts on, so the PM's policy can be found: by default its action's
    /// ([`action::Action::ask_target`]).
    fn ask_target(&self, tool: &str, input: &Value) -> Option<AskTarget> {
        self.actions().into_iter().find(|a| a.decl().name == tool).and_then(|a| a.ask_target(input))
    }

    /// The title of the user's approval of a call of one of its tools: by default its action's
    /// ([`action::Action::approval_title`]).
    fn ask_title(&self, tool: &str, input: &Value) -> Option<String> {
        self.actions().into_iter().find(|a| a.decl().name == tool).and_then(|a| a.title(input))
    }

    /// Takes credentials an agent wrote into a call of one of its tools out of the call as PDT stores and shows it
    /// (the call itself still gets them). Returns the values taken out, so the raw line can lose them too.
    fn redact_tool_input(&self, _tool: &str, _input: &mut Value) -> Vec<String> {
        vec![]
    }

    /// What the PM is told about its tools, as a line of the PM's instructions (`- Chat: …`).
    fn pm_instructions(&self) -> Option<&'static str> {
        None
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

    /// The CI provider it is for a project with these settings (its label), if it is one: an automated merge waits
    /// for its report on the pull request's head ([`crate::ci`]).
    fn project_ci(&self, _settings: &pdt_store::ProjectSettings) -> Option<&'static str> {
        None
    }

    /// Its members of the 60 s poll, run after PDT's own checks.
    fn pollers(&self) -> Vec<Poller> {
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
}

/// PDT's own integrations, always on, in the catalog's order.
fn own() -> Vec<Arc<dyn Integration>> {
    vec![
        Arc::new(core::Core),
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
    /// Its integration's [`Integration::call_tool`].
    Integration,
    Action(Arc<dyn AnyAction>),
}

/// The enabled integrations, and what they add, gathered.
pub struct Registry {
    integrations: Vec<Arc<dyn Integration>>,
    /// Every tool as `tools/list` gives it, what runs it and the index of its integration.
    tools: Vec<(ToolInfo, Runner, usize)>,
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
        let mut tools: Vec<(ToolInfo, Runner, usize)> = vec![];
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
                .chain(int.tools().into_iter().map(|t| (t.decl, t.info(), Runner::Integration)));
            for (decl, info, runner) in own {
                if !decls.iter().any(|x| x.name == decl.name) {
                    decls.push(decl);
                    tools.push((info, runner, i));
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
        self.tools.iter().map(|(t, _, _)| t.clone()).collect()
    }

    /// Runs a tool by name. Err is a message for the agent.
    pub async fn call_tool(&self, cx: &Cx<'_>, name: &str, args: Value) -> Result<Value, String> {
        match self.tools.iter().find(|(t, _, _)| t.name == name) {
            Some((_, Runner::Integration, i)) => self.integrations[*i].call_tool(cx, name, args).await,
            Some((_, Runner::Action(a), _)) => a.call(cx, args).await,
            None => Err(format!("no tool is called {name}")),
        }
    }

    /// The integration with the pdt-mcp tool `name` (Claude's `mcp__pdt__<name>`, Codex's `mcp:pdt/<name>` or the
    /// bare name), and the bare name.
    fn tool_owner<'a>(&self, name: &'a str) -> Option<(&dyn Integration, &'a str)> {
        let server = pdt_core::mcp::SERVER_NAME;
        let bare = name
            .strip_prefix(&format!("mcp__{server}__"))
            .or_else(|| name.strip_prefix(&format!("mcp:{server}/")))
            .unwrap_or(name);
        let (_, _, i) = self.tools.iter().find(|(t, _, _)| t.name == bare)?;
        Some((self.integrations[*i].as_ref(), bare))
    }

    /// What a call of an integration's asking tool acts on ([`Integration::ask_target`]).
    pub fn ask_target(&self, tool: &str, input: &Value) -> Option<AskTarget> {
        let (int, bare) = self.tool_owner(tool)?;
        int.ask_target(bare, input)
    }

    /// The title of the user's approval of a call of an integration's tool ([`Integration::ask_title`]).
    pub fn ask_title(&self, tool: &str, input: &Value) -> Option<String> {
        let (int, bare) = self.tool_owner(tool)?;
        int.ask_title(bare, input)
    }

    /// Takes the credentials out of a call of an integration's tool ([`Integration::redact_tool_input`]).
    pub fn redact_tool_input(&self, tool: &str, input: &mut Value) -> Vec<String> {
        match self.tool_owner(tool) {
            Some((int, bare)) => int.redact_tool_input(bare, input),
            None => vec![],
        }
    }

    /// The enabled integrations' lines of the PM's instructions.
    pub fn pm_instructions(&self) -> Vec<&'static str> {
        self.integrations.iter().filter_map(|i| i.pm_instructions()).collect()
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

    /// The CI providers a project with these settings has besides its forge's ([`Integration::project_ci`]).
    pub fn project_ci(&self, settings: &pdt_store::ProjectSettings) -> Vec<&'static str> {
        self.integrations.iter().filter_map(|i| i.project_ci(settings)).collect()
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
    /// The ids of the enabled ones, and their registry with PDT's own.
    current: RwLock<(Vec<&'static str>, Arc<Registry>)>,
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
        Self { built, current: RwLock::new((vec![], registry)) }
    }

    /// The optional integrations this build has.
    pub fn built(&self) -> &[Arc<dyn Integration>] {
        &self.built
    }

    /// The registry of PDT's own integrations and the enabled ones.
    pub fn registry(&self) -> Arc<Registry> {
        self.current.read().unwrap_or_else(|e| e.into_inner()).1.clone()
    }

    /// The ids of the enabled integrations, in the build's order.
    pub fn enabled(&self) -> Vec<&'static str> {
        self.current.read().unwrap_or_else(|e| e.into_inner()).0.clone()
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.current.read().unwrap_or_else(|e| e.into_inner()).0.contains(&id)
    }

    /// Enables exactly the integrations in `ids` this build has (the rest are ignored), and makes their registry.
    pub fn enable(&self, ids: &[String]) {
        let on: Vec<Arc<dyn Integration>> =
            self.built.iter().filter(|i| ids.iter().any(|x| x == i.id())).cloned().collect();
        let enabled: Vec<&'static str> = on.iter().map(|i| i.id()).collect();
        let registry =
            Registry::new(own().into_iter().chain(on).collect(), self.built.iter().map(|i| i.id()).collect());
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = (enabled, Arc::new(registry));
    }
}

impl AppState {
    /// The registry of the integrations this instance has enabled (and PDT's own).
    pub fn registry(&self) -> Arc<Registry> {
        self.integrations.registry()
    }
}

/// Enables the integrations the store says this instance enabled (pdtd's start, and after a change).
pub async fn load_enabled(st: &AppState) -> Result<(), pdt_store::StoreError> {
    let ids = st.store.enabled_integrations().await?;
    st.integrations.enable(&ids);
    Ok(())
}

/// An integration as Settings → Integrations lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IntegrationInfo {
    pub id: String,
    pub label: String,
    pub description: String,
    pub enabled: bool,
    /// This build has it (pdtd's cargo feature); one it lacks can't be enabled.
    pub built: bool,
}

/// What `PUT /api/integrations` takes: the ids of the integrations to enable (the rest are disabled).
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnableIntegrations {
    pub enabled: Vec<String>,
}

fn infos(st: &AppState) -> Vec<IntegrationInfo> {
    let built = st.integrations.built();
    let mut out: Vec<IntegrationInfo> = built
        .iter()
        .map(|i| IntegrationInfo {
            id: i.id().into(),
            label: i.label().into(),
            description: i.description().into(),
            enabled: st.integrations.is_enabled(i.id()),
            built: true,
        })
        .collect();
    for k in pdt_core::integration::KNOWN.iter().filter(|k| !built.iter().any(|i| i.id() == k.id)) {
        out.push(IntegrationInfo {
            id: k.id.into(),
            label: k.label.into(),
            description: String::new(),
            enabled: false,
            built: false,
        });
    }
    out
}

/// `GET /api/integrations`: every integration this build has, enabled or not, then the known ones it lacks.
pub(crate) async fn list(State(st): State<AppState>) -> axum::Json<Vec<IntegrationInfo>> {
    axum::Json(infos(&st))
}

/// `PUT /api/integrations`: enables exactly the given integrations. Turning one on installs its built-in workflows
/// and gives the PM its tools; turning one off leaves its connections, settings and workflows where they are (a
/// workflow using it then fails validation, naming it).
pub(crate) async fn put(
    State(st): State<AppState>,
    payload: Result<axum::Json<EnableIntegrations>, axum::extract::rejection::JsonRejection>,
) -> Result<axum::Json<Vec<IntegrationInfo>>, crate::error::ApiError> {
    let axum::Json(input) = payload?;
    let mut ids: Vec<String> = vec![];
    for id in input.enabled {
        if !st.integrations.built().iter().any(|i| i.id() == id) {
            let what = match pdt_core::integration::known(&id) {
                Some(k) => {
                    format!("this {} was built without the {} integration", pdt_core::brand::SHORT_NAME, k.label)
                }
                None => format!("there is no integration {id}"),
            };
            return Err(crate::error::ApiError::new(axum::http::StatusCode::UNPROCESSABLE_ENTITY, "invalid", what));
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    st.store.set_enabled_integrations(&ids).await?;
    st.integrations.enable(&ids);
    // Built-in workflows and personas follow what is enabled on the engine's next tick.
    st.builtins_installed.store(false, std::sync::atomic::Ordering::SeqCst);
    st.personas_installed.store(false, std::sync::atomic::Ordering::SeqCst);
    tracing::info!(enabled = ?st.integrations.enabled(), "changed the enabled integrations");
    Ok(axum::Json(infos(&st)))
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

/// `GET /api/integrations/project-settings`: what a project sets for each integration, for its settings form.
pub(crate) async fn project_settings(State(st): State<AppState>) -> axum::Json<Vec<ProjectSettingsSchema>> {
    axum::Json(st.registry().project_settings().cloned().collect())
}

#[cfg(test)]
mod tests;
