//! Ticket actions: `ticket.finish` (the domain command, Phase 7a spec §5.1), `ticket.move`, `ticket.comment`.

use crate::finish::{FinishOutcome, FinishRequest};
use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::ticket::{Actor, TicketStage};
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
            let stage = *c.stage.value().ok_or("stage is not a stage")?;
            if cx.dry_run {
                return Ok(Outcome::next(
                    "out",
                    json!({"would": format!("move ticket {} to {}", c.ticket, stage.as_str())}),
                ));
            }
            let data = cx.by().data();
            let t = cx
                .st
                .store
                .move_ticket_by(&c.ticket, stage, Actor::System, Some(&data), now_ms())
                .await
                .map_err(|e| e.to_string())?
                .ok_or("no such ticket")?;
            Ok(Outcome::next("out", json!({"ticketId": t.summary.id, "stage": t.summary.stage})))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CommentConfig {
    pub ticket: String,
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
            "Adds a comment to a ticket's timeline, from the workflow.",
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
            let e = cx
                .st
                .store
                .add_comment_by(&c.ticket, &c.body, Actor::System, Some(cx.by().data()), now_ms())
                .await
                .map_err(|e| e.to_string())?
                .ok_or("no such ticket")?;
            Ok(Outcome::next("out", json!({"eventId": e.id})))
        })
    }
}
