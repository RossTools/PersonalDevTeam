//! Notifications (PDT-108): what PDT tells the user, made once by pdtd and shown by every client. Each kind has a
//! Show level (Off, Dot, Inbox, Toast, Desktop) the user tunes in Settings → Notifications. Pure types and rules;
//! pdt-store keeps the rows, pdt-api's Notifier makes and clears them.

use crate::activity::{Activity, is_unseen};
use crate::inbox::{InboxGroup, InboxKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// How far a kind of notification reaches the user. Each level includes the ones below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Show {
    /// Nothing.
    Off,
    /// Only the marker on the thing itself (a session's dot, a ticket's card). No Inbox entry, no row.
    Dot,
    /// Also an entry in the Inbox.
    Inbox,
    /// Also a toast while PDT is visible.
    Toast,
    /// Also a desktop notification while PDT's page is hidden, on devices that allowed it.
    Desktop,
}

impl Show {
    pub const ALL: [Show; 5] = [Show::Off, Show::Dot, Show::Inbox, Show::Toast, Show::Desktop];
    pub fn as_str(self) -> &'static str {
        match self {
            Show::Off => "off",
            Show::Dot => "dot",
            Show::Inbox => "inbox",
            Show::Toast => "toast",
            Show::Desktop => "desktop",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
    /// Whether a notification at this level is stored (Off makes nothing; Dot is the target's own state).
    pub fn stored(self) -> bool {
        self >= Show::Inbox
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, rename = "NotificationSection")]
pub enum Section {
    /// Waits on the user.
    NeedsYou,
    /// Worth knowing; waits on nothing.
    Updates,
}

impl Section {
    pub fn as_str(self) -> &'static str {
        match self {
            Section::NeedsYou => "needs_you",
            Section::Updates => "updates",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Section::NeedsYou, Section::Updates].into_iter().find(|v| v.as_str() == s)
    }
}

/// Every kind of notification (PDT-108 §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum NotificationKind {
    #[serde(rename = "session.needs_you")]
    SessionNeedsYou,
    #[serde(rename = "inbox.question")]
    InboxQuestion,
    #[serde(rename = "inbox.approval")]
    InboxApproval,
    #[serde(rename = "inbox.gate")]
    InboxGate,
    #[serde(rename = "inbox.manual")]
    InboxManual,
    #[serde(rename = "inbox.stalled")]
    InboxStalled,
    #[serde(rename = "inbox.loop_limit")]
    InboxLoopLimit,
    #[serde(rename = "inbox.notice")]
    InboxNotice,
    #[serde(rename = "session.turn_ended")]
    SessionTurnEnded,
    #[serde(rename = "session.exited")]
    SessionExited,
    /// A message PDT or the user sent an agent never reached it (PDT-129).
    #[serde(rename = "session.undelivered")]
    SessionUndelivered,
    #[serde(rename = "pr.ci_failed")]
    PrCiFailed,
    #[serde(rename = "pr.conflict")]
    PrConflict,
    #[serde(rename = "pr.queue_left")]
    PrQueueLeft,
    #[serde(rename = "pr.merged")]
    PrMerged,
    #[serde(rename = "workflow.failed")]
    WorkflowFailed,
    #[serde(rename = "workflow.notify")]
    WorkflowNotify,
    #[serde(rename = "connection.failed")]
    ConnectionFailed,
    #[serde(rename = "host.loop_stale")]
    HostLoopStale,
    #[serde(rename = "todo.due")]
    TodoDue,
    /// Something happened to a ticket or run the user watches (PDT-319).
    #[serde(rename = "watch.event")]
    WatchEvent,
}

use NotificationKind as K;

impl NotificationKind {
    /// In Settings order: Needs you, then Updates.
    pub const ALL: [NotificationKind; 21] = [
        K::SessionNeedsYou,
        K::InboxQuestion,
        K::InboxApproval,
        K::InboxGate,
        K::InboxManual,
        K::InboxStalled,
        K::InboxLoopLimit,
        K::InboxNotice,
        K::SessionTurnEnded,
        K::SessionExited,
        K::SessionUndelivered,
        K::PrCiFailed,
        K::PrConflict,
        K::PrQueueLeft,
        K::PrMerged,
        K::WorkflowFailed,
        K::WorkflowNotify,
        K::ConnectionFailed,
        K::HostLoopStale,
        K::TodoDue,
        K::WatchEvent,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            K::SessionNeedsYou => "session.needs_you",
            K::InboxQuestion => "inbox.question",
            K::InboxApproval => "inbox.approval",
            K::InboxGate => "inbox.gate",
            K::InboxManual => "inbox.manual",
            K::InboxStalled => "inbox.stalled",
            K::InboxLoopLimit => "inbox.loop_limit",
            K::InboxNotice => "inbox.notice",
            K::SessionTurnEnded => "session.turn_ended",
            K::SessionExited => "session.exited",
            K::SessionUndelivered => "session.undelivered",
            K::PrCiFailed => "pr.ci_failed",
            K::PrConflict => "pr.conflict",
            K::PrQueueLeft => "pr.queue_left",
            K::PrMerged => "pr.merged",
            K::WorkflowFailed => "workflow.failed",
            K::WorkflowNotify => "workflow.notify",
            K::ConnectionFailed => "connection.failed",
            K::HostLoopStale => "host.loop_stale",
            K::TodoDue => "todo.due",
            K::WatchEvent => "watch.event",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub fn needs_you(self) -> bool {
        matches!(
            self,
            K::SessionNeedsYou
                | K::InboxQuestion
                | K::InboxApproval
                | K::InboxGate
                | K::InboxManual
                | K::InboxStalled
                | K::InboxLoopLimit
                | K::InboxNotice
        )
    }

    pub fn section(self) -> Section {
        if self.needs_you() { Section::NeedsYou } else { Section::Updates }
    }

    /// Kinds the Inbox owns: a step waits on them, so they never go below `inbox`.
    pub fn inbox_owned(self) -> bool {
        self.needs_you() && self != K::SessionNeedsYou
    }

    pub fn default_show(self) -> Show {
        match self {
            K::SessionNeedsYou | K::InboxQuestion | K::InboxApproval => Show::Desktop,
            K::InboxGate
            | K::InboxManual
            | K::InboxStalled
            | K::InboxLoopLimit
            | K::InboxNotice
            | K::WorkflowNotify
            | K::SessionUndelivered
            | K::ConnectionFailed
            | K::HostLoopStale
            | K::TodoDue => Show::Toast,
            K::WatchEvent => Show::Desktop,
            K::SessionTurnEnded => Show::Dot,
            K::SessionExited | K::PrCiFailed | K::PrConflict | K::PrQueueLeft | K::PrMerged | K::WorkflowFailed => {
                Show::Inbox
            }
        }
    }

    /// The lowest level the user may pick.
    pub fn min_show(self) -> Show {
        if self.inbox_owned() { Show::Inbox } else { Show::Off }
    }

    /// The levels Settings offers.
    pub fn allowed(self) -> Vec<Show> {
        Show::ALL.into_iter().filter(|s| *s >= self.min_show()).collect()
    }

    pub fn label(self) -> &'static str {
        match self {
            K::SessionNeedsYou => "Session waiting",
            K::InboxQuestion => "Question",
            K::InboxApproval => "Approval",
            K::InboxGate => "Gate",
            K::InboxManual => "Your turn",
            K::InboxStalled => "Stalled",
            K::InboxLoopLimit => "Loop limit",
            K::InboxNotice => "Unheard notice",
            K::SessionTurnEnded => "Turn ended",
            K::SessionExited => "Session exited",
            K::SessionUndelivered => "Message not delivered",
            K::PrCiFailed => "CI failed",
            K::PrConflict => "Conflict",
            K::PrQueueLeft => "Left the merge queue",
            K::PrMerged => "Merged",
            K::WorkflowFailed => "Workflow failed",
            K::WorkflowNotify => "Workflow notification",
            K::ConnectionFailed => "Connection failing",
            K::HostLoopStale => "Background task stalled",
            K::TodoDue => "Todo due",
            K::WatchEvent => "Watched",
        }
    }

    /// When it happens, in a line.
    pub fn when(self) -> &'static str {
        match self {
            K::SessionNeedsYou => "A session waits on a permission prompt or a question.",
            K::InboxQuestion => "An agent asks you something (ask_user).",
            K::InboxApproval => "A conversation asks to use a tool.",
            K::InboxGate => "A workflow waits at a gate, or work is ready to accept.",
            K::InboxManual => "A manual step waits for you.",
            K::InboxStalled => "An agent step ended its turns, or timed out, without an outcome.",
            K::InboxLoopLimit => "An agent step reached its loop limit.",
            K::InboxNotice => "A pull request conflicts or fails CI and no agent session could be told.",
            K::SessionTurnEnded => "A session finished its turn and you haven't looked yet.",
            K::SessionExited => "A session's agent exited on its own (not your Stop).",
            K::SessionUndelivered => {
                "A message for an agent could not be delivered (it stopped, PDT restarted, or it \
                                      never became ready)."
            }
            K::PrCiFailed => "CI failed on an attempt's pull request.",
            K::PrConflict => "An attempt's pull request conflicts with its base.",
            K::PrQueueLeft => "The merge queue dropped an attempt's pull request.",
            K::PrMerged => "An attempt's pull request merged.",
            K::WorkflowFailed => "A workflow run failed.",
            K::WorkflowNotify => "A workflow's Notify me step.",
            K::ConnectionFailed => "A sensor could not use a connection (a revoked token, the service down).",
            K::HostLoopStale => "One of PDT's background tasks (PR checks, the workflow engine…) stopped running.",
            K::TodoDue => "One of your todos comes due (its day starts, or its time comes).",
            K::WatchEvent => {
                "A ticket or workflow run you watch finished, failed, got an outcome or a comment, or waits on you."
            }
        }
    }

    /// The kind an Inbox item notifies as.
    pub fn of_inbox(kind: InboxKind, group: InboxGroup) -> Self {
        match (kind, group) {
            (InboxKind::Question | InboxKind::Secret, _) => K::InboxQuestion,
            (InboxKind::Approval | InboxKind::Confirm, _) => K::InboxApproval,
            (InboxKind::Stalled | InboxKind::TimedOut, _) => K::InboxStalled,
            (InboxKind::LoopLimit, _) => K::InboxLoopLimit,
            (InboxKind::Notice | InboxKind::Connect | InboxKind::Storage, _) => K::InboxNotice,
            (InboxKind::Manual, _) => K::InboxManual,
            (InboxKind::Gate | InboxKind::Session | InboxKind::Admit | InboxKind::Proposal, _) => K::InboxGate,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, rename = "NotificationSeverity")]
pub enum Severity {
    #[default]
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Severity::Info, Severity::Warning, Severity::Error].into_iter().find(|v| v.as_str() == s)
    }
}

/// What clicking a notification opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum NotificationTarget {
    Session {
        id: String,
        project_id: String,
    },
    Inbox {
        id: String,
    },
    Ticket {
        id: String,
        project_id: String,
    },
    /// A ticket by its key (a `notify` step's link).
    TicketKey {
        key: String,
    },
    Workflow {
        id: String,
    },
    Url {
        url: String,
    },
    /// A todo in the right panel's list.
    Todo {
        id: String,
    },
    /// Settings → Plugins.
    Connections,
    None,
}

/// A notification as stored and shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Notification {
    pub id: String,
    /// Grows with every change (created, updated, cleared): the stream's cursor.
    #[ts(type = "number")]
    pub seq: i64,
    pub kind: NotificationKind,
    pub needs_you: bool,
    pub severity: Severity,
    /// The kind's level when it was made (or last updated).
    pub show: Show,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub inbox_item_id: Option<String>,
    pub title: String,
    pub body: String,
    pub target: NotificationTarget,
    /// One live row per key: a cause that fires again updates its row.
    pub dedupe_key: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub read_at: Option<i64>,
    #[ts(type = "number | null")]
    pub cleared_at: Option<i64>,
    /// The user cleared it from Updates (PDT-224): hidden, and while its cause lasts (the row stays live) the same
    /// cause updates it out of sight instead of raising it again.
    #[ts(type = "number | null")]
    pub dismissed_at: Option<i64>,
}

/// A notification to make (the Notifier fills in its Show and needs-you).
#[derive(Debug, Clone, PartialEq)]
pub struct NewNotification {
    pub kind: NotificationKind,
    pub severity: Severity,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub inbox_item_id: Option<String>,
    pub title: String,
    pub body: String,
    pub target: NotificationTarget,
    pub dedupe_key: String,
}

impl NewNotification {
    pub fn new(
        kind: NotificationKind,
        dedupe_key: impl Into<String>,
        title: impl Into<String>,
        target: NotificationTarget,
    ) -> Self {
        Self {
            kind,
            severity: Severity::Info,
            project_id: None,
            ticket_id: None,
            session_id: None,
            run_id: None,
            inbox_item_id: None,
            title: title.into(),
            body: String::new(),
            target,
            dedupe_key: dedupe_key.into(),
        }
    }
}

/// The dedupe keys of the kinds made from live state.
pub fn session_key(session_id: &str, kind: NotificationKind) -> String {
    match kind {
        K::SessionTurnEnded => format!("session:{session_id}:turn_ended"),
        K::SessionExited => format!("session:{session_id}:exited"),
        _ => format!("session:{session_id}:needs_you"),
    }
}

/// The one live row per failing connection (by name: a missing connection has no id).
pub fn connection_key(name: &str) -> String {
    format!("connection:{name}")
}

pub fn inbox_key(item_id: &str) -> String {
    format!("inbox:{item_id}")
}

pub fn attempt_key(attempt_id: &str, kind: NotificationKind) -> String {
    let what = match kind {
        K::PrCiFailed => "ci_failed",
        K::PrConflict => "conflict",
        K::PrQueueLeft => "queue_left",
        _ => "merged",
    };
    format!("attempt:{attempt_id}:{what}")
}

/// The one reminder per todo and due date (and time): re-dating it makes a new cause.
pub fn todo_key(todo_id: &str, due_on: &str, due_time: Option<&str>) -> String {
    match due_time {
        Some(t) => format!("todo:{todo_id}:{due_on}T{t}"),
        None => format!("todo:{todo_id}:{due_on}"),
    }
}

/// A session left in a turn-ended state this long is worth a notification (when the kind is above `dot`).
pub const TURN_ENDED_AFTER_MS: i64 = 30_000;

/// The session kind a session's state calls for now, if any: `needs_you` while it waits on the user and they haven't
/// looked since; `turn_ended` once it has been idle and unseen for [`TURN_ENDED_AFTER_MS`]. Viewing it (on any
/// device) makes it seen, so neither comes back until its next change.
pub fn session_wants(
    activity: Option<Activity>,
    attention_at: Option<i64>,
    last_viewed_at: Option<i64>,
    now: i64,
) -> Option<NotificationKind> {
    if !is_unseen(attention_at, last_viewed_at) {
        return None;
    }
    match activity? {
        Activity::NeedsYou => Some(K::SessionNeedsYou),
        Activity::Idle if attention_at.is_some_and(|a| now - a >= TURN_ENDED_AFTER_MS) => Some(K::SessionTurnEnded),
        _ => None,
    }
}

/// The host's settings: the overrides of each kind's default Show (host setting `notifications.show`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NotificationSettings {
    pub overrides: BTreeMap<NotificationKind, Show>,
}

impl NotificationSettings {
    /// A kind's effective Show: its override (never below its minimum), else its default.
    pub fn show(&self, kind: NotificationKind) -> Show {
        self.overrides.get(&kind).copied().map_or(kind.default_show(), |s| s.max(kind.min_show()))
    }

    /// Sets (or with None, resets) a kind's Show. An override equal to the default is not kept.
    pub fn set(&mut self, kind: NotificationKind, show: Option<Show>) -> Result<(), String> {
        match show {
            Some(s) if s < kind.min_show() => {
                Err(format!("{} can't go below {}: a step waits on it", kind.as_str(), kind.min_show().as_str()))
            }
            Some(s) if s != kind.default_show() => {
                self.overrides.insert(kind, s);
                Ok(())
            }
            _ => {
                self.overrides.remove(&kind);
                Ok(())
            }
        }
    }

    /// Reads the stored map, skipping kinds this build doesn't know.
    pub fn from_json(v: &serde_json::Value) -> Self {
        let mut s = Self::default();
        if let Some(m) = v.as_object() {
            for (k, v) in m {
                if let (Some(kind), Some(show)) = (NotificationKind::parse(k), v.as_str().and_then(Show::parse)) {
                    let _ = s.set(kind, Some(show));
                }
            }
        }
        s
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.overrides.iter().map(|(k, v)| (k.as_str().to_string(), v.as_str().into())).collect(),
        )
    }

    /// Every kind as Settings lists it.
    pub fn kinds(&self) -> Vec<KindSetting> {
        NotificationKind::ALL
            .into_iter()
            .map(|k| KindSetting {
                kind: k,
                section: k.section(),
                label: k.label().into(),
                when: k.when().into(),
                show: self.show(k),
                default_show: k.default_show(),
                allowed: k.allowed(),
            })
            .collect()
    }
}

/// One kind in `GET /api/notifications/settings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KindSetting {
    pub kind: NotificationKind,
    pub section: Section,
    pub label: String,
    pub when: String,
    pub show: Show,
    pub default_show: Show,
    pub allowed: Vec<Show>,
}

/// `GET /api/notifications`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NotificationList {
    pub items: Vec<Notification>,
    /// Updates not read yet (not cleared, nor dismissed by the user): the dot on the Inbox.
    pub unread_updates: u32,
}

/// `POST /api/notifications/read-all`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReadAll {
    #[serde(default)]
    #[ts(optional)]
    pub section: Option<Section>,
    /// Only one session's rows (viewing it).
    #[serde(default)]
    #[ts(optional)]
    pub session_id: Option<String>,
    /// Only one Inbox item's rows (opening it).
    #[serde(default)]
    #[ts(optional)]
    pub inbox_item_id: Option<String>,
}

/// What the stream says about a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, rename = "NotificationChange")]
pub enum Change {
    Created,
    Updated,
    Cleared,
}

impl Change {
    pub fn as_str(self) -> &'static str {
        match self {
            Change::Created => "created",
            Change::Updated => "updated",
            Change::Cleared => "cleared",
        }
    }
    /// A row read back after a missed stretch (a reconnect): cleared, else created.
    pub fn of(n: &Notification) -> Self {
        if n.cleared_at.is_some() { Change::Cleared } else { Change::Created }
    }
}

/// How long notifications are kept.
pub const RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_serialize_as_their_names() {
        for k in NotificationKind::ALL {
            assert_eq!(NotificationKind::parse(k.as_str()), Some(k));
            assert_eq!(serde_json::to_value(k).unwrap(), k.as_str());
        }
        assert_eq!(Show::parse("toast"), Some(Show::Toast));
        assert!(Show::Desktop > Show::Toast && Show::Toast > Show::Inbox && Show::Dot > Show::Off);
    }

    #[test]
    fn defaults_follow_the_spec() {
        assert_eq!(K::SessionNeedsYou.default_show(), Show::Desktop);
        assert_eq!(K::InboxQuestion.default_show(), Show::Desktop);
        assert_eq!(K::InboxGate.default_show(), Show::Toast);
        assert_eq!(K::SessionTurnEnded.default_show(), Show::Dot, "today's unseen dot");
        assert_eq!(K::PrMerged.default_show(), Show::Inbox);
        assert_eq!(K::WorkflowNotify.default_show(), Show::Toast);
        assert!(K::SessionNeedsYou.needs_you() && !K::PrCiFailed.needs_you());
        assert_eq!(K::SessionExited.section(), Section::Updates);
    }

    #[test]
    fn inbox_kinds_never_go_below_inbox() {
        assert_eq!(K::InboxGate.allowed(), vec![Show::Inbox, Show::Toast, Show::Desktop]);
        assert_eq!(K::SessionNeedsYou.allowed().len(), 5, "a session's prompt is the session's to show");
        let mut s = NotificationSettings::default();
        assert!(s.set(K::InboxQuestion, Some(Show::Dot)).is_err());
        s.set(K::InboxQuestion, Some(Show::Inbox)).unwrap();
        s.set(K::SessionNeedsYou, Some(Show::Off)).unwrap();
        assert_eq!((s.show(K::InboxQuestion), s.show(K::SessionNeedsYou)), (Show::Inbox, Show::Off));
        assert_eq!(s.show(K::PrMerged), Show::Inbox, "the default");
    }

    #[test]
    fn settings_keep_only_overrides_and_read_back() {
        let mut s = NotificationSettings::default();
        s.set(K::PrMerged, Some(Show::Toast)).unwrap();
        s.set(K::InboxGate, Some(Show::Toast)).unwrap();
        assert_eq!(s.overrides.len(), 1, "the default is not stored");
        let json = s.to_json();
        assert_eq!(json, serde_json::json!({"pr.merged": "toast"}));
        let mut back = NotificationSettings::from_json(&serde_json::json!({
            "pr.merged": "toast", "nope.kind": "off", "inbox.gate": "off", "pr.conflict": "loud"
        }));
        assert_eq!(back.show(K::PrMerged), Show::Toast);
        assert_eq!(back.show(K::InboxGate), Show::Toast, "below the minimum is ignored");
        assert_eq!(back.show(K::PrConflict), Show::Inbox);
        back.set(K::PrMerged, None).unwrap();
        assert!(back.overrides.is_empty(), "null restores the default");
        let kinds = back.kinds();
        assert_eq!(kinds.len(), NotificationKind::ALL.len());
        assert_eq!(kinds[0].section, Section::NeedsYou);
    }

    #[test]
    fn a_session_notifies_while_it_waits_unseen() {
        let now = 100_000;
        assert_eq!(session_wants(Some(Activity::NeedsYou), Some(10), None, now), Some(K::SessionNeedsYou));
        assert_eq!(session_wants(Some(Activity::NeedsYou), Some(10), Some(10), now), None, "viewed");
        assert_eq!(session_wants(Some(Activity::NeedsYou), Some(20), Some(10), now), Some(K::SessionNeedsYou));
        assert_eq!(session_wants(Some(Activity::Working), Some(10), None, now), None);
        assert_eq!(session_wants(None, Some(10), None, now), None, "ended");
    }

    #[test]
    fn a_turn_ends_after_thirty_seconds_unseen() {
        let at = 1_000;
        assert_eq!(session_wants(Some(Activity::Idle), Some(at), None, at + TURN_ENDED_AFTER_MS - 1), None);
        assert_eq!(
            session_wants(Some(Activity::Idle), Some(at), None, at + TURN_ENDED_AFTER_MS),
            Some(K::SessionTurnEnded)
        );
        assert_eq!(session_wants(Some(Activity::Idle), Some(at), Some(at + 5), at + 60_000), None);
    }

    #[test]
    fn inbox_items_map_to_their_kinds() {
        assert_eq!(NotificationKind::of_inbox(InboxKind::Gate, InboxGroup::Accept), K::InboxGate);
        assert_eq!(NotificationKind::of_inbox(InboxKind::Manual, InboxGroup::Gate), K::InboxManual);
        assert_eq!(NotificationKind::of_inbox(InboxKind::Approval, InboxGroup::Approval), K::InboxApproval);
        assert_eq!(inbox_key("i1"), "inbox:i1");
        assert_eq!(session_key("s", K::SessionNeedsYou), "session:s:needs_you");
        assert_eq!(attempt_key("a", K::PrCiFailed), "attempt:a:ci_failed");
        assert_eq!(todo_key("t", "2026-10-23", None), "todo:t:2026-10-23");
        assert_eq!(todo_key("t", "2026-10-23", Some("09:00")), "todo:t:2026-10-23T09:00");
    }

    #[test]
    fn targets_serialize_tagged() {
        let t = NotificationTarget::Session { id: "s".into(), project_id: "p".into() };
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            serde_json::json!({"kind": "session", "id": "s", "projectId": "p"})
        );
        let back: NotificationTarget = serde_json::from_value(serde_json::json!({"kind": "none"})).unwrap();
        assert_eq!(back, NotificationTarget::None);
    }
}
