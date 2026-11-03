//! The PM's `inbox_decide` (PDT-475): it proposes decisions on Inbox items (ticket proposals for now), many in one call,
//! and the user confirms them in one table that lists every row (a card in the PM's conversation, also in the Inbox;
//! an Inbox item from a terminal), unticking the ones they don't want. Pure rules:
//! the call's shape, the rows the confirmation shows, and each row as the answer a click in the Inbox would give.
//! pdt-api finds the items, asks the user and applies the rows they kept.

use crate::inbox::{self, InboxAnswer};
use crate::ticket::Priority;
use crate::ticket_proposal::ProposalAnswer;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The most rows one call takes.
pub const MAX_ROWS: usize = 100;

/// What the PM proposes for a ticket proposal: the Inbox's four choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DecideChoice {
    /// Accept to To do.
    AcceptTodo,
    /// Accept to backlog (Ready).
    AcceptBacklog,
    /// Duplicate of another ticket (`duplicateOf`): Won't do, linked to it.
    Duplicate,
    /// Reject (Won't do), with the reason in `note`.
    Reject,
}

impl DecideChoice {
    /// The Inbox choice it is.
    pub fn choice(self) -> &'static str {
        match self {
            DecideChoice::AcceptTodo => inbox::ACCEPT_TODO,
            DecideChoice::AcceptBacklog => inbox::ACCEPT_BACKLOG,
            DecideChoice::Duplicate => inbox::DUPLICATE,
            DecideChoice::Reject => inbox::REJECT,
        }
    }

    fn accepts(self) -> bool {
        matches!(self, DecideChoice::AcceptTodo | DecideChoice::AcceptBacklog)
    }
}

/// One Inbox item and the decision the PM proposes for it.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemDecision {
    /// The Inbox item: its id, or the proposed ticket's key (`ABC-12`).
    pub item: String,
    pub decision: DecideChoice,
    /// Accept only: label it `agentic`, so agents may pick it up.
    #[serde(default)]
    pub agentic: bool,
    /// Accept only: a new title.
    #[serde(default)]
    pub title: Option<String>,
    /// Accept only: a new priority.
    #[serde(default)]
    pub priority: Option<Priority>,
    /// Accept only: the labels it gets (every label; `agentic` aside, which the `agentic` field decides).
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// Duplicate only: the ticket it repeats (its key).
    #[serde(default)]
    pub duplicate_of: Option<String>,
    /// Why, in a line: the user sees it in the confirmation and the proposing agent hears it. Needed for Reject.
    #[serde(default)]
    pub note: Option<String>,
}

/// `inbox_decide`'s arguments.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InboxDecide {
    /// The decisions, one per item (at most 100).
    pub items: Vec<ItemDecision>,
}

/// One row of the user's confirmation (the item's body `rows`): the item, its ticket and the PM's decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DecideRow {
    pub item_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    /// The ticket's title now (the proposed one).
    pub ticket_title: String,
    pub decision: DecideChoice,
    pub agentic: bool,
    #[ts(optional)]
    pub title: Option<String>,
    #[ts(optional)]
    pub priority: Option<Priority>,
    #[ts(optional)]
    pub labels: Option<Vec<String>>,
    /// The ticket it repeats, as the PM gave it.
    #[ts(optional)]
    pub duplicate_of: Option<String>,
    #[ts(optional)]
    pub note: Option<String>,
}

fn blank(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(|s| s.trim().is_empty())
}

/// Whether the call is one to put to the user: 1 to [`MAX_ROWS`] rows, each with what its decision needs and nothing
/// it can't use. Err names the row (1-based) and what is wrong.
pub fn check(a: &InboxDecide) -> Result<(), String> {
    if a.items.is_empty() {
        return Err("give at least one item to decide".into());
    }
    if a.items.len() > MAX_ROWS {
        return Err(format!("at most {MAX_ROWS} items in one call (got {}); split them", a.items.len()));
    }
    for (n, d) in a.items.iter().enumerate() {
        let row = |m: &str| Err(format!("item {} ({}): {m}", n + 1, d.item.trim()));
        if d.item.trim().is_empty() {
            return row("name the Inbox item (its id or the ticket's key)");
        }
        let edits = d.agentic || d.title.is_some() || d.priority.is_some() || d.labels.is_some();
        if edits && !d.decision.accepts() {
            return row("agentic, title, priority and labels go with accept_todo or accept_backlog only");
        }
        if d.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
            return row("the title is empty");
        }
        match d.decision {
            DecideChoice::Duplicate if blank(&d.duplicate_of) => return row("duplicate needs duplicateOf"),
            DecideChoice::Reject if blank(&d.note) => return row("reject needs the reason in note"),
            c if c != DecideChoice::Duplicate && d.duplicate_of.is_some() => {
                return row("duplicateOf goes with duplicate only");
            }
            _ => {}
        }
    }
    Ok(())
}

/// Each item once: Err names the ticket given twice.
pub fn check_unique(rows: &[DecideRow]) -> Result<(), String> {
    for (n, r) in rows.iter().enumerate() {
        if rows[..n].iter().any(|x| x.item_id == r.item_id) {
            return Err(format!("{} is in the call twice; give each item once", r.ticket_key));
        }
    }
    Ok(())
}

impl DecideRow {
    /// The row for `d` on the item `item_id`, whose ticket is `ticket_id` / `ticket_key` titled `ticket_title`.
    pub fn new(d: &ItemDecision, item_id: &str, ticket_id: &str, ticket_key: &str, ticket_title: &str) -> Self {
        let trim = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        DecideRow {
            item_id: item_id.into(),
            ticket_id: ticket_id.into(),
            ticket_key: ticket_key.into(),
            ticket_title: ticket_title.into(),
            decision: d.decision,
            agentic: d.agentic,
            title: trim(&d.title),
            priority: d.priority,
            labels: d.labels.clone(),
            duplicate_of: trim(&d.duplicate_of),
            note: trim(&d.note),
        }
    }

    /// The answer the user's click in the Inbox would give: the choice, the edits and the reason.
    pub fn answer(&self) -> InboxAnswer {
        InboxAnswer {
            choice: Some(self.decision.choice().into()),
            text: self.note.clone(),
            proposal: Some(ProposalAnswer {
                title: self.title.clone(),
                priority: self.priority,
                labels: self.labels.clone(),
                agentic: self.agentic,
                duplicate_of: self.duplicate_of.clone(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

/// The rows the user kept of `n` (0-based, in order; every row when they sent none). Err when they kept none or named
/// one that isn't there.
pub fn kept(n: usize, rows: Option<&[u32]>) -> Result<Vec<usize>, String> {
    let Some(rows) = rows else { return Ok((0..n).collect()) };
    let mut out: Vec<usize> = vec![];
    for &r in rows {
        let r = r as usize;
        if r >= n {
            return Err(format!("there is no row {} (the call has {n})", r + 1));
        }
        if !out.contains(&r) {
            out.push(r);
        }
    }
    if out.is_empty() {
        return Err("tick at least one row, or Deny the call".into());
    }
    out.sort_unstable();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(v: serde_json::Value) -> InboxDecide {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_call_takes_each_decision_with_what_it_needs() {
        let ok = call(json!({"items": [
            {"item": "PDT-1", "decision": "accept_todo", "agentic": true, "title": "Better", "priority": "high",
             "labels": ["bug"]},
            {"item": "PDT-2", "decision": "accept_backlog"},
            {"item": "PDT-3", "decision": "duplicate", "duplicateOf": "PDT-9", "note": "same as PDT-9"},
            {"item": "PDT-4", "decision": "reject", "note": "not needed"},
        ]}));
        assert_eq!(check(&ok), Ok(()));
        let bad = |v: serde_json::Value, says: &str| {
            let e = check(&call(v)).unwrap_err();
            assert!(e.contains(says), "{e}");
        };
        bad(json!({"items": []}), "at least one");
        bad(json!({"items": [{"item": " ", "decision": "accept_todo"}]}), "name the Inbox item");
        bad(json!({"items": [{"item": "PDT-1", "decision": "reject"}]}), "reason");
        bad(json!({"items": [{"item": "PDT-1", "decision": "reject", "note": "  "}]}), "reason");
        bad(json!({"items": [{"item": "PDT-1", "decision": "duplicate"}]}), "duplicateOf");
        bad(json!({"items": [{"item": "PDT-1", "decision": "reject", "note": "n", "agentic": true}]}), "accept");
        bad(
            json!({"items": [{"item": "PDT-1", "decision": "duplicate", "duplicateOf": "PDT-2", "title": "t"}]}),
            "accept",
        );
        bad(json!({"items": [{"item": "PDT-1", "decision": "accept_todo", "duplicateOf": "PDT-2"}]}), "duplicate only");
        bad(json!({"items": [{"item": "PDT-1", "decision": "accept_todo", "title": ""}]}), "title is empty");
        let many: Vec<_> =
            (0..=MAX_ROWS).map(|n| json!({"item": format!("PDT-{n}"), "decision": "accept_todo"})).collect();
        bad(json!({"items": many}), "at most 100");
        let e = check(&call(json!({"items": [{"item": "PDT-1", "decision": "accept_todo"},
                                             {"item": "PDT-2", "decision": "reject"}]})))
        .unwrap_err();
        assert!(e.starts_with("item 2 (PDT-2)"), "names the row: {e}");
    }

    #[test]
    fn unknown_fields_and_decisions_are_refused() {
        assert!(
            serde_json::from_value::<InboxDecide>(json!({"items": [{"item": "A-1", "decision": "maybe"}]})).is_err()
        );
        assert!(
            serde_json::from_value::<InboxDecide>(json!({"items": [{"item": "A-1", "decision": "reject", "x": 1}]}))
                .is_err()
        );
    }

    fn row(d: serde_json::Value, item: &str) -> DecideRow {
        let d: ItemDecision = serde_json::from_value(d).unwrap();
        DecideRow::new(&d, item, &format!("t-{item}"), &format!("PDT-{item}"), "Proposed")
    }

    #[test]
    fn a_row_answers_as_the_inbox_click_would() {
        let r = row(
            json!({"item": "x", "decision": "accept_todo", "agentic": true, "title": " Better ", "priority": "high",
                           "labels": ["bug"], "note": " fits "}),
            "1",
        );
        assert_eq!((r.title.as_deref(), r.note.as_deref()), (Some("Better"), Some("fits")));
        let a = r.answer();
        assert_eq!((a.choice.as_deref(), a.text.as_deref()), (Some(inbox::ACCEPT_TODO), Some("fits")));
        let p = a.proposal.unwrap();
        assert!(p.agentic);
        assert_eq!(
            (p.title.as_deref(), p.priority, p.labels),
            (Some("Better"), Some(Priority::High), Some(vec!["bug".into()]))
        );
        let d = row(json!({"item": "x", "decision": "duplicate", "duplicateOf": "PDT-9"}), "2").answer();
        assert_eq!(d.choice.as_deref(), Some(inbox::DUPLICATE));
        assert_eq!(d.proposal.unwrap().duplicate_of.as_deref(), Some("PDT-9"));
        let j = row(json!({"item": "x", "decision": "reject", "note": "no"}), "3").answer();
        assert_eq!((j.choice.as_deref(), j.text.as_deref()), (Some(inbox::REJECT), Some("no")));
        let b = row(json!({"item": "x", "decision": "accept_backlog"}), "4").answer();
        assert_eq!((b.choice.as_deref(), b.text), (Some(inbox::ACCEPT_BACKLOG), None));
        assert!(!b.proposal.unwrap().agentic, "agentic only when asked");
    }

    #[test]
    fn each_item_goes_in_once() {
        let a = row(json!({"item": "x", "decision": "accept_todo"}), "1");
        let b = row(json!({"item": "y", "decision": "accept_todo"}), "2");
        assert_eq!(check_unique(&[a.clone(), b.clone()]), Ok(()));
        let e = check_unique(&[a.clone(), b, a]).unwrap_err();
        assert!(e.contains("PDT-1") && e.contains("twice"), "{e}");
    }

    #[test]
    fn the_user_keeps_the_rows_they_left_ticked() {
        assert_eq!(kept(3, None), Ok(vec![0, 1, 2]));
        assert_eq!(kept(3, Some(&[2, 0, 2])), Ok(vec![0, 2]));
        assert!(kept(3, Some(&[])).unwrap_err().contains("Deny"));
        assert!(kept(3, Some(&[3])).unwrap_err().contains("no row 4"));
    }

    #[test]
    fn rows_serialize_for_the_confirmation() {
        let v = serde_json::to_value(row(json!({"item": "x", "decision": "reject", "note": "no"}), "1")).unwrap();
        assert_eq!(
            (v["itemId"].as_str(), v["ticketKey"].as_str(), v["decision"].as_str()),
            (Some("1"), Some("PDT-1"), Some("reject"))
        );
        let back: DecideRow = serde_json::from_value(v).unwrap();
        assert_eq!(back.note.as_deref(), Some("no"));
    }
}
