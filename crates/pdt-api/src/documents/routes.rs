//! The documents API (PDT-314): `/api/documents…` (list, filter, create, import, rename, link, delete, content,
//! comments), `/api/tickets/{id}/design-doc` and `/api/projects/{id}/document-settings`.

use super::local::{self, DiffsApplied, DocEvent, LocalProvider};
use super::{CommentInput, DocumentProvider, NewFile, ProviderError, provider, providers};
use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::document::{
    ChartView, CommentReply, Document, DocumentComment, DocumentCommentInput, DocumentContent, DocumentEdit,
    DocumentFilter, DocumentKind, ProjectDocumentSettings, ProviderFile, ProviderInfo, clean_title, design_doc,
    documents_folder, effective_provider,
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

async fn document(st: &AppState, id: &str) -> Result<Document, ApiError> {
    st.store.get_document(id).await?.ok_or_else(|| ApiError::not_found("document"))
}

/// The provider of a document PDT has.
fn provider_of(st: &AppState, d: &Document) -> Result<Arc<dyn DocumentProvider>, ApiError> {
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

async fn link_of(st: &AppState, l: &DocumentLinkInput) -> Result<DocumentLink, ApiError> {
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
    /// The version the edits were made on (a doc's editor sends it): when the document has changed since, nothing
    /// is changed and the answer is a `conflict`.
    #[serde(default)]
    #[ts(optional)]
    pub base_version: Option<String>,
}

/// The new version after edits.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditResult {
    pub version: Option<String>,
}

/// `POST /api/documents/{id}/edits`: changes its content, in order.
pub(crate) async fn edit(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<DocumentEdits>, JsonRejection>,
) -> Result<Json<EditResult>, ApiError> {
    let Json(e) = payload?;
    let d = document(&st, &id).await?;
    let p = provider_of(&st, &d)?;
    if !p.capabilities().edit {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported",
            format!("{} can't change documents", p.label()),
        ));
    }
    if let Some(base) = e.base_version.as_deref().filter(|b| !b.is_empty()) {
        let now = p.version(&d.provider_ref).await?;
        if now.as_deref().is_some_and(|v| v != base) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "conflict",
                "the document changed since you opened it (someone else, or an agent, saved it)",
            ));
        }
    }
    let version = p.apply(&d.provider_ref, e.edits).await?;
    st.store.touch_document(&id, now_ms()).await?;
    Ok(Json(EditResult { version }))
}

async fn with_comments(st: &AppState, id: &str) -> Result<(Document, Arc<dyn DocumentProvider>), ApiError> {
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
    /// `user` from PDT's UI.
    #[serde(default)]
    #[ts(optional)]
    pub by: Option<Actor>,
}

/// Who wrote a comment, and the name it shows.
fn author(by: Option<Actor>) -> (Actor, String) {
    if by == Some(Actor::User) { (Actor::User, "You".into()) } else { (Actor::Agent, "Agent".into()) }
}

/// The longest comment kept (characters).
const MAX_COMMENT: usize = 20_000;

fn comment_body(body: &str) -> Result<String, ApiError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(invalid("a comment needs some text"));
    }
    Ok(body.chars().take(MAX_COMMENT).collect())
}

/// `POST /api/documents/{id}/comments`: a comment, on a heading, some text, or a sheet's cell or range.
pub(crate) async fn add_comment(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<DocumentCommentInput>, JsonRejection>,
) -> Result<(StatusCode, Json<DocumentComment>), ApiError> {
    let Json(c) = payload?;
    let body = comment_body(&c.body)?;
    let (d, p) = with_comments(&st, &id).await?;
    let (actor, author) = author(c.by);
    let c = p.comment(&d.provider_ref, CommentInput { body, anchor: c.anchor, author, actor }).await?;
    Ok((StatusCode::CREATED, Json(c)))
}

/// `POST /api/documents/{id}/comments/{comment}/replies`.
pub(crate) async fn reply(
    State(st): State<AppState>,
    Path((id, comment)): Path<(String, String)>,
    payload: Result<Json<CommentReplyInput>, JsonRejection>,
) -> Result<(StatusCode, Json<CommentReply>), ApiError> {
    let Json(r) = payload?;
    let body = comment_body(&r.body).map_err(|_| invalid("a reply needs some text"))?;
    let (d, p) = with_comments(&st, &id).await?;
    Ok((StatusCode::CREATED, Json(p.reply(&d.provider_ref, &comment, &body, author(r.by)).await?)))
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

/// `POST /api/documents/{id}/comments/{comment}/reopen`.
pub(crate) async fn reopen(
    State(st): State<AppState>,
    Path((id, comment)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let (d, p) = with_comments(&st, &id).await?;
    p.reopen(&d.provider_ref, &comment).await?;
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

// ---- Local documents (PDT-315): live sheets, the event stream, charts, export, import, images ----

/// A local document and PDT's local provider.
async fn local_doc(st: &AppState, id: &str) -> Result<(Document, Arc<LocalProvider>), ApiError> {
    let d = document(st, id).await?;
    if d.provider != local::ID {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "unsupported", "only PDT's local documents can do this"));
    }
    Ok((d, st.local.clone()))
}

fn bytes_response(bytes: Vec<u8>, mime: &str, extra: &[(&str, String)]) -> axum::response::Response {
    let mut b = axum::response::Response::builder().header(axum::http::header::CONTENT_TYPE, mime);
    for (k, v) in extra {
        b = b.header(*k, v.as_str());
    }
    b.body(axum::body::Body::from(bytes)).unwrap_or_default()
}

/// `GET /api/documents/{id}/sheet`: a local sheet's live model as IronCalc's bytes (`Model.fromBytes`); the number of
/// its last change is in `x-sheet-seq`.
pub(crate) async fn sheet(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    let (d, local) = local_doc(&st, &id).await?;
    let (bytes, seq) = local.sheet_bytes(&d.provider_ref)?;
    Ok(bytes_response(
        bytes,
        "application/octet-stream",
        &[("x-sheet-seq", seq.to_string()), ("cache-control", "no-store".into())],
    ))
}

#[derive(Deserialize)]
pub(crate) struct DiffsQuery {
    /// The view that sent them (its stream doesn't get them back).
    view: String,
    /// The last change the view had.
    seq: u64,
}

/// `POST /api/documents/{id}/sheet/diffs?view=&seq=`: a view's edits (IronCalc's `flushSendQueue()` bytes).
pub(crate) async fn sheet_diffs(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DiffsQuery>,
    body: axum::body::Bytes,
) -> Result<Json<DiffsApplied>, ApiError> {
    let (d, local) = local_doc(&st, &id).await?;
    let view = q.view.trim();
    if view.is_empty() || view == "pdt" || view.len() > 64 {
        return Err(invalid("give the view's own id"));
    }
    let r = local.view_diffs(&d.provider_ref, view, q.seq, &body).map_err(|e| match e {
        ProviderError::Invalid(m) => ApiError::new(StatusCode::CONFLICT, "sheet_conflict", m),
        e => e.into(),
    })?;
    st.store.touch_document(&id, now_ms()).await?;
    Ok(Json(r))
}

#[derive(Deserialize)]
pub(crate) struct EventsQuery {
    #[serde(default)]
    view: String,
}

/// How often an open document's stream checks its file for changes made outside PDT.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// `GET /api/documents/{id}/events?view=`: Server-Sent Events of a local document's changes (`diffs`, `reload`,
/// `changed`, `comments`, `charts`), each a JSON [`DocEvent`]. A view's own diffs are not sent back to it.
pub(crate) async fn events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<EventsQuery>,
) -> Result<
    axum::response::sse::Sse<
        impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    ApiError,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    let (d, local) = local_doc(&st, &id).await?;
    let reference = d.provider_ref.clone();
    let rx = local.subscribe(&reference);
    local.check_external(&reference);
    let wake = st.resume_changed.subscribe();
    let mut tick = tokio::time::interval(WATCH_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let init = (st, local, reference, rx, wake, tick, q.view);
    let s = futures_util::stream::unfold(init, |(st, local, reference, mut rx, mut wake, mut tick, view)| async move {
        loop {
            if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                return None;
            }
            let got = tokio::select! {
                r = rx.recv() => r,
                _ = tick.tick() => {
                    local.check_external(&reference);
                    continue;
                }
                w = wake.changed() => {
                    if w.is_err() {
                        return None;
                    }
                    continue;
                }
            };
            match got {
                Ok(DocEvent::Diffs { from, .. }) if from == view => continue,
                Ok(e) => {
                    let ev = Event::default()
                        .event(e.name())
                        .json_data(&e)
                        .unwrap_or_else(|_| Event::default().comment("?"));
                    return Some((Ok(ev), (st, local, reference, rx, wake, tick, view)));
                }
                // Missed some: the view loads the sheet again.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let e = DocEvent::Reload { seq: u64::MAX };
                    let ev = Event::default()
                        .event(e.name())
                        .json_data(&e)
                        .unwrap_or_else(|_| Event::default().comment("?"));
                    return Some((Ok(ev), (st, local, reference, rx, wake, tick, view)));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(s).keep_alive(KeepAlive::default()))
}

/// `GET /api/documents/{id}/charts`: a sheet's charts with the numbers they draw (none for a provider without
/// charts).
pub(crate) async fn charts(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ChartView>>, ApiError> {
    let d = document(&st, &id).await?;
    if d.provider != local::ID || d.kind != DocumentKind::Sheet {
        return Ok(Json(vec![]));
    }
    Ok(Json(st.local.charts(&d.id, &d.provider_ref).await?))
}

#[derive(Deserialize)]
pub(crate) struct ExportQuery {
    /// `md` (a doc), `xlsx` or `csv` (a sheet; `csv` is one sheet, the first unless `sheet` names one).
    format: String,
    #[serde(default)]
    sheet: Option<String>,
}

/// `GET /api/documents/{id}/export?format=md|xlsx|csv[&sheet=]`: the document as a file to download. Any provider's
/// doc exports as `.md` and sheet as `.csv`; `.xlsx` is for local sheets.
pub(crate) async fn export(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Result<axum::response::Response, ApiError> {
    let d = document(&st, &id).await?;
    let format = q.format.trim().to_ascii_lowercase();
    let sheet = q.sheet.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let out = if d.provider == local::ID {
        st.local.export(&d.provider_ref, &d.title, &format, sheet)?
    } else {
        let p = provider_of(&st, &d)?;
        let stem = local::file_stem(&d.title);
        match (p.read(&d.provider_ref).await?, format.as_str()) {
            (DocumentContent::Doc { markdown, .. }, "md") => local::Export {
                bytes: markdown.into_bytes(),
                file_name: format!("{stem}.md"),
                mime: "text/markdown; charset=utf-8",
            },
            (DocumentContent::Sheet { sheets, .. }, "csv") => {
                let s = match sheet {
                    Some(n) => sheets.iter().find(|s| s.name == n),
                    None => sheets.first(),
                }
                .ok_or_else(|| invalid("there is no such sheet"))?;
                let mut csv = String::new();
                for row in &s.rows {
                    let line: Vec<String> = row
                        .iter()
                        .map(|c| {
                            if c.value.contains([',', '"', '\n']) {
                                format!("\"{}\"", c.value.replace('"', "\"\""))
                            } else {
                                c.value.clone()
                            }
                        })
                        .collect();
                    csv.push_str(&line.join(","));
                    csv.push('\n');
                }
                local::Export {
                    bytes: csv.into_bytes(),
                    file_name: format!("{stem}.csv"),
                    mime: "text/csv; charset=utf-8",
                }
            }
            _ => return Err(invalid(format!("{} can't export this as {format}", p.label()))),
        }
    };
    let disposition = format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        out.file_name.replace(|c: char| !c.is_ascii() || c == '"', "_"),
        out.file_name
            .bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            })
            .collect::<String>()
    );
    Ok(bytes_response(out.bytes, out.mime, &[("content-disposition", disposition)]))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UploadQuery {
    /// The file's name (`Budget.xlsx`): its type and the document's title.
    name: String,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    ticket_id: Option<String>,
    #[serde(default)]
    by: Option<Actor>,
}

/// An imported file: its document, and what of the file PDT couldn't keep.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UploadedDocument {
    pub document: Document,
    /// What the file had that PDT's sheets don't keep (`charts`, `images and drawings`, `pivot tables`).
    pub dropped: Vec<String>,
}

/// `POST /api/documents/upload?name=&projectId=&ticketId=`: a file from the user's computer (the body) made into a
/// local document: `.md` a doc, `.xlsx` or `.csv` a sheet.
pub(crate) async fn upload(
    State(st): State<AppState>,
    Query(q): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<UploadedDocument>), ApiError> {
    if body.is_empty() {
        return Err(invalid("the file is empty"));
    }
    if provider(&st, local::ID).is_none() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "no_provider", "this host has no local documents"));
    }
    let (project, ticket) = targets(&st, q.project_id.as_deref(), q.ticket_id.as_deref()).await?;
    let folder = folder(&st, project.as_deref()).await?;
    let (file, dropped) = st.local.import_file(&folder, q.name.trim(), &body)?;
    let (_, document) = record(&st, local::ID, file, project.as_deref(), ticket.as_deref(), &by_of(q.by)).await?;
    Ok((StatusCode::CREATED, Json(UploadedDocument { document, dropped })))
}

#[derive(Deserialize)]
pub(crate) struct ImageQuery {
    name: String,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedImage {
    /// The path to write in the doc's Markdown (`![](path)`), relative to it.
    pub path: String,
}

/// `POST /api/documents/{id}/images?name=`: an image (the body) saved beside a local doc.
pub(crate) async fn image(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ImageQuery>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<SavedImage>), ApiError> {
    let (d, local) = local_doc(&st, &id).await?;
    if body.is_empty() {
        return Err(invalid("the image is empty"));
    }
    let path = local.save_image(&d.provider_ref, q.name.trim(), &body)?;
    Ok((StatusCode::CREATED, Json(SavedImage { path })))
}

/// `GET /api/documents/{id}/assets/{*path}`: one of a local doc's images, by the path its Markdown gives.
pub(crate) async fn asset(
    State(st): State<AppState>,
    Path((id, path)): Path<(String, String)>,
) -> Result<axum::response::Response, ApiError> {
    let (d, local) = local_doc(&st, &id).await?;
    let (bytes, mime) = local.asset(&d.provider_ref, &path)?;
    Ok(bytes_response(
        bytes,
        mime,
        &[
            ("x-content-type-options", "nosniff".into()),
            ("content-security-policy", "default-src 'none'; style-src 'unsafe-inline'".into()),
        ],
    ))
}
