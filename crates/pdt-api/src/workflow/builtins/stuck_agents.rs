//! When an agent is stuck (PDT-367): what happens to an agent step that stalls, reaches its loop limit or times out.
//! Its inputs (template params) are what Settings → Agents shows; the workflow they fill in is the built-in.

use super::*;
use crate::workflow::templates::{params, unfill};
use pdt_core::workflow::template::{TemplateParam, fill, param_values};
use serde_json::{Map, Value};

/// The note on a version saved from Settings: only its inputs changed, so it still follows PDT's newer versions.
pub const SETTINGS_NOTE: &str = "inputs changed in Settings";

pub const FIRST_NUDGE: &str =
    "You stopped without an outcome. Finish with submit_outcome, or ask_user if you're blocked.";
pub const LATER_NUDGE: &str = "You stopped again without an outcome. The step's goal:\n\n{{trigger.goal}}\n\n\
Your last message ended:\n\n{{trigger.lastMessage}}\n\nFinish with submit_outcome, or ask_user if you're blocked.";

/// Its inputs.
pub fn stuck_params() -> Vec<TemplateParam> {
    params(json!([
        {"id": "nudges", "title": "Nudges", "type": "number", "default": 2,
         "description": "How many times a stalled agent is nudged before it is restarted or you are asked."},
        {"id": "firstNudge", "title": "First nudge", "type": "text", "default": FIRST_NUDGE,
         "description": "What the agent is told the first time it stops without an outcome."},
        {"id": "laterNudge", "title": "Later nudges", "type": "text", "default": LATER_NUDGE,
         "description": "What it is told after that. {{trigger.goal}} is the step's task, {{trigger.lastMessage}} \
                         the end of what the agent last said."},
        {"id": "restart", "title": "Restart in a fresh session before asking you", "type": "boolean", "default": true,
         "description": "Once the nudges are used up, the step starts again in a new session with its task, the \
                         earlier session's last message and the branch's state."},
        {"id": "onLoopLimit", "title": "At a loop limit", "type": "choice", "choices": ["ask", "stop"],
         "default": "ask", "description": "ask: the Inbox asks you (Once more, Take over, Stop). stop: the step \
                                           ends by its stopped port."},
        {"id": "timeoutRestarts", "title": "Fresh sessions after a time limit", "type": "number", "default": 1,
         "description": "How many times a step that timed out is restarted in a fresh session before you are asked."},
    ]))
}

/// The document with `{{params.*}}` in it.
pub fn stuck_template() -> WorkflowDoc {
    let step = json!({"step": "{{trigger.stepId}}"});
    doc(json!({
        "name": "When an agent is stuck",
        "description": "What happens when an agent step stops without an outcome, reaches its loop limit or runs \
                        out of time. A stalled agent is nudged, then restarted in a fresh session, then you are \
                        asked in the Inbox; a loop limit asks you; a step that timed out is restarted once, then \
                        you are asked. Change the nudges, the messages and the rest in Settings → Agents. Turned \
                        off, a stalled agent is nudged once, then you are asked.",
        "nodes": [
            {"id": "stalled", "kind": "agent.stalled", "version": 1, "position": {"x": 0, "y": 0},
             "title": "An agent stalled"},
            {"id": "nudges_left", "kind": "condition", "version": 1, "position": {"x": 300, "y": 0},
             "title": "Nudges left?", "config": {"conditions": [
                {"left": "{{trigger.nudges}}", "op": "lt", "right": "{{params.nudges}}"}]}},
            {"id": "first", "kind": "condition", "version": 1, "position": {"x": 600, "y": -120},
             "title": "First nudge?", "config": {"conditions": [
                {"left": "{{trigger.nudges}}", "op": "eq", "right": 0}]}},
            {"id": "nudge", "kind": "agent.nudge", "version": 1, "position": {"x": 900, "y": -200},
             "title": "Nudge", "config": {"step": "{{trigger.stepId}}", "message": "{{params.firstNudge}}"}},
            {"id": "nudge_again", "kind": "agent.nudge", "version": 1, "position": {"x": 900, "y": -60},
             "title": "Nudge again", "config": {"step": "{{trigger.stepId}}", "message": "{{params.laterNudge}}"}},
            {"id": "can_restart", "kind": "condition", "version": 1, "position": {"x": 900, "y": 100},
             "title": "Restart?", "config": {"conditions": [
                {"left": "{{params.restart}}", "op": "truthy"},
                {"left": "{{trigger.restarts}}", "op": "lt", "right": 1}]}},
            {"id": "restart", "kind": "agent.restart", "version": 1, "position": {"x": 1200, "y": 100},
             "title": "Restart in a fresh session", "config": step},
            {"id": "ask", "kind": "agent.inbox", "version": 1, "position": {"x": 1200, "y": 300},
             "title": "Ask me in the Inbox", "config": step},
            {"id": "loop_limit", "kind": "agent.loop_limit", "version": 1, "position": {"x": 0, "y": 400},
             "title": "At its loop limit"},
            {"id": "stop_at_limit", "kind": "condition", "version": 1, "position": {"x": 300, "y": 400},
             "title": "Stop at the limit?", "config": {"conditions": [
                {"left": "{{params.onLoopLimit}}", "op": "eq", "right": "stop"}]}},
            {"id": "stop", "kind": "agent.stop", "version": 1, "position": {"x": 600, "y": 400},
             "title": "Stop the step", "config": step},
            {"id": "timed_out", "kind": "agent.timed_out", "version": 1, "position": {"x": 0, "y": 600},
             "title": "Timed out"},
            {"id": "restarts_left", "kind": "condition", "version": 1, "position": {"x": 300, "y": 600},
             "title": "Restarts left?", "config": {"conditions": [
                {"left": "{{trigger.restarts}}", "op": "lt", "right": "{{params.timeoutRestarts}}"}]}},
            {"id": "done", "kind": "end", "version": 1, "position": {"x": 1500, "y": 0}, "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "stalled", "to": "nudges_left"},
            {"from": "nudges_left", "port": "true", "to": "first"},
            {"from": "nudges_left", "port": "false", "to": "can_restart"},
            {"from": "first", "port": "true", "to": "nudge"},
            {"from": "first", "port": "false", "to": "nudge_again"},
            {"from": "nudge", "port": "nudged", "to": "done"},
            {"from": "nudge", "port": "no_session", "to": "can_restart"},
            {"from": "nudge", "port": "skipped", "to": "done"},
            {"from": "nudge_again", "port": "nudged", "to": "done"},
            {"from": "nudge_again", "port": "no_session", "to": "can_restart"},
            {"from": "nudge_again", "port": "skipped", "to": "done"},
            {"from": "can_restart", "port": "true", "to": "restart"},
            {"from": "can_restart", "port": "false", "to": "ask"},
            {"from": "restart", "port": "restarted", "to": "done"},
            {"from": "restart", "port": "skipped", "to": "ask"},
            {"from": "ask", "port": "asked", "to": "done"},
            {"from": "ask", "port": "skipped", "to": "done"},
            {"from": "loop_limit", "to": "stop_at_limit"},
            {"from": "stop_at_limit", "port": "true", "to": "stop"},
            {"from": "stop_at_limit", "port": "false", "to": "ask"},
            {"from": "stop", "port": "stopped", "to": "done"},
            {"from": "stop", "port": "skipped", "to": "done"},
            {"from": "timed_out", "to": "restarts_left"},
            {"from": "restarts_left", "port": "true", "to": "restart"},
            {"from": "restarts_left", "port": "false", "to": "ask"},
        ],
    }))
}

/// The inputs' defaults.
pub fn defaults() -> Map<String, Value> {
    param_values(&stuck_params(), &Map::new()).expect("the stuck workflow's defaults")
}

/// The workflow filled in with `values` (checked against the inputs; missing ones take their defaults).
pub fn filled(values: &Map<String, Value>) -> Result<WorkflowDoc, String> {
    fill(&stuck_template(), &param_values(&stuck_params(), values)?)
}

/// The built-in as PDT ships it.
pub(super) fn stuck_agents() -> WorkflowDoc {
    filled(&Map::new()).expect("the stuck workflow fills in")
}

/// The inputs a document was filled in with: each one read back from it, else its default.
pub fn values_of(doc: &WorkflowDoc) -> Map<String, Value> {
    let mut out = defaults();
    for (k, v) in unfill(&stuck_template(), doc) {
        if out.contains_key(&k) {
            out.insert(k, v);
        }
    }
    out
}

/// Whether a document is the workflow filled in with some inputs (its name and description aside): else the user
/// changed it beyond its inputs.
pub fn customized(doc: &WorkflowDoc) -> bool {
    match filled(&values_of(doc)) {
        Ok(mut d) => {
            (d.name, d.description) = (doc.name.clone(), doc.description.clone());
            d != *doc
        }
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::validate::{Severity, validate};

    /// Every port leads somewhere, every node is reached, and the defaults are the ticket's.
    #[test]
    fn the_default_escalation_is_complete() {
        let d = stuck_agents();
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        assert!(!issues.iter().any(|i| i.code == "unreachable" || i.code == "open_port"), "{issues:?}");
        assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
        assert_eq!(d.node("nudge").unwrap().config["message"], FIRST_NUDGE);
        assert_eq!(d.node("nudges_left").unwrap().config["conditions"][0]["right"], 2);
        assert_eq!(d.node("can_restart").unwrap().config["conditions"][0]["left"], true);
        assert!(d.node("nudge_again").unwrap().config["message"].as_str().unwrap().contains("{{trigger.goal}}"));
        assert!(!customized(&d));
        assert_eq!(values_of(&d), defaults());
    }

    /// Changed inputs land in the document and read back; an edit beyond them is a customization.
    #[test]
    fn inputs_fill_in_and_read_back() {
        let given = json!({"nudges": 1, "firstNudge": "Hurry up.", "restart": false, "onLoopLimit": "stop",
                           "timeoutRestarts": 0});
        let d = filled(given.as_object().unwrap()).unwrap();
        assert_eq!(d.node("nudge").unwrap().config["message"], "Hurry up.");
        assert_eq!(d.node("nudges_left").unwrap().config["conditions"][0]["right"], 1);
        assert_eq!(d.node("stop_at_limit").unwrap().config["conditions"][0]["left"], "stop");
        assert_eq!(d.node("restarts_left").unwrap().config["conditions"][0]["right"], 0);
        let v = values_of(&d);
        assert_eq!(
            (v["nudges"].clone(), v["restart"].clone(), v["laterNudge"].clone()),
            (json!(1), json!(false), json!(LATER_NUDGE))
        );
        assert!(!customized(&d));
        let mut edited = d.clone();
        edited.nodes.retain(|n| n.id != "nudge_again");
        assert!(customized(&edited));
        let mut renamed = d.clone();
        renamed.name = "Stuck agents".into();
        assert!(!customized(&renamed), "a new name is not a customization");
        assert!(filled(json!({"onLoopLimit": "maybe"}).as_object().unwrap()).is_err());
    }
}
