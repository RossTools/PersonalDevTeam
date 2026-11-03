//! Asking-tool calls that wait on their Inbox `confirm` item (PDT-492): a terminal session's call (no conversation to
//! show a card in) and the PM's `inbox_decide` table. The call blocks until the user answers, as an inline card's does
//! (`conversations::held`), so the agent never ends its turn to wait. [`answer_confirm`](super::answer_confirm) hands
//! the answer to the waiting call; one that is no longer waiting (the agent gave up on it) gets the old way: PDT runs
//! the call and sends the session the result.

use crate::routes::AppState;
use pdt_core::inbox::{InboxItem, InboxStatus};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;

/// How often a waiting call checks that its item is still open (closed with its step, replaced, dismissed).
const CHECK: Duration = Duration::from_secs(2);

/// The user's answer to a waiting call's item.
#[derive(Debug, PartialEq)]
pub(crate) enum Confirmed {
    /// Allowed, with the rows of a batch call the user kept (0-based; every row when None).
    Allowed(Option<Vec<u32>>),
    Denied,
    /// The item closed before the user answered.
    Ended,
}

/// The calls waiting on their items, by item id.
#[derive(Default)]
pub(crate) struct ConfirmWaits(Mutex<HashMap<String, oneshot::Sender<Confirmed>>>);

impl ConfirmWaits {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, oneshot::Sender<Confirmed>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Waits for the user's answer to `item`, which the calling session just opened.
pub(crate) async fn wait(st: &AppState, item: &InboxItem) -> Confirmed {
    let (tx, mut rx) = oneshot::channel();
    st.confirm_waits.map().insert(item.id.clone(), tx);
    // Dropped unanswered (the agent gave up on the call), the item stays for the old way.
    let _gone = Unwait { st, id: &item.id };
    loop {
        tokio::select! {
            r = &mut rx => return r.unwrap_or(Confirmed::Ended),
            _ = tokio::time::sleep(CHECK) => {}
        }
        // An item that ends without an answer (its step went away, a newer call replaced it) ends the wait; one being
        // answered has already taken its waiter.
        let open = matches!(st.store.get_inbox_item(&item.id).await, Ok(Some(i)) if i.status == InboxStatus::Open);
        if !open && st.confirm_waits.map().contains_key(&item.id) {
            return Confirmed::Ended;
        }
    }
}

/// Hands the answer to the call waiting on `item_id`: false when none is (it gets the old way).
pub(crate) fn answer(st: &AppState, item_id: &str, c: Confirmed) -> bool {
    let tx = st.confirm_waits.map().remove(item_id);
    tx.is_some_and(|tx| tx.send(c).is_ok())
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

/// The Allow for `tool` on a waiting call is the user's yes to that very call (PDT-201), as an inline card's is.
pub(crate) fn grant(st: &AppState, session_id: &str, item_id: &str, tool: &str, input: &serde_json::Value) {
    st.grants.hold(session_id, item_id, tool, input);
    st.grants.answered(session_id, item_id, true);
}
