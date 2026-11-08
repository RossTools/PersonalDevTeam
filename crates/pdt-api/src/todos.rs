//! The user's todo list (PDT-125): `GET/POST /api/todos`, `PATCH/DELETE /api/todos/{id}`. The PM reads it with the
//! `list_todos` tool. Checking off a repeating todo (PDT-141) makes the next one.

use crate::error::{ApiError, OrNotFound};
use crate::extract::ApiQuery;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::todo::{self, NewTodo, Todo, TodoPatch};
use pdt_store::TodoFilter;
use serde::Deserialize;

/// A project id or key the user gave, as an id.
async fn project(st: &AppState, given: &str) -> Result<String, ApiError> {
    crate::pm::project_id(st, given).await.map_err(ApiError::invalid)
}

/// A ticket id or key the user gave: its id and its project's.
async fn ticket(st: &AppState, given: &str) -> Result<(String, String), ApiError> {
    let id = crate::mcp::tools::resolve_ticket(st, given, None).await.map_err(ApiError::invalid)?;
    let t = st.ticket(&id).await?;
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
pub(crate) async fn list(
    State(st): State<AppState>,
    ApiQuery(q): ApiQuery<ListQuery>,
) -> Result<Json<Vec<Todo>>, ApiError> {
    let f = filter(&st, q.project.as_deref(), q.done).await?;
    Ok(Json(st.store.list_todos(&f).await?))
}

/// `POST /api/todos`: the new todo goes at the top of the list.
pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewTodo>, JsonRejection>,
) -> Result<(StatusCode, Json<Todo>), ApiError> {
    let Json(input) = payload?;
    let title = todo::valid_title(&input.title).map_err(ApiError::invalid)?;
    let body = todo::valid_body(input.body.as_deref().unwrap_or_default()).map_err(ApiError::invalid)?;
    let (mut ticket_id, mut project_id) = (None, None);
    if let Some(t) = input.ticket.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let (t, p) = ticket(&st, t).await?;
        ticket_id = Some(t);
        project_id = Some(p);
    }
    if let Some(p) = input.project.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        project_id = Some(project(&st, p).await?);
    }
    let due_on = todo::valid_due_on(input.due_on.as_deref().unwrap_or_default()).map_err(ApiError::invalid)?;
    let due_time = todo::valid_due_time(input.due_time.as_deref().unwrap_or_default()).map_err(ApiError::invalid)?;
    let (due_on, due_time) = todo::valid_due(due_on, due_time).map_err(ApiError::invalid)?;
    let recurrence = todo::valid_recurrence(input.recurrence, due_on.as_deref()).map_err(ApiError::invalid)?;
    let now = now_ms();
    let position = st.store.first_todo_position().await?.map_or(0.0, |p| p - 1.0);
    let id = uuid::Uuid::now_v7().to_string();
    st.store
        .insert_todo(&Todo {
            id: id.clone(),
            project_id,
            ticket_id,
            title,
            body,
            done: false,
            position,
            created_at: now,
            updated_at: now,
            done_at: None,
            project_key: None,
            ticket_key: None,
            ticket_title: None,
            due_on,
            due_time,
            recurrence,
        })
        .await?;
    // A todo added already due reminds at once.
    crate::notifications::kick(&st);
    let made = st.store.get_todo(&id).await?.or_404("todo")?;
    Ok((StatusCode::CREATED, Json(made)))
}

/// `PATCH /api/todos/{id}`: title, body, done, position, project and ticket (`""` removes a link), due date and time (`""`
/// clears; clearing the date clears the time). A linked ticket's project is the todo's: linking one sets the project,
/// and moving the todo to another project (or none) drops the ticket.
pub(crate) async fn patch(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TodoPatch>, JsonRejection>,
) -> Result<Json<Todo>, ApiError> {
    let Json(p) = payload?;
    let mut t = st.store.get_todo(&id).await?.or_404("todo")?;
    let was_done = t.done;
    let now = now_ms();
    if let Some(title) = &p.title {
        t.title = todo::valid_title(title).map_err(ApiError::invalid)?;
    }
    if let Some(body) = &p.body {
        t.body = todo::valid_body(body).map_err(ApiError::invalid)?;
    }
    if let Some(done) = p.done {
        t.done_at = todo::done_at(t.done_at, done, now);
        t.done = done;
    }
    if let Some(position) = p.position {
        t.position = todo::valid_position(position).map_err(ApiError::invalid)?;
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
    if p.due_on.is_some() || p.due_time.is_some() {
        let mut due_on = t.due_on.clone();
        let mut due_time = t.due_time.clone();
        if let Some(d) = &p.due_on {
            due_on = todo::valid_due_on(d).map_err(ApiError::invalid)?;
            if due_on.is_none() {
                due_time = None;
            }
        }
        if let Some(d) = &p.due_time {
            due_time = todo::valid_due_time(d).map_err(ApiError::invalid)?;
        }
        (t.due_on, t.due_time) = todo::valid_due(due_on, due_time).map_err(ApiError::invalid)?;
        // No date, no repeating.
        if t.due_on.is_none() {
            t.recurrence = None;
        }
    }
    if let Some(r) = p.recurrence.clone() {
        t.recurrence = r;
    }
    t.recurrence = todo::valid_recurrence(t.recurrence.take(), t.due_on.as_deref()).map_err(ApiError::invalid)?;
    t.updated_at = now;
    let next = if t.done && !was_done { next_todo(&t, now) } else { None };
    let saved = match &next {
        Some(next) => st.store.complete_todo(&t, next).await?.is_some(),
        None => st.store.update_todo(&t).await?,
    };
    if !saved {
        return Err(ApiError::not_found("todo"));
    }
    // Done or re-dated: its reminder clears (or a new one comes) now rather than on the next pass.
    crate::notifications::kick(&st);
    Ok(Json(st.store.get_todo(&id).await?.or_404("todo")?))
}

/// The todo a repeating one makes when it is checked off (or skipped) now: due on the rule's next day, with the same
/// title, body, time, links and rule, where it was in the list. None for a todo that doesn't repeat.
fn next_todo(t: &Todo, now: i64) -> Option<Todo> {
    let rule = t.recurrence.as_ref()?;
    let due_on = todo::next_due_on_local(rule, t.due_on.as_deref()?)?;
    Some(Todo {
        id: uuid::Uuid::now_v7().to_string(),
        done: false,
        created_at: now,
        updated_at: now,
        done_at: None,
        due_on: Some(due_on),
        ..t.clone()
    })
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DeleteQuery {
    /// For a repeating todo: `only` (the default) skips to the next one, `series` stops it repeating.
    series: Option<String>,
}

/// `DELETE /api/todos/{id}?series=only|series`. Deleting an open repeating todo as "only this one" makes its next one,
/// as checking it off would; "stop the series" doesn't.
pub(crate) async fn delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<DeleteQuery>,
) -> Result<StatusCode, ApiError> {
    let stop = match q.series.as_deref().unwrap_or("only") {
        "only" => false,
        "series" => true,
        other => return Err(ApiError::invalid(format!("series is \"only\" or \"series\", not {other:?}"))),
    };
    let t = st.store.get_todo(&id).await?.or_404("todo")?;
    let next = if stop || t.done { None } else { next_todo(&t, now_ms()) };
    if !st.store.delete_todo_keeping(&id, next.as_ref()).await? {
        return Err(ApiError::not_found("todo"));
    }
    crate::notifications::kick(&st);
    Ok(StatusCode::NO_CONTENT)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, patch};
    axum::Router::new()
        .route("/todos", get(list).post(create))
        .route("/todos/{id}", patch(self::patch).delete(self::delete))
}
