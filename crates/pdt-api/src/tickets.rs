//! Ticket routes: the backlog query, create/get/edit/delete, rank moves, bulk actions, links, the
//! timeline and comments, and a project's labels.

use crate::error::{ApiError, OrNotFound, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use pdt_core::ticket::{
    Actor, AttemptState, BulkRequest, BulkResult, By, EventPage, EventQuery, MAX_LIMIT, NewComment, NewTicket,
    PipelineStage, RankMove, Ticket, TicketEvent, TicketLink, TicketPage, TicketPatch, TicketQuery, TicketSource,
    TicketStage, ValidBulk, ValidPatch, valid_comment,
};
use pdt_core::workflow::api::{NewLogEvent, RunStatus};
use pdt_store::{DeleteTicket, TicketError};

pub(crate) async fn list(
    State(st): State<AppState>,
    query: Result<Query<TicketQuery>, QueryRejection>,
) -> Result<Json<TicketPage>, ApiError> {
    let Query(q) = query.map_err(|e| ApiError::invalid(e.body_text()))?;
    let mut filter = q.validate()?;
    let idx = crate::overlap::index(&st, filter.project.as_deref()).await?;
    match filter.conflicts {
        Some(true) => filter.only_ids = Some(idx.ids()),
        Some(false) => filter.except_ids = idx.ids(),
        None => {}
    }
    let mut page = st.store.query_tickets(&filter).await?;
    for t in page.items.iter_mut() {
        idx.apply_summary(t);
        crate::github::apply_summary(&st, t);
    }
    crate::parts::apply_page(&st, &mut page.items).await?;
    crate::scheduler::apply_agents(&st, &mut page.items).await;
    crate::scheduler::apply_auto_start(&st, &mut page.items).await;
    Ok(Json(page))
}

/// The ticket as the API returns it: with its overlaps.
async fn reply(st: &AppState, t: Ticket) -> Result<Json<Ticket>, ApiError> {
    Ok(Json(crate::overlap::decorate(st, t).await?))
}

/// The header an agent names its session in when it calls the HTTP API (its session id), so what it makes is
/// recorded as that session's (PDT-363).
pub(crate) const SESSION_HEADER: &str = "x-agent-session";

/// Who makes a ticket over the HTTP API (PDT-363), and the source it gets: the user from PDT's web client (it sends the
/// UI token); the agent session `x-agent-session` names, with its persona and the ticket it works on (the PM's are `pm`,
/// a triage session's `external`); any other caller is an agent's curl.
pub(crate) async fn creator(st: &AppState, headers: &HeaderMap) -> Result<(By, TicketSource), ApiError> {
    if crate::untrusted::from_ui(st, headers)? {
        return Ok((Actor::User.into(), TicketSource::User));
    }
    let named = headers.get(SESSION_HEADER).and_then(|v| v.to_str().ok()).map(str::trim).filter(|s| !s.is_empty());
    let session = match named {
        Some(id) => st.store.get_session(id).await?,
        None => None,
    };
    let Some(s) = session else { return Ok((Actor::Agent.into(), TicketSource::Agent)) };
    let source = if crate::untrusted::triaging(st, &s).await {
        TicketSource::External
    } else if crate::pm::is_pm_session(st, &s).await {
        TicketSource::Pm
    } else {
        TicketSource::Agent
    };
    Ok((crate::mcp::tools::agent_by(st, &s).await, source))
}

/// `POST /api/projects/{id}/tickets`: a [`NewTicket`], and from an agent why it makes it (`reason`; PDT-366).
#[derive(serde::Deserialize)]
pub(crate) struct CreateTicket {
    #[serde(flatten)]
    ticket: NewTicket,
    #[serde(default)]
    reason: Option<String>,
}

pub(crate) async fn create(
    State(st): State<AppState>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<CreateTicket>, JsonRejection>,
) -> Result<(StatusCode, Json<Ticket>), ApiError> {
    let Json(CreateTicket { ticket: input, reason }) = payload?;
    let (by, source) = creator(&st, &headers).await?;
    let mut t = pdt_core::ticket::ValidNewTicket { source, ..input.validate()? };
    // An agent's ticket waits for the user as a proposal (PDT-366).
    let proposal = pdt_core::ticket_proposal::proposes(source, false)
        && crate::workflow::nodes::ticket_proposal::proposals_on(&st).await;
    if proposal {
        pdt_core::ticket_proposal::hold(&mut t);
    }
    let id = uuid::Uuid::now_v7().to_string();
    let made = pdt_store::Made { reason: reason.as_deref(), proposal, ..Default::default() };
    let ticket = st.store.create_ticket_made(&id, &project_id, &t, by, made, now_ms()).await?;
    Ok((StatusCode::CREATED, reply(&st, ticket).await?))
}

/// `GET /api/tickets/{id}`: by id, or by key (`PDT-12`, any case; PDT-271).
pub(crate) async fn get_one(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Ticket>, ApiError> {
    let ticket = match st.store.get_ticket(&id).await? {
        Some(t) => Some(t),
        None => match st.store.ticket_id_by_key(&id).await? {
            Some(id) => st.store.get_ticket(&id).await?,
            None => None,
        },
    };
    reply(&st, ticket.or_404("ticket")?).await
}

pub(crate) async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<TicketPatch>, JsonRejection>,
) -> Result<Json<Ticket>, ApiError> {
    let Json(patch) = payload?;
    let patch = patch.validate()?;
    // Only the user's own click lets an external ticket in (PDT-204): an agent's curl counts as the user otherwise.
    if !crate::untrusted::from_ui(&st, &headers)?
        && let Some(t) = st.store.get_ticket_summary(&id).await?
    {
        if crate::untrusted::lets_in(&t, &patch) {
            return Err(crate::untrusted::refused(&t.key));
        }
        // Nor does it decide a ticket proposal (PDT-366).
        crate::untrusted::decides_proposal(&t, &patch)?;
    }
    let since = now_ms();
    let t = st.store.patch_ticket(&id, &patch, since).await?.or_404("ticket")?;
    // Closing a ticket ends its attempt, once the close went through; moving it back before work
    // stops its work (PDT-159).
    match patch.stage {
        Some(stage) if stage.is_closed() => {
            crate::start::close_attempt(&st, &id, stage, since, Actor::User, None).await?;
            reply(&st, st.ticket(&id).await?).await
        }
        Some(stage) if stage.is_before_work() && t.attempts.iter().any(|a| a.state == AttemptState::Active) => {
            let task = tokio::spawn({
                let (st, id) = (st.clone(), id.clone());
                async move { crate::stop::after_move(&st, &id, stage, since, Actor::User).await }
            });
            task.await.map_err(ApiError::internal)?;
            reply(&st, st.ticket(&id).await?).await
        }
        _ => reply(&st, t).await,
    }
}

pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    match st.store.delete_ticket(&id).await? {
        DeleteTicket::Deleted => {
            crate::ticket_attachments::prune(&st).await;
            Ok(StatusCode::NO_CONTENT)
        }
        DeleteTicket::NotFound => Err(ApiError::not_found("ticket")),
        DeleteTicket::Active => {
            let key = st.store.get_ticket_summary(&id).await?.map(|t| t.key).unwrap_or(id);
            Err(TicketError::Active(key).into())
        }
        DeleteTicket::HasParts => {
            let key = st.store.get_ticket_summary(&id).await?.map(|t| t.key).unwrap_or(id);
            Err(TicketError::HasParts(key).into())
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
    reply(&st, t.or_404("ticket")?).await
}

pub(crate) async fn bulk(
    State(st): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<BulkRequest>, JsonRejection>,
) -> Result<Json<BulkResult>, ApiError> {
    let Json(req) = payload?;
    let action = req.validate()?;
    // As `update`: only the user's own click lets an external ticket in (PDT-204).
    let patch = match &action {
        ValidBulk::Stage(stage) => Some(ValidPatch { stage: Some(*stage), ..Default::default() }),
        ValidBulk::AddLabel(label) => Some(ValidPatch { labels: Some(vec![label.clone()]), ..Default::default() }),
        _ => None,
    };
    if let Some(patch) = patch
        && !crate::untrusted::from_ui(&st, &headers)?
    {
        for id in &req.ids {
            if let Some(t) = st.store.get_ticket_summary(id).await? {
                if crate::untrusted::lets_in(&t, &patch) {
                    return Err(crate::untrusted::refused(&t.key));
                }
                crate::untrusted::decides_proposal(&t, &patch)?;
            }
        }
    }
    let since = now_ms();
    let count = st.store.bulk_tickets(&req.ids, &action, since).await?;
    if matches!(action, ValidBulk::Delete) {
        crate::ticket_attachments::prune(&st).await;
    }
    if let ValidBulk::Stage(stage) = action
        && stage.is_closed()
    {
        // The tickets are closed: end every attempt there is, whatever one of them does.
        for id in &req.ids {
            if let Err(e) = crate::start::close_attempt(&st, id, stage, since, Actor::User, None).await {
                tracing::warn!(ticket = %id, error = %e.message(), "could not end the closed ticket's attempt");
            }
        }
    } else if let ValidBulk::Stage(stage) = action {
        // Moved back before work: their work stops (PDT-159).
        let ids = req.ids.clone();
        let task = tokio::spawn({
            let st = st.clone();
            async move {
                for id in &ids {
                    crate::stop::after_move(&st, id, stage, since, Actor::User).await;
                }
            }
        });
        task.await.map_err(ApiError::internal)?;
    }
    Ok(Json(BulkResult { count }))
}

pub(crate) async fn add_link(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TicketLink>, JsonRejection>,
) -> Result<Json<Ticket>, ApiError> {
    let Json(link) = payload?;
    let t = st.store.add_link(&id, link.to.trim(), link.kind, now_ms()).await?;
    reply(&st, t.or_404("ticket")?).await
}

pub(crate) async fn remove_link(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TicketLink>, JsonRejection>,
) -> Result<Json<Ticket>, ApiError> {
    let Json(link) = payload?;
    let t = st.store.remove_link(&id, link.to.trim(), link.kind, now_ms()).await?;
    reply(&st, t.or_404("ticket")?).await
}

pub(crate) async fn events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    query: Result<Query<EventQuery>, QueryRejection>,
) -> Result<Json<EventPage>, ApiError> {
    let Query(q) = query.map_err(|e| ApiError::invalid(e.body_text()))?;
    let limit = q.limit.unwrap_or(pdt_core::ticket::DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let cursor = q.cursor.as_deref().map(str::trim).filter(|c| !c.is_empty());
    let page = st.store.list_events(&id, cursor, limit, q.newest_first.unwrap_or(false)).await?;
    Ok(Json(page.or_404("ticket")?))
}

pub(crate) async fn comment(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<NewComment>, JsonRejection>,
) -> Result<(StatusCode, Json<TicketEvent>), ApiError> {
    let Json(c) = payload?;
    let body = valid_comment(&c.body)?;
    let actor = match c.by {
        Some(Actor::User) => Actor::User,
        None | Some(Actor::Agent) => Actor::Agent,
        Some(Actor::System) => {
            return Err(ApiError::invalid("only PDT writes system comments"));
        }
    };
    let event = st.store.add_comment_by(&id, &body, actor, None, now_ms()).await?.or_404("ticket")?;
    Ok((StatusCode::CREATED, Json(event)))
}

/// `POST /api/tickets/{id}/refine`: asks the project's pipeline to refine a Draft or Ready ticket.
/// Sets its pipeline stage to `refine` and logs `ticket.refine_requested`; an enabled workflow does the rest.
pub(crate) async fn refine(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Ticket>, ApiError> {
    let by = crate::host_guard::optional_json::<NewRefine>(&headers, &body)?.by;
    let actor = if by == Some(Actor::User) { Actor::User } else { Actor::Agent };
    let t = st.store.get_ticket_summary(&id).await?.or_404("ticket")?;
    if !matches!(t.stage, TicketStage::Draft | TicketStage::Ready) {
        return Err(ApiError::conflict(
            codes::TICKET_NOT_REFINABLE,
            format!("{} is not a Draft or Ready ticket", t.key),
        ));
    }
    let kind = pdt_core::workflow::events::TICKET_REFINE_REQUESTED;
    if !crate::scheduler::workflow_listens(&st, &t.project_id, kind).await {
        return Err(ApiError::conflict(
            codes::NO_REFINE_WORKFLOW,
            "no enabled workflow refines this project's tickets; enable the Pipeline in Workflows",
        ));
    }
    if t.pipeline_stage == Some(PipelineStage::Refine) {
        for status in [RunStatus::Running, RunStatus::Waiting] {
            let f =
                pdt_store::RunFilter { workflow_id: None, ticket_id: Some(id.clone()), status: Some(status), limit: 1 };
            if !st.store.list_workflow_runs(&f).await?.is_empty() {
                return Err(ApiError::conflict(codes::REFINE_RUNNING, format!("{} is being refined already", t.key)));
            }
        }
    }
    st.store.set_pipeline_stage(&id, Some(PipelineStage::Refine)).await?;
    let payload = pdt_core::workflow::events::TicketRefineRequested {
        project_id: t.project_id.clone(),
        ticket_id: t.id.clone(),
        ticket_key: t.key.clone(),
        title: t.title.clone(),
        stage: t.stage.as_str().into(),
        actor: actor.as_str().into(),
    };
    let key = format!("{}:{}", t.id, uuid::Uuid::now_v7());
    st.store.emit_event(&NewLogEvent::new(kind, key, "ticket", payload), now_ms()).await?;
    reply(&st, st.ticket(&id).await?).await
}

/// `POST /api/tickets/{id}/refine`'s optional body.
#[derive(Debug, Default, serde::Deserialize)]
struct NewRefine {
    /// `user` from PDT's UI; anything else is an agent.
    #[serde(default)]
    by: Option<Actor>,
}

pub(crate) async fn labels(
    State(st): State<AppState>,
    Path(project_id): Path<String>,
) -> Result<Json<Vec<String>>, ApiError> {
    st.project(&project_id).await?;
    Ok(Json(st.store.project_labels(&project_id).await?))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/projects/{id}/tickets", post(create))
        .route("/projects/{id}/labels", get(labels))
        .route("/tickets", get(list))
        .route("/tickets/bulk", post(bulk))
        .route("/tickets/{id}", get(get_one).patch(update).delete(remove))
        .route("/tickets/{id}/rank", post(rank))
        .route("/tickets/{id}/links", post(add_link).delete(remove_link))
        .route("/tickets/{id}/events", get(events))
        .route("/tickets/{id}/comments", post(comment))
        .route("/tickets/{id}/refine", post(refine))
}
