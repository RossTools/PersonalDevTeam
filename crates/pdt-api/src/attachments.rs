//! What a conversation's composer adds to a message: images, stored in
//! `<data dir>/sessions/<id>/attachments/<attachment id>.<ext>` (removed with the session), and `@` file suggestions
//! from the session's folder.

use crate::error::{ApiError, OrNotFound, codes};
use crate::routes::AppState;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use pdt_agents::adapter::ImageFile;
use pdt_core::conversation::composer::{
    MAX_IMAGE_BYTES, MAX_IMAGES, image_extension, image_type, rank_paths, valid_attachment_id,
};
use serde::Serialize;
use std::path::PathBuf;
use ts_rs::TS;

/// The image types PDT keeps (the file's own bytes decide).
const TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
/// How many `@` suggestions one query returns at most.
const MAX_SUGGESTIONS: usize = 50;

fn dir(st: &AppState, session_id: &str) -> PathBuf {
    st.data_dir.join("sessions").join(session_id).join("attachments")
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Attachment {
    pub id: String,
    pub media_type: String,
    #[ts(type = "number")]
    pub size: usize,
}

/// `POST /api/sessions/{id}/attachments`: the body is the image itself (PNG, JPEG, GIF or WebP, at most 10 MB).
pub(crate) async fn upload(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<Attachment>), ApiError> {
    st.session(&id).await?;
    if body.len() > MAX_IMAGE_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            codes::IMAGE_TOO_LARGE,
            "an image may be at most 10 MB",
        ));
    }
    let media_type = image_type(&body).ok_or_else(|| {
        ApiError::bad_request(codes::UNSUPPORTED_IMAGE, "only PNG, JPEG, GIF and WebP images can be attached")
    })?;
    let aid = uuid::Uuid::now_v7().to_string();
    let file = dir(&st, &id).join(format!("{aid}.{}", image_extension(media_type)));
    let size = body.len();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(file.parent().expect("attachments dir"))?;
        std::fs::write(&file, &body)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::io("saving the image", e))?;
    Ok((StatusCode::CREATED, Json(Attachment { id: aid, media_type: media_type.into(), size })))
}

/// The stored file for an attachment id, if the session has it.
fn find(st: &AppState, session_id: &str, aid: &str) -> Option<ImageFile> {
    if !valid_attachment_id(aid) {
        return None;
    }
    TYPES.iter().find_map(|t| {
        let path = dir(st, session_id).join(format!("{aid}.{}", image_extension(t)));
        path.is_file().then(|| ImageFile { path, media_type: (*t).to_string() })
    })
}

/// The images a message names, all of which must belong to the session.
pub(crate) fn resolve(st: &AppState, session_id: &str, ids: &[String]) -> Result<Vec<ImageFile>, ApiError> {
    if ids.len() > MAX_IMAGES {
        return Err(ApiError::bad_request(codes::INVALID_MESSAGE, "a message may carry at most 10 images"));
    }
    ids.iter()
        .map(|aid| {
            find(st, session_id, aid)
                .ok_or_else(|| ApiError::bad_request(codes::UNKNOWN_ATTACHMENT, "that image is not attached here"))
        })
        .collect()
}

/// `GET /api/sessions/{id}/attachments/{attachmentId}`: the image, for the conversation's thumbnails.
pub(crate) async fn get(
    State(st): State<AppState>,
    Path((id, aid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    st.store.get_session(&id).await?.or_404("session")?;
    let img = find(&st, &id, &aid).or_404("attachment")?;
    let bytes =
        tokio::fs::read(&img.path).await.map_err(|e| ApiError::io_404("attachment", "reading the attachment", e))?;
    Ok((
        [
            (header::CONTENT_TYPE, img.media_type),
            // Ids are never reused, so the bytes never change.
            (header::CACHE_CONTROL, "private, max-age=31536000, immutable".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ],
        bytes,
    )
        .into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct FilesQuery {
    #[serde(default)]
    q: String,
}

/// `GET /api/sessions/{id}/files?q=`: the session folder's files (as git lists them, `.gitignore` honoured) that best
/// match an `@` query, best first.
pub(crate) async fn files(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<FilesQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    let session = st.session(&id).await?;
    let cwd = PathBuf::from(&session.cwd);
    let all = tokio::task::spawn_blocking(move || pdt_git::list_files(&cwd, ""))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .unwrap_or_default();
    Ok(Json(rank_paths(q.q.trim(), &all, MAX_SUGGESTIONS).into_iter().map(String::from).collect()))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route(
            "/sessions/{id}/attachments",
            post(upload)
                // Room for the largest image; anything bigger is refused with a clear error.
                .layer(axum::extract::DefaultBodyLimit::max(pdt_core::conversation::composer::MAX_IMAGE_BYTES + 1)),
        )
        .route("/sessions/{id}/attachments/{attachment_id}", get(self::get))
        .route("/sessions/{id}/files", get(files))
}
