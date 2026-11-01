//! Ticket proposals (PDT-366): a ticket an agent session makes (not the PM's, not triage's, not Make into parts')
//! waits in Draft, off the board, for the user's Accept to To do, Accept to backlog, Duplicate of… or Reject in the
//! Inbox. The "Ticket proposals" built-in workflow puts it there; turning that workflow off turns proposals off. Pure
//! rules; pdt-api holds the ticket, makes the item and answers it.

use crate::ticket::{AGENTIC_LABEL, Actor, Priority, TicketSource, TicketStage, ValidNewTicket};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The user's Accept or Duplicate of… on a ticket proposal (`InboxAnswer.proposal`). Absent fields keep the agent's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProposalAnswer {
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub priority: Option<Priority>,
    /// Replaces every label (`agentic` aside: the box decides that).
    #[serde(default)]
    #[ts(optional)]
    pub labels: Option<Vec<String>>,
    /// Accept labels it `agentic`, so agents may pick it up. Unticked unless the user ticks it.
    #[serde(default)]
    pub agentic: bool,
    /// Duplicate of…: the ticket it repeats (key or id).
    #[serde(default)]
    #[ts(optional)]
    pub duplicate_of: Option<String>,
}

/// What the user decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Accept to To do; `agentic` when they ticked the box.
    Todo { agentic: bool },
    /// Accept to backlog (Ready).
    Backlog { agentic: bool },
    /// Duplicate of the ticket with this key: Won't do, linked to it.
    Duplicate { of: String },
    /// Reject: Won't do.
    Rejected,
}

impl Decision {
    /// Where the ticket goes.
    pub fn stage(&self) -> TicketStage {
        match self {
            Decision::Todo { .. } => TicketStage::Todo,
            Decision::Backlog { .. } => TicketStage::Ready,
            Decision::Duplicate { .. } | Decision::Rejected => TicketStage::WontDo,
        }
    }
}

/// Whether a new ticket is one to propose: made by an agent session other than the PM's or triage's (whose sources
/// are `pm` and `external`), and not one of the parts Make into parts makes.
pub fn proposes(source: TicketSource, via_parts: bool) -> bool {
    source == TicketSource::Agent && !via_parts
}

/// A proposal is made in Draft and without the `agentic` label, whatever the agent asked: the user decides both.
pub fn hold(t: &mut ValidNewTicket) {
    t.stage = TicketStage::Draft;
    t.labels.retain(|l| !l.eq_ignore_ascii_case(AGENTIC_LABEL));
}

/// The labels an accepted proposal gets: `labels` with [`AGENTIC_LABEL`] exactly when `agentic`.
pub fn accepted_labels(labels: &[String], agentic: bool) -> Vec<String> {
    let mut out: Vec<String> = labels.iter().filter(|l| !l.eq_ignore_ascii_case(AGENTIC_LABEL)).cloned().collect();
    if agentic {
        out.push(AGENTIC_LABEL.into());
    }
    out
}

/// An agent can't decide a proposal itself (PDT-366): while it waits in the Inbox, an agent's move of it (or its
/// `agentic` label) is refused. `changes`: the change moves it or adds the label.
pub fn agent_gate(proposed: bool, actor: Actor, changes: bool, key: &str) -> Result<(), String> {
    if proposed && changes && actor == Actor::Agent {
        return Err(format!(
            "{key} is a ticket proposal waiting for the user in PDT's Inbox: only they move it out of Draft or label \
             it {AGENTIC_LABEL}. Leave it there; you hear their decision."
        ));
    }
    Ok(())
}

/// What the proposing agent hears (in its session, else as a comment on the ticket it came from): `key` and `title`
/// the proposal's, `note` the user's reason or note.
pub fn decision_text(key: &str, title: &str, decision: &Decision, note: Option<&str>) -> String {
    let mut s = match decision {
        Decision::Todo { agentic } => format!(
            "The user accepted your ticket proposal {key} \"{title}\" to To do{}.",
            if *agentic { ", labelled agentic so agents can pick it up" } else { "" }
        ),
        Decision::Backlog { agentic } => format!(
            "The user accepted your ticket proposal {key} \"{title}\" to the backlog (Ready){}.",
            if *agentic { ", labelled agentic" } else { "" }
        ),
        Decision::Duplicate { of } => format!(
            "The user closed your ticket proposal {key} \"{title}\" as a duplicate of {of}: it is Won't do and \
             linked to {of}."
        ),
        Decision::Rejected => format!("The user rejected your ticket proposal {key} \"{title}\": it is Won't do."),
    };
    if let Some(n) = note.map(str::trim).filter(|n| !n.is_empty()) {
        s.push_str(&format!("\n\nTheir reason: {n}"));
    }
    s
}

/// What `create_ticket` tells an agent whose ticket became a proposal.
pub fn created_note(key: &str) -> String {
    format!(
        "{key} is a proposal: it waits in Draft, off the board, for the user to accept, merge or reject it in PDT's \
         Inbox. You hear their decision (in this session if it is still open, else as a comment on the ticket you \
         work on). Don't move it or label it {AGENTIC_LABEL} yourself."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_agents_tickets_are_proposed() {
        assert!(proposes(TicketSource::Agent, false));
        assert!(!proposes(TicketSource::Agent, true), "Make into parts' parts");
        for s in [TicketSource::User, TicketSource::Pm, TicketSource::Workflow, TicketSource::External] {
            assert!(!proposes(s, false), "{s:?}");
        }
    }

    #[test]
    fn a_proposal_is_held_in_draft_without_agentic() {
        let mut t = crate::ticket::NewTicket {
            title: "Fix it".into(),
            stage: Some(TicketStage::Todo),
            labels: Some(vec!["Agentic".into(), "bug".into()]),
            ..Default::default()
        }
        .validate()
        .unwrap();
        hold(&mut t);
        assert_eq!((t.stage, t.labels), (TicketStage::Draft, vec!["bug".to_string()]));
    }

    #[test]
    fn the_box_decides_agentic() {
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(accepted_labels(&l(&["bug", "agentic"]), false), l(&["bug"]));
        assert_eq!(accepted_labels(&l(&["bug"]), true), l(&["bug", "agentic"]));
        assert_eq!(accepted_labels(&l(&["agentic"]), true), l(&["agentic"]), "once");
    }

    #[test]
    fn agents_cant_decide_a_proposal() {
        let e = agent_gate(true, Actor::Agent, true, "PX-4").unwrap_err();
        assert!(e.contains("PX-4") && e.contains("Inbox"), "{e}");
        assert!(agent_gate(true, Actor::User, true, "PX-4").is_ok());
        assert!(agent_gate(true, Actor::Agent, false, "PX-4").is_ok(), "an edit that doesn't move it");
        assert!(agent_gate(false, Actor::Agent, true, "PX-4").is_ok());
    }

    #[test]
    fn decisions_go_where_they_say() {
        assert_eq!(Decision::Todo { agentic: false }.stage(), TicketStage::Todo);
        assert_eq!(Decision::Backlog { agentic: true }.stage(), TicketStage::Ready);
        assert_eq!(Decision::Duplicate { of: "PX-1".into() }.stage(), TicketStage::WontDo);
        assert_eq!(Decision::Rejected.stage(), TicketStage::WontDo);
    }

    #[test]
    fn the_agent_hears_the_decision_and_reason() {
        let t = decision_text("PX-4", "Fix it", &Decision::Todo { agentic: true }, None);
        assert_eq!(
            t,
            "The user accepted your ticket proposal PX-4 \"Fix it\" to To do, labelled agentic so agents can pick it up."
        );
        let t = decision_text("PX-4", "Fix it", &Decision::Duplicate { of: "PX-1".into() }, Some(" "));
        assert!(t.contains("duplicate of PX-1") && !t.contains("reason"), "{t}");
        let t = decision_text("PX-4", "Fix it", &Decision::Rejected, Some("Already fixed"));
        assert!(t.ends_with("it is Won't do.\n\nTheir reason: Already fixed"), "{t}");
        assert!(
            decision_text("PX-4", "Fix it", &Decision::Backlog { agentic: false }, None).contains("backlog (Ready).")
        );
    }
}
