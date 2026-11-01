//! A ticket's attachments (PDT-300): screenshots and text files in `<data dir>/tickets/<ticket id>/attachments/`,
//! their metadata in the store. The user adds them from the UI (`POST /api/tickets/{id}/attachments`), the PM and
//! agents through pdt-mcp by local path ([`attach_path`]). Removed with their ticket ([`prune`]).

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use pdt_core::conversation::composer::MAX_IMAGE_BYTES;
use pdt_core::session::Session;
use pdt_core::ticket::{Actor, By};
use pdt_core::ticket_attachment::{Refused, TicketAttachment, clean_name, path_refusal, sniff};
use pdt_store::AddAttachment;
use std::path::PathBuf;

fn tickets_dir(st: &AppState) -> PathBuf {
    st.data_dir.join("tickets")
}

fn dir(st: &AppState, ticket_id: &str) -> PathBuf {
    tickets_dir(st).join(ticket_id).join("attachments")
}

/// Where an attachment's file is (what `get_ticket` gives agents).
pub(crate) fn path_of(st: &AppState, a: &TicketAttachment) -> PathBuf {
    dir(st, &a.ticket_id).join(a.file_name())
}

/// A `ticket.commented` trigger with its attachments' local paths filled in (`attachments[].path`), and
/// `attachmentList`, the lines a message gives them in (PDT-341).
pub(crate) fn fill_paths(st: &AppState, trigger: &mut serde_json::Value) {
    let Some(ticket) = trigger["ticketId"].as_str().map(String::from) else { return };
    let Some(list) = trigger.get_mut("attachments").and_then(|a| a.as_array_mut()) else { return };
    let mut lines = Vec::new();
    for a in list {
        let Some(file) = a["fileName"].as_str().filter(|f| !f.is_empty()) else { continue };
        let path = dir(st, &ticket).join(file);
        lines.push(format!("- {}: {}", a["name"].as_str().unwrap_or(file), path.display()));
        a["path"] = serde_json::json!(path);
    }
    if !lines.is_empty() {
        trigger["attachmentList"] =
            serde_json::json!(format!("\n\nAttached (open them at these paths):\n{}", lines.join("\n")));
    }
}

fn refused(r: Refused) -> ApiError {
    let status = if r.too_large { StatusCode::PAYLOAD_TOO_LARGE } else { StatusCode::BAD_REQUEST };
    ApiError::new(status, r.code, r.message)
}

/// A ticket's id from its id or key (PDT-271).
async fn ticket_id(st: &AppState, given: &str) -> Result<String, ApiError> {
    if st.store.get_ticket_summary(given).await?.is_some() {
        return Ok(given.to_string());
    }
    st.store.ticket_id_by_key(given).await?.ok_or_else(|| ApiError::not_found("ticket"))
}

/// Keeps `bytes` as an attachment of the ticket, by `by`. The bytes decide what it is ([`sniff`]).
pub(crate) async fn attach(
    st: &AppState,
    ticket_id: &str,
    name: &str,
    bytes: Vec<u8>,
    by: &By,
) -> Result<TicketAttachment, ApiError> {
    let media_type = sniff(name, &bytes).map_err(refused)?;
    let a = TicketAttachment {
        id: uuid::Uuid::now_v7().to_string(),
        ticket_id: ticket_id.to_string(),
        name: clean_name(name, media_type),
        media_type: media_type.to_string(),
        size: bytes.len() as i64,
        actor: if by.actor == Actor::User { Actor::User } else { Actor::Agent },
        agent: by.agent.clone(),
        event_id: None,
        created_at: now_ms(),
    };
    let file = path_of(st, &a);
    let written = file.clone();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(written.parent().expect("attachments dir"))?;
        std::fs::write(&written, &bytes)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(format!("saving the attachment: {e}")))?;
    let added = st.store.add_ticket_attachment(&a).await;
    if !matches!(added, Ok(AddAttachment::Added)) {
        let _ = std::fs::remove_file(&file);
    }
    match added? {
        AddAttachment::Added => Ok(a),
        AddAttachment::NoTicket => Err(ApiError::not_found("ticket")),
        AddAttachment::Full => Err(ApiError::new(
            StatusCode::CONFLICT,
            "too_many_attachments",
            format!(
                "a ticket holds at most {} attachments; remove one first",
                pdt_core::ticket_attachment::MAX_PER_TICKET
            ),
        )),
    }
}

/// The file an agent names, as `(name, bytes)`: a local path (absolute; a readable regular file, never a secret, see
/// [`path_refusal`]), or `pasted:N`, the Nth newest image the user pasted into this conversation (`pasted:1` the
/// latest).
pub(crate) async fn read_for_agent(st: &AppState, session: &Session, given: &str) -> Result<(String, Vec<u8>), String> {
    let given = given.trim();
    let path = match given.strip_prefix("pasted:") {
        Some(n) => pasted(st, session, n)?,
        None => PathBuf::from(given),
    };
    let data_dir = st.data_dir.canonicalize().unwrap_or_else(|_| st.data_dir.clone());
    tokio::task::spawn_blocking(move || {
        if !path.is_absolute() {
            return Err(format!("{} is not an absolute path", path.display()));
        }
        let real = path.canonicalize().map_err(|e| format!("can't read {}: {e}", path.display()))?;
        if let Some(why) = path_refusal(&real, &data_dir).or_else(|| path_refusal(&path, &data_dir)) {
            return Err(format!("{}: {why}", path.display()));
        }
        let meta = std::fs::metadata(&real).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
        if meta.len() > MAX_IMAGE_BYTES as u64 {
            return Err(format!("{} is too big: an image may be at most 10 MB, a text file 1 MB", path.display()));
        }
        let bytes = std::fs::read(&real).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok((name, bytes))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The Nth newest image pasted into the session's conversation (attachment ids are time-ordered).
fn pasted(st: &AppState, session: &Session, n: &str) -> Result<PathBuf, String> {
    let n: usize = n.trim().parse().ok().filter(|n| *n >= 1).ok_or("pasted:N needs a number from 1 (the latest)")?;
    let folder = st.data_dir.join("sessions").join(&session.id).join("attachments");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    files.sort();
    files.into_iter().rev().nth(n - 1).ok_or_else(|| match n {
        1 => "no image has been pasted into this conversation".to_string(),
        _ => format!("this conversation has fewer than {n} pasted images"),
    })
}

/// Reads and checks each file an agent names (see [`read_for_agent`]) before any is kept, so a bad one changes nothing.
pub(crate) async fn read_files(
    st: &AppState,
    session: &Session,
    files: &[String],
) -> Result<Vec<(String, Vec<u8>)>, String> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    // A session reading outside input attaches nothing: it could be talked into attaching a private file (PDT-201).
    if crate::untrusted::triaging(st, session).await {
        return Err("this session triages input from outside PDT, so it can't attach files".into());
    }
    let mut read = Vec::new();
    for f in files {
        let (name, bytes) = read_for_agent(st, session, f).await?;
        sniff(&name, &bytes).map_err(|r| format!("{f}: {}", r.message))?;
        read.push((name, bytes));
    }
    Ok(read)
}

/// Attaches files [`read_files`] read.
pub(crate) async fn attach_read(
    st: &AppState,
    ticket_id: &str,
    read: Vec<(String, Vec<u8>)>,
    by: &By,
) -> Result<Vec<TicketAttachment>, String> {
    let mut out = Vec::new();
    for (name, bytes) in read {
        out.push(attach(st, ticket_id, &name, bytes, by).await.map_err(|e| e.message().to_string())?);
    }
    Ok(out)
}

/// What a tool's answer says about the files it attached.
pub(crate) fn attached_json(st: &AppState, list: &[TicketAttachment]) -> serde_json::Value {
    serde_json::Value::Array(
        list.iter()
            .map(|a| {
                serde_json::json!({"id": a.id, "name": a.name, "mediaType": a.media_type, "size": a.size,
                                   "path": path_of(st, a), "markdown": a.markdown()})
            })
            .collect(),
    )
}

/// Removes the attachment folders of tickets that no longer exist (after a ticket or project is deleted). Never fails.
pub(crate) async fn prune(st: &AppState) {
    let root = tickets_dir(st);
    let ids: Vec<String> = std::fs::read_dir(&root)
        .map(|r| {
            r.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return;
    }
    let Ok(live) = st.store.existing_ticket_ids(&ids).await else { return };
    for id in ids.iter().filter(|id| !live.contains(id)) {
        if let Err(e) = std::fs::remove_dir_all(root.join(id)) {
            tracing::warn!(ticket = %id, error = %e, "could not remove a deleted ticket's attachments");
        }
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct UploadQuery {
    /// The file's name (its extension names a text file's type).
    #[serde(default)]
    name: String,
    /// `user` from PDT's UI; anything else is an agent.
    #[serde(default)]
    by: Option<Actor>,
}

/// `POST /api/tickets/{id}/attachments?name=`: the body is the file itself (an image up to 10 MB, or UTF-8 text up
/// to 1 MB).
pub(crate) async fn upload(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<TicketAttachment>), ApiError> {
    let id = ticket_id(&st, &id).await?;
    let by = By::from(if q.by == Some(Actor::User) { Actor::User } else { Actor::Agent });
    let a = attach(&st, &id, &q.name, body.to_vec(), &by).await?;
    Ok((StatusCode::CREATED, Json(a)))
}

/// `GET /api/tickets/{id}/attachments`: oldest first.
pub(crate) async fn list(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TicketAttachment>>, ApiError> {
    let id = ticket_id(&st, &id).await?;
    Ok(Json(st.store.ticket_attachments(&id).await?))
}

/// `GET /api/tickets/{id}/attachments/{aid}`: the file. Text is always `text/plain` (never rendered as a page).
pub(crate) async fn get(
    State(st): State<AppState>,
    Path((id, aid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let id = ticket_id(&st, &id).await?;
    let a = st.store.get_ticket_attachment(&id, &aid).await?.ok_or_else(|| ApiError::not_found("attachment"))?;
    let bytes = tokio::fs::read(path_of(&st, &a)).await.map_err(|_| ApiError::not_found("attachment"))?;
    let media_type = if a.is_image() { a.media_type.clone() } else { "text/plain; charset=utf-8".into() };
    let ascii: String =
        a.name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' { c } else { '_' }).collect();
    Ok((
        [
            (header::CONTENT_TYPE, media_type),
            (header::CONTENT_DISPOSITION, format!("inline; filename=\"{ascii}\"")),
            // Ids are never reused, so the bytes never change.
            (header::CACHE_CONTROL, "private, max-age=31536000, immutable".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; sandbox".to_string()),
        ],
        bytes,
    )
        .into_response())
}

/// `DELETE /api/tickets/{id}/attachments/{aid}`.
pub(crate) async fn remove(
    State(st): State<AppState>,
    Path((id, aid)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let id = ticket_id(&st, &id).await?;
    let a = st.store.get_ticket_attachment(&id, &aid).await?.ok_or_else(|| ApiError::not_found("attachment"))?;
    st.store.delete_ticket_attachment(&id, &aid).await?;
    let _ = tokio::fs::remove_file(path_of(&st, &a)).await;
    Ok(StatusCode::NO_CONTENT)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new()
        .route(
            "/tickets/{id}/attachments",
            get(list)
                .post(upload)
                // Room for the largest image; anything bigger is refused with a clear error.
                .layer(axum::extract::DefaultBodyLimit::max(pdt_core::conversation::composer::MAX_IMAGE_BYTES + 1)),
        )
        .route("/tickets/{id}/attachments/{attachment_id}", get(self::get).delete(remove))
}
