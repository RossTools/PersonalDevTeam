//! Watches (PDT-319): a ticket, workflow run or document (PDT-616) a PM conversation or the user wants to hear about.
//! Each engine tick reads what happened to each watched target since its watch last looked (the event log after its
//! cursor, Inbox items about it, the run's status; a document's `document.*` events, which the documents sensor logs),
//! notes what the watcher asked for, and tells a burst of notes as one message:
//! into the PM conversation through the outbox (which waits for it to be idle, and wakes a parked one), or to the user
//! as a `watch.event` notification. Setting a watch asks nobody: it only tells the watcher's own conversation. A watch
//! whose conversation was closed tells the user what it had left, and ends. A `once` watch ends when its target
//! finishes. The cursor is stored with the notes, so a restart catches up on what it missed, once.
//!
//! A document never finishes: its watch ends when it is removed from PDT or its provider no longer has it, saying so.
//! The watch keeps the document's content as last told (`snapshot`), and a burst of edits is told with one diff from
//! it to the sensor's latest read, fenced, since whoever can edit the document wrote it.
//!
//! Routes: `GET /api/watches?ticket=&run=&document=&session=`, `POST /api/watches` (the user's), `DELETE /api/watches/{id}`.
//! The PM's tools `watch`, `unwatch` and `list_watches` run [`watch`], [`unwatch`] and [`infos`].

use crate::error::{ApiError, OrNotFound, codes};
use crate::extract::ApiQuery;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::notification::{NewNotification, NotificationKind, NotificationTarget};
use pdt_core::pm::Rotation;
use pdt_core::watch::{self, Watch, WatchEvent, WatchInfo, WatchNote, WatchTarget};
use pdt_core::workflow::events as ev;
use schemars::JsonSchema;
use serde::Deserialize;

/// Events a tick reads per watch at most (the rest wait for the next tick).
const EVENTS_PER_TICK: i64 = 200;

/// What to watch and what to hear about (`POST /api/watches`, the PM's `watch`).
#[derive(Debug, Default, Deserialize, JsonSchema, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct NewWatch {
    /// A ticket key (`ABC-12`) or id. Give this, `run` or `document`.
    #[serde(default)]
    #[ts(optional)]
    pub ticket: Option<String>,
    /// A workflow run's id (`list_workflow_runs` gives them).
    #[serde(default)]
    #[ts(optional)]
    pub run: Option<String>,
    /// A document's id, or its exact title (`documents_list` gives them). Import or link one outside PDT first.
    #[serde(default)]
    #[ts(optional)]
    pub document: Option<String>,
    /// What to hear about. A ticket or run: `finished`, `outcome` (an agent step reported), `comment`, `needs_you`,
    /// `failed` (default `finished`, `failed` and `needs_you`). A document: `changed`, `comment` (both by default).
    #[serde(default)]
    #[ts(optional)]
    pub events: Option<Vec<WatchEvent>>,
    /// End the watch once the target finishes (default true; false for a document, which never finishes).
    #[serde(default)]
    #[ts(optional)]
    pub once: Option<bool>,
}

/// The target a request names: (kind, id).
async fn target_of(
    st: &AppState,
    ticket: Option<&str>,
    run: Option<&str>,
    document: Option<&str>,
) -> Result<(WatchTarget, String), ApiError> {
    let given = |v: Option<&str>| v.map(str::trim).filter(|t| !t.is_empty()).map(String::from);
    match (given(ticket), given(run), given(document)) {
        (Some(t), None, None) => {
            let id = crate::mcp::tools::resolve_ticket(st, &t, None).await.map_err(ApiError::invalid)?;
            Ok((WatchTarget::Ticket, id))
        }
        (None, Some(r), None) => {
            let run = st.store.get_workflow_run(&r).await?.or_404("workflow run")?;
            Ok((WatchTarget::Run, run.id))
        }
        (None, None, Some(d)) => {
            let doc = crate::documents::tools::find(st, &d).await.map_err(ApiError::invalid)?;
            Ok((WatchTarget::Document, doc.id))
        }
        _ => Err(ApiError::invalid("give one of a ticket, a run or a document")),
    }
}

/// Starts watching (or changes what an existing watch of the same watcher hears about). `session` is the PM
/// conversation told; None: the user. What already happened is no news: the watch starts from now.
pub(crate) async fn watch(st: &AppState, input: &NewWatch, session: Option<&str>) -> Result<Watch, ApiError> {
    let (target, target_id) =
        target_of(st, input.ticket.as_deref(), input.run.as_deref(), input.document.as_deref()).await?;
    let events = watch::events_or_default(target, input.events.as_deref()).map_err(ApiError::invalid)?;
    let now = now_ms();
    let cursor = latest_seq(st).await?;
    let run_status = match target {
        WatchTarget::Run => st.store.get_workflow_run(&target_id).await?.map(|r| r.status.as_str().to_string()),
        WatchTarget::Ticket | WatchTarget::Document => None,
    };
    let once = input.once.unwrap_or(target.once_by_default());
    // A `once` watch ends when its target finishes: one that already has would never end.
    if once {
        let finished = match target {
            WatchTarget::Run => run_status.as_deref().is_some_and(watch::run_finished),
            WatchTarget::Ticket => st.store.get_ticket(&target_id).await?.is_some_and(|t| t.summary.stage.is_closed()),
            WatchTarget::Document => false,
        };
        if finished {
            return Err(ApiError::conflict(
                codes::ALREADY_FINISHED,
                "it has already finished, so there is nothing to wait for (give `once: false` to hear if it starts again)",
            ));
        }
    }
    let snapshot = match target {
        WatchTarget::Document if events.contains(&WatchEvent::Changed) => {
            crate::documents::watch::first_snapshot(st, &target_id).await
        }
        _ => None,
    };
    let w = Watch {
        id: uuid::Uuid::now_v7().to_string(),
        target,
        target_id,
        session_id: session.map(String::from),
        events,
        once,
        cursor,
        inbox_since: now,
        run_status,
        pending: vec![],
        has_snapshot: snapshot.is_some(),
        created_at: now,
        ended_at: None,
    };
    Ok(st.store.put_watch(&w, snapshot.as_ref()).await?)
}

/// The newest event's `seq` (0 with none).
async fn latest_seq(st: &AppState) -> Result<i64, ApiError> {
    let f = pdt_store::EventFilter { newest_first: true, limit: 1, ..Default::default() };
    Ok(st.store.list_log_events(&f).await?.first().map_or(0, |e| e.seq))
}

/// Ends a watch. A PM conversation (`session`) may end only its own.
pub(crate) async fn unwatch(st: &AppState, id: &str, session: Option<&str>) -> Result<(), ApiError> {
    let w = st.store.get_watch(id).await?.filter(|w| w.ended_at.is_none()).or_404("watch")?;
    if session.is_some() && w.session_id.as_deref() != session {
        return Err(ApiError::forbidden(codes::NOT_YOURS, "that watch is another conversation's"));
    }
    st.store.end_watch(id, now_ms()).await?;
    Ok(())
}

/// A target's label: `ABC-12 “Title”`, `the run of “Workflow”`, or `“Title”` (a document's).
async fn label(st: &AppState, target: WatchTarget, id: &str) -> Option<String> {
    match target {
        WatchTarget::Ticket => {
            st.store.get_ticket(id).await.ok().flatten().map(|t| format!("{} “{}”", t.summary.key, t.summary.title))
        }
        WatchTarget::Run => {
            st.store.get_workflow_run(id).await.ok().flatten().map(|r| format!("the run of “{}”", r.workflow_name))
        }
        WatchTarget::Document => st.store.get_document(id).await.ok().flatten().map(|d| format!("“{}”", d.title)),
    }
}

/// Watches as the API shows them.
pub(crate) async fn infos(st: &AppState, watches: Vec<Watch>) -> Result<Vec<WatchInfo>, ApiError> {
    let mut out = vec![];
    for w in watches {
        // (label, title, state, workflow): what the conversation header's watch list shows of each.
        let (label, target_title, target_state, workflow_id) = match w.target {
            WatchTarget::Ticket => match st.store.get_ticket(&w.target_id).await? {
                Some(t) => {
                    (Some(t.summary.key), Some(t.summary.title), Some(t.summary.stage.as_str().to_string()), None)
                }
                None => (None, None, None, None),
            },
            WatchTarget::Run => match st.store.get_workflow_run(&w.target_id).await? {
                Some(r) => (Some(r.workflow_name), None, Some(r.status.as_str().to_string()), Some(r.workflow_id)),
                None => (None, None, None, None),
            },
            WatchTarget::Document => (st.store.get_document(&w.target_id).await?.map(|d| d.title), None, None, None),
        };
        let target_label = label.unwrap_or_else(|| w.target_id.clone());
        let watcher = match &w.session_id {
            None => "you".to_string(),
            Some(sid) => match st.store.get_session(sid).await? {
                Some(s) => format!("PM “{}”", s.title),
                None => "a PM conversation".to_string(),
            },
        };
        out.push(WatchInfo {
            id: w.id,
            target: w.target,
            target_id: w.target_id,
            target_label,
            target_title,
            target_state,
            workflow_id,
            session_id: w.session_id,
            watcher,
            events: w.events,
            once: w.once,
            created_at: w.created_at,
        });
    }
    Ok(out)
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ListQuery {
    ticket: Option<String>,
    run: Option<String>,
    document: Option<String>,
    /// A PM conversation's watches.
    session: Option<String>,
}

/// `GET /api/watches?ticket=&run=&document=&session=`: a ticket's, run's or document's watches, a conversation's, or
/// every one.
pub(crate) async fn list(
    State(st): State<AppState>,
    ApiQuery(q): ApiQuery<ListQuery>,
) -> Result<Json<Vec<WatchInfo>>, ApiError> {
    let watches = if q.ticket.is_some() || q.run.is_some() || q.document.is_some() {
        let (target, id) = target_of(&st, q.ticket.as_deref(), q.run.as_deref(), q.document.as_deref()).await?;
        st.store.watches_of(target, &id).await?
    } else if let Some(s) = q.session.as_deref() {
        st.store.watches_by(Some(s)).await?
    } else {
        st.store.active_watches().await?
    };
    Ok(Json(infos(&st, watches).await?))
}

/// `POST /api/watches`: the user watches a ticket, run or document.
pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewWatch>, JsonRejection>,
) -> Result<(StatusCode, Json<WatchInfo>), ApiError> {
    let Json(input) = payload?;
    let w = watch(&st, &input, None).await?;
    let info = infos(&st, vec![w]).await?.pop().ok_or_else(|| ApiError::internal("the watch went missing"))?;
    Ok((StatusCode::CREATED, Json(info)))
}

/// `DELETE /api/watches/{id}`: anyone's watch (the user removes a PM's from the ticket's page too).
pub(crate) async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    unwatch(&st, &id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A PM Reset (or anything else that puts a fresh conversation in another's place): its watches follow.
pub async fn follow(st: &AppState, from: &str, to: &str) {
    match st.store.move_watches(from, to, now_ms()).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(from, to, watches = n, "watches followed the conversation"),
        Err(e) => tracing::warn!(from, to, error = %e, "could not move a conversation's watches"),
    }
}

/// One pass over every watch (the engine's tick).
pub async fn tick(st: &AppState) {
    tick_at(st, now_ms()).await
}

/// [`tick`] at `now` (tests move the clock).
pub async fn tick_at(st: &AppState, now: i64) {
    let watches = match st.store.active_watches().await {
        Ok(v) => v,
        Err(e) => return tracing::warn!(error = %e, "could not list the watches"),
    };
    if watches.is_empty() {
        return;
    }
    let inbox = st.store.open_inbox_items(None).await.unwrap_or_default();
    for w in watches {
        if let Err(e) = tick_one(st, w, &inbox, now).await {
            tracing::warn!(error = %e.message(), "could not look at a watch");
        }
    }
}

async fn tick_one(st: &AppState, mut w: Watch, inbox: &[pdt_core::inbox::InboxItem], now: i64) -> Result<(), ApiError> {
    let before = (w.cursor, w.inbox_since, w.run_status.clone(), w.pending.len(), w.has_snapshot);
    let progress = |w: &Watch| (w.cursor, w.inbox_since, w.run_status.clone(), w.pending.len(), w.has_snapshot);
    // A document's new snapshot, to store with the progress (the one it has is read only to tell a change).
    let mut snapshot: Option<pdt_core::document::DocumentContent> = None;
    let mut finished = false;
    // Why a document's watch ends (it is gone), said in its message.
    let mut gone: Option<&'static str> = None;
    let mut notes: Vec<WatchNote> = vec![];
    match w.target {
        WatchTarget::Ticket => {
            if st.store.get_ticket(&w.target_id).await?.is_none() {
                st.store.end_watch(&w.id, now).await?;
                return Ok(());
            }
            let f = pdt_store::EventFilter {
                ticket_id: Some(w.target_id.clone()),
                after: Some(w.cursor),
                limit: EVENTS_PER_TICK,
                ..Default::default()
            };
            for e in st.store.list_log_events(&f).await? {
                w.cursor = w.cursor.max(e.seq);
                if let Some((event, line, ends)) = watch::ticket_note(&e.kind, &e.payload, w.session_id.as_deref()) {
                    finished |= ends;
                    notes.push(WatchNote { event, at: e.created_at, line, ends, outside: None });
                }
            }
        }
        WatchTarget::Run => {
            let Some(run) = st.store.get_workflow_run(&w.target_id).await? else {
                st.store.end_watch(&w.id, now).await?;
                return Ok(());
            };
            // Its agent steps' outcomes are logged on its ticket.
            if let Some(ticket) = &run.ticket_id {
                let f = pdt_store::EventFilter {
                    ticket_id: Some(ticket.clone()),
                    kind: Some(ev::AGENT_OUTCOME.into()),
                    after: Some(w.cursor),
                    limit: EVENTS_PER_TICK,
                    ..Default::default()
                };
                for e in st.store.list_log_events(&f).await? {
                    w.cursor = w.cursor.max(e.seq);
                    if e.payload.get("runId").and_then(|r| r.as_str()) == Some(run.id.as_str()) {
                        let line = watch::outcome_line(&e.payload);
                        notes.push(WatchNote {
                            event: WatchEvent::Outcome,
                            at: e.created_at,
                            line,
                            ends: false,
                            outside: None,
                        });
                    }
                }
            }
            let status = run.status.as_str();
            if w.run_status.as_deref() != Some(status) {
                if let Some((event, line)) = watch::run_note(status, run.error.as_deref()) {
                    let ends = watch::run_finished(status);
                    finished |= ends;
                    // A failed run is also a finished one: told as finished to a watch that didn't ask for failures.
                    let event = if event == WatchEvent::Failed && !w.events.contains(&event) {
                        WatchEvent::Finished
                    } else {
                        event
                    };
                    notes.push(WatchNote { event, at: run.ended_at.unwrap_or(now), line, ends, outside: None });
                }
                w.run_status = Some(status.to_string());
            }
        }
        WatchTarget::Document => {
            if st.store.get_document(&w.target_id).await?.is_none() {
                gone = Some("it was removed from PDT");
            } else if crate::documents::watch::missing(st, &w.target_id).await {
                gone = Some("its provider no longer has it (deleted, or PDT can no longer open it)");
            }
            let f = pdt_store::EventFilter {
                kinds: vec![ev::DOCUMENT_CHANGED.into(), ev::DOCUMENT_COMMENT_ADDED.into()],
                after: Some(w.cursor),
                limit: EVENTS_PER_TICK,
                ..Default::default()
            };
            for e in st.store.list_log_events(&f).await? {
                w.cursor = w.cursor.max(e.seq);
                if !document_copy(&e.payload, &w.target_id) {
                    continue;
                }
                let to_user = w.session_id.is_none();
                if let Some((event, line, outside)) = watch::document_note(&e.kind, &e.payload, to_user) {
                    notes.push(WatchNote { event, at: e.created_at, line, ends: false, outside });
                }
            }
            // The first look's read, for a watch that started before the sensor had one (or its provider could
            // read it): what its first change is diffed against.
            if !w.has_snapshot
                && w.events.contains(&WatchEvent::Changed)
                && !notes.iter().chain(&w.pending).any(|n| n.event == WatchEvent::Changed)
            {
                snapshot = crate::documents::watch::last_read(st, &w.target_id).await;
                w.has_snapshot = snapshot.is_some();
            }
        }
    }
    if w.target != WatchTarget::Document {
        let about = |i: &&pdt_core::inbox::InboxItem| match w.target {
            WatchTarget::Ticket => i.ticket_id.as_deref() == Some(w.target_id.as_str()),
            WatchTarget::Run => i.run_id.as_deref() == Some(w.target_id.as_str()),
            WatchTarget::Document => false,
        };
        let mut newest = w.inbox_since;
        for i in inbox.iter().filter(about).filter(|i| i.created_at > w.inbox_since) {
            newest = newest.max(i.created_at);
            notes.push(WatchNote {
                event: WatchEvent::NeedsYou,
                at: i.created_at,
                line: watch::needs_you_line(&i.title),
                ends: false,
                outside: None,
            });
        }
        w.inbox_since = newest;
    }
    w.pending.extend(notes.into_iter().filter(|n| w.events.contains(&n.event)));
    w.pending.sort_by_key(|n| n.at);

    let ending = (finished && w.once) || gone.is_some();
    let closed = match w.session_id.clone() {
        None => false,
        Some(sid) => match watcher(st, &sid).await {
            Watcher::Open => false,
            Watcher::Closed => true,
            // Mid-reset: the news waits for the fresh conversation.
            Watcher::Resetting => {
                if before != progress(&w) {
                    st.store.set_watch_progress(&w, snapshot.as_ref()).await?;
                }
                return Ok(());
            }
            Watcher::ResetTo(fresh) => {
                follow(st, &sid, &fresh).await;
                w.session_id = Some(fresh);
                false
            }
        },
    };
    let (quiet, max) = if w.target == WatchTarget::Document { st.document_watch_wait_ms } else { st.watch_wait_ms };
    if (!w.pending.is_empty() || gone.is_some()) && (closed || ending || watch::due(&w.pending, now, quiet, max)) {
        let latest = match w.target {
            WatchTarget::Document if w.pending.iter().any(|n| n.event == WatchEvent::Changed) => {
                crate::documents::watch::last_read(st, &w.target_id).await
            }
            _ => None,
        };
        let diff = match &latest {
            Some(b) if w.has_snapshot => match st.store.watch_snapshot(&w.id).await? {
                Some(a) => crate::documents::watch::diff(a, b.clone()).await,
                None => None,
            },
            _ => None,
        };
        tell(st, &w, Told { closed, ending, gone, diff: diff.as_deref() }).await?;
        w.pending.clear();
        if latest.is_some() {
            snapshot = latest;
            w.has_snapshot = true;
        }
    }
    if closed || ending {
        st.store.end_watch(&w.id, now).await?;
    } else if before != progress(&w) {
        // (Telling a change replaces the snapshot, and always moves the cursor or empties `pending` too.)
        st.store.set_watch_progress(&w, snapshot.as_ref()).await?;
    }
    Ok(())
}

/// Whether a `document.*` event is about `document`, and the one copy of it a watch takes (an event is logged once
/// per project the document is linked to: the first project's copy, or the only one of an unlinked document).
fn document_copy(payload: &serde_json::Value, document: &str) -> bool {
    let s = |k: &str| payload.get(k).and_then(|v| v.as_str()).unwrap_or_default();
    if s("documentId") != document {
        return false;
    }
    let first = payload.get("projectIds").and_then(|p| p.as_array()).and_then(|p| p.first()).and_then(|p| p.as_str());
    s("projectId") == first.unwrap_or("")
}

/// Where a watching PM conversation stands.
enum Watcher {
    Open,
    /// Closed or gone: its news goes to the user.
    Closed,
    /// Being reset: the fresh conversation doesn't exist yet.
    Resetting,
    /// Reset: the fresh conversation that carries on from it.
    ResetTo(String),
}

async fn watcher(st: &AppState, sid: &str) -> Watcher {
    if st.pm_resets.lock().is_ok_and(|r| r.contains(sid)) {
        return Watcher::Resetting;
    }
    match st.store.get_pm_session(sid).await {
        Ok(Some(row)) if row.rotation == Rotation::None => Watcher::Open,
        // The user closed it and it is writing its journal (a reset's journal turn is caught above): its news goes
        // to the user, never into the closing conversation, which would hold up its end.
        Ok(Some(row)) if row.rotation == Rotation::Journal => Watcher::Closed,
        Ok(Some(row)) if row.reason == Some(pdt_core::pm::RotateReason::Reset) => {
            let rows = st.store.list_pm_sessions().await.unwrap_or_default();
            match rows.into_iter().find(|r| r.reset_from.as_deref() == Some(sid)) {
                Some(fresh) => Watcher::ResetTo(fresh.session_id),
                // A reset that never made its fresh conversation.
                None => Watcher::Closed,
            }
        }
        Ok(Some(_)) => Watcher::Closed,
        Ok(None) if matches!(st.store.get_session(sid).await, Ok(Some(_))) => Watcher::Open,
        Ok(None) => Watcher::Closed,
        // Unknown: try again next tick.
        Err(_) => Watcher::Resetting,
    }
}

/// How a watch's notes are told.
struct Told<'a> {
    /// Its conversation is closed: the user is told.
    closed: bool,
    /// The watch ends after this.
    ending: bool,
    /// Why a document's watch ends.
    gone: Option<&'a str>,
    /// What changed in a document since its watcher was last told.
    diff: Option<&'a str>,
}

/// Tells a watch's pending notes: to its conversation through the outbox, or to the user (theirs, or a closed
/// conversation's leftovers).
async fn tell(st: &AppState, w: &Watch, how: Told<'_>) -> Result<(), ApiError> {
    let label = label(st, w.target, &w.target_id).await.unwrap_or_else(|| w.target_id.clone());
    let dedupe = match w.pending.first() {
        Some(n) => format!("watch:{}:{}", w.id, n.at),
        None => format!("watch:{}:end", w.id),
    };
    match w.session_id.as_deref().filter(|_| !how.closed) {
        Some(sid) => {
            let text = match w.target {
                WatchTarget::Document => {
                    let title = st.store.get_document(&w.target_id).await?.map(|d| d.title);
                    let fence = pdt_core::fence::Fence::new(uuid::Uuid::new_v4().simple().to_string())
                        .with_budget(pdt_core::fence::BUDGET, None);
                    watch::document_message(
                        &w.target_id,
                        title.as_deref().unwrap_or("(gone)"),
                        &w.pending,
                        how.diff,
                        how.gone,
                        &fence,
                    )
                }
                _ => watch::message(&label, &w.pending, how.ending),
            };
            let ticket_id = (w.target == WatchTarget::Ticket).then(|| w.target_id.clone());
            let m = pdt_store::SessionMessage {
                id: uuid::Uuid::now_v7().to_string(),
                attempt_id: None,
                session_id: Some(sid.to_string()),
                ticket_id,
                dedupe: Some(dedupe),
                text,
                timeline: serde_json::Value::Null,
                // Not the run's own message: the outbox would drop it once the run has ended.
                run_id: None,
                step_id: None,
                status: "waiting".into(),
                told_session: None,
                created_at: now_ms(),
                resolved_at: None,
                by_user: false,
                pending_id: None,
                // The PM wakes up to report it.
                reopens: true,
            };
            st.store.queue_session_message(&m).await?;
            tracing::info!(watch = %w.id, session = %sid, notes = w.pending.len(), "told a PM conversation what it watches");
        }
        None => notify_user(st, w, &label, &dedupe, &how).await?,
    }
    Ok(())
}

/// A `watch.event` notification: the user's watch, or a closed conversation's.
async fn notify_user(st: &AppState, w: &Watch, label: &str, key: &str, how: &Told<'_>) -> Result<(), ApiError> {
    let (target, project_id, ticket_id, run_id) = match w.target {
        WatchTarget::Ticket => {
            let t = st.store.get_ticket(&w.target_id).await?;
            let project = t.as_ref().map(|t| t.summary.project_id.clone());
            let target = match &project {
                Some(p) => NotificationTarget::Ticket { id: w.target_id.clone(), project_id: p.clone() },
                None => NotificationTarget::None,
            };
            (target, project, Some(w.target_id.clone()), None)
        }
        WatchTarget::Run => {
            let r = st.store.get_workflow_run(&w.target_id).await?;
            let target = match &r {
                Some(r) => NotificationTarget::Workflow { id: r.workflow_id.clone() },
                None => NotificationTarget::None,
            };
            let project = r.as_ref().and_then(|r| r.project_id.clone());
            let ticket = r.as_ref().and_then(|r| r.ticket_id.clone());
            (target, project, ticket, Some(w.target_id.clone()))
        }
        WatchTarget::Document => {
            let d = st.store.get_document(&w.target_id).await?;
            let target = match &d {
                Some(d) => NotificationTarget::Document { id: d.id.clone() },
                None => NotificationTarget::None,
            };
            let project = d.as_ref().and_then(|d| d.projects.first().map(|p| p.id.clone()));
            (target, project, None, None)
        }
    };
    let (title, mut body) = match w.target {
        WatchTarget::Document => {
            let title = label.trim_matches(|c| c == '“' || c == '”');
            watch::document_notice(title, &w.pending, how.diff, how.gone)
        }
        _ => {
            let title = match w.pending.as_slice() {
                [one] => format!("{label}: {}", one.line),
                many => format!("{label}: {} updates", many.len()),
            };
            let lines: Vec<String> = w.pending.iter().map(|n| format!("- {}", n.line)).collect();
            (title, lines.join("\n"))
        }
    };
    if let Some(sid) = &w.session_id {
        let who = match st.store.get_session(sid).await? {
            Some(s) => format!("the PM conversation “{}”", s.title),
            None => "a PM conversation".into(),
        };
        body = format!("{} watched this; it was closed, so you're told instead.\n\n{body}", capitalise(&who));
    }
    let mut n = NewNotification::new(NotificationKind::WatchEvent, key.to_string(), title, target);
    n.body = body;
    n.project_id = project_id;
    n.ticket_id = ticket_id;
    n.run_id = run_id;
    crate::notifications::notify(st, n).await;
    Ok(())
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{delete, get};
    axum::Router::new().route("/watches", get(list).post(create)).route("/watches/{id}", delete(self::delete))
}
