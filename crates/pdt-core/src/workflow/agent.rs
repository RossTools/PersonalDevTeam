//! Agent steps (Phase 7b spec §4): the outcomes a step declares, and the texts PDT sends a persona's session (the
//! first message, nudges). The node itself is pdt-api's `workflow/nodes/agent.rs`.

use crate::brand::SHORT_NAME;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Ports an agent node has besides its outcomes.
pub const TIMEOUT: &str = "timeout";
pub const STOPPED: &str = "stopped";

/// One outcome a step allows: its port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct OutcomeSpec {
    /// `approved`; lower case, digits and `_`.
    pub id: String,
    /// When the agent should pick it.
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentStepState {
    /// The session works on the step.
    Working,
    /// The session asked the user (`ask_user`) and waits for the answers.
    Asking,
    /// Its turns ended without an outcome; the Inbox has it.
    Stalled,
    /// It has its outcome (or the step ended otherwise).
    Done,
}

impl AgentStepState {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentStepState::Working => "working",
            AgentStepState::Asking => "asking",
            AgentStepState::Stalled => "stalled",
            AgentStepState::Done => "done",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Working, Self::Asking, Self::Stalled, Self::Done].into_iter().find(|x| x.as_str() == s)
    }
}

/// A comment an agent hands in with its outcome: a review comment on the attempt's changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OutcomeComment {
    /// The file, relative to the checkout.
    pub file: String,
    /// The first line (1-based, in the changed file).
    pub line: u32,
    /// The last line, for a range.
    #[serde(default)]
    pub end_line: Option<u32>,
    pub body: String,
}

/// An agent step's session binding (Phase 7b spec §4.2): which session runs it and what it may answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentStep {
    pub step_id: String,
    pub run_id: String,
    pub node_id: String,
    pub session_id: Option<String>,
    pub persona_id: Option<String>,
    pub persona_name: String,
    pub persona_version: u32,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub attempt_id: Option<String>,
    pub outcomes: Vec<OutcomeSpec>,
    pub state: AgentStepState,
    /// Turns that ended without an outcome.
    pub turns: u32,
    /// Nudges sent (automatic and the user's).
    pub nudges: u32,
    pub max_turns: u32,
    /// Automatic nudges before it counts as stalled.
    pub auto_nudges: u32,
    pub outcome: Option<String>,
    pub summary: Option<String>,
    pub comments: Vec<OutcomeComment>,
    /// The user took it over and picked the outcome.
    pub by_user: bool,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// Whether `id` can be an outcome (and so a port).
pub fn valid_outcome_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 40 {
        return Err("an outcome id is 1 to 40 characters".into());
    }
    if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(format!("outcome {id}: use lower case letters, digits and _"));
    }
    if [TIMEOUT, STOPPED, crate::workflow::doc::ERROR_PORT].contains(&id) {
        return Err(format!("{id} is a port every agent step has; name the outcome differently"));
    }
    Ok(())
}

/// `approved (it can merge as is), changes_requested (…)`.
pub fn outcome_list(outcomes: &[OutcomeSpec]) -> String {
    outcomes
        .iter()
        .map(|o| {
            if o.description.trim().is_empty() {
                format!("`{}`", o.id)
            } else {
                format!("`{}` ({})", o.id, o.description.trim())
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The ticket a step is about, for its first message.
pub struct StepTicket<'a> {
    pub key: &'a str,
    pub title: &'a str,
}

/// The first message of a step: the task, the ticket and how to finish.
pub fn first_message(prompt: &str, ticket: Option<StepTicket>, outcomes: &[OutcomeSpec]) -> String {
    let mut out = prompt.trim().to_string();
    if let Some(t) = ticket {
        out.push_str(&format!(
            "\n\nTicket: {} — {} (`get_ticket` gives its description, comments and earlier outcomes).",
            t.key, t.title
        ));
    }
    out.push_str(&format!(
        "\n\nWhen you are done, call `submit_outcome` with one of: {}. If only the user can unblock you, call \
         `ask_user`.",
        outcome_list(outcomes)
    ));
    out
}

/// The reminder a step's session gets when a turn ends without an outcome.
pub fn nudge_message(outcomes: &[OutcomeSpec]) -> String {
    format!(
        "[{SHORT_NAME}] You ended your turn without `submit_outcome`. Finish the step by calling it with one of: {}; \
         or call `ask_user` if you need the user.",
        outcome_list(outcomes)
    )
}

/// The section a step's session gets in its instructions (spec §4.2).
pub fn step_section(persona: &str, instructions: &str, outcomes: &[OutcomeSpec]) -> String {
    let mut out = format!(
        "\n# Your role: {persona}\n\n\
         This session is a step in a {SHORT_NAME} workflow. {SHORT_NAME} moves the work on from the outcome you \
         submit, never from what you write.\n\
         - Finish by calling `submit_outcome` with one of: {}, and a short summary.\n\
         - If only the user can unblock you, call `ask_user` with your questions and end your turn; their answers \
         come back as a message.\n\
         - Ending a turn without either counts as stalled.\n",
        outcome_list(outcomes)
    );
    let text = instructions.trim();
    if !text.is_empty() {
        out.push_str(&format!("\n{text}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outs() -> Vec<OutcomeSpec> {
        vec![
            OutcomeSpec { id: "approved".into(), description: "it can merge".into() },
            OutcomeSpec { id: "changes_requested".into(), description: String::new() },
        ]
    }

    #[test]
    fn outcome_ids_are_port_names_but_not_the_reserved_ones() {
        assert!(valid_outcome_id("changes_requested").is_ok());
        assert!(valid_outcome_id("Approved").is_err());
        assert!(valid_outcome_id("").is_err());
        assert!(valid_outcome_id("timeout").is_err());
        assert!(valid_outcome_id("error").is_err());
    }

    #[test]
    fn the_first_message_has_the_task_ticket_and_outcomes() {
        let m = first_message(" Review it. ", Some(StepTicket { key: "PDT-4", title: "Pipeline" }), &outs());
        assert!(m.starts_with("Review it.\n\nTicket: PDT-4 — Pipeline"), "{m}");
        assert!(m.contains("`submit_outcome` with one of: `approved` (it can merge), `changes_requested`."), "{m}");
        assert!(!first_message("x", None, &outs()).contains("Ticket:"));
    }

    #[test]
    fn nudges_and_the_step_section_name_the_outcomes() {
        assert!(nudge_message(&outs()).starts_with("[PDT] You ended your turn without `submit_outcome`."));
        let s = step_section("Reviewer", "Be strict.\n", &outs());
        assert!(s.contains("# Your role: Reviewer"));
        assert!(s.contains("`approved` (it can merge), `changes_requested`"));
        assert!(s.ends_with("\nBe strict.\n"));
        assert!(!step_section("R", "  ", &outs()).ends_with("\n\n"));
    }
}
