use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_agents::{PtyError, SpawnSpec};
use pdt_core::session::{
    AgentKind, NewSession, Resume, Session, SessionKind, SessionStatus, TerminalSize, clean_title, default_title,
    is_scrubbed, launch,
};
use pdt_store::Store;
use std::collections::HashSet;
use std::path::{Path as FsPath, PathBuf};

pub(crate) async fn list(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<Session>>, ApiError> {
    st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    Ok(Json(st.store.list_sessions(&id).await?))
}

pub(crate) async fn get_one(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Session>, ApiError> {
    Ok(Json(load(&st, &id).await?))
}

async fn load(st: &AppState, id: &str) -> Result<Session, ApiError> {
    st.store.get_session(id).await?.ok_or_else(|| ApiError::not_found("session"))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewSession>, JsonRejection>,
) -> Result<(StatusCode, Json<Session>), ApiError> {
    let Json(input) = payload?;
    let valid = input.validate()?;
    let project = st.store.get_project(&valid.project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let (worktree_id, cwd, place) = match &valid.worktree_id {
        Some(wid) => {
            let wt = st.store.get_worktree(wid).await?.filter(|w| w.project_id == project.id).ok_or_else(|| {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", "worktree does not belong to this project")
            })?;
            (Some(wt.id), wt.path, wt.branch)
        }
        None => (None, project.repo_path.clone(), project.name.clone()),
    };
    let session = Session {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: project.id,
        worktree_id,
        kind: valid.kind,
        agent: valid.agent,
        title: default_title(valid.kind, valid.agent, &place),
        cwd,
        status: SessionStatus::Running,
        exit_code: None,
        created_at: now_ms(),
        ended_at: None,
        external_session_id: None,
        started_at: Some(now_ms()),
    };
    st.store.insert_session(&session).await?;
    if let Err(e) = start_pty(&st, &session, valid.size, &Resume::Fresh) {
        st.store.delete_session(&session.id).await?;
        return Err(e.into());
    }
    Ok((StatusCode::CREATED, Json(session)))
}

fn is_codex(s: &Session) -> bool {
    s.kind == SessionKind::AgentTui && s.agent == Some(AgentKind::Codex)
}

/// Decides how to start a session that is being restarted: the agent's saved conversation if there is one, else a
/// fresh one. Neither agent saves a conversation before its first turn, so a session that never had one starts over.
async fn plan_resume(st: &AppState, s: &Session) -> Resume {
    if is_codex(s) {
        return match bind_codex(&st.store, st.platform.codex_home.as_deref(), &s.id).await {
            Some(conversation) => Resume::Codex(conversation),
            None => Resume::Fresh,
        };
    }
    if s.kind != SessionKind::AgentTui {
        return Resume::Fresh;
    }
    // Claude: resuming a conversation that was never saved fails, so start a fresh one under the same id. Only a few
    // directory entries are read, so this stays on the async thread. Unknown config dir: assume it was saved.
    let saved = st
        .platform
        .claude_config_dir
        .as_deref()
        .is_none_or(|dir| pdt_agents::launch::claude_conversation_exists(dir, &s.id));
    if saved { Resume::Claude } else { Resume::Fresh }
}

/// Finds and records the Codex conversation a session created (see `find_codex_conversation`). Returns the
/// session's conversation id, whether just found or bound earlier; None if there is none yet (no turn was taken, or
/// the rollout file was not found). Codex's own files are scanned on a blocking thread.
async fn bind_codex(store: &Store, codex_home: Option<&FsPath>, id: &str) -> Option<String> {
    let s = store.get_session(id).await.ok()??;
    if !is_codex(&s) {
        return None;
    }
    if s.external_session_id.is_some() {
        return s.external_session_id;
    }
    let home = codex_home?.to_path_buf();
    let claimed: HashSet<String> = store.claimed_external_session_ids(id).await.ok()?.into_iter().collect();
    let (cwd, started_at) = (PathBuf::from(&s.cwd), s.started_at.unwrap_or(s.created_at));
    let found = tokio::task::spawn_blocking(move || {
        pdt_agents::launch::find_codex_conversation(&home, &cwd, started_at, &claimed)
    })
    .await
    .ok()??;
    match store.bind_external_session_id(id, &found).await {
        Ok(true) => {
            tracing::info!(session = %id, conversation = %found, "bound the session to its Codex conversation");
            Some(found)
        }
        // Lost a race with another binding; use whatever won.
        Ok(false) => store.get_session(id).await.ok()??.external_session_id,
        Err(e) => {
            tracing::warn!(session = %id, error = %e, "could not record the Codex conversation id");
            None
        }
    }
}

/// Spawns the session's process and records its exit when it ends.
fn start_pty(st: &AppState, s: &Session, size: TerminalSize, resume: &Resume) -> Result<(), PtyError> {
    let l = launch(s.kind, s.agent, &st.platform, resume, &s.id);
    let login_path = st.platform.login_path.as_deref();
    let (program, args) = pdt_agents::launch::resolve(&l.program, &l.args, login_path)?;
    let mut env = vec![("TERM".to_string(), "xterm-256color".to_string()), ("COLORTERM".into(), "truecolor".into())];
    if let Some(path) = login_path {
        // So `#!/usr/bin/env node` shebangs and the agents' own child processes find the user's tools.
        env.push(("PATH".into(), path.into()));
    }
    let spec = SpawnSpec {
        id: s.id.clone(),
        program,
        args,
        cwd: PathBuf::from(&s.cwd),
        env_remove: std::env::vars_os()
            .filter_map(|(k, _)| k.into_string().ok())
            .filter(|k| is_scrubbed(s.kind, k))
            .collect(),
        env,
        cols: size.cols,
        rows: size.rows,
    };
    let exit = st.ptys.spawn(spec)?;
    let (store, id) = (st.store.clone(), s.id.clone());
    let codex_home = is_codex(s).then(|| st.platform.codex_home.clone()).flatten();
    let handle = tokio::spawn(async move {
        if let Ok(code) = exit.await {
            if let Err(e) = store.end_session(&id, SessionStatus::Finished, Some(code), now_ms()).await {
                tracing::warn!(session = %id, error = %e, "failed to record session exit");
            }
            // Remember which Codex conversation this was while its start time is still fresh.
            if codex_home.is_some() {
                bind_codex(&store, codex_home.as_deref(), &id).await;
            }
        }
    });
    st.exit_tasks.lock().unwrap_or_else(|e| e.into_inner()).insert(s.id.clone(), handle);
    Ok(())
}

/// Waits for the previous run's exit write (the process has already exited, so this is prompt).
async fn settle_exit(st: &AppState, id: &str) {
    let handle = st.exit_tasks.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    if let Some(h) = handle {
        let _ = h.await;
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct Rename {
    title: String,
}

pub(crate) async fn rename(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<Rename>, JsonRejection>,
) -> Result<Json<Session>, ApiError> {
    let Json(body) = payload?;
    let title = clean_title(&body.title)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid_title", "title must be 1 to 100 characters"))?;
    if !st.store.rename_session(&id, &title).await? {
        return Err(ApiError::not_found("session"));
    }
    Ok(Json(load(&st, &id).await?))
}

pub(crate) async fn kill(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let _guard = st.lock_session(&id).await;
    load(&st, &id).await?;
    st.ptys.kill(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn restart(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TerminalSize>, JsonRejection>,
) -> Result<Json<Session>, ApiError> {
    let Json(size) = payload?;
    let size = size.validate()?;
    let _guard = st.lock_session(&id).await;
    load(&st, &id).await?;
    if st.ptys.is_running(&id) {
        return Err(ApiError::new(StatusCode::CONFLICT, "already_running", "this session is already running"));
    }
    restart_locked(&st, &id, size).await?;
    Ok(Json(load(&st, &id).await?))
}

/// Starts a session that has no running process, resuming its agent conversation. The caller holds the session lock.
async fn restart_locked(st: &AppState, id: &str, size: TerminalSize) -> Result<(), ApiError> {
    settle_exit(st, id).await;
    st.ptys.remove(id);
    // After the previous run's exit task, which may have just bound its Codex conversation.
    let session = load(st, id).await?;
    let resume = plan_resume(st, &session).await;
    // Mark running before spawning, so an instant exit is recorded after (not before) this update.
    let started_at = now_ms();
    st.store.set_session_running(id, started_at).await?;
    let session = Session { started_at: Some(started_at), ..session };
    if let Err(e) = start_pty(st, &session, size, &resume) {
        // AlreadyExists means another run owns the session (it is running), so leave its status alone.
        if !matches!(e, PtyError::AlreadyExists(_)) {
            st.store.end_session(id, SessionStatus::Interrupted, None, now_ms()).await?;
        }
        return Err(e.into());
    }
    Ok(())
}

/// Restarts the sessions a previous pdtd run left `interrupted`, so they come back by themselves. Returns how many
/// started. A session that fails to start stays interrupted (logged); it never stops the others.
///
/// Each session resumes its own conversation (see `plan_resume`), so sessions sharing a folder do not interfere.
pub async fn resume_interrupted(st: &AppState) -> usize {
    let sessions = match st.store.list_sessions_by_status(SessionStatus::Interrupted).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "could not list interrupted sessions to resume");
            return 0;
        }
    };
    let size = TerminalSize { cols: 120, rows: 30 };
    let mut resumed = 0;
    for s in &sessions {
        let _guard = st.lock_session(&s.id).await;
        // Re-read under the lock: the user may have restarted or removed it already.
        let current = match st.store.get_session(&s.id).await {
            Ok(Some(c)) if c.status == SessionStatus::Interrupted && !st.ptys.is_running(&c.id) => c,
            _ => continue,
        };
        match restart_locked(st, &current.id, size).await {
            Ok(()) => resumed += 1,
            Err(e) => tracing::warn!(session = %s.id, error = ?e, "could not resume session; it stays interrupted"),
        }
    }
    resumed
}

pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let _guard = st.lock_session(&id).await;
    load(&st, &id).await?;
    if st.ptys.is_running(&id) {
        return Err(ApiError::new(StatusCode::CONFLICT, "session_running", "stop the session before removing it"));
    }
    settle_exit(&st, &id).await;
    st.ptys.remove(&id);
    st.store.delete_session(&id).await?;
    st.forget_session_lock(&id);
    Ok(StatusCode::NO_CONTENT)
}
