//! Gaps in a conversation's stored log (PDT-331): events PDT received but never stored (its writes failed and pdtd
//! stopped before they could be retried), found again in the agent's own record of the conversation (Claude's
//! transcript) by their item ids.

use super::{AgentEvent, MessageSource};
use std::collections::{HashMap, HashSet};

/// The kinds of event a gap is filled with.
const REPAIRED: [&str; 4] = ["agent_message", "user_message", "tool_call", "reasoning"];

/// One event of the agent's record: its item id, turn id and event.
pub type Recorded = (Option<String>, Option<String>, AgentEvent);

/// Events missing from the log, all of them from between two stored items.
#[derive(Debug, Clone, PartialEq)]
pub struct Gap {
    /// The stored item they come right after.
    pub after: String,
    /// In conversation order: (item id, turn id, event).
    pub events: Vec<(String, Option<String>, AgentEvent)>,
}

/// The gaps in a log that has the items `stored`, against the agent's record `history` (in conversation order; an item
/// that changes appears once per version). Only items between two stored ones count: what comes before the log's first
/// item was never PDT's to log, and what comes after its last may still be on its way. A message the record has as
/// typed in the terminal was sent from PDT's composer (a gap only happens where PDT was listening). Only messages,
/// tool calls and reasoning are filled in: turn markers and notices of turns the agent started on its own (a background
/// task's report) have other ids live than in the record, and a missing one shows nothing wrong.
pub fn gaps(history: &[Recorded], stored: &HashSet<String>) -> Vec<Gap> {
    // Each item once, where it first appears, as its last version.
    let mut order: Vec<String> = Vec::new();
    let mut latest: HashMap<String, (Option<String>, AgentEvent)> = HashMap::new();
    for (item, turn, event) in history {
        let Some(item) = item else { continue };
        match latest.get_mut(item) {
            Some((t, e)) => {
                *e = event.clone();
                if turn.is_some() {
                    t.clone_from(turn);
                }
            }
            None => {
                order.push(item.clone());
                latest.insert(item.clone(), (turn.clone(), event.clone()));
            }
        }
    }
    let Some(last) = order.iter().rposition(|i| stored.contains(i)) else { return Vec::new() };
    let mut out: Vec<Gap> = Vec::new();
    let mut prev: Option<&String> = None;
    let mut open = false;
    for item in &order[..last] {
        if stored.contains(item) {
            prev = Some(item);
            open = false;
            continue;
        }
        let Some(after) = prev else { continue };
        if !REPAIRED.contains(&latest[item].1.kind()) {
            continue;
        }
        let (turn, mut event) = latest.remove(item).expect("every item has a version");
        if let AgentEvent::UserMessage { source, .. } = &mut event
            && *source == MessageSource::Terminal
        {
            *source = MessageSource::Pdt;
        }
        if !open {
            out.push(Gap { after: after.clone(), events: Vec::new() });
            open = true;
        }
        out.last_mut().expect("just pushed").events.push((item.clone(), turn, event));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::TurnPhase;
    use super::*;

    fn said(text: &str) -> AgentEvent {
        AgentEvent::AgentMessage { id: text.into(), markdown: text.into(), phase: super::super::MessagePhase::Final }
    }

    fn turn(phase: TurnPhase) -> AgentEvent {
        AgentEvent::Turn { phase, status: None, duration_ms: None, reason: None }
    }

    fn rec(item: &str, e: AgentEvent) -> Recorded {
        (Some(item.into()), Some("t".into()), e)
    }

    fn stored(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finds_the_items_between_stored_ones() {
        let history = vec![
            rec("m:0", said("before the log")),
            rec("m:1", said("one")),
            rec("m:2", said("lost")),
            rec("m:3", said("lost too")),
            rec("m:4", said("four")),
            rec("m:5", said("lost again")),
            rec("m:6", said("six")),
            rec("m:7", said("still coming")),
        ];
        let g = gaps(&history, &stored(&["m:1", "m:4", "m:6"]));
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].after, "m:1");
        let items: Vec<&str> = g[0].events.iter().map(|(i, _, _)| i.as_str()).collect();
        assert_eq!(items, ["m:2", "m:3"]);
        assert_eq!((g[1].after.as_str(), g[1].events.len()), ("m:4", 1));
        assert!(gaps(&history, &stored(&[])).is_empty(), "nothing stored: nothing to line up");
        assert!(gaps(&history, &stored(&["m:1", "m:2", "m:3", "m:4", "m:5", "m:6"])).is_empty());
    }

    #[test]
    fn an_item_counts_once_as_its_last_version_and_a_lost_prompt_was_sent_from_pdt() {
        let prompt =
            AgentEvent::UserMessage { text: "hi".into(), images: vec![], source: MessageSource::Terminal, pdt: None };
        let history = vec![
            rec("turn:1", turn(TurnPhase::Started)),
            rec("u:1", prompt),
            rec("m:1", said("draft")),
            rec("m:1", said("final")),
            rec("turn-end:1", turn(TurnPhase::Ended)),
            rec("turn:2", turn(TurnPhase::Started)),
            rec("turn-end:2", turn(TurnPhase::Ended)),
            rec("turn:3", turn(TurnPhase::Started)),
        ];
        let g = gaps(&history, &stored(&["turn:1", "turn-end:1", "turn:3"]));
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].after, "turn:1");
        assert!(matches!(&g[0].events[0].2, AgentEvent::UserMessage { source: MessageSource::Pdt, .. }));
        assert_eq!(g[0].events[1].2, said("final"));
        assert_eq!(g[0].events.len(), 2, "turn markers are not filled in");
    }
}
