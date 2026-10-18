//! Ticket routes (tickets spec §4): the backlog query, create/get/edit/delete, rank moves, bulk actions, links, the
//! timeline and comments, and a project's labels.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::ticket::{
    BulkRequest, BulkResult, EventPage, EventQuery, MAX_LIMIT, NewComment, NewTicket, RankMove, Ticket, TicketEvent,
    TicketLink, TicketPage, TicketPatch, TicketQuery, valid_comment,
};
use pdt_store::{DeleteTicket, TicketError};

fn unknown_ticket() -> ApiError {
    ApiError::not_found("ticket")
}

pub(crate) async fn list(
    State(st): State<AppState>,
    query: Result<Query<TicketQuery>, QueryRejection>,
) -> Result<Json<TicketPage>, ApiError> {
    let Query(q) = query.map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", e.body_text()))?;
    let filter = q.validate()?;
    Ok(Json(st.store.query_tickets(&filter).await?))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    Path(project_id): Path<String>,
    payload: Result<Json<NewTicket>, JsonRejection>,
) -> Result<(StatusCode, Json<Ticket>), ApiError> {
    let Json(input) = payload?;
    let t = input.validate()?;
    let id = uuid::Uuid::now_v7().to_string();
    let ticket = st.store.create_ticket(&id, &project_id, &t, now_ms()).await?;
    Ok((StatusCode::CREATED, Json(ticket)))
}

pub(crate) async fn get_one(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Ticket>, ApiError> {
    Ok(Json(st.store.get_ticket(&id).await?.ok_or_else(unknown_ticket)?))
}

pub(crate) async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TicketPatch>, JsonRejection>,
) -> Result<Json<Ticket>, ApiError> {
    let Json(patch) = payload?;
    let patch = patch.validate()?;
    Ok(Json(st.store.patch_ticket(&id, &patch, now_ms()).await?.ok_or_else(unknown_ticket)?))
}

pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    match st.store.delete_ticket(&id).await? {
        DeleteTicket::Deleted => Ok(StatusCode::NO_CONTENT),
        DeleteTicket::NotFound => Err(unknown_ticket()),
        DeleteTicket::Active => {
            let key = st.store.get_ticket_summary(&id).await?.map(|t| t.key).unwrap_or(id);
            Err(TicketError::Active(key).into())
        }
    }
}

pub(crate) async fn rank(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<RankMove>, JsonRejection>,
) -> Result<Json<Ticket>, ApiError> {
    let Json(m) = payload?;
    let nonblank = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let (before, after) = (nonblank(&m.before), nonblank(&m.after));
    let t = st.store.move_ticket(&id, before.as_deref(), after.as_deref(), m.to).await?;
    Ok(Json(t.ok_or_else(unknown_ticket)?))
}

pub(crate) async fn bulk(
    State(st): State<AppState>,
    payload: Result<Json<BulkRequest>, JsonRejection>,
) -> Result<Json<BulkResult>, ApiError> {
    let Json(req) = payload?;
    let action = req.validate()?;
    let count = st.store.bulk_tickets(&req.ids, &action, now_ms()).await?;
    Ok(Json(BulkResult { count }))
}

pub(crate) async fn add_link(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TicketLink>, JsonRejection>,
) -> Result<Json<Ticket>, ApiError> {
    let Json(link) = payload?;
    let t = st.store.add_link(&id, link.to.trim(), link.kind, now_ms()).await?;
    Ok(Json(t.ok_or_else(unknown_ticket)?))
}

pub(crate) async fn remove_link(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TicketLink>, JsonRejection>,
) -> Result<Json<Ticket>, ApiError> {
    let Json(link) = payload?;
    let t = st.store.remove_link(&id, link.to.trim(), link.kind, now_ms()).await?;
    Ok(Json(t.ok_or_else(unknown_ticket)?))
}

pub(crate) async fn events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    query: Result<Query<EventQuery>, QueryRejection>,
) -> Result<Json<EventPage>, ApiError> {
    let Query(q) = query.map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", e.body_text()))?;
    let limit = q.limit.unwrap_or(pdt_core::ticket::DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let cursor = q.cursor.as_deref().map(str::trim).filter(|c| !c.is_empty());
    let page = st.store.list_events(&id, cursor, limit, q.newest_first.unwrap_or(false)).await?;
    Ok(Json(page.ok_or_else(unknown_ticket)?))
}

pub(crate) async fn comment(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<NewComment>, JsonRejection>,
) -> Result<(StatusCode, Json<TicketEvent>), ApiError> {
    let Json(c) = payload?;
    let body = valid_comment(&c.body)?;
    let event = st.store.add_comment(&id, &body, now_ms()).await?.ok_or_else(unknown_ticket)?;
    Ok((StatusCode::CREATED, Json(event)))
}

pub(crate) async fn labels(
    State(st): State<AppState>,
    Path(project_id): Path<String>,
) -> Result<Json<Vec<String>>, ApiError> {
    st.store.get_project(&project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    Ok(Json(st.store.project_labels(&project_id).await?))
}
