//! The user's todo list (PDT-125): `GET/POST /api/todos`, `PATCH/DELETE /api/todos/{id}`. The PM reads it with the
//! `list_todos` tool.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::todo::{self, NewTodo, Todo, TodoPatch};
use pdt_store::TodoFilter;
use serde::Deserialize;

fn bad(m: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", m)
}

/// A project id or key the user gave, as an id.
async fn project(st: &AppState, given: &str) -> Result<String, ApiError> {
    crate::pm::project_id(st, given).await.map_err(bad)
}

/// A ticket id or key the user gave: its id and its project's.
async fn ticket(st: &AppState, given: &str) -> Result<(String, String), ApiError> {
    let id = crate::mcp::tools::resolve_ticket(st, given, None).await.map_err(bad)?;
    let t = st.store.get_ticket(&id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
    Ok((id, t.summary.project_id))
}

/// The filter a list asks for: a project (id or key) and done or open.
pub(crate) async fn filter(
    st: &AppState,
    project_given: Option<&str>,
    done: Option<bool>,
) -> Result<TodoFilter, ApiError> {
    let project_id = match project_given.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Some(project(st, p).await?),
        None => None,
    };
    Ok(TodoFilter { project_id, done })
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ListQuery {
    project: Option<String>,
    done: Option<bool>,
}

/// `GET /api/todos?project=&done=`.
pub(crate) async fn list(State(st): State<AppState>, Query(q): Query<ListQuery>) -> Result<Json<Vec<Todo>>, ApiError> {
    let f = filter(&st, q.project.as_deref(), q.done).await?;
    Ok(Json(st.store.list_todos(&f).await?))
}

/// `POST /api/todos`: the new todo goes at the top of the list.
pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewTodo>, JsonRejection>,
) -> Result<(StatusCode, Json<Todo>), ApiError> {
    let Json(input) = payload?;
    let title = todo::valid_title(&input.title).map_err(bad)?;
    let (mut ticket_id, mut project_id) = (None, None);
    if let Some(t) = input.ticket.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let (t, p) = ticket(&st, t).await?;
        ticket_id = Some(t);
        project_id = Some(p);
    }
    if let Some(p) = input.project.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        project_id = Some(project(&st, p).await?);
    }
    let now = now_ms();
    let position = st.store.first_todo_position().await?.map_or(0.0, |p| p - 1.0);
    let id = uuid::Uuid::now_v7().to_string();
    st.store
        .insert_todo(&Todo {
            id: id.clone(),
            project_id,
            ticket_id,
            title,
            done: false,
            position,
            created_at: now,
            updated_at: now,
            done_at: None,
            project_key: None,
            ticket_key: None,
            ticket_title: None,
        })
        .await?;
    let made = st.store.get_todo(&id).await?.ok_or_else(|| ApiError::not_found("todo"))?;
    Ok((StatusCode::CREATED, Json(made)))
}

/// `PATCH /api/todos/{id}`: title, done, position, project and ticket (`""` removes a link). A linked ticket's project
/// is the todo's: linking one sets the project, and moving the todo to another project (or none) drops the ticket.
pub(crate) async fn patch(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TodoPatch>, JsonRejection>,
) -> Result<Json<Todo>, ApiError> {
    let Json(p) = payload?;
    let mut t = st.store.get_todo(&id).await?.ok_or_else(|| ApiError::not_found("todo"))?;
    let now = now_ms();
    if let Some(title) = &p.title {
        t.title = todo::valid_title(title).map_err(bad)?;
    }
    if let Some(done) = p.done {
        t.done_at = todo::done_at(t.done_at, done, now);
        t.done = done;
    }
    if let Some(position) = p.position {
        t.position = todo::valid_position(position).map_err(bad)?;
    }
    if let Some(given) = &p.ticket {
        t.ticket_id = match given.trim() {
            "" => None,
            given => {
                let (ticket_id, project_id) = ticket(&st, given).await?;
                t.project_id = Some(project_id);
                Some(ticket_id)
            }
        };
    }
    if let Some(given) = &p.project {
        t.project_id = match given.trim() {
            "" => None,
            given => Some(project(&st, given).await?),
        };
        // A linked ticket is always in the todo's project: moving it elsewhere drops the link.
        if let Some(ticket_id) = &t.ticket_id {
            let in_project = st.store.get_ticket(ticket_id).await?.map(|k| k.summary.project_id);
            if in_project != t.project_id {
                t.ticket_id = None;
            }
        }
    }
    t.updated_at = now;
    if !st.store.update_todo(&t).await? {
        return Err(ApiError::not_found("todo"));
    }
    Ok(Json(st.store.get_todo(&id).await?.ok_or_else(|| ApiError::not_found("todo"))?))
}

/// `DELETE /api/todos/{id}`.
pub(crate) async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !st.store.delete_todo(&id).await? {
        return Err(ApiError::not_found("todo"));
    }
    Ok(StatusCode::NO_CONTENT)
}
