//! `agent`: an agent step. Runs a persona in a new or reused session with a declared outcome set;
//! the outcome it submits through pdt-mcp picks the port. `workflow::agent` does the work.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::agent::{OutcomeSpec, STOPPED, TIMEOUT, valid_outcome_id};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const KIND: &str = "agent";

const DEFAULT_MAX_TURNS: u32 = 10;
const DEFAULT_MAX_RUNTIME_MS: u64 = 4 * 60 * 60 * 1000;
const DEFAULT_NUDGES: u32 = 1;
const DEFAULT_LOOP_LIMIT: u32 = 3;
const DEFAULT_BACKGROUND_WAIT_MS: u64 = 30 * 60 * 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SessionChoice {
    /// A fresh session for each pass, stopped once its outcome is in (Refiner).
    #[default]
    New,
    /// The session this node used earlier in the run, else the attempt's running agent session, else a new one that
    /// becomes the attempt's (Coder).
    Reuse,
    /// The session this node used earlier in the run, else a new one of its own: never the attempt's, and parked once
    /// its outcome is in (Reviewer, QA; PDT-183).
    Own,
}

/// Who does an agent step's work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Owner {
    /// The persona, in a session.
    #[default]
    Agent,
    /// The user (a manual stage): no session; the Inbox asks them for the outcome.
    User,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AgentConfig {
    /// A persona's name or id.
    pub persona: String,
    /// The task; PDT adds the ticket and the outcome rules.
    #[schemars(extend("x-format" = "markdown"))]
    pub prompt: String,
    /// What the step may end with; one port each.
    pub outcomes: Vec<OutcomeSpec>,
    #[serde(default)]
    pub session: SessionChoice,
    /// The ticket (default `{{trigger.ticketId}}`).
    #[serde(default)]
    pub ticket: Option<String>,
    /// The attempt (default `{{trigger.attemptId}}`, else the ticket's active attempt).
    #[serde(default)]
    pub attempt: Option<String>,
    /// Turns that may end without an outcome, nudges included (default 10).
    #[serde(default)]
    pub max_turns: Option<u32>,
    /// Then `timeout` (default 4 hours).
    #[serde(default)]
    #[ts(type = "number | null")]
    pub max_runtime_ms: Option<u64>,
    /// Automatic nudges before the step counts as stalled (default 1).
    #[serde(default)]
    pub nudges: Option<u32>,
    /// How long the session may wait on background tasks it started, its turn ended, before the step stalls
    /// (default 30 minutes).
    #[serde(default)]
    #[ts(type = "number | null")]
    pub background_wait_ms: Option<u64>,
    /// How many times the node may run again in one run before the Inbox asks (default 3; 0: no limit).
    #[serde(default)]
    pub loop_limit: Option<u32>,
    /// `user`: a manual stage, done by the user, who picks the outcome in the Inbox.
    #[serde(default)]
    pub owner: Owner,
    /// The ticket's pipeline stage while this step runs (the Board's column).
    #[serde(default)]
    #[ts(optional)]
    pub pipeline_stage: Option<pdt_core::ticket::PipelineStage>,
}

impl AgentConfig {
    pub fn max_turns(&self) -> u32 {
        self.max_turns.unwrap_or(DEFAULT_MAX_TURNS).max(1)
    }
    pub fn max_runtime_ms(&self) -> u64 {
        self.max_runtime_ms.unwrap_or(DEFAULT_MAX_RUNTIME_MS)
    }
    pub fn nudges(&self) -> u32 {
        self.nudges.unwrap_or(DEFAULT_NUDGES)
    }
    pub fn background_wait_ms(&self) -> u64 {
        self.background_wait_ms.unwrap_or(DEFAULT_BACKGROUND_WAIT_MS)
    }
    pub fn loop_limit(&self) -> u32 {
        self.loop_limit.unwrap_or(DEFAULT_LOOP_LIMIT)
    }
}

/// The step's output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentOutput {
    pub outcome: Option<String>,
    pub summary: Option<String>,
    pub comments: Vec<pdt_core::workflow::agent::OutcomeComment>,
    pub session_id: Option<String>,
    pub persona: String,
    pub persona_version: u32,
    pub turns: u32,
    /// `agent`, or `user` when taken over.
    pub by: String,
}

pub struct Agent;

impl NodeKind for Agent {
    fn spec(&self) -> KindSpec {
        spec::<AgentConfig, AgentOutput>(
            KIND,
            1,
            Category::Agent,
            "Agent step",
            "Runs a persona in a session with a set of outcomes; the outcome the agent submits (submit_outcome) picks \
             the port. It can ask the user (ask_user); a turn that ends without an outcome is nudged, then stalled in \
             the Inbox (not while background tasks it started run, up to backgroundWaitMs). `timeout`: no outcome \
             within its max runtime or turns; `stopped`: stopped at its loop limit.",
            &[(TIMEOUT, "No outcome in time"), (STOPPED, "Stopped at the loop limit")],
            true,
        )
    }

    fn ports(&self, config: &Value) -> Vec<String> {
        let mut out: Vec<String> =
            parse::<AgentConfig>(config).map(|c| c.outcomes.into_iter().map(|o| o.id).collect()).unwrap_or_default();
        out.extend([TIMEOUT.to_string(), STOPPED.to_string()]);
        out
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: AgentConfig = parse(config)?;
        if c.persona.trim().is_empty() {
            return Err("name the persona".into());
        }
        if c.prompt.trim().is_empty() {
            return Err("the prompt is empty".into());
        }
        if c.outcomes.is_empty() {
            return Err("an agent step needs at least one outcome".into());
        }
        let mut seen = std::collections::HashSet::new();
        for o in &c.outcomes {
            valid_outcome_id(&o.id)?;
            if !seen.insert(o.id.as_str()) {
                return Err(format!("outcome {} is listed twice", o.id));
            }
        }
        Ok(())
    }

    fn agent_text(&self) -> &'static [&'static str] {
        &["prompt"]
    }

    /// Agents take a while: the engine's timeout covers starting the session, not the work.
    fn default_timeout_ms(&self) -> u64 {
        2 * 60 * 1000
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: AgentConfig = parse(&input)?;
            if cx.dry_run {
                let first = c.outcomes.first().map(|o| o.id.clone()).ok_or("no outcomes")?;
                if c.owner == Owner::User {
                    let would = format!(
                        "ask the user to do it and pick one of: {}",
                        c.outcomes.iter().map(|o| o.id.as_str()).collect::<Vec<_>>().join(", ")
                    );
                    return Ok(Outcome::next(&first, json!({"would": would})));
                }
                if let Some(why) = crate::untrusted::outsider_refusal(cx.ctx, cx.origin) {
                    return Err(why);
                }
                // What a real run would check first (PDT-152): the persona, and where the step runs.
                let persona = crate::workflow::agent::step_persona(cx.st, &c.persona).await?;
                crate::workflow::agent::agent_on(cx.st, &persona).await?;
                let t = crate::workflow::agent::target(cx, &c, true).await?;
                let would = format!(
                    "run {} ({} session{}) and wait for one of: {}",
                    persona.name,
                    match c.session {
                        SessionChoice::New => "a new",
                        SessionChoice::Reuse => "reused",
                        SessionChoice::Own => "its own, reused across passes",
                    },
                    if t.host { ", in the host project with no ticket, as the PM runs" } else { "" },
                    c.outcomes.iter().map(|o| o.id.as_str()).collect::<Vec<_>>().join(", ")
                );
                return Ok(Outcome::next(&first, json!({"would": would})));
            }
            crate::workflow::agent::start_step(cx, c).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_are_ports_and_are_checked() {
        let cfg = json!({"persona": "Reviewer", "prompt": "Review it.",
                         "outcomes": [{"id": "approved"}, {"id": "changes_requested", "description": "fix"}]});
        assert!(Agent.check(&cfg).is_ok());
        assert_eq!(Agent.ports(&cfg), ["approved", "changes_requested", "timeout", "stopped"]);
        let dup = json!({"persona": "R", "prompt": "p", "outcomes": [{"id": "a"}, {"id": "a"}]});
        assert!(Agent.check(&dup).unwrap_err().contains("twice"));
        let reserved = json!({"persona": "R", "prompt": "p", "outcomes": [{"id": "timeout"}]});
        assert!(Agent.check(&reserved).is_err());
        assert!(Agent.check(&json!({"persona": "R", "prompt": "p", "outcomes": []})).is_err());
        let c: AgentConfig = parse(&cfg).unwrap();
        assert_eq!((c.max_turns(), c.nudges(), c.loop_limit(), c.session), (10, 1, 3, SessionChoice::New));
        assert_eq!((c.owner, c.pipeline_stage), (Owner::Agent, None));
        let manual = json!({"persona": "Reviewer", "prompt": "Review it.", "outcomes": [{"id": "approved"}],
                            "owner": "user", "pipelineStage": "review"});
        let m: AgentConfig = parse(&manual).unwrap();
        assert_eq!((m.owner, m.pipeline_stage), (Owner::User, Some(pdt_core::ticket::PipelineStage::Review)));
    }
}
