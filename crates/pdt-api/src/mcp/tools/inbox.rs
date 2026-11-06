//! `inbox_decide` (PDT-475): the PM proposes decisions on Inbox items (ticket proposals), many in one call, and the user
//! confirms them, for every agent and whatever the project's policy. In the PM's conversation the call asks there, with
//! a card listing every row (PDT-502), which is also in the Inbox; elsewhere (a terminal) it asks with an Inbox item
//! (PDT-492). Either way the user unticks the rows they don't want and allows the rest, and the call waits for their
//! answer. Their Allow (only from PDT's web client) resolves each kept row as the user's own decision, proposed by the
//! PM. A call that gave up waiting on its Inbox item runs again from the Allow (`cx.confirmed`), and the PM hears the
//! result in a message.

use super::*;
use pdt_core::inbox::{InboxItem, InboxKind, InboxStatus};
use pdt_core::inbox_decide::{self as decide, DecideRow, InboxDecide, ItemDecision};

const NAME: &str = "inbox_decide";

pub(super) fn defs() -> Vec<Tool> {
    vec![Tool {
        decl: ToolDecl::new(NAME, "Decide Inbox items (ticket proposals), confirmed by the user").asks_in_inbox().pm(),
        title: "Decide Inbox items",
        description: "Proposes decisions on ticket proposals waiting in the user's Inbox, many in one call (at most \
                      100): `accept_todo` or `accept_backlog` (with `agentic` and optional title, priority and \
                      labels), `duplicate` (with `duplicateOf`, the ticket it repeats) or `reject` (with the reason in \
                      `note`). Name each by its ticket's key. Nothing changes until the user confirms: the call always \
                      waits for them, even where the project lets you act, with one table in your conversation (also \
                      in their Inbox) where they can untick rows. Their Allow resolves the rows they kept as their own decisions, and the call returns \
                      what it did (`decided`, `failed`, `unticked`); a Deny returns an error. The PM's only.",
        schema: schema::<InboxDecide>,
    }]
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    match name {
        NAME => run(cx, args(a)?).await,
        other => Err(format!("no tool is called {other}")),
    }
}

/// The open ticket proposal `d` names (its item's id, or its ticket's key or id), as its confirmation row.
async fn find(st: &AppState, open: &[InboxItem], d: &ItemDecision) -> Result<(DecideRow, InboxItem), String> {
    let given = d.item.trim();
    let item = match open.iter().find(|i| i.id == given) {
        Some(i) => i.clone(),
        None => {
            let ticket = resolve_ticket(st, given, None).await?;
            open.iter()
                .find(|i| i.ticket_id.as_deref() == Some(ticket.as_str()))
                .cloned()
                .ok_or_else(|| format!("{given} is not a ticket proposal waiting in the Inbox"))?
        }
    };
    let ticket_id = item.ticket_id.clone().ok_or("the item has no ticket")?;
    let t = st.store.get_ticket_summary(&ticket_id).await.map_err(store)?.ok_or("the ticket is gone")?;
    Ok((DecideRow::new(d, &item.id, &ticket_id, &t.key, &t.title), item))
}

async fn run(cx: &Cx<'_>, mut a: InboxDecide) -> Result<Value, String> {
    let st = cx.st;
    // Only the PM: a Coder, QA, Reviewer or workflow agent never decides proposals, its own or anyone's.
    if !crate::pm::is_pm_session(st, cx.session).await {
        return Err(format!("{NAME} is the project manager's tool; this session can't decide Inbox items"));
    }
    decide::check(&a)?;
    if cx.confirmed {
        return apply(cx, &a).await;
    }
    let rows: Vec<DecideRow> = found(st, &a).await.into_iter().map(|f| f.map(|(r, _)| r)).collect::<Result<_, _>>()?;
    decide::check_unique(&rows)?;
    // The call waits on the table (in the conversation, else in the Inbox): the user's Allow applies the rows they
    // kept, in this same call.
    let ask = Ask { inline: true, can_always: false, rows: Some(rows.clone()), unchanged: "nothing changed" };
    let kept = ask_for_call(cx, NAME, ask).await?.unwrap_or_default();
    let unticked: Vec<&str> =
        rows.iter().enumerate().filter(|(n, _)| !kept.contains(n)).map(|(_, r)| r.ticket_key.as_str()).collect();
    a.items = a.items.into_iter().enumerate().filter(|(n, _)| kept.contains(n)).map(|(_, d)| d).collect();
    let mut out = apply(cx, &a).await?;
    out["unticked"] = json!(unticked);
    Ok(out)
}

/// Each decision's row and open item, or why it has none.
async fn found(st: &AppState, a: &InboxDecide) -> Vec<Result<(DecideRow, InboxItem), String>> {
    let open: Vec<InboxItem> = match st.store.open_inbox_items_of(InboxKind::TicketProposal).await {
        Ok(all) => all.into_iter().filter(|i| i.status == InboxStatus::Open).collect(),
        Err(e) => return vec![Err(store(e))],
    };
    let mut found = vec![];
    for d in &a.items {
        found.push(find(st, &open, d).await.map_err(|e| format!("{}: {e}", d.item.trim())));
    }
    found
}

/// The user's Allow: each row they kept is their decision, proposed by the PM.
async fn apply(cx: &Cx<'_>, a: &InboxDecide) -> Result<Value, String> {
    let st = cx.st;
    let pm = agent_by(st, cx.session).await.agent.ok_or("the PM session is unknown")?;
    let by = By::user_via(pm);
    let (mut decided, mut failed) = (vec![], vec![]);
    for f in found(st, a).await {
        let (row, item) = match f {
            Ok(x) => x,
            Err(e) => {
                failed.push(json!({"error": e}));
                continue;
            }
        };
        let answer = row.answer();
        match crate::workflow::nodes::ticket_proposal::decide(st, &item, &answer, by.clone()).await {
            Ok(v) => {
                crate::workflow::sensors::inbox_answered(
                    st,
                    &item,
                    answer.choice.as_deref(),
                    row.note.as_deref(),
                    "user",
                )
                .await;
                decided.push(json!({"ticket": row.ticket_key, "decision": row.decision, "stage": v.get("stage")}));
            }
            Err(e) => failed.push(json!({"ticket": row.ticket_key, "error": e})),
        }
    }
    crate::notifications::kick(st);
    Ok(json!({"decided": decided, "failed": failed}))
}
