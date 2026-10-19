//! Workspace and tab routes (workspaces spec §5).

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::workspace::{
    NewWorkspace, OpenTab, Tab, TabItem, TabPatch, TabPlacement, Workspace, WorkspacePatch, valid_workspace_name,
};
use pdt_store::DeleteWorkspace;

fn unknown_workspace() -> ApiError {
    ApiError::not_found("workspace")
}

pub(crate) async fn list(State(st): State<AppState>) -> Result<Json<Vec<Workspace>>, ApiError> {
    let mut list = st.store.list_workspaces().await?;
    crate::github::apply_workspaces(&st, &mut list);
    Ok(Json(list))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewWorkspace>, JsonRejection>,
) -> Result<(StatusCode, Json<Workspace>), ApiError> {
    let Json(input) = payload?;
    let w = Workspace {
        id: uuid::Uuid::now_v7().to_string(),
        name: valid_workspace_name(&input.name)?,
        project_id: None,
        position: 0,
        created_at: now_ms(),
        tabs: vec![],
        ticket: None,
    };
    st.store.insert_workspace(&w).await?;
    let w = st.store.get_workspace(&w.id).await?.ok_or_else(unknown_workspace)?;
    Ok((StatusCode::CREATED, Json(w)))
}

pub(crate) async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<WorkspacePatch>, JsonRejection>,
) -> Result<Json<Workspace>, ApiError> {
    let Json(patch) = payload?;
    let name = patch.name.as_deref().map(valid_workspace_name).transpose()?;
    st.store.get_workspace(&id).await?.ok_or_else(unknown_workspace)?;
    if let Some(name) = name {
        st.store.rename_workspace(&id, &name).await?;
    }
    if let Some(pos) = patch.position {
        st.store.move_workspace(&id, pos).await?;
    }
    let mut w = st.store.get_workspace(&id).await?.ok_or_else(unknown_workspace)?;
    crate::github::apply_workspaces(&st, std::slice::from_mut(&mut w));
    Ok(Json(w))
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct RemoveQuery {
    /// Close the workspace's tabs instead of moving them to their projects' workspaces.
    close_tabs: bool,
}

pub(crate) async fn remove(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<RemoveQuery>,
) -> Result<StatusCode, ApiError> {
    match st.store.delete_workspace(&id, q.close_tabs).await? {
        DeleteWorkspace::Deleted => Ok(StatusCode::NO_CONTENT),
        DeleteWorkspace::NotFound => Err(unknown_workspace()),
        DeleteWorkspace::ProjectWorkspace => Err(ApiError::new(
            StatusCode::CONFLICT,
            "project_workspace",
            "a project's own workspace goes only with the project",
        )),
    }
}

async fn item_exists(st: &AppState, item: &TabItem) -> Result<bool, ApiError> {
    Ok(match item {
        TabItem::Session(id) => st.store.get_session(id).await?.is_some(),
        TabItem::Run(id) => st.store.get_run(id).await?.is_some(),
        TabItem::Ticket(id) => st.store.get_ticket_summary(id).await?.is_some(),
    })
}

pub(crate) async fn open_tab(
    State(st): State<AppState>,
    Path(workspace_id): Path<String>,
    payload: Result<Json<OpenTab>, JsonRejection>,
) -> Result<Json<Tab>, ApiError> {
    let Json(input) = payload?;
    let item = input.item()?;
    if !item_exists(&st, &item).await? {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "that session, run or ticket does not exist",
        ));
    }
    let tab = st
        .store
        .open_tab(&uuid::Uuid::now_v7().to_string(), &workspace_id, &item, input.position, now_ms())
        .await?
        .ok_or_else(unknown_workspace)?;
    Ok(Json(tab))
}

pub(crate) async fn update_tab(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TabPatch>, JsonRejection>,
) -> Result<Json<Tab>, ApiError> {
    let Json(patch) = payload?;
    st.store.get_tab(&id).await?.ok_or_else(|| ApiError::not_found("tab"))?;
    let tab = st.store.move_tab(&id, patch.workspace_id.as_deref(), patch.position).await?;
    Ok(Json(tab.ok_or_else(unknown_workspace)?))
}

/// Closes a tab. A session or run tab also stops and removes what it shows, so nothing is left without a tab (PDT-28);
/// a ticket tab only closes. If the item cannot be stopped the tab stays open.
pub(crate) async fn close_tab(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let tab = st.store.get_tab(&id).await?.ok_or_else(|| ApiError::not_found("tab"))?;
    if let Some(sid) = &tab.session_id {
        crate::sessions::stop_and_remove(&st, sid).await?;
    } else if let Some(rid) = &tab.run_id {
        crate::runs::stop_and_remove(&st, rid).await?;
    }
    // Removing the item deleted its tab with it (cascade); a ticket tab is closed here.
    if !st.store.close_tab(&id).await? {
        st.store.renumber_workspace_tabs(&tab.workspace_id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Checks a requested placement before anything is created.
pub(crate) async fn check_placement(st: &AppState, placement: Option<&TabPlacement>) -> Result<(), ApiError> {
    if let Some(p) = placement
        && st.store.get_workspace(&p.workspace_id).await?.is_none()
    {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", "the tab's workspace does not exist"));
    }
    Ok(())
}

/// Opens the tab of a newly created session or run: where requested, else at the end of its project's workspace.
/// A failure is logged, not returned: the item exists and the user can still open it from the tree.
pub(crate) async fn open_for_new(st: &AppState, project_id: &str, item: TabItem, placement: Option<&TabPlacement>) {
    let target = match placement {
        Some(p) => Some((p.workspace_id.clone(), p.position)),
        None => st.store.project_workspace_id(project_id).await.ok().flatten().map(|id| (id, None)),
    };
    let Some((workspace_id, position)) = target else {
        tracing::warn!(project = %project_id, "no workspace to open the new tab in");
        return;
    };
    let id = uuid::Uuid::now_v7().to_string();
    if let Err(e) = st.store.open_tab(&id, &workspace_id, &item, position, now_ms()).await {
        tracing::warn!(error = %e, "could not open a tab for the new item");
    }
}
