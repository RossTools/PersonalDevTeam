//! Ticket pickup (PDT-327): the pure rules of the `tickets.candidates` and `tickets.choose` workflow steps, which the
//! built-in "Pick up tickets" runs with each project's auto-start settings. Candidates filters and orders a project's
//! tickets; choose applies the caps, the start delay and skip-failed-until-changed, and says why it passed over each
//! ticket it didn't choose.

use crate::policy::{AutoStart, AutoStartOrder, ProjectPolicy};
use crate::ticket::{AttemptState, OverlapLevel, Priority, TicketStage, TicketSummary};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use ts_rs::TS;

/// The order candidates come in, within each stage (stages in the order the filter lists them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PickupOrder {
    /// Most urgent first (urgent, high, medium, low, none), then backlog rank.
    #[default]
    Priority,
    /// Backlog rank only.
    Rank,
    /// Oldest first (when it was created), then backlog rank.
    Age,
}

impl PickupOrder {
    /// Where `t` comes in this order: lower goes first.
    fn key(self, t: &TicketSummary) -> (std::cmp::Reverse<u8>, i64, &str, &str) {
        let urgency = match self {
            PickupOrder::Priority => t.priority.weight(),
            PickupOrder::Rank | PickupOrder::Age => 0,
        };
        let age = match self {
            PickupOrder::Age => t.created_at,
            _ => 0,
        };
        (std::cmp::Reverse(urgency), age, t.rank.as_str(), t.id.as_str())
    }
}

impl From<AutoStartOrder> for PickupOrder {
    fn from(o: AutoStartOrder) -> Self {
        match o {
            AutoStartOrder::Priority => PickupOrder::Priority,
            AutoStartOrder::Rank => PickupOrder::Rank,
        }
    }
}

/// The stages a project's auto-start setting takes, To do first (none while it is off).
pub fn policy_stages(policy: &ProjectPolicy) -> Vec<TicketStage> {
    match policy.auto_start {
        AutoStart::Off => vec![],
        AutoStart::Todo => vec![TicketStage::Todo],
        AutoStart::Ready => vec![TicketStage::Todo, TicketStage::Ready],
    }
}

/// Which tickets are candidates, and in what order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateFilter {
    /// Only tickets in these stages, in this order (To do before Ready, say).
    pub stages: Vec<TicketStage>,
    /// Only tickets with any of these labels (empty: any).
    pub labels: Vec<String>,
    /// No ticket with any of these labels.
    pub exclude_labels: Vec<String>,
    /// Only tickets at least this urgent.
    pub min_priority: Option<Priority>,
    /// Only tickets in any of these areas (empty: any).
    pub areas: Vec<String>,
    /// Leave out tickets with an open blocker.
    pub exclude_blocked: bool,
    /// Leave out parents (with them, a parent's start starts its next part).
    pub exclude_parents: bool,
    /// Leave out tickets that conflict with work in flight (a shared exclusive area).
    pub exclude_conflicts: bool,
    pub order: PickupOrder,
    pub limit: usize,
}

/// How many candidates a project gives unless the step says.
pub const DEFAULT_LIMIT: usize = 50;

impl CandidateFilter {
    /// What auto-start takes under `policy`: its stages and order, nothing blocked, no parents, no conflicts.
    pub fn from_policy(policy: &ProjectPolicy) -> Self {
        CandidateFilter {
            stages: policy_stages(policy),
            labels: vec![],
            exclude_labels: vec![],
            min_priority: None,
            areas: vec![],
            exclude_blocked: true,
            exclude_parents: true,
            exclude_conflicts: true,
            order: policy.auto_start_order.into(),
            limit: DEFAULT_LIMIT,
        }
    }

    /// Whether `t` is a candidate. A ticket with an active attempt, or waiting until clear and not clear yet, never is.
    pub fn accepts(&self, t: &TicketSummary) -> bool {
        let has = |list: &[String], of: &[String]| list.iter().any(|x| of.contains(x));
        self.stages.contains(&t.stage)
            && t.attempt.as_ref().is_none_or(|a| a.state != AttemptState::Active)
            && (!t.waiting || t.clear_to_start)
            && !(self.exclude_parents && t.parts.is_some())
            && !(self.exclude_blocked && t.blocked_by > 0)
            && !(self.exclude_conflicts && t.overlap == OverlapLevel::Conflict)
            && (self.labels.is_empty() || has(&self.labels, &t.labels))
            && !has(&self.exclude_labels, &t.labels)
            && self.min_priority.is_none_or(|p| t.priority.weight() >= p.weight())
            && (self.areas.is_empty() || has(&self.areas, &t.areas))
    }

    /// The candidates among `tickets`, ordered: by stage as listed, then by `order`; at most `limit`.
    pub fn apply<'a>(&self, tickets: &'a [TicketSummary]) -> Vec<&'a TicketSummary> {
        let stage_at = |t: &TicketSummary| self.stages.iter().position(|s| *s == t.stage).unwrap_or(usize::MAX);
        let mut out: Vec<&TicketSummary> = tickets.iter().filter(|t| self.accepts(t)).collect();
        out.sort_by(|a, b| (stage_at(a), self.order.key(a)).cmp(&(stage_at(b), self.order.key(b))));
        out.truncate(self.limit);
        out
    }
}

/// A candidate as the pickup steps pass it on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PickupTicket {
    pub id: String,
    pub project_id: String,
    pub key: String,
    pub title: String,
    pub stage: TicketStage,
    pub priority: Priority,
    pub labels: Vec<String>,
    pub areas: Vec<String>,
    pub rank: String,
    /// The worst overlap with work in flight, and the keys of the tickets it overlaps.
    #[schemars(with = "String")]
    pub overlap: OverlapLevel,
    pub overlap_with: Vec<String>,
    pub blocked_by: u32,
    pub has_parts: bool,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

impl From<&TicketSummary> for PickupTicket {
    fn from(t: &TicketSummary) -> Self {
        PickupTicket {
            id: t.id.clone(),
            project_id: t.project_id.clone(),
            key: t.key.clone(),
            title: t.title.clone(),
            stage: t.stage,
            priority: t.priority,
            labels: t.labels.clone(),
            areas: t.areas.clone(),
            rank: t.rank.clone(),
            overlap: t.overlap,
            overlap_with: t.overlap_with.clone(),
            blocked_by: t.blocked_by,
            has_parts: t.parts.is_some(),
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

/// A ticket in flight (an active attempt), for the area and label caps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ActiveTicket {
    pub id: String,
    pub key: String,
    pub labels: Vec<String>,
    pub areas: Vec<String>,
}

/// One project's candidates, with what choose needs to know of it: its auto-start cap and start delay, and the
/// tickets in flight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PickupProject {
    pub project_id: String,
    pub key: String,
    pub name: String,
    /// The project's auto-start cap (its running cap, else 1).
    pub cap: u32,
    /// The project's start delay.
    pub start_delay_seconds: u32,
    pub active: Vec<ActiveTicket>,
    pub tickets: Vec<PickupTicket>,
}

/// How choose overlaps candidates with work in flight (more rules come with overlap-aware pickup).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OverlapRule {
    /// No rule of its own (candidates leaves out conflicts).
    #[default]
    Off,
}

/// What choose applies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChooseRules {
    /// The most tickets in flight per project; None: each project's auto-start cap.
    pub cap: Option<u32>,
    /// The most tickets in flight with an area (by area).
    pub area_caps: BTreeMap<String, u32>,
    /// The most tickets in flight with a label (by label).
    pub label_caps: BTreeMap<String, u32>,
    /// How long a ticket goes unchanged first; None: each project's start delay.
    pub start_delay_seconds: Option<u32>,
    /// Pass over a ticket that failed to start until it changes.
    pub skip_failed: bool,
    pub overlap: OverlapRule,
}

impl ChooseRules {
    /// What auto-start applies: each project's cap and delay, skipping failed starts.
    pub fn auto_start() -> Self {
        ChooseRules { skip_failed: true, ..Default::default() }
    }
}

/// Why choose passed over a candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "reason")]
#[ts(export)]
pub enum PassedReason {
    /// The project has `active` tickets in flight, at its cap.
    #[serde(rename = "at_cap")]
    AtCap { cap: u32, active: u32 },
    /// `area` has `cap` tickets in flight.
    #[serde(rename = "area_cap")]
    AreaCap { area: String, cap: u32 },
    /// `label` has `cap` tickets in flight.
    #[serde(rename = "label_cap")]
    LabelCap { label: String, cap: u32 },
    /// Changed within the start delay: it may go at `at`.
    #[serde(rename = "settling")]
    Settling {
        #[ts(type = "number")]
        at: i64,
    },
    /// It failed to start and hasn't changed since.
    #[serde(rename = "failed_before")]
    FailedBefore,
}

impl PassedReason {
    /// The reason in a few words, for the run log.
    pub fn describe(&self) -> String {
        match self {
            PassedReason::AtCap { cap, active } => format!("the project is at its cap ({active} of {cap} in flight)"),
            PassedReason::AreaCap { area, cap } => format!("area {area} is at its cap of {cap}"),
            PassedReason::LabelCap { label, cap } => format!("label {label} is at its cap of {cap}"),
            PassedReason::Settling { .. } => "changed within the start delay".into(),
            PassedReason::FailedBefore => "it failed to start and hasn't changed since".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Passed {
    pub id: String,
    pub key: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub reason: PassedReason,
    pub why: String,
}

/// A chosen ticket, with the project cap it was chosen under (the start checks it again).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Pick {
    #[serde(flatten)]
    #[ts(flatten)]
    pub ticket: PickupTicket,
    pub cap: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    /// Every ticket that may start, project by project, each in candidate order: the first of a project is its pick.
    pub chosen: Vec<Pick>,
    pub passed: Vec<Passed>,
}

/// Choose from `projects`' candidates under `rules` at `now`. `failed`: tickets that failed to start, with their
/// `updated_at` then.
pub fn choose(projects: &[PickupProject], rules: &ChooseRules, failed: &HashMap<String, i64>, now: i64) -> Choice {
    let mut out = Choice::default();
    for p in projects {
        let cap = rules.cap.unwrap_or(p.cap);
        let active = u32::try_from(p.active.len()).unwrap_or(u32::MAX);
        let delay_ms = i64::from(rules.start_delay_seconds.unwrap_or(p.start_delay_seconds)) * 1000;
        let in_flight = |pick: &dyn Fn(&ActiveTicket) -> &Vec<String>, name: &str| {
            p.active.iter().filter(|a| pick(a).iter().any(|x| x == name)).count()
        };
        for t in &p.tickets {
            let at = t.created_at.max(t.updated_at).saturating_add(delay_ms);
            let full = |caps: &BTreeMap<String, u32>, of: &[String], pick: &dyn Fn(&ActiveTicket) -> &Vec<String>| {
                of.iter().find_map(|x| {
                    caps.get(x)
                        .filter(|c| in_flight(pick, x) >= usize::try_from(**c).unwrap_or(usize::MAX))
                        .map(|c| (x.clone(), *c))
                })
            };
            let reason = if active >= cap {
                Some(PassedReason::AtCap { cap, active })
            } else if at > now {
                Some(PassedReason::Settling { at })
            } else if rules.skip_failed && failed.get(&t.id) == Some(&t.updated_at) {
                Some(PassedReason::FailedBefore)
            } else if let Some((area, cap)) = full(&rules.area_caps, &t.areas, &|a| &a.areas) {
                Some(PassedReason::AreaCap { area, cap })
            } else if let Some((label, cap)) = full(&rules.label_caps, &t.labels, &|a| &a.labels) {
                Some(PassedReason::LabelCap { label, cap })
            } else {
                None
            };
            match reason {
                Some(reason) => {
                    out.passed.push(Passed { id: t.id.clone(), key: t.key.clone(), why: reason.describe(), reason })
                }
                None => out.chosen.push(Pick { ticket: t.clone(), cap }),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::tests::ticket;
    use crate::ticket::{AttemptSummary, TicketStage};

    /// Long after the test tickets were made.
    const LATER: i64 = 1_000_000_000;

    fn project(policy: &ProjectPolicy, active: u32, list: &[TicketSummary]) -> PickupProject {
        PickupProject {
            project_id: "p".into(),
            key: "P".into(),
            name: "P".into(),
            cap: policy.auto_start_cap(),
            start_delay_seconds: policy.start_delay_seconds,
            active: (0..active)
                .map(|n| ActiveTicket { id: format!("a{n}"), key: format!("P-a{n}"), labels: vec![], areas: vec![] })
                .collect(),
            tickets: CandidateFilter::from_policy(policy).apply(list).into_iter().map(PickupTicket::from).collect(),
        }
    }

    /// What the built-in "Pick up tickets" starts next: candidates and choose with the project's settings (what
    /// `policy::pick_next` did before PDT-327).
    fn pick(policy: &ProjectPolicy, active: u32, list: &[TicketSummary], skip: &[String], now: i64) -> Option<String> {
        let failed: HashMap<String, i64> =
            list.iter().filter(|t| skip.contains(&t.id)).map(|t| (t.id.clone(), t.updated_at)).collect();
        choose(&[project(policy, active, list)], &ChooseRules::auto_start(), &failed, now)
            .chosen
            .first()
            .map(|p| p.ticket.id.clone())
    }

    fn active_attempt() -> AttemptSummary {
        AttemptSummary {
            id: "a".into(),
            branch: "b".into(),
            base_branch: "main".into(),
            worktree_id: None,
            state: AttemptState::Active,
            created_at: 0,
            ended_at: None,
            last_head: None,
            workspace_id: None,
            pr: None,
            ci: None,
        }
    }

    #[test]
    fn auto_start_takes_to_do_in_order_then_ready_and_skips_what_it_must() {
        let todo = ProjectPolicy { auto_start: AutoStart::Todo, ..ProjectPolicy::default() };
        let ready = ProjectPolicy { auto_start: AutoStart::Ready, max_running_attempts: Some(3), ..todo.clone() };
        let blocked = TicketSummary { blocked_by: 1, ..ticket("blocked", TicketStage::Todo) };
        let conflict = TicketSummary { overlap: OverlapLevel::Conflict, ..ticket("conflict", TicketStage::Todo) };
        let overlap = TicketSummary { overlap: OverlapLevel::Overlap, ..ticket("overlap", TicketStage::Todo) };
        let started = TicketSummary { attempt: Some(active_attempt()), ..ticket("started", TicketStage::Todo) };
        let waiting = TicketSummary { waiting: true, ..ticket("waiting", TicketStage::Todo) };
        let r1 = ticket("r1", TicketStage::Ready);
        let list = vec![blocked, conflict, started.clone(), waiting.clone(), overlap.clone(), r1.clone()];
        assert_eq!(pick(&todo, 0, &list, &[], LATER).as_deref(), Some("overlap"));
        // Only an active attempt holds a ticket back (PDT-159): one stopped and moved back to To do starts again.
        let mut stopped = started.clone();
        stopped.id = "stopped".into();
        for (state, ended) in [(AttemptState::Abandoned, Some(1)), (AttemptState::Merged, Some(1))] {
            let a = stopped.attempt.as_mut().unwrap();
            (a.state, a.ended_at) = (state, ended);
            assert_eq!(pick(&todo, 0, std::slice::from_ref(&stopped), &[], LATER).as_deref(), Some("stopped"));
        }
        // Skipped after a failed start, so Ready is next only when the policy takes Ready.
        let skip = vec!["overlap".to_string()];
        assert_eq!(pick(&todo, 0, &list, &skip, LATER), None);
        assert_eq!(pick(&ready, 2, &list, &skip, LATER).as_deref(), Some("r1"));
        // A failed ticket that changed since is tried again.
        let mut changed = list.clone();
        changed[4].updated_at = 5;
        let failed = HashMap::from([("overlap".to_string(), 0)]);
        let c = choose(&[project(&todo, 0, &changed)], &ChooseRules::auto_start(), &failed, LATER);
        assert_eq!(c.chosen[0].ticket.id, "overlap");
        // At the cap: nothing.
        assert_eq!(pick(&todo, 1, &list, &[], LATER), None);
        assert_eq!(pick(&ready, 3, &list, &[], LATER), None);
        assert_eq!(pick(&ProjectPolicy::default(), 0, &list, &[], LATER), None);
        // A waiting ticket goes once it is clear.
        let clear = TicketSummary { clear_to_start: true, ..waiting };
        assert_eq!(pick(&todo, 0, &[clear], &[], LATER).as_deref(), Some("waiting"));
    }

    #[test]
    fn auto_start_takes_the_most_urgent_first_unless_the_project_keeps_to_rank() {
        let by_priority = ProjectPolicy { auto_start: AutoStart::Ready, ..ProjectPolicy::default() };
        assert_eq!(by_priority.auto_start_order, AutoStartOrder::Priority, "most urgent first unless the project says");
        let by_rank = ProjectPolicy { auto_start_order: AutoStartOrder::Rank, ..by_priority.clone() };
        let t =
            |id: &str, stage, priority, rank: &str| TicketSummary { priority, rank: rank.into(), ..ticket(id, stage) };
        let list = vec![
            t("medium", TicketStage::Todo, Priority::Medium, "a"),
            t("high-later", TicketStage::Todo, Priority::High, "c"),
            t("high", TicketStage::Todo, Priority::High, "b"),
            t("urgent-ready", TicketStage::Ready, Priority::Urgent, "0"),
            t("low-ready", TicketStage::Ready, Priority::Low, "1"),
            t("none-ready", TicketStage::Ready, Priority::None, "2"),
        ];
        let p = |p: &ProjectPolicy, skip: &[&str]| {
            let skip: Vec<String> = skip.iter().map(|s| s.to_string()).collect();
            pick(p, 0, &list, &skip, LATER)
        };
        // A high-priority ticket ranked below a medium one goes first; within a priority, rank decides.
        assert_eq!(p(&by_priority, &[]).as_deref(), Some("high"));
        assert_eq!(p(&by_priority, &["high"]).as_deref(), Some("high-later"));
        assert_eq!(p(&by_priority, &["high", "high-later"]).as_deref(), Some("medium"));
        // By rank: today's order, whatever the priority.
        assert_eq!(p(&by_rank, &[]).as_deref(), Some("medium"));
        assert_eq!(p(&by_rank, &["medium"]).as_deref(), Some("high"));
        // To do comes before Ready in both, however urgent the Ready ticket.
        let todo = ["medium", "high", "high-later"];
        assert_eq!(p(&by_priority, &todo).as_deref(), Some("urgent-ready"));
        assert_eq!(p(&by_priority, &["medium", "high", "high-later", "urgent-ready"]).as_deref(), Some("low-ready"));
        assert_eq!(p(&by_rank, &todo).as_deref(), Some("urgent-ready"));
        // Blocked tickets still wait.
        let mut blocked = list.clone();
        blocked[2].blocked_by = 1;
        assert_eq!(pick(&by_priority, 0, &blocked, &[], LATER).as_deref(), Some("high-later"));
        // Every chosen ticket comes in candidate order, the pick first.
        let c = choose(&[project(&by_priority, 0, &list)], &ChooseRules::auto_start(), &HashMap::new(), LATER);
        let ids: Vec<&str> = c.chosen.iter().map(|p| p.ticket.id.as_str()).collect();
        assert_eq!(ids, ["high", "high-later", "medium", "urgent-ready", "low-ready", "none-ready"]);
    }

    #[test]
    fn auto_start_waits_until_a_ticket_has_been_quiet_for_the_delay() {
        let todo = ProjectPolicy { auto_start: AutoStart::Todo, ..ProjectPolicy::default() };
        assert_eq!(todo.start_delay_seconds, 120, "two minutes unless the project says otherwise");
        let made = 10_000;
        let fresh = TicketSummary { created_at: made, updated_at: made, ..ticket("fresh", TicketStage::Todo) };
        let list = vec![fresh.clone()];
        // Just created: not picked, and the run log says when it may go.
        assert_eq!(pick(&todo, 0, &list, &[], made + 1_000), None);
        let c = choose(&[project(&todo, 0, &list)], &ChooseRules::auto_start(), &HashMap::new(), made + 1_000);
        assert_eq!(c.passed[0].reason, PassedReason::Settling { at: made + 120_000 });
        // After the delay: picked.
        assert_eq!(pick(&todo, 0, &list, &[], made + 120_000).as_deref(), Some("fresh"));
        // A change inside the window (a link bumps `updated_at`) restarts it.
        let linked = TicketSummary { updated_at: made + 90_000, ..fresh.clone() };
        assert_eq!(pick(&todo, 0, std::slice::from_ref(&linked), &[], made + 120_000), None);
        assert!(pick(&todo, 0, std::slice::from_ref(&linked), &[], made + 210_000).is_some());
        // A blocker linked inside the window keeps it from starting at all.
        let blocked = TicketSummary { blocked_by: 1, ..linked };
        assert_eq!(pick(&todo, 0, std::slice::from_ref(&blocked), &[], LATER), None);
        // 0: at once.
        let now = ProjectPolicy { start_delay_seconds: 0, ..todo.clone() };
        assert_eq!(pick(&now, 0, &list, &[], made).as_deref(), Some("fresh"));
        // The step's own delay wins over the project's.
        let rules = ChooseRules { start_delay_seconds: Some(0), ..ChooseRules::auto_start() };
        let c = choose(&[project(&todo, 0, &list)], &rules, &HashMap::new(), made);
        assert_eq!(c.chosen.len(), 1);
    }

    #[test]
    fn candidates_filter_by_label_priority_area_and_parents_and_order_by_age() {
        let mut all = ProjectPolicy { auto_start: AutoStart::Ready, ..ProjectPolicy::default() };
        all.auto_start_order = AutoStartOrder::Rank;
        let t = |id: &str, labels: &[&str], priority, made: i64| TicketSummary {
            labels: labels.iter().map(|s| s.to_string()).collect(),
            priority,
            created_at: made,
            rank: id.into(),
            ..ticket(id, TicketStage::Todo)
        };
        let list = vec![
            t("a", &["agentic"], Priority::Low, 30),
            t("b", &["agentic", "ui"], Priority::High, 10),
            t("c", &[], Priority::Urgent, 20),
            TicketSummary { areas: vec!["web".into()], ..t("d", &["agentic"], Priority::Medium, 40) },
        ];
        let ids = |f: &CandidateFilter| f.apply(&list).iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        let base = CandidateFilter::from_policy(&all);
        assert_eq!(ids(&base), ["a", "b", "c", "d"]);
        assert_eq!(ids(&CandidateFilter { labels: vec!["agentic".into()], ..base.clone() }), ["a", "b", "d"]);
        assert_eq!(ids(&CandidateFilter { exclude_labels: vec!["ui".into()], ..base.clone() }), ["a", "c", "d"]);
        assert_eq!(ids(&CandidateFilter { min_priority: Some(Priority::Medium), ..base.clone() }), ["b", "c", "d"]);
        assert_eq!(ids(&CandidateFilter { areas: vec!["web".into()], ..base.clone() }), ["d"]);
        assert_eq!(ids(&CandidateFilter { order: PickupOrder::Age, ..base.clone() }), ["b", "c", "a", "d"]);
        assert_eq!(ids(&CandidateFilter { limit: 2, ..base.clone() }), ["a", "b"]);
        // Ready only: none of these.
        assert!(ids(&CandidateFilter { stages: vec![TicketStage::Ready], ..base.clone() }).is_empty());
        // Parents and blocked tickets only when asked for.
        let parent = TicketSummary {
            parts: Some(crate::parts::PartsProgress { total: 1, ..Default::default() }),
            blocked_by: 1,
            ..t("e", &[], Priority::None, 0)
        };
        let list = [parent];
        assert!(base.apply(&list).is_empty());
        let open = CandidateFilter { exclude_parents: false, exclude_blocked: false, ..base.clone() };
        assert_eq!(open.apply(&list).len(), 1);
        assert!(PickupTicket::from(open.apply(&list)[0]).has_parts);
        // Off: no stages, no candidates.
        assert!(CandidateFilter::from_policy(&ProjectPolicy::default()).stages.is_empty());
    }

    #[test]
    fn choose_keeps_to_area_and_label_caps_and_its_own_project_cap() {
        let policy = ProjectPolicy { auto_start: AutoStart::Todo, start_delay_seconds: 0, ..ProjectPolicy::default() };
        let t = |id: &str, labels: &[&str], areas: &[&str]| TicketSummary {
            labels: labels.iter().map(|s| s.to_string()).collect(),
            areas: areas.iter().map(|s| s.to_string()).collect(),
            rank: id.into(),
            ..ticket(id, TicketStage::Todo)
        };
        let list = vec![t("a", &["agentic"], &["web"]), t("b", &[], &["api"]), t("c", &["agentic"], &[])];
        let mut p = project(&policy, 0, &list);
        p.cap = 5;
        p.active = vec![ActiveTicket {
            id: "x".into(),
            key: "P-x".into(),
            labels: vec!["agentic".into()],
            areas: vec!["web".into()],
        }];
        let rules = ChooseRules {
            area_caps: BTreeMap::from([("web".into(), 1)]),
            label_caps: BTreeMap::from([("agentic".into(), 1)]),
            ..ChooseRules::auto_start()
        };
        let c = choose(std::slice::from_ref(&p), &rules, &HashMap::new(), LATER);
        let ids: Vec<&str> = c.chosen.iter().map(|p| p.ticket.id.as_str()).collect();
        assert_eq!(ids, ["b"]);
        assert_eq!(c.passed[0].reason, PassedReason::AreaCap { area: "web".into(), cap: 1 });
        assert_eq!(c.passed[1].reason, PassedReason::LabelCap { label: "agentic".into(), cap: 1 });
        assert_eq!(c.chosen[0].cap, 5);
        // The step's cap wins over the project's.
        let c = choose(std::slice::from_ref(&p), &ChooseRules { cap: Some(1), ..rules }, &HashMap::new(), LATER);
        assert!(c.chosen.is_empty());
        assert_eq!(c.passed[0].reason, PassedReason::AtCap { cap: 1, active: 1 });
        assert_eq!(c.passed[0].why, "the project is at its cap (1 of 1 in flight)");
    }
}
