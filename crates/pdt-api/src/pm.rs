//! The project manager (PDT-81): PM sessions in the host
//! project, the briefing pdtd writes into each new one, journal checkpoints and daily updates, closing with a journal
//! entry, the notes routes, and the project policy that answers the PM's asking tools for the user.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::conversation::{Answer, PdtMessage, PermissionMode, SessionMode};
use pdt_core::pm::{
    self, AskTarget, BriefConversation, BriefEvent, BriefInbox, BriefJournal, BriefNote, BriefProject, BriefTicket,
    BriefingData, NewNote, NoteKind, NotePatch, PmNote, PmSessionInfo, PmSessionPatch, PmState, RotateReason, Rotation,
    StartPm,
};
use pdt_core::project::Project;
use pdt_core::session::{Session, SessionKind, SessionPersona, SessionStatus, TerminalSize, ValidSession, clean_title};
use pdt_core::ticket::{TicketQuery, TicketStage};
use pdt_core::workflow::api::{Scope, Workflow};
use pdt_store::{NoteFilter, PmSessionRow};
use serde::Deserialize;
use std::path::PathBuf;

/// The built-in PM persona's template id.
pub(crate) const PERSONA: &str = "pm";
const BRIEFING_FILE: &str = "briefing.md";
/// Events a briefing reads.
const BRIEF_EVENTS: i64 = 200;

/// The host project, made on first use with its folder `<data dir>/pm`.
pub(crate) async fn host_project(st: &AppState) -> Result<Project, ApiError> {
    if let Some(p) = st.store.host_project().await? {
        return Ok(p);
    }
    let dir = st.data_dir.join("pm");
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(format!("making the PM's folder: {e}")))?;
    let id = uuid::Uuid::now_v7().to_string();
    Ok(st.store.ensure_host_project(&id, &dir.to_string_lossy(), now_ms()).await?)
}

/// Where a new PM session runs (PDT-100): the folder holding the projects' checkouts and the Projects folder (the host
/// setting; its default `~/work` only while there are no projects), else the home folder, else the PM's own folder.
pub(crate) async fn work_folder(st: &AppState, own: &str) -> Result<String, ApiError> {
    let projects = st.store.list_projects().await?;
    let setting = st.store.get_host_settings().await?.projects_dir;
    let projects_dir = match &setting {
        Some(_) => crate::projects::projects_dir(setting.as_deref()),
        None if projects.is_empty() => crate::projects::projects_dir(None).filter(|d| d.is_dir()),
        None => None,
    };
    let checkouts = projects.iter().map(|p| std::path::Path::new(&p.repo_path));
    let dir = pm::work_folder(checkouts, projects_dir.as_deref()).or_else(|| st.home_dir.clone());
    Ok(dir.map_or_else(|| own.to_string(), |d| d.to_string_lossy().into_owned()))
}

/// Whether a session is a PM session (one of the host project's).
pub(crate) async fn is_pm_session(st: &AppState, s: &Session) -> bool {
    matches!(st.store.get_project(&s.project_id).await, Ok(Some(p)) if p.is_host())
}

/// The PM persona with its current version.
async fn persona(st: &AppState) -> Result<pdt_core::persona::Persona, ApiError> {
    crate::personas::ensure_builtins(st).await;
    st.store.persona_by_builtin(PERSONA).await?.ok_or_else(|| ApiError::internal("the built-in PM persona is missing"))
}

/// `GET /api/pm`.
pub(crate) async fn state(State(st): State<AppState>) -> Result<Json<PmState>, ApiError> {
    Ok(Json(pm_state(&st).await?))
}

/// `GET /api/pm/sessions`: every PM conversation, the open ones first (PDT-149).
pub(crate) async fn list(State(st): State<AppState>) -> Result<Json<Vec<PmSessionInfo>>, ApiError> {
    Ok(Json(pm_state(&st).await?.sessions))
}

async fn pm_state(st: &AppState) -> Result<PmState, ApiError> {
    let mut open = vec![];
    let mut ended = vec![];
    // Newest first: the first open one is the newest (`current`, for clients that know only one).
    let mut current = None;
    let rows = st.store.list_pm_sessions().await?;
    let in_flight = st.pm_resets.lock().map(|r| r.clone()).unwrap_or_default();
    for row in &rows {
        let Some(session) = st.store.get_session(&row.session_id).await? else { continue };
        // One being reset stays listed (and in place) while its reset runs, until the fresh one takes its place
        // (PDT-308). One whose reset never finished (pdtd stopped mid-way) is an ended conversation like a closed one.
        let resetting = in_flight.contains(&row.session_id);
        let is_open = row.rotation != Rotation::Done || resetting;
        if is_open && current.is_none() {
            current = Some(session.clone());
        }
        let last_activity = match (is_open, row.listed_at) {
            (false, _) => row.ended_at.unwrap_or(row.created_at),
            (true, Some(at)) if row.reason == Some(RotateReason::Reset) => at,
            (true, listed_at) => {
                let base = listed_at.unwrap_or(row.created_at);
                st.store.last_session_event_ts(&row.session_id).await?.unwrap_or(base).max(base)
            }
        };
        let journal = st.store.session_journal(&row.session_id).await?.map(|n| n.body);
        let info = PmSessionInfo {
            open: is_open,
            title: session.title.clone(),
            unseen: session.unseen,
            last_activity,
            session,
            rotation: row.rotation,
            reason: row.reason,
            journal,
            via: row.via.clone(),
            reset_from: row.reset_from.clone(),
        };
        if is_open { open.push(info) } else { ended.push(info) }
    }
    open.sort_by_key(|i| std::cmp::Reverse(i.last_activity));
    open.extend(ended);
    Ok(PmState { current, sessions: open })
}

/// When a PM conversation last had activity, as the list orders it.
async fn last_activity(st: &AppState, id: &str) -> Result<i64, ApiError> {
    let state = pm_state(st).await?;
    Ok(state.sessions.iter().find(|i| i.session.id == id).map_or_else(now_ms, |i| i.last_activity))
}

/// `POST /api/pm/sessions`: a new PM conversation, beside the open ones (PDT-149: starting one ends none).
pub(crate) async fn start(
    State(st): State<AppState>,
    payload: Result<Json<StartPm>, JsonRejection>,
) -> Result<(StatusCode, Json<Session>), ApiError> {
    let Json(input) = payload?;
    let text = input.text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let title = match input.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => clean_title(t).ok_or_else(|| bad("a title is 1 to 100 characters"))?,
        None => pm::conversation_title(text.as_deref().unwrap_or("")),
    };
    Ok((StatusCode::CREATED, Json(new_session(&st, title, text, None).await?)))
}

// ---- for integrations that hold PM conversations elsewhere (PDT-304: Discord's DMs) ----

/// Every PM conversation, the open ones first.
pub async fn conversations(st: &AppState) -> Result<Vec<PmSessionInfo>, ApiError> {
    Ok(pm_state(st).await?.sessions)
}

/// A new PM conversation held `via` an integration too (`discord`), titled `title` or after its first message, which
/// it is sent.
pub async fn start_conversation(
    st: &AppState,
    title: Option<&str>,
    text: Option<String>,
    via: &str,
) -> Result<Session, ApiError> {
    let text = text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let title = match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => clean_title(t).ok_or_else(|| bad("a title is 1 to 100 characters"))?,
        None => pm::conversation_title(text.as_deref().unwrap_or("")),
    };
    new_session(st, title, text, Some(via)).await
}

/// Marks a PM conversation held `via` an integration too (None: only in PDT).
pub async fn set_via(st: &AppState, id: &str, via: Option<&str>) -> Result<(), ApiError> {
    pm_session(st, id).await?;
    st.store.set_pm_via(id, via).await?;
    Ok(())
}

/// The user's message to an open PM conversation, from elsewhere: queued as the next turn while one runs, the same as
/// the web's.
pub async fn send_from_user(st: &AppState, id: &str, text: &str) -> Result<(), ApiError> {
    let (row, s) = pm_session(st, id).await?;
    if row.rotation != Rotation::None {
        return Err(ApiError::new(StatusCode::CONFLICT, "pm_closed", "that PM conversation is closed"));
    }
    crate::conversations::send_user_message(st, &s, text).await?;
    user_wrote(st, &s, text).await;
    Ok(())
}

/// Closes a PM conversation the way the web's Close does: it writes its journal entry, then ends.
pub async fn close_conversation(st: &AppState, id: &str) -> Result<(), ApiError> {
    close(State(st.clone()), Path(id.to_string())).await.map(|_| ())
}

/// A PM conversation by id (404 for any other session).
async fn pm_session(st: &AppState, id: &str) -> Result<(PmSessionRow, Session), ApiError> {
    let not_found = || ApiError::not_found("PM conversation");
    let row = st.store.get_pm_session(id).await?.ok_or_else(not_found)?;
    let s = st.store.get_session(id).await?.ok_or_else(not_found)?;
    Ok((row, s))
}

/// `PATCH /api/pm/sessions/{id}`: renames a PM conversation.
pub(crate) async fn patch(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<PmSessionPatch>, JsonRejection>,
) -> Result<Json<Session>, ApiError> {
    let Json(input) = payload?;
    let (_, mut s) = pm_session(&st, &id).await?;
    if let Some(t) = &input.title {
        let title = clean_title(t).ok_or_else(|| bad("a title is 1 to 100 characters"))?;
        st.store.rename_session(&id, &title).await?;
        s.title = title;
    }
    Ok(Json(s))
}

/// `DELETE /api/pm/sessions/{id}`: closes a PM conversation, the only way one ends (spec §5.3). One that said anything
/// is asked for its journal first and ends when that turn does; one that never did ends at once. Closing one that is
/// already ending does nothing.
pub(crate) async fn close(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let (row, s) = pm_session(&st, &id).await?;
    if row.rotation != Rotation::None {
        return Ok(StatusCode::NO_CONTENT);
    }
    if st.store.last_session_event_ts(&id).await?.is_none() {
        end_session(&st, &s, Rotation::None, Some(RotateReason::User)).await;
        return Ok(StatusCode::NO_CONTENT);
    }
    if st.store.set_pm_rotation(&id, Rotation::None, Rotation::Journal, Some(RotateReason::User), now_ms()).await? {
        tracing::info!(session = %id, "closing a PM conversation");
        // A checkpoint or update still waiting is overtaken by the journal request.
        drop_checkpoints(&st, &id).await;
        // The user's click: it reopens a parked conversation to write its journal.
        ask_for_journal(&st, &s, pm::journal_request(), true).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/pm/sessions/{id}/reset` (PDT-308): the conversation writes its journal and ends, and a fresh one with
/// its title, agent, model, permission mode and list position carries on from that journal. Returns the fresh one.
pub(crate) async fn reset(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Session>, ApiError> {
    Ok(Json(reset_conversation(&st, &id).await?))
}

/// Resets a PM conversation the way the web's Reset does; the fresh conversation. The reset runs as a task of its
/// own: a caller that goes away (a closed page, a dropped connection) doesn't stop it half-way.
pub async fn reset_conversation(st: &AppState, id: &str) -> Result<Session, ApiError> {
    if !st.pm_resets.lock().map_err(|_| ApiError::internal("the PM's resets are poisoned"))?.insert(id.to_string()) {
        return Err(ApiError::new(StatusCode::CONFLICT, "pm_closed", "that PM conversation is already ending"));
    }
    let (st2, id2) = (st.clone(), id.to_string());
    let task = tokio::spawn(async move {
        let r = run_reset(&st2, &id2).await;
        if let Ok(mut resets) = st2.pm_resets.lock() {
            resets.remove(&id2);
        }
        r
    });
    task.await.map_err(|e| ApiError::internal(format!("the reset stopped: {e}")))?
}

async fn run_reset(st: &AppState, id: &str) -> Result<Session, ApiError> {
    let (row, s) = pm_session(st, id).await?;
    if row.rotation != Rotation::None {
        return Err(ApiError::new(StatusCode::CONFLICT, "pm_closed", "that PM conversation is already ending"));
    }
    let listed_at = last_activity(st, id).await?;
    st.store.set_pm_listed_at(id, listed_at).await?;
    if st.store.last_session_event_ts(id).await?.is_none() {
        // It never said anything: nothing to write down.
        end_session(st, &s, Rotation::None, Some(RotateReason::Reset)).await;
    } else if st
        .store
        .set_pm_rotation(id, Rotation::None, Rotation::Journal, Some(RotateReason::Reset), now_ms())
        .await?
    {
        tracing::info!(session = %id, "resetting a PM conversation");
        drop_checkpoints(st, id).await;
        if s.status == SessionStatus::Finished {
            // Its agent is gone: it can't write one.
            end_session(st, &s, Rotation::Journal, None).await;
        } else {
            ask_for_journal(st, &s, pm::reset_request(), true).await;
            wait_for_journal(st, &s).await;
        }
    } else {
        return Err(ApiError::new(StatusCode::CONFLICT, "pm_closed", "that PM conversation is already ending"));
    }
    // The journal turn is no reply for the user (PDT-205, PDT-207).
    let _ = st.store.mark_viewed(id, now_ms()).await;
    // When this fails nothing takes its place: it is listed with the ended conversations.
    reset_session(st, &s, &row, listed_at).await
}

/// Waits for a conversation being reset to write its journal and end (`turn_ended` ends it), up to
/// `pm_reset_wait_ms`; then ends it with no journal.
async fn wait_for_journal(st: &AppState, s: &Session) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(st.pm_reset_wait_ms.max(0) as u64);
    loop {
        let row = st.store.get_pm_session(&s.id).await.ok().flatten();
        let done = row.as_ref().is_none_or(|r| r.rotation == Rotation::Done);
        // The row is marked done just before a stand-in journal is written.
        if done && matches!(st.store.session_journal(&s.id).await, Ok(Some(_))) {
            return;
        }
        if std::time::Instant::now() >= deadline {
            if !done {
                tracing::info!(session = %s.id, "a PM conversation being reset wrote no journal in time");
                drop_waiting(st, &s.id, |m| m.dedupe.as_deref() == Some(pm::JOURNAL_KEY)).await;
                end_session(st, s, Rotation::Journal, None).await;
            }
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// The fresh conversation that carries on from `old` (PDT-308): its title, agent, model, permission mode, persona,
/// folder and `via`, listed where it was, with a briefing that leads with its journal.
async fn reset_session(st: &AppState, old: &Session, row: &PmSessionRow, listed_at: i64) -> Result<Session, ApiError> {
    let project = host_project(st).await?;
    let id = uuid::Uuid::now_v7().to_string();
    // Written before the session starts, so its first instructions carry it.
    let text = pm::briefing(&briefing_data(st, &id, Some(&old.id)).await?);
    let path = st.data_dir.join("sessions").join(&id).join(BRIEFING_FILE);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ApiError::internal(format!("making the session's folder: {e}")))?;
    }
    std::fs::write(&path, text).map_err(|e| ApiError::internal(format!("writing the briefing: {e}")))?;
    let p = persona(st).await?;
    let req = crate::sessions::CreateSession {
        valid: ValidSession {
            project_id: project.id.clone(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: old.agent.or(Some(p.setup.agent)),
            bypass_permissions: old.bypass_permissions,
            size: TerminalSize { cols: 120, rows: 30 },
            initial_prompt: None,
            mode: SessionMode::Conversation,
            model: old.model.clone(),
            permission_mode: old.permission_mode,
        },
        title: Some(old.title.clone()),
        attempt_id: None,
        ticket: None,
        tab: None,
        persona: old.persona.clone(),
        cwd: Some(old.cwd.clone()),
        id: Some(id),
    };
    let s = crate::sessions::create_session(st, req).await?;
    st.store.insert_pm_reset(&s.id, s.created_at, &old.id, listed_at).await?;
    // What the old conversation watched, the fresh one does (PDT-319).
    crate::watches::follow(st, &old.id, &s.id).await;
    if row.via.is_some() {
        st.store.set_pm_via(&s.id, row.via.as_deref()).await?;
    }
    Ok(s)
}

/// The user wrote to a PM conversation: one still called [`pm::NEW_TITLE`] is named after the message.
pub(crate) async fn user_wrote(st: &AppState, s: &Session, text: &str) {
    if s.title != pm::NEW_TITLE || text.trim().is_empty() || !is_pm_session(st, s).await {
        return;
    }
    if let Err(e) = st.store.rename_session(&s.id, &pm::conversation_title(text)).await {
        tracing::warn!(session = %s.id, error = %e, "could not title a PM conversation");
    }
}

async fn new_session(
    st: &AppState,
    title: String,
    first: Option<String>,
    via: Option<&str>,
) -> Result<Session, ApiError> {
    let project = host_project(st).await?;
    let p = persona(st).await?;
    let mode = p.setup.permission_mode.unwrap_or(PermissionMode::Ask);
    let req = crate::sessions::CreateSession {
        valid: ValidSession {
            project_id: project.id.clone(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(p.setup.agent),
            bypass_permissions: mode == PermissionMode::Bypass,
            size: TerminalSize { cols: 120, rows: 30 },
            // Sent once the session is a PM session (below): its first turn may end at once.
            initial_prompt: None,
            mode: SessionMode::Conversation,
            model: p.setup.model.clone(),
            permission_mode: Some(mode),
        },
        title: Some(title),
        attempt_id: None,
        ticket: None,
        tab: None,
        persona: Some(SessionPersona { id: p.id.clone(), name: p.name.clone(), version: p.version, step_id: None }),
        cwd: Some(work_folder(st, &project.repo_path).await?),
        id: None,
    };
    let s = crate::sessions::create_session(st, req).await?;
    st.store.insert_pm_session(&s.id, s.created_at).await?;
    if via.is_some() {
        st.store.set_pm_via(&s.id, via).await?;
    }
    if let Some(text) = first {
        crate::conversations::send_user_message(st, &s, &text).await?;
    }
    Ok(s)
}

/// Ends a PM session that is at `from`: its journal (or "(no journal)") recorded, the row done, the conversation
/// stopped. Does nothing when another caller moved it on first.
async fn end_session(st: &AppState, s: &Session, from: Rotation, reason: Option<RotateReason>) {
    match st.store.set_pm_rotation(&s.id, from, Rotation::Done, reason, now_ms()).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => return tracing::warn!(session = %s.id, error = %e, "could not end a PM session"),
    }
    if from == Rotation::Journal && matches!(st.store.session_journal(&s.id).await, Ok(None)) {
        let _ = write_journal(st, s, pm::NO_JOURNAL).await;
    }
    if let Err(e) = crate::conversations::stop(st, s).await {
        tracing::warn!(session = %s.id, error = %e.message(), "could not stop an ended PM session");
    }
}

/// Records a PM session's journal entry (replacing an earlier one of the same session).
pub(crate) async fn write_journal(st: &AppState, s: &Session, body: &str) -> Result<PmNote, ApiError> {
    let now = now_ms();
    if let Some(mut n) = st.store.session_journal(&s.id).await? {
        n.body = body.to_string();
        n.updated_at = now;
        st.store.update_note(&n).await?;
        return Ok(n);
    }
    let n = PmNote {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: None,
        kind: NoteKind::Journal,
        title: format!("Journal of {}", s.title),
        body: body.to_string(),
        pinned: false,
        session_id: Some(s.id.clone()),
        created_at: now,
        updated_at: now,
    };
    st.store.insert_note(&n).await?;
    Ok(n)
}

/// A PM session ended a turn (spec §5.3): one in use gets its daily update when it is due, and one the user closed
/// ends once the turn that was asked for its journal is over. Neither a full context nor age ends one: the agent
/// compacts its own context ([`compacted`]).
pub(crate) async fn turn_ended(st: &AppState, id: &str) {
    let Ok(Some(row)) = st.store.get_pm_session(id).await else { return };
    let Ok(Some(s)) = st.store.get_session(id).await else { return };
    match row.rotation {
        Rotation::Done => {}
        Rotation::Journal => {
            // A turn of the user's that ended before the journal request went in: wait for the request's turn.
            let waiting = st.store.waiting_session_messages().await.unwrap_or_default();
            if waiting.iter().any(|m| m.session_id.as_deref() == Some(id)) {
                return;
            }
            end_session(st, &s, Rotation::Journal, None).await
        }
        Rotation::None => {
            let now = now_ms();
            // News is for a conversation in use: the turn answering an update never asks for another.
            if !pm::update_due(row.caught_up_at, now, st.pm_update_every_ms)
                || !user_wrote_since(st, id, row.caught_up_at).await
                || checkpoint_waiting(st, id).await
            {
                return;
            }
            let message = match update_data(st, id, &row, now).await {
                Ok(d) => pm::daily_message(&d),
                Err(e) => return tracing::warn!(session = %id, error = %e.message(), "could not gather a PM update"),
            };
            if !matches!(st.store.set_pm_caught_up(id, row.caught_up_at, now).await, Ok(true)) {
                return;
            }
            // A quiet day since a checkpoint newer than the user's last message: nothing to say (PDT-211).
            let Some((kind, text)) = message else { return };
            let key = if kind == PdtMessage::Update { pm::UPDATE_KEY } else { pm::CHECKPOINT_KEY };
            tracing::info!(session = %id, kind = ?kind, "sending a PM conversation its daily update");
            queue_checkpoint(st, &s, key, text).await;
        }
    }
}

/// A PM conversation's agent compacted its context: it is asked for a journal checkpoint, through the outbox so the
/// request waits for the turn to end (spec §5.3). The conversation carries on.
pub(crate) async fn compacted(st: &AppState, id: &str) {
    let Ok(Some(row)) = st.store.get_pm_session(id).await else { return };
    if row.rotation != Rotation::None || checkpoint_waiting(st, id).await {
        return;
    }
    let Ok(Some(s)) = st.store.get_session(id).await else { return };
    tracing::info!(session = %id, "asking a compacted PM conversation for a journal checkpoint");
    queue_checkpoint(st, &s, pm::CHECKPOINT_KEY, pm::checkpoint_request()).await;
}

/// Whether the user (not PDT) wrote to a PM conversation after `since`.
async fn user_wrote_since(st: &AppState, id: &str, since: i64) -> bool {
    user_wrote_at(st, id).await.is_some_and(|at| at > since)
}

/// When the user (not PDT) last wrote to a PM conversation.
async fn user_wrote_at(st: &AppState, id: &str) -> Option<i64> {
    use pdt_core::conversation::{AgentEvent, MessageSource};
    let events = st.store.session_events_of_kind(id, "user_message").await.unwrap_or_default();
    events
        .iter()
        .rev()
        .find(|e| matches!(&e.event, AgentEvent::UserMessage { source, .. } if *source != MessageSource::Notice))
        .map(|e| e.ts)
}

/// Whether a waiting outbox message is a PM conversation's checkpoint request or update.
fn is_checkpoint(m: &pdt_store::SessionMessage, id: &str) -> bool {
    m.session_id.as_deref() == Some(id) && matches!(m.dedupe.as_deref(), Some(pm::CHECKPOINT_KEY | pm::UPDATE_KEY))
}

async fn checkpoint_waiting(st: &AppState, id: &str) -> bool {
    let waiting = st.store.waiting_session_messages().await.unwrap_or_default();
    waiting.iter().any(|m| is_checkpoint(m, id))
}

/// Drops a PM conversation's checkpoint request or update that is still waiting.
async fn drop_checkpoints(st: &AppState, id: &str) {
    drop_waiting(st, id, |m| is_checkpoint(m, id)).await;
}

/// Drops a PM conversation's waiting outbox messages that `which` picks.
async fn drop_waiting(st: &AppState, id: &str, which: impl Fn(&pdt_store::SessionMessage) -> bool) {
    for m in st.store.waiting_session_messages().await.unwrap_or_default() {
        if m.session_id.as_deref() == Some(id) && which(&m) {
            let _ = st.store.resolve_session_message(&m.id, None, now_ms()).await;
        }
    }
}

/// Queues a checkpoint request or update for a running PM conversation (never into a running turn: the outbox waits
/// for it to be idle, and never wakes a parked one). `key` ([`pm::UPDATE_KEY`] or [`pm::CHECKPOINT_KEY`]) is how
/// the conversation view knows it (PDT-211).
async fn queue_checkpoint(st: &AppState, s: &Session, key: &str, text: String) {
    let m = pdt_store::SessionMessage {
        id: uuid::Uuid::now_v7().to_string(),
        attempt_id: None,
        session_id: Some(s.id.clone()),
        ticket_id: None,
        dedupe: Some(key.into()),
        text,
        timeline: serde_json::Value::Null,
        run_id: None,
        step_id: None,
        status: "waiting".into(),
        told_session: None,
        created_at: now_ms(),
        resolved_at: None,
        by_user: false,
        pending_id: None,
        reopens: false,
    };
    if let Err(e) = st.store.queue_session_message(&m).await {
        tracing::warn!(session = %s.id, error = %e, "could not queue a PM checkpoint");
    }
}

/// What a PM conversation's daily update says: events, the Inbox, and other conversations' journal entries since it
/// was last brought up to date.
async fn update_data(st: &AppState, id: &str, row: &PmSessionRow, now: i64) -> Result<pm::UpdateData, ApiError> {
    let since = row.caught_up_at;
    let filter =
        pdt_store::EventFilter { since: Some(since), newest_first: true, limit: BRIEF_EVENTS, ..Default::default() };
    let events = brief_events(st, &filter).await?;
    let inbox = brief_inbox(st).await?;
    let mut journal = vec![];
    let entries =
        st.store.search_notes(&NoteFilter { kind: Some(NoteKind::Journal), limit: 50, ..Default::default() }).await?;
    for n in entries.into_iter().filter(|n| n.updated_at > since && n.session_id.as_deref() != Some(id)) {
        let title = match &n.session_id {
            Some(sid) => st.store.get_session(sid).await?.map_or(n.title.clone(), |s| s.title),
            None => n.title.clone(),
        };
        journal.push(pm::BriefConversationJournal { title, at: n.updated_at, body: n.body });
    }
    let checkpointed_at = st.store.session_journal(id).await?.map(|n| n.updated_at);
    let user_wrote_at = user_wrote_at(st, id).await.unwrap_or(0);
    Ok(pm::UpdateData {
        now,
        since,
        first: since == row.created_at,
        events,
        inbox,
        journal,
        checkpointed_at,
        user_wrote_at,
    })
}

/// Asks a PM conversation the user closed or is resetting (rotation `journal`) for its journal entry with `text`, through the outbox (7a's
/// delivery rule) so it never goes into a running turn; `reopens` wakes a parked one. Ends it at once when the
/// request can't be queued.
async fn ask_for_journal(st: &AppState, s: &Session, text: String, reopens: bool) {
    let m = pdt_store::SessionMessage {
        id: uuid::Uuid::now_v7().to_string(),
        attempt_id: None,
        session_id: Some(s.id.clone()),
        ticket_id: None,
        dedupe: Some(pm::JOURNAL_KEY.into()),
        text,
        timeline: serde_json::Value::Null,
        run_id: None,
        step_id: None,
        status: "waiting".into(),
        told_session: None,
        created_at: now_ms(),
        resolved_at: None,
        by_user: false,
        pending_id: None,
        reopens,
    };
    if let Err(e) = st.store.queue_session_message(&m).await {
        tracing::warn!(session = %s.id, error = %e, "could not ask the PM session for its journal");
        end_session(st, s, Rotation::Journal, None).await;
    }
}

/// A conversation opened a permission request: when it is a PM asking tool whose target project lets the PM act,
/// PDT answers yes for the user (spec §4).
pub(crate) async fn request_opened(
    st: &AppState,
    session_id: &str,
    request_id: &str,
    tool: &str,
    input: &serde_json::Value,
) {
    let Some(target) = st.registry().ask_target(tool, input) else { return };
    let Ok(Some(s)) = st.store.get_session(session_id).await else { return };
    // Letting an external ticket in always asks the user (PDT-201): their yes is kept for the call it allows.
    let short = tool.strip_prefix(&format!("mcp__{}__", pdt_core::mcp::SERVER_NAME)).unwrap_or(tool);
    if crate::untrusted::needs_the_user(st, &s, short, input).await {
        tracing::info!(session = %session_id, tool, "a call that lets an external ticket in: the user answers it");
        st.grants.hold(session_id, request_id, short, input);
        return;
    }
    if !acts_without_asking(st, &target).await {
        return;
    }
    tracing::info!(session = %session_id, tool, "the project lets the PM act: allowing without asking");
    if let Err(e) = crate::conversations::answer_request(st, &s, request_id, &Answer::Approve { always: false }).await {
        tracing::warn!(session = %session_id, error = %e.message(), "could not answer the PM's request");
    }
}

/// Whether the project an asking tool's call acts on lets the PM act without asking.
pub(crate) async fn acts_without_asking(st: &AppState, target: &AskTarget) -> bool {
    if let AskTarget::Connection { kind, name } = target {
        return crate::workflow::routes::connection_of_kind(st, kind, name.as_deref())
            .await
            .is_ok_and(|c| pm::connection_lets_pm_act(&c.config));
    }
    if let AskTarget::Document(d) = target {
        let Ok(Some(d)) = st.store.get_document(d).await else { return false };
        for p in &d.projects {
            if st.store.get_project_settings(&p.id).await.is_ok_and(|s| s.policy.pm_acts) {
                return true;
            }
        }
        return false;
    }
    let project = match target {
        AskTarget::Project(p) => project_id(st, p).await.ok(),
        AskTarget::Ticket(t) => match crate::mcp::tools::resolve_ticket(st, t, None).await {
            Ok(id) => st.store.get_ticket(&id).await.ok().flatten().map(|t| t.summary.project_id),
            Err(_) => None,
        },
        AskTarget::Session(s) => st.store.get_session(s).await.ok().flatten().map(|s| s.project_id),
        AskTarget::Connection { .. } | AskTarget::Document(_) | AskTarget::Always => None,
    };
    let Some(project) = project else { return false };
    st.store.get_project_settings(&project).await.is_ok_and(|s| s.policy.pm_acts)
}

/// A project's id from its id or key (not the host project).
pub(crate) async fn project_id(st: &AppState, given: &str) -> Result<String, String> {
    let g = given.trim();
    let projects = st.store.list_projects().await.map_err(|e| e.to_string())?;
    projects
        .into_iter()
        .find(|p| p.id == g || p.key.eq_ignore_ascii_case(g) || p.name.eq_ignore_ascii_case(g))
        .map(|p| p.id)
        .ok_or_else(|| format!("no project is called {g} (use a project's key, such as ABC)"))
}

/// A PM session's instructions: PDT's PM context, the persona's instructions and the briefing, made once per session
/// and kept in `<session dir>/briefing.md` so a restart sends the same text.
pub(crate) async fn instructions(st: &AppState, s: &Session) -> Result<String, ApiError> {
    let path: PathBuf = st.data_dir.join("sessions").join(&s.id).join(BRIEFING_FILE);
    let briefing = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            let reset_from = st.store.get_pm_session(&s.id).await?.and_then(|r| r.reset_from);
            let text = pm::briefing(&briefing_data(st, &s.id, reset_from.as_deref()).await?);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, &text) {
                tracing::warn!(session = %s.id, error = %e, "could not keep the PM session's briefing");
            }
            text
        }
    };
    let persona_text = match &s.persona {
        Some(p) => st.store.get_persona_version(&p.id, p.version).await?.map(|v| v.setup.instructions),
        None => None,
    }
    .unwrap_or_else(|| pdt_core::persona::PM_INSTRUCTIONS.to_string());
    let reg = st.registry();
    let tools = crate::mcp::session_tools(st, s).await;
    // An agent step of a personal workflow runs here too (PDT-152): its role and outcome rules.
    let step = crate::workflow::agent::step_section_for(st, s).await;
    Ok(pm::instructions(&pm::PmContext {
        session_id: &s.id,
        port: st.listen_port,
        work_folder: &s.cwd,
        tools: &tools,
        tool_table: reg.tool_table(),
        integrations: &reg.pm_prompt(&tools),
        persona: &persona_text,
        briefing: &briefing,
        step: step.as_deref(),
    }))
}

fn stage_label(s: TicketStage) -> &'static str {
    match s {
        TicketStage::Draft => "Draft",
        TicketStage::Ready => "Ready",
        TicketStage::Todo => "To do",
        TicketStage::InProgress => "In progress",
        TicketStage::Done => "Done",
        TicketStage::WontDo => "Won't do",
    }
}

const OPEN_STAGES: [TicketStage; 4] =
    [TicketStage::Draft, TicketStage::Ready, TicketStage::Todo, TicketStage::InProgress];

/// Open tickets per stage of one project (stages with none left out).
pub(crate) async fn stage_counts(st: &AppState, project_id: &str) -> Result<Vec<(String, u32)>, ApiError> {
    let mut out = vec![];
    for stage in OPEN_STAGES {
        let q = TicketQuery {
            project: Some(project_id.to_string()),
            stage: Some(stage.as_str().to_string()),
            limit: Some(1),
            ..Default::default()
        };
        let n = st.store.query_tickets(&q.validate()?).await?.total;
        if n > 0 {
            out.push((stage_label(stage).to_string(), n));
        }
    }
    Ok(out)
}

/// Tickets in progress (every project, or one), with PR, CI and agent state as the API reports them.
pub(crate) async fn in_flight(
    st: &AppState,
    project: Option<&str>,
) -> Result<Vec<pdt_core::ticket::TicketSummary>, ApiError> {
    let q = TicketQuery {
        project: project.map(String::from),
        stage: Some(TicketStage::InProgress.as_str().into()),
        limit: Some(100),
        ..Default::default()
    };
    let Json(page) = crate::tickets::list(State(st.clone()), Ok(Query(q))).await?;
    Ok(page.items)
}

/// The enabled release workflow (Cut a release, made from its template, or a copy) that runs in a project, by name
/// (PDT-296, PDT-301): the PM's only release guidance comes with it.
async fn release_workflow(st: &AppState, workflows: &[Workflow], project_id: &str) -> Result<Option<String>, ApiError> {
    let reg = st.registry();
    let template = format!("{}{}", crate::workflow::templates::BUILTIN_PREFIX, crate::workflow::builtins::CUT_RELEASE);
    for w in workflows.iter().filter(|w| w.enabled && !crate::workflow::builtins::hidden(&reg, w)) {
        let here = match w.scope {
            Scope::Project => w.project_id.as_deref() == Some(project_id),
            Scope::Host => true,
            Scope::Personal => false,
        };
        if !here {
            continue;
        }
        let from_template = st
            .store
            .workflow_state(&w.id)
            .await?
            .get(crate::workflow::templates::TEMPLATE_STATE)
            .is_some_and(|t| t["id"] == template.as_str());
        if from_template || w.name.eq_ignore_ascii_case("Cut a release") {
            return Ok(Some(w.name.clone()));
        }
    }
    Ok(None)
}

/// A new PM session's briefing; `reset_from`: the conversation it carries on from (PDT-308), whose journal leads it.
async fn briefing_data(st: &AppState, session_id: &str, reset_from: Option<&str>) -> Result<BriefingData, ApiError> {
    let now = now_ms();
    let projects = st.store.list_projects().await?;
    let mut keys = std::collections::HashMap::new();
    let workflows = st.store.list_workflows(None).await?;
    let mut out_projects = vec![];
    for p in &projects {
        keys.insert(p.id.clone(), p.key.clone());
        let pm_acts = st.store.get_project_settings(&p.id).await?.policy.pm_acts;
        let release = if p.is_personal() { None } else { release_workflow(st, &workflows, &p.id).await? };
        out_projects.push(BriefProject {
            name: p.name.clone(),
            key: p.key.clone(),
            personal: p.is_personal(),
            folder: p.repo_path.clone(),
            stages: stage_counts(st, &p.id).await?,
            pm_acts,
            release,
        });
    }
    let in_flight = in_flight(st, None)
        .await?
        .iter()
        .map(|t| BriefTicket {
            key: t.key.clone(),
            title: t.title.clone(),
            stage: stage_label(t.stage).into(),
            state: pm::ticket_state(t),
        })
        .collect();
    let inbox = brief_inbox(st).await?;
    let since =
        st.store.list_pm_sessions().await?.into_iter().find(|r| r.session_id != session_id).map(|r| r.created_at);
    let filter = pdt_store::EventFilter {
        since: Some(since.unwrap_or(now - pm::UPDATE_EVERY_MS)),
        newest_first: true,
        limit: BRIEF_EVENTS,
        ..Default::default()
    };
    let events = brief_events(st, &filter).await?;
    let journal = st
        .store
        .search_notes(&NoteFilter {
            kind: Some(NoteKind::Journal),
            limit: pm::BRIEF_JOURNALS as i64,
            ..Default::default()
        })
        .await?
        .into_iter()
        .filter(|n| reset_from.is_none() || n.session_id.as_deref() != reset_from)
        .map(|n| BriefJournal { at: n.updated_at, body: n.body })
        .collect();
    let own = match reset_from {
        Some(id) => st.store.session_journal(id).await?.map(|n| BriefJournal { at: n.updated_at, body: n.body }),
        None => None,
    };
    let pinned = st
        .store
        .search_notes(&NoteFilter { kind: Some(NoteKind::Note), pinned: Some(true), limit: 100, ..Default::default() })
        .await?
        .into_iter()
        .map(|n| BriefNote {
            id: n.id,
            project: n.project_id.as_ref().and_then(|p| keys.get(p).cloned()),
            title: n.title,
        })
        .collect();
    let mut others = vec![];
    for info in pm_state(st).await?.sessions {
        if !info.open || info.session.id == session_id || Some(info.session.id.as_str()) == reset_from {
            continue;
        }
        others.push(BriefConversation {
            title: info.title,
            at: info.last_activity,
            summary: last_said(st, &info.session.id).await?,
        });
    }
    Ok(BriefingData {
        now,
        projects: out_projects,
        in_flight,
        inbox,
        since,
        events,
        journal,
        reset_from: own,
        pinned,
        others,
    })
}

/// The Inbox, for a briefing or an update.
async fn brief_inbox(st: &AppState) -> Result<Vec<BriefInbox>, ApiError> {
    let Json(inbox) = crate::inbox::list(State(st.clone()), Query(Default::default())).await?;
    Ok(inbox
        .items
        .into_iter()
        .map(|i| BriefInbox {
            kind: serde_json::to_value(i.group).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
            ticket: i.ticket_key,
            title: i.title,
        })
        .collect())
}

/// Events of the log, with their tickets' keys, for a briefing or an update.
async fn brief_events(st: &AppState, filter: &pdt_store::EventFilter) -> Result<Vec<BriefEvent>, ApiError> {
    let mut tickets = std::collections::HashMap::new();
    let mut events = vec![];
    for e in st.store.list_log_events(filter).await? {
        let (ticket, title) = match &e.ticket_id {
            Some(t) => {
                if !tickets.contains_key(t) {
                    let found = st.store.get_ticket(t).await?.map(|t| (t.summary.key, t.summary.title));
                    tickets.insert(t.clone(), found);
                }
                tickets.get(t).cloned().flatten().unzip()
            }
            None => (None, None),
        };
        events.push(BriefEvent { at: e.created_at, kind: e.kind, ticket, title, payload: e.payload });
    }
    Ok(events)
}

/// Events a conversation's summary line looks back through.
const SUMMARY_EVENTS: i64 = 50;

/// A conversation's last message (the agent's, else the user's) as one line; empty when nothing was said.
async fn last_said(st: &AppState, session_id: &str) -> Result<String, ApiError> {
    use pdt_core::conversation::AgentEvent;
    let events = st.store.recent_session_events(session_id, SUMMARY_EVENTS).await?;
    let last = events.iter().rev().find_map(|e| match &e.event {
        AgentEvent::AgentMessage { markdown, .. } if !markdown.trim().is_empty() => Some(markdown.as_str()),
        AgentEvent::UserMessage { text, .. } if !text.trim().is_empty() => Some(text.as_str()),
        _ => None,
    });
    Ok(last.map(pm::summary_line).unwrap_or_default())
}

// ---- notes ----

#[derive(Debug, Default, Deserialize)]
pub(crate) struct NotesQuery {
    q: Option<String>,
    project: Option<String>,
    kind: Option<String>,
    limit: Option<i64>,
}

/// `GET /api/pm/notes?q=&project=&kind=`.
pub(crate) async fn list_notes(
    State(st): State<AppState>,
    Query(q): Query<NotesQuery>,
) -> Result<Json<Vec<PmNote>>, ApiError> {
    let project_id = match q.project.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Some(project_id(&st, p).await.map_err(bad)?),
        None => None,
    };
    let kind = match q.kind.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => Some(NoteKind::parse(k).ok_or_else(|| bad(format!("{k} is not a note kind")))?),
        None => None,
    };
    let f = NoteFilter { q: q.q, project_id, kind, pinned: None, limit: q.limit.unwrap_or(200) };
    Ok(Json(st.store.search_notes(&f).await?))
}

fn bad(m: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", m)
}

/// A note made by the user or the PM: the PM's (`note`) unless it is the user's own (`user`).
pub(crate) async fn add_note(st: &AppState, input: NewNote) -> Result<PmNote, ApiError> {
    let kind = match input.kind.unwrap_or_default() {
        NoteKind::Journal => return Err(bad("journal entries are written by PM conversations")),
        k => k,
    };
    let title = pm::valid_title(&input.title).map_err(bad)?;
    let body = pm::valid_body(&input.body).map_err(bad)?;
    let project_id = match input.project.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Some(project_id(st, p).await.map_err(bad)?),
        None => None,
    };
    let now = now_ms();
    let n = PmNote {
        id: uuid::Uuid::now_v7().to_string(),
        project_id,
        kind,
        title,
        body,
        pinned: input.pinned.unwrap_or(false),
        session_id: None,
        created_at: now,
        updated_at: now,
    };
    st.store.insert_note(&n).await?;
    Ok(n)
}

/// Changes a note.
pub(crate) async fn edit_note(st: &AppState, id: &str, p: NotePatch) -> Result<PmNote, ApiError> {
    let mut n = st.store.get_note(id).await?.ok_or_else(|| ApiError::not_found("note"))?;
    if let Some(t) = &p.title {
        n.title = pm::valid_title(t).map_err(bad)?;
    }
    if let Some(b) = &p.body {
        n.body = pm::valid_body(b).map_err(bad)?;
    }
    if let Some(pinned) = p.pinned {
        n.pinned = pinned;
    }
    if let Some(project) = &p.project {
        n.project_id = match project.trim() {
            "" => None,
            given => Some(project_id(st, given).await.map_err(bad)?),
        };
    }
    n.updated_at = now_ms();
    if !st.store.update_note(&n).await? {
        return Err(ApiError::not_found("note"));
    }
    Ok(n)
}

/// `POST /api/pm/notes`.
pub(crate) async fn create_note(
    State(st): State<AppState>,
    payload: Result<Json<NewNote>, JsonRejection>,
) -> Result<(StatusCode, Json<PmNote>), ApiError> {
    let Json(input) = payload?;
    Ok((StatusCode::CREATED, Json(add_note(&st, input).await?)))
}

/// `GET /api/pm/notes/{id}`.
pub(crate) async fn get_note(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<PmNote>, ApiError> {
    Ok(Json(st.store.get_note(&id).await?.ok_or_else(|| ApiError::not_found("note"))?))
}

/// `PATCH /api/pm/notes/{id}`.
pub(crate) async fn patch_note(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<NotePatch>, JsonRejection>,
) -> Result<Json<PmNote>, ApiError> {
    let Json(input) = payload?;
    Ok(Json(edit_note(&st, &id, input).await?))
}

/// `DELETE /api/pm/notes/{id}`.
pub(crate) async fn delete_note(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !st.store.delete_note(&id).await? {
        return Err(ApiError::not_found("note"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Whether a session's status still lets it take messages (for `send_to_session`).
pub(crate) fn takes_messages(s: &Session) -> bool {
    s.kind == SessionKind::AgentTui && !matches!(s.status, SessionStatus::Finished)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, patch, post};
    axum::Router::new()
        .route("/pm", get(state))
        .route("/pm/sessions", get(list).post(start))
        .route("/pm/sessions/{id}", patch(self::patch).delete(close))
        .route("/pm/sessions/{id}/reset", post(reset))
        .route("/pm/notes", get(list_notes).post(create_note))
        .route("/pm/notes/{id}", get(get_note).patch(patch_note).delete(delete_note))
}
