//! Notifications (PDT-108): the Notifier makes each one once, in the `notifications` table, and every client shows it
//! from there. Event causes (a session's agent exiting, PR news, failed runs, `notify` steps) call [`notify`] where
//! they happen; causes that are state (a session waiting on the user or left unseen, open Inbox items) are kept in
//! step by [`sync`], which makes a row when its cause appears and clears it when the cause goes away. It runs every
//! 2 s and at once when something [`kick`]s it. A todo coming due (PDT-140) is state too: its one reminder per due date
//! is remembered on the todo, so a pdtd that was down at the due moment makes it once when it starts. Needs-you notifications never go through workflows: they work with
//! every workflow off.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use pdt_core::inbox::{InboxItem, InboxKind};
use pdt_core::notification::{
    Change, KindSetting, NewNotification, Notification, NotificationKind as K, NotificationList, NotificationSettings,
    NotificationTarget, RETENTION_MS, ReadAll, Section, Severity, Show, attempt_key, inbox_key, session_key,
    session_wants, todo_key,
};
use pdt_core::pr::{Mergeable, NoticeKind, PrState};
use pdt_core::session::{SessionKind, SessionStatus};
use pdt_core::workflow::api::LogEvent;
use pdt_core::workflow::events as ev;
use serde::Deserialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use tokio::sync::broadcast;

/// The host setting that holds the overrides of each kind's Show.
pub const SETTINGS_KEY: &str = "notifications.show";
/// How often [`run_notifier`] syncs without a kick.
const SYNC_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
/// How often old rows are pruned.
const PRUNE_EVERY_MS: i64 = 60 * 60 * 1000;

/// The Notifier's in-memory side: the stream's channel, the kick, the settings and the sessions PDT itself stopped.
pub struct Notifier {
    tx: broadcast::Sender<(Change, Notification)>,
    kick: tokio::sync::Notify,
    settings: Mutex<Option<NotificationSettings>>,
    /// Sessions whose process PDT killed (Stop, close, restart): their exit is not news.
    killed: Mutex<HashSet<String>>,
    last_prune: Mutex<i64>,
}

impl Default for Notifier {
    fn default() -> Self {
        Self {
            tx: broadcast::channel(256).0,
            kick: tokio::sync::Notify::new(),
            settings: Mutex::new(None),
            killed: Mutex::new(HashSet::new()),
            last_prune: Mutex::new(0),
        }
    }
}

impl Notifier {
    /// Asks for a sync now (a session's activity changed, it was viewed, an Inbox item opened or closed).
    pub fn kick(&self) {
        self.kick.notify_one();
    }
    /// PDT is about to stop a session's process: its exit is the user's doing.
    pub(crate) fn expect_exit(&self, session_id: &str) {
        self.killed.lock().unwrap_or_else(|e| e.into_inner()).insert(session_id.to_string());
    }
    /// Whether PDT stopped the session (and forgets it).
    pub(crate) fn take_expected_exit(&self, session_id: &str) -> bool {
        self.killed.lock().unwrap_or_else(|e| e.into_inner()).remove(session_id)
    }
    /// Every notification change from now on (an integration that passes them on, PDT-304).
    pub fn subscribe(&self) -> broadcast::Receiver<(Change, Notification)> {
        self.tx.subscribe()
    }
    fn send(&self, change: Change, n: &Notification) {
        let _ = self.tx.send((change, n.clone()));
    }
}

pub(crate) fn kick(st: &AppState) {
    st.notifier.kick();
}

/// The host's settings (read once, then kept in memory until they change).
pub(crate) async fn settings(st: &AppState) -> NotificationSettings {
    if let Some(s) = st.notifier.settings.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return s;
    }
    let stored = st.store.get_host_value(SETTINGS_KEY).await.ok().flatten();
    let s = stored.as_ref().map(NotificationSettings::from_json).unwrap_or_default();
    *st.notifier.settings.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    s
}

/// Makes (or updates) a notification at its kind's Show; nothing below `inbox` is stored. An update to a live row
/// the user dismissed (PDT-224) is kept quiet: the same cause doesn't raise it again.
pub(crate) async fn notify(st: &AppState, n: NewNotification) -> Option<Notification> {
    let show = settings(st).await.show(n.kind);
    if !show.stored() {
        return None;
    }
    match st.store.put_notification(&n, show, now_ms()).await {
        Ok((row, change)) => {
            if row.dismissed_at.is_none() {
                st.notifier.send(change, &row);
            }
            Some(row)
        }
        Err(e) => {
            tracing::warn!(kind = n.kind.as_str(), error = %e, "could not record a notification");
            None
        }
    }
}

/// Clears a cause's live row, if it has one.
pub(crate) async fn clear(st: &AppState, dedupe_key: &str) {
    match st.store.clear_notification(dedupe_key, now_ms()).await {
        Ok(Some(row)) => st.notifier.send(Change::Cleared, &row),
        Ok(None) => {}
        Err(e) => tracing::warn!(key = dedupe_key, error = %e, "could not clear a notification"),
    }
}

/// Keeps the state-made rows in step with their causes, until shutdown.
pub async fn run_notifier(st: AppState) {
    st.loops.register(LOOP, SYNC_EVERY, None);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(SYNC_EVERY) => {}
            _ = st.notifier.kick.notified() => {}
        }
        if st.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let pass = st.clone();
        let ended = crate::loops::contain(LOOP, None, async move { notifier_pass(&pass).await }).await;
        st.loops.passed(LOOP, now_ms(), ended == crate::loops::Ended::Panicked);
    }
}

/// The notifier's name among the background loops.
const LOOP: &str = "notifier";

async fn notifier_pass(st: &AppState) {
    sync(st).await;
    let now = now_ms();
    let due = {
        let mut last = st.notifier.last_prune.lock().unwrap_or_else(|e| e.into_inner());
        let due = now - *last >= PRUNE_EVERY_MS;
        if due {
            *last = now;
        }
        due
    };
    if due && let Err(e) = st.store.prune_notifications(now - RETENTION_MS).await {
        tracing::warn!(error = %e, "could not prune notifications");
    }
}

/// One pass over the state-made causes: sessions waiting or left unseen, open Inbox items, PRs whose conflict or
/// queue drop was fixed, sessions restarted or removed after exiting, background loops gone stale. Never fails; problems are logged.
pub async fn sync(st: &AppState) {
    if let Err(e) = sync_inner(st).await {
        tracing::warn!(error = ?e, "could not sync notifications");
    }
}

async fn sync_inner(st: &AppState) -> Result<(), ApiError> {
    // Unheard notices whose cause went away close first, so their rows clear in this pass.
    crate::inbox::settle_notices(st, None).await;
    let now = now_ms();
    let cfg = settings(st).await;
    let live: HashMap<String, Notification> =
        st.store.live_notifications().await?.into_iter().map(|n| (n.dedupe_key.clone(), n)).collect();
    let mut wanted: HashSet<String> = HashSet::new();

    // Sessions: waiting on the user, or a turn left unseen. A conversation with an open approval is the approval's.
    let approvals = crate::inbox::approvals(st, None).await?;
    let asking: HashSet<String> = approvals.iter().filter_map(|i| i.session_id.clone()).collect();
    for s in st.store.list_sessions_by_status(SessionStatus::Running).await? {
        if s.kind != SessionKind::AgentTui {
            continue;
        }
        let Some(kind) = session_wants(s.activity, s.attention_at, s.last_viewed_at, now) else { continue };
        if (kind == K::SessionNeedsYou && asking.contains(&s.id)) || !cfg.show(kind).stored() {
            continue;
        }
        let key = session_key(&s.id, kind);
        if !live.contains_key(&key) {
            let body = if kind == K::SessionNeedsYou {
                "Waits on a permission prompt or a question."
            } else {
                "Finished its turn."
            };
            let mut n = NewNotification::new(
                kind,
                &key,
                s.title.clone(),
                NotificationTarget::Session { id: s.id.clone(), project_id: s.project_id.clone() },
            );
            n.body = with_ticket(body, s.ticket.as_ref().map(|t| t.key.as_str()));
            n.project_id = Some(s.project_id.clone());
            n.ticket_id = s.ticket.as_ref().map(|t| t.id.clone());
            n.session_id = Some(s.id.clone());
            notify(st, n).await;
        }
        wanted.insert(key);
    }

    // Inbox items, stored and approvals.
    let mut items = st.store.open_inbox_items(None).await?;
    items.extend(approvals);
    items.extend(crate::inbox::connect_forge(st));
    let fresh: Vec<&InboxItem> = items.iter().filter(|i| !live.contains_key(&inbox_key(&i.id))).collect();
    let mut fresh: Vec<InboxItem> = fresh.into_iter().cloned().collect();
    crate::inbox::decorate(st, &mut fresh).await?;
    for i in &fresh {
        let kind = K::of_inbox(i.kind, i.group);
        let mut n = NewNotification::new(
            kind,
            inbox_key(&i.id),
            i.title.clone(),
            NotificationTarget::Inbox { id: i.id.clone() },
        );
        let label = if i.group == pdt_core::inbox::InboxGroup::Accept { "Ready to accept" } else { kind.label() };
        let what =
            [Some(label.to_string()), i.persona_name.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        n.body = with_ticket(&what, i.ticket_key.as_deref());
        n.severity = if matches!(
            i.kind,
            InboxKind::Stalled | InboxKind::TimedOut | InboxKind::LoopLimit | InboxKind::Notice | InboxKind::Connect
        ) {
            Severity::Warning
        } else {
            Severity::Info
        };
        n.project_id = i.project_id.clone();
        n.ticket_id = i.ticket_id.clone();
        n.session_id = i.session_id.clone();
        n.run_id = i.run_id.clone();
        n.inbox_item_id = Some(i.id.clone());
        notify(st, n).await;
    }
    wanted.extend(items.iter().map(|i| inbox_key(&i.id)));

    // Background loops without a finished pass for too long (PDT-127). The notifier's own can't be seen here.
    if cfg.show(K::HostLoopStale).stored() {
        for l in st.loops.health(now).into_iter().filter(|l| l.stale) {
            let key = loop_key(&l.name);
            if !live.contains_key(&key) {
                let since = l
                    .last_pass_at
                    .map_or("since it started".to_string(), |at| format!("for {} min", (now - at) / 60_000));
                let mut n = NewNotification::new(
                    K::HostLoopStale,
                    &key,
                    format!("PDT's {} loop stopped", l.name),
                    NotificationTarget::None,
                );
                n.severity = Severity::Error;
                n.body = format!("It has not finished a pass {since}; see pdtd's log. Restarting PDT starts it again.");
                notify(st, n).await;
            }
            wanted.insert(key);
        }
    }

    // Todos come due: one reminder per todo and due date, made once even if it is dismissed or pdtd restarts.
    for (t, reminded) in st.store.dated_open_todos().await? {
        let Some(due_on) = t.due_on.as_deref() else { continue };
        let Some(at) = pdt_core::todo::due_at_local(Some(due_on), t.due_time.as_deref()) else { continue };
        if at > now {
            continue;
        }
        let key = todo_key(&t.id, due_on, t.due_time.as_deref());
        if reminded.as_deref() != Some(key.as_str()) {
            st.store.set_todo_reminded(&t.id, &key).await?;
            let mut n =
                NewNotification::new(K::TodoDue, &key, t.title.clone(), NotificationTarget::Todo { id: t.id.clone() });
            // Late (pdtd was down): say since when.
            let today = jiff::Zoned::now().date().to_string();
            let when = match t.due_time.as_deref() {
                _ if due_on < today.as_str() => format!("Overdue since {due_on}"),
                Some(time) => format!("Due at {time}"),
                None => "Due today".to_string(),
            };
            n.body = with_ticket(&when, t.ticket_key.as_deref());
            n.project_id = t.project_id.clone();
            n.ticket_id = t.ticket_id.clone();
            notify(st, n).await;
        }
        wanted.insert(key);
    }

    // What went away.
    for (key, n) in &live {
        let gone = match n.kind {
            K::SessionNeedsYou | K::SessionTurnEnded => !wanted.contains(key),
            k if k.inbox_owned() => !wanted.contains(key),
            K::SessionExited => match &n.session_id {
                Some(id) => st.store.get_session(id).await?.is_none_or(|s| s.status == SessionStatus::Running),
                None => true,
            },
            K::PrConflict | K::PrQueueLeft => pr_fixed(st, n),
            K::HostLoopStale | K::TodoDue => !wanted.contains(key),
            _ => false,
        };
        if gone {
            clear(st, key).await;
        }
    }
    Ok(())
}

/// Whether the PR of a conflict or queue-drop row is clean or back in the queue (or no longer open).
fn pr_fixed(st: &AppState, n: &Notification) -> bool {
    let Some(attempt) = n.dedupe_key.strip_prefix("attempt:").and_then(|r| r.rsplit_once(':')).map(|(a, _)| a) else {
        return false;
    };
    // Unknown after a restart until the PR check looks again.
    let Some(pr) = crate::github::watched_pr(st, attempt) else { return false };
    if pr.state != PrState::Open {
        return true;
    }
    match n.kind {
        K::PrConflict => pr.mergeable == Mergeable::Mergeable,
        _ => pr.in_merge_queue,
    }
}

fn loop_key(name: &str) -> String {
    format!("loop:{name}:stale")
}

fn with_ticket(body: &str, key: Option<&str>) -> String {
    match key {
        Some(k) if body.is_empty() => k.to_string(),
        Some(k) => format!("{k} · {body}"),
        None => body.to_string(),
    }
}

// ---- event causes ----

/// A session's agent exited on its own (not a Stop, close or restart by PDT) with a failure code.
pub(crate) async fn session_exited(st: &AppState, id: &str, code: i32) {
    if st.notifier.take_expected_exit(id) || st.shutting_down.load(Ordering::SeqCst) || code == 0 {
        return;
    }
    let Ok(Some(s)) = st.store.get_session(id).await else { return };
    if s.kind != SessionKind::AgentTui {
        return;
    }
    let mut n = NewNotification::new(
        K::SessionExited,
        session_key(id, K::SessionExited),
        s.title.clone(),
        NotificationTarget::Session { id: s.id.clone(), project_id: s.project_id.clone() },
    );
    n.severity = Severity::Warning;
    n.body = with_ticket(&format!("The agent exited (code {code})."), s.ticket.as_ref().map(|t| t.key.as_str()));
    n.project_id = Some(s.project_id.clone());
    n.ticket_id = s.ticket.as_ref().map(|t| t.id.clone());
    n.session_id = Some(s.id.clone());
    notify(st, n).await;
}

/// An event just logged (PR and CI news): its notification, or the one it clears.
pub async fn on_event(st: &AppState, e: &LogEvent) {
    let kind = match e.kind.as_str() {
        ev::CI_RUN_FAILED => K::PrCiFailed,
        ev::PR_CONFLICT => K::PrConflict,
        ev::PR_QUEUE_LEFT => K::PrQueueLeft,
        ev::PR_MERGED => K::PrMerged,
        ev::CI_RUN_PASSED => {
            if let Some(a) = &e.attempt_id {
                clear(st, &attempt_key(a, K::PrCiFailed)).await;
                crate::inbox::settle_notices(st, Some((a, &[NoticeKind::CiFailed]))).await;
            }
            return;
        }
        _ => return,
    };
    let p = &e.payload;
    let s = |k: &str| p.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let (attempt, ticket_key, ticket_id, project_id) = (s("attemptId"), s("ticketKey"), s("ticketId"), s("projectId"));
    if attempt.is_empty() {
        return;
    }
    let pr = p.get("pr").and_then(|v| v.as_u64()).map(|n| format!(" #{n}")).unwrap_or_default();
    let (title, body, severity) = match kind {
        K::PrCiFailed => {
            let jobs: Vec<&str> =
                p.get("jobs").and_then(|j| j.as_array()).into_iter().flatten().filter_map(|j| j.as_str()).collect();
            let body = if jobs.is_empty() { String::new() } else { format!("Failed: {}", jobs.join(", ")) };
            (format!("CI failed on {ticket_key}{pr}"), body, Severity::Error)
        }
        K::PrConflict => (format!("{ticket_key}{pr} conflicts with its base"), String::new(), Severity::Warning),
        K::PrQueueLeft => (format!("{ticket_key}{pr} left the merge queue"), String::new(), Severity::Warning),
        _ => {
            for k in [K::PrCiFailed, K::PrConflict, K::PrQueueLeft] {
                clear(st, &attempt_key(&attempt, k)).await;
            }
            let all = [NoticeKind::CiFailed, NoticeKind::Conflict, NoticeKind::QueueLeft, NoticeKind::Behind];
            crate::inbox::settle_notices(st, Some((&attempt, &all))).await;
            (format!("{ticket_key}{pr} merged"), String::new(), Severity::Info)
        }
    };
    let title = p.get("notice").and_then(|n| n.get("title")).and_then(|t| t.as_str()).map_or(title, String::from);
    let target = if ticket_id.is_empty() {
        NotificationTarget::None
    } else {
        NotificationTarget::Ticket { id: ticket_id.clone(), project_id: project_id.clone() }
    };
    let mut n = NewNotification::new(kind, attempt_key(&attempt, kind), title, target);
    n.body = body;
    n.severity = severity;
    n.project_id = (!project_id.is_empty()).then_some(project_id);
    n.ticket_id = (!ticket_id.is_empty()).then_some(ticket_id);
    notify(st, n).await;
}

/// A workflow run ended failed.
pub(crate) async fn run_failed(st: &AppState, run: &pdt_core::workflow::api::Run, error: Option<&str>) {
    let name = if run.workflow_name.is_empty() { "A workflow" } else { run.workflow_name.as_str() };
    let mut n = NewNotification::new(
        K::WorkflowFailed,
        format!("run:{}:failed", run.id),
        format!("{name} failed"),
        NotificationTarget::Workflow { id: run.workflow_id.clone() },
    );
    n.severity = Severity::Error;
    n.body = error.unwrap_or_default().to_string();
    n.project_id = run.project_id.clone();
    n.ticket_id = run.ticket_id.clone();
    n.run_id = Some(run.id.clone());
    notify(st, n).await;
}

// ---- routes ----

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ListQuery {
    section: Option<String>,
    project: Option<String>,
    #[serde(default)]
    unread: bool,
    limit: Option<i64>,
    before: Option<i64>,
}

/// `GET /api/notifications?section=needs_you|updates&project=&unread=&limit=&before=`.
pub(crate) async fn list(
    State(st): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<NotificationList>, ApiError> {
    let section = match q.section.as_deref() {
        None | Some("") => None,
        Some(s) => Some(Section::parse(s).ok_or_else(|| {
            ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", "section is needs_you or updates")
        })?),
    };
    let f = pdt_store::NotificationFilter {
        section,
        project_id: q.project.clone(),
        unread: q.unread,
        live: section == Some(Section::NeedsYou),
        before: q.before,
        limit: q.limit.unwrap_or(100),
    };
    let items = st.store.list_notifications(&f).await?;
    let unread_updates = st.store.unread_updates(q.project.as_deref()).await?;
    Ok(Json(NotificationList { items, unread_updates }))
}

/// `POST /api/notifications/{id}/read`.
pub(crate) async fn read(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !st.store.read_notification(&id, now_ms()).await? {
        return Err(ApiError::not_found("notification"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/notifications/read-all {section?, sessionId?, inboxItemId?}`.
pub(crate) async fn read_all(
    State(st): State<AppState>,
    payload: Result<Json<ReadAll>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(r) = payload?;
    st.store.read_notifications(r.section, r.session_id.as_deref(), r.inbox_item_id.as_deref(), now_ms()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/notifications/{id}/clear`: the user clears one Update from the Inbox (PDT-224). Needs-you rows are
/// cleared by their cause, never here.
pub(crate) async fn clear_one(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !st.store.dismiss_notification(&id, now_ms()).await? {
        return match st.store.get_notification(&id).await? {
            Some(_) => Err(ApiError::new(
                StatusCode::CONFLICT,
                "needs_you",
                "only Updates can be cleared; this one waits on you",
            )),
            None => Err(ApiError::not_found("notification")),
        };
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/notifications/clear-all`: every Update ("Clear all").
pub(crate) async fn clear_all(State(st): State<AppState>) -> Result<StatusCode, ApiError> {
    st.store.dismiss_updates(now_ms()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/notifications/settings`: every kind with its section, current Show, default and allowed levels.
pub(crate) async fn get_settings(State(st): State<AppState>) -> Json<Vec<KindSetting>> {
    Json(settings(&st).await.kinds())
}

/// `PATCH /api/notifications/settings {kind: show | null}` (`null` restores the default).
pub(crate) async fn patch_settings(
    State(st): State<AppState>,
    payload: Result<Json<HashMap<String, Option<Show>>>, JsonRejection>,
) -> Result<Json<Vec<KindSetting>>, ApiError> {
    let Json(changes) = payload?;
    let _lock = st.settings_lock.lock().await;
    let mut s = settings(&st).await;
    for (kind, show) in changes {
        let k = pdt_core::notification::NotificationKind::parse(&kind).ok_or_else(|| {
            ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", format!("{kind} is not a kind of notification"))
        })?;
        s.set(k, show).map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", m))?;
    }
    st.store.put_host_value(SETTINGS_KEY, &s.to_json()).await?;
    *st.notifier.settings.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    // Rows of kinds turned down below `inbox` go; kinds turned up get theirs.
    kick(&st);
    Ok(Json(s.kinds()))
}

fn event(change: Change, n: &Notification) -> Event {
    Event::default()
        .id(n.seq.to_string())
        .event(change.as_str())
        .json_data(n)
        .unwrap_or_else(|_| Event::default().comment("unserializable"))
}

struct StreamState {
    st: AppState,
    rx: broadcast::Receiver<(Change, Notification)>,
    /// Bumped when shutdown starts: the stream ends so connections can drain.
    wake: tokio::sync::watch::Receiver<u64>,
    backlog: VecDeque<Notification>,
    last: i64,
}

/// `GET /api/notifications/stream`: Server-Sent Events of `created`, `updated` and `cleared` rows (event id = `seq`).
/// A reconnect's `Last-Event-ID` gets what it missed (as `created` or `cleared`).
pub(crate) async fn stream(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let rx = st.notifier.tx.subscribe();
    let from = headers.get("last-event-id").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<i64>().ok());
    let (last, backlog) = match from {
        Some(seq) => (seq, st.store.notifications_after(seq, 500).await?.into()),
        None => (st.store.last_notification_seq().await?, VecDeque::new()),
    };
    let wake = st.resume_changed.subscribe();
    let init = StreamState { st, rx, wake, backlog, last };
    let s = futures_util::stream::unfold(init, |mut s| async move {
        loop {
            if let Some(n) = s.backlog.pop_front() {
                if n.seq <= s.last {
                    continue;
                }
                s.last = n.seq;
                return Some((Ok(event(Change::of(&n), &n)), s));
            }
            if s.st.shutting_down.load(Ordering::SeqCst) {
                return None;
            }
            let got = tokio::select! {
                r = s.rx.recv() => r,
                w = s.wake.changed() => {
                    if w.is_err() {
                        return None;
                    }
                    continue;
                }
            };
            match got {
                Ok((change, n)) if n.seq > s.last => {
                    s.last = n.seq;
                    return Some((Ok(event(change, &n)), s));
                }
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    s.backlog = s.st.store.notifications_after(s.last, 500).await.unwrap_or_default().into();
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(s).keep_alive(KeepAlive::default()))
}
