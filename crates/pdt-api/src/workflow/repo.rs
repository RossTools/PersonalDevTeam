//! Workflows kept in a project's repo (Phase 7a spec §3.3): each `.pdt/workflows/<name>.json` in the main checkout is
//! a workflow with `source: repo`, read-only in PDT. A changed file is a new version (author `repo`); a new one starts
//! disabled (enabling stays the user's choice on this host); a removed file removes its workflow.

use crate::routes::{AppState, now_ms};
use pdt_core::workflow::api::{Author, Scope, Source, Workflow};
use pdt_core::workflow::doc::WorkflowDoc;
use std::collections::HashSet;
use std::path::Path;

pub const DIR: &str = ".pdt/workflows";

/// The documents in a repo's `.pdt/workflows/` (file name relative to the repo, document); unreadable ones are
/// logged and left out.
pub(crate) fn read_dir(repo: &Path) -> Vec<(String, WorkflowDoc)> {
    let Ok(entries) = std::fs::read_dir(repo.join(DIR)) else { return vec![] };
    let mut out = vec![];
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let file = format!("{DIR}/{name}");
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<WorkflowDoc>(&t).map_err(|e| e.to_string()))
        {
            Ok(doc) => out.push((file, doc)),
            Err(e) => tracing::debug!(file = %path.display(), error = %e, "skipped a workflow file that does not read"),
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Syncs one project's repo workflows.
pub(crate) async fn sync_project(
    st: &AppState,
    project: &pdt_core::project::Project,
) -> Result<(), pdt_store::StoreError> {
    let repo = std::path::PathBuf::from(&project.repo_path);
    let found = tokio::task::spawn_blocking(move || read_dir(&repo)).await.unwrap_or_default();
    let now = now_ms();
    let mut seen = HashSet::new();
    for (file, doc) in found {
        seen.insert(file.clone());
        match st.store.workflow_by_repo_file(&project.id, &file).await? {
            Some(w) => {
                if st.store.workflow_doc(&w.id, w.version).await?.as_ref() != Some(&doc) {
                    st.store.save_workflow_version(&w.id, &doc, Author::Repo, "changed in the repo", now).await?;
                }
            }
            None => {
                let w = Workflow {
                    id: uuid::Uuid::now_v7().to_string(),
                    name: doc.name.clone(),
                    description: doc.description.clone(),
                    scope: Scope::Project,
                    project_id: Some(project.id.clone()),
                    enabled: false,
                    builtin: None,
                    source: Source::Repo,
                    repo_file: Some(file.clone()),
                    version: 1,
                    proposed: None,
                    created_at: now,
                    updated_at: now,
                };
                st.store.insert_workflow(&w, &doc, Author::Repo, "read from the repo").await?;
                tracing::info!(project = %project.name, file = %file, "found a workflow in the repo (disabled until enabled)");
            }
        }
    }
    for w in st.store.list_workflows(Some(&project.id)).await? {
        if w.source == Source::Repo
            && w.project_id.as_deref() == Some(&project.id)
            && !w.repo_file.as_ref().is_some_and(|f| seen.contains(f))
        {
            st.store.delete_workflow(&w.id).await?;
            tracing::info!(project = %project.name, workflow = %w.name, "its file left the repo: removed the workflow");
        }
    }
    Ok(())
}

pub(crate) async fn sync_all(st: &AppState) {
    let projects = match st.store.list_projects().await {
        Ok(p) => p,
        Err(e) => return tracing::warn!(error = %e, "could not list projects to read their workflows"),
    };
    for p in projects {
        if let Err(e) = sync_project(st, &p).await {
            tracing::warn!(project = %p.name, error = %e, "could not read the repo's workflows");
        }
    }
}
