//! `inbox_decide` (PDT-475): the PM proposes decisions on Inbox items (ticket proposals), many in one call. The call
//! always waits in the Inbox as one item listing every row, for every agent and whatever the project's policy; the
//! user unticks the rows they don't want and allows the rest. Their Allow runs it here again (`cx.confirmed`, only
//! from PDT's web client), and each kept row resolves as the user's own decision, proposed by the PM.

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
                      waits in their Inbox as one table, even where the project lets you act, and they can untick \
                      rows. Their Allow resolves the rows they kept as their own decisions; you get a message with \
                      the result. The PM's only.",
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

async fn run(cx: &Cx<'_>, a: InboxDecide) -> Result<Value, String> {
    let st = cx.st;
    // Only the PM: a Coder, QA, Reviewer or workflow agent never decides proposals, its own or anyone's.
    if !crate::pm::is_pm_session(st, cx.session).await {
        return Err(format!("{NAME} is the project manager's tool; this session can't decide Inbox items"));
    }
    decide::check(&a)?;
    let open: Vec<InboxItem> = st
        .store
        .open_inbox_items_of(InboxKind::TicketProposal)
        .await
        .map_err(store)?
        .into_iter()
        .filter(|i| i.status == InboxStatus::Open)
        .collect();
    let mut found = vec![];
    for d in &a.items {
        found.push(find(st, &open, d).await.map_err(|e| format!("{}: {e}", d.item.trim())));
    }
    if !cx.confirmed {
        let rows: Vec<DecideRow> = found.into_iter().map(|f| f.map(|(r, _)| r)).collect::<Result<_, _>>()?;
        decide::check_unique(&rows)?;
        let n = rows.len();
        let item = crate::workflow::agent::ask_confirm_rows(st, cx.session, NAME, cx.input, json!(rows)).await?;
        crate::notifications::kick(st);
        return Ok(json!({
            "waiting": true,
            "itemId": item.id,
            "message": format!(
                "Nothing has changed yet: PDT put the {n} decision{} in the user's Inbox (item {}) for them to confirm, \
                 unticking any they don't want. Don't call {NAME} again for these: when they allow it, PDT applies \
                 the rows they kept and sends you a message with the result; if they deny it, you hear that instead. \
                 End your turn now.",
                if n == 1 { "" } else { "s" },
                item.id
            ),
        }));
    }
    // The user's Allow: each row they kept is their decision, proposed by the PM.
    let pm = agent_by(st, cx.session).await.agent.ok_or("the PM session is unknown")?;
    let by = By::user_via(pm);
    let (mut decided, mut failed) = (vec![], vec![]);
    for f in found {
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
