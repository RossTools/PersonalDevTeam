//! The workflow API (Phase 7a spec §9): the catalog, documents and versions, validation and dry runs, runs, the event
//! log and connections.

use super::catalog::{KindSpec, PortSpec};
use super::engine::{self, DryEvent, DryRun};
use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::workflow::api::{
    Author, Connection, LogEvent, Run, RunDetail, RunStatus, Scope, Source, VersionStatus, Workflow, WorkflowDetail,
    WorkflowVersion,
};
use pdt_core::workflow::diff::{DocDiff, diff};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::validate::{Issue, Severity, is_valid, validate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use ts_rs::TS;

pub(crate) fn conflict(code: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, message)
}

pub(crate) fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", message)
}

pub(crate) fn body<T: Default>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    match payload {
        Ok(Json(v)) => Ok(v),
        Err(JsonRejection::MissingJsonContentType(_)) => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

/// The document's issues, or 422 `workflow_invalid` naming its errors.
pub(crate) fn checked(st: &AppState, doc: &WorkflowDoc) -> Result<Vec<Issue>, ApiError> {
    let issues = validate(doc, st.registry().catalog());
    if !is_valid(&issues) {
        let errors: Vec<&str> =
            issues.iter().filter(|i| i.severity == Severity::Error).map(|i| i.message.as_str()).collect();
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "workflow_invalid", errors.join("; ")));
    }
    Ok(issues)
}

pub(crate) async fn load(st: &AppState, id: &str) -> Result<Workflow, ApiError> {
    st.store.get_workflow(id).await?.ok_or_else(|| ApiError::not_found("workflow"))
}

/// A document's issues, with warnings about its MCP tool nodes from their servers' cached tools.
pub(crate) fn validate_with_tools(st: &AppState, doc: &WorkflowDoc) -> Vec<Issue> {
    let mut issues = validate(doc, st.registry().catalog());
    issues.extend(super::nodes::mcp_tool::tool_issues(doc, |s| st.mcp.cached(s)));
    issues
}

/// [`validate_with_tools`], with warnings for the connections, personas and projects it names that are not here.
pub(crate) async fn doc_issues(st: &AppState, doc: &WorkflowDoc) -> Vec<Issue> {
    let mut issues = validate_with_tools(st, doc);
    issues.extend(super::transfer::reference_issues(st, doc).await);
    issues
}

pub(crate) async fn detail(st: &AppState, w: Workflow) -> Result<WorkflowDetail, ApiError> {
    let doc = st.store.workflow_doc(&w.id, w.version).await?.ok_or_else(|| ApiError::not_found("workflow version"))?;
    let issues = doc_issues(st, &doc).await;
    let needs_project = w.project_id.is_none() && super::catalog::doc_needs_project(st.registry().catalog(), &doc);
    Ok(WorkflowDetail { workflow: w, doc, issues, needs_project })
}

pub(crate) async fn get_catalog(State(st): State<AppState>) -> Json<Vec<KindSpec>> {
    Json(super::catalog::listed(&st))
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ListQuery {
    project: Option<String>,
}

pub(crate) async fn list(
    State(st): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<Workflow>>, ApiError> {
    let reg = st.registry();
    let all = st.store.list_workflows(q.project.as_deref()).await?;
    Ok(Json(all.into_iter().filter(|w| !super::builtins::hidden(&reg, w)).collect()))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewWorkflow {
    pub doc: Option<WorkflowDoc>,
    pub scope: Option<Scope>,
    pub project_id: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub note: String,
}

pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewWorkflow>, JsonRejection>,
) -> Result<(StatusCode, Json<WorkflowDetail>), ApiError> {
    let n = body(payload)?;
    let doc = n.doc.ok_or_else(|| invalid("doc is required"))?;
    let w = create_workflow(&st, &doc, n.scope, n.project_id, n.enabled, &n.note, Author::User).await?;
    Ok((StatusCode::CREATED, Json(w)))
}

/// Adds a workflow (`POST /api/workflows`, instantiating a template, and pdt-mcp's `workflow_save`). One an agent
/// writes starts disabled: enabling is the user's.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_workflow(
    st: &AppState,
    doc: &WorkflowDoc,
    scope: Option<Scope>,
    project_id: Option<String>,
    enabled: bool,
    note: &str,
    author: Author,
) -> Result<WorkflowDetail, ApiError> {
    checked(st, doc)?;
    let scope = scope.unwrap_or(if project_id.is_some() { Scope::Project } else { Scope::Host });
    let project_id = match scope {
        Scope::Project => {
            let id = project_id.ok_or_else(|| invalid("a project workflow needs projectId"))?;
            st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
            Some(id)
        }
        _ if project_id.is_some() => return Err(invalid("only a project workflow has a projectId")),
        _ => None,
    };
    let now = now_ms();
    let w = Workflow {
        id: uuid::Uuid::now_v7().to_string(),
        name: doc.name.clone(),
        description: doc.description.clone(),
        scope,
        project_id,
        enabled: enabled && author != Author::Agent,
        builtin: None,
        source: Source::Host,
        repo_file: None,
        version: 1,
        proposed: None,
        created_at: now,
        updated_at: now,
    };
    st.store.insert_workflow(&w, doc, author, note).await?;
    detail(st, load(st, &w.id).await?).await
}

pub(crate) async fn get_one(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WorkflowDetail>, ApiError> {
    let w = load(&st, &id).await?;
    Ok(Json(detail(&st, w).await?))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveWorkflow {
    pub doc: Option<WorkflowDoc>,
    #[serde(default)]
    pub note: String,
    /// Propose the version instead (an agent's change): it waits for the user to accept it (Phase 7d spec §6.2).
    #[serde(default)]
    pub propose: bool,
    /// Who proposes it (PDT-320): set by `workflow_save` from the agent's session, never by a request.
    #[serde(skip)]
    #[ts(skip)]
    #[schemars(skip)]
    pub by: Option<String>,
}

/// `PUT /api/workflows/{id}`: a new version, or with `propose` a proposal.
pub(crate) async fn save(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<SaveWorkflow>, JsonRejection>,
) -> Result<Json<WorkflowDetail>, ApiError> {
    let s = body(payload)?;
    let doc = s.doc.ok_or_else(|| invalid("doc is required"))?;
    let w = load(&st, &id).await?;
    if w.source == Source::Repo {
        return Err(conflict(
            "workflow_in_repo",
            format!("{} lives in the repo ({}): edit the file", w.name, w.repo_file.unwrap_or_default()),
        ));
    }
    checked(&st, &doc)?;
    if s.propose {
        st.store.propose_workflow_version(&id, &doc, Author::Agent, &s.note, s.by.as_deref(), now_ms()).await?;
    } else {
        st.store.save_workflow_version(&id, &doc, Author::User, &s.note, now_ms()).await?;
    }
    Ok(Json(detail(&st, load(&st, &id).await?).await?))
}

async fn proposal(st: &AppState, id: &str, v: u32) -> Result<WorkflowDoc, ApiError> {
    load(st, id).await?;
    match st.store.workflow_version_status(id, v).await? {
        None => Err(ApiError::not_found("workflow version")),
        Some(VersionStatus::Proposed) => {
            st.store.workflow_doc(id, v).await?.ok_or_else(|| ApiError::not_found("workflow version"))
        }
        Some(_) => Err(conflict("not_proposed", format!("version {v} is not a proposal waiting for review"))),
    }
}

/// `POST /api/workflows/{id}/versions/{v}/accept`: the proposal becomes the current version.
pub(crate) async fn accept(
    State(st): State<AppState>,
    Path((id, v)): Path<(String, u32)>,
) -> Result<Json<WorkflowDetail>, ApiError> {
    let doc = proposal(&st, &id, v).await?;
    checked(&st, &doc)?;
    if !st.store.accept_workflow_version(&id, v, now_ms()).await? {
        return Err(conflict("not_proposed", format!("version {v} is not a proposal waiting for review")));
    }
    Ok(Json(detail(&st, load(&st, &id).await?).await?))
}

/// `POST /api/workflows/{id}/versions/{v}/discard`: sets the proposal aside.
pub(crate) async fn discard(
    State(st): State<AppState>,
    Path((id, v)): Path<(String, u32)>,
) -> Result<Json<WorkflowDetail>, ApiError> {
    proposal(&st, &id, v).await?;
    st.store.discard_workflow_version(&id, v).await?;
    Ok(Json(detail(&st, load(&st, &id).await?).await?))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowPatch {
    pub enabled: Option<bool>,
}

/// `PATCH /api/workflows/{id}`: enable or disable (enabling is consent to what it does).
pub(crate) async fn patch(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<WorkflowPatch>, JsonRejection>,
) -> Result<Json<Workflow>, ApiError> {
    let p = body(payload)?;
    let w = load(&st, &id).await?;
    let install_updates = w.builtin.as_deref() == Some(super::builtins::INSTALL_UPDATES);
    if let Some(enabled) = p.enabled {
        if enabled {
            checked(&st, &detail(&st, w).await?.doc)?;
        }
        st.store.set_workflow_enabled(&id, enabled, now_ms()).await?;
        // Turned on with an update on offer: it installs that one too (PDT-369).
        if enabled && install_updates {
            crate::install_updates::wake(&st).await;
        }
    }
    Ok(Json(load(&st, &id).await?))
}

pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let w = load(&st, &id).await?;
    if w.builtin.is_some() {
        return Err(conflict("builtin_workflow", format!("{} is built in: disable it instead", w.name)));
    }
    if w.source == Source::Repo {
        return Err(conflict("workflow_in_repo", format!("{} lives in the repo: remove its file", w.name)));
    }
    engine::delete_workflow(&st, &id).await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn versions(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<WorkflowVersion>>, ApiError> {
    load(&st, &id).await?;
    Ok(Json(st.store.list_workflow_versions(&id).await?))
}

pub(crate) async fn version(
    State(st): State<AppState>,
    Path((id, v)): Path<(String, u32)>,
) -> Result<Json<WorkflowVersion>, ApiError> {
    load(&st, &id).await?;
    let mut meta = st
        .store
        .list_workflow_versions(&id)
        .await?
        .into_iter()
        .find(|x| x.version == v)
        .ok_or_else(|| ApiError::not_found("workflow version"))?;
    meta.doc = st.store.workflow_doc(&id, v).await?;
    Ok(Json(meta))
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DiffQuery {
    from: Option<u32>,
    to: Option<u32>,
}

/// `GET /api/workflows/{id}/diff?from=&to=`: by default the current version against the one before.
pub(crate) async fn version_diff(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<DocDiff>, ApiError> {
    let w = load(&st, &id).await?;
    let to = q.to.unwrap_or(w.version);
    let from = q.from.unwrap_or(to.saturating_sub(1).max(1));
    let a = st.store.workflow_doc(&id, from).await?.ok_or_else(|| ApiError::not_found("workflow version"))?;
    let b = st.store.workflow_doc(&id, to).await?.ok_or_else(|| ApiError::not_found("workflow version"))?;
    Ok(Json(diff(&a, &b, &format!("v{from}"), &format!("v{to}"))))
}

pub(crate) async fn state(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    load(&st, &id).await?;
    Ok(Json(Value::Object(st.store.workflow_state(&id).await?)))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunNow {
    #[serde(default)]
    #[ts(type = "unknown")]
    pub input: Value,
    pub run_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Started {
    /// None when a run with the key already ran.
    pub run_id: Option<String>,
}

/// `POST /api/workflows/{id}/run`: Run now (the `manual` trigger), enabled or not.
pub(crate) async fn run_now(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<RunNow>, JsonRejection>,
) -> Result<Json<Started>, ApiError> {
    let r = body(payload)?;
    let w = load(&st, &id).await?;
    let run_id = engine::start_manual(&st, &w, r.input, r.run_key).await.map_err(|e| conflict("cannot_run", e))?;
    Ok(Json(Started { run_id }))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ValidateRequest {
    pub doc: Option<WorkflowDoc>,
}

/// What the builder needs after every change: the issues, and each node's ports as its config gives them.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowCheck {
    pub issues: Vec<Issue>,
    /// By node id (unknown kinds left out); `error` is not listed.
    pub ports: BTreeMap<String, Vec<PortSpec>>,
}

/// `POST /api/workflows/check`.
pub(crate) async fn check_doc(
    State(st): State<AppState>,
    payload: Result<Json<ValidateRequest>, JsonRejection>,
) -> Result<Json<WorkflowCheck>, ApiError> {
    let doc = body(payload)?.doc.ok_or_else(|| invalid("doc is required"))?;
    let ports = doc.nodes.iter().filter_map(|n| Some((n.id.clone(), st.registry().catalog().node_ports(n)?))).collect();
    Ok(Json(WorkflowCheck { issues: doc_issues(&st, &doc).await, ports }))
}

pub(crate) async fn validate_doc(
    State(st): State<AppState>,
    payload: Result<Json<ValidateRequest>, JsonRejection>,
) -> Result<Json<Vec<Issue>>, ApiError> {
    let doc = body(payload)?.doc.ok_or_else(|| invalid("doc is required"))?;
    Ok(Json(doc_issues(&st, &doc).await))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DryRunRequest {
    /// A document to try (else `workflowId`'s current one).
    pub doc: Option<WorkflowDoc>,
    pub workflow_id: Option<String>,
    /// A hand-written event (else `eventId` from the log).
    pub event: Option<DryEvent>,
    pub event_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DryRunResult {
    pub issues: Vec<Issue>,
    pub run: Option<DryRun>,
}

/// `POST /api/workflows/dry-run`: no side effects; shows what each node would do.
pub(crate) async fn dry_run(
    State(st): State<AppState>,
    payload: Result<Json<DryRunRequest>, JsonRejection>,
) -> Result<Json<DryRunResult>, ApiError> {
    let r = body(payload)?;
    let (wf, doc) = match (r.doc, r.workflow_id) {
        (Some(doc), id) => {
            let wf = match id {
                Some(id) => load(&st, &id).await?,
                None => Workflow {
                    id: "dry-run".into(),
                    name: doc.name.clone(),
                    description: doc.description.clone(),
                    scope: Scope::Host,
                    project_id: None,
                    enabled: false,
                    builtin: None,
                    source: Source::Host,
                    repo_file: None,
                    version: 0,
                    proposed: None,
                    created_at: 0,
                    updated_at: 0,
                },
            };
            (wf, doc)
        }
        (None, Some(id)) => {
            let d = detail(&st, load(&st, &id).await?).await?;
            (d.workflow, d.doc)
        }
        (None, None) => return Err(invalid("give doc or workflowId")),
    };
    let event = match (r.event, r.event_id) {
        (Some(e), _) => e,
        (None, Some(id)) => {
            let e = st.store.get_log_event(&id).await?.ok_or_else(|| ApiError::not_found("event"))?;
            let payload = engine::trigger_value(&st, &e);
            DryEvent { kind: e.kind, payload }
        }
        (None, None) => return Err(invalid("give event or eventId")),
    };
    let issues = validate_with_tools(&st, &doc);
    if !is_valid(&issues) {
        return Ok(Json(DryRunResult { issues, run: None }));
    }
    let run = engine::dry_run(&st, &wf, &doc, event).await;
    Ok(Json(DryRunResult { issues, run: Some(run) }))
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RunsQuery {
    workflow: Option<String>,
    ticket: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
}

pub(crate) async fn list_runs(
    State(st): State<AppState>,
    Query(q): Query<RunsQuery>,
) -> Result<Json<Vec<Run>>, ApiError> {
    let status = match q.status.as_deref() {
        Some(s) => Some(RunStatus::parse(s).ok_or_else(|| invalid(format!("no run status is called {s}")))?),
        None => None,
    };
    let f = pdt_store::RunFilter { workflow_id: q.workflow, ticket_id: q.ticket, status, limit: q.limit.unwrap_or(50) };
    Ok(Json(st.store.list_workflow_runs(&f).await?))
}

async fn run_detail(st: &AppState, id: &str) -> Result<RunDetail, ApiError> {
    let run = st.store.get_workflow_run(id).await?.ok_or_else(|| ApiError::not_found("workflow run"))?;
    let steps = st.store.list_steps(id).await?;
    let resume_from = pdt_core::workflow::agent::resumable_step(&steps)
        .filter(|_| run.parent_step_id.is_none() || !run.status.is_finished())
        .map(|s| s.id.clone());
    Ok(RunDetail { run, steps, resume_from })
}

pub(crate) async fn get_run(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<RunDetail>, ApiError> {
    Ok(Json(run_detail(&st, &id).await?))
}

pub(crate) async fn retry_run(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<RunDetail>, ApiError> {
    run_detail(&st, &id).await?;
    engine::retry_run(&st, &id).await.map_err(|e| conflict("cannot_retry", e))?;
    Ok(Json(run_detail(&st, &id).await?))
}

/// `POST /api/workflow-runs/{id}/resume`: runs the agent step it timed out or stopped at again (PDT-123).
pub(crate) async fn resume_run(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunDetail>, ApiError> {
    run_detail(&st, &id).await?;
    engine::resume_run(&st, &id, None).await.map_err(|e| conflict("cannot_resume", e))?;
    Ok(Json(run_detail(&st, &id).await?))
}

pub(crate) async fn cancel_run(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunDetail>, ApiError> {
    run_detail(&st, &id).await?;
    engine::cancel_run(&st, &id).await.map_err(|e| conflict("cannot_cancel", e))?;
    Ok(Json(run_detail(&st, &id).await?))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GateAnswer {
    pub choice: String,
    /// The user's note with the choice (e.g. why they rejected): it goes on the run's ticket as their comment.
    #[serde(default)]
    #[ts(optional)]
    pub text: Option<String>,
}

pub(crate) async fn answer(
    State(st): State<AppState>,
    Path((id, step)): Path<(String, String)>,
    payload: Result<Json<GateAnswer>, JsonRejection>,
) -> Result<Json<RunDetail>, ApiError> {
    let a = body(payload)?;
    run_detail(&st, &id).await?;
    // The gate's Inbox item, for `inbox.answered` (PDT-364; once per item, whichever way it was answered).
    let items = st.store.open_items_for_step(&step).await?;
    engine::answer(&st, &id, &step, &a.choice, a.text.as_deref()).await.map_err(|e| conflict("cannot_answer", e))?;
    for item in &items {
        crate::workflow::sensors::inbox_answered(&st, item, Some(&a.choice), a.text.as_deref(), "user").await;
    }
    Ok(Json(run_detail(&st, &id).await?))
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct EventsQuery {
    #[serde(rename = "type")]
    kind: Option<String>,
    project: Option<String>,
    ticket: Option<String>,
    attempt: Option<String>,
    after: Option<i64>,
    limit: Option<i64>,
}

/// `GET /api/events`: newest first, or oldest first after `after` (to follow the log).
pub(crate) async fn list_events(
    State(st): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<Vec<LogEvent>>, ApiError> {
    let f = pdt_store::EventFilter {
        newest_first: q.after.is_none(),
        kind: q.kind,
        project_id: q.project,
        ticket_id: q.ticket,
        attempt_id: q.attempt,
        after: q.after,
        since: None,
        limit: q.limit.unwrap_or(100),
    };
    Ok(Json(st.store.list_log_events(&f).await?))
}

pub(crate) async fn get_event(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<LogEvent>, ApiError> {
    Ok(Json(st.store.get_log_event(&id).await?.ok_or_else(|| ApiError::not_found("event"))?))
}

// ---- connections ----

fn secret_account(id: &str) -> String {
    format!("connection:{id}")
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionInput {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub url: Option<String>,
    #[ts(type = "Record<string, unknown> | null")]
    pub config: Option<Value>,
}

fn valid_name(name: &str) -> Result<String, ApiError> {
    let n = name.trim();
    if n.is_empty() || n.len() > 80 {
        return Err(invalid("a connection needs a name of at most 80 characters"));
    }
    Ok(n.to_string())
}

/// What a kind needs in its config, as its integration checks it (a bot: its server's id). A kind of an integration
/// that is turned off (or not built in) is refused.
fn check_config(st: &AppState, kind: &str, config: &Value) -> Result<(), ApiError> {
    let reg = st.registry();
    if let Some(i) = pdt_core::integration::owner_of_connection(kind)
        && !reg.connection_kinds().iter().any(|k| k.id == kind)
    {
        let why = if st.integrations.built().iter().any(|b| b.id() == i.id) {
            "which is turned off (Settings → Plugins)".to_string()
        } else {
            format!("which this {} was built without", pdt_core::brand::SHORT_NAME)
        };
        return Err(invalid(format!("{kind} connections are part of the {} integration, {why}", i.label)));
    }
    reg.check_connection_config(kind, config).map_err(invalid)
}

pub(crate) async fn list_connections(State(st): State<AppState>) -> Result<Json<Vec<Connection>>, ApiError> {
    Ok(Json(st.store.list_connections().await?))
}

pub async fn create_connection(
    State(st): State<AppState>,
    payload: Result<Json<ConnectionInput>, JsonRejection>,
) -> Result<(StatusCode, Json<Connection>), ApiError> {
    let c = body(payload)?;
    let now = now_ms();
    let config = c.config.unwrap_or(Value::Object(Default::default()));
    if !config.is_object() {
        return Err(invalid("config must be an object"));
    }
    let kind = c.kind.unwrap_or_else(|| "http".into()).trim().to_string();
    check_config(&st, &kind, &config)?;
    let conn = Connection {
        id: uuid::Uuid::now_v7().to_string(),
        name: valid_name(c.name.as_deref().unwrap_or_default())?,
        kind,
        url: c.url.unwrap_or_default().trim().to_string(),
        config,
        has_secret: false,
        created_at: now,
        updated_at: now,
        last_error: None,
        last_error_at: None,
    };
    st.store.insert_connection(&conn).await.map_err(|e| match e {
        pdt_store::StoreError::Conflict(_) => {
            conflict("connection_exists", format!("a connection is already called {}", conn.name))
        }
        other => other.into(),
    })?;
    crate::connection_health::forget(&st, &[&conn.name]).await;
    crate::mcp_client::connection_changed(&st, Some(&conn)).await;
    crate::local_network::prompt_soon(&st);
    Ok((StatusCode::CREATED, Json(conn)))
}

async fn load_connection(st: &AppState, id: &str) -> Result<Connection, ApiError> {
    st.store.get_connection(id).await?.ok_or_else(|| ApiError::not_found("connection"))
}

pub(crate) async fn get_connection(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Connection>, ApiError> {
    Ok(Json(load_connection(&st, &id).await?))
}

pub(crate) async fn patch_connection(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<ConnectionInput>, JsonRejection>,
) -> Result<Json<Connection>, ApiError> {
    let p = body(payload)?;
    let mut c = load_connection(&st, &id).await?;
    let old_name = c.name.clone();
    if let Some(n) = p.name {
        c.name = valid_name(&n)?;
    }
    if let Some(k) = p.kind {
        c.kind = k.trim().to_string();
    }
    if let Some(u) = p.url {
        c.url = u.trim().to_string();
    }
    if let Some(cfg) = p.config {
        if !cfg.is_object() {
            return Err(invalid("config must be an object"));
        }
        c.config = cfg;
    }
    check_config(&st, &c.kind, &c.config)?;
    c.updated_at = now_ms();
    st.store.update_connection(&c).await.map_err(|e| match e {
        pdt_store::StoreError::Conflict(_) => {
            conflict("connection_exists", format!("a connection is already called {}", c.name))
        }
        other => other.into(),
    })?;
    (c.last_error, c.last_error_at) = (None, None);
    crate::connection_health::forget(&st, &[&old_name, &c.name]).await;
    crate::mcp_client::connection_changed(&st, Some(&c)).await;
    crate::local_network::prompt_soon(&st);
    Ok(Json(c))
}

pub(crate) async fn delete_connection(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let c = load_connection(&st, &id).await?;
    // Its integration forgets what it keeps for it (Google's sign-in), turned on or not.
    for int in st.integrations.built() {
        if int.connection_kinds().iter().any(|k| k.id == c.kind) {
            int.connection_deleted(&st, &c).await;
        }
    }
    let secrets = st.secrets.clone();
    let account = secret_account(&id);
    tokio::task::spawn_blocking(move || secrets.delete(&account))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "keychain_failed", e.to_string()))?;
    st.store.delete_connection(&id).await?;
    if c.kind == crate::mcp_client::MCP {
        crate::mcp_oauth::forget(&st, &id).await;
    }
    crate::connection_health::forget(&st, &[&c.name]).await;
    crate::mcp_client::connection_changed(&st, None).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionSecret {
    pub secret: String,
}

/// `PUT /api/connections/{id}/secret`: kept in the OS keychain, never returned.
pub async fn put_secret(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<ConnectionSecret>, JsonRejection>,
) -> Result<Json<Connection>, ApiError> {
    let s = body(payload)?;
    if s.secret.is_empty() {
        return Err(invalid("the secret is empty"));
    }
    let mut c = load_connection(&st, &id).await?;
    let secrets = st.secrets.clone();
    let account = secret_account(&id);
    tokio::task::spawn_blocking(move || secrets.set(&account, &s.secret))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "keychain_failed", e.to_string()))?;
    c.has_secret = true;
    c.updated_at = now_ms();
    st.store.update_connection(&c).await?;
    (c.last_error, c.last_error_at) = (None, None);
    crate::connection_health::forget(&st, &[&c.name]).await;
    crate::mcp_client::connection_changed(&st, Some(&c)).await;
    Ok(Json(c))
}

pub(crate) async fn delete_secret(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Connection>, ApiError> {
    let mut c = load_connection(&st, &id).await?;
    let secrets = st.secrets.clone();
    let account = secret_account(&id);
    tokio::task::spawn_blocking(move || secrets.delete(&account))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "keychain_failed", e.to_string()))?;
    c.has_secret = false;
    c.updated_at = now_ms();
    st.store.update_connection(&c).await?;
    (c.last_error, c.last_error_at) = (None, None);
    crate::connection_health::forget(&st, &[&c.name]).await;
    crate::mcp_client::connection_changed(&st, Some(&c)).await;
    Ok(Json(c))
}

/// `GET /api/connections/{id}/tools`: an `mcp` connection's tools as last listed (null before the first listing).
pub(crate) async fn connection_tools(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<crate::mcp_client::McpTools>>, ApiError> {
    let c = mcp_connection(&st, &id).await?;
    Ok(Json(st.mcp.cached(&c.name)))
}

/// `POST /api/connections/{id}/tools`: lists an `mcp` connection's tools now (starting its server if need be).
pub(crate) async fn refresh_connection_tools(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::mcp_client::McpTools>, ApiError> {
    let c = mcp_connection(&st, &id).await?;
    Ok(Json(st.mcp.refresh(&st, &c.name).await))
}

async fn mcp_connection(st: &AppState, id: &str) -> Result<Connection, ApiError> {
    let c = load_connection(st, id).await?;
    if c.kind != crate::mcp_client::MCP {
        return Err(conflict("not_mcp", format!("{} is a {} connection, not an MCP server", c.name, c.kind)));
    }
    Ok(c)
}

/// The connection of `kind` that `name` means: that one, or the only one of its kind there is (the PM's tools).
pub async fn connection_of_kind(st: &AppState, kind: &str, name: Option<&str>) -> Result<Connection, String> {
    let label = st.registry().connection_label(kind);
    let all: Vec<Connection> =
        st.store.list_connections().await.map_err(|e| e.to_string())?.into_iter().filter(|c| c.kind == kind).collect();
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => {
            let c = all.into_iter().find(|c| c.name.eq_ignore_ascii_case(n));
            c.ok_or_else(|| format!("no {label} connection is called {n}"))
        }
        None => match <[Connection; 1]>::try_from(all) {
            Ok([one]) => Ok(one),
            Err(all) if all.is_empty() => {
                Err(format!("there is no {label} connection: the user adds one in Settings → Plugins"))
            }
            Err(all) => Err(format!(
                "there are several {label} connections; name one (`connection`): {}",
                all.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
            )),
        },
    }
}

/// A connection and its secret by name, for node kinds that call external services (7e).
pub async fn connection_with_secret(st: &AppState, name: &str) -> Result<(Connection, Option<String>), String> {
    let c = st
        .store
        .connection_by_name(name)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no connection is called {name}"))?;
    let secrets = st.secrets.clone();
    let account = secret_account(&c.id);
    let what = format!("the secret of the connection {}", c.name);
    let secret = tokio::task::spawn_blocking(move || secrets.get(&account).map_err(|e| e.naming(&account, &what)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok((c, secret))
}

pub(crate) fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post, put};
    axum::Router::new()
        .route("/workflows", get(list).post(create))
        .route("/workflows/catalog", get(get_catalog))
        .route("/workflows/validate", post(validate_doc))
        .route("/workflows/check", post(check_doc))
        .route("/workflows/dry-run", post(dry_run))
        .route("/workflows/export", post(super::transfer::export_many))
        .route("/workflows/import", post(super::transfer::import_route))
        .route("/workflows/{id}/export", get(super::transfer::export_one))
        .route("/workflows/{id}", get(get_one).put(save).patch(patch).delete(remove))
        .route("/workflows/{id}/versions", get(versions))
        .route("/workflows/{id}/versions/{v}", get(version))
        .route("/workflows/{id}/versions/{v}/accept", post(accept))
        .route("/workflows/{id}/versions/{v}/discard", post(discard))
        .route("/workflow-templates", get(super::templates::list).post(super::templates::create))
        .route(
            "/workflow-templates/{id}",
            get(super::templates::get_one).put(super::templates::save).delete(super::templates::remove),
        )
        .route("/workflow-templates/{id}/instantiate", post(super::templates::instantiate))
        .route("/workflows/{id}/diff", get(version_diff))
        .route("/workflows/{id}/state", get(state))
        .route("/workflows/{id}/run", post(run_now))
        .route("/workflow-runs", get(list_runs))
        .route("/workflow-runs/{id}", get(get_run))
        .route("/workflow-runs/{id}/retry", post(retry_run))
        .route("/workflow-runs/{id}/resume", post(resume_run))
        .route("/workflow-runs/{id}/cancel", post(cancel_run))
        .route("/workflow-runs/{id}/steps/{step}/answer", post(answer))
        .route("/events", get(list_events))
        .route("/events/{id}", get(get_event))
        .route("/connections", get(list_connections).post(create_connection))
        .route("/connections/kinds", get(crate::integrations::connection_kinds))
        .route("/integrations/project-settings", get(crate::integrations::project_settings))
        .route("/integrations", get(crate::integrations::list).put(crate::integrations::put))
        .route("/integrations/{id}/check", post(crate::integrations::check))
        .route("/connections/{id}", get(get_connection).patch(patch_connection).delete(delete_connection))
        .route("/connections/{id}/secret", put(put_secret).delete(delete_secret))
        .route("/connections/{id}/status", get(crate::integrations::connection_status))
        .route("/projects/{id}/import/{integration}", post(crate::integrations::import_by_key))
        .route("/connections/{id}/tools", get(connection_tools).post(refresh_connection_tools))
}
