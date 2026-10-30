//! OhMyPi's `omp acp` (PDT-238; notes: docs/superpowers/notes/omp-acp.md): plain ACP (`super::acp`) plus a tool call's
//! `name`, and its `ask` tool's forms, where each question is a field's `title` (its header the `description`) with a
//! `<field>__other` field beside it for an answer in the user's own words. What a built-in profile's flavor adds
//! ([`AcpFlavor`]) is mapped here.

use super::acp::AcpExtensions;
use serde_json::Value;

/// What OhMyPi adds to ACP.
#[derive(Debug, Clone, Copy, Default)]
pub struct OmpExtensions;

/// The suffix of the field beside each of an `ask` form's questions that takes the user's own words.
const OTHER_SUFFIX: &str = "__other";

impl AcpExtensions for OmpExtensions {
    fn name(&self) -> &str {
        "OhMyPi"
    }

    /// `tool_call` carries the tool's own name (`bash`, `edit`, `read`) beside its ACP kind.
    fn tool_name(&self, update: &Value) -> Option<String> {
        update.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(String::from)
    }

    fn question_in_title(&self) -> bool {
        true
    }

    fn other_field(&self, key: &str) -> Option<String> {
        (!key.ends_with(OTHER_SUFFIX)).then(|| format!("{key}{OTHER_SUFFIX}"))
    }
}

/// `session/load` failed because OhMyPi has no saved session by that id: it answers `-32603` (internal error) with
/// "ACP session not found" in the error's details.
pub fn session_missing(code: i64, message: &str) -> bool {
    code == -32603 && message.contains("session not found")
}

#[cfg(test)]
mod tests {
    use super::super::acp::{AcpMapper, AcpOut};
    use super::super::{AgentEvent, Answer, RequestKind, ToolStatus, TurnPhase, TurnStatus};
    use super::*;
    use crate::acp_agent::AcpFlavor;
    use serde_json::json;
    use std::collections::BTreeMap;

    /// `omp acp`'s answers to `initialize` and `session/new`, a prompt with no model set up, and the session's first
    /// updates (recorded from omp 18.14.0).
    const START: &str = include_str!("../../tests/fixtures/omp/start.jsonl");
    /// A turn recorded from omp 18.14.0 (Claude as its model): an MCP tool call (omp writes to `xd://mcp__<tool>`),
    /// `ls`, which it asks to run before it reports the call, an edit, and two questions in one `ask` form.
    const TURN: &str = include_str!("../../tests/fixtures/omp/turn.jsonl");
    /// Plan mode, recorded likewise: the plan written, its approval (a one-field select form), omp back in `default`,
    /// and the approved command.
    const PLAN: &str = include_str!("../../tests/fixtures/omp/plan.jsonl");

    fn mapper() -> AcpMapper<AcpFlavor> {
        AcpMapper::with_extensions(AcpFlavor::Omp, "/work/repo", None, "p")
    }

    /// Feeds frames as the adapter would, answering requests with `answer`; returns the events and what PDT sent back.
    fn feed(
        m: &mut AcpMapper<AcpFlavor>,
        frames: &str,
        mut answer: impl FnMut(&AgentEvent) -> Option<Answer>,
    ) -> (Vec<AgentEvent>, Vec<Value>) {
        let mut out = m.begin_prompt("go");
        let mut sent = Vec::new();
        for line in frames.lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).unwrap();
            match (v.get("method").and_then(Value::as_str), v.get("id")) {
                (Some("session/update"), None) => out.extend(m.on_update(&v["params"]["update"])),
                (Some(method), Some(id)) => {
                    let evs = m.on_request(id.clone(), method, &v["params"]);
                    for e in &evs {
                        if let AcpOut::Respond { result, .. } = e {
                            sent.push(result.clone());
                        }
                        if let AcpOut::Event { event: ev @ AgentEvent::Request { id, .. }, .. } = e
                            && let Some(a) = answer(ev)
                        {
                            sent.push(m.answer(id, &a).unwrap().result);
                        }
                    }
                    out.extend(evs);
                }
                (None, Some(_)) => out.extend(m.end_prompt(Ok(&v["result"]))),
                _ => {}
            }
        }
        let events = out
            .into_iter()
            .filter_map(|o| match o {
                AcpOut::Event { event, .. } => Some(event),
                _ => None,
            })
            .collect();
        (events, sent)
    }

    fn last_tool<'a>(ev: &'a [AgentEvent], id: &str) -> &'a AgentEvent {
        ev.iter().rev().find(|e| matches!(e, AgentEvent::ToolCall { id: i, .. } if i.ends_with(id))).unwrap()
    }

    #[test]
    fn omp_starts_with_session_loading_its_modes_and_mcp_servers() {
        let frames: Vec<Value> = START.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let caps = &frames[0]["result"]["agentCapabilities"];
        assert_eq!(caps["loadSession"], true, "PDT resumes with session/load");
        assert!(caps["mcpCapabilities"].is_object());
        let modes = &frames[1]["result"]["modes"];
        assert_eq!(
            crate::acp_agent::advertised_modes(&modes["availableModes"]),
            vec![
                (crate::conversation::PermissionMode::Ask, "default".into()),
                (crate::conversation::PermissionMode::Plan, "plan".into())
            ]
        );
        assert_eq!(modes["currentModeId"], "default");
        // With no model set up, a prompt fails with an internal error whose details say why.
        assert_eq!(frames[2]["error"]["code"], -32603);
        assert!(frames[2]["error"]["data"]["details"].as_str().unwrap().starts_with("No model selected."));
    }

    #[test]
    fn an_omp_turn_maps_tools_by_name_and_its_ask_forms_to_questions() {
        let mut m = mapper();
        let (ev, sent) = feed(&mut m, TURN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Approval, tool, title, can_always, .. } => {
                // Asked before omp reports the call: the request's own call names it.
                assert_eq!((tool.as_str(), title.as_str()), ("execute", "ls"));
                // omp's "Always allow" is its own rule for the process; PDT never offers it.
                assert!(!*can_always);
                Some(Answer::Approve { always: false })
            }
            AgentEvent::Request { request_kind: RequestKind::Question, input, title, .. } => {
                assert_eq!(title, "Which colour?");
                let qs = input["questions"].as_array().unwrap();
                assert_eq!(qs.len(), 2, "the own-words fields are no questions of their own: {qs:?}");
                assert_eq!(
                    (qs[0]["question"].as_str(), qs[0]["header"].as_str()),
                    (Some("Which colour?"), Some("Colour"))
                );
                assert_eq!(qs[0]["options"][1]["label"], "Blue");
                assert_eq!(
                    (qs[1]["question"].as_str(), qs[1]["multiSelect"].as_bool()),
                    (Some("Which extras?"), Some(true))
                );
                let answers = BTreeMap::from([
                    ("Which colour?".to_string(), vec!["Blue".to_string()]),
                    ("Which extras?".to_string(), vec!["Tests".to_string(), "Benchmarks".to_string()]),
                ]);
                Some(Answer::Question { answers })
            }
            _ => None,
        });
        assert_eq!(sent[0], json!({"outcome": {"outcome": "selected", "optionId": "allow_once"}}));
        // Own words go in the question's `__other` field (what omp was sent when this was recorded).
        assert_eq!(
            sent[1],
            json!({"action": "accept", "content": {"q0": "Blue", "q1": ["Tests"], "q1__other": "Benchmarks"}})
        );
        let AgentEvent::ToolCall { tool, output, .. } = last_tool(&ev, "xi6bHf") else { panic!() };
        assert_eq!((tool.as_str(), output.as_deref()), ("write", Some("the secret word is PAPAYA")));
        let AgentEvent::ToolCall { tool, status, title, .. } = last_tool(&ev, "3XWDp5") else { panic!() };
        assert_eq!((tool.as_str(), title.as_str(), *status), ("bash", "ls", ToolStatus::Done));
        let AgentEvent::ToolCall { tool, diff, .. } = last_tool(&ev, "JWM4mL") else { panic!() };
        assert_eq!(tool, "edit");
        assert!(diff.as_deref().unwrap().contains("+goodbye"), "{diff:?}");
        assert!(ev.iter().any(|e| matches!(e, AgentEvent::Usage { context_used: Some(14279), .. })));
        assert!(
            ev.iter().any(|e| matches!(e, AgentEvent::AgentMessage { markdown, .. } if markdown.contains("PAPAYA")))
        );
        assert!(matches!(
            ev.last().unwrap(),
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
    }

    #[test]
    fn an_answer_in_own_words_alone_leaves_the_choice_out() {
        let mut m = mapper();
        let (_, sent) = feed(&mut m, TURN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Approval, .. } => Some(Answer::Deny { message: None }),
            AgentEvent::Request { request_kind: RequestKind::Question, .. } => Some(Answer::Question {
                answers: BTreeMap::from([("Which colour?".to_string(), vec!["Green".to_string()])]),
            }),
            _ => None,
        });
        assert_eq!(sent[0]["outcome"]["optionId"], "reject_once");
        assert_eq!(sent[1], json!({"action": "accept", "content": {"q0__other": "Green"}}));
    }

    #[test]
    fn plan_approval_is_a_question_with_the_plan_in_it() {
        let mut m = mapper();
        let (ev, sent) = feed(&mut m, PLAN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Question, input, .. } => {
                let q = &input["questions"][0];
                let text = q["question"].as_str().unwrap();
                assert!(text.starts_with("Approve plan \"rename-a-to-b\""), "{text}");
                assert!(text.contains("mv a.txt b.txt"), "the plan is in it");
                assert_eq!(q["options"][0]["label"], "Approve and execute");
                Some(Answer::Question {
                    answers: BTreeMap::from([(text.to_string(), vec!["Approve and execute".to_string()])]),
                })
            }
            AgentEvent::Request { request_kind: RequestKind::Approval, title, .. } => {
                assert_eq!(title, "test ! -e b.txt && mv a.txt b.txt && ls");
                Some(Answer::Approve { always: false })
            }
            _ => None,
        });
        assert_eq!(sent[0], json!({"action": "accept", "content": {"value": "Approve and execute"}}));
        assert_eq!(sent[1]["outcome"]["optionId"], "allow_once");
        assert!(matches!(
            ev.last().unwrap(),
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
    }

    #[test]
    fn approval_leaves_plan_mode() {
        let mut m = mapper();
        m.begin_prompt("go");
        let modes: Vec<String> = PLAN
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["method"] == "session/update")
            .flat_map(|v| m.on_update(&v["params"]["update"]))
            .filter_map(|o| match o {
                AcpOut::Settings { mode: Some(mode), .. } => Some(mode),
                _ => None,
            })
            .collect();
        // Its `current_mode_update` and `config_option_update` both say so.
        assert!(
            !modes.is_empty() && modes.iter().all(|m| m == "default"),
            "omp leaves plan mode once approved: {modes:?}"
        );
    }

    #[test]
    fn a_missing_saved_session_is_told_apart_from_other_errors() {
        assert!(AcpFlavor::Omp.session_missing(-32603, "Internal error: ACP session not found: 01a1"));
        assert!(!AcpFlavor::Omp.session_missing(-32603, "Internal error: No model selected."));
        assert!(AcpFlavor::Plain.session_missing(-32002, "Resource not found"));
        assert!(!AcpFlavor::Plain.session_missing(-32603, "Internal error: ACP session not found"));
    }

    #[test]
    fn plain_flavor_keeps_plain_acp_forms() {
        assert_eq!(AcpFlavor::Plain.other_field("q0"), None);
        assert!(!AcpFlavor::Plain.question_in_title());
        assert_eq!(AcpFlavor::Plain.tool_name(&json!({"name": "bash"})), None);
        assert_eq!(AcpFlavor::Omp.tool_name(&json!({"name": "bash"})).as_deref(), Some("bash"));
        assert_eq!(AcpFlavor::Omp.other_field("q0__other"), None);
    }
}
