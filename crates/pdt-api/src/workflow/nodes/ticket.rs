//! Ticket actions: `ticket.finish` (the domain command), `ticket.move`, `ticket.comment`,
//! `ticket.create`.

use crate::finish::{FinishOutcome, FinishRequest};
use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::ticket::{Actor, TicketSource, TicketStage, ValidNewTicket};
use pdt_core::workflow::tpl::Tpl;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct FinishConfig {
    /// The ticket's id (`{{trigger.ticketId}}`).
    pub ticket: String,
    /// Stop the sessions and runs still going (else they block).
    #[serde(default = "yes")]
    #[ts(type = "boolean | string")]
    pub stop_running: Tpl<bool>,
    /// Delete the branch with the worktree.
    #[serde(default)]
    #[ts(type = "boolean | string")]
    pub delete_branch: Tpl<bool>,
}

fn yes() -> Tpl<bool> {
    Tpl::Value(true)
}

pub struct Finish;

impl NodeKind for Finish {
    fn spec(&self) -> KindSpec {
        spec::<FinishConfig, Value>(
            "ticket.finish",
            1,
            Category::Action,
            "Finish the ticket",
            "Moves the ticket to Done, stops and removes its sessions and runs, removes its worktree and workspace and \
             pulls the base branch. Uncommitted changes are never discarded: then nothing changes (`blocked`).",
            &[("finished", "Finished"), ("blocked", "Blocked")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<FinishConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: FinishConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next("finished", json!({"would": format!("finish ticket {}", c.ticket)})));
            }
            let req = FinishRequest {
                stop_running: c.stop_running.or(true),
                discard_changes: false,
                delete_branch: c.delete_branch.or(false),
                keep_worktree: false,
            };
            let by = cx.by();
            let r = crate::finish::finish_ticket(cx.st, &c.ticket, &req, Some(&by))
                .await
                .map_err(|e| e.message().to_string())?;
            let port = if r.outcome == FinishOutcome::Blocked { "blocked" } else { "finished" };
            let mut out = serde_json::to_value(&r).unwrap_or(Value::Null);
            if port == "blocked" {
                out["quiet"] = json!(true);
            }
            Ok(Outcome::Next { port: port.into(), output: out })
        })
    }

    fn default_timeout_ms(&self) -> u64 {
        2 * 60 * 1000
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct MoveConfig {
    pub ticket: String,
    #[ts(type = "string")]
    pub stage: Tpl<TicketStage>,
    /// Where an external ticket the user hasn't let in goes instead of To do or In progress (PDT-201: only the user
    /// puts it there). Without it, such a move fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "string")]
    pub external_stage: Option<Tpl<TicketStage>>,
}

pub struct Move;

impl NodeKind for Move {
    fn spec(&self) -> KindSpec {
        spec::<MoveConfig, Value>(
            "ticket.move",
            1,
            Category::Action,
            "Move the ticket",
            "Moves a ticket to a stage.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<MoveConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: MoveConfig = parse(&input)?;
            let mut stage = *c.stage.value().ok_or("stage is not a stage")?;
            if cx.dry_run {
                return Ok(Outcome::next(
                    "out",
                    json!({"would": format!("move ticket {} to {}", c.ticket, stage.as_str())}),
                ));
            }
            if let Some(instead) = &c.external_stage {
                let instead = *instead.value().ok_or("externalStage is not a stage")?;
                let cur = cx.st.store.get_ticket_summary(&c.ticket).await.map_err(crate::workflow::err)?;
                if let Some(cur) = cur.filter(crate::untrusted::held_back)
                    && pdt_core::ticket::boards(Some(cur.stage), Some(stage), &[], None)
                {
                    stage = instead;
                }
            }
            let data = cx.by().data();
            let since = now_ms();
            let t = cx
                .st
                .store
                .move_ticket_by(&c.ticket, stage, Actor::System, Some(&data), since)
                .await
                .map_err(crate::workflow::err)?
                .ok_or("no such ticket")?;
            // Closing it ends its attempt and winds its work down (PDT-339), but this run's.
            if stage.is_closed() {
                crate::start::close_attempt(cx.st, &t.summary.id, stage, since, Actor::System, Some(&cx.run.id))
                    .await
                    .map_err(|e| e.message().to_string())?;
            }
            Ok(Outcome::next("out", json!({"ticketId": t.summary.id, "stage": t.summary.stage})))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CommentConfig {
    /// The ticket's id or key (`PDT-12`).
    pub ticket: String,
    #[schemars(extend("x-format" = "markdown"))]
    pub body: String,
}

pub struct Comment;

impl NodeKind for Comment {
    fn spec(&self) -> KindSpec {
        spec::<CommentConfig, Value>(
            "ticket.comment",
            1,
            Category::Action,
            "Comment on the ticket",
            "Adds a comment to a ticket's timeline (by its id or key), from the workflow.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: CommentConfig = parse(config)?;
        if c.body.trim().is_empty() {
            return Err("body is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CommentConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next(
                    "out",
                    json!({"would": format!("comment on ticket {}: {}", c.ticket, c.body)}),
                ));
            }
            let ticket = crate::mcp::tools::resolve_ticket(cx.st, &c.ticket, cx.run.project_id.as_deref()).await?;
            // The comment carries its step, so the step run again after a restart finds it (PDT-131).
            if let Some(e) = cx.st.store.comment_by_step(&ticket, &cx.step.id).await.map_err(crate::workflow::err)? {
                return Ok(Outcome::next("out", json!({"eventId": e.id})));
            }
            let mut data = cx.by().data();
            data["stepId"] = json!(cx.step.id);
            let e = cx
                .st
                .store
                .add_comment_by(&ticket, &c.body, Actor::System, Some(data), now_ms())
                .await
                .map_err(crate::workflow::err)?
                .ok_or("no such ticket")?;
            Ok(Outcome::next("out", json!({"eventId": e.id})))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CreateConfig {
    /// The project's id (`{{trigger.projectId}}`).
    pub project: String,
    pub title: String,
    #[serde(default)]
    #[ts(optional)]
    #[schemars(extend("x-format" = "markdown"))]
    pub description: Option<String>,
    /// Draft when left out.
    #[serde(default)]
    #[ts(optional)]
    pub stage: Option<TicketStage>,
    #[serde(default)]
    #[ts(optional)]
    pub labels: Option<Vec<String>>,
}

pub struct Create;

impl NodeKind for Create {
    fn spec(&self) -> KindSpec {
        spec::<CreateConfig, Value>(
            "ticket.create",
            1,
            Category::Action,
            "Create a ticket",
            "Creates a ticket in a project (at the top of its stage), e.g. a follow-up. Output: its `id` and `key`.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: CreateConfig = parse(config)?;
        if c.title.trim().is_empty() {
            return Err("title is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CreateConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next(
                    "out",
                    json!({"would": format!("create ticket {} in {}", c.title, c.project)}),
                ));
            }
            let input = pdt_core::ticket::NewTicket {
                title: c.title,
                description: c.description,
                stage: c.stage,
                priority: None,
                labels: c.labels,
                areas: None,
                before: None,
                after: None,
                parent: None,
                blocked_by_previous: None,
            };
            // A run outside input started makes external tickets, kept out of To do (PDT-201).
            let untrusted = crate::untrusted::untrusted_run(cx.st, cx.run).await;
            let source = match untrusted {
                true => TicketSource::External,
                false => TicketSource::Workflow,
            };
            let valid = ValidNewTicket { source, ..input.validate().map_err(crate::workflow::err)? };
            let id = uuid::Uuid::now_v7().to_string();
            let t = cx
                .st
                .store
                .create_ticket_by(&id, &c.project, &valid, Actor::System, now_ms())
                .await
                .map_err(crate::workflow::err)?;
            if untrusted {
                crate::untrusted::record_run_on_ticket(cx.st, cx.run, &t.summary.id).await;
            }
            Ok(Outcome::next("out", json!({"id": t.summary.id, "key": t.summary.key})))
        })
    }

    /// A second run would make a second ticket.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }
}
