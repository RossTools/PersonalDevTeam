//! `ticket.stalled` (PDT-510): an idle ticket (`crate::workflow::idle`) in the Inbox as stalled, until something goes
//! on again. "When an agent is stuck" puts it after `ticket.idle`.

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, parses, spec};
use crate::workflow::idle::{Busy, idle};
use pdt_core::inbox::{InboxChoice, InboxGroup, InboxKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const KIND: &str = "ticket.stalled";
pub const DISMISS: &str = "dismiss";
pub const MOVED_ON: &str = "moved_on";
/// How often it looks at the ticket again.
const LOOK_MS: i64 = 60_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct TicketStalledConfig {
    /// The idle ticket's attempt (`{{trigger.attemptId}}`).
    pub attempt: String,
}

pub struct TicketStalled;

fn text(pr: Option<(u32, String)>, minutes: u64) -> String {
    let pr = match pr {
        Some((n, url)) => format!(" Its [pull request #{n}]({url}) has no CI running and isn't merging."),
        None => String::new(),
    };
    format!(
        "Nothing has gone on for {minutes} minutes: no agent works or waits on it, and nothing in the Inbox is about \
         it, but a workflow on it hasn't finished.{pr}\n\nLook at its workflow runs and its pull request: re-run CI, \
         start the next step yourself, or move the ticket. This item goes away by itself once something goes on \
         again."
    )
}

impl NodeKind for TicketStalled {
    fn spec(&self) -> KindSpec {
        spec::<TicketStalledConfig, Value>(
            KIND,
            1,
            Category::Action,
            "Report the idle ticket",
            "Puts one stalled item in the Inbox about the attempt's idle ticket (nothing goes on, but a workflow on \
             it hasn't finished), with Dismiss (`dismiss`); the ticket shows as stalled meanwhile. It looks again \
             every minute and closes the item by itself once something goes on (`moved_on`): an agent works, CI \
             runs, the ticket moves.",
            &[(DISMISS, "Dismiss"), (MOVED_ON, "Moved on")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<TicketStalledConfig>(config)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: TicketStalledConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::would(DISMISS, "report the idle ticket in the Inbox"));
            }
            let store = &cx.st.store;
            let info = store.get_attempt(&c.attempt).await.map_err(crate::workflow::err)?.ok_or("no such attempt")?;
            let active = info.attempt.state == pdt_core::ticket::AttemptState::Active;
            if !active || !idle(cx.st, &info, &Busy::read(cx.st).await, Some(&cx.run.id)).await {
                return Ok(Outcome::Next { port: MOVED_ON.into(), output: json!({"quiet": true}) });
            }
            let title = format!("{} is stalled: nothing is going on", info.ticket_key);
            if store.open_items_for_step(&cx.step.id).await.map_err(crate::workflow::err)?.is_empty() {
                let pr = crate::github::watched_pr(cx.st, &info.attempt.id).map(|p| (p.number, p.url));
                let minutes = cx.ctx.pointer("/trigger/idleMinutes").and_then(Value::as_u64).unwrap_or(0);
                let body = json!({
                    "question": title, "text": text(pr.clone(), minutes), "url": pr.map(|p| p.1), "reason": crate::workflow::idle::IDLE_REASON,
                    "who": "Ticket", "workflow": cx.workflow.name, "node": cx.node_title(),
                });
                let item = cx
                    .inbox_item(InboxKind::Gate, InboxGroup::Stalled, &title, body)
                    .choices(vec![InboxChoice::new(DISMISS, "Dismiss", false)])
                    .ticket(Some(info.ticket_id.clone()))
                    .attempt(Some(info.attempt.id.clone()));
                if store
                    .add_inbox_item_once(&item, &format!("idle:{}", cx.step.id))
                    .await
                    .map_err(crate::workflow::err)?
                    .is_some()
                {
                    crate::notifications::kick(cx.st);
                }
            }
            Ok(Outcome::Wait {
                wait: json!({"retry": true, (crate::workflow::engine::POLL): true,
                             "answer": {"question": title, "choices": [DISMISS]}}),
                until: Some(now_ms() + LOOK_MS),
                timeout_port: None,
                output: json!({}),
            })
        })
    }
}
