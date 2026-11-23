//! Placement on the owner (PDT-632): which instance a ticket's attempt runs on (the rules are pdt-core `placement`),
//! where pickup would start each candidate (PDT-636), and a project's label rules, `GET`/`PUT /api/projects/{id}/placement`.

use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use pdt_core::link::{InstanceFacts, LinkState, RemoteInstance};
use pdt_core::pickup::{InstancePool, Place};
use pdt_core::placement::{Candidate, Placed, PlacementError, PlacementSettings};
use pdt_core::project::Project;

/// What placement needs to know of a project, read once: its joined instances, label rules, the repository's needs
/// and this instance's own facts (only read when the repository has needs).
pub(crate) struct Placer {
    pub instances: Vec<RemoteInstance>,
    rules: Vec<pdt_core::placement::LabelRule>,
    repo: Vec<pdt_core::placement::RepoRule>,
    own: Option<InstanceFacts>,
    now: i64,
}

impl Placer {
    pub(crate) async fn load(st: &AppState, project: &Project) -> Result<Self, ApiError> {
        let instances = st.store.list_remote_instances(Some(&project.id)).await?;
        let rules = st.store.get_project_placement(&project.id).await?.rules;
        let repo = match crate::runconfig::load_effective(st, &project.id, std::path::Path::new(&project.repo_path))
            .await
        {
            Ok(c) => c.placement,
            Err(e) => {
                tracing::warn!(project = %project.key, error = %e.message(), "no placement needs: the configuration can't be read");
                vec![]
            }
        };
        // This instance's facts count for the repository's needs only.
        let own = if repo.is_empty() { None } else { Some(crate::link::facts(st).await) };
        Ok(Placer { instances, rules, repo, own, now: now_ms() })
    }

    /// Where a ticket with `labels` goes, from the Start dialog's `runs_on`, the project's label rules and the
    /// repository's `[[placement]]` needs.
    pub(crate) fn place(&self, labels: &[String], runs_on: Option<&str>) -> Result<Placed, PlacementError> {
        let home = runs_on.is_none_or(|r| r.eq_ignore_ascii_case(pdt_core::placement::OWNER));
        if self.instances.is_empty() && self.repo.is_empty() && home {
            return Ok(Placed::Owner);
        }
        let candidates: Vec<Candidate<'_>> = self
            .instances
            .iter()
            .map(|i| {
                let state = pdt_core::link::state(i.last_seen_at, true, i.paused, self.now);
                Candidate {
                    id: &i.instance_id,
                    name: &i.name,
                    facts: i.facts.as_ref(),
                    available: state == LinkState::Connected,
                    paused: state == LinkState::Paused,
                }
            })
            .collect();
        pdt_core::placement::place(runs_on, labels, &self.rules, &self.repo, &candidates, self.own.as_ref())
    }

    /// Where pickup would start a ticket with `labels` (PDT-636): each place is a slot pool of its own.
    pub(crate) fn pickup_place(&self, labels: &[String]) -> Place {
        let name = |id: &str| self.instances.iter().find(|i| i.instance_id == id).map(|i| i.name.clone());
        match self.place(labels, None) {
            Ok(Placed::Owner) => Place::Owner,
            Ok(Placed::Remote { id, .. }) => Place::Instance { name: name(&id).unwrap_or_else(|| id.clone()), id },
            Err(PlacementError::Unavailable { id, name, paused }) => Place::Waiting { id, name, paused },
            Err(e) => Place::NoInstance { why: e.to_string() },
        }
    }

    /// The slot pools of the joined instances.
    pub(crate) fn pools(&self) -> Vec<InstancePool> {
        self.instances
            .iter()
            .map(|i| InstancePool { id: i.instance_id.clone(), name: i.name.clone(), cap: i.pool_cap() })
            .collect()
    }
}

/// The remote instance a ticket with `labels` runs on (None: the owner itself), from the Start dialog's `runs_on`, the
/// project's label rules and the repository's `[[placement]]` needs.
pub(crate) async fn place(
    st: &AppState,
    project: &Project,
    labels: &[String],
    runs_on: Option<&str>,
) -> Result<Option<RemoteInstance>, ApiError> {
    let placer = Placer::load(st, project).await?;
    let placed = placer.place(labels, runs_on).map_err(|e| ApiError::conflict(codes::PLACEMENT, e.to_string()))?;
    Ok(match placed {
        Placed::Owner => None,
        Placed::Remote { id, .. } => placer.instances.into_iter().find(|i| i.instance_id == id),
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
