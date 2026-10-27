//! The project manager (PDT-81, docs/superpowers/specs/2026-10-21-pm-persona-design.md): PM sessions in the host
//! project, the briefing pdtd writes into each new one, journal checkpoints and daily updates, closing with a journal
//! entry, the notes routes, and the project policy that answers the PM's asking tools for the user.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::conversation::{Answer, PermissionMode, SessionMode};
use pdt_core::pm::{
    self, AskTarget, BriefConversation, BriefEvent, BriefInbox, BriefJournal, BriefNote, BriefProject, BriefTicket,
    BriefingData, NewNote, NoteKind, NotePatch, PmNote, PmSessionInfo, PmSessionPatch, PmState, RotateReason, Rotation,
    StartPm,
};
use pdt_core::project::Project;
use pdt_core::session::{Session, SessionKind, SessionPersona, SessionStatus, TerminalSize, ValidSession, clean_title};
use pdt_core::ticket::{TicketQuery, TicketStage};
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
    for row in st.store.list_pm_sessions().await? {
        let Some(session) = st.store.get_session(&row.session_id).await? else { continue };
        let is_open = row.rotation != Rotation::Done;
        if is_open && current.is_none() {
            current = Some(session.clone());
        }
        let last_activity = if is_open {
            st.store.last_session_event_ts(&row.session_id).await?.unwrap_or(row.created_at).max(row.created_at)
        } else {
            row.ended_at.unwrap_or(row.created_at)
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
        };
        if is_open { open.push(info) } else { ended.push(info) }
    }
    open.sort_by_key(|i| std::cmp::Reverse(i.last_activity));
    open.extend(ended);
    Ok(PmState { current, sessions: open })
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
    Ok((StatusCode::CREATED, Json(new_session(&st, title, text).await?)))
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
        ask_for_journal(&st, &s, true).await;
    }
    Ok(StatusCode::NO_CONTENT)
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

async fn new_session(st: &AppState, title: String, first: Option<String>) -> Result<Session, ApiError> {
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
    };
    let s = crate::sessions::create_session(st, req).await?;
    st.store.insert_pm_session(&s.id, s.created_at).await?;
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

/// What a PM conversation's checkpoint requests and daily updates are queued under, so one waits at a time.
const CHECKPOINT_KEY: &str = "pm:checkpoint";

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
            let text = match update_data(st, id, &row, now).await {
                Ok(d) => pm::update_message(&d),
                Err(e) => return tracing::warn!(session = %id, error = %e.message(), "could not gather a PM update"),
            };
            if !matches!(st.store.set_pm_caught_up(id, row.caught_up_at, now).await, Ok(true)) {
                return;
            }
            tracing::info!(session = %id, "sending a PM conversation its daily update");
            queue_checkpoint(st, &s, text).await;
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
    queue_checkpoint(st, &s, pm::checkpoint_request()).await;
}

/// Whether the user (not PDT) wrote to a PM conversation after `since`.
async fn user_wrote_since(st: &AppState, id: &str, since: i64) -> bool {
    use pdt_core::conversation::{AgentEvent, MessageSource};
    let events = st.store.session_events_of_kind(id, "user_message").await.unwrap_or_default();
    events
        .iter()
        .rev()
        .take_while(|e| e.ts > since)
        .any(|e| matches!(&e.event, AgentEvent::UserMessage { source, .. } if *source != MessageSource::Notice))
}

async fn checkpoint_waiting(st: &AppState, id: &str) -> bool {
    let waiting = st.store.waiting_session_messages().await.unwrap_or_default();
    waiting.iter().any(|m| m.session_id.as_deref() == Some(id) && m.dedupe.as_deref() == Some(CHECKPOINT_KEY))
}

/// Drops a PM conversation's checkpoint request or update that is still waiting.
async fn drop_checkpoints(st: &AppState, id: &str) {
    for m in st.store.waiting_session_messages().await.unwrap_or_default() {
        if m.session_id.as_deref() == Some(id) && m.dedupe.as_deref() == Some(CHECKPOINT_KEY) {
            let _ = st.store.resolve_session_message(&m.id, None, now_ms()).await;
        }
    }
}

/// Queues a checkpoint request or update for a running PM conversation (never into a running turn: the outbox waits
/// for it to be idle, and never wakes a parked one).
async fn queue_checkpoint(st: &AppState, s: &Session, text: String) {
    let m = pdt_store::SessionMessage {
        id: uuid::Uuid::now_v7().to_string(),
        attempt_id: None,
        session_id: Some(s.id.clone()),
        ticket_id: None,
        dedupe: Some(CHECKPOINT_KEY.into()),
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
    Ok(pm::UpdateData { now, since, first: since == row.created_at, events, inbox, journal })
}

/// Asks a PM conversation the user closed (rotation `journal`) for its journal entry, through the outbox (7a's
/// delivery rule) so it never goes into a running turn; `reopens` wakes a parked one. Ends it at once when the
/// request can't be queued.
async fn ask_for_journal(st: &AppState, s: &Session, reopens: bool) {
    let m = pdt_store::SessionMessage {
        id: uuid::Uuid::now_v7().to_string(),
        attempt_id: None,
        session_id: Some(s.id.clone()),
        ticket_id: None,
        dedupe: None,
        text: pm::journal_request(),
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
    let Some(target) = pm::ask_target(tool, input) else { return };
    let Ok(Some(s)) = st.store.get_session(session_id).await else { return };
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
    let project = match target {
        AskTarget::Project(p) => project_id(st, p).await.ok(),
        AskTarget::Ticket(t) => match crate::mcp::tools::resolve_ticket(st, t, None).await {
            Ok(id) => st.store.get_ticket(&id).await.ok().flatten().map(|t| t.summary.project_id),
            Err(_) => None,
        },
        AskTarget::Session(s) => st.store.get_session(s).await.ok().flatten().map(|s| s.project_id),
        AskTarget::Connection { .. } | AskTarget::Always => None,
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
        .ok_or_else(|| format!("no project is called {g} (use a project's key, such as PDT)"))
}

/// A PM session's instructions: PDT's PM context, the persona's instructions and the briefing, made once per session
/// and kept in `<session dir>/briefing.md` so a restart sends the same text.
pub(crate) async fn instructions(st: &AppState, s: &Session) -> Result<String, ApiError> {
    let path: PathBuf = st.data_dir.join("sessions").join(&s.id).join(BRIEFING_FILE);
    let briefing = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            let text = pm::briefing(&briefing_data(st, &s.id).await?);
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
    let tools = crate::mcp::session_tools(st, s).await;
    // An agent step of a personal workflow runs here too (PDT-152): its role and outcome rules.
    let step = crate::workflow::agent::step_section_for(st, s).await;
    Ok(pm::instructions(&pm::PmContext {
        session_id: &s.id,
        port: st.listen_port,
        work_folder: &s.cwd,
        tools: &tools,
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

async fn briefing_data(st: &AppState, session_id: &str) -> Result<BriefingData, ApiError> {
    let now = now_ms();
    let projects = st.store.list_projects().await?;
    let mut keys = std::collections::HashMap::new();
    let mut out_projects = vec![];
    for p in &projects {
        keys.insert(p.id.clone(), p.key.clone());
        let pm_acts = st.store.get_project_settings(&p.id).await?.policy.pm_acts;
        out_projects.push(BriefProject {
            name: p.name.clone(),
            key: p.key.clone(),
            personal: p.is_personal(),
            folder: p.repo_path.clone(),
            stages: stage_counts(st, &p.id).await?,
            pm_acts,
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
        .map(|n| BriefJournal { at: n.updated_at, body: n.body })
        .collect();
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
        if !info.open || info.session.id == session_id {
            continue;
        }
        others.push(BriefConversation {
            title: info.title,
            at: info.last_activity,
            summary: last_said(st, &info.session.id).await?,
        });
    }
    Ok(BriefingData { now, projects: out_projects, in_flight, inbox, since, events, journal, pinned, others })
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
    let mut ticket_keys = std::collections::HashMap::new();
    let mut events = vec![];
    for e in st.store.list_log_events(filter).await? {
        let ticket = match &e.ticket_id {
            Some(t) => {
                if !ticket_keys.contains_key(t) {
                    let key = st.store.get_ticket(t).await?.map(|t| t.summary.key);
                    ticket_keys.insert(t.clone(), key);
                }
                ticket_keys.get(t).cloned().flatten()
            }
            None => None,
        };
        events.push(BriefEvent { at: e.created_at, kind: e.kind, ticket });
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
