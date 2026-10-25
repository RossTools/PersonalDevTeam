//! Moving workflows between PDTs (PDT-155): the export envelope (the same text copied or downloaded), project
//! placeholders, what a document needs on the host it lands on, and reading what the user pastes. Pure: pdt-api builds
//! exports and imports them.

use super::api::{Scope, Source};
use super::doc::WorkflowDoc;
use super::refs::has_ref;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use ts_rs::TS;

/// The envelope's `format`.
pub const FORMAT: &str = "pdt-workflow";
/// The envelope version this PDT writes and reads.
pub const FORMAT_VERSION: u32 = 1;
/// The key of a project placeholder: `{"$project": "<KEY>"}` stands for a project's id.
pub const PROJECT_PLACEHOLDER: &str = "$project";
/// What a copy's name ends with when its name is taken.
pub const COPY_SUFFIX: &str = "(imported)";

/// One or more workflows as exported: current documents only (no runs, state or versions), project ids as
/// placeholders, and what they need on the host they move to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowExport {
    pub format: String,
    pub format_version: u32,
    pub pdt_version: String,
    #[ts(type = "number")]
    pub exported_at: i64,
    pub workflows: Vec<ExportedWorkflow>,
    #[serde(default)]
    pub requires: Requires,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportedWorkflow {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub scope: Scope,
    /// A project workflow's project, by key (also in `requires.projects`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub project: Option<String>,
    pub doc: WorkflowDoc,
}

/// What the exported documents refer to by name: connections (MCP servers and webhook hooks are connections of kind
/// `mcp` and `webhook`), personas, and projects (by key).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Requires {
    pub connections: Vec<NeedConnection>,
    pub personas: Vec<String>,
    pub projects: Vec<NeedProject>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NeedConnection {
    pub name: String,
    /// The connection kind the node's schema asks for (`x-connection-kind`; `any` for an HTTP request's).
    pub kind: String,
    /// An MCP server's tools that `mcp.tool` steps call.
    #[serde(default)]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NeedProject {
    pub key: String,
    #[serde(default)]
    pub name: String,
}

/// A config field that names something on the host: a connection of a kind, or a persona.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefField {
    Connection { field: String, kind: String },
    Persona { field: String },
}

/// A name a node's config refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameRef {
    pub node: String,
    pub node_kind: String,
    pub field: String,
    pub name: String,
    /// The connection kind; None for a persona.
    pub kind: Option<String>,
}

/// The names `doc`'s nodes refer to, given each kind's [`RefField`]s. Empty and templated values (`{{…}}`) are left
/// out: they are only known when the workflow runs.
pub fn name_refs(doc: &WorkflowDoc, fields: &dyn Fn(&str) -> Vec<RefField>) -> Vec<NameRef> {
    let mut out = vec![];
    for n in &doc.nodes {
        for f in fields(&n.kind) {
            let (field, kind) = match f {
                RefField::Connection { field, kind } => (field, Some(kind)),
                RefField::Persona { field } => (field, None),
            };
            let Some(name) = n.config.get(&field).and_then(Value::as_str).map(str::trim) else { continue };
            if name.is_empty() || has_ref(name) {
                continue;
            }
            out.push(NameRef { node: n.id.clone(), node_kind: n.kind.clone(), field, name: name.to_string(), kind });
        }
    }
    out
}

/// Renames what `doc`'s nodes refer to: `connections` and `personas` map a name (as written, any case) to the one to
/// use. Names not in the maps stay.
pub fn rename_refs(
    doc: &mut WorkflowDoc,
    fields: &dyn Fn(&str) -> Vec<RefField>,
    connections: &HashMap<String, String>,
    personas: &HashMap<String, String>,
) {
    let lower = |m: &HashMap<String, String>| -> HashMap<String, String> {
        m.iter().map(|(k, v)| (k.trim().to_lowercase(), v.clone())).collect()
    };
    let (connections, personas) = (lower(connections), lower(personas));
    for n in &mut doc.nodes {
        for f in fields(&n.kind) {
            let (field, map) = match f {
                RefField::Connection { field, .. } => (field, &connections),
                RefField::Persona { field } => (field, &personas),
            };
            let Some(Value::String(s)) = n.config.get_mut(&field) else { continue };
            if let Some(to) = map.get(&s.trim().to_lowercase()) {
                *s = to.clone();
            }
        }
    }
}

/// Replaces every config string that is exactly a project id in `keys` (id → key) with `{"$project": KEY}`; returns
/// the keys used.
pub fn to_placeholders(doc: &mut WorkflowDoc, keys: &HashMap<String, String>) -> BTreeSet<String> {
    fn walk(v: &mut Value, keys: &HashMap<String, String>, used: &mut BTreeSet<String>) {
        match v {
            Value::String(s) => {
                if let Some(key) = keys.get(s.as_str()) {
                    used.insert(key.clone());
                    *v = json!({PROJECT_PLACEHOLDER: key});
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, keys, used)),
            Value::Object(m) => m.values_mut().for_each(|x| walk(x, keys, used)),
            _ => {}
        }
    }
    let mut used = BTreeSet::new();
    for n in &mut doc.nodes {
        walk(&mut n.config, keys, &mut used);
    }
    used
}

fn placeholder_key(m: &Map<String, Value>) -> Option<&str> {
    match (m.len(), m.get(PROJECT_PLACEHOLDER)) {
        (1, Some(Value::String(k))) => Some(k),
        _ => None,
    }
}

/// The project keys `doc`'s placeholders name.
pub fn placeholder_keys(doc: &WorkflowDoc) -> BTreeSet<String> {
    fn walk(v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(m) => match placeholder_key(m) {
                Some(k) => {
                    out.insert(k.to_string());
                }
                None => m.values().for_each(|x| walk(x, out)),
            },
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    for n in &doc.nodes {
        walk(&n.config, &mut out);
    }
    out
}

/// Replaces each placeholder with `resolve(key)`: the local project's id, or what an unmapped one becomes.
pub fn fill_placeholders(doc: &mut WorkflowDoc, resolve: &dyn Fn(&str) -> String) {
    fn walk(v: &mut Value, resolve: &dyn Fn(&str) -> String) {
        let to = match v {
            Value::Object(m) => placeholder_key(m).map(resolve),
            _ => None,
        };
        if let Some(to) = to {
            *v = Value::String(to);
            return;
        }
        match v {
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, resolve)),
            Value::Object(m) => m.values_mut().for_each(|x| walk(x, resolve)),
            _ => {}
        }
    }
    for n in &mut doc.nodes {
        walk(&mut n.config, resolve);
    }
}

/// What the user pasted (or the file they chose): an export, or a bare document (the editor's text view), which is one
/// host workflow with no `requires`. Errors say what is wrong in a line.
pub fn parse_import(text: &str) -> Result<WorkflowExport, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("paste an export (or a workflow document) first".into());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| format!("this is not valid JSON: {e}"))?;
    let Value::Object(m) = &v else {
        return Err("this is not a workflow export: expected a JSON object".into());
    };
    if m.contains_key("format") || m.contains_key("formatVersion") {
        let format = m.get("format").and_then(Value::as_str).unwrap_or_default();
        if format != FORMAT {
            return Err(format!("this is not a PDT workflow export (its format is {format:?}, not {FORMAT:?})"));
        }
        let version = m.get("formatVersion").and_then(Value::as_u64);
        match version {
            Some(v) if v == u64::from(FORMAT_VERSION) => {}
            Some(v) => {
                return Err(format!(
                    "this export has format version {v}, and this PDT reads version {FORMAT_VERSION}: update PDT, or \
                     export again from a PDT of this version"
                ));
            }
            None => return Err("this export has no formatVersion".into()),
        }
        let e: WorkflowExport = serde_json::from_value(v).map_err(|e| format!("this export is malformed: {e}"))?;
        if e.workflows.is_empty() {
            return Err("this export has no workflows".into());
        }
        return Ok(e);
    }
    if m.contains_key("nodes") {
        let doc: WorkflowDoc =
            serde_json::from_value(v).map_err(|e| format!("this workflow document is malformed: {e}"))?;
        return Ok(WorkflowExport {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION,
            pdt_version: String::new(),
            exported_at: 0,
            workflows: vec![ExportedWorkflow {
                name: doc.name.clone(),
                description: doc.description.clone(),
                scope: Scope::Host,
                project: None,
                doc,
            }],
            requires: Requires::default(),
        });
    }
    Err("this is not a workflow export or document: it has neither `format` nor `nodes`".into())
}

/// The export's text: pretty-printed, so it pastes cleanly into a chat, a note or a gist.
pub fn export_text(e: &WorkflowExport) -> String {
    serde_json::to_string_pretty(e).expect("an export serializes")
}

// ---- matching and mapping ----

/// How a needed name stands on this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NeedStatus {
    Found,
    Missing,
    /// A connection of that name is here, of another kind.
    KindMismatch,
}

/// A connection on this host, as matching sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalConnection {
    pub name: String,
    pub kind: String,
    pub has_secret: bool,
}

/// A project on this host, as matching sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalProject {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// What is here to match against.
#[derive(Debug, Clone, Default)]
pub struct Local {
    pub connections: Vec<LocalConnection>,
    pub personas: Vec<String>,
    pub projects: Vec<LocalProject>,
}

impl Local {
    pub fn connection(&self, name: &str) -> Option<&LocalConnection> {
        self.connections.iter().find(|c| c.name.eq_ignore_ascii_case(name.trim()))
    }
    pub fn persona(&self, name: &str) -> Option<&String> {
        self.personas.iter().find(|p| p.eq_ignore_ascii_case(name.trim()))
    }
    pub fn project_by_key(&self, key: &str) -> Option<&LocalProject> {
        self.projects.iter().find(|p| p.key.eq_ignore_ascii_case(key.trim()))
    }
    pub fn project(&self, id_or_key: &str) -> Option<&LocalProject> {
        self.projects.iter().find(|p| p.id == id_or_key).or_else(|| self.project_by_key(id_or_key))
    }
}

/// Whether a connection of `have` serves a field that asks for `want`.
pub fn kind_fits(want: &str, have: &str) -> bool {
    want == "any" || want == have
}

pub fn connection_status(need: &NeedConnection, local: &Local) -> NeedStatus {
    match local.connection(&need.name) {
        None => NeedStatus::Missing,
        Some(c) if kind_fits(&need.kind, &c.kind) => NeedStatus::Found,
        Some(_) => NeedStatus::KindMismatch,
    }
}

/// The user's answers for what is not found: each name (a connection's or persona's as the documents have it, a
/// project's key) to a local one, or null to leave it unmapped. A name that is found needs no entry; one given anyway
/// is used.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ImportMapping {
    /// To a local connection's name.
    pub connections: BTreeMap<String, Option<String>>,
    /// To a local persona's name.
    pub personas: BTreeMap<String, Option<String>>,
    /// A project key to a local project's id (or key).
    pub projects: BTreeMap<String, Option<String>>,
}

fn mapped<'a>(m: &'a BTreeMap<String, Option<String>>, name: &str) -> Option<&'a Option<String>> {
    m.iter().find(|(k, _)| k.trim().eq_ignore_ascii_case(name.trim())).map(|(_, v)| v)
}

/// What the import does with each needed name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    /// Renames for [`rename_refs`] (needed name → local name).
    pub connections: HashMap<String, String>,
    pub personas: HashMap<String, String>,
    /// Project key → local id; a key left unmapped stays the key.
    pub projects: HashMap<String, String>,
    /// What is left unmapped, in a line each.
    pub unmapped: Vec<String>,
}

/// Applies the mapping to what the documents need. A name that is neither found nor answered (mapped, or explicitly
/// null) is an error: the import waits for it.
pub fn resolve(requires: &Requires, mapping: &ImportMapping, local: &Local) -> Result<Resolved, String> {
    let mut r = Resolved::default();
    let mut unanswered = vec![];
    for c in &requires.connections {
        match mapped(&mapping.connections, &c.name) {
            Some(Some(to)) => {
                let l = local.connection(to).ok_or_else(|| format!("no connection here is called {to}"))?;
                if !kind_fits(&c.kind, &l.kind) {
                    return Err(format!("{} is a {} connection; {} needs a {} one", l.name, l.kind, c.name, c.kind));
                }
                r.connections.insert(c.name.clone(), l.name.clone());
            }
            Some(None) => r.unmapped.push(format!("the {} connection {} is left unmapped", c.kind, c.name)),
            None if connection_status(c, local) == NeedStatus::Found => {}
            None => unanswered.push(format!("connection {}", c.name)),
        }
    }
    for p in &requires.personas {
        match mapped(&mapping.personas, p) {
            Some(Some(to)) => {
                let l = local.persona(to).ok_or_else(|| format!("no persona here is called {to}"))?;
                r.personas.insert(p.clone(), l.clone());
            }
            Some(None) => r.unmapped.push(format!("the persona {p} is left unmapped")),
            None if local.persona(p).is_some() => {}
            None => unanswered.push(format!("persona {p}")),
        }
    }
    for p in &requires.projects {
        match mapped(&mapping.projects, &p.key) {
            Some(Some(to)) => {
                let l = local.project(to).ok_or_else(|| format!("no project here is {to}"))?;
                r.projects.insert(p.key.clone(), l.id.clone());
            }
            Some(None) => {
                r.unmapped.push(format!("the project {} is left unmapped", p.key));
                r.projects.insert(p.key.clone(), p.key.clone());
            }
            None => match local.project_by_key(&p.key) {
                Some(l) => {
                    r.projects.insert(p.key.clone(), l.id.clone());
                }
                None => unanswered.push(format!("project {}", p.key)),
            },
        }
    }
    if !unanswered.is_empty() {
        return Err(format!("map these to something here, or leave them unmapped: {}", unanswered.join(", ")));
    }
    Ok(r)
}

/// The name for a copy that clashes with `taken`: `<name> (imported)`, then `(imported 2)` …
pub fn copy_name(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let first = format!("{name} {COPY_SUFFIX}");
    if !taken(&first) {
        return first;
    }
    (2..).map(|n| format!("{name} (imported {n})")).find(|c| !taken(c)).expect("a free name")
}

/// Where one imported workflow goes, in the export's order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ImportTarget {
    /// Default: the export's scope (a project workflow needs `projectId`).
    #[ts(optional)]
    pub scope: Option<Scope>,
    #[ts(optional)]
    pub project_id: Option<String>,
    /// Import as a new version of this workflow (never a repo one).
    #[ts(optional)]
    pub version_of: Option<String>,
    /// Import as a copy (`<name> (imported)`) when a workflow has the name already.
    pub copy: bool,
    /// Leave this one out.
    pub skip: bool,
}

/// `POST /api/workflows/import`: `{text, check: true}` reports what the text holds and needs; `{text, mapping, targets}`
/// imports it, disabled.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ImportRequest {
    /// The export (or a bare document) as text.
    pub text: String,
    /// Only report: change nothing.
    pub check: bool,
    pub mapping: ImportMapping,
    /// One per workflow in the text, in order (missing ones: the defaults).
    pub targets: Vec<ImportTarget>,
}

/// A workflow here with the same name as one being imported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExistingWorkflow {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub project_id: Option<String>,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportPreview {
    pub name: String,
    pub description: String,
    pub scope: Scope,
    /// A project workflow's project in the export, by key.
    pub project: Option<String>,
    pub nodes: u32,
    /// Workflows here called the same: import as a copy or a new version of one.
    pub existing: Vec<ExistingWorkflow>,
    /// Validation's errors and warnings, as it would land with nothing remapped.
    pub issues: Vec<super::validate::Issue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionMatch {
    #[serde(flatten)]
    #[ts(flatten)]
    pub need: NeedConnection,
    pub status: NeedStatus,
    /// The kind of the connection here that has the name (when it is another kind).
    #[ts(optional)]
    pub local_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonaMatch {
    pub name: String,
    pub status: NeedStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectMatch {
    #[serde(flatten)]
    #[ts(flatten)]
    pub need: NeedProject,
    pub status: NeedStatus,
    /// The project here with the key.
    #[ts(optional)]
    pub local_id: Option<String>,
}

/// The answer to `{text, check: true}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportCheck {
    /// The PDT that exported it (empty for a bare document).
    pub pdt_version: String,
    pub workflows: Vec<ImportPreview>,
    pub connections: Vec<ConnectionMatch>,
    pub personas: Vec<PersonaMatch>,
    pub projects: Vec<ProjectMatch>,
    /// Things to know that block nothing (an absolute folder from the other machine).
    pub warnings: Vec<String>,
}

/// What an import made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportResult {
    /// Each workflow imported (new ones disabled), in the export's order.
    pub workflows: Vec<super::api::Workflow>,
    /// What is left to do: names left unmapped, webhooks whose secret needs setting, folders from the other machine.
    pub warnings: Vec<String>,
}

/// The report on what `requires` needs here.
pub fn matches(requires: &Requires, local: &Local) -> (Vec<ConnectionMatch>, Vec<PersonaMatch>, Vec<ProjectMatch>) {
    let connections = requires
        .connections
        .iter()
        .map(|c| {
            let status = connection_status(c, local);
            let local_kind = (status == NeedStatus::KindMismatch)
                .then(|| local.connection(&c.name).map(|l| l.kind.clone()))
                .flatten();
            ConnectionMatch { need: c.clone(), status, local_kind }
        })
        .collect();
    let personas = requires
        .personas
        .iter()
        .map(|p| PersonaMatch {
            name: p.clone(),
            status: if local.persona(p).is_some() { NeedStatus::Found } else { NeedStatus::Missing },
        })
        .collect();
    let projects = requires
        .projects
        .iter()
        .map(|p| {
            let l = local.project_by_key(&p.key);
            ProjectMatch {
                need: p.clone(),
                status: if l.is_some() { NeedStatus::Found } else { NeedStatus::Missing },
                local_id: l.map(|l| l.id.clone()),
            }
        })
        .collect();
    (connections, personas, projects)
}

/// What these workflows need, from their documents (so a bare document is matched too): each named connection with
/// its kind (and an MCP server's tools, from `tool_of(node) -> (server, tool)`), each persona, each placeholder's
/// project and each project workflow's own (their names from `names`, the export's `requires`).
pub fn requires_of(
    workflows: &[ExportedWorkflow],
    fields: &dyn Fn(&str) -> Vec<RefField>,
    tool_of: &dyn Fn(&super::doc::Node) -> Option<(String, String)>,
    names: &Requires,
) -> Requires {
    let mut connections: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut personas: BTreeMap<String, String> = BTreeMap::new();
    let mut keys = BTreeSet::new();
    for w in workflows {
        let d = &w.doc;
        if let Some(k) = &w.project {
            keys.insert(k.clone());
        }
        for r in name_refs(d, fields) {
            match r.kind {
                Some(kind) => {
                    let has = connections.keys().find(|(n, k)| n.eq_ignore_ascii_case(&r.name) && *k == kind).cloned();
                    connections.entry(has.unwrap_or((r.name, kind))).or_default();
                }
                None => {
                    personas.entry(r.name.to_lowercase()).or_insert(r.name);
                }
            }
        }
        for n in &d.nodes {
            if let Some((server, tool)) = tool_of(n)
                && let Some((_, tools)) = connections.iter_mut().find(|((s, _), _)| s.eq_ignore_ascii_case(&server))
            {
                tools.insert(tool);
            }
        }
        keys.extend(placeholder_keys(d));
    }
    Requires {
        connections: connections
            .into_iter()
            .map(|((name, kind), tools)| NeedConnection { name, kind, tools: tools.into_iter().collect() })
            .collect(),
        personas: personas.into_values().collect(),
        projects: keys
            .into_iter()
            .map(|key| {
                let name = names.projects.iter().find(|p| p.key == key).map(|p| p.name.clone()).unwrap_or_default();
                NeedProject { key, name }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::doc::Node;

    fn fields(kind: &str) -> Vec<RefField> {
        match kind {
            "discord.post" => vec![RefField::Connection { field: "connection".into(), kind: "discord_webhook".into() }],
            "webhook" => vec![RefField::Connection { field: "hook".into(), kind: "webhook".into() }],
            "mcp.tool" => vec![RefField::Connection { field: "server".into(), kind: "mcp".into() }],
            "http.request" => vec![RefField::Connection { field: "connection".into(), kind: "any".into() }],
            "agent" => vec![RefField::Persona { field: "persona".into() }],
            _ => vec![],
        }
    }

    fn tool_of(n: &Node) -> Option<(String, String)> {
        (n.kind == "mcp.tool").then(|| {
            (
                n.config["server"].as_str().unwrap_or_default().into(),
                n.config["tool"].as_str().unwrap_or_default().into(),
            )
        })
    }

    const P1: &str = "01a14586-8891-729f-ba01-c5d6234fb975";

    fn doc() -> WorkflowDoc {
        serde_json::from_value(json!({
            "name": "Alerts",
            "nodes": [
                {"id": "t", "kind": "ticket.moved", "version": 1, "config": {"projects": [P1, "other"]}},
                {"id": "r", "kind": "agent", "version": 1, "config": {"persona": "Reviewer", "prompt": "p"}},
                {"id": "d", "kind": "discord.post", "version": 1, "config": {"connection": "alerts", "text": "x"}},
                {"id": "d2", "kind": "discord.post", "version": 1, "config": {"connection": "{{state.c}}"}},
                {"id": "m", "kind": "mcp.tool", "version": 1, "config": {"server": "files", "tool": "read"}},
                {"id": "c", "kind": "ticket.create", "version": 1, "config": {"project": P1, "title": "t"}},
            ],
        }))
        .unwrap()
    }

    fn keys() -> HashMap<String, String> {
        HashMap::from([(P1.to_string(), "PDT".to_string())])
    }

    #[test]
    fn project_ids_become_placeholders_and_back() {
        let mut d = doc();
        let used = to_placeholders(&mut d, &keys());
        assert_eq!(used, BTreeSet::from(["PDT".to_string()]));
        assert_eq!(d.nodes[0].config["projects"], json!([{"$project": "PDT"}, "other"]));
        assert_eq!(d.nodes[5].config["project"], json!({"$project": "PDT"}));
        assert_eq!(placeholder_keys(&d), used);
        fill_placeholders(&mut d, &|k| if k == "PDT" { "local-id".into() } else { k.into() });
        assert_eq!(d.nodes[0].config["projects"], json!(["local-id", "other"]));
        assert_eq!(d.nodes[5].config["project"], json!("local-id"));
        // An object with more than the one key is left alone.
        let mut odd: WorkflowDoc = serde_json::from_value(json!({"name": "x", "nodes": [
            {"id": "a", "kind": "k", "version": 1, "config": {"v": {"$project": "PDT", "other": 1}}}]}))
        .unwrap();
        fill_placeholders(&mut odd, &|_| "id".into());
        assert_eq!(odd.nodes[0].config["v"]["other"], 1);
    }

    #[test]
    fn requires_lists_connections_personas_tools_and_projects() {
        let mut d = doc();
        to_placeholders(&mut d, &keys());
        let names = Requires {
            projects: vec![NeedProject { key: "PDT".into(), name: "Dev Team".into() }],
            ..Default::default()
        };
        let w = ExportedWorkflow {
            name: d.name.clone(),
            description: String::new(),
            scope: Scope::Project,
            project: Some("OPS".into()),
            doc: d,
        };
        let r = requires_of(&[w], &fields, &tool_of, &names);
        assert_eq!(
            r.connections,
            vec![
                NeedConnection { name: "alerts".into(), kind: "discord_webhook".into(), tools: vec![] },
                NeedConnection { name: "files".into(), kind: "mcp".into(), tools: vec!["read".into()] },
            ]
        );
        assert_eq!(r.personas, vec!["Reviewer".to_string()]);
        assert_eq!(
            r.projects,
            vec![
                NeedProject { key: "OPS".into(), name: String::new() },
                NeedProject { key: "PDT".into(), name: "Dev Team".into() }
            ]
        );
    }

    #[test]
    fn rename_refs_rewrites_mapped_names_only() {
        let mut d = doc();
        let conns = HashMap::from([("ALERTS".to_string(), "team-alerts".to_string())]);
        let personas = HashMap::from([("reviewer".to_string(), "Critic".to_string())]);
        rename_refs(&mut d, &fields, &conns, &personas);
        assert_eq!(d.nodes[2].config["connection"], "team-alerts");
        assert_eq!(d.nodes[3].config["connection"], "{{state.c}}");
        assert_eq!(d.nodes[1].config["persona"], "Critic");
        assert_eq!(d.nodes[4].config["server"], "files");
    }

    fn local() -> Local {
        Local {
            connections: vec![
                LocalConnection { name: "Alerts".into(), kind: "discord_webhook".into(), has_secret: true },
                LocalConnection { name: "files".into(), kind: "http".into(), has_secret: false },
                LocalConnection { name: "ops".into(), kind: "discord_webhook".into(), has_secret: true },
            ],
            personas: vec!["Reviewer".into(), "Critic".into()],
            projects: vec![LocalProject { id: "here".into(), key: "WEB".into(), name: "Web".into() }],
        }
    }

    fn requires() -> Requires {
        Requires {
            connections: vec![
                NeedConnection { name: "alerts".into(), kind: "discord_webhook".into(), tools: vec![] },
                NeedConnection { name: "files".into(), kind: "mcp".into(), tools: vec!["read".into()] },
            ],
            personas: vec!["Coder".into()],
            projects: vec![NeedProject { key: "PDT".into(), name: "Dev".into() }],
        }
    }

    #[test]
    fn matches_reports_found_missing_and_kind_mismatch() {
        let (c, p, pr) = matches(&requires(), &local());
        assert_eq!(c[0].status, NeedStatus::Found);
        assert_eq!((c[1].status, c[1].local_kind.as_deref()), (NeedStatus::KindMismatch, Some("http")));
        assert_eq!(p[0].status, NeedStatus::Missing);
        assert_eq!((pr[0].status, pr[0].local_id.as_deref()), (NeedStatus::Missing, None));
        let mut r = requires();
        r.projects[0].key = "web".into();
        assert_eq!(matches(&r, &local()).2[0].local_id.as_deref(), Some("here"));
    }

    #[test]
    fn resolve_needs_every_missing_name_answered() {
        let e = resolve(&requires(), &ImportMapping::default(), &local()).unwrap_err();
        assert!(e.contains("connection files") && e.contains("persona Coder") && e.contains("project PDT"), "{e}");
        let m = ImportMapping {
            connections: BTreeMap::from([("files".into(), None)]),
            personas: BTreeMap::from([("coder".into(), Some("critic".into()))]),
            projects: BTreeMap::from([("PDT".into(), Some("WEB".into()))]),
        };
        let r = resolve(&requires(), &m, &local()).unwrap();
        assert_eq!(r.personas.get("Coder").map(String::as_str), Some("Critic"));
        assert_eq!(r.projects.get("PDT").map(String::as_str), Some("here"));
        assert!(r.connections.is_empty());
        assert_eq!(r.unmapped, vec!["the mcp connection files is left unmapped".to_string()]);
        // A mapping to a connection of the wrong kind is refused.
        let bad = ImportMapping { connections: BTreeMap::from([("files".into(), Some("ops".into()))]), ..m.clone() };
        assert!(resolve(&requires(), &bad, &local()).unwrap_err().contains("needs a mcp one"));
        // An unmapped project stays its key.
        let left = ImportMapping { projects: BTreeMap::from([("PDT".into(), None)]), ..m };
        assert_eq!(resolve(&requires(), &left, &local()).unwrap().projects.get("PDT").map(String::as_str), Some("PDT"));
    }

    #[test]
    fn parse_reads_an_export_and_a_bare_doc() {
        let mut d = doc();
        to_placeholders(&mut d, &keys());
        let e = WorkflowExport {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION,
            pdt_version: "1.2.3".into(),
            exported_at: 5,
            workflows: vec![ExportedWorkflow {
                name: d.name.clone(),
                description: String::new(),
                scope: Scope::Host,
                project: None,
                doc: d,
            }],
            requires: Requires::default(),
        };
        let text = export_text(&e);
        assert!(text.contains("\n  \"format\": \"pdt-workflow\""), "pretty-printed");
        assert_eq!(parse_import(&text).unwrap(), e);
        let bare = serde_json::to_string(&doc()).unwrap();
        let b = parse_import(&bare).unwrap();
        assert_eq!((b.workflows.len(), b.workflows[0].name.as_str(), b.workflows[0].scope), (1, "Alerts", Scope::Host));
        assert_eq!(b.requires, Requires::default());
    }

    #[test]
    fn parse_errors_say_what_is_wrong() {
        assert!(parse_import("").unwrap_err().contains("paste"));
        assert!(parse_import("{nope").unwrap_err().starts_with("this is not valid JSON"));
        assert!(parse_import("[1]").unwrap_err().contains("JSON object"));
        let newer = json!({"format": FORMAT, "formatVersion": 2, "workflows": []}).to_string();
        assert!(parse_import(&newer).unwrap_err().contains("format version 2"));
        let other = json!({"format": "n8n", "formatVersion": 1}).to_string();
        assert!(parse_import(&other).unwrap_err().contains("not a PDT workflow export"));
        let empty = json!({"format": FORMAT, "formatVersion": 1, "pdtVersion": "", "exportedAt": 0, "workflows": []});
        assert!(parse_import(&empty.to_string()).unwrap_err().contains("no workflows"));
        assert!(parse_import(r#"{"name": "x"}"#).unwrap_err().contains("neither"));
    }

    #[test]
    fn copy_names_avoid_taken_ones() {
        assert_eq!(copy_name("A", &|_| false), "A (imported)");
        assert_eq!(copy_name("A", &|n| n == "A (imported)" || n == "A (imported 2)"), "A (imported 3)");
    }
}
