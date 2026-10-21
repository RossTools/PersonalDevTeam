//! Areas and overlap (tickets spec §7): the project's configured areas, each active attempt's changed files (kept in
//! memory, refreshed every 30 s by pdtd), and the per-ticket overlaps every ticket response and the backlog carry.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::{Path, State};
use pdt_core::overlap::{AreaSet, Participant, compute};
use pdt_core::project::Project;
use pdt_core::runconfig::ProjectAreas;
use pdt_core::ticket::{Actor, EventKind, MAX_OVERLAP_FILES, OverlapLevel, Ticket, TicketOverlap, TicketSummary};
use pdt_store::OverlapRow;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

/// Changed files per active attempt id.
pub(crate) type ChangedFiles = std::sync::Arc<std::sync::RwLock<HashMap<String, Vec<String>>>>;

/// The areas the project's main checkout and the host overrides declare; an invalid configuration gives none, with
/// the reason.
pub(crate) async fn project_areas(st: &AppState, project: &Project) -> ProjectAreas {
    let repo = PathBuf::from(&project.repo_path);
    match crate::runconfig::load_effective(st, &project.id, &repo).await {
        Ok(cfg) => ProjectAreas { areas: cfg.areas, error: None },
        Err(e) => ProjectAreas { areas: vec![], error: Some(e.message().to_string()) },
    }
}

/// `GET /api/projects/{id}/areas`.
pub(crate) async fn areas(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<ProjectAreas>, ApiError> {
    let project = st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    Ok(Json(project_areas(&st, &project).await))
}

/// Every open ticket's overlaps with the tickets of its project that have an active attempt.
#[derive(Debug, Default)]
pub(crate) struct OverlapIndex {
    by_ticket: HashMap<String, Vec<TicketOverlap>>,
}

impl OverlapIndex {
    pub(crate) fn of(&self, ticket_id: &str) -> &[TicketOverlap] {
        self.by_ticket.get(ticket_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The tickets with any overlap.
    pub(crate) fn ids(&self) -> Vec<String> {
        self.by_ticket.keys().cloned().collect()
    }

    pub(crate) fn apply_summary(&self, s: &mut TicketSummary) {
        let list = self.of(&s.id);
        s.overlap = list.iter().map(|o| o.level).max().unwrap_or_default();
        s.overlap_with = list.iter().map(|o| o.key.clone()).collect();
        s.clear_to_start = s.waiting && s.blocked_by == 0 && s.overlap == OverlapLevel::None;
    }

    pub(crate) fn apply(&self, t: &mut Ticket) {
        self.apply_summary(&mut t.summary);
        t.overlaps = self.of(&t.summary.id).to_vec();
    }
}

/// Computes the overlaps of every open ticket (or one project's).
pub(crate) async fn index(st: &AppState, project: Option<&str>) -> Result<OverlapIndex, ApiError> {
    let rows = st.store.overlap_rows(project).await?;
    let mut by_project: HashMap<&str, Vec<&OverlapRow>> = HashMap::new();
    for r in &rows {
        by_project.entry(r.project_id.as_str()).or_default().push(r);
    }
    let files = st.changed_files.read().map(|m| m.clone()).unwrap_or_default();
    let mut by_ticket = HashMap::new();
    for (project_id, rows) in by_project {
        let Some(project) = st.store.get_project(project_id).await? else { continue };
        let areas = AreaSet::new(&project_areas(st, &project).await.areas);
        let participants: Vec<Participant> = rows
            .iter()
            .map(|r| Participant {
                id: r.ticket_id.clone(),
                active: r.attempt_id.is_some(),
                areas: r.areas.clone(),
                files: r.attempt_id.as_ref().and_then(|a| files.get(a).cloned()).unwrap_or_default(),
            })
            .collect();
        let found = compute(&areas, &participants);
        let info: HashMap<&str, &&OverlapRow> = rows.iter().map(|r| (r.ticket_id.as_str(), r)).collect();
        for (id, list) in found {
            let mut out: Vec<(u32, TicketOverlap)> = list
                .into_iter()
                .filter_map(|o| {
                    let other = info.get(o.other.as_str())?;
                    let total = o.files.len();
                    Some((
                        other.number,
                        TicketOverlap {
                            ticket_id: o.other,
                            key: other.key.clone(),
                            title: other.title.clone(),
                            stage: other.stage,
                            level: o.level,
                            areas: o.areas,
                            files: o.files.into_iter().take(MAX_OVERLAP_FILES).collect(),
                            files_total: u32::try_from(total).unwrap_or(u32::MAX),
                        },
                    ))
                })
                .collect();
            out.sort_by(|(na, a), (nb, b)| b.level.cmp(&a.level).then(na.cmp(nb)));
            by_ticket.insert(id, out.into_iter().map(|(_, o)| o).collect());
        }
    }
    Ok(OverlapIndex { by_ticket })
}

/// The ticket with its overlap fields filled in.
pub(crate) async fn decorate(st: &AppState, mut t: Ticket) -> Result<Ticket, ApiError> {
    index(st, Some(&t.summary.project_id)).await?.apply(&mut t);
    crate::github::apply(st, &mut t);
    crate::parts::apply(st, &mut t).await?;
    crate::scheduler::apply_agents(st, std::slice::from_mut(&mut t.summary)).await;
    Ok(t)
}

/// Recomputes every active attempt's changed files, then records new actual overlaps on both tickets' timelines
/// (once per pair and file set). pdtd runs it every 30 s. Never fails; problems are logged.
pub async fn refresh_overlaps(st: &AppState) {
    let active = match st.store.list_active_attempts().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "the overlap check could not list active attempts");
            return;
        }
    };
    let mut fresh: HashMap<String, Vec<String>> = HashMap::new();
    for info in active {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Some(path) = info.worktree_path.clone() else { continue };
        let (base, start) = (info.attempt.base_branch.clone(), info.base_commit.clone());
        let found =
            tokio::task::spawn_blocking(move || pdt_git::changed_files(std::path::Path::new(&path), &base, &start))
                .await;
        match found {
            Ok(Ok(files)) => {
                fresh.insert(info.attempt.id.clone(), files);
            }
            Ok(Err(e)) => tracing::debug!(attempt = %info.attempt.id, error = %e, "could not list changed files"),
            Err(e) => tracing::warn!(error = %e, "the changed-files task failed"),
        }
    }
    if let Ok(mut m) = st.changed_files.write() {
        *m = fresh;
    }
    if let Err(e) = record_overlap_events(st).await {
        tracing::warn!(error = %e.message(), "could not record overlap events");
    }
}

async fn record_overlap_events(st: &AppState) -> Result<(), ApiError> {
    let idx = index(st, None).await?;
    for (ticket_id, list) in &idx.by_ticket {
        for o in list.iter().filter(|o| !o.files.is_empty()) {
            if st.store.last_overlap_files(ticket_id, &o.ticket_id).await?.as_deref() == Some(o.files.as_slice()) {
                continue;
            }
            let data = json!({"ticketId": o.ticket_id, "key": o.key, "files": o.files, "level": o.level});
            st.store.add_ticket_event(ticket_id, EventKind::Overlap, Actor::System, None, Some(data), now_ms()).await?;
        }
    }
    Ok(())
}

/// One line naming a ticket's overlaps, for Start's refusal: `overlaps PDT-2 (web), conflicts with PDT-4
/// (migrations; 3 files)`.
pub(crate) fn describe(list: &[TicketOverlap]) -> String {
    list.iter()
        .map(|o| {
            let verb = if o.level == OverlapLevel::Conflict { "conflicts with" } else { "overlaps" };
            let mut what: Vec<String> = vec![];
            if !o.areas.is_empty() {
                what.push(o.areas.join(", "));
            }
            if o.files_total > 0 {
                what.push(format!("{} file{}", o.files_total, if o.files_total == 1 { "" } else { "s" }));
            }
            format!("{verb} {} ({})", o.key, what.join("; "))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The worst level in a list (for Start's event).
pub(crate) fn worst(list: &[TicketOverlap]) -> OverlapLevel {
    list.iter().map(|o| o.level).max().unwrap_or_default()
}
