//! Project policies (Phase 7c spec §4, §6): who may merge, whether an automated merge needs a person's approval, the
//! cap on running attempts and auto-start. Pure rules: the domain commands (Merge, Start) and the scheduler apply them.

use crate::ticket::{OverlapLevel, TicketSort, TicketSummary};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Who may merge a project's attempts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MergeBy {
    /// Agents, workflows and people.
    #[default]
    Agents,
    /// People only (PDT's Merge…).
    User,
    /// Nobody in PDT: merges happen elsewhere (external reviewers on GitHub) and PDT only notices them.
    External,
}

impl MergeBy {
    pub const ALL: [MergeBy; 3] = [MergeBy::Agents, MergeBy::User, MergeBy::External];
    pub fn as_str(self) -> &'static str {
        match self {
            MergeBy::Agents => "agents",
            MergeBy::User => "user",
            MergeBy::External => "external",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// Which tickets the scheduler starts on its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AutoStart {
    #[default]
    Off,
    /// To do tickets: the ones a person picked for the board.
    Todo,
    /// To do tickets first, then Ready ones.
    Ready,
}

impl AutoStart {
    pub const ALL: [AutoStart; 3] = [AutoStart::Off, AutoStart::Todo, AutoStart::Ready];
    pub fn as_str(self) -> &'static str {
        match self {
            AutoStart::Off => "off",
            AutoStart::Todo => "todo",
            AutoStart::Ready => "ready",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// The order auto-start takes a project's candidates in (PDT-218); To do always comes before Ready.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AutoStartOrder {
    /// Most urgent first (urgent, high, medium, low, none), then backlog rank.
    #[default]
    Priority,
    /// Backlog rank only.
    Rank,
}

impl AutoStartOrder {
    pub const ALL: [AutoStartOrder; 2] = [AutoStartOrder::Priority, AutoStartOrder::Rank];
    pub fn as_str(self) -> &'static str {
        match self {
            AutoStartOrder::Priority => "priority",
            AutoStartOrder::Rank => "rank",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
    /// The ticket list sort that gives this order.
    pub fn ticket_sort(self) -> TicketSort {
        match self {
            AutoStartOrder::Priority => TicketSort::Priority,
            AutoStartOrder::Rank => TicketSort::Rank,
        }
    }
}

/// What happens to a ticket triage drafts (PDT-225).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TriageApproval {
    /// It waits in the Inbox for the user's Approve (to To do) or Reject (to Won't do).
    #[default]
    Inbox,
    /// It stays where triage left it (Draft, or Ready once refined) until the user moves it.
    Off,
}

impl TriageApproval {
    pub const ALL: [TriageApproval; 2] = [TriageApproval::Inbox, TriageApproval::Off];
    pub fn as_str(self) -> &'static str {
        match self {
            TriageApproval::Inbox => "inbox",
            TriageApproval::Off => "off",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// When an attempt's conflicts with its base get resolved (PDT-328).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ResolveConflicts {
    /// Once, when the ticket is next to merge: a conflict while it is in code, review, QA or accept is recorded and
    /// shown, not sent to its agent; the merge step brings the branch up to date first.
    #[default]
    MergeTime,
    /// As soon as the pull request conflicts: its agent is told at once.
    Immediately,
}

impl ResolveConflicts {
    pub const ALL: [ResolveConflicts; 2] = [ResolveConflicts::MergeTime, ResolveConflicts::Immediately];
    pub fn as_str(self) -> &'static str {
        match self {
            ResolveConflicts::MergeTime => "merge_time",
            ResolveConflicts::Immediately => "immediately",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// How long a ticket goes unchanged before auto-start takes it, unless the project says otherwise (PDT-146).
pub const DEFAULT_START_DELAY_SECONDS: u32 = 120;

fn default_start_delay() -> u32 {
    DEFAULT_START_DELAY_SECONDS
}

/// A project's policies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectPolicy {
    pub merge_by: MergeBy,
    /// An automated merge needs a person's approval at the branch's current head.
    pub require_approval: bool,
    /// Tickets with any of these labels merge automatically without that approval (`require_approval` stays on for
    /// the rest).
    #[serde(default)]
    pub approval_exempt_labels: Vec<String>,
    /// The most active attempts the project may have (null: no cap; auto-start then keeps to one).
    pub max_running_attempts: Option<u32>,
    pub auto_start: AutoStart,
    /// Which candidate auto-start takes first (PDT-218).
    #[serde(default)]
    pub auto_start_order: AutoStartOrder,
    /// The PM may create and edit the project's tickets and message its sessions without asking (PDT-81).
    #[serde(default)]
    pub pm_acts: bool,
    /// Auto-start takes a ticket only once it has gone this long unchanged (PDT-146); 0: at once.
    #[serde(default = "default_start_delay")]
    pub start_delay_seconds: u32,
    /// An automated merge of a pull request waits for its head's CI to pass, and is refused while it fails (PDT-151),
    /// whatever GitHub's branch protection requires.
    #[serde(default = "yes")]
    pub require_green_ci: bool,
    /// Whether tickets triage drafts wait for the user's approval in the Inbox (PDT-225).
    #[serde(default)]
    pub triage_approval: TriageApproval,
    /// When conflicts with the base are resolved: at merge time, or as soon as they appear (PDT-328).
    #[serde(default)]
    pub resolve_conflicts: ResolveConflicts,
    /// Areas two tickets in flight at once often collide in (PDT-329): "Pick up tickets" starts no ticket in one while
    /// another ticket in it is in flight.
    #[serde(default)]
    pub hot_areas: Vec<String>,
    /// A ticket without areas may start while a ticket in a hot area is in flight, and a ticket in a hot area while one
    /// without areas is.
    #[serde(default = "yes")]
    pub untagged_alongside_hot: bool,
}

fn yes() -> bool {
    true
}

impl Default for ProjectPolicy {
    fn default() -> Self {
        ProjectPolicy {
            merge_by: MergeBy::default(),
            require_approval: false,
            approval_exempt_labels: vec![],
            max_running_attempts: None,
            auto_start: AutoStart::default(),
            auto_start_order: AutoStartOrder::default(),
            pm_acts: false,
            start_delay_seconds: DEFAULT_START_DELAY_SECONDS,
            require_green_ci: true,
            triage_approval: TriageApproval::default(),
            resolve_conflicts: ResolveConflicts::default(),
            hot_areas: vec![],
            untagged_alongside_hot: true,
        }
    }
}

impl ProjectPolicy {
    /// How many attempts auto-start fills up to.
    pub fn auto_start_cap(&self) -> u32 {
        self.max_running_attempts.unwrap_or(1)
    }

    /// Whether an automated merge of a ticket with `labels` needs a person's approval.
    pub fn needs_approval(&self, labels: &[String]) -> bool {
        self.require_approval && !labels.iter().any(|l| self.approval_exempt_labels.contains(l))
    }

    /// Whether Start would go over the cap with `active` attempts already running.
    pub fn over_cap(&self, active: u32) -> bool {
        self.max_running_attempts.is_some_and(|cap| active >= cap)
    }
}

/// Who asks for a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merger {
    /// A person, through PDT's Merge… (their merge is their own approval).
    User,
    /// A workflow, or an agent (pdt-mcp, or the API without `by: "user"`).
    Automated,
}

/// Why a policy refuses a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeRefusal {
    /// `merge_not_allowed`.
    NotAllowed,
    /// `approval_required`.
    ApprovalRequired,
}

impl MergeRefusal {
    pub fn code(self) -> &'static str {
        match self {
            MergeRefusal::NotAllowed => "merge_not_allowed",
            MergeRefusal::ApprovalRequired => "approval_required",
        }
    }
    pub fn message(self, policy: &ProjectPolicy) -> &'static str {
        match (self, policy.merge_by) {
            (MergeRefusal::ApprovalRequired, _) => {
                "this project needs a person's approval of the branch's current head before an automated merge"
            }
            (_, MergeBy::External) => "this project's merges happen outside PDT",
            _ => "in this project only a person merges",
        }
    }
}

/// Whether `merger` may merge a ticket with `labels` under `policy`; `approved`: a person approved the branch's
/// current head.
pub fn may_merge(
    policy: &ProjectPolicy,
    merger: Merger,
    approved: bool,
    labels: &[String],
) -> Result<(), MergeRefusal> {
    match (policy.merge_by, merger) {
        (MergeBy::External, _) | (MergeBy::User, Merger::Automated) => Err(MergeRefusal::NotAllowed),
        (_, Merger::User) => Ok(()),
        (MergeBy::Agents, Merger::Automated) if policy.needs_approval(labels) && !approved => {
            Err(MergeRefusal::ApprovalRequired)
        }
        (MergeBy::Agents, Merger::Automated) => Ok(()),
    }
}

/// Whether auto-start may take this ticket (spec §6.1): not a parent, no active attempt (an abandoned or merged one
/// doesn't count: a ticket stopped and moved back to To do starts again, PDT-159), not blocked, no conflict, and not
/// waiting unless it is clear to start.
pub fn startable(t: &TicketSummary) -> bool {
    t.parts.is_none()
        && t.attempt.as_ref().is_none_or(|a| a.state != crate::ticket::AttemptState::Active)
        && t.blocked_by == 0
        && t.overlap != OverlapLevel::Conflict
        && (!t.waiting || t.clear_to_start)
}

/// When `t` has been quiet for the policy's start delay (PDT-146), in ms: the delay after it was created or last
/// changed (an edit, a move, a link on either side, a comment: anything that bumps `updated_at`).
pub fn settles_at(policy: &ProjectPolicy, t: &TicketSummary) -> i64 {
    t.created_at.max(t.updated_at).saturating_add(i64::from(policy.start_delay_seconds) * 1000)
}

/// Whether auto-start takes tickets in `t`'s stage under `policy`.
pub fn takes_stage(policy: &ProjectPolicy, t: &TicketSummary) -> bool {
    use crate::ticket::TicketStage;
    match policy.auto_start {
        AutoStart::Off => false,
        AutoStart::Todo => t.stage == TicketStage::Todo,
        AutoStart::Ready => matches!(t.stage, TicketStage::Todo | TicketStage::Ready),
    }
}

/// When auto-start may take `t`, while it is a candidate still waiting out the start delay at `now` (the Board's
/// "Auto-starts in 1:30"); None once the delay is over, or when auto-start would not take it anyway.
pub fn auto_start_at(policy: &ProjectPolicy, t: &TicketSummary, now: i64) -> Option<i64> {
    let at = settles_at(policy, t);
    (takes_stage(policy, t) && startable(t) && at > now).then_some(at)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ticket::{Priority, TicketStage};

    fn policy(merge_by: MergeBy, require_approval: bool) -> ProjectPolicy {
        ProjectPolicy { merge_by, require_approval, ..ProjectPolicy::default() }
    }

    #[test]
    fn merges_follow_who_may_merge_and_approval() {
        use MergeRefusal::*;
        use Merger::*;
        let agents = policy(MergeBy::Agents, false);
        assert_eq!(may_merge(&agents, Automated, false, &[]), Ok(()));
        assert_eq!(may_merge(&agents, User, false, &[]), Ok(()));
        let strict = policy(MergeBy::Agents, true);
        assert_eq!(may_merge(&strict, Automated, false, &[]), Err(ApprovalRequired));
        assert_eq!(may_merge(&strict, Automated, true, &[]), Ok(()));
        // An exempt label skips the approval; other labels don't.
        let exempt = ProjectPolicy { approval_exempt_labels: vec!["automerge".into()], ..strict.clone() };
        assert_eq!(may_merge(&exempt, Automated, false, &["docs".into(), "automerge".into()]), Ok(()));
        assert_eq!(may_merge(&exempt, Automated, false, &["docs".into()]), Err(ApprovalRequired));
        assert_eq!(may_merge(&exempt, Automated, false, &[]), Err(ApprovalRequired));
        let people_exempt = ProjectPolicy { merge_by: MergeBy::User, ..exempt.clone() };
        assert_eq!(may_merge(&people_exempt, Automated, true, &["automerge".into()]), Err(NotAllowed));
        // A person's merge is its own approval.
        assert_eq!(may_merge(&strict, User, false, &[]), Ok(()));
        let people = policy(MergeBy::User, false);
        assert_eq!(may_merge(&people, Automated, true, &[]), Err(NotAllowed));
        assert_eq!(may_merge(&people, User, false, &[]), Ok(()));
        let external = policy(MergeBy::External, false);
        assert_eq!(may_merge(&external, User, true, &[]), Err(NotAllowed));
        assert_eq!(NotAllowed.message(&external), "this project's merges happen outside PDT");
        assert_eq!(NotAllowed.message(&people), "in this project only a person merges");
    }

    #[test]
    fn the_cap_counts_active_attempts() {
        let none = ProjectPolicy::default();
        assert!(!none.over_cap(50));
        assert_eq!(none.auto_start_cap(), 1);
        let two = ProjectPolicy { max_running_attempts: Some(2), ..ProjectPolicy::default() };
        assert!(!two.over_cap(1));
        assert!(two.over_cap(2));
        assert_eq!(two.auto_start_cap(), 2);
    }

    #[test]
    fn values_round_trip() {
        for m in MergeBy::ALL {
            assert_eq!(MergeBy::parse(m.as_str()), Some(m));
        }
        for a in AutoStart::ALL {
            assert_eq!(AutoStart::parse(a.as_str()), Some(a));
        }
        for o in AutoStartOrder::ALL {
            assert_eq!(AutoStartOrder::parse(o.as_str()), Some(o));
        }
        assert_eq!(MergeBy::parse("robots"), None);
    }

    pub(crate) fn ticket(id: &str, stage: TicketStage) -> TicketSummary {
        TicketSummary {
            id: id.into(),
            project_id: "p".into(),
            key: format!("P-{id}"),
            number: 1,
            title: id.into(),
            stage,
            priority: Priority::None,
            labels: vec![],
            areas: vec![],
            rank: "a".into(),
            waiting: false,
            clear_to_start: false,
            blocked_by: 0,
            attempt: None,
            overlap: OverlapLevel::None,
            overlap_with: vec![],
            created_at: 0,
            updated_at: 0,
            closed_at: None,
            parent: None,
            parts: None,
            pipeline_stage: None,
            agent: None,
            auto_start_at: None,
            pickup_hold: None,
            pickup_paused: None,
            pickup_failed: None,
            external: vec![],
            source: Default::default(),
            admitted: false,
            created_by: None,
            proposed: false,
        }
    }

    #[test]
    fn the_countdown_shows_while_a_candidate_waits_out_the_delay() {
        let todo = ProjectPolicy { auto_start: AutoStart::Todo, ..ProjectPolicy::default() };
        let made = 10_000;
        let fresh = TicketSummary { created_at: made, updated_at: made, ..ticket("fresh", TicketStage::Todo) };
        assert_eq!(auto_start_at(&todo, &fresh, made + 1_000), Some(made + 120_000));
        assert_eq!(auto_start_at(&todo, &fresh, made + 120_000), None);
        let linked = TicketSummary { updated_at: made + 90_000, ..fresh.clone() };
        assert_eq!(auto_start_at(&todo, &linked, made + 120_000), Some(made + 210_000));
        // Blocked: no countdown.
        let blocked = TicketSummary { blocked_by: 1, ..linked };
        assert_eq!(auto_start_at(&todo, &blocked, made), None);
        // 0: at once.
        let now = ProjectPolicy { start_delay_seconds: 0, ..todo.clone() };
        assert_eq!(auto_start_at(&now, &fresh, made), None);
        // Only tickets auto-start would take count down.
        assert_eq!(auto_start_at(&ProjectPolicy::default(), &fresh, made), None);
        let ready = TicketSummary { stage: TicketStage::Ready, ..fresh };
        assert_eq!(auto_start_at(&todo, &ready, made), None);
        let take_ready = ProjectPolicy { auto_start: AutoStart::Ready, ..todo };
        assert_eq!(auto_start_at(&take_ready, &ready, made), Some(made + 120_000));
    }
}
