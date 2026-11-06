//! Asking the user about an asking tool's call (PDT-526): an approval card in the session's conversation
//! (`conversations::held`, PDT-490), else an Inbox item (`workflow::agent::confirm_wait`, PDT-492). Either way the call
//! waits for the answer, and this is the one place that turns it into the call's result: the notifications around the
//! wait, the user's yes recorded for the call (PDT-201) and the words the agent reads.

use super::Cx;
use crate::routes::AppState;
use pdt_core::inbox_decide::DecideRow;
use serde_json::json;

/// How the user answered PDT's ask about a call, from the card or the Inbox item.
#[derive(Debug, PartialEq)]
pub(crate) enum Answered {
    /// With the rows the user kept, for a call with rows (0-based; every row when None).
    Allowed(Option<Vec<u32>>),
    /// With the user's reason, if they gave one.
    Denied(Option<String>),
    /// The ask ended before the user answered: the turn or the agent ended (a card), or the item closed.
    Ended,
    /// The user hasn't answered within `AppState::ask_wait` (PDT-589): the ask stays open, and once they answer PDT
    /// runs the call (or not) and messages the session.
    Waiting,
}

/// How to ask about a call.
pub(crate) struct Ask<'a> {
    /// Ask with a card in the session's conversation when it is one; false: always with an Inbox item.
    pub(crate) inline: bool,
    /// The card offers "Always allow in this session", for what it would cover (`pm::ask_scope`, PDT-539).
    pub(crate) always: Option<&'a str>,
    /// The user's yes is kept for this very call: only when it lets an external ticket in, the one call that checks
    /// for it (`edit_ticket`, PDT-201, PDT-539).
    pub(crate) grant: bool,
    /// A batch call's rows, which the user can untick (the PM's `inbox_decide`, PDT-502).
    pub(crate) rows: Option<Vec<DecideRow>>,
    /// What a Deny leaves, for the agent: "PDT didn't make it", "nothing changed".
    pub(crate) unchanged: &'a str,
}

/// Asks the user about this call of `tool` and waits for their answer: Ok when they allowed it, with the rows they
/// kept for a call with rows; Err is what the agent reads.
pub(crate) async fn ask_for_call(cx: &Cx<'_>, tool: &str, ask: Ask<'_>) -> Result<Option<Vec<usize>>, String> {
    let (st, s) = (cx.st, cx.session);
    let count = ask.rows.as_ref().map(Vec::len);
    let inline = match ask.inline {
        true => crate::conversations::ask_inline(st, s, tool, cx.input, ask.always, ask.rows.clone()).await,
        false => None,
    };
    let (answered, ended) = match inline {
        Some(a) => (a, "the turn ended"),
        None => {
            let rows = ask.rows.map(|r| json!(r)).unwrap_or_default();
            let item = crate::workflow::agent::ask_confirm(st, s, tool, cx.input, rows).await?;
            crate::notifications::kick(st);
            let a = crate::workflow::agent::wait_confirm(st, &item).await;
            crate::notifications::kick(st);
            (a, "its Inbox item closed")
        }
    };
    match answered {
        Answered::Allowed(kept) => {
            // The user's own yes is good for this very call (PDT-201), as for an agent's prompt.
            if ask.grant {
                st.grants.allow(&s.id, tool, cx.input);
            }
            count.map(|n| pdt_core::inbox_decide::kept(n, kept.as_deref())).transpose()
        }
        Answered::Denied(why) => Err(format!("The user denied this {tool} call, so {}.{}", ask.unchanged, said(why))),
        Answered::Ended => Err(format!("{tool} was not run: {ended} before the user answered.")),
        Answered::Waiting => Err(still_waiting(tool)),
    }
}

/// The user's reason for a Deny, as the agent reads it after the denial.
pub(crate) fn said(why: Option<String>) -> String {
    why.map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).map(|w| format!(" They said: {w}")).unwrap_or_default()
}

/// What a call that waited `AppState::ask_wait` for the user returns (PDT-589), in place of the agent's own timeout:
/// the ask stays open, and the answer comes as a message ([`crate::workflow::agent::answer_confirm`],
/// `conversations::held`).
pub(crate) fn still_waiting(tool: &str) -> String {
    format!(
        "Still waiting for the user's answer to this {tool} call in PDT's Inbox. PDT will message you the result \
         once they answer. Don't call {tool} again for this: carry on with other work, or end your turn."
    )
}

/// The title the user sees for `tool` on its card or Inbox item: its own, else its name.
pub(crate) fn tool_title(st: &AppState, tool: &str) -> String {
    st.registry()
        .tool_infos()
        .into_iter()
        .find(|t| t.name == tool)
        .map(|t| t.title)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| tool.to_string())
}
