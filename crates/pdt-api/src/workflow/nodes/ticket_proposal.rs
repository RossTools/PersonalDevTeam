//! `inbox.ticket_proposal` (PDT-366): a ticket an agent made waits in the user's Inbox (Ticket proposals), held in
//! Draft and off the board, for Accept to To do, Accept to backlog (each with the user's edits and the `agentic`
//! box), Duplicate of… (Won't do, linked to the ticket it repeats) or Reject (Won't do). The proposing agent hears the
//! decision in its session while that runs, else as a comment on the ticket it came from. The step does not wait:
//! the item closes by itself once the ticket leaves Draft some other way (the store does that on every move).

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::inbox::{self, InboxAnswer, InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
use pdt_core::session::{SessionKind, SessionStatus};
use pdt_core::ticket::{Actor, By, LinkKind, TicketPatch, TicketStage};
use pdt_core::ticket_proposal::{self as proposal, Decision};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const KIND: &str = "inbox.ticket_proposal";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct TicketProposalConfig {
    /// The proposed ticket (its key or id, `{{trigger.ticketId}}`). Empty: nothing to propose (`skipped`).
    #[serde(default)]
    pub ticket: Option<String>,
    /// Why the agent made it (`{{trigger.reason}}`), shown with it.
    #[serde(default)]
    #[schemars(extend("x-format" = "markdown"))]
    pub reason: Option<String>,
    /// Who proposed it: the persona (`{{trigger.persona}}`) and its session (`{{trigger.sessionId}}`), which hears
    /// the decision while it runs.
    #[serde(default)]
    pub persona: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
    /// The ticket that session was working on (`{{trigger.fromTicketId}}`): the decision goes on it as a comment when
    /// the session has ended.
    #[serde(default)]
    pub from_ticket: Option<String>,
}

pub struct TicketProposal;

fn skipped(reason: &str) -> Outcome {
    Outcome::next("skipped", json!({"quiet": true, "reason": reason}))
}

fn nonblank(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

impl NodeKind for TicketProposal {
    fn spec(&self) -> KindSpec {
        spec::<TicketProposalConfig, Value>(
            KIND,
            1,
            Category::Action,
            "Propose a ticket in the Inbox",
            "Puts a ticket an agent made in your Inbox (Ticket proposals), with its description, who proposed it, from \
             which ticket and why. It stays in Draft, off the board, until you decide: Accept to To do or Accept to \
             backlog (Ready), with your edits and the agentic box; Duplicate of… (Won't do, linked to the ticket it \
             repeats); or Reject (Won't do). The agent hears the decision in its session if it still runs, else as a \
             comment on the ticket it came from. Once per ticket; `skipped` when the ticket has left Draft or no \
             ticket is given. The step does not wait: the item closes by itself when the ticket leaves Draft some \
             other way.",
            &[("added", "In the Inbox"), ("skipped", "Not proposed")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<TicketProposalConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: TicketProposalConfig = parse(&input)?;
            let Some(given) = nonblank(&c.ticket) else { return Ok(skipped("no ticket to propose")) };
            if cx.dry_run && pdt_core::workflow::dry::is_stand_in(given) {
                return Ok(Outcome::next("added", json!({"would": format!("propose {given} in your Inbox")})));
            }
            let id = crate::mcp::tools::resolve_ticket(cx.st, given, cx.run.project_id.as_deref()).await?;
            let store = &cx.st.store;
            let t = store.get_ticket(&id).await.map_err(crate::workflow::err)?.ok_or("no such ticket")?;
            if t.summary.stage != TicketStage::Draft {
                return Ok(skipped("the ticket has left Draft"));
            }
            if cx.dry_run {
                return Ok(Outcome::next(
                    "added",
                    json!({"would": format!("propose {} in your Inbox", t.summary.key)}),
                ));
            }
            let from_id = nonblank(&c.from_ticket).map(String::from);
            let from_key = match &from_id {
                Some(f) => store.get_ticket_summary(f).await.map_err(crate::workflow::err)?.map(|s| s.key),
                None => None,
            };
            let s = &t.summary;
            let body = json!({
                "description": t.description, "priority": s.priority, "labels": s.labels,
                "reason": nonblank(&c.reason), "persona": nonblank(&c.persona),
                "fromTicket": from_key, "fromTicketId": from_id, "workflow": cx.workflow.name,
            });
            let item = cx
                .inbox_item(InboxKind::TicketProposal, InboxGroup::TicketProposal, &s.title, body)
                .project(s.project_id.clone())
                .ticket(id.clone())
                .session(nonblank(&c.session).map(String::from))
                // Not the step's: the item outlives the run.
                .step(None)
                .choices(vec![
                    InboxChoice::new(inbox::ACCEPT_TODO, "Accept to Todo", true),
                    InboxChoice::new(inbox::ACCEPT_BACKLOG, "Accept to backlog", false),
                    InboxChoice::new(inbox::DUPLICATE, "Duplicate of…", false),
                    InboxChoice::new(inbox::REJECT, "Reject", false),
                ]);
            match store
                .add_inbox_item_once(&item, &format!("ticket-proposal:{id}"))
                .await
                .map_err(crate::workflow::err)?
            {
                Some(i) => {
                    crate::notifications::kick(cx.st);
                    Ok(Outcome::next("added", json!({"itemId": i.id})))
                }
                None => Ok(skipped("already in the Inbox")),
            }
        })
    }
}

/// Whether tickets agents make are held as proposals now: the "Ticket proposals" built-in is on.
pub(crate) async fn proposals_on(st: &AppState) -> bool {
    let w = st.store.workflow_by_builtin(crate::workflow::builtins::TICKET_PROPOSALS).await;
    matches!(w, Ok(Some(w)) if w.enabled)
}

/// The user's decision on a ticket proposal. `from_ui`: it came from PDT's web client; an agent's curl never decides
/// one (it could accept its own).
pub(crate) async fn answer(st: &AppState, item: &InboxItem, a: &InboxAnswer, from_ui: bool) -> Result<Value, String> {
    if !from_ui {
        return Err("only the user decides a ticket proposal, from PDT's Inbox".into());
    }
    decide(st, item, a, Actor::User.into()).await
}

/// Applies the user's decision `a` on a ticket proposal's item, as `by`: the user's click, or the user's Allow of the
/// PM's `inbox_decide` ([`By::user_via`], PDT-475), whose events and resolution keep the PM as `proposedBy`. Never
/// reached from an agent's own request.
pub(crate) async fn decide(st: &AppState, item: &InboxItem, a: &InboxAnswer, by: By) -> Result<Value, String> {
    if item.kind != InboxKind::TicketProposal {
        return Err("that item is not a ticket proposal".into());
    }
    let store = &st.store;
    let id = item.ticket_id.as_deref().ok_or("the item has no ticket")?;
    let t = store.get_ticket_summary(id).await.map_err(crate::workflow::err)?.ok_or("the ticket is gone")?;
    let p = a.proposal.clone().unwrap_or_default();
    let note = a.text.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let mut duplicate_of = None;
    let decision = match a.choice.as_deref() {
        Some(inbox::ACCEPT_TODO) => Decision::Todo { agentic: p.agentic },
        Some(inbox::ACCEPT_BACKLOG) => Decision::Backlog { agentic: p.agentic },
        Some(inbox::DUPLICATE) => {
            let given = nonblank(&p.duplicate_of).ok_or("pick the ticket it is a duplicate of")?;
            let other = crate::mcp::tools::resolve_ticket(st, given, Some(&t.project_id)).await?;
            if other == id {
                return Err("a ticket can't be a duplicate of itself".into());
            }
            let key =
                store.get_ticket_summary(&other).await.map_err(crate::workflow::err)?.ok_or("no such ticket")?.key;
            duplicate_of = Some(other);
            Decision::Duplicate { of: key }
        }
        Some(inbox::REJECT) => Decision::Rejected,
        _ => return Err("choose Accept to Todo, Accept to backlog, Duplicate of… or Reject".into()),
    };
    // The edits are checked before the item is answered.
    let accept = match &decision {
        Decision::Todo { agentic } | Decision::Backlog { agentic } => Some(
            TicketPatch {
                title: p.title.clone(),
                description: p.description.clone(),
                stage: Some(decision.stage()),
                priority: p.priority,
                labels: Some(proposal::accepted_labels(p.labels.as_deref().unwrap_or(&t.labels), *agentic)),
                ..Default::default()
            }
            .validate()
            .map_err(crate::workflow::err)?,
        ),
        _ => None,
    };
    let mut resolution = json!({"choice": a.choice, "note": note, "duplicateOf": duplicate_of});
    if let Some(p) = &by.proposed_by {
        resolution["proposedBy"] = json!(p);
    }
    if !store
        .end_inbox_item(&item.id, InboxStatus::Resolved, &resolution, now_ms())
        .await
        .map_err(crate::workflow::err)?
    {
        return Err("this item was already answered".into());
    }
    let to = decision.stage();
    let ticket = match accept {
        Some(patch) => store.patch_ticket_by(id, &patch, &by, now_ms()).await,
        None => {
            if let Some(other) = &duplicate_of {
                store.add_link(id, other, LinkKind::Relates, now_ms()).await.map_err(crate::workflow::err)?;
            }
            let since = now_ms();
            let moved = store.move_ticket_by(id, to, &by, Some(&json!({"via": "inbox"})), since).await;
            if moved.as_ref().is_ok_and(|m| m.is_some()) {
                crate::start::close_attempt(st, id, to, since, Actor::User, None)
                    .await
                    .map_err(|e| e.message().to_string())?;
            }
            moved
        }
    };
    let ticket = ticket.map_err(crate::workflow::err)?.ok_or("the ticket is gone")?;
    tell(st, item, &proposal::decision_text(&t.key, &ticket.summary.title, &decision, note)).await;
    Ok(json!({"stage": to, "key": t.key}))
}

/// Tells the proposing agent: in its session while it runs, else as a comment on the ticket it came from. Never
/// fails; problems are logged.
async fn tell(st: &AppState, item: &InboxItem, text: &str) {
    let session = match item.session_id.as_deref() {
        Some(s) => st.store.get_session(s).await.ok().flatten(),
        None => None,
    };
    if let Some(s) = session.filter(|s| s.kind == SessionKind::AgentTui && s.status == SessionStatus::Running) {
        let m = pdt_store::SessionMessage {
            id: uuid::Uuid::now_v7().to_string(),
            attempt_id: None,
            session_id: Some(s.id.clone()),
            ticket_id: s.ticket.as_ref().map(|t| t.id.clone()),
            dedupe: Some(format!("ticket-proposal:{}", item.id)),
            text: format!("[{}] {text}", pdt_core::brand::SHORT_NAME),
            timeline: Value::Null,
            run_id: None,
            step_id: None,
            status: "waiting".into(),
            told_session: None,
            created_at: now_ms(),
            resolved_at: None,
            by_user: false,
            pending_id: None,
            reopens: false,
        };
        match st.store.queue_session_message(&m).await {
            Ok(_) => return,
            Err(e) => tracing::warn!(session = %s.id, error = %e, "could not tell an agent about its ticket proposal"),
        }
    }
    let Some(from) = item.body.get("fromTicketId").and_then(Value::as_str) else { return };
    let data = json!({"ticketProposal": item.ticket_id});
    if let Err(e) = st.store.add_comment_by(from, text, Actor::System, Some(data), now_ms()).await {
        tracing::warn!(ticket = %from, error = %e, "could not comment a ticket proposal's decision");
    }
}
