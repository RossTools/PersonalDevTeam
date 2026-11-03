//! `ticket.close_parent` (PDT-377): a parent follows its parts. Its last open part closing closes it (Done, or Won't
//! do when every part was Won't do), unless it has work of its own; a part opening again moves a closed parent back to
//! In progress. The rule is pdt-core's `parts::parent_follow`.

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::parts::{ParentFollow, parent_follow};
use pdt_core::session::SessionStatus;
use pdt_core::ticket::{Actor, TicketStage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const KIND: &str = "ticket.close_parent";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CloseParentConfig {
    /// The part that moved (its id or key, `{{trigger.ticketId}}`).
    pub ticket: String,
}

pub struct CloseParent;

fn skipped(reason: &str) -> Outcome {
    Outcome::next("skipped", json!({"quiet": true, "reason": reason}))
}

impl NodeKind for CloseParent {
    fn spec(&self) -> KindSpec {
        spec::<CloseParentConfig, Value>(
            KIND,
            1,
            Category::Action,
            "Close or reopen the parent",
            "Makes a part's parent follow its parts. When the last open part reaches Done or Won't do, the parent moves \
             to Done (Won't do if every part was Won't do): `closed`. A parent with an active attempt or a running \
             session stays open: `held`. When a part opens again, a closed parent moves back to In progress: \
             `reopened`. Otherwise (a ticket that is not a part, parts still open) `skipped`. Output: the parent's \
             `parentId`, `parentKey`, `parentTitle`, `stage` (`stageName`: `Won't do`), and its `parts` count.",
            &[("closed", "Closed"), ("held", "Has open work"), ("reopened", "Reopened"), ("skipped", "Nothing to do")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: CloseParentConfig = parse(config)?;
        if c.ticket.trim().is_empty() {
            return Err("ticket is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CloseParentConfig = parse(&input)?;
            if cx.dry_run && pdt_core::workflow::dry::is_stand_in(&c.ticket) {
                return Ok(Outcome::would("closed", "close the part's parent once its parts are closed"));
            }
            let store = &cx.st.store;
            let id = crate::mcp::tools::resolve_ticket(cx.st, &c.ticket, cx.run.project_id.as_deref()).await?;
            let part = store.get_ticket_summary(&id).await.map_err(crate::workflow::err)?.ok_or("no such ticket")?;
            let Some(parent_ref) = part.parent else { return Ok(skipped("the ticket is not a part")) };
            let Some(parent) = store.get_ticket(&parent_ref.id).await.map_err(crate::workflow::err)? else {
                return Ok(skipped("the parent is gone"));
            };
            let parts: Vec<TicketStage> = parent.part_list.iter().map(|p| p.stage).collect();
            let busy = store.active_attempt(&parent_ref.id).await.map_err(crate::workflow::err)?.is_some()
                || parent.sessions.iter().any(|s| s.status == SessionStatus::Running);
            let follow = parent_follow(parent.summary.stage, &parts, busy);
            let out = |stage: TicketStage| {
                json!({"parentId": parent_ref.id, "parentKey": parent_ref.key, "parentTitle": parent_ref.title,
                       "stage": stage, "stageName": stage.label(), "parts": parts.len()})
            };
            let (port, to) = match follow {
                ParentFollow::Nothing => return Ok(skipped("nothing changes for the parent")),
                ParentFollow::Hold => {
                    let mut o = out(parent.summary.stage);
                    o["quiet"] = json!(true);
                    return Ok(Outcome::next("held", o));
                }
                ParentFollow::Close(to) => ("closed", to),
                ParentFollow::Reopen => ("reopened", TicketStage::InProgress),
            };
            if cx.dry_run {
                let would = format!("move {} to {}", parent_ref.key, to.as_str());
                return Ok(Outcome::would(port, would));
            }
            let mut data = cx.by().data();
            data["via"] = json!("parts");
            let since = now_ms();
            let moved = store
                .move_ticket_by(&parent_ref.id, to, Actor::System, Some(&data), since)
                .await
                .map_err(crate::workflow::err)?
                .ok_or("the parent is gone")?;
            if to.is_closed() {
                crate::start::close_attempt(cx.st, &parent_ref.id, to, since, Actor::System, Some(&cx.run.id))
                    .await
                    .map_err(|e| e.message().to_string())?;
            }
            Ok(Outcome::next(port, out(moved.summary.stage)))
        })
    }
}
