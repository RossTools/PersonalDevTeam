//! The documents API (PDT-314): `/api/documents…` (list, filter, create, import, rename, link, delete, content,
//! comments), `/api/tickets/{id}/design-doc` and `/api/projects/{id}/document-settings`.

use super::{DocumentProvider, NewFile, ProviderError, provider, providers};
use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::document::{
    CommentReply, Document, DocumentComment, DocumentContent, DocumentEdit, DocumentFilter, DocumentKind,
    ProjectDocumentSettings, ProviderFile, ProviderInfo, clean_title, design_doc, documents_folder, effective_provider,
};
use pdt_core::ticket::{Actor, By};
use pdt_store::{AddDocument, DocumentLink};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

fn invalid(m: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", m)
}

impl From<ProviderError> for ApiError {
    fn from(e: ProviderError) -> Self {
        match e {
            ProviderError::Unsupported(m) => ApiError::new(StatusCode::BAD_REQUEST, "unsupported", m),
            ProviderError::Invalid(m) => invalid(m),
            ProviderError::NotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "document_missing",
                "the provider no longer has this document (deleted, or access was taken away)",
            ),
            ProviderError::Failed(m) => ApiError::new(StatusCode::BAD_GATEWAY, "provider_failed", m),
        }
    }
}

/// `by` as a request gives it: `user` from PDT's UI; anything else is an agent.
fn by_of(given: Option<Actor>) -> By {
    By::from(if given == Some(Actor::User) { Actor::User } else { Actor::Agent })
}

fn need(p: &Option<Arc<dyn DocumentProvider>>, id: &str) -> Result<Arc<dyn DocumentProvider>, ApiError> {
    p.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "no_provider",
            format!("{id} isn't a document provider this host has (is its plugin on?)"),
        )
    })
}

/// A project's id from its id or key.
async fn project_id(st: &AppState, given: &str) -> Result<String, ApiError> {
    if st.store.get_project(given).await?.is_some() {
        return Ok(given.to_string());
    }
    st.store
        .list_projects()
        .await?
        .into_iter()
        .find(|p| p.key.eq_ignore_ascii_case(given.trim()))
        .map(|p| p.id)
        .ok_or_else(|| ApiError::not_found("project"))
}

/// A ticket's id from its id or key.
async fn ticket_id(st: &AppState, given: &str) -> Result<String, ApiError> {
    if st.store.get_ticket_summary(given).await?.is_some() {
        return Ok(given.to_string());
    }
    st.store.ticket_id_by_key(given).await?.ok_or_else(|| ApiError::not_found("ticket"))
}

pub(crate) async fn document(st: &AppState, id: &str) -> Result<Document, ApiError> {
    st.store.get_document(id).await?.ok_or_else(|| ApiError::not_found("document"))
}

/// The provider of a document PDT has.
pub(crate) fn provider_of(st: &AppState, d: &Document) -> Result<Arc<dyn DocumentProvider>, ApiError> {
    need(&provider(st, &d.provider), &d.provider)
}

/// The provider a new document in `project` goes to (its default, else the host's, else the first PDT has).
pub(crate) async fn default_provider(st: &AppState, project: Option<&str>) -> Result<Option<String>, ApiError> {
    let host = st.store.get_host_settings().await?.documents_provider;
    let own = match project {
        Some(p) => st.store.project_document_settings(p).await?.provider,
        None => None,
    };
    let all = providers(st);
    let ids: Vec<&str> = all.iter().map(|p| p.id()).collect();
    Ok(effective_provider(own.as_deref(), host.as_deref(), &ids))
}

/// Where local documents of `project` go.
pub(crate) async fn folder(st: &AppState, project: Option<&str>) -> Result<std::path::PathBuf, ApiError> {
    let host = st.store.get_host_settings().await?.documents_dir;
    let own = match project {
        Some(p) => st.store.project_document_settings(p).await?.folder,
        None => None,
    };
    Ok(documents_folder(own.as_deref(), host.as_deref(), &st.data_dir, st.home_dir.as_deref()))
}

/// The document providers this host has, and where new documents go.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentProvidersView {
    pub providers: Vec<ProviderInfo>,
    /// The provider new documents go to (the project's, else the host's default, else the first); none when PDT
    /// has no provider.
    pub default: Option<String>,
    /// Where local documents go (the project's folder, else the host's, else PDT's data folder).
    pub folder: String,
}

#[derive(Deserialize)]
pub(crate) struct ProvidersQuery {
    #[serde(default)]
    project: Option<String>,
}

/// `GET /api/documents/providers[?project=]`.
pub(crate) async fn list_providers(
    State(st): State<AppState>,
    Query(q): Query<ProvidersQuery>,
) -> Result<Json<DocumentProvidersView>, ApiError> {
    let project = match q.project.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(p) => Some(project_id(&st, p).await?),
        None => None,
    };
    Ok(Json(DocumentProvidersView {
        providers: providers(&st).iter().map(|p| p.info()).collect(),
        default: default_provider(&st, project.as_deref()).await?,
        folder: folder(&st, project.as_deref()).await?.to_string_lossy().into_owned(),
    }))
}

#[derive(Deserialize)]
pub(crate) struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    kind: Option<DocumentKind>,
}

/// `GET /api/documents/providers/{id}/files?q=&kind=`: the provider's files to import (its picker).
pub(crate) async fn search(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<ProviderFile>>, ApiError> {
    let p = need(&provider(&st, &id), &id)?;
    Ok(Json(p.search(q.q.trim(), q.kind).await?))
}

/// `GET /api/documents?project=&ticket=&kind=&provider=&q=`: the most recently changed first. `project` and
/// `ticket` take an id or a key.
pub(crate) async fn list(
    State(st): State<AppState>,
    Query(mut f): Query<DocumentFilter>,
) -> Result<Json<Vec<Document>>, ApiError> {
    if let Some(p) = f.project.take().filter(|p| !p.trim().is_empty()) {
        f.project = Some(project_id(&st, &p).await?);
    }
    if let Some(t) = f.ticket.take().filter(|t| !t.trim().is_empty()) {
        f.ticket = Some(ticket_id(&st, &t).await?);
    }
    f.provider = f.provider.filter(|p| !p.trim().is_empty());
    Ok(Json(st.store.list_documents(&f).await?))
}

/// `GET /api/documents/{id}`.
pub(crate) async fn get(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Document>, ApiError> {
    Ok(Json(document(&st, &id).await?))
}

/// Records a provider's file as a document (or finds PDT's record of it), linked to `project` and `ticket`.
async fn record(
    st: &AppState,
    provider: &str,
    file: ProviderFile,
    project: Option<&str>,
    ticket: Option<&str>,
    by: &By,
) -> Result<(bool, Document), ApiError> {
    let now = now_ms();
    let d = Document {
        id: uuid::Uuid::now_v7().to_string(),
        title: clean_title(&file.title).unwrap_or_else(|_| "Untitled".into()),
        kind: file.kind,
        provider: provider.to_string(),
        provider_ref: file.reference,
        url: file.url,
        actor: if by.actor == Actor::User { Actor::User } else { Actor::Agent },
        agent: by.agent.clone(),
        created_at: now,
        updated_at: now,
        projects: vec![],
        tickets: vec![],
    };
    let (added, id) = match st.store.add_document(&d).await? {
        AddDocument::Added => (true, d.id),
        AddDocument::Exists(id) => (false, id),
    };
    for link in project
        .map(|p| DocumentLink::Project(p.into()))
        .into_iter()
        .chain(ticket.map(|t| DocumentLink::Ticket(t.into())))
    {
        st.store.link_document(&id, &link, now).await?;
    }
    Ok((added, document(st, &id).await?))
}

/// What a new document needs: the ticket's and project's ids (a ticket brings its project).
async fn targets(
    st: &AppState,
    project: Option<&str>,
    ticket: Option<&str>,
) -> Result<(Option<String>, Option<String>), ApiError> {
    let ticket = match ticket.filter(|t| !t.trim().is_empty()) {
        Some(t) => Some(ticket_id(st, t).await?),
        None => None,
    };
    let project = match project.filter(|p| !p.trim().is_empty()) {
        Some(p) => Some(project_id(st, p).await?),
        None => match &ticket {
            Some(t) => st.store.get_ticket_summary(t).await?.map(|s| s.project_id),
            None => None,
        },
    };
    Ok((project, ticket))
}

/// Makes a document with the provider and records it, linked.
pub(crate) async fn create_document(st: &AppState, new: NewDocument, by: &By) -> Result<Document, ApiError> {
    let title = clean_title(&new.title).map_err(invalid)?;
    let (project, ticket) = targets(st, new.project_id.as_deref(), new.ticket_id.as_deref()).await?;
    let id = match new.provider.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => default_provider(st, project.as_deref()).await?.ok_or_else(|| {
            ApiError::new(StatusCode::BAD_REQUEST, "no_provider", "this host has no document provider yet")
        })?,
    };
    let p = need(&provider(st, &id), &id)?;
    let caps = p.capabilities();
    if !caps.create {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported",
            format!("{} can't make documents", p.label()),
        ));
    }
    if (new.kind == DocumentKind::Doc && !caps.docs) || (new.kind == DocumentKind::Sheet && !caps.sheets) {
        let what = if new.kind == DocumentKind::Doc { "text documents" } else { "spreadsheets" };
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "unsupported", format!("{} has no {what}", p.label())));
    }
    let file = p
        .create(NewFile {
            title,
            kind: new.kind,
            markdown: new.markdown.filter(|_| new.kind == DocumentKind::Doc),
            folder: folder(st, project.as_deref()).await?,
        })
        .await?;
    Ok(record(st, &id, file, project.as_deref(), ticket.as_deref(), by).await?.1)
}

/// A document to make (`POST /api/documents`).
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewDocument {
    pub title: String,
    pub kind: DocumentKind,
    /// None: the project's default provider, else the host's.
    #[serde(default)]
    #[ts(optional)]
    pub provider: Option<String>,
    /// Links it to this project (id or key).
    #[serde(default)]
    #[ts(optional)]
    pub project_id: Option<String>,
    /// Links it to this ticket (id or key), and the ticket's project.
    #[serde(default)]
    #[ts(optional)]
    pub ticket_id: Option<String>,
    /// A text document's first content.
    #[serde(default)]
    #[ts(optional)]
    pub markdown: Option<String>,
    /// `user` from PDT's UI.
    #[serde(default)]
    #[ts(optional)]
    pub by: Option<Actor>,
}

/// `POST /api/documents`.
pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewDocument>, JsonRejection>,
) -> Result<(StatusCode, Json<Document>), ApiError> {
    let Json(mut new) = payload?;
    let by = by_of(new.by.take());
    Ok((StatusCode::CREATED, Json(create_document(&st, new, &by).await?)))
}

/// A provider's file to import or link (`POST /api/documents/import`).
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportDocument {
    pub provider: String,
    /// The provider's reference (from its picker: `GET /api/documents/providers/{id}/files`).
    pub reference: String,
    #[serde(default)]
    #[ts(optional)]
    pub project_id: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub ticket_id: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub by: Option<Actor>,
}

/// `POST /api/documents/import`: records a file the provider has (201), or links the document PDT has for it
/// already (200).
pub(crate) async fn import(
    State(st): State<AppState>,
    payload: Result<Json<ImportDocument>, JsonRejection>,
) -> Result<(StatusCode, Json<Document>), ApiError> {
    let Json(i) = payload?;
    let p = need(&provider(&st, i.provider.trim()), &i.provider)?;
    let (project, ticket) = targets(&st, i.project_id.as_deref(), i.ticket_id.as_deref()).await?;
    let file = p.file(i.reference.trim()).await?;
    let (added, d) = record(&st, p.id(), file, project.as_deref(), ticket.as_deref(), &by_of(i.by)).await?;
    Ok((if added { StatusCode::CREATED } else { StatusCode::OK }, Json(d)))
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentPatch {
    pub title: String,
}

/// `PATCH /api/documents/{id}`: renames it (where it is kept too, when the provider can).
pub(crate) async fn rename(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<DocumentPatch>, JsonRejection>,
) -> Result<Json<Document>, ApiError> {
    let Json(patch) = payload?;
    let title = clean_title(&patch.title).map_err(invalid)?;
    let d = document(&st, &id).await?;
    if let Some(p) = provider(&st, &d.provider)
        && p.capabilities().rename
    {
        p.rename(&d.provider_ref, &title).await?;
    }
    st.store.rename_document(&id, &title, now_ms()).await?;
    Ok(Json(document(&st, &id).await?))
}

#[derive(Deserialize)]
pub(crate) struct DeleteQuery {
    /// Delete the file where it is kept too; otherwise PDT only forgets it.
    #[serde(default)]
    file: bool,
}

/// `DELETE /api/documents/{id}[?file=true]`: forgets the document (and its links); with `file`, deletes it where it
/// is kept too.
pub(crate) async fn remove(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DeleteQuery>,
) -> Result<StatusCode, ApiError> {
    let d = document(&st, &id).await?;
    if q.file {
        let p = provider_of(&st, &d)?;
        if !p.capabilities().delete {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "unsupported",
                format!("{} can't delete documents; remove it from PDT only", p.label()),
            ));
        }
        match p.delete(&d.provider_ref).await {
            Ok(()) | Err(ProviderError::NotFound) => {}
            Err(e) => return Err(e.into()),
        }
    }
    st.store.delete_document(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A link to add or remove: a project or a ticket (id or key).
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentLinkInput {
    #[serde(default)]
    #[ts(optional)]
    pub project_id: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub ticket_id: Option<String>,
}

pub(crate) async fn link_of(st: &AppState, l: &DocumentLinkInput) -> Result<DocumentLink, ApiError> {
    match (l.project_id.as_deref(), l.ticket_id.as_deref()) {
        (Some(p), None) => Ok(DocumentLink::Project(project_id(st, p).await?)),
        (None, Some(t)) => Ok(DocumentLink::Ticket(ticket_id(st, t).await?)),
        _ => Err(invalid("give a projectId or a ticketId")),
    }
}

/// `POST /api/documents/{id}/links`.
pub(crate) async fn link(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<DocumentLinkInput>, JsonRejection>,
) -> Result<Json<Document>, ApiError> {
    let Json(l) = payload?;
    let link = link_of(&st, &l).await?;
    if !st.store.link_document(&id, &link, now_ms()).await? {
        return Err(ApiError::not_found("document"));
    }
    Ok(Json(document(&st, &id).await?))
}

/// `DELETE /api/documents/{id}/links`.
pub(crate) async fn unlink(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<DocumentLinkInput>, JsonRejection>,
) -> Result<Json<Document>, ApiError> {
    let Json(l) = payload?;
    document(&st, &id).await?;
    let link = link_of(&st, &l).await?;
    st.store.unlink_document(&id, &link).await?;
    Ok(Json(document(&st, &id).await?))
}

/// A document's content, with where it opens inside PDT when the provider has such a page.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentRead {
    pub content: DocumentContent,
    pub embed_url: Option<String>,
}

/// `GET /api/documents/{id}/content`: a doc as Markdown, a sheet as its sheets of cells.
pub(crate) async fn content(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DocumentRead>, ApiError> {
    let d = document(&st, &id).await?;
    let p = provider_of(&st, &d)?;
    let content = p.read(&d.provider_ref).await?;
    let embed_url = if p.capabilities().embed { p.embed_url(&d.provider_ref) } else { None };
    Ok(Json(DocumentRead { content, embed_url }))
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentEdits {
    pub edits: Vec<DocumentEdit>,
}

/// The new version after edits.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditResult {
    pub version: Option<String>,
    /// The ids of the charts the edits added, in order.
    pub charts: Vec<String>,
}

/// Applies edits to a document at its provider, in order: new charts get an id when they have none.
pub(crate) async fn apply_edits(
    st: &AppState,
    d: &Document,
    mut edits: Vec<DocumentEdit>,
) -> Result<EditResult, ApiError> {
    let p = provider_of(st, d)?;
    if !p.capabilities().edit {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported",
            format!("{} can't change documents", p.label()),
        ));
    }
    if edits.is_empty() {
        return Err(invalid("give at least one edit"));
    }
    let mut charts = Vec::new();
    for e in &mut edits {
        if let DocumentEdit::AddChart { id, .. } = e {
            let given = id.take().map(|i| i.trim().to_string()).filter(|i| !i.is_empty());
            let new = given.unwrap_or_else(|| format!("chart-{}", &uuid::Uuid::now_v7().simple().to_string()[20..]));
            charts.push(new.clone());
            *id = Some(new);
        }
    }
    let version = p.apply(&d.provider_ref, edits).await?;
    st.store.touch_document(&d.id, now_ms()).await?;
    Ok(EditResult { version, charts })
}

/// `POST /api/documents/{id}/edits`: changes its content, in order.
pub(crate) async fn edit(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<DocumentEdits>, JsonRejection>,
) -> Result<Json<EditResult>, ApiError> {
    let Json(e) = payload?;
    let d = document(&st, &id).await?;
    Ok(Json(apply_edits(&st, &d, e.edits).await?))
}

pub(crate) async fn with_comments(st: &AppState, id: &str) -> Result<(Document, Arc<dyn DocumentProvider>), ApiError> {
    let d = document(st, id).await?;
    let p = provider_of(st, &d)?;
    if !p.capabilities().comments {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "unsupported", format!("{} has no comments", p.label())));
    }
    Ok((d, p))
}

/// `GET /api/documents/{id}/comments`.
pub(crate) async fn comments(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<DocumentComment>>, ApiError> {
    let (d, p) = with_comments(&st, &id).await?;
    Ok(Json(p.comments(&d.provider_ref).await?))
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommentReplyInput {
    pub body: String,
}

/// `POST /api/documents/{id}/comments/{comment}/replies`.
pub(crate) async fn reply(
    State(st): State<AppState>,
    Path((id, comment)): Path<(String, String)>,
    payload: Result<Json<CommentReplyInput>, JsonRejection>,
) -> Result<(StatusCode, Json<CommentReply>), ApiError> {
    let Json(r) = payload?;
    let body = r.body.trim();
    if body.is_empty() {
        return Err(invalid("a reply needs some text"));
    }
    let (d, p) = with_comments(&st, &id).await?;
    Ok((StatusCode::CREATED, Json(p.reply(&d.provider_ref, &comment, body).await?)))
}

/// `POST /api/documents/{id}/comments/{comment}/resolve`.
pub(crate) async fn resolve(
    State(st): State<AppState>,
    Path((id, comment)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let (d, p) = with_comments(&st, &id).await?;
    p.resolve(&d.provider_ref, &comment).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewDesignDoc {
    /// None: the project's default provider.
    #[serde(default)]
    #[ts(optional)]
    pub provider: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub by: Option<Actor>,
}

/// Makes a ticket's design doc: named after it, from the template, in its project's default provider, linked to the
/// ticket and its project.
pub(crate) async fn create_design_doc(
    st: &AppState,
    ticket: &str,
    provider: Option<String>,
    by: &By,
) -> Result<Document, ApiError> {
    let id = ticket_id(st, ticket).await?;
    let t = st.store.get_ticket_summary(&id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
    let (title, markdown) = design_doc(&t.key, &t.title);
    let new = NewDocument {
        title,
        kind: DocumentKind::Doc,
        provider,
        project_id: Some(t.project_id),
        ticket_id: Some(id),
        markdown: Some(markdown),
        by: None,
    };
    create_document(st, new, by).await
}

/// `POST /api/tickets/{id}/design-doc`.
pub(crate) async fn design(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<Document>), ApiError> {
    let input: NewDesignDoc = if body.iter().all(u8::is_ascii_whitespace) {
        NewDesignDoc::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| invalid(e.to_string()))?
    };
    let d = create_design_doc(&st, &id, input.provider, &by_of(input.by)).await?;
    Ok((StatusCode::CREATED, Json(d)))
}

/// `GET /api/projects/{id}/document-settings`.
pub(crate) async fn get_settings(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectDocumentSettings>, ApiError> {
    let id = project_id(&st, &id).await?;
    Ok(Json(st.store.project_document_settings(&id).await?))
}

/// `PUT /api/projects/{id}/document-settings`: blank clears a field (back to the host's).
pub(crate) async fn put_settings(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<ProjectDocumentSettings>, JsonRejection>,
) -> Result<Json<ProjectDocumentSettings>, ApiError> {
    let Json(s) = payload?;
    let id = project_id(&st, &id).await?;
    let s = s.cleaned().map_err(invalid)?;
    st.store.put_project_document_settings(&id, &s).await?;
    Ok(Json(s))
}

/// A ticket's documents for `get_ticket`: title, kind, provider and id, and where to read each.
pub(crate) async fn for_agent(st: &AppState, ticket: &str) -> Result<Vec<serde_json::Value>, ApiError> {
    let f = DocumentFilter { ticket: Some(ticket.to_string()), ..Default::default() };
    Ok(st
        .store
        .list_documents(&f)
        .await?
        .into_iter()
        .map(|d| {
            serde_json::json!({"id": d.id, "title": d.title, "kind": d.kind, "provider": d.provider, "url": d.url,
                               "read": format!("/api/documents/{}/content", d.id)})
        })
        .collect())
}

/// A conversation's session, which must be an agent's.
async fn beside_session(st: &AppState, id: &str) -> Result<(), ApiError> {
    match st.store.get_session(id).await? {
        Some(s) if s.kind == pdt_core::session::SessionKind::AgentTui => Ok(()),
        Some(_) => Err(invalid("only an agent or PM conversation has documents beside it")),
        None => Err(ApiError::not_found("session")),
    }
}

/// `GET /api/sessions/{id}/documents`: the documents open beside a conversation, in tab order, and the one shown.
pub(crate) async fn beside(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<pdt_core::document::DocumentsBeside>, ApiError> {
    beside_session(&st, &id).await?;
    Ok(Json(st.store.documents_beside(&id).await?))
}

/// A document to open beside a conversation (`POST /api/sessions/{id}/documents`).
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenBeside {
    pub document_id: String,
    /// `user` from PDT's UI.
    #[serde(default)]
    #[ts(optional)]
    pub by: Option<Actor>,
}

/// Opens a document beside a conversation (or shows its tab there).
pub(crate) async fn open_beside_doc(
    st: &AppState,
    session: &str,
    document_id: &str,
    by: Actor,
) -> Result<pdt_core::document::DocumentsBeside, ApiError> {
    beside_session(st, session).await?;
    document(st, document_id).await?;
    st.store.open_beside(session, document_id, by, now_ms()).await?;
    Ok(st.store.documents_beside(session).await?)
}

/// `POST /api/sessions/{id}/documents`.
pub(crate) async fn open_beside(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<OpenBeside>, JsonRejection>,
) -> Result<Json<pdt_core::document::DocumentsBeside>, ApiError> {
    let Json(o) = payload?;
    let by = if o.by == Some(Actor::User) { Actor::User } else { Actor::Agent };
    Ok(Json(open_beside_doc(&st, &id, o.document_id.trim(), by).await?))
}

/// The tabs' new order (`PUT /api/sessions/{id}/documents`).
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BesideOrder {
    pub document_ids: Vec<String>,
}

/// `PUT /api/sessions/{id}/documents`: puts the tabs in this order.
pub(crate) async fn order_beside(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<BesideOrder>, JsonRejection>,
) -> Result<Json<pdt_core::document::DocumentsBeside>, ApiError> {
    let Json(o) = payload?;
    beside_session(&st, &id).await?;
    st.store.order_beside(&id, &o.document_ids).await?;
    Ok(Json(st.store.documents_beside(&id).await?))
}

/// `DELETE /api/sessions/{id}/documents/{doc}`: closes its tab.
pub(crate) async fn close_beside(
    State(st): State<AppState>,
    Path((id, doc)): Path<(String, String)>,
) -> Result<Json<pdt_core::document::DocumentsBeside>, ApiError> {
    beside_session(&st, &id).await?;
    st.store.close_beside(&id, &doc).await?;
    Ok(Json(st.store.documents_beside(&id).await?))
}
