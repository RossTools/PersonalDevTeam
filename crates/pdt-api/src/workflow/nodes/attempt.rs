//! `attempt.agent_working`: whether an agent step is on the attempt now (PDT-104). Read when the step runs, not when
//! the event was logged: two events of one look at a PR (a conflict and the queue drop it caused) both say no agent
//! is on it, but the first run's fix step is by the time the second run gets here (runs advance one at a time).

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::tpl::Tpl;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const KIND: &str = "attempt.agent_working";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AgentWorkingConfig {
    /// The attempt (`{{trigger.attemptId}}`).
    pub attempt: String,
    /// Already known to be busy (the event's `{{trigger.agentWorking}}`): `yes` without looking.
    #[serde(default)]
    #[ts(type = "boolean | string | null")]
    pub known: Option<Tpl<bool>>,
}

pub struct AgentWorking;

impl NodeKind for AgentWorking {
    fn spec(&self) -> KindSpec {
        spec::<AgentWorkingConfig, Value>(
            KIND,
            1,
            Category::Condition,
            "An agent on the attempt?",
            "`yes` while an agent step works on the attempt, in this workflow or another (bound to a session, or \
             waiting for a slot), else `no`. Read as the step runs, so of two runs from one event burst only the first \
             finds it free.",
            &[("yes", "An agent is on it"), ("no", "No agent on it")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<AgentWorkingConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: AgentWorkingConfig = parse(&input)?;
            let busy = c.known.is_some_and(|k| k.or(false))
                || cx.st.store.attempt_agent_busy(&c.attempt, &cx.run.id).await.map_err(|e| e.to_string())?;
            Ok(Outcome::next(if busy { "yes" } else { "no" }, json!({"working": busy})))
        })
    }
}
