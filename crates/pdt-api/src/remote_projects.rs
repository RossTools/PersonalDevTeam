//! The remote instance's side of remote projects (PDT-631): Add project → Remote project of another PDT (the owner's
//! address and the code it shows), the remote projects with their link's state, Pause, and Remove (which tells the
//! owner and revokes the link). A remote project is a project of kind `remote`: it is never in `list_projects`, and
//! `AppState::project` refuses it, so this instance's own PM, workflows, pickup and Inbox never see it.

use crate::error::{ApiError, codes};
use crate::extract::{ApiJson, ApiPath};
use crate::link;
use crate::remote_instances::LinkEnded;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use pdt_core::link::{LinkState, OwnerLink, PairReply, PairRequest};
use pdt_core::project::{Project, ProjectKind};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A remote project, as this instance lists it.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteProjectView {
    pub project: Project,
    pub link: OwnerLink,
    /// Connected while the owner's heartbeats arrive (or paused here); offline once they stop.
    pub state: LinkState,
    /// The owner's board, for "Open on <owner>".
    pub open_url: String,
    /// Where this instance keeps the project's checkout and worktrees (the link's setting, else the default).
    pub checkout_path: String,
    /// The owner's sessions this instance runs now (read-only here).
    pub running: Vec<crate::exec::RunningView>,
}

/// Add project → Remote project of another PDT.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JoinRemoteProject {
    /// The owner's address: its tailnet name (`studio.example.ts.net`) or an `https://` URL.
    pub owner_address: String,
    /// The code the owner shows under Project Settings → Remote instances → Add.
    pub code: String,
    /// Where the owner reaches this instance; this instance's remote-access address when absent.
    #[serde(default)]
    #[ts(optional)]
    pub address: Option<String>,
}

/// Pause or resume a remote project here, move where its checkout goes, or pick whose update channel the owner's
/// updates use.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteProjectPatch {
    #[serde(default)]
    #[ts(optional)]
    pub paused: Option<bool>,
    /// The checkout's folder; empty: the default one. Used from the next worktree made (a checkout already there
    /// stays where it is).
    #[serde(default)]
    #[ts(optional)]
    pub checkout_path: Option<String>,
    /// An update the owner asks for uses this instance's own update channel (false: the owner's).
    #[serde(default)]
    #[ts(optional)]
    pub own_update_channel: Option<bool>,
    /// The most of the project's tickets this instance takes at once; 0: no cap of its own (the owner's decides).
    #[serde(default)]
    #[ts(optional)]
    pub cap: Option<u32>,
}

async fn view_of(st: &AppState, project: Project, link: OwnerLink, now: i64) -> RemoteProjectView {
    let state = match link.removed_at {
        Some(_) => LinkState::Removed,
        None => pdt_core::link::state(link.last_contact_at, true, link.paused, now),
    };
    let open_url = format!("{}/tickets?view=board", link.owner_url);
    let checkout_path = crate::exec::checkout_dir(st, &link).to_string_lossy().into_owned();
    let running = crate::exec::running_view(st, &link.link_id).await;
    RemoteProjectView { project, link, state, open_url, checkout_path, running }
}

async fn list(State(st): State<AppState>) -> Result<Json<Vec<RemoteProjectView>>, ApiError> {
    let now = now_ms();
    let mut out = vec![];
    for (p, l) in st.store.list_remote_projects().await? {
        out.push(view_of(&st, p, l, now).await);
    }
    Ok(Json(out))
}

async fn one(st: &AppState, id: &str) -> Result<(Project, OwnerLink), ApiError> {
    let p = st.store.get_remote_project(id).await?.ok_or_else(|| ApiError::not_found("remote project"))?;
    let l = st.store.get_owner_link(id).await?.ok_or_else(|| ApiError::not_found("remote project"))?;
    Ok((p, l))
}

/// A local project of this instance's on the same repository as `repo_url`, if there is one.
async fn local_with_repo(st: &AppState, repo_url: &str) -> Result<Option<Project>, ApiError> {
    for p in st.store.list_projects().await?.into_iter().filter(Project::has_repo) {
        let path = std::path::PathBuf::from(&p.repo_path);
        let origin = tokio::task::spawn_blocking(move || pdt_git::remote_url(&path, "origin"))
            .await
            .map_err(ApiError::internal)?;
        if origin.is_some_and(|o| pdt_core::link::same_repo(&o, repo_url)) {
            return Ok(Some(p));
        }
    }
    Ok(None)
}

/// Refuses to add `repo` as a project of this instance's own when it is already a remote project here: an instance
/// has a project either local or remote, never both.
pub(crate) async fn refuse_if_remote(st: &AppState, repo: &std::path::Path) -> Result<(), ApiError> {
    let path = repo.to_path_buf();
    let Some(origin) =
        tokio::task::spawn_blocking(move || pdt_git::remote_url(&path, "origin")).await.map_err(ApiError::internal)?
    else {
        return Ok(());
    };
    for (p, l) in st.store.list_remote_projects().await? {
        if l.repo_url.as_deref().is_some_and(|u| pdt_core::link::same_repo(u, &origin)) {
            return Err(ApiError::conflict(
                codes::REMOTE_PROJECT,
                format!(
                    "{} is already a remote project of {} on this instance: an instance can't have a project both as \
                     its own and as a remote project",
                    p.name, l.owner_name
                ),
            ));
        }
    }
    Ok(())
}

/// Tells the owner this instance left the link; true when it was told (best effort: a link the owner can't be told
/// about stays listed there until it is removed, and its heartbeats are refused).
async fn tell_owner_left(st: &AppState, l: &OwnerLink, key: Option<&ed25519_dalek::SigningKey>) -> bool {
    let key = match key {
        Some(k) => k.clone(),
        None => match link::load_key_for_revoke(st, &l.link_id).await {
            Some(k) => k,
            None => return false,
        },
    };
    let told: Result<serde_json::Value, _> = link::post(
        st,
        &l.owner_url,
        link::LEAVE_PATH,
        Some((&key, &l.link_id, &l.owner_project_id)),
        &serde_json::json!({}),
    )
    .await;
    link::got_through(&told, &l.owner_name, "could not tell the owner this instance left")
}

/// Joins a project of another instance: pairs with the owner on the code it shows, then adds the remote project.
async fn join(
    State(st): State<AppState>,
    ApiJson(body): ApiJson<JoinRemoteProject>,
) -> Result<(StatusCode, Json<RemoteProjectView>), ApiError> {
    let owner_url = pdt_core::link::instance_url(&body.owner_address).map_err(|e| ApiError::invalid(e.to_string()))?;
    let code = pdt_core::link::normalize_code(&body.code)
        .ok_or_else(|| ApiError::invalid("a pairing code is 8 letters and digits, e.g. ABCD-EFGH"))?;
    let address = match body.address.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => pdt_core::link::instance_url(a).map_err(|e| ApiError::invalid(e.to_string()))?,
        None => link::own_url(&st).ok_or_else(|| {
            ApiError::conflict(
                codes::REMOTE_ACCESS_OFF,
                "turn on remote access in Settings → This host first: the owner reaches this instance through it",
            )
        })?,
    };
    let key = link::new_key();
    let req = PairRequest { code, instance: link::hello(&st).await?, url: address, public_key: link::public_key(&key) };
    let reply: PairReply =
        link::post(&st, &owner_url, link::PAIR_PATH, None, &req).await.map_err(|e| e.into_api(&owner_url))?;

    let pending = OwnerLink {
        project_id: String::new(),
        link_id: reply.link_id.clone(),
        owner_project_id: reply.project.id.clone(),
        owner_instance_id: reply.owner.instance_id.clone(),
        owner_name: reply.owner.name.clone(),
        owner_url: owner_url.clone(),
        owner_version: reply.owner.version.clone(),
        owner_public_key: reply.public_key.clone(),
        // An owner from before PDT-649 sends its origin as it is, credentials and all.
        repo_url: reply.project.repo_url.as_deref().map(pdt_core::git_url::without_credentials),
        checkout_path: None,
        paused: false,
        own_update_channel: false,
        cap: None,
        last_contact_at: Some(now_ms()),
        removed_at: None,
        created_at: now_ms(),
    };
    // Whatever goes wrong from here, the owner is told this instance left, so its link isn't left half made.
    let undo = |e: ApiError| {
        let (st, l, key) = (st.clone(), pending.clone(), key.clone());
        async move {
            tell_owner_left(&st, &l, Some(&key)).await;
            e
        }
    };
    if !pdt_core::link::compatible(st.link.protocol(), reply.owner.protocol) {
        let me = link::hello(&st).await?;
        let them = pdt_core::link::Side {
            name: &reply.owner.name,
            protocol: reply.owner.protocol,
            version: &reply.owner.version,
        };
        let us = pdt_core::link::Side { name: &me.name, protocol: me.protocol, version: &me.version };
        let e = ApiError::conflict(codes::LINK_INCOMPATIBLE, pdt_core::link::incompatible_message(them, us));
        return Err(undo(e).await);
    }
    if let Some(url) = &reply.project.repo_url
        && let Some(local) = local_with_repo(&st, url).await?
    {
        let e = ApiError::conflict(
            codes::REMOTE_PROJECT,
            format!(
                "{} is already a project of this instance's own ({}): an instance can't have a project both as its \
                 own and as a remote project",
                local.name, local.repo_path
            ),
        );
        return Err(undo(e).await);
    }
    // Paired again with the same owner's project: the new link replaces the old one.
    for (p, l) in st.store.list_remote_projects().await? {
        if l.owner_instance_id == pending.owner_instance_id && l.owner_project_id == pending.owner_project_id {
            st.store.delete_project(&p.id).await?;
            link::delete_key(&st, &l.link_id).await;
        }
    }
    if let Err(e) = link::save_key(&st, &reply.link_id, &key).await {
        return Err(undo(e).await);
    }
    let keys = st.store.project_keys().await?;
    let project_key = if keys.iter().any(|k| k == &reply.project.key) {
        pdt_core::project::suggest_key(&reply.project.name, &keys)
    } else {
        reply.project.key.clone()
    };
    let project = Project {
        id: uuid::Uuid::now_v7().to_string(),
        name: reply.project.name.clone(),
        // Not a folder: the checkout comes when work is placed here. Unique, as every project's path is.
        repo_path: format!("remote:{}", reply.link_id),
        base_branch: reply.project.base_branch.clone(),
        created_at: now_ms(),
        key: project_key,
        forge: None,
        kind: ProjectKind::Remote,
    };
    let link = OwnerLink { project_id: project.id.clone(), ..pending.clone() };
    if let Err(e) = st.store.insert_remote_project(&project, &link).await {
        link::delete_key(&st, &reply.link_id).await;
        return Err(undo(e.into()).await);
    }
    tracing::info!(project = %project.name, owner = %link.owner_name, "joined a remote project");
    Ok((StatusCode::CREATED, Json(view_of(&st, project, link, now_ms()).await)))
}

async fn patch(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(body): ApiJson<RemoteProjectPatch>,
) -> Result<Json<RemoteProjectView>, ApiError> {
    one(&st, &id).await?;
    if let Some(paused) = body.paused {
        st.store.set_remote_project_paused(&id, paused).await?;
    }
    if let Some(path) = body.checkout_path {
        let path = path.trim();
        if !path.is_empty()
            && !std::path::Path::new(&crate::host::expand_home(path, st.home_dir.get().as_deref())).is_absolute()
        {
            return Err(ApiError::invalid("the checkout's folder must be a full path"));
        }
        st.store.set_remote_project_checkout(&id, Some(path).filter(|p| !p.is_empty())).await?;
    }
    if let Some(own) = body.own_update_channel {
        st.store.set_remote_project_own_channel(&id, own).await?;
    }
    if let Some(cap) = body.cap {
        st.store.set_remote_project_cap(&id, Some(cap).filter(|c| *c > 0)).await?;
    }
    let (p, l) = one(&st, &id).await?;
    Ok(Json(view_of(&st, p, l, now_ms()).await))
}

/// Leaves a remote project: the owner is told, and the project and this end's key go. An owner that can't be reached
/// is left all the same, and the reply says it wasn't told.
async fn remove(State(st): State<AppState>, ApiPath(id): ApiPath<String>) -> Result<Json<LinkEnded>, ApiError> {
    let (p, l) = one(&st, &id).await?;
    // An owner that removed this instance already has nothing to be told.
    let told = l.removed_at.is_some() || tell_owner_left(&st, &l, None).await;
    crate::exec::close_link(&st, &l.link_id).await;
    st.store.delete_project(&p.id).await?;
    link::delete_key(&st, &l.link_id).await;
    tracing::info!(project = %p.name, owner = %l.owner_name, told, "left a remote project");
    Ok(Json(LinkEnded { told }))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, patch as patch_route};
    axum::Router::new()
        .route("/remote-projects", get(list).post(join))
        .route("/remote-projects/{id}", patch_route(patch).delete(remove))
}
