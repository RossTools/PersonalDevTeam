//! Export and import (PDT-155): one envelope of current documents with project placeholders and what they need, the
//! same text copied or downloaded; importing checks it against this host, applies the user's mapping and creates the
//! workflows disabled. The pure part is pdt-core `workflow::transfer`.

use super::routes::{body, checked, conflict, create_workflow, invalid, load, validate_with_tools};
use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use pdt_core::workflow::api::{Author, Scope, Source, Workflow};
use pdt_core::workflow::doc::{Node, WorkflowDoc};
use pdt_core::workflow::transfer::{
    ExistingWorkflow, ExportedWorkflow, FORMAT, FORMAT_VERSION, ImportCheck, ImportPreview, ImportRequest,
    ImportResult, ImportTarget, Local, LocalConnection, LocalProject, NeedProject, NeedStatus, RefField, Requires,
    WorkflowExport, copy_name, fill_placeholders, matches, name_refs, parse_import, rename_refs, requires_of, resolve,
    to_placeholders,
};
use pdt_core::workflow::validate::{Issue, Severity};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path as FsPath;
use ts_rs::TS;

/// The fields of a kind's config that name a connection (its schema's `x-connection-kind`) or a persona.
pub(crate) fn ref_fields(reg: &crate::integrations::Registry, kind: &str) -> Vec<RefField> {
    let mut out = vec![];
    if kind == super::nodes::agent::KIND {
        out.push(RefField::Persona { field: "persona".into() });
    }
    let Some(spec) = reg.catalog().spec(kind) else { return out };
    if let Some(props) = spec.config_schema.get("properties").and_then(Value::as_object) {
        for (field, p) in props {
            if let Some(k) = p.get("x-connection-kind").and_then(Value::as_str) {
                out.push(RefField::Connection { field: field.clone(), kind: k.to_string() });
            }
        }
    }
    out
}

/// An `mcp.tool` step's server and tool.
fn tool_of(n: &Node) -> Option<(String, String)> {
    if n.kind != super::nodes::mcp_tool::KIND {
        return None;
    }
    let s = |f: &str| n.config.get(f).and_then(Value::as_str).map(|s| s.trim().to_string());
    Some((s("server")?, s("tool")?))
}

/// The connections, personas and projects here.
async fn local(st: &AppState) -> Result<Local, ApiError> {
    crate::personas::ensure_builtins(st).await;
    let connections = st
        .store
        .list_connections()
        .await?
        .into_iter()
        .map(|c| LocalConnection { name: c.name, kind: c.kind, has_secret: c.has_secret })
        .collect();
    let personas = st.store.list_personas().await?.into_iter().filter(|p| !p.archived).map(|p| p.name).collect();
    let projects = st
        .store
        .list_projects()
        .await?
        .into_iter()
        .map(|p| LocalProject { id: p.id, key: p.key, name: p.name })
        .collect();
    Ok(Local { connections, personas, projects })
}

/// The export of these workflows' current versions.
pub(crate) async fn export(st: &AppState, ids: &[String]) -> Result<WorkflowExport, ApiError> {
    if ids.is_empty() {
        return Err(invalid("name the workflows to export (ids)"));
    }
    let here = local(st).await?;
    let keys: HashMap<String, String> = here.projects.iter().map(|p| (p.id.clone(), p.key.clone())).collect();
    let mut workflows = vec![];
    for id in ids {
        let w = load(st, id).await?;
        let mut doc =
            st.store.workflow_doc(&w.id, w.version).await?.ok_or_else(|| ApiError::not_found("workflow version"))?;
        to_placeholders(&mut doc, &keys);
        let project = w.project_id.as_ref().and_then(|p| keys.get(p)).cloned();
        workflows.push(ExportedWorkflow { name: w.name, description: w.description, scope: w.scope, project, doc });
    }
    let names = Requires {
        projects: here.projects.iter().map(|p| NeedProject { key: p.key.clone(), name: p.name.clone() }).collect(),
        ..Default::default()
    };
    let requires = requires_of(&workflows, &|k: &str| ref_fields(&st.registry(), k), &tool_of, &names);
    Ok(WorkflowExport {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        pdt_version: env!("CARGO_PKG_VERSION").into(),
        exported_at: now_ms(),
        workflows,
        requires,
    })
}

/// `GET /api/workflows/{id}/export`.
pub(crate) async fn export_one(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WorkflowExport>, ApiError> {
    Ok(Json(export(&st, &[id]).await?))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ExportRequest {
    /// The workflows' ids, in the order they go in the export.
    pub ids: Vec<String>,
}

/// `POST /api/workflows/export {ids}`: several workflows as one envelope.
pub(crate) async fn export_many(
    State(st): State<AppState>,
    payload: Result<Json<ExportRequest>, JsonRejection>,
) -> Result<Json<WorkflowExport>, ApiError> {
    let r = body(payload)?;
    Ok(Json(export(&st, &r.ids).await?))
}

/// `command` steps that run in an absolute folder: a path on the machine the workflow came from.
fn folder_warnings(doc: &WorkflowDoc) -> Vec<String> {
    doc.nodes
        .iter()
        .filter(|n| n.kind == "command")
        .filter_map(|n| {
            let cwd = n.config.get("cwd").and_then(Value::as_str)?.trim();
            (FsPath::new(cwd).is_absolute() || cwd.starts_with('/'))
                .then(|| format!("{}: {} runs in {cwd}, a folder on the machine it came from", doc.name, n.id))
        })
        .collect()
}

/// Webhook connections here, needed by the import, that have no secret yet.
fn secret_warnings(requires: &Requires, here: &Local, renamed: &HashMap<String, String>) -> Vec<String> {
    requires
        .connections
        .iter()
        .filter(|c| c.kind == super::nodes::webhook::WEBHOOK_CONNECTION)
        .filter_map(|c| {
            let name = renamed.get(&c.name).unwrap_or(&c.name);
            let l = here.connection(name)?;
            (!l.has_secret).then(|| format!("the webhook {} has no secret yet: set it in Settings → Plugins", l.name))
        })
        .collect()
}

fn same_name(all: &[Workflow], name: &str) -> Vec<ExistingWorkflow> {
    all.iter()
        .filter(|w| w.name.trim().eq_ignore_ascii_case(name.trim()))
        .map(|w| ExistingWorkflow {
            id: w.id.clone(),
            name: w.name.clone(),
            scope: w.scope,
            project_id: w.project_id.clone(),
            source: w.source,
        })
        .collect()
}

fn parsed(text: &str) -> Result<WorkflowExport, ApiError> {
    parse_import(text).map_err(|m| ApiError::new(axum::http::StatusCode::BAD_REQUEST, "invalid_export", m))
}

/// `{text, check: true}`: what the text holds and how what it needs stands here.
pub(crate) async fn check(st: &AppState, text: &str) -> Result<ImportCheck, ApiError> {
    let e = parsed(text)?;
    let here = local(st).await?;
    let requires = requires_of(&e.workflows, &|k: &str| ref_fields(&st.registry(), k), &tool_of, &e.requires);
    let (connections, personas, projects) = matches(&requires, &here);
    let all = st.store.list_workflows(None).await?;
    let mut warnings = vec![];
    let workflows = e
        .workflows
        .iter()
        .map(|w| {
            let mut doc = w.doc.clone();
            // Validated as it would land with nothing remapped: found projects by their id, the rest their key.
            fill_placeholders(&mut doc, &|k| here.project_by_key(k).map(|p| p.id.clone()).unwrap_or_else(|| k.into()));
            warnings.extend(folder_warnings(&doc));
            ImportPreview {
                name: w.name.clone(),
                description: w.description.clone(),
                scope: w.scope,
                project: w.project.clone(),
                nodes: u32::try_from(w.doc.nodes.len()).unwrap_or(u32::MAX),
                existing: same_name(&all, &w.name),
                issues: validate_with_tools(st, &doc),
            }
        })
        .collect();
    warnings.extend(secret_warnings(&requires, &here, &HashMap::new()));
    Ok(ImportCheck { pdt_version: e.pdt_version, workflows, connections, personas, projects, warnings })
}

/// One workflow, ready to write.
struct Planned {
    doc: WorkflowDoc,
    scope: Scope,
    project_id: Option<String>,
    version_of: Option<Workflow>,
}

/// Imports the text with the user's mapping and targets: new workflows start disabled; a new version of an existing
/// one is saved (an agent's is proposed). Nothing is written unless every workflow can be.
pub(crate) async fn import(st: &AppState, r: &ImportRequest, author: Author) -> Result<ImportResult, ApiError> {
    let e = parsed(&r.text)?;
    let here = local(st).await?;
    let requires = requires_of(&e.workflows, &|k: &str| ref_fields(&st.registry(), k), &tool_of, &e.requires);
    let resolved = resolve(&requires, &r.mapping, &here).map_err(|m| conflict("needs_mapping", m))?;
    let all = st.store.list_workflows(None).await?;
    let mut taken: Vec<String> = all.iter().map(|w| w.name.to_lowercase()).collect();
    let mut planned = vec![];
    let mut warnings = resolved.unmapped.clone();
    for (i, w) in e.workflows.iter().enumerate() {
        let t = r.targets.get(i).cloned().unwrap_or_default();
        if t.skip {
            continue;
        }
        let mut doc = w.doc.clone();
        rename_refs(&mut doc, &|k: &str| ref_fields(&st.registry(), k), &resolved.connections, &resolved.personas);
        fill_placeholders(&mut doc, &|k| resolved.projects.get(k).cloned().unwrap_or_else(|| k.into()));
        warnings.extend(folder_warnings(&doc));
        let version_of = match &t.version_of {
            Some(id) => {
                let existing = load(st, id).await?;
                if existing.source == Source::Repo {
                    return Err(conflict(
                        "workflow_in_repo",
                        format!("{} lives in the repo: import as a copy instead", existing.name),
                    ));
                }
                Some(existing)
            }
            None => None,
        };
        if version_of.is_none() {
            if taken.contains(&doc.name.to_lowercase()) {
                if !t.copy {
                    return Err(conflict(
                        "workflow_exists",
                        format!("a workflow is already called {}: import it as a copy or as a new version", doc.name),
                    ));
                }
                doc.name = copy_name(&doc.name, &|n| taken.contains(&n.to_lowercase()));
            }
            taken.push(doc.name.to_lowercase());
        }
        checked(st, &doc)
            .map_err(|err| ApiError::new(err.status(), err.code(), format!("{}: {}", w.name, err.message())))?;
        let (scope, project_id) = target_scope(w, &t, &resolved.projects, &here)?;
        planned.push(Planned { doc, scope, project_id, version_of });
    }
    if planned.is_empty() {
        return Err(invalid("every workflow is skipped: nothing to import"));
    }
    let from = match e.pdt_version.as_str() {
        "" => "imported".to_string(),
        v => format!("imported from PDT {v}"),
    };
    let mut workflows = vec![];
    for p in planned {
        let w = match p.version_of {
            Some(existing) => {
                let now = now_ms();
                if author == Author::Agent {
                    st.store.propose_workflow_version(&existing.id, &p.doc, author, &from, now).await?;
                } else {
                    st.store.save_workflow_version(&existing.id, &p.doc, author, &from, now).await?;
                }
                load(st, &existing.id).await?
            }
            None => create_workflow(st, &p.doc, Some(p.scope), p.project_id, false, &from, author).await?.workflow,
        };
        workflows.push(w);
    }
    warnings.extend(secret_warnings(&requires, &here, &resolved.connections));
    Ok(ImportResult { workflows, warnings })
}

/// Where a workflow goes: the target's scope and project, else the export's (its project through the mapping).
fn target_scope(
    w: &ExportedWorkflow,
    t: &ImportTarget,
    projects: &HashMap<String, String>,
    here: &Local,
) -> Result<(Scope, Option<String>), ApiError> {
    let scope = t.scope.unwrap_or(w.scope);
    if scope != Scope::Project {
        return Ok((scope, None));
    }
    let given = t.project_id.as_deref().filter(|p| !p.trim().is_empty());
    let from_export = w.project.as_ref().and_then(|k| projects.get(k)).map(String::as_str);
    match given.or(from_export).and_then(|p| here.project(p)) {
        Some(p) => Ok((scope, Some(p.id.clone()))),
        None => Err(invalid(format!("{}: pick the project it applies to (projectId)", w.name))),
    }
}

/// `POST /api/workflows/import`: `{text, check: true}` → [`ImportCheck`]; `{text, mapping, targets}` → [`ImportResult`].
pub(crate) async fn import_route(
    State(st): State<AppState>,
    payload: Result<Json<ImportRequest>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let r = body(payload)?;
    let out = if r.check {
        serde_json::to_value(check(&st, &r.text).await?)
    } else {
        serde_json::to_value(import(&st, &r, Author::User).await?)
    };
    Ok(Json(out.map_err(ApiError::internal)?))
}

/// Warnings for what `doc` names that is not here (shown on the workflow's page): connections (missing, of another
/// kind, a webhook without its secret), personas and projects.
pub(crate) async fn reference_issues(st: &AppState, doc: &WorkflowDoc) -> Vec<Issue> {
    let here = match local(st).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e.message(), "could not check a workflow's references");
            return vec![];
        }
    };
    let warn = |code: &str, node: &str, message: String| Issue {
        severity: Severity::Warning,
        code: code.into(),
        node: Some(node.into()),
        message,
    };
    let mut out = vec![];
    // The personas' agents, for a step whose agent is switched off (PDT-270).
    let host = st.store.get_host_settings().await.ok();
    let personas = st.store.list_personas().await.unwrap_or_default();
    for r in name_refs(doc, &|k: &str| ref_fields(&st.registry(), k)) {
        match &r.kind {
            Some(kind) => {
                let need = pdt_core::workflow::transfer::NeedConnection {
                    name: r.name.clone(),
                    kind: kind.clone(),
                    tools: vec![],
                };
                match pdt_core::workflow::transfer::connection_status(&need, &here) {
                    NeedStatus::Missing => out.push(warn(
                        "missing_connection",
                        &r.node,
                        format!("{}: no connection here is called {} (add it in Settings → Plugins)", r.node, r.name),
                    )),
                    NeedStatus::KindMismatch => out.push(warn(
                        "connection_kind",
                        &r.node,
                        format!(
                            "{}: {} is a {} connection; this step needs a {kind} one",
                            r.node,
                            r.name,
                            here.connection(&r.name).map(|c| c.kind.as_str()).unwrap_or_default()
                        ),
                    )),
                    NeedStatus::Found => {
                        let c = here.connection(&r.name).expect("found");
                        if c.kind == super::nodes::webhook::WEBHOOK_CONNECTION && !c.has_secret {
                            out.push(warn(
                                "hook_secret",
                                &r.node,
                                format!(
                                    "{}: the webhook {} has no secret yet: set it in Settings → Plugins",
                                    r.node, c.name
                                ),
                            ));
                        }
                    }
                }
            }
            None if here.persona(&r.name).is_none() => {
                out.push(warn("missing_persona", &r.node, format!("{}: no persona here is called {}", r.node, r.name)))
            }
            None => {
                let by_agent = doc
                    .nodes
                    .iter()
                    .find(|n| n.id == r.node)
                    .is_none_or(|n| n.config.get("owner").and_then(Value::as_str) != Some("user"));
                let persona = personas.iter().find(|p| !p.archived && p.name.eq_ignore_ascii_case(r.name.trim()));
                if let (true, Some(p), Some(host)) = (by_agent, persona, &host)
                    && let Some(why) = host.agent_off(p.setup.agent)
                {
                    out.push(warn(
                        "agent_off",
                        &r.node,
                        format!("{}: {} runs on an agent that is off: {why}", r.node, p.name),
                    ));
                }
            }
        }
    }
    for n in &doc.nodes {
        for id in project_refs(n) {
            if !here.projects.iter().any(|p| p.id == id) {
                out.push(warn("missing_project", &n.id, format!("{}: no project here is {id}", n.id)));
            }
        }
    }
    out
}

/// The project ids a node names outright: a trigger's `projects`, a created or imported ticket's `project`.
fn project_refs(n: &Node) -> Vec<String> {
    let plain = |v: &Value| v.as_str().map(str::trim).filter(|s| !s.is_empty() && !s.contains("{{")).map(String::from);
    let mut out: Vec<String> = n
        .config
        .get("projects")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(plain).collect())
        .unwrap_or_default();
    if matches!(n.kind.as_str(), "ticket.create" | "github.import") {
        out.extend(n.config.get("project").and_then(plain));
    }
    out
}
