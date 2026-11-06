//! Asking-tool calls that wait on their Inbox `confirm` item (PDT-492): a terminal session's call (no conversation to
//! show a card in) and the PM's `inbox_decide` table. The call blocks until the user answers, as an inline card's does
//! (`conversations::held`), so the agent never ends its turn to wait. [`answer_confirm`](super::answer_confirm) hands
//! the answer to the waiting call; one that is no longer waiting (the agent gave up on it, or PDT stopped it at
//! `AppState::ask_wait`, PDT-589) gets the old way: PDT runs the call and sends the session the result. The call's
//! side (`mcp::tools::asking`) turns the answer into its result, as it does an inline card's.

use crate::mcp::tools::Answered;
use crate::routes::AppState;
use pdt_core::inbox::{InboxItem, InboxStatus};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;

/// How often a waiting call checks that its item is still open (closed with its step, replaced, dismissed).
const CHECK: Duration = Duration::from_secs(2);

/// How long an asking tool's call waits for the answer (`AppState::ask_wait`): the agent's budget for PDT's tools
/// ([`pdt_agents::mcp::PDT_TOOL_TIMEOUT_SECS`]) less a margin for the result to reach it.
pub(crate) const ASK_WAIT: Duration = Duration::from_secs(pdt_agents::mcp::PDT_TOOL_TIMEOUT_SECS - 10 * 60);

/// The calls waiting on their items, by item id.
#[derive(Default)]
pub(crate) struct ConfirmWaits(Mutex<HashMap<String, oneshot::Sender<Answered>>>);

impl ConfirmWaits {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, oneshot::Sender<Answered>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Waits for the user's answer to `item`, which the calling session just opened (`Ended`: the item closed first).
pub(crate) async fn wait(st: &AppState, item: &InboxItem) -> Answered {
    let (tx, mut rx) = oneshot::channel();
    st.confirm_waits.map().insert(item.id.clone(), tx);
    // Dropped unanswered (the agent gave up on the call), the item stays for the old way.
    // Gone with it once the budget is up: the answer then takes the old way.
    let _gone = Unwait { st, id: &item.id };
    let until = tokio::time::Instant::now() + st.ask_wait;
    loop {
        tokio::select! {
            r = &mut rx => return r.unwrap_or(Answered::Ended),
            _ = tokio::time::sleep(CHECK) => {}
            _ = tokio::time::sleep_until(until) => {
                // Closed first, so an answer is either here now or sent nowhere (and takes the old way).
                rx.close();
                return rx.try_recv().unwrap_or(Answered::Waiting);
            }
        }
        // An item that ends without an answer (its step went away, a newer call replaced it) ends the wait. One being
        // answered keeps it: [`take`] removes the waiter before the item ends, and the answer follows.
        let open = matches!(st.store.get_inbox_item(&item.id).await, Ok(Some(i)) if i.status == InboxStatus::Open);
        if !open && st.confirm_waits.map().contains_key(&item.id) {
            return Answered::Ended;
        }
    }
}

/// The call waiting on an item, taken out before the item ends so the wait never mistakes the answer for the item
/// closing unanswered.
pub(crate) struct Waiter(oneshot::Sender<Answered>);

/// Takes the waiter of `item_id`, if a call waits on it.
pub(crate) fn take(st: &AppState, item_id: &str) -> Option<Waiter> {
    st.confirm_waits.map().remove(item_id).map(Waiter)
}

impl Waiter {
    /// Hands the call its answer: false when it stopped waiting meanwhile (it gets the old way).
    pub(crate) fn answer(self, a: Answered) -> bool {
        self.0.send(a).is_ok()
    }

    /// The item wasn't answered after all (someone else answered it first): the call waits on.
    pub(crate) fn put_back(self, st: &AppState, item_id: &str) {
        if !self.0.is_closed() {
            st.confirm_waits.map().insert(item_id.to_string(), self.0);
        }
    }
}

struct Unwait<'a> {
    st: &'a AppState,
    id: &'a str,
}

impl Drop for Unwait<'_> {
    fn drop(&mut self) {
        self.st.confirm_waits.map().remove(self.id);
    }
}
