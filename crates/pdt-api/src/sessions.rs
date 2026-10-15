use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_agents::{PtyError, SpawnSpec};
use pdt_core::session::{
    AgentKind, NewSession, Session, SessionKind, SessionStatus, TerminalSize, clean_title, default_title, is_scrubbed,
    launch,
};
use std::path::PathBuf;

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
    };
    st.store.insert_session(&session).await?;
    if let Err(e) = start_pty(&st, &session, valid.size, false) {
        st.store.delete_session(&session.id).await?;
        return Err(e.into());
    }
    Ok((StatusCode::CREATED, Json(session)))
}

/// Spawns the session's process and records its exit when it ends.
fn start_pty(st: &AppState, s: &Session, size: TerminalSize, resume: bool) -> Result<(), PtyError> {
    // Claude saves its conversation only after the first turn; resuming one that was never saved fails, so
    // start a fresh conversation under the same id instead.
    let resume = resume
        && !(s.kind == SessionKind::AgentTui
            && s.agent != Some(AgentKind::Codex)
            && st
                .platform
                .claude_config_dir
                .as_deref()
                .is_some_and(|dir| !pdt_agents::launch::claude_conversation_exists(dir, &s.id)));
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
    let handle = tokio::spawn(async move {
        if let Ok(code) = exit.await
            && let Err(e) = store.end_session(&id, SessionStatus::Finished, Some(code), now_ms()).await
        {
            tracing::warn!(session = %id, error = %e, "failed to record session exit");
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
    let session = load(&st, &id).await?;
    if st.ptys.is_running(&id) {
        return Err(ApiError::new(StatusCode::CONFLICT, "already_running", "this session is already running"));
    }
    restart_locked(&st, &session, size).await?;
    Ok(Json(load(&st, &id).await?))
}

/// Starts a session that has no running process, resuming its agent conversation. The caller holds the session lock.
async fn restart_locked(st: &AppState, session: &Session, size: TerminalSize) -> Result<(), ApiError> {
    let id = &session.id;
    settle_exit(st, id).await;
    st.ptys.remove(id);
    // Mark running before spawning, so an instant exit is recorded after (not before) this update.
    st.store.set_session_running(id).await?;
    if let Err(e) = start_pty(st, session, size, true) {
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
/// Codex resumes the newest conversation in its folder (`resume --last`), so among interrupted Codex sessions
/// sharing a folder only the most recently created one is resumed; the rest stay interrupted for a manual restart.
pub async fn resume_interrupted(st: &AppState) -> usize {
    let sessions = match st.store.list_sessions_by_status(SessionStatus::Interrupted).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "could not list interrupted sessions to resume");
            return 0;
        }
    };
    let is_codex = |s: &Session| s.kind == SessionKind::AgentTui && s.agent == Some(AgentKind::Codex);
    // The list is oldest first, so the last Codex session seen for a folder is the newest.
    let mut newest_codex: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for s in sessions.iter().filter(|s| is_codex(s)) {
        newest_codex.insert(&s.cwd, &s.id);
    }
    let size = TerminalSize { cols: 120, rows: 30 };
    let mut resumed = 0;
    for s in &sessions {
        if is_codex(s) && newest_codex.get(s.cwd.as_str()) != Some(&s.id.as_str()) {
            continue;
        }
        let _guard = st.lock_session(&s.id).await;
        // Re-read under the lock: the user may have restarted or removed it already.
        let current = match st.store.get_session(&s.id).await {
            Ok(Some(c)) if c.status == SessionStatus::Interrupted && !st.ptys.is_running(&c.id) => c,
            _ => continue,
        };
        match restart_locked(st, &current, size).await {
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
