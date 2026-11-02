//! `agent.nudge`, `agent.restart`, `agent.inbox` and `agent.stop` (PDT-367): what "When an agent is stuck" does with a
//! stuck agent step (`{{trigger.stepId}}` of `agent.stalled`, `agent.loop_limit` or `agent.timed_out`).
//! `workflow::stuck` does the work.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct StuckStepConfig {
    /// The stuck step (`{{trigger.stepId}}`).
    pub step: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct NudgeConfig {
    /// The stalled step (`{{trigger.stepId}}`).
    pub step: String,
    /// What the agent is told; PDT adds the step's outcomes. `{{trigger.goal}}` is the step's task,
    /// `{{trigger.lastMessage}}` the end of the agent's last message.
    #[schemars(extend("x-format" = "markdown"))]
    pub message: String,
}

fn check_step(step: &str) -> Result<(), String> {
    if step.trim().is_empty() { Err("name the step (`{{trigger.stepId}}`)".into()) } else { Ok(()) }
}

const SKIPPED: (&str, &str) = ("skipped", "Not stuck");

pub struct Nudge;

impl NodeKind for Nudge {
    fn spec(&self) -> KindSpec {
        spec::<NudgeConfig, Value>(
            "agent.nudge",
            1,
            Category::Action,
            "Nudge the agent",
            "Sends a stalled agent step's session a message (a stopped conversation reopens to take it) and the step \
             works again. `no_session` when its session can't take one (removed, or a stopped terminal); `skipped` \
             when the step is no longer stalled.",
            &[("nudged", "Nudged"), ("no_session", "No session to nudge"), SKIPPED],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: NudgeConfig = parse(config)?;
        check_step(&c.step)?;
        if c.message.trim().is_empty() {
            return Err("the message is empty".into());
        }
        Ok(())
    }

    fn agent_text(&self) -> &'static [&'static str] {
        &["message"]
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: NudgeConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next("nudged", json!({"would": format!("nudge step {}", c.step)})));
            }
            crate::workflow::stuck::nudge(cx, &c.step, &c.message).await
        })
    }
}

pub struct Restart;

impl NodeKind for Restart {
    fn spec(&self) -> KindSpec {
        spec::<StuckStepConfig, Value>(
            "agent.restart",
            1,
            Category::Action,
            "Restart in a fresh session",
            "Starts the stuck step again in a new session with its task, a quote of what the earlier session last \
             said and the branch's state; the earlier session is stopped. A stalled step goes on in it; a timed-out \
             one is resumed (its run goes back to it). `skipped` when the step is no longer stuck, or is at its loop \
             limit.",
            &[("restarted", "Restarted"), SKIPPED],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        check_step(&parse::<StuckStepConfig>(config)?.step)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: StuckStepConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next("restarted", json!({"would": format!("restart step {}", c.step)})));
            }
            crate::workflow::stuck::restart(cx, &c.step).await
        })
    }
}

pub struct Ask;

impl NodeKind for Ask {
    fn spec(&self) -> KindSpec {
        spec::<StuckStepConfig, Value>(
            "agent.inbox",
            1,
            Category::Action,
            "Ask me in the Inbox",
            "Puts the stuck step in the Inbox as PDT does on its own: Stalled (Nudge, Take over), Loop limit (Once \
             more, Take over, Stop) or Timed out (Resume, Stop). `skipped` when the step is no longer stuck.",
            &[("asked", "In the Inbox"), SKIPPED],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        check_step(&parse::<StuckStepConfig>(config)?.step)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: StuckStepConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next("asked", json!({"would": format!("put step {} in the Inbox", c.step)})));
            }
            crate::workflow::stuck::ask(cx, &c.step).await
        })
    }
}

pub struct Stop;

impl NodeKind for Stop {
    fn spec(&self) -> KindSpec {
        spec::<StuckStepConfig, Value>(
            "agent.stop",
            1,
            Category::Action,
            "Stop the step",
            "Ends a stalled or loop-limited step by its `stopped` port; a timed-out step has ended already and is left \
             as it is. `skipped` when the step is no longer stuck.",
            &[("stopped", "Stopped"), SKIPPED],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        check_step(&parse::<StuckStepConfig>(config)?.step)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: StuckStepConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next("stopped", json!({"would": format!("stop step {}", c.step)})));
            }
            crate::workflow::stuck::stop(cx, &c.step).await
        })
    }
}
