//! A project's board settings on this host (PDT-377): `GET`/`PUT /api/projects/{id}/board`.

use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use pdt_core::board::BoardSettings;

/// `GET /api/projects/{id}/board`: the project's board settings (the defaults until it is given some).
pub(crate) async fn get_settings(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BoardSettings>, ApiError> {
    st.project(&id).await?;
    Ok(Json(st.store.get_project_board(&id).await?))
}

/// `PUT /api/projects/{id}/board`: replaces them.
pub(crate) async fn put_settings(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<BoardSettings>, JsonRejection>,
) -> Result<Json<BoardSettings>, ApiError> {
    let Json(s) = payload?;
    st.project(&id).await?;
    let s = s.validate().map_err(ApiError::invalid)?;
    st.store.put_project_board(&id, &s).await?;
    Ok(Json(s))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new().route("/projects/{id}/board", get(get_settings).put(put_settings))
}
