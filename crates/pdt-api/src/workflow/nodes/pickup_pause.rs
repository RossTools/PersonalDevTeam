//! `pickup.pause` and `pickup.resume` (PDT-370): pause auto-start host-wide with a reason the Board shows, and resume
//! it. `crate::pickup_pause` keeps the pause.

use crate::pickup_pause;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, parses, spec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PickupPauseConfig {
    /// Why, as the Board shows it on each ticket auto-start would take ("the disk is almost full").
    pub reason: String,
}

pub struct PickupPauseKind;

impl NodeKind for PickupPauseKind {
    fn spec(&self) -> KindSpec {
        spec::<PickupPauseConfig, Value>(
            "pickup.pause",
            1,
            Category::Action,
            "Pause pickup",
            "Pauses auto-start in every project until `pickup.resume` in this workflow (or you, in Settings › \
             Storage) resumes it: no new ticket starts by itself, and the Board says why on each one that would have. \
             Agents already running carry on. Pausing again only changes the reason.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: PickupPauseConfig = parse(config)?;
        if c.reason.trim().is_empty() {
            return Err("reason is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PickupPauseConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::would("out", format!("pause pickup: {}", c.reason)));
            }
            let p = pickup_pause::pause(cx.st, &c.reason, Some(&cx.workflow.id)).await?;
            Ok(Outcome::next("out", p))
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PickupResumeConfig {
    /// Resume a pause made elsewhere too (another workflow's); by default only this workflow's.
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub any: bool,
}

pub struct PickupResumeKind;

impl NodeKind for PickupResumeKind {
    fn spec(&self) -> KindSpec {
        spec::<PickupResumeConfig, Value>(
            "pickup.resume",
            1,
            Category::Action,
            "Resume pickup",
            "Resumes auto-start paused by `pickup.pause` in this workflow (any pause with `any`): tickets waiting for \
             it start again. `not_paused` when there was no such pause.",
            &[("resumed", "Resumed"), ("not_paused", "Wasn't paused")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<PickupResumeConfig>(config)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PickupResumeConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::would("resumed", "resume pickup"));
            }
            let by = (!c.any).then_some(cx.workflow.id.as_str());
            Ok(if pickup_pause::resume(cx.st, by).await? {
                Outcome::next("resumed", json!({}))
            } else {
                Outcome::next("not_paused", json!({}))
            })
        })
    }
}
