//! Ticket parts in the API (PDT-38; docs/superpowers/specs/2026-10-19-ticket-parts-design.md): the parents' roll-up
//! on every ticket response (it needs the in-memory PR state, so it is computed here, like overlap), the part a
//! parent's "Start next part" picks, a part's prompt context, and `POST /api/tickets/{id}/parts`.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::parts::{
    NewParts, NoNextPart, PartFacts, PartState, PartsCreated, PartsProgress, next_part, part_state, progress,
};
use pdt_core::pr::PrState;
use pdt_core::start::PartContext;
use pdt_core::ticket::{AttemptState, Ticket, TicketSummary};
use pdt_store::PartRow;
use std::collections::HashMap;

/// A part's state, with its PR as pdtd last saw it.
fn state_of(st: &AppState, row: &PartRow) -> (PartState, Option<u32>) {
    let pr = row.attempt.as_ref().and_then(|(id, _)| crate::github::watched_pr(st, id));
    let open = row.attempt.as_ref().is_some_and(|(_, s)| *s == AttemptState::Active)
        && pr.as_ref().is_some_and(|p| p.state == PrState::Open);
    (part_state(row.stage, row.attempt.as_ref().map(|(_, s)| *s), open), pr.map(|p| p.number))
}

/// The roll-up of each of `parents` that has parts.
async fn rollups(st: &AppState, parents: &[String]) -> Result<HashMap<String, PartsProgress>, ApiError> {
    let mut by_parent: HashMap<String, Vec<PartState>> = HashMap::new();
    for row in st.store.part_rows(parents).await? {
        let (state, _) = state_of(st, &row);
        by_parent.entry(row.parent_id).or_default().push(state);
    }
    Ok(by_parent.into_iter().map(|(id, states)| (id, progress(&states))).collect())
}

/// Fills in `parts` on backlog rows.
pub(crate) async fn apply_page(st: &AppState, items: &mut [TicketSummary]) -> Result<(), ApiError> {
    let ids: Vec<String> = items.iter().map(|t| t.id.clone()).collect();
    let mut roll = rollups(st, &ids).await?;
    for t in items.iter_mut() {
        t.parts = roll.remove(&t.id);
    }
    Ok(())
}

/// Fills in a full ticket's roll-up and its parts' PRs.
pub(crate) async fn apply(st: &AppState, t: &mut Ticket) -> Result<(), ApiError> {
    if t.part_list.is_empty() {
        t.summary.parts = None;
        return Ok(());
    }
    t.summary.parts = rollups(st, std::slice::from_ref(&t.summary.id)).await?.remove(&t.summary.id);
    for p in t.part_list.iter_mut() {
        crate::github::apply_summary(st, p);
    }
    Ok(())
}

fn label(state: PartState, pr: Option<u32>) -> String {
    let pr = pr.map(|n| format!(" (#{n})")).unwrap_or_default();
    match state {
        PartState::Done => format!("Done{pr}"),
        PartState::Merged => format!("Merged{pr}"),
        PartState::Review => format!("In review{pr}"),
        PartState::Progress => format!("In progress{pr}"),
        PartState::Todo => "To do".into(),
        PartState::Backlog => "Not started".into(),
    }
}

/// The "Part of" block of a part's first prompt: its parent and the parent's other parts. None for a ticket that is
/// not a part.
pub(crate) async fn context(st: &AppState, part: &Ticket) -> Result<Option<PartContext>, ApiError> {
    let Some(parent) = &part.summary.parent else { return Ok(None) };
    let Some(p) = st.store.get_ticket(&parent.id).await? else { return Ok(None) };
    let others = st
        .store
        .part_rows(std::slice::from_ref(&parent.id))
        .await?
        .into_iter()
        .filter(|r| r.id != part.summary.id)
        .map(|r| {
            let (state, pr) = state_of(st, &r);
            format!("{} {}: {}", r.key, r.title, label(state, pr))
        })
        .collect();
    Ok(Some(PartContext {
        parent_key: p.summary.key,
        parent_title: p.summary.title,
        parent_description: p.description,
        others,
    }))
}

/// The id of the part "Start next part" starts on `parent`.
pub(crate) async fn next(st: &AppState, parent: &str) -> Result<Result<String, NoNextPart>, ApiError> {
    let rows = st.store.part_rows(&[parent.to_string()]).await?;
    let facts: Vec<PartFacts> = rows
        .iter()
        .map(|r| PartFacts {
            key: r.key.clone(),
            state: state_of(st, r).0,
            active: r.attempt.as_ref().is_some_and(|(_, s)| *s == AttemptState::Active),
            open_blockers: r.open_blockers,
        })
        .collect();
    Ok(next_part(&facts).map(|i| rows[i].id.clone()))
}

/// `POST /api/tickets/{id}/parts`: makes several parts in order after the existing ones (all or none).
pub(crate) async fn create(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<NewParts>, JsonRejection>,
) -> Result<(StatusCode, Json<PartsCreated>), ApiError> {
    let Json(input) = payload?;
    let v = input.validate()?;
    let mut items = st.store.create_parts(&id, &v, now_ms()).await?;
    for t in items.iter_mut() {
        crate::github::apply_summary(&st, t);
    }
    Ok((StatusCode::CREATED, Json(PartsCreated { items })))
}
