//! `gate`: a human gate. It takes the port of the user's choice: one made ahead of time (`ahead`, e.g. Merge…'s
//! "finish once it merges", PDT-75) passes at once; otherwise it waits for an answer from the Inbox (Phase 7b) or the
//! API, or takes `unanswered` at once.

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::inbox::{InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
use pdt_core::workflow::cond::{Condition, all};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const UNANSWERED: &str = "unanswered";
pub const TIMEOUT: &str = "timeout";
pub const EVENT: &str = "event";

/// A choice made ahead of time: taken when every condition holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Ahead {
    pub when: Vec<Condition>,
    pub choice: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Ask {
    /// Wait for an answer (`POST /api/workflow-runs/{run}/steps/{step}/answer`).
    #[default]
    Wait,
    /// Don't ask: take `unanswered` (something else asks, e.g. the merged banner).
    Skip,
}

/// Where the Inbox lists a waiting gate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GateInbox {
    /// Under Gates.
    #[default]
    Gate,
    /// Under Ready to accept, with Open changes beside the choices.
    Accept,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GateConfig {
    pub question: String,
    /// One port each.
    pub choices: Vec<String>,
    #[serde(default)]
    pub ahead: Vec<Ahead>,
    #[serde(default)]
    pub ask: Ask,
    /// How long to wait for an answer before taking `timeout`.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub timeout_ms: Option<u64>,
    /// Where the Inbox lists it while it waits.
    #[serde(default)]
    pub inbox: GateInbox,
    /// The ticket's pipeline stage while it waits (the Board's column).
    #[serde(default)]
    #[ts(optional)]
    pub pipeline_stage: Option<pdt_core::ticket::PipelineStage>,
    /// A choice that is the user's approval of the run's attempt: taking it records an approval at the branch head
    /// (as Approve does), which a project that requires one needs before an automated merge.
    #[serde(default)]
    #[ts(optional)]
    pub approve: Option<String>,
    /// An event that makes the question moot (e.g. the attempt merging): it takes `event` and closes the Inbox item.
    /// Its conditions use `{{event.…}}` like a wait's.
    #[serde(default)]
    #[ts(optional)]
    pub event: Option<crate::workflow::nodes::wait::EventWait>,
}

pub struct Gate;

/// A choice as the Inbox's button says it: `merge_again` → "Merge again".
fn choice_title(choice: &str) -> String {
    let words = choice.replace('_', " ");
    let mut c = words.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

impl NodeKind for Gate {
    fn spec(&self) -> KindSpec {
        spec::<GateConfig, Value>(
            "gate",
            1,
            Category::Gate,
            "Ask the user",
            "A human gate: one port per choice. A choice made ahead of time passes at once; with `event`, that event \
             coming first takes `event`.",
            &[(UNANSWERED, "Not asked"), (TIMEOUT, "No answer in time")],
            true,
        )
    }

    fn ports(&self, config: &Value) -> Vec<String> {
        let c = parse::<GateConfig>(config).ok();
        let event = c.as_ref().is_some_and(|c| c.event.is_some());
        let mut out: Vec<String> = c.map(|c| c.choices).unwrap_or_default();
        out.extend([UNANSWERED.to_string(), TIMEOUT.to_string()]);
        if event {
            out.push(EVENT.to_string());
        }
        out
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GateConfig = parse(config)?;
        if c.choices.is_empty() {
            return Err("a gate needs at least one choice".into());
        }
        if c.choices.iter().any(|x| [UNANSWERED, TIMEOUT, EVENT, "error"].contains(&x.as_str()) || x.trim().is_empty())
        {
            return Err("choices can't be empty, unanswered, timeout, event or error".into());
        }
        if let Some(e) = &c.event {
            e.check()?;
        }
        if let Some(a) = c.ahead.iter().find(|a| !c.choices.contains(&a.choice)) {
            return Err(format!("ahead names {}, which is not a choice", a.choice));
        }
        if let Some(a) = c.approve.as_ref().filter(|a| !c.choices.contains(a)) {
            return Err(format!("approve names {a}, which is not a choice"));
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GateConfig = parse(&input)?;
            if let Some(a) = c.ahead.iter().find(|a| all(&a.when, cx.ctx)) {
                return Ok(Outcome::next(&a.choice, json!({"choice": a.choice, "answeredAhead": true})));
            }
            if c.ask == Ask::Skip {
                return Ok(Outcome::next(UNANSWERED, json!({"quiet": true})));
            }
            if cx.dry_run {
                return Ok(Outcome::next(
                    UNANSWERED,
                    json!({"would": format!("ask: {} ({})", c.question, c.choices.join(", "))}),
                ));
            }
            crate::workflow::nodes::set_stage(cx, c.pipeline_stage).await;
            let group = match c.inbox {
                GateInbox::Gate => InboxGroup::Gate,
                GateInbox::Accept => InboxGroup::Accept,
            };
            let item = InboxItem {
                id: uuid::Uuid::now_v7().to_string(),
                kind: InboxKind::Gate,
                group,
                status: InboxStatus::Open,
                project_id: cx.run.project_id.clone(),
                ticket_id: cx.run.ticket_id.clone(),
                ticket_key: None,
                ticket_title: None,
                attempt_id: cx.run.attempt_id.clone(),
                session_id: None,
                run_id: Some(cx.run.id.clone()),
                step_id: Some(cx.step.id.clone()),
                persona_id: None,
                persona_name: None,
                title: c.question.clone(),
                body: json!({"question": c.question, "workflow": cx.workflow.name, "node": cx.node_title()}),
                choices: c
                    .choices
                    .iter()
                    .enumerate()
                    .map(|(i, x)| InboxChoice::new(x, choice_title(x), i == 0))
                    .collect(),
                resolution: Value::Null,
                created_at: now_ms(),
                resolved_at: None,
            };
            cx.st.store.open_inbox_item(&item).await.map_err(|e| e.to_string())?;
            let mut wait = json!({"answer": {"question": c.question, "choices": c.choices, "approve": c.approve}});
            if let Some(e) = c.event {
                wait["event"] = e.wait();
            }
            Ok(Outcome::Wait {
                wait,
                until: c.timeout_ms.map(|ms| now_ms() + i64::try_from(ms).unwrap_or(i64::MAX / 2)),
                timeout_port: Some(TIMEOUT.into()),
                output: Value::Null,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_read_as_words() {
        assert_eq!(choice_title("merge_again"), "Merge again");
        assert_eq!(choice_title("stop"), "Stop");
        assert_eq!(choice_title(""), "");
    }

    #[test]
    fn an_event_adds_its_port_and_must_be_known() {
        let c = json!({"question": "q", "choices": ["go"], "event": {"type": "attempt.merged"}});
        assert_eq!(Gate.ports(&c), ["go", UNANSWERED, TIMEOUT, EVENT]);
        assert_eq!(Gate.ports(&json!({"question": "q", "choices": ["go"]})), ["go", UNANSWERED, TIMEOUT]);
        assert!(Gate.check(&c).is_ok());
        let bad = json!({"question": "q", "choices": ["go"], "event": {"type": "nope"}});
        assert!(Gate.check(&bad).unwrap_err().contains("nope"));
        assert!(Gate.check(&json!({"question": "q", "choices": ["event"]})).is_err());
    }
}
