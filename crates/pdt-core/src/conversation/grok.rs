//! Grok Build's `grok agent --no-leader stdio` (PDT-236): the Agent Client Protocol mapping (`super::acp`) with what
//! Grok adds to it. Grok advertises no session modes over ACP, so PDT keeps the session's mode by answering its
//! permission requests (`crate::acp_agent::enforced_answer`). Its own `_x.ai/*` notifications (setup phases, MCP
//! status, retries, queue and usage) are not `session/update`s and are ignored.

use super::acp::AcpExtensions;
use serde_json::Value;

/// The name prefix Grok gives MCP tools (`<server>__<tool>`): pdt-mcp's are `pdt__get_ticket`.
const PDT_TOOL_PREFIX: &str = "pdt__";

/// The permission option that allows every edit for this session only (Grok remembers nothing from it). Grok's
/// `always-allow` saves a rule for the project, which PDT never picks.
const EDITS_THIS_SESSION: &str = "allow-edits-session";

/// What Grok adds to ACP.
#[derive(Debug, Clone, Copy, Default)]
pub struct GrokExtensions;

impl AcpExtensions for GrokExtensions {
    fn name(&self) -> &str {
        "Grok"
    }

    /// Grok reports `/private/tmp/…` for `/tmp/…` (a read's path, the git root).
    fn resolves_symlinks(&self) -> bool {
        true
    }

    /// Grok's tool (`_meta["x.ai/tool"].name`: `write`, `run_terminal_command`, `read_file`); an MCP tool, which
    /// Grok calls through its `use_tool`, by the MCP tool's name (`pdt__get_ticket`).
    fn tool_name(&self, update: &Value) -> Option<String> {
        let name = update.get("_meta")?.get("x.ai/tool")?.get("name")?.as_str()?;
        let mcp = update.pointer("/rawInput/tool_name").and_then(Value::as_str).filter(|_| name == "use_tool");
        Some(mcp.unwrap_or(name).to_string())
    }

    /// A call the user refused ends `failed` with `_meta.toolNotRun: "rejected"`.
    fn rejected(&self, update: &Value) -> bool {
        update.pointer("/_meta/toolNotRun").and_then(Value::as_str) == Some("rejected")
    }

    fn session_option(&self) -> Option<&str> {
        Some(EDITS_THIS_SESSION)
    }

    fn pdt_tool_prefix(&self) -> Option<&str> {
        Some(PDT_TOOL_PREFIX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::acp::{AcpMapper, AcpOut};
    use crate::conversation::{
        AgentEvent, Answer, PermissionMode, RequestKind, RequestState, ToolStatus, TurnPhase, TurnStatus,
    };
    use serde_json::json;

    // Recorded from grok 1.0.70 (`grok agent --no-leader stdio`), one turn each: frames from the agent only.
    /// Writes a file (asks: an edit) and runs `ls` (Grok's own read-only list: no request).
    const EDIT: &str = include_str!("../../tests/fixtures/grok/edit.jsonl");
    /// Calls pdt-mcp's `get_ticket` (asks), reads a file and runs `touch` (neither asks).
    const MCP: &str = include_str!("../../tests/fixtures/grok/mcp.jsonl");
    /// Runs `echo two > two.txt` (asks: a command), allowed.
    const EXECUTE: &str = include_str!("../../tests/fixtures/grok/execute.jsonl");
    /// Runs `echo one > one.txt` (asks), refused: Grok ends the turn `cancelled`.
    const REJECT: &str = include_str!("../../tests/fixtures/grok/reject.jsonl");
    /// `sleep 30`, cancelled by `session/cancel` while it runs.
    const CANCEL: &str = include_str!("../../tests/fixtures/grok/cancel.jsonl");
    /// A `session/load` in a new process: the earlier turns replayed.
    const LOAD: &str = include_str!("../../tests/fixtures/grok/load.jsonl");

    const CWD: &str = "/tmp/grokprobe/cwd";

    fn mapper(mode: Option<PermissionMode>) -> AcpMapper<GrokExtensions> {
        let mut m = AcpMapper::with_extensions(GrokExtensions, CWD, Some(format!("/private{CWD}")), "p");
        m.enforce_mode(mode);
        m
    }

    /// Feeds frames as the adapter would, starting the turn first and answering requests with `answer`; what PDT
    /// sent back for a request the user answered is recorded as `Unknown { raw: {sent, followUp} }`.
    fn feed(
        m: &mut AcpMapper<GrokExtensions>,
        frames: &str,
        mut answer: impl FnMut(&AgentEvent) -> Option<Answer>,
    ) -> Vec<AcpOut> {
        let mut out = m.begin_prompt("go");
        for line in frames.lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).unwrap();
            match (v.get("method").and_then(Value::as_str), v.get("id")) {
                (Some("session/update"), None) => out.extend(m.on_update(&v["params"]["update"])),
                (Some(method), Some(id)) => {
                    let evs = m.on_request(id.clone(), method, &v["params"]);
                    let ask = evs.iter().find_map(|e| match e {
                        AcpOut::Event { event: ev @ AgentEvent::Request { .. }, .. } => Some(ev.clone()),
                        _ => None,
                    });
                    out.extend(evs);
                    if let Some(ev) = ask
                        && let AgentEvent::Request { id, .. } = &ev
                        && let Some(a) = answer(&ev)
                    {
                        let a = m.answer(id, &a).unwrap();
                        out.push(AcpOut::Event {
                            item_id: None,
                            turn_id: None,
                            event: AgentEvent::Unknown { raw: json!({"sent": a.result, "followUp": a.follow_up}) },
                        });
                        out.extend(a.events);
                    }
                }
                (None, Some(_)) => out.extend(m.end_prompt(Ok(&v["result"]))),
                _ => {}
            }
        }
        out
    }

    fn events(out: &[AcpOut]) -> Vec<AgentEvent> {
        out.iter()
            .filter_map(|o| match o {
                AcpOut::Event { event, .. } => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    /// The requests the user was asked.
    fn requests(ev: &[AgentEvent]) -> Vec<(RequestKind, String, String, bool)> {
        ev.iter()
            .filter_map(|e| match e {
                AgentEvent::Request { request_kind, tool, title, can_always, state: RequestState::Open, .. } => {
                    Some((*request_kind, tool.clone(), title.clone(), *can_always))
                }
                _ => None,
            })
            .collect()
    }

    /// The options PDT picked itself (an enforced mode, PDT's own tools).
    fn picked(out: &[AcpOut]) -> Vec<String> {
        out.iter()
            .filter_map(|o| match o {
                AcpOut::Respond { result, .. } => result["outcome"]["optionId"].as_str().map(String::from),
                _ => None,
            })
            .collect()
    }

    /// What PDT sent for the requests the user answered.
    fn sent(ev: &[AgentEvent]) -> Vec<Value> {
        ev.iter()
            .filter_map(|e| match e {
                AgentEvent::Unknown { raw } => Some(raw.clone()),
                _ => None,
            })
            .collect()
    }

    fn tool<'a>(ev: &'a [AgentEvent], name: &str) -> (&'a str, ToolStatus, Option<&'a str>, Option<&'a str>) {
        ev.iter()
            .rev()
            .find_map(|e| match e {
                AgentEvent::ToolCall { tool, title, status, output, diff, .. } if tool == name => {
                    Some((title.as_str(), *status, output.as_deref(), diff.as_deref()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {name} call"))
    }

    fn ended(ev: &[AgentEvent]) -> Option<TurnStatus> {
        match ev.last() {
            Some(AgentEvent::Turn { phase: TurnPhase::Ended, status, .. }) => *status,
            _ => None,
        }
    }

    #[test]
    fn an_edit_asks_in_ask_mode_and_always_allows_edits_for_this_session_only() {
        let mut m = mapper(Some(PermissionMode::Ask));
        let out = feed(&mut m, EDIT, |_| Some(Answer::Approve { always: true }));
        let ev = events(&out);
        assert_eq!(requests(&ev), vec![(RequestKind::Approval, "write".into(), "hello.txt".into(), true)]);
        assert_eq!(sent(&ev)[0]["sent"]["outcome"]["optionId"], "allow-edits-session", "never Grok's saved rule");
        let (title, status, _, diff) = tool(&ev, "write");
        assert_eq!((title, status), ("hello.txt", ToolStatus::Done));
        // Grok gives a new file's old text as "", not null.
        assert_eq!(diff, Some("--- a/hello.txt\n+++ b/hello.txt\n@@ -0,0 +1 @@\n+hi\n"));
        // Grok runs its own read-only commands without asking.
        let (title, status, output, _) = tool(&ev, "run_terminal_command");
        assert_eq!((title, status, output), ("ls", ToolStatus::Done, Some("hello.txt")));
        assert!(ev.iter().any(|e| matches!(e, AgentEvent::TurnDiff { diff } if diff.contains("+hi"))));
        assert!(ev.iter().any(|e| matches!(e, AgentEvent::AgentMessage { .. })));
        assert_eq!(ended(&ev), Some(TurnStatus::Completed));
    }

    #[test]
    fn pdt_keeps_accept_edits_plan_and_bypass_by_answering_for_grok() {
        // Accept edits: the write is allowed without asking.
        let mut m = mapper(Some(PermissionMode::AcceptEdits));
        let out = feed(&mut m, EDIT, |_| panic!("nothing asks"));
        assert_eq!(picked(&out), vec!["allow-once"]);
        assert_eq!(ended(&events(&out)), Some(TurnStatus::Completed));
        // Plan: it is refused.
        let mut m = mapper(Some(PermissionMode::Plan));
        assert_eq!(picked(&feed(&mut m, EDIT, |_| panic!("nothing asks"))), vec!["reject-once"]);
        // A command still asks in Accept edits, is refused in Plan and allowed in Bypass.
        let mut m = mapper(Some(PermissionMode::AcceptEdits));
        let out = feed(&mut m, EXECUTE, |_| Some(Answer::Approve { always: false }));
        let ev = events(&out);
        assert_eq!(
            requests(&ev),
            vec![(RequestKind::Approval, "run_terminal_command".into(), "echo two > two.txt".into(), false)],
            "Grok offers no session-only option for commands"
        );
        assert_eq!(sent(&ev)[0]["sent"]["outcome"]["optionId"], "allow-once");
        assert_eq!(tool(&ev, "run_terminal_command").1, ToolStatus::Done);
        let mut m = mapper(Some(PermissionMode::Plan));
        assert_eq!(picked(&feed(&mut m, EXECUTE, |_| panic!("nothing asks"))), vec!["reject-once"]);
        let mut m = mapper(Some(PermissionMode::Bypass));
        assert_eq!(picked(&feed(&mut m, EXECUTE, |_| panic!("nothing asks"))), vec!["allow-once"]);
    }

    #[test]
    fn pdts_own_tools_run_without_asking_and_grok_reads_on_its_own() {
        let mut m = mapper(Some(PermissionMode::Ask));
        let out = feed(&mut m, MCP, |_| panic!("nothing asks"));
        assert_eq!(picked(&out), vec!["allow-once"]);
        let ev = events(&out);
        assert_eq!(tool(&ev, "pdt__get_ticket").1, ToolStatus::Done);
        let (title, status, output, _) = tool(&ev, "read_file");
        assert_eq!((title, status), ("Read `hello.txt`", ToolStatus::Done));
        assert!(output.unwrap().contains("hi"));
        assert_eq!(tool(&ev, "run_terminal_command").0, "touch made.txt");
        assert_eq!(ended(&ev), Some(TurnStatus::Completed));
    }

    #[test]
    fn a_refused_command_is_declined_and_grok_ends_the_turn() {
        let mut m = mapper(Some(PermissionMode::Ask));
        let out = feed(&mut m, REJECT, |_| Some(Answer::Deny { message: Some("use two.txt".into()) }));
        let ev = events(&out);
        let sent = sent(&ev);
        assert_eq!(sent[0]["sent"]["outcome"]["optionId"], "reject-once");
        assert_eq!(sent[0]["followUp"], "use two.txt", "the note goes as the next prompt");
        assert_eq!(tool(&ev, "run_terminal_command").1, ToolStatus::Declined);
        assert_eq!(ended(&ev), Some(TurnStatus::Interrupted), "Grok stops after a refusal (`cancelled`)");
    }

    #[test]
    fn a_cancelled_turn_ends_interrupted_with_its_command_failed() {
        let mut m = mapper(Some(PermissionMode::Ask));
        let out = feed(&mut m, CANCEL, |_| panic!("nothing asks"));
        let ev = events(&out);
        let (title, status, ..) = tool(&ev, "run_terminal_command");
        assert_eq!((title, status), ("sleep 30", ToolStatus::Failed));
        assert_eq!(ended(&ev), Some(TurnStatus::Interrupted));
    }

    #[test]
    fn a_loaded_sessions_replay_maps_to_nothing() {
        let mut m = mapper(Some(PermissionMode::Ask));
        m.begin_load(None);
        let mut out = Vec::new();
        for line in LOAD.lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).unwrap();
            if v["method"] == "session/update" {
                out.extend(m.on_update(&v["params"]["update"]));
            }
        }
        out.extend(m.end_load());
        assert!(events(&out).is_empty(), "PDT has logged the history already");
        assert_eq!(m.turn(), None);
    }

    #[test]
    fn the_model_and_its_choices_come_from_the_session_answer() {
        // Trimmed from grok's `session/new` answer: it sends no `config_option_update`.
        let answer = json!({"sessionId": "s", "configOptions": [
            {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "grok-4.7",
             "options": [{"value": "grok-4.7", "name": "Grok 4.7"}]},
            {"id": "reasoning_effort", "name": "Reasoning Effort", "category": "thought_level", "type": "select",
             "currentValue": "high", "options": [{"value": "high", "name": "High"}]}]});
        let out = mapper(None).on_session(&answer);
        assert_eq!(out[0], AcpOut::Settings { mode: None, model: Some("grok-4.7".into()) });
        let AcpOut::Models(models) = &out[1] else { panic!("{out:?}") };
        assert_eq!((models.len(), models[0].display_name.as_str()), (1, "Grok 4.7"));
        assert!(mapper(None).on_session(&json!({"sessionId": "s"})).is_empty());
    }

    #[test]
    fn tool_names_come_from_groks_meta_and_mcp_calls_by_the_mcp_tool() {
        let g = GrokExtensions;
        let write = json!({"_meta": {"x.ai/tool": {"name": "write", "kind": "write"}}});
        assert_eq!(g.tool_name(&write).as_deref(), Some("write"));
        let mcp =
            json!({"rawInput": {"tool_name": "pdt__submit_outcome"}, "_meta": {"x.ai/tool": {"name": "use_tool"}}});
        assert_eq!(g.tool_name(&mcp).as_deref(), Some("pdt__submit_outcome"));
        assert_eq!(g.tool_name(&json!({"kind": "edit"})), None);
        assert!(g.rejected(&json!({"status": "failed", "_meta": {"toolNotRun": "rejected"}})));
        assert!(!g.rejected(&json!({"status": "failed"})));
    }
}
