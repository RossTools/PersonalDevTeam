//! New releases on projects' repositories (PDT-365). pdtd polls, every 60 s, the forge of each project an enabled
//! workflow's `release.published` trigger covers, and logs `release.published` for each release (pre-releases too,
//! never drafts) it has not seen there. The first look at a project only records its releases
//! (`sensor.releases.<project>`), so turning a trigger on never replays a repository's history. A release whose event
//! could not be logged is left out of the record, so the next look tries it again.

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::parse;
use crate::workflow::nodes::trigger::TriggerConfig;
use pdt_core::release::notes_summary;
use pdt_core::workflow::api::{NewLogEvent, Scope};
use pdt_core::workflow::events::{RELEASE_PUBLISHED, RELEASE_SUMMARY_CHARS, ReleaseAssetRef, ReleasePublished};
use serde_json::json;
use std::collections::BTreeSet;

fn baseline_key(project_id: &str) -> String {
    format!("sensor.releases.{project_id}")
}

/// The projects enabled `release.published` triggers cover.
async fn watched_projects(st: &AppState, all: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for wf in st.store.enabled_workflows().await.unwrap_or_default() {
        let Ok(Some(doc)) = st.store.workflow_doc(&wf.id, wf.version).await else { continue };
        for node in doc.nodes.iter().filter(|n| n.kind == RELEASE_PUBLISHED) {
            let Ok(c) = st.registry().catalog().current_config(node).and_then(|c| parse::<TriggerConfig>(&c)) else {
                continue;
            };
            let covered = all.iter().filter(|p| match wf.scope {
                Scope::Project => wf.project_id.as_deref() == Some(p.as_str()),
                Scope::Host => true,
                Scope::Personal => false,
            });
            out.extend(covered.filter(|p| c.projects.is_empty() || c.projects.contains(p)).cloned());
        }
    }
    out
}

/// One look at the watched projects' recent releases. Never fails; problems are logged (a failing read once per
/// project until it works again).
pub async fn refresh_releases(st: &AppState) {
    let projects = st.store.list_projects().await.unwrap_or_default();
    let ids: Vec<String> = projects.iter().map(|p| p.id.clone()).collect();
    let watched = watched_projects(st, &ids).await;
    for p in projects.iter().filter(|p| watched.contains(&p.id)) {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Some(f) = crate::forge::project_forge(st, &p.id, &p.repo_path).await else { continue };
        if !f.api.capabilities().releases {
            continue;
        }
        let key = baseline_key(&p.id);
        let seen: Option<BTreeSet<String>> = match st.store.get_host_value(&key).await {
            Ok(v) => v.and_then(|v| serde_json::from_value(v["tags"].clone()).ok()),
            Err(e) => {
                tracing::warn!(project = %p.id, error = %e, "could not read which releases were seen");
                continue;
            }
        };
        let (api, repo) = (f.api.clone(), f.repo.clone());
        let what = format!("releases:{}", f.repo);
        let releases = match tokio::task::spawn_blocking(move || api.recent_releases(&repo)).await {
            Ok(Ok(r)) => {
                crate::github::gh_worked(st, &what);
                r
            }
            Ok(Err(e)) => {
                crate::github::gh_failed(st, &what, &e.to_string());
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "the releases task failed");
                continue;
            }
        };
        let mut now_seen: BTreeSet<String> = releases.iter().map(|r| r.tag.clone()).collect();
        if let Some(seen) = &seen {
            // Oldest first: the forge lists newest first.
            for r in releases.iter().rev().filter(|r| !seen.contains(&r.tag)) {
                let payload = ReleasePublished {
                    project_id: p.id.clone(),
                    forge: f.kind.to_string(),
                    repo: f.repo.clone(),
                    tag: r.tag.clone(),
                    name: r.name.clone(),
                    url: r.url.clone(),
                    prerelease: r.prerelease,
                    published_at: r.published_at.clone(),
                    notes: r.notes.clone(),
                    summary: notes_summary(&r.notes, RELEASE_SUMMARY_CHARS),
                    assets: r
                        .assets
                        .iter()
                        .map(|a| ReleaseAssetRef { name: a.name.clone(), url: a.url.clone(), size: a.size })
                        .collect(),
                };
                let e = NewLogEvent::new(
                    RELEASE_PUBLISHED,
                    format!("release:{}:{}", p.id, r.tag),
                    "forge_releases",
                    payload,
                );
                if let Err(x) = st.store.emit_event(&e, now_ms()).await {
                    tracing::warn!(project = %p.id, tag = %r.tag, error = %x, "could not log a new release");
                    now_seen.remove(&r.tag);
                }
            }
        }
        if seen.as_ref() != Some(&now_seen)
            && let Err(e) = st.store.put_host_value(&key, &json!({"tags": now_seen})).await
        {
            tracing::warn!(project = %p.id, error = %e, "could not record which releases were seen");
        }
    }
}
