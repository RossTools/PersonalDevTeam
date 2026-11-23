//! Placement on the owner (PDT-632): which instance a ticket's attempt runs on (the rules are pdt-core `placement`),
//! and a project's label rules, `GET`/`PUT /api/projects/{id}/placement`.

use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use pdt_core::link::RemoteInstance;
use pdt_core::placement::{Candidate, Placed, PlacementSettings};
use pdt_core::project::Project;

/// The remote instance a ticket with `labels` runs on (None: the owner itself), from the Start dialog's `runs_on`, the
/// project's label rules and the repository's `[[placement]]` needs.
pub(crate) async fn place(
    st: &AppState,
    project: &Project,
    labels: &[String],
    runs_on: Option<&str>,
) -> Result<Option<RemoteInstance>, ApiError> {
    let instances = st.store.list_remote_instances(Some(&project.id)).await?;
    let rules = st.store.get_project_placement(&project.id).await?.rules;
    if instances.is_empty() && runs_on.is_none_or(|r| r.eq_ignore_ascii_case(pdt_core::placement::OWNER)) {
        return Ok(None);
    }
    let repo = match crate::runconfig::load_effective(st, &project.id, std::path::Path::new(&project.repo_path)).await {
        Ok(c) => c.placement,
        Err(e) => {
            tracing::warn!(project = %project.key, error = %e.message(), "no placement needs: the configuration can't be read");
            vec![]
        }
    };
    let now = now_ms();
    let candidates: Vec<Candidate<'_>> = instances
        .iter()
        .map(|i| Candidate {
            id: &i.instance_id,
            name: &i.name,
            facts: i.facts.as_ref(),
            available: pdt_core::link::state(i.last_seen_at, true, i.paused, now)
                == pdt_core::link::LinkState::Connected,
        })
        .collect();
    let own = crate::link::facts(st).await;
    let placed = pdt_core::placement::place(runs_on, labels, &rules, &repo, &candidates, Some(&own))
        .map_err(|e| ApiError::conflict(codes::PLACEMENT, e.to_string()))?;
    Ok(match placed {
        Placed::Owner => None,
        Placed::Remote { id, .. } => instances.into_iter().find(|i| i.instance_id == id),
    })
}

/// `GET /api/projects/{id}/placement`.
async fn get_settings(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<PlacementSettings>, ApiError> {
    st.project(&id).await?;
    Ok(Json(st.store.get_project_placement(&id).await?))
}

/// `PUT /api/projects/{id}/placement`: replaces the label rules. Each names a joined instance.
async fn put_settings(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<PlacementSettings>, JsonRejection>,
) -> Result<Json<PlacementSettings>, ApiError> {
    let Json(s) = payload?;
    st.project(&id).await?;
    let s = s.validate().map_err(ApiError::invalid)?;
    let joined = st.store.list_remote_instances(Some(&id)).await?;
    if let Some(r) = s.rules.iter().find(|r| !joined.iter().any(|i| i.instance_id == r.instance)) {
        return Err(ApiError::invalid(format!("the rule for {} names an instance that hasn't joined", r.label)));
    }
    st.store.put_project_placement(&id, &s).await?;
    Ok(Json(s))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new().route("/projects/{id}/placement", get(get_settings).put(put_settings))
}
