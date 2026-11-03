//! `inbox.admit` (PDT-225): a ticket triage drafted waits in the user's Inbox for Approve (to To do, as the user's own
//! move: it lets an external ticket in, PDT-201) or Reject (to Won't do, where "Done closes the GitHub issue" closes a
//! linked issue as not planned). Once per ticket, unless the project's `triageApproval` is `off`. The item closes by
//! itself once the ticket moves past Ready some other way (the store does that on every move).

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, parses, spec};
use pdt_core::inbox::{self, InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
use pdt_core::policy::TriageApproval;
use pdt_core::ticket::{Actor, TicketStage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const KIND: &str = "inbox.admit";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct InboxAdmitConfig {
    /// The ticket triage drafted (its key or id, `{{steps.import.ticketId}}`). Empty: nothing to approve (`skipped`).
    #[serde(default)]
    pub ticket: Option<String>,
    /// Triage's notes, shown with it (`{{steps.triage.summary}}`).
    #[serde(default)]
    #[schemars(extend("x-format" = "markdown"))]
    pub summary: Option<String>,
    /// The issue it came from (`{{trigger.url}}`).
    #[serde(default)]
    pub issue: Option<String>,
}

pub struct InboxAdmit;

fn skipped(reason: &str) -> Outcome {
    Outcome::next("skipped", json!({"quiet": true, "reason": reason}))
}

impl NodeKind for InboxAdmit {
    fn spec(&self) -> KindSpec {
        spec::<InboxAdmitConfig, Value>(
            KIND,
            1,
            Category::Action,
            "Wait for my approval in the Inbox",
            "Puts a ticket triage drafted in your Inbox (Triaged tickets), with triage's notes and the issue it came \
             from: Approve moves it to To do as your own move (so it lets an external ticket in), Reject moves it to \
             Won't do (where \"Done closes the GitHub issue\" closes a linked issue as not planned). Once per ticket; \
             `skipped` when the project's triage approval is off, the ticket is already past Ready, or no ticket is \
             given. The step does not wait: the item closes by itself when the ticket moves past Ready some other way.",
            &[("added", "In the Inbox"), ("skipped", "Not asked")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<InboxAdmitConfig>(config)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: InboxAdmitConfig = parse(&input)?;
            let Some(given) = nonblank(&c.ticket) else { return Ok(skipped("no ticket to approve")) };
            // A dry run's stand-in for a ticket an earlier step would make (PDT-189).
            if cx.dry_run && pdt_core::workflow::dry::is_stand_in(given) {
                return Ok(Outcome::would("added", format!("ask you to approve {given}")));
            }
            let id = crate::mcp::tools::resolve_ticket(cx.st, given, cx.run.project_id.as_deref()).await?;
            let store = &cx.st.store;
            let t = store.get_ticket_summary(&id).await.map_err(crate::workflow::err)?.ok_or("no such ticket")?;
            let policy = store.get_project_settings(&t.project_id).await.map_err(crate::workflow::err)?.policy;
            if policy.triage_approval == TriageApproval::Off {
                return Ok(skipped("the project's triage approval is off"));
            }
            if !matches!(t.stage, TicketStage::Draft | TicketStage::Ready) {
                return Ok(skipped("the ticket is already past Ready"));
            }
            if cx.dry_run {
                return Ok(Outcome::would("added", format!("ask you to approve {}", t.key)));
            }
            let item = InboxItem {
                id: uuid::Uuid::now_v7().to_string(),
                kind: InboxKind::Admit,
                group: InboxGroup::Admit,
                status: InboxStatus::Open,
                project_id: Some(t.project_id.clone()),
                ticket_id: Some(id.clone()),
                ticket_key: None,
                ticket_title: None,
                attempt_id: None,
                session_id: None,
                run_id: Some(cx.run.id.clone()),
                // Not the step's: the item outlives the run.
                step_id: None,
                persona_id: None,
                persona_name: None,
                change_request: None,
                title: t.title.clone(),
                body: json!({
                    "summary": nonblank(&c.summary), "issue": nonblank(&c.issue),
                    "external": crate::untrusted::held_back(&t), "workflow": cx.workflow.name,
                }),
                choices: vec![
                    InboxChoice::new(inbox::APPROVE, "Approve", true),
                    InboxChoice::new(inbox::REJECT, "Reject", false),
                ],
                resolution: Value::Null,
                created_at: now_ms(),
                resolved_at: None,
            };
            match store.add_inbox_item_once(&item, &format!("admit:{id}")).await.map_err(crate::workflow::err)? {
                Some(i) => {
                    crate::notifications::kick(cx.st);
                    Ok(Outcome::next("added", json!({"itemId": i.id})))
                }
                None => Ok(skipped("already in the Inbox")),
            }
        })
    }
}

/// The user's Approve or Reject of a triaged ticket. `from_ui`: the answer came from PDT's web client (PDT-204), which
/// an Approve that lets an external ticket in needs: an agent's curl never admits one.
pub(crate) async fn answer(st: &AppState, item: &InboxItem, choice: &str, from_ui: bool) -> Result<Value, String> {
    let id = item.ticket_id.as_deref().ok_or("the item has no ticket")?;
    let t = st.store.get_ticket_summary(id).await.map_err(crate::workflow::err)?.ok_or("the ticket is gone")?;
    let to = match choice {
        inbox::APPROVE => TicketStage::Todo,
        inbox::REJECT => TicketStage::WontDo,
        other => return Err(format!("{other} is not a choice here: approve or reject")),
    };
    if to == TicketStage::Todo && crate::untrusted::held_back(&t) && !from_ui {
        return Err(format!(
            "{} came from outside PDT, so only the user can approve it, from PDT's Inbox or board",
            t.key
        ));
    }
    if !st
        .store
        .end_inbox_item(&item.id, InboxStatus::Resolved, &json!({"choice": choice}), now_ms())
        .await
        .map_err(crate::workflow::err)?
    {
        return Err("this item was already answered".into());
    }
    let extra = json!({"via": "inbox"});
    let since = now_ms();
    st.store
        .move_ticket_by(id, to, Actor::User, Some(&extra), now_ms())
        .await
        .map_err(crate::workflow::err)?
        .ok_or("the ticket is gone")?;
    if to.is_closed() {
        crate::start::close_attempt(st, id, to, since, Actor::User, None).await.map_err(|e| e.message().to_string())?;
    }
    Ok(json!({"stage": to}))
}
