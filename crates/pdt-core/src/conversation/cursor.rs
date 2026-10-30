//! Cursor's `cursor-agent acp` (PDT-237; notes: docs/superpowers/notes/cursor-acp.md): plain ACP (`super::acp`) plus
//! its tool calls' `rawInput` (an MCP call names its server and tool there), and its own blocking requests:
//! `cursor/ask_question` (multiple-choice questions) and `cursor/create_plan` (a plan to approve). What a built-in
//! profile's flavor adds ([`AcpFlavor`]) is mapped here.

use super::acp::{AcpExtensions, ExtAnswer, ExtQuestion, ExtRequest};
use serde_json::{Value, json};

/// What Cursor adds to ACP.
#[derive(Debug, Clone, Copy, Default)]
pub struct CursorExtensions;

const ASK_QUESTION: &str = "cursor/ask_question";
const CREATE_PLAN: &str = "cursor/create_plan";

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|x| !x.is_empty()).map(String::from)
}

impl AcpExtensions for CursorExtensions {
    fn name(&self) -> &str {
        "Cursor"
    }

    /// An MCP call's `rawInput` is `{providerIdentifier, toolName, args}`: named `mcp__<server>__<tool>`, as Claude
    /// names them. Cursor's own tools sometimes give `_toolName` (`task`, `updateTodos`).
    fn tool_name(&self, update: &Value) -> Option<String> {
        let input = update.get("rawInput")?;
        match (s(input, "providerIdentifier"), s(input, "toolName")) {
            (Some(server), Some(tool)) => Some(format!("mcp__{server}__{tool}")),
            _ => s(input, "_toolName"),
        }
    }

    fn pdt_tool_prefix(&self) -> Option<&str> {
        Some(PDT_TOOL_PREFIX)
    }

    /// A command's result is `rawOutput` `{exitCode, stdout, stderr}`; its `content` is empty.
    fn raw_output(&self, update: &Value) -> Option<(String, Option<i32>)> {
        let out = update.get("rawOutput")?;
        let code = out.get("exitCode").and_then(Value::as_i64).and_then(|c| i32::try_from(c).ok());
        let text: Vec<&str> = ["stdout", "stderr"]
            .iter()
            .filter_map(|k| out.get(*k).and_then(Value::as_str).filter(|x| !x.trim().is_empty()))
            .collect();
        (code.is_some() || !text.is_empty()).then(|| (text.join("\n"), code))
    }

    fn ext_request(&self, method: &str, p: &Value) -> Option<ExtRequest> {
        match method {
            ASK_QUESTION => {
                let title = s(p, "title");
                let questions = p.get("questions").and_then(Value::as_array)?;
                let questions: Vec<ExtQuestion> = questions
                    .iter()
                    .filter_map(|q| {
                        let options = q
                            .get("options")
                            .and_then(Value::as_array)
                            .map(|o| o.iter().filter_map(|o| Some((s(o, "id")?, s(o, "label")?))).collect())
                            .unwrap_or_default();
                        Some(ExtQuestion {
                            id: s(q, "id")?,
                            question: s(q, "prompt")?,
                            header: title.clone(),
                            options,
                            multi: q.get("allowMultiple").and_then(Value::as_bool).unwrap_or(false),
                        })
                    })
                    .collect();
                (!questions.is_empty()).then_some(ExtRequest::Questions(questions))
            }
            CREATE_PLAN => Some(ExtRequest::Plan { markdown: plan_markdown(p) }),
            _ => None,
        }
    }

    fn ext_answer(&self, method: &str, answer: &ExtAnswer) -> Value {
        let outcome = match (method, answer) {
            (_, ExtAnswer::Cancelled) => json!({"outcome": "cancelled"}),
            (ASK_QUESTION, ExtAnswer::Answered(picked)) => json!({
                "outcome": "answered",
                "answers": picked.iter().map(|(id, options)| json!({"questionId": id, "selectedOptionIds": options}))
                    .collect::<Vec<_>>(),
            }),
            (ASK_QUESTION, ExtAnswer::Skipped { reason }) => {
                json!({"outcome": "skipped", "reason": reason.as_deref().unwrap_or("The user skipped the questions")})
            }
            (CREATE_PLAN, ExtAnswer::Plan { approve: true, .. }) => json!({"outcome": "accepted"}),
            (CREATE_PLAN, ExtAnswer::Plan { approve: false, feedback }) => match feedback {
                Some(f) => json!({"outcome": "rejected", "reason": f}),
                None => json!({"outcome": "rejected"}),
            },
            _ => json!({"outcome": "cancelled"}),
        };
        json!({"outcome": outcome})
    }
}

/// The prefix Cursor's MCP calls to pdt-mcp get from [`CursorExtensions::tool_name`] (Cursor's MCP clients go by the
/// `name` PDT gives the server in `mcpServers`: `crate::mcp::SERVER_NAME`).
const PDT_TOOL_PREFIX: &str = "mcp__pdt__";

/// A `cursor/create_plan` request's plan as Markdown: its name, overview and plan (its to-dos also arrive as a
/// `plan` update).
fn plan_markdown(p: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(name) = s(p, "name") {
        parts.push(format!("# {name}"));
    }
    if let Some(overview) = s(p, "overview") {
        parts.push(overview);
    }
    if let Some(plan) = s(p, "plan") {
        parts.push(plan);
    }
    parts.join("\n\n")
}

/// `session/load` failed because Cursor has no saved session by that id: it answers `-32602` (invalid params) with
/// `Session "<id>" not found` in the error's `data.message`.
pub fn session_missing(code: i64, message: &str) -> bool {
    code == -32602 && message.contains("not found")
}

#[cfg(test)]
mod tests {
    use super::super::acp::{AcpMapper, AcpOut};
    use super::super::{AgentEvent, Answer, PermissionMode, RequestKind, ToolStatus, TurnPhase, TurnStatus};
    use super::*;
    use crate::acp_agent::AcpFlavor;
    use std::collections::BTreeMap;

    /// `cursor-agent acp`'s answers to `initialize`, `session/new` (models cut to three), `session/set_mode` (plan,
    /// ask, an unknown id, agent), a prompt on Cursor's Free plan, and `session/load` of an id with nothing saved
    /// (recorded from Cursor CLI 2026.10.22-27dcf5f).
    const START: &str = include_str!("../../tests/fixtures/cursor/start.jsonl");
    /// A turn in Cursor 2026.10.22's frames (from its ACP mode's source: the permission gate, the tool-call
    /// presenter, `cursor/ask_question`): the prompt echoed back, two pdt-mcp calls (the second asked about before
    /// its `tool_call` has its `rawInput`, as Cursor does live), a command, an edit, two questions.
    const TURN: &str = include_str!("../../tests/fixtures/cursor/turn.jsonl");
    /// Plan mode's `cursor/create_plan`, after the plan's to-dos as a `plan` update.
    const PLAN: &str = include_str!("../../tests/fixtures/cursor/plan.jsonl");

    fn mapper(mode: PermissionMode) -> AcpMapper<AcpFlavor> {
        let mut m = AcpMapper::with_extensions(AcpFlavor::Cursor, "/work/repo", None, "p");
        // PDT keeps every mode but Plan itself (`acp_agent::cursor`).
        m.enforce_mode((mode != PermissionMode::Plan).then_some(mode));
        m
    }

    /// Feeds frames as the adapter would, answering requests with `answer`; returns the events and what PDT sent back
    /// (its own answers and the user's), and the answers' follow-up messages and modes.
    #[allow(clippy::type_complexity)]
    fn feed(
        m: &mut AcpMapper<AcpFlavor>,
        frames: &str,
        mut answer: impl FnMut(&AgentEvent) -> Option<Answer>,
    ) -> (Vec<AgentEvent>, Vec<Value>, Vec<(Option<String>, Option<PermissionMode>)>) {
        let mut out = m.begin_prompt("go");
        let mut sent = Vec::new();
        let mut after = Vec::new();
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
                            let a = m.answer(id, &a).unwrap();
                            sent.push(a.result);
                            after.push((a.follow_up, a.mode));
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
        (events, sent, after)
    }

    fn last_tool<'a>(ev: &'a [AgentEvent], id: &str) -> &'a AgentEvent {
        ev.iter().rev().find(|e| matches!(e, AgentEvent::ToolCall { id: i, .. } if i.ends_with(id))).unwrap()
    }

    #[test]
    fn cursor_starts_with_session_loading_its_modes_and_mcp_servers() {
        let frames: Vec<Value> = START.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let caps = &frames[0]["result"]["agentCapabilities"];
        assert_eq!(caps["loadSession"], true, "PDT resumes with session/load");
        assert_eq!(caps["mcpCapabilities"]["http"], true, "pdt-mcp goes in mcpServers");
        let modes = &frames[1]["result"]["modes"];
        assert_eq!(modes["currentModeId"], "agent");
        let ids: Vec<&str> =
            modes["availableModes"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
        assert_eq!(ids, ["agent", "plan", "ask"]);
        // Its `ask` is read-only Q&A, which reads as PDT's Ask by name: the profile maps only `plan`.
        let profile = crate::acp_agent::find(&[], "cursor").unwrap();
        assert_eq!(profile.modes, vec![(PermissionMode::Plan, "plan".to_string())]);
        // session/set_mode works and reports the change; an unknown id is refused with the valid ones.
        let reported: Vec<&str> =
            frames.iter().filter_map(|f| f["params"]["update"]["currentModeId"].as_str()).collect();
        assert_eq!(reported, ["plan", "ask", "agent"]);
        let bogus = frames.iter().find(|f| f["id"] == 5).unwrap();
        assert!(bogus["error"]["data"]["details"].as_str().unwrap().contains("Valid modes: agent, plan, ask"));
        // On the Free plan a prompt ends normally with a message saying so.
        let refused = frames.iter().find(|f| f["params"]["update"]["sessionUpdate"] == "agent_message_chunk").unwrap();
        assert!(refused["params"]["update"]["content"]["text"].as_str().unwrap().contains("Upgrade your plan"));
        // Nothing saved under the id: -32602 with the reason in data.message, and PDT starts a new session.
        let load = frames.iter().find(|f| f["id"] == 10).unwrap();
        let message = format!("Invalid params: {}", load["error"]["data"]["message"].as_str().unwrap());
        assert!(AcpFlavor::Cursor.session_missing(load["error"]["code"].as_i64().unwrap(), &message));
    }

    #[test]
    fn a_cursor_turn_runs_pdt_tools_and_edits_asks_for_commands_and_maps_its_questions() {
        // Accept edits: what a Cursor session in Ask runs as (Cursor edits without asking).
        let mut m = mapper(PermissionMode::AcceptEdits);
        let mut approvals = Vec::new();
        let (ev, sent, _) = feed(&mut m, TURN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Approval, tool, can_always, .. } => {
                // Cursor's "Allow always" writes its own allowlist; PDT never offers it.
                assert!(!*can_always);
                approvals.push(tool.clone());
                Some(Answer::Approve { always: false })
            }
            AgentEvent::Request { request_kind: RequestKind::Question, input, title, .. } => {
                assert_eq!(title, "Which colour?");
                let qs = input["questions"].as_array().unwrap();
                assert_eq!(
                    (qs[0]["header"].as_str(), qs[0]["options"][1]["label"].as_str()),
                    (Some("Need input"), Some("Blue"))
                );
                assert_eq!(qs[1]["multiSelect"], true);
                Some(Answer::Question {
                    answers: BTreeMap::from([
                        ("Which colour?".to_string(), vec!["Blue".to_string()]),
                        ("Which extras?".to_string(), vec!["Tests".to_string(), "Benchmarks".to_string()]),
                    ]),
                })
            }
            _ => None,
        });
        assert_eq!(approvals, ["execute"], "pdt-mcp calls and edits never ask");
        assert_eq!(
            sent,
            vec![
                json!({"outcome": {"outcome": "selected", "optionId": "allow-once"}}),
                json!({"outcome": {"outcome": "selected", "optionId": "allow-once"}}),
                json!({"outcome": {"outcome": "selected", "optionId": "allow-once"}}),
                json!({"outcome": {"outcome": "selected", "optionId": "allow-once"}}),
                json!({"outcome": {"outcome": "answered", "answers": [
                    {"questionId": "q1", "selectedOptionIds": ["blue"]},
                    {"questionId": "q2", "selectedOptionIds": ["tests", "bench"]},
                ]}}),
            ]
        );
        // The prompt Cursor echoes back is not a second user message.
        let users = ev.iter().filter(|e| matches!(e, AgentEvent::UserMessage { .. })).count();
        assert_eq!(users, 1);
        let AgentEvent::ToolCall { tool, status, .. } = last_tool(&ev, "tool_mcp1") else { panic!() };
        assert_eq!((tool.as_str(), *status), ("mcp__pdt__get_ticket", ToolStatus::Done));
        let AgentEvent::ToolCall { tool, .. } = last_tool(&ev, "tool_mcp2") else { panic!() };
        assert_eq!(tool, "mcp__pdt__submit_outcome", "named once its rawInput arrives");
        let AgentEvent::ToolCall { title, output, exit_code, .. } = last_tool(&ev, "tool_sh1") else { panic!() };
        assert_eq!((title.as_str(), output.as_deref(), *exit_code), ("ls", Some("README.md"), Some(0)));
        let AgentEvent::ToolCall { diff, .. } = last_tool(&ev, "tool_ed1") else { panic!() };
        assert!(diff.as_deref().unwrap().contains("+hello world"));
        assert!(matches!(
            ev.last().unwrap(),
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
    }

    #[test]
    fn bypass_allows_everything_and_own_words_skip_the_questions_with_them() {
        let mut m = mapper(PermissionMode::Bypass);
        let (_, sent, _) = feed(&mut m, TURN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Approval, .. } => panic!("Bypass asks nothing"),
            AgentEvent::Request { request_kind: RequestKind::Question, .. } => Some(Answer::Question {
                answers: BTreeMap::from([("Which colour?".to_string(), vec!["Green".to_string()])]),
            }),
            _ => None,
        });
        assert!(sent[..4].iter().all(|s| s["outcome"]["optionId"] == "allow-once"), "{sent:?}");
        assert_eq!(sent[4]["outcome"]["outcome"], "skipped");
        assert!(sent[4]["outcome"]["reason"].as_str().unwrap().contains("Which colour?: Green"));
    }

    #[test]
    fn skipped_questions_and_cancelled_turns_answer_in_cursors_terms() {
        let mut m = mapper(PermissionMode::Bypass);
        let (_, sent, _) = feed(&mut m, TURN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Question, .. } => {
                Some(Answer::Deny { message: Some("Pick for me".into()) })
            }
            _ => None,
        });
        assert_eq!(sent[4], json!({"outcome": {"outcome": "skipped", "reason": "Pick for me"}}));
        // An open question when the turn is cancelled.
        let mut m = mapper(PermissionMode::Bypass);
        let _ = m.begin_prompt("go");
        let q = TURN.lines().find(|l| l.contains("cursor/ask_question")).unwrap();
        let v: Value = serde_json::from_str(q).unwrap();
        let _ = m.on_request(v["id"].clone(), "cursor/ask_question", &v["params"]);
        let (replies, _) = m.cancel_requests();
        assert_eq!(replies, vec![(json!(103), json!({"outcome": {"outcome": "cancelled"}}))]);
    }

    #[test]
    fn create_plan_is_a_plan_request_whose_approval_builds_in_the_mode_picked() {
        let mut m = mapper(PermissionMode::Plan);
        let (ev, sent, after) = feed(&mut m, PLAN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Plan, input, .. } => {
                let plan = input["plan"].as_str().unwrap();
                assert!(plan.starts_with("# Rename a.txt\n\nRename a.txt to b.txt."), "{plan}");
                assert!(plan.contains("git mv a.txt b.txt"));
                Some(Answer::Plan { approve: true, feedback: None, mode: Some(PermissionMode::AcceptEdits) })
            }
            _ => None,
        });
        assert_eq!(sent, vec![json!({"outcome": {"outcome": "accepted"}})]);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].1, Some(PermissionMode::AcceptEdits), "PDT leaves Cursor's plan mode itself");
        assert!(after[0].0.as_deref().unwrap().contains("implement"));
        assert!(ev.iter().any(|e| matches!(e, AgentEvent::Plan { markdown } if markdown.contains("git mv"))));

        // Kept planning: rejected with the feedback, no mode change.
        let mut m = mapper(PermissionMode::Plan);
        let (_, sent, after) = feed(&mut m, PLAN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Plan, .. } => {
                Some(Answer::Plan { approve: false, feedback: Some("Keep a.txt".into()), mode: None })
            }
            _ => None,
        });
        assert_eq!(sent, vec![json!({"outcome": {"outcome": "rejected", "reason": "Keep a.txt"}})]);
        assert_eq!(after, vec![(None, None)]);
    }

    #[test]
    fn plain_flavor_keeps_plain_acp() {
        let update = json!({"rawInput": {"providerIdentifier": "pdt", "toolName": "get_ticket"}});
        assert_eq!(AcpFlavor::Plain.tool_name(&update), None);
        assert_eq!(AcpFlavor::Cursor.tool_name(&update).as_deref(), Some("mcp__pdt__get_ticket"));
        assert_eq!(AcpFlavor::Plain.ext_request("cursor/create_plan", &json!({})), None);
        assert_eq!(AcpFlavor::Plain.raw_output(&json!({"rawOutput": {"exitCode": 1}})), None);
        assert!(AcpFlavor::Plain.session_missing(-32002, "Resource not found"));
        assert!(!AcpFlavor::Plain.session_missing(-32602, "Invalid params: Session \"x\" not found"));
        assert!(!AcpFlavor::Cursor.session_missing(-32602, "Invalid params: Invalid model value: x"));
    }
}
