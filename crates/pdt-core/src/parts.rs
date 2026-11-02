//! Ticket parts (PDT-38): a ticket that takes several PRs
//! is a parent whose parts are child tickets, one level deep. The rules here are pure: which ticket may be a parent,
//! the parent's roll-up, which part Start picks next, and whether a parent may close.

use crate::ticket::{AttemptState, TicketStage};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A part's parent, as the part shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketParent {
    pub id: String,
    /// `PDT-2`.
    pub key: String,
    pub title: String,
}

/// Where a part is, for the parent's stepper and counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PartState {
    /// Done or Won't do.
    Done,
    /// Its attempt merged; the part is not moved to Done yet.
    Merged,
    /// Being worked on, with a pull request open.
    Review,
    /// Being worked on (an active attempt, or In progress without one).
    Progress,
    /// Picked: To do.
    Todo,
    /// Draft or Ready.
    Backlog,
}

/// A parent's roll-up of its parts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PartsProgress {
    pub total: u32,
    pub done: u32,
    pub merged: u32,
    pub review: u32,
    pub progress: u32,
    pub todo: u32,
    pub backlog: u32,
    /// One per part, in the parts' rank order.
    pub segments: Vec<PartState>,
}

/// The most parts one `POST /api/tickets/{id}/parts` makes.
pub const MAX_NEW_PARTS: usize = 50;

/// One part of [`NewParts`].
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewPart {
    pub title: String,
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
}

/// `POST /api/tickets/{id}/parts`: makes several parts, in order, after the parent's existing parts.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewParts {
    pub parts: Vec<NewPart>,
    /// Each new part is blocked by the part before it (the first by the parent's last existing part).
    #[serde(default)]
    #[ts(optional)]
    pub blocked_by_previous: Option<bool>,
}

/// The parts [`NewParts`] made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PartsCreated {
    pub items: Vec<crate::ticket::TicketSummary>,
}

/// A checked [`NewParts`]: titles and descriptions.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidParts {
    pub parts: Vec<(String, String)>,
    pub blocked_by_previous: bool,
}

impl NewParts {
    pub fn validate(&self) -> Result<ValidParts, crate::ticket::TicketValidationError> {
        use crate::ticket::{TicketValidationError, valid_description, valid_title};
        if self.parts.is_empty() {
            return Err(TicketValidationError::Invalid("no parts given".into()));
        }
        if self.parts.len() > MAX_NEW_PARTS {
            return Err(TicketValidationError::Invalid(format!("at most {MAX_NEW_PARTS} parts at once")));
        }
        let parts = self
            .parts
            .iter()
            .map(|p| Ok((valid_title(&p.title)?, valid_description(p.description.as_deref().unwrap_or(""))?)))
            .collect::<Result<Vec<_>, TicketValidationError>>()?;
        Ok(ValidParts { parts, blocked_by_previous: self.blocked_by_previous.unwrap_or(false) })
    }
}

/// A part's state from its stage, its attempt (the active one, else the latest) and whether that attempt has an open
/// pull request.
pub fn part_state(stage: TicketStage, attempt: Option<AttemptState>, pr_open: bool) -> PartState {
    if stage.is_closed() {
        return PartState::Done;
    }
    match attempt {
        Some(AttemptState::Merged) => PartState::Merged,
        Some(AttemptState::Active) if pr_open => PartState::Review,
        Some(AttemptState::Active) => PartState::Progress,
        _ => match stage {
            TicketStage::InProgress => PartState::Progress,
            TicketStage::Todo => PartState::Todo,
            _ => PartState::Backlog,
        },
    }
}

/// The roll-up of parts in rank order.
pub fn progress(states: &[PartState]) -> PartsProgress {
    let mut p = PartsProgress { total: states.len() as u32, segments: states.to_vec(), ..Default::default() };
    for s in states {
        *match s {
            PartState::Done => &mut p.done,
            PartState::Merged => &mut p.merged,
            PartState::Review => &mut p.review,
            PartState::Progress => &mut p.progress,
            PartState::Todo => &mut p.todo,
            PartState::Backlog => &mut p.backlog,
        } += 1;
    }
    p
}

/// What is known about a would-be parent and part when a ticket is given a parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParentFacts {
    pub same_project: bool,
    pub is_self: bool,
    /// The would-be parent is itself a part.
    pub parent_has_parent: bool,
    /// The ticket being made a part has parts of its own.
    pub child_has_parts: bool,
    pub parent_has_active_attempt: bool,
    pub parent_closed: bool,
}

/// Why a ticket can't be another's part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParentRule {
    #[error("a part must be in the same project as its parent")]
    OtherProject,
    #[error("a ticket can't be its own part")]
    Itself,
    #[error("the parent is itself a part; parts are one level deep")]
    ParentIsAPart,
    #[error("the ticket has parts of its own; parts are one level deep")]
    ChildHasParts,
    #[error("the parent has an active attempt; finish or abandon it before giving it parts")]
    ParentActive,
    #[error("the parent is closed")]
    ParentClosed,
}

/// Whether a ticket may become a part of another (spec §4, "Valid parent").
pub fn valid_parent(f: &ParentFacts) -> Result<(), ParentRule> {
    if f.is_self {
        return Err(ParentRule::Itself);
    }
    if !f.same_project {
        return Err(ParentRule::OtherProject);
    }
    if f.parent_has_parent {
        return Err(ParentRule::ParentIsAPart);
    }
    if f.child_has_parts {
        return Err(ParentRule::ChildHasParts);
    }
    if f.parent_has_active_attempt {
        return Err(ParentRule::ParentActive);
    }
    if f.parent_closed {
        return Err(ParentRule::ParentClosed);
    }
    Ok(())
}

/// A part as "Start next part" sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartFacts {
    pub key: String,
    pub state: PartState,
    /// It has an active attempt.
    pub active: bool,
    /// Open tickets blocking it.
    pub open_blockers: u32,
}

/// Why a parent has no part to start next.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoNextPart {
    #[error("it has no parts")]
    NoParts,
    #[error("all its parts are closed")]
    AllClosed,
    #[error("every open part is started or merged")]
    AllStarted,
    #[error("its next parts are blocked: {}", .0.join(", "))]
    AllBlocked(Vec<String>),
}

/// The index of the part "Start next part" starts: the first open part (rank order) that has no active attempt, has
/// not merged, and has no open blockers.
pub fn next_part(parts: &[PartFacts]) -> Result<usize, NoNextPart> {
    if parts.is_empty() {
        return Err(NoNextPart::NoParts);
    }
    let startable = |p: &PartFacts| p.state != PartState::Done && p.state != PartState::Merged && !p.active;
    if parts.iter().all(|p| p.state == PartState::Done) {
        return Err(NoNextPart::AllClosed);
    }
    if let Some(i) = parts.iter().position(|p| startable(p) && p.open_blockers == 0) {
        return Ok(i);
    }
    let blocked: Vec<String> = parts.iter().filter(|p| startable(p)).map(|p| p.key.clone()).collect();
    if blocked.is_empty() { Err(NoNextPart::AllStarted) } else { Err(NoNextPart::AllBlocked(blocked)) }
}

/// The keys of the parts still open (stage not Done/Won't do), which keep their parent from closing.
pub fn open_parts<'a>(parts: impl IntoIterator<Item = (&'a str, TicketStage)>) -> Vec<String> {
    parts.into_iter().filter(|(_, s)| !s.is_closed()).map(|(k, _)| k.to_string()).collect()
}

/// What happens to a parent when one of its parts moves (PDT-377, the "Close finished parents" workflow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentFollow {
    /// Its last open part closed: it closes too, to this stage (Won't do when every part was Won't do, else Done).
    Close(TicketStage),
    /// Its parts are all closed, but it has work of its own (an active attempt, a running session): it stays open.
    Hold,
    /// It is closed and a part is open again: back to In progress.
    Reopen,
    /// Nothing to do: a part is still open, or nothing changed.
    Nothing,
}

/// What a parent in `parent` does now that its parts are in `parts` (`busy`: it has work of its own going).
pub fn parent_follow(parent: TicketStage, parts: &[TicketStage], busy: bool) -> ParentFollow {
    let open = parts.iter().any(|s| !s.is_closed());
    if parent.is_closed() {
        return if open { ParentFollow::Reopen } else { ParentFollow::Nothing };
    }
    if parts.is_empty() || open {
        return ParentFollow::Nothing;
    }
    if busy {
        return ParentFollow::Hold;
    }
    let dropped = parts.iter().all(|s| *s == TicketStage::WontDo);
    ParentFollow::Close(if dropped { TicketStage::WontDo } else { TicketStage::Done })
}

#[cfg(test)]
mod tests {
    use super::*;
    use PartState::*;

    #[test]
    fn a_part_state_comes_from_its_stage_attempt_and_pr() {
        use AttemptState as A;
        use TicketStage as S;
        assert_eq!(part_state(S::Done, Some(A::Active), true), Done);
        assert_eq!(part_state(S::WontDo, None, false), Done);
        assert_eq!(part_state(S::InProgress, Some(A::Merged), false), Merged);
        assert_eq!(part_state(S::InProgress, Some(A::Active), true), Review);
        assert_eq!(part_state(S::InProgress, Some(A::Active), false), Progress);
        assert_eq!(part_state(S::InProgress, Some(A::Abandoned), false), Progress);
        assert_eq!(part_state(S::InProgress, None, false), Progress);
        assert_eq!(part_state(S::Todo, None, false), Todo);
        assert_eq!(part_state(S::Todo, Some(A::Abandoned), true), Todo);
        assert_eq!(part_state(S::Ready, None, false), Backlog);
        assert_eq!(part_state(S::Draft, None, false), Backlog);
    }

    #[test]
    fn progress_counts_each_state_and_keeps_the_order() {
        let p = progress(&[Done, Done, Review, Progress, Todo, Backlog, Merged]);
        assert_eq!((p.total, p.done, p.merged, p.review, p.progress, p.todo, p.backlog), (7, 2, 1, 1, 1, 1, 1));
        assert_eq!(p.segments, [Done, Done, Review, Progress, Todo, Backlog, Merged]);
        assert_eq!(progress(&[]), PartsProgress::default());
        assert_eq!(serde_json::to_value(Review).unwrap(), serde_json::json!("review"));
    }

    #[test]
    fn a_parent_must_be_a_fitting_ticket() {
        let ok = ParentFacts { same_project: true, ..Default::default() };
        assert_eq!(valid_parent(&ok), Ok(()));
        let with = |f: fn(&mut ParentFacts)| {
            let mut x = ok;
            f(&mut x);
            valid_parent(&x)
        };
        assert_eq!(with(|f| f.is_self = true), Err(ParentRule::Itself));
        assert_eq!(with(|f| f.same_project = false), Err(ParentRule::OtherProject));
        assert_eq!(with(|f| f.parent_has_parent = true), Err(ParentRule::ParentIsAPart));
        assert_eq!(with(|f| f.child_has_parts = true), Err(ParentRule::ChildHasParts));
        assert_eq!(with(|f| f.parent_has_active_attempt = true), Err(ParentRule::ParentActive));
        assert_eq!(with(|f| f.parent_closed = true), Err(ParentRule::ParentClosed));
    }

    fn part(key: &str, state: PartState, active: bool, open_blockers: u32) -> PartFacts {
        PartFacts { key: key.into(), state, active, open_blockers }
    }

    #[test]
    fn the_next_part_is_the_first_open_unstarted_unblocked_one() {
        let parts = [
            part("A", Done, false, 0),
            part("B", Merged, false, 0),
            part("C", Review, true, 0),
            part("D", Todo, false, 1),
            part("E", Backlog, false, 0),
        ];
        assert_eq!(next_part(&parts), Ok(4));
        // An abandoned attempt can be started again.
        assert_eq!(next_part(&[part("A", Progress, false, 0)]), Ok(0));
    }

    #[test]
    fn no_next_part_says_why() {
        assert_eq!(next_part(&[]), Err(NoNextPart::NoParts));
        assert_eq!(next_part(&[part("A", Done, false, 0)]), Err(NoNextPart::AllClosed));
        assert_eq!(
            next_part(&[part("A", Done, false, 0), part("B", Progress, true, 0), part("C", Merged, false, 0)]),
            Err(NoNextPart::AllStarted)
        );
        assert_eq!(
            next_part(&[part("A", Progress, true, 0), part("B", Todo, false, 1), part("C", Backlog, false, 2)]),
            Err(NoNextPart::AllBlocked(vec!["B".into(), "C".into()]))
        );
        assert_eq!(NoNextPart::AllBlocked(vec!["B".into()]).to_string(), "its next parts are blocked: B");
    }

    #[test]
    fn new_parts_need_titles_and_a_bound() {
        let n = |titles: &[&str]| NewParts {
            parts: titles.iter().map(|t| NewPart { title: t.to_string(), description: None }).collect(),
            blocked_by_previous: Some(true),
        };
        let v = n(&[" 5a ", "5b"]).validate().unwrap();
        assert_eq!(v.parts, [("5a".to_string(), String::new()), ("5b".to_string(), String::new())]);
        assert!(v.blocked_by_previous);
        assert!(n(&[]).validate().is_err());
        assert!(n(&["ok", " "]).validate().is_err());
        assert!(n(&vec!["x"; MAX_NEW_PARTS + 1]).validate().is_err());
    }

    #[test]
    fn open_parts_keep_a_parent_open() {
        let parts = [("A", TicketStage::Done), ("B", TicketStage::WontDo), ("C", TicketStage::Ready)];
        assert_eq!(open_parts(parts), ["C"]);
        assert!(open_parts([("A", TicketStage::Done)]).is_empty());
    }

    #[test]
    fn a_parent_follows_its_parts() {
        use ParentFollow::*;
        use TicketStage::{Done as D, InProgress, Ready, Todo, WontDo};
        assert_eq!(parent_follow(InProgress, &[D, WontDo], false), Close(D));
        assert_eq!(parent_follow(Todo, &[WontDo, WontDo], false), Close(WontDo), "every part dropped");
        assert_eq!(parent_follow(InProgress, &[D, Ready], false), Nothing, "a part still open");
        assert_eq!(parent_follow(InProgress, &[], false), Nothing, "no parts");
        assert_eq!(parent_follow(InProgress, &[D, D], true), Hold, "work of its own");
        assert_eq!(parent_follow(D, &[D, InProgress], false), Reopen);
        assert_eq!(parent_follow(WontDo, &[Todo], true), Reopen);
        assert_eq!(parent_follow(D, &[D, D], false), Nothing, "closed already");
    }
}
