//! Project policies (Phase 7c spec §4, §6): who may merge, whether an automated merge needs a person's approval, the
//! cap on running attempts and auto-start. Pure rules: the domain commands (Merge, Start) and the scheduler apply them.

use crate::ticket::{OverlapLevel, TicketSummary};
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

/// A project's policies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectPolicy {
    pub merge_by: MergeBy,
    /// An automated merge needs a person's approval at the branch's current head.
    pub require_approval: bool,
    /// The most active attempts the project may have (null: no cap; auto-start then keeps to one).
    pub max_running_attempts: Option<u32>,
    pub auto_start: AutoStart,
    /// The PM may create and edit the project's tickets and message its sessions without asking (PDT-81).
    #[serde(default)]
    pub pm_acts: bool,
}

impl ProjectPolicy {
    /// How many attempts auto-start fills up to.
    pub fn auto_start_cap(&self) -> u32 {
        self.max_running_attempts.unwrap_or(1)
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

/// Whether `merger` may merge under `policy`; `approved`: a person approved the branch's current head.
pub fn may_merge(policy: &ProjectPolicy, merger: Merger, approved: bool) -> Result<(), MergeRefusal> {
    match (policy.merge_by, merger) {
        (MergeBy::External, _) | (MergeBy::User, Merger::Automated) => Err(MergeRefusal::NotAllowed),
        (_, Merger::User) => Ok(()),
        (MergeBy::Agents, Merger::Automated) if policy.require_approval && !approved => {
            Err(MergeRefusal::ApprovalRequired)
        }
        (MergeBy::Agents, Merger::Automated) => Ok(()),
    }
}

/// Whether auto-start may take this ticket (spec §6.1): not a parent, never started, not blocked, no conflict, and
/// not waiting unless it is clear to start.
pub fn startable(t: &TicketSummary) -> bool {
    t.parts.is_none()
        && t.attempt.is_none()
        && t.blocked_by == 0
        && t.overlap != OverlapLevel::Conflict
        && (!t.waiting || t.clear_to_start)
}

/// The next ticket auto-start takes from `candidates` (To do in rank order, then Ready ones when the policy says so),
/// or None. `skip`: tickets it already failed to start.
pub fn pick_next<'a>(
    policy: &ProjectPolicy,
    active: u32,
    candidates: &'a [TicketSummary],
    skip: &[String],
) -> Option<&'a TicketSummary> {
    use crate::ticket::TicketStage;
    if policy.auto_start == AutoStart::Off || active >= policy.auto_start_cap() {
        return None;
    }
    let ok = |t: &&TicketSummary| startable(t) && !skip.contains(&t.id);
    let todo = candidates.iter().filter(|t| t.stage == TicketStage::Todo).find(ok);
    todo.or_else(|| {
        (policy.auto_start == AutoStart::Ready)
            .then(|| candidates.iter().filter(|t| t.stage == TicketStage::Ready).find(ok))
            .flatten()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ticket::{AttemptState, AttemptSummary, Priority, TicketStage};

    fn policy(merge_by: MergeBy, require_approval: bool) -> ProjectPolicy {
        ProjectPolicy { merge_by, require_approval, ..ProjectPolicy::default() }
    }

    #[test]
    fn merges_follow_who_may_merge_and_approval() {
        use MergeRefusal::*;
        use Merger::*;
        let agents = policy(MergeBy::Agents, false);
        assert_eq!(may_merge(&agents, Automated, false), Ok(()));
        assert_eq!(may_merge(&agents, User, false), Ok(()));
        let strict = policy(MergeBy::Agents, true);
        assert_eq!(may_merge(&strict, Automated, false), Err(ApprovalRequired));
        assert_eq!(may_merge(&strict, Automated, true), Ok(()));
        // A person's merge is its own approval.
        assert_eq!(may_merge(&strict, User, false), Ok(()));
        let people = policy(MergeBy::User, false);
        assert_eq!(may_merge(&people, Automated, true), Err(NotAllowed));
        assert_eq!(may_merge(&people, User, false), Ok(()));
        let external = policy(MergeBy::External, false);
        assert_eq!(may_merge(&external, User, true), Err(NotAllowed));
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
        }
    }

    #[test]
    fn auto_start_takes_to_do_in_order_then_ready_and_skips_what_it_must() {
        let todo = ProjectPolicy { auto_start: AutoStart::Todo, ..ProjectPolicy::default() };
        let ready = ProjectPolicy { auto_start: AutoStart::Ready, max_running_attempts: Some(3), ..todo.clone() };
        let blocked = TicketSummary { blocked_by: 1, ..ticket("blocked", TicketStage::Todo) };
        let conflict = TicketSummary { overlap: OverlapLevel::Conflict, ..ticket("conflict", TicketStage::Todo) };
        let overlap = TicketSummary { overlap: OverlapLevel::Overlap, ..ticket("overlap", TicketStage::Todo) };
        let started = TicketSummary {
            attempt: Some(AttemptSummary {
                id: "a".into(),
                branch: "b".into(),
                base_branch: "main".into(),
                worktree_id: None,
                state: AttemptState::Abandoned,
                created_at: 0,
                ended_at: Some(1),
                last_head: None,
                workspace_id: None,
                pr: None,
            }),
            ..ticket("started", TicketStage::Todo)
        };
        let waiting = TicketSummary { waiting: true, ..ticket("waiting", TicketStage::Todo) };
        let r1 = ticket("r1", TicketStage::Ready);
        let list = vec![blocked, conflict, started, waiting.clone(), overlap.clone(), r1.clone()];
        assert_eq!(pick_next(&todo, 0, &list, &[]).map(|t| t.id.as_str()), Some("overlap"));
        // Skipped after a failed start, so Ready is next only when the policy takes Ready.
        let skip = vec!["overlap".to_string()];
        assert_eq!(pick_next(&todo, 0, &list, &skip), None);
        assert_eq!(pick_next(&ready, 2, &list, &skip).map(|t| t.id.as_str()), Some("r1"));
        // At the cap: nothing.
        assert_eq!(pick_next(&todo, 1, &list, &[]), None);
        assert_eq!(pick_next(&ready, 3, &list, &[]), None);
        assert_eq!(pick_next(&ProjectPolicy::default(), 0, &list, &[]), None);
        // A waiting ticket goes once it is clear.
        let clear = TicketSummary { clear_to_start: true, ..waiting };
        assert_eq!(pick_next(&todo, 0, &[clear], &[]).map(|t| t.id.as_str()), Some("waiting"));
    }
}
