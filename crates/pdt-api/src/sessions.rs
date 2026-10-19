use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_agents::{PtyError, SpawnSpec};
use pdt_core::conversation::{PermissionMode, SessionMode};
use pdt_core::session::{
    AgentKind, NewSession, Resume, Session, SessionKind, SessionStatus, TerminalSize, ValidSession, clean_title,
    default_title, is_scrubbed, launch, prompt_arg, shim_prompt,
};
use pdt_core::workspace::TabPlacement;
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
    crate::workspaces::check_placement(&st, input.tab.as_ref()).await?;
    let req = CreateSession { valid, title: None, attempt_id: None, ticket_key: None, tab: Some(input.tab) };
    Ok((StatusCode::CREATED, Json(create_session(&st, req).await?)))
}

/// What `create_session` makes.
pub(crate) struct CreateSession {
    pub valid: ValidSession,
    /// The default title (`<Kind> · <branch or project>`) when None.
    pub title: Option<String>,
    pub attempt_id: Option<String>,
    /// The attempt's ticket key, for the short prompt a Windows `.cmd` shim gets.
    pub ticket_key: Option<String>,
    /// Some: open the session's tab there (None inside: the end of its project's workspace). None: the caller opens it.
    pub tab: Option<Option<TabPlacement>>,
}

/// Makes a session and starts its process (`POST /api/sessions` and ticket Start): the row, the terminal (with the
/// first prompt, if any), activity tracking and, if asked, its tab. Nothing is left behind if the process can't start.
pub(crate) async fn create_session(st: &AppState, req: CreateSession) -> Result<Session, ApiError> {
    let valid = req.valid;
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
    // A fresh Codex terminal is matched to its conversation by start time, so launches in a folder are spaced out.
    // (A Codex conversation knows its thread id from the start.)
    let codex = valid.kind == SessionKind::AgentTui
        && valid.agent == Some(AgentKind::Codex)
        && valid.mode == SessionMode::Terminal;
    let fresh_started_at = if codex { Some(wait_for_fresh_codex_slot(st, &cwd).await) } else { None };
    let session = Session {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: project.id,
        worktree_id,
        kind: valid.kind,
        agent: valid.agent,
        bypass_permissions: valid.bypass_permissions,
        title: req.title.unwrap_or_else(|| default_title(&place)),
        cwd,
        status: SessionStatus::Running,
        exit_code: None,
        created_at: now_ms(),
        ended_at: None,
        external_session_id: None,
        fresh_started_at,
        activity: None,
        attention_at: None,
        last_viewed_at: None,
        unseen: false,
        attempt_id: req.attempt_id,
        ticket: None,
        mode: valid.mode,
        model: valid.model.clone(),
        permission_mode: valid.permission_mode,
    };
    // Before the row exists, so a lookup failure leaves no half-created session.
    let creds = credentials_for(st, &session.project_id).await?;
    st.store.insert_session(&session).await?;
    if session.mode == SessionMode::Conversation {
        // The first prompt is the first message; the agent starts now so a missing program fails here.
        if let Err(e) = crate::conversations::start(st, &session, valid.initial_prompt).await {
            crate::conversations::forget(st, &session.id).await;
            st.store.delete_session(&session.id).await?;
            let _ = std::fs::remove_dir_all(session_data_dir(st, &session.id));
            return Err(e);
        }
        if let Some(tab) = req.tab {
            let item = pdt_core::workspace::TabItem::Session(session.id.clone());
            crate::workspaces::open_for_new(st, &session.project_id, item, tab.as_ref()).await;
        }
        return load(st, &session.id).await;
    }
    let prompt = valid.initial_prompt.map(|text| InitialPrompt { text, ticket_key: req.ticket_key });
    let started = match agent_instructions(st, &session).await {
        Ok(instructions) => {
            start_pty(st, &session, valid.size, &Resume::Fresh, creds, prompt.as_ref(), instructions.as_ref())
                .map_err(ApiError::from)
        }
        Err(e) => Err(e),
    };
    if let Err(e) = started {
        st.store.delete_session(&session.id).await?;
        let _ = std::fs::remove_dir_all(session_data_dir(st, &session.id));
        return Err(e);
    }
    crate::activity::track(st, &session).await;
    if let Some(tab) = req.tab {
        let item = pdt_core::workspace::TabItem::Session(session.id.clone());
        crate::workspaces::open_for_new(st, &session.project_id, item, tab.as_ref()).await;
    }
    load(st, &session.id).await
}

/// A new session's first prompt.
pub(crate) struct InitialPrompt {
    pub text: String,
    /// The ticket it is about (`PDT-4`), if any.
    pub ticket_key: Option<String>,
}

/// The credential policy for agents in this project (project override, else host setting).
pub(crate) async fn credentials_for(
    st: &AppState,
    project_id: &str,
) -> Result<pdt_core::host::CredentialPolicy, ApiError> {
    let host = st.store.get_host_settings().await?.agent_credentials;
    let project = st.store.get_project_settings(project_id).await?.agent_credentials;
    Ok(pdt_core::host::effective_credentials(host, project))
}

/// PDT's instructions for an agent session (PDT-46): where PDT is, which session, project and checkout this is, and
/// the project's own instructions from its settings. None for shells. Codex's text keeps the user's own
/// `developer_instructions` first, since PDT's value replaces them. Written to `<session dir>/instructions.md`.
pub(crate) async fn agent_instructions(
    st: &AppState,
    s: &Session,
) -> Result<Option<pdt_agents::adapter::SessionInstructions>, ApiError> {
    if s.kind != SessionKind::AgentTui {
        return Ok(None);
    }
    let project = st.store.get_project(&s.project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let branch = match &s.worktree_id {
        Some(w) => st.store.get_worktree(w).await?.map(|w| w.branch),
        None => None,
    };
    let settings = st.store.get_project_settings(&s.project_id).await?;
    let ctx = pdt_core::agent_context::SessionContext {
        port: st.listen_port,
        session_id: &s.id,
        project_id: &project.id,
        project_name: &project.name,
        project_key: &project.key,
        checkout: &s.cwd,
        branch: branch.as_deref(),
        repo_path: &project.repo_path,
        base_branch: &project.base_branch,
    };
    let ours = pdt_core::agent_context::session_instructions(&ctx, settings.agent_instructions.as_deref());
    let text = match (s.agent, st.platform.codex_home.as_deref()) {
        (Some(AgentKind::Codex), Some(home)) => {
            let user = pdt_agents::instructions::user_codex_developer_instructions(home);
            pdt_agents::instructions::combine(user.as_deref(), &ours)
        }
        _ => ours,
    };
    let file = pdt_agents::instructions::write_instructions(&session_data_dir(st, &s.id), &text)
        .map_err(|e| ApiError::internal(format!("writing the session's instructions: {e}")))?;
    Ok(Some(pdt_agents::adapter::SessionInstructions { text, file }))
}

fn is_codex(s: &Session) -> bool {
    s.kind == SessionKind::AgentTui && s.agent == Some(AgentKind::Codex)
}

/// Decides how to start a session that is being restarted: the agent's saved conversation if there is one, else a
/// fresh one. Neither agent saves a conversation before its first turn, so a session that never had one starts over.
async fn plan_resume(st: &AppState, s: &Session) -> Resume {
    if is_codex(s) {
        return match bind_codex(st, &s.id).await {
            Some(conversation) => Resume::Codex(conversation),
            None => Resume::Fresh,
        };
    }
    if s.kind != SessionKind::AgentTui {
        return Resume::Fresh;
    }
    // Claude: resuming a conversation that was never saved fails, so start a fresh one under the same id (a new
    // conversation's own after `/clear`: the session id's transcript is taken). Only a few
    // directory entries are read, so this stays on the async thread. Unknown config dir: assume it was saved.
    let saved = st
        .platform
        .claude_config_dir
        .as_deref()
        .is_none_or(|dir| pdt_agents::launch::claude_conversation_exists(dir, s.claude_conversation_id()));
    if saved { Resume::Claude } else { Resume::Fresh }
}

/// Waits until no other fresh Codex launch in this folder happened within the spacing, then claims the slot and
/// returns its time (ms), which becomes the session's `fresh_started_at`. Nothing is held while waiting.
async fn wait_for_fresh_codex_slot(st: &AppState, cwd: &str) -> i64 {
    let key = pdt_agents::launch::canonical_folder_key(cwd);
    let gap = i64::try_from(st.codex_spawn_spacing.as_millis()).unwrap_or(i64::MAX);
    loop {
        let wait = {
            let mut last = st.codex_spawns.lock().unwrap_or_else(|e| e.into_inner());
            let now = now_ms();
            match last.get(&key) {
                Some(&l) if now < l.saturating_add(gap) => l.saturating_add(gap) - now,
                _ => {
                    last.insert(key, now);
                    return now;
                }
            }
        };
        tokio::time::sleep(std::time::Duration::from_millis(wait as u64)).await;
    }
}

/// Finds and records the Codex conversation a session created (see `find_codex_conversation`). Returns the
/// session's conversation id, whether just found or bound earlier; None if there is none (no turn was taken, the
/// rollout file was not found, the launch time is unknown, or another session was launched too close to tell the
/// two apart). Codex's own files are scanned on a blocking thread.
async fn bind_codex(st: &AppState, id: &str) -> Option<String> {
    bind_codex_with(&st.store, st.platform.codex_home.as_deref(), st.codex_spawn_spacing, id).await
}

async fn bind_codex_with(
    store: &Store,
    codex_home: Option<&FsPath>,
    spacing: std::time::Duration,
    id: &str,
) -> Option<String> {
    let s = store.get_session(id).await.ok()??;
    if !is_codex(&s) {
        return None;
    }
    if s.external_session_id.is_some() {
        return s.external_session_id;
    }
    let fresh = s.fresh_started_at?;
    let home = codex_home?.to_path_buf();
    let folder = pdt_agents::launch::canonical_folder_key(&s.cwd);
    let peers: Vec<i64> = store
        .list_codex_sessions()
        .await
        .ok()?
        .iter()
        .filter(|p| p.id != s.id && pdt_agents::launch::canonical_folder_key(&p.cwd) == folder)
        .filter_map(|p| p.fresh_started_at)
        .collect();
    let gap = i64::try_from(spacing.as_millis()).unwrap_or(i64::MAX);
    if peers.iter().any(|p| (p - fresh).abs() < gap) {
        tracing::warn!(session = %id, "another Codex session in this folder was launched too close in time; not binding");
        return None;
    }
    // A second pass covers losing a race to bind the conversation we found.
    for _ in 0..2 {
        let claimed: HashSet<String> = store.claimed_external_session_ids(id).await.ok()?.into_iter().collect();
        let (home, cwd, peers) = (home.clone(), PathBuf::from(&s.cwd), peers.clone());
        let found = tokio::task::spawn_blocking(move || {
            pdt_agents::launch::find_codex_conversation(&home, &cwd, fresh, &peers, &claimed)
        })
        .await
        .ok()??;
        match store.bind_external_session_id(id, &found).await {
            Ok(true) => {
                tracing::info!(session = %id, conversation = %found, "bound the session to its Codex conversation");
                return Some(found);
            }
            Ok(false) => {
                if let Some(bound) = store.get_session(id).await.ok()??.external_session_id {
                    return Some(bound);
                }
            }
            Err(e) => {
                tracing::warn!(session = %id, error = %e, "could not record the Codex conversation id");
                return None;
            }
        }
    }
    None
}

/// Spawns the session's process and records its exit when it ends. An agent's first prompt (fresh launches at
/// creation only) is its last argument; an agent run through `cmd.exe` (a Windows `.cmd` shim, which would interpret
/// the prompt's text) gets the prompt in `<session dir>/prompt.md` and a short prompt naming that file instead.
fn start_pty(
    st: &AppState,
    s: &Session,
    size: TerminalSize,
    resume: &Resume,
    creds: pdt_core::host::CredentialPolicy,
    prompt: Option<&InitialPrompt>,
    instructions: Option<&pdt_agents::adapter::SessionInstructions>,
) -> Result<(), PtyError> {
    // Claude: `--resume`/`--session-id` take its conversation id, which `/clear` moves off the session id.
    let l = launch(s.kind, s.agent, &st.platform, resume, s.claude_conversation_id(), s.bypass_permissions);
    let login_path = st.platform.login_path.as_deref();
    let launch_args = with_hook_args(st, s, l.args)?;
    let resolved = pdt_agents::launch::resolve_launch(&l.program, &launch_args, login_path)?;
    let (program, mut args) = (resolved.program, resolved.args);
    if let Some(i) = instructions.filter(|_| s.kind == SessionKind::AgentTui) {
        match s.agent {
            // Codex's flags go before its arguments (`resume <id>`), right after any cmd.exe prefix.
            Some(AgentKind::Codex) => {
                let at = args.len() - launch_args.len();
                let extra = pdt_agents::instructions::codex_tui_args(&i.text, resolved.via_cmd, &i.file);
                args.splice(at..at, extra);
            }
            _ => args.extend(pdt_agents::instructions::claude_args(&i.file)),
        }
    }
    if let Some(p) = prompt.filter(|_| s.kind == SessionKind::AgentTui) {
        if resolved.via_cmd {
            let dir = session_data_dir(st, &s.id);
            let file = dir.join("prompt.md");
            std::fs::create_dir_all(&dir)
                .and_then(|_| std::fs::write(&file, &p.text))
                .map_err(|e| PtyError::Spawn(format!("writing the session's prompt file: {e}")))?;
            match shim_prompt(p.ticket_key.as_deref(), &file.to_string_lossy()) {
                Some(short) => args.push(short),
                None => tracing::warn!(session = %s.id, file = %file.display(),
                    "the prompt file's path is unsafe on a cmd.exe command line; starting without the prompt"),
            }
        } else {
            args.push(prompt_arg(&p.text));
        }
    }
    let mut env = pdt_core::session::terminal_env();
    if let Some(path) = login_path {
        // So `#!/usr/bin/env node` shebangs and the agents' own child processes find the user's tools.
        env.push(("PATH".into(), path.into()));
    }
    if s.kind == SessionKind::AgentTui {
        // For `pdtd hook`. Set after the inherited PDT_* variables are scrubbed, so only these values reach it.
        env.push(("PDT_SESSION_ID".into(), s.id.clone()));
        env.push(("PDT_HOOK_PORT".into(), st.listen_port.to_string()));
    }
    let spec = SpawnSpec {
        id: s.id.clone(),
        program,
        args,
        cwd: PathBuf::from(&s.cwd),
        env_remove: std::env::vars_os()
            .filter_map(|(k, _)| k.into_string().ok())
            .filter(|k| is_scrubbed(s.kind, k, creds))
            .collect(),
        env,
        cols: size.cols,
        rows: size.rows,
    };
    let exit = st.ptys.spawn(spec)?;
    let (store, id) = (st.store.clone(), s.id.clone());
    let codex_home = is_codex(s).then(|| st.platform.codex_home.clone()).flatten();
    let spacing = st.codex_spawn_spacing;
    let handle = tokio::spawn(async move {
        if let Ok(code) = exit.await {
            if let Err(e) = store.end_session(&id, SessionStatus::Finished, Some(code), now_ms()).await {
                tracing::warn!(session = %id, error = %e, "failed to record session exit");
            }
            // Remember which Codex conversation this was while its start time is still fresh.
            if codex_home.is_some() {
                bind_codex_with(&store, codex_home.as_deref(), spacing, &id).await;
            }
        }
    });
    st.exit_tasks.lock().unwrap_or_else(|e| e.into_inner()).insert(s.id.clone(), handle);
    Ok(())
}

/// The agent's launch arguments plus its activity hooks (remote-host spec §5.1): Claude gets a per-session
/// `--settings` file in PDT's data folder, Codex a `-c notify=[…]` that chains to the user's own notify. The user's
/// own agent settings are never written. Limit: with a Windows `.cmd` shim (npm installs) the arguments pass through
/// `cmd.exe`, so a hook program path containing `&`, `|`, `^` or `%` would be mangled.
fn with_hook_args(st: &AppState, s: &Session, args: Vec<String>) -> Result<Vec<String>, PtyError> {
    if s.kind != SessionKind::AgentTui {
        return Ok(args);
    }
    match s.agent {
        Some(AgentKind::Claude) => {
            let file = pdt_agents::hooks::write_claude_settings(&session_data_dir(st, &s.id), &st.hook_program)
                .map_err(|e| PtyError::Spawn(format!("writing the session's hook settings: {e}")))?;
            let mut args = args;
            args.extend(["--settings".to_string(), file.to_string_lossy().into_owned()]);
            Ok(args)
        }
        Some(AgentKind::Codex) => {
            let chain = st
                .platform
                .codex_home
                .as_deref()
                .and_then(|h| pdt_agents::hooks::user_codex_notify(h, &st.hook_program))
                .unwrap_or_default();
            let mut all = pdt_agents::hooks::codex_notify_args(&st.hook_program, &chain);
            all.extend(args);
            Ok(all)
        }
        None => Ok(args),
    }
}

/// PDT's own files for one session (the Claude hook settings).
fn session_data_dir(st: &AppState, id: &str) -> PathBuf {
    st.data_dir.join("sessions").join(id)
}

/// Waits for the previous run's exit write (the process has already exited, so this is prompt).
pub(crate) async fn settle_exit(st: &AppState, id: &str) {
    let handle = st.exit_tasks.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    if let Some(h) = handle {
        let _ = h.await;
    }
}

/// `PATCH /api/sessions/{id}`: any of a new title, and for conversations the model (null or "" for the agent's
/// default) and permission mode.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateSession {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, deserialize_with = "present")]
    model: Option<Option<String>>,
    #[serde(default)]
    permission_mode: Option<PermissionMode>,
}

/// Tells an absent field (None) from an explicit null (Some(None)).
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    use serde::Deserialize;
    Ok(Some(Option::<String>::deserialize(d)?))
}

pub(crate) async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<UpdateSession>, JsonRejection>,
) -> Result<Json<Session>, ApiError> {
    let Json(body) = payload?;
    let session = load(&st, &id).await?;
    if let Some(title) = &body.title {
        let title = clean_title(title).ok_or_else(|| {
            ApiError::new(StatusCode::BAD_REQUEST, "invalid_title", "title must be 1 to 100 characters")
        })?;
        st.store.rename_session(&id, &title).await?;
    }
    if body.model.is_some() || body.permission_mode.is_some() {
        if session.mode != SessionMode::Conversation {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "not_conversation",
                "the model and permission mode are set on conversations",
            ));
        }
        let model = match body.model {
            Some(m) => match m.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                Some(m) if !pdt_core::conversation::valid_model(m) => {
                    return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", "invalid model name"));
                }
                m => Some(m.map(String::from)),
            },
            None => None,
        };
        let change = pdt_agents::adapter::SessionChange { model, permission_mode: body.permission_mode };
        crate::conversations::apply_settings(&st, &session, change).await?;
    }
    Ok(Json(load(&st, &id).await?))
}

pub(crate) async fn kill(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let _guard = st.lock_session(&id).await;
    let session = load(&st, &id).await?;
    if session.mode == SessionMode::Conversation {
        crate::conversations::stop(&st, &session).await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    st.kill_pty(&id).await?;
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
    if session.mode == SessionMode::Conversation {
        crate::conversations::reopen(&st, &session).await?;
        return Ok(Json(load(&st, &id).await?));
    }
    if st.ptys.is_running(&id) {
        return Err(ApiError::new(StatusCode::CONFLICT, "already_running", "this session is already running"));
    }
    restart_locked(&st, &id, size, None).await?;
    Ok(Json(load(&st, &id).await?))
}

/// Starts a session that has no running process, resuming its agent conversation. The caller holds the session lock.
pub(crate) async fn restart_locked(
    st: &AppState,
    id: &str,
    size: TerminalSize,
    planned: Option<Resume>,
) -> Result<(), ApiError> {
    settle_exit(st, id).await;
    st.remove_pty(id).await;
    // After the previous run's exit task, which may have just bound its Codex conversation.
    let session = load(st, id).await?;
    let resume = match planned {
        Some(r) => r,
        None => plan_resume(st, &session).await,
    };
    let creds = credentials_for(st, &session.project_id).await?;
    let mut session = session;
    if is_codex(&session) && resume == Resume::Fresh {
        // Launched without a conversation: its new conversation will be matched by this launch time.
        let at = wait_for_fresh_codex_slot(st, &session.cwd).await;
        st.store.set_fresh_started_at(id, at).await?;
        session.fresh_started_at = Some(at);
    }
    let instructions = agent_instructions(st, &session).await?;
    // Mark running before spawning, so an instant exit is recorded after (not before) this update.
    st.store.set_session_running(id).await?;
    if let Err(e) = start_pty(st, &session, size, &resume, creds, None, instructions.as_ref()) {
        // AlreadyExists means another run owns the session (it is running), so leave its status alone.
        if !matches!(e, PtyError::AlreadyExists(_)) {
            st.store.end_session(id, SessionStatus::Interrupted, None, now_ms()).await?;
        }
        return Err(e.into());
    }
    crate::activity::track(st, &session).await;
    Ok(())
}

/// The sessions a previous pdtd run left `interrupted`, each with how to resume it. Made by
/// `plan_resume_interrupted`, started by `ResumePlan::start`.
pub struct ResumePlan(Vec<(Session, Resume)>);

/// Marks a session's auto-resume attempt as over when dropped.
struct ResumeDone<'a> {
    st: &'a AppState,
    id: &'a str,
}

impl Drop for ResumeDone<'_> {
    fn drop(&mut self) {
        self.st.finish_resume(self.id);
    }
}

impl ResumePlan {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Restarts the planned sessions so they come back by themselves. Returns how many started. A session that
    /// fails to start stays interrupted (logged); it never stops the others. A session that is no longer
    /// interrupted when its turn comes (the user restarted or removed it meanwhile) is skipped.
    ///
    /// This can take a while (fresh Codex launches in one folder are spaced out), so pdtd runs it in the
    /// background once it is serving.
    pub async fn start(self, st: &AppState) -> usize {
        let size = TerminalSize { cols: 120, rows: 30 };
        // Concurrently, so sessions in different folders do not wait for each other.
        let results = futures_util::future::join_all(self.0.into_iter().map(|(s, plan)| async move {
            // Declared before the lock, so it drops after it: terminals waiting on this session are released once
            // the attempt is over, however it ends (even a panic or a cancelled future).
            let _done = ResumeDone { st, id: &s.id };
            let _guard = st.lock_session(&s.id).await;
            if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                return false;
            }
            // Re-read under the lock: a manual Restart may have won it, or the user removed the session.
            match st.store.get_session(&s.id).await {
                Ok(Some(c)) if c.status == SessionStatus::Interrupted && !st.ptys.is_running(&c.id) => {}
                _ => return false,
            }
            match restart_locked(st, &s.id, size, Some(plan)).await {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!(session = %s.id, error = ?e, "could not resume session; it stays interrupted");
                    false
                }
            }
        }))
        .await;
        results.into_iter().filter(|ok| *ok).count()
    }
}

/// Decides how to resume each interrupted session; for Codex this binds each session to its own conversation.
/// All of it is decided before any session restarts: a Codex session that relaunches fresh gets a new launch time,
/// which must not change how its neighbours' conversations are matched. Each session resumes its own conversation
/// (see `plan_resume`), so sessions sharing a folder do not interfere.
pub async fn plan_resume_interrupted(st: &AppState) -> ResumePlan {
    let sessions = match st.store.list_sessions_by_status(SessionStatus::Interrupted).await {
        // Conversations reopen on the next message instead (spec §9).
        Ok(v) => v.into_iter().filter(|s| s.mode != SessionMode::Conversation).collect::<Vec<_>>(),
        Err(e) => {
            tracing::warn!(error = %e, "could not list interrupted sessions to resume");
            return ResumePlan(Vec::new());
        }
    };
    // Marked before pdtd serves, so a terminal reconnecting right after a restart waits for the outcome.
    st.pending_resume.lock().unwrap_or_else(|e| e.into_inner()).extend(sessions.iter().map(|s| s.id.clone()));
    let mut plan = Vec::new();
    for s in sessions {
        let resume = plan_resume(st, &s).await;
        plan.push((s, resume));
    }
    ResumePlan(plan)
}

/// Plans and starts the resume of every interrupted session. Returns how many started.
pub async fn resume_interrupted(st: &AppState) -> usize {
    plan_resume_interrupted(st).await.start(st).await
}

/// Stops a session if it runs and forgets it entirely (row, terminal, hook settings). Used when its project or
/// worktree goes away.
pub(crate) async fn discard(st: &AppState, id: &str) {
    let _guard = st.lock_session(id).await;
    crate::conversations::forget(st, id).await;
    if st.ptys.is_running(id) {
        let _ = st.kill_pty(id).await;
    }
    settle_exit(st, id).await;
    st.remove_pty(id).await;
    if let Err(e) = st.store.delete_session(id).await {
        tracing::warn!(session = %id, error = %e, "could not delete the session row");
    }
    let _ = std::fs::remove_dir_all(session_data_dir(st, id));
    crate::activity::forget(st, id);
    drop(_guard);
    st.forget_session_lock(id);
}

/// Removes a session, stopping it first if it runs.
pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    load(&st, &id).await?;
    stop_and_remove(&st, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes a session that has no running process. The caller holds the session lock.
async fn remove_locked(st: &AppState, id: &str) -> Result<(), ApiError> {
    crate::conversations::forget(st, id).await;
    settle_exit(st, id).await;
    st.remove_pty(id).await;
    st.store.delete_session(id).await?;
    // Best-effort: the session's hook settings.
    let _ = std::fs::remove_dir_all(session_data_dir(st, id));
    crate::activity::forget(st, id);
    st.forget_session_lock(id);
    Ok(())
}

/// How long closing a tab waits for its session's process to exit after the kill.
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Stops a session if it runs, waits for it to exit, then deletes it (closing its tab, PDT-28). A session that is
/// already gone is fine. Fails with 409 `session_running` when the process outlives the kill.
pub(crate) async fn stop_and_remove(st: &AppState, id: &str) -> Result<(), ApiError> {
    let _guard = st.lock_session(id).await;
    if st.store.get_session(id).await?.is_none() {
        st.forget_session_lock(id);
        return Ok(());
    }
    if st.ptys.is_running(id) {
        let _ = st.kill_pty(id).await;
        let exited = tokio::time::timeout(STOP_WAIT, async {
            while st.ptys.is_running(id) {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        if exited.is_err() {
            return Err(ApiError::new(StatusCode::CONFLICT, "session_running", "the session did not stop in time"));
        }
    }
    remove_locked(st, id).await
}
