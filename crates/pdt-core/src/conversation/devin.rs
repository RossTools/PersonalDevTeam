//! Devin's `devin acp`: the Agent Client Protocol mapping (`super::acp`)
//! with what Devin adds to it, its `cognition.ai/*` extra fields, its tool and option names, and its modes.

use super::PermissionMode;
pub use super::acp::{AcpAnswer as DevinAnswer, AcpOut as DevinOut, prompt_blocks};
use super::acp::{AcpExtensions, AcpMapper};
use serde_json::Value;
#[cfg(test)]
use {
    super::{
        AgentEvent, Answer, DeltaField, MessagePhase, MessageSource, RequestKind, RequestState, ToolStatus, TurnPhase,
        TurnStatus,
    },
    serde_json::json,
    std::collections::BTreeMap,
};

/// The mapper for one `devin acp` session.
pub type DevinMapper = AcpMapper<DevinExtensions>;

impl DevinMapper {
    /// A mapper for a session working in `cwd` (plus its canonical path, when that differs). `prefix` keeps ids from
    /// this process apart from an earlier one's.
    pub fn new(cwd: impl Into<String>, canonical: Option<String>, prefix: impl Into<String>) -> Self {
        AcpMapper::with_extensions(DevinExtensions, cwd, canonical, prefix)
    }
}

/// The PDT permission modes Devin has. Devin has no mode that asks before edits: its "Code" mode (`accept-edits`)
/// makes edits in the folder without asking and asks before commands it does not judge safe.
pub const DEVIN_MODES: [PermissionMode; 3] =
    [PermissionMode::AcceptEdits, PermissionMode::Plan, PermissionMode::Bypass];

/// Devin's session mode for a PDT permission mode (Ask has no equivalent; Code comes closest).
pub fn devin_mode(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask | PermissionMode::AcceptEdits => "accept-edits",
        PermissionMode::Plan => "plan",
        PermissionMode::Bypass => "bypass",
    }
}

/// The PDT mode for Devin's; None for modes PDT does not offer (`smart`, and `ask`, which only answers questions).
pub fn from_devin_mode(mode: &str) -> Option<PermissionMode> {
    match mode {
        "accept-edits" => Some(PermissionMode::AcceptEdits),
        "plan" => Some(PermissionMode::Plan),
        "bypass" => Some(PermissionMode::Bypass),
        _ => None,
    }
}

/// `session/load` and `session/prompt` fail with this when Devin has no saved session by that id (it saves one only
/// at its first prompt).
pub const SESSION_NOT_FOUND: i64 = -32016;

/// The name prefix Devin gives pdt-mcp's tools (`mcp__pdt__get_ticket`).
const PDT_TOOL_PREFIX: &str = "mcp__pdt__";

fn meta<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.get("_meta").and_then(|m| m.get(k))
}

fn meta_flag(v: &Value, k: &str) -> bool {
    meta(v, k).and_then(Value::as_bool) == Some(true)
}

/// What Devin adds to ACP.
#[derive(Debug, Clone, Copy, Default)]
pub struct DevinExtensions;

impl AcpExtensions for DevinExtensions {
    fn name(&self) -> &str {
        "Devin"
    }

    /// Devin reports `/private/tmp/…` for `/tmp/…`.
    fn resolves_symlinks(&self) -> bool {
        true
    }

    fn tool_name(&self, update: &Value) -> Option<String> {
        meta(update, "cognition.ai/inferenceToolName").and_then(Value::as_str).map(String::from)
    }

    fn is_exit_plan(&self, update: &Value, tool: &str) -> bool {
        meta_flag(update, "cognition.ai/isExitPlan") || tool == "exit_plan_mode"
    }

    fn written_plan(&self, tool: &str, input: &Value) -> Option<String> {
        (tool == "write_plan").then(|| input.get("content").and_then(Value::as_str).map(String::from)).flatten()
    }

    fn rejected(&self, update: &Value) -> bool {
        meta_flag(update, "cognition.ai/rejected")
    }

    /// Devin sends a command's whole output so far each time, not the new part.
    fn cumulative_output(&self) -> bool {
        true
    }

    /// Sub-agents report their own context too; the conversation's is the root agent's.
    fn ignore_usage(&self, update: &Value) -> bool {
        meta(update, "cognition.ai/subagent_context").is_some()
    }

    fn replayed_prompt(&self, update: &Value) -> (Option<String>, Option<i64>) {
        let client = meta(update, "cognition.ai/clientMessageId").and_then(Value::as_str).map(String::from);
        let at = meta(update, "cognition.ai/timestamp").and_then(Value::as_str).and_then(iso_ms);
        (client, at)
    }

    /// Only Devin's "this session" option: the others save a rule to Devin's settings (never from PDT).
    fn session_option(&self) -> Option<&str> {
        Some("allow_session")
    }

    fn is_plan_request(&self, option_ids: &[&str]) -> bool {
        option_ids.iter().any(|id| id.starts_with("plan_"))
    }

    fn plan_option(&self, mode: Option<PermissionMode>) -> Option<&str> {
        Some(match mode {
            Some(PermissionMode::Bypass) => "plan_bypass",
            _ => "plan_accept_edits",
        })
    }

    fn pdt_tool_prefix(&self) -> Option<&str> {
        Some(PDT_TOOL_PREFIX)
    }

    /// Devin names fields `q0`, `q1`, … in order: q2 before q10.
    fn order_fields(&self, keys: &mut [String]) {
        keys.sort_by_key(|k| {
            (k.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u64>().unwrap_or(u64::MAX), k.clone())
        });
    }

    fn retryable(&self, data: &Value) -> bool {
        data.get("cognition.ai/retryable").and_then(Value::as_bool).unwrap_or(false)
    }
}

/// An RFC 3339 time (`2026-10-19T23:53:11.255937+00:00`, as Devin stamps messages) in ms since the epoch.
pub fn iso_ms(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> { t.get(r)?.parse().ok() };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' {
        return None;
    }
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut i = 19;
    let mut ms = 0;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        i = start;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        let frac = t.get(start..i)?;
        ms = format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()?;
    }
    let offset = match b.get(i) {
        Some(b'Z' | b'z') | None => 0,
        Some(&sign @ (b'+' | b'-')) => {
            let o = num(i + 1..i + 3)? * 60 + num(i + 4..i + 6)?;
            if sign == b'+' { o } else { -o }
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi - offset) * 60_000 + sec * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDIT: &str = include_str!("../../tests/fixtures/devin/edit.jsonl");
    const APPROVAL: &str = include_str!("../../tests/fixtures/devin/approval.jsonl");
    const QUESTION_PLAN: &str = include_str!("../../tests/fixtures/devin/question_plan.jsonl");
    const CANCEL: &str = include_str!("../../tests/fixtures/devin/cancel.jsonl");

    const CWD: &str = "/tmp/acp-probe/repo";

    fn mapper() -> DevinMapper {
        DevinMapper::new(CWD, Some(format!("/private{CWD}")), "p")
    }

    /// Feeds recorded frames as the adapter would: updates, agent requests (answered with `answer` when given) and
    /// prompt results. `prompt` sends each prompt's text before the first frame of its turn.
    fn feed(m: &mut DevinMapper, frames: &str, mut answer: impl FnMut(&AgentEvent) -> Option<Answer>) -> Vec<DevinOut> {
        let mut out = Vec::new();
        for line in frames.lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).unwrap();
            if m.turn().is_none() && (v.get("method").is_some_and(|x| x == "session/update")) {
                let k = v["params"]["update"]["sessionUpdate"].as_str().unwrap_or("");
                if !matches!(
                    k,
                    "config_option_update"
                        | "current_mode_update"
                        | "available_commands_update"
                        | "usage_update"
                        | "session_info_update"
                ) {
                    out.extend(m.begin_prompt("go"));
                }
            }
            match (v.get("method").and_then(Value::as_str), v.get("id")) {
                (Some("session/update"), None) => out.extend(m.on_update(&v["params"]["update"])),
                (Some(_), None) => {}
                (Some(method), Some(id)) => {
                    let evs = m.on_request(id.clone(), method, &v["params"]);
                    let ask = evs.iter().find_map(|e| match e {
                        DevinOut::Event { event: ev @ AgentEvent::Request { .. }, .. } => Some(ev.clone()),
                        _ => None,
                    });
                    out.extend(evs);
                    if let Some(ev) = ask
                        && let AgentEvent::Request { id, .. } = &ev
                        && let Some(a) = answer(&ev)
                    {
                        let a = m.answer(id, &a).unwrap();
                        out.push(DevinOut::Event {
                            item_id: None,
                            turn_id: None,
                            event: AgentEvent::Unknown { raw: json!({"sent": a.result, "followUp": a.follow_up}) },
                        });
                        out.extend(a.events);
                    }
                }
                (None, Some(_)) => {
                    if let Some(r) = v.get("result") {
                        out.extend(m.end_prompt(Ok(r)));
                    } else {
                        let msg = v["error"]["message"].as_str().unwrap_or("").to_string();
                        out.extend(m.end_prompt(Err((msg, false))));
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn events(out: &[DevinOut]) -> Vec<(Option<String>, AgentEvent)> {
        out.iter()
            .filter_map(|o| match o {
                DevinOut::Event { item_id, event, .. } => Some((item_id.clone(), event.clone())),
                _ => None,
            })
            .collect()
    }

    fn last_tool<'a>(ev: &'a [(Option<String>, AgentEvent)], tool: &str) -> &'a AgentEvent {
        ev.iter()
            .rev()
            .map(|(_, e)| e)
            .find(|e| matches!(e, AgentEvent::ToolCall { tool: t, .. } if t == tool))
            .unwrap()
    }

    #[test]
    fn a_recorded_turn_maps_to_messages_tools_diffs_and_its_end() {
        let mut m = mapper();
        let out = feed(&mut m, EDIT, |_| None);
        let ev = events(&out);
        assert!(matches!(&ev[0].1, AgentEvent::Turn { phase: TurnPhase::Started, .. }));
        assert!(matches!(&ev[1].1, AgentEvent::UserMessage { text, source: MessageSource::Pdt, .. } if text == "go"));
        // Thoughts become reasoning; the closing message is the turn's answer, streamed first.
        assert!(
            ev.iter()
                .any(|(_, e)| matches!(e, AgentEvent::Reasoning { summary } if summary.contains("user wants me to")))
        );
        let deltas: String = out
            .iter()
            .filter_map(|o| match o {
                DevinOut::Delta { field: DeltaField::Markdown, text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, "Done. 1 file (a.txt).");
        assert!(ev.iter().any(|(_, e)| matches!(e,
            AgentEvent::AgentMessage { markdown, phase: MessagePhase::Final, .. } if markdown == "Done. 1 file (a.txt).")));
        // The read names the file inside the folder (Devin reports the /private path).
        let AgentEvent::ToolCall { title, status, .. } = last_tool(&ev, "read") else { panic!() };
        assert_eq!((title.as_str(), *status), ("a.txt", ToolStatus::Done));
        let AgentEvent::ToolCall { title, status, diff, .. } = last_tool(&ev, "edit") else { panic!() };
        assert_eq!((title.as_str(), *status), ("a.txt", ToolStatus::Done));
        assert_eq!(diff.as_deref(), Some("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-hello\n+goodbye\n"));
        let AgentEvent::ToolCall { title, output, exit_code, .. } = last_tool(&ev, "exec") else { panic!() };
        assert_eq!(title, "ls -la");
        assert_eq!(*exit_code, Some(0));
        assert!(output.as_deref().unwrap().ends_with("a.txt"), "{output:?}");
        let n = ev.len();
        assert!(matches!(&ev[n - 2].1, AgentEvent::TurnDiff { diff } if diff.contains("+goodbye")));
        assert!(matches!(
            &ev[n - 1].1,
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
        assert!(ev.iter().any(|(_, e)| matches!(
            e,
            AgentEvent::Usage { context_used: Some(12029), context_window: Some(200000), .. }
        )));
        assert_eq!(m.turn(), None);
        // Tool calls update one row each.
        let edits = ev.iter().filter(|(i, _)| i.as_deref() == Some("t:call_136003b5d4fb4aa7a1412539")).count();
        assert!(edits >= 2);
    }

    #[test]
    fn approvals_offer_the_session_option_and_rejects_are_declined_with_a_note() {
        let mut m = mapper();
        let mut n = 0;
        let out = feed(&mut m, APPROVAL, |e| {
            n += 1;
            let AgentEvent::Request { request_kind, can_always, title, tool, .. } = e else { return None };
            assert_eq!(*request_kind, RequestKind::Approval);
            assert!(*can_always);
            assert_eq!((title.as_str(), tool.as_str()), ("touch b.txt", "exec"));
            Some(if n == 1 {
                Answer::Deny { message: Some(" not now ".into()) }
            } else {
                Answer::Approve { always: true }
            })
        });
        let ev = events(&out);
        let sent: Vec<&Value> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::Unknown { raw } => Some(raw),
                _ => None,
            })
            .collect();
        assert_eq!(sent[0]["sent"]["outcome"]["optionId"], "reject_once");
        assert_eq!(sent[0]["followUp"], "not now");
        assert_eq!(sent[1]["sent"]["outcome"]["optionId"], "allow_session", "never a persistent rule");
        let declined = ev.iter().any(|(_, e)| matches!(e, AgentEvent::ToolCall { status: ToolStatus::Declined, .. }));
        assert!(declined);
        let ends = ev
            .iter()
            .filter(|(_, e)| {
                matches!(e, AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. })
            })
            .count();
        assert_eq!(ends, 2);
        assert!(m.answer("nope", &Answer::Approve { always: false }).is_err());
    }

    #[test]
    fn questions_and_plans_become_requests_answered_in_devins_terms() {
        let mut m = mapper();
        let out = feed(&mut m, QUESTION_PLAN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Question, input, title, .. } => {
                assert_eq!(title, "Which color do you prefer?");
                assert_eq!(input["questions"][0]["options"][1]["label"], "Blue");
                assert_eq!(input["questions"][0]["header"], "Color preference");
                let answers = BTreeMap::from([("Which color do you prefer?".to_string(), vec!["Blue".to_string()])]);
                Some(Answer::Question { answers })
            }
            AgentEvent::Request { request_kind: RequestKind::Plan, input, .. } => {
                assert!(input["plan"].as_str().unwrap().starts_with("# Add c.txt"), "{input}");
                Some(Answer::Plan { approve: true, feedback: None, mode: Some(PermissionMode::Bypass) })
            }
            _ => None,
        });
        let ev = events(&out);
        let sent: Vec<&Value> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::Unknown { raw } => Some(&raw["sent"]),
                _ => None,
            })
            .collect();
        assert_eq!(sent[0], &json!({"action": "accept", "content": {"q0": "Blue"}}));
        assert_eq!(sent[1], &json!({"outcome": {"outcome": "selected", "optionId": "plan_bypass"}}));
        // Mode changes are reported for the runtime; the plan file outside the folder stays out of the turn's diff.
        assert!(out.iter().any(|o| matches!(o, DevinOut::Settings { mode: Some(m), .. } if m == "accept-edits")));
        let diffs: Vec<&String> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::TurnDiff { diff } => Some(diff),
                _ => None,
            })
            .collect();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0], "--- /dev/null\n+++ b/c.txt\n@@ -0,0 +1 @@\n+Blue\n");
    }

    #[test]
    fn a_cancelled_turn_ends_interrupted_and_its_tool_stays_failed() {
        let mut m = mapper();
        let out = feed(&mut m, CANCEL, |_| Some(Answer::Approve { always: false }));
        let ev = events(&out);
        let AgentEvent::ToolCall { status, .. } = last_tool(&ev, "exec") else { panic!() };
        assert_eq!(*status, ToolStatus::Failed, "the late `completed` does not undo the cancel");
        assert!(matches!(&ev.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Interrupted), .. }));
    }

    #[test]
    fn prompts_sent_mid_turn_join_it_and_the_turn_ends_with_the_last_answer() {
        let mut m = mapper();
        m.begin_prompt("first");
        let steer = events(&m.begin_prompt("second"));
        assert!(matches!(&steer[0].1, AgentEvent::UserMessage { source: MessageSource::Steer, .. }));
        let done = json!({"stopReason": "end_turn"});
        assert!(events(&m.end_prompt(Ok(&done))).is_empty());
        let end = events(&m.end_prompt(Ok(&done)));
        assert!(matches!(&end.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Completed), .. }));
        // A late answer for a turn already ended maps to nothing.
        assert!(m.end_prompt(Ok(&done)).is_empty());
        // An error ends the turn as failed, with the error shown.
        m.begin_prompt("x");
        let end = events(&m.end_prompt(Err(("Quota exhausted".into(), true))));
        assert!(matches!(&end[0].1, AgentEvent::Error { retryable: true, .. }));
        assert!(
            matches!(&end.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Failed), reason: Some(r), .. } if r == "Quota exhausted")
        );
        // Interrupting: requests are answered as cancelled, and the turn ends interrupted even on end_turn.
        m.begin_prompt("y");
        m.on_request(
            json!("r1"),
            "session/request_permission",
            &json!({"toolCall": {"toolCallId": "c"}, "options": [{"optionId": "allow_once", "kind": "allow_once"}]}),
        );
        m.note_interrupt();
        let (replies, expired) = m.cancel_requests();
        assert_eq!(replies, vec![(json!("r1"), json!({"outcome": {"outcome": "cancelled"}}))]);
        assert!(matches!(&events(&expired)[0].1, AgentEvent::Request { state: RequestState::Expired, .. }));
        let end = events(&m.end_prompt(Ok(&done)));
        assert!(matches!(&end.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Interrupted), .. }));
    }

    #[test]
    fn replayed_history_is_dropped_but_settings_and_commands_are_kept() {
        let mut m = mapper();
        m.begin_load(None);
        assert!(
            m.on_update(&json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "old"}}))
                .is_empty()
        );
        assert!(
            m.on_update(&json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "old"}}))
                .is_empty()
        );
        let out = m.on_update(&json!({"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "compact", "description": "Force conversation compaction"},
            {"name": "plan", "description": "Plan", "input": {"hint": "[prompt]"}}]}));
        assert!(
            matches!(&out[0], DevinOut::Commands(c) if c.len() == 2 && c[1].argument_hint.as_deref() == Some("[prompt]"))
        );
        let out = m.on_update(&json!({"sessionUpdate": "config_option_update", "configOptions": [
            {"id": "mode", "currentValue": "plan"},
            {"id": "model", "currentValue": "swe-1-6-slow", "options": [{"value": "swe-1-6-slow", "name": "SWE-1.6 Slow"}]}]}));
        assert_eq!(out[0], DevinOut::Settings { mode: Some("plan".into()), model: Some("swe-1-6-slow".into()) });
        assert!(matches!(&out[1], DevinOut::Models(v) if v[0].display_name == "SWE-1.6 Slow"));
        assert!(m.end_load().is_empty());
        // Unknown agent requests are refused.
        let out = m.on_request(json!(7), "fs/read_text_file", &json!({}));
        assert!(matches!(&out[0], DevinOut::RespondError { id, .. } if *id == json!(7)));
        // The process stopping ends a running turn as interrupted.
        m.begin_prompt("z");
        let end = events(&m.on_exit("Devin stopped"));
        assert!(
            matches!(&end.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Interrupted), reason: Some(r), .. } if r == "Devin stopped")
        );
    }

    #[test]
    fn turns_taken_in_the_terminal_are_imported_from_the_replay() {
        let chunk = |kind: &str, text: &str, id: &str, at: &str| {
            json!({"sessionUpdate": kind, "content": {"type": "text", "text": text},
                   "_meta": {"cognition.ai/clientMessageId": id, "cognition.ai/timestamp": at}})
        };
        let switched = iso_ms("2026-10-19T23:00:00+00:00").unwrap();
        let mut m = mapper();
        m.begin_load(Some(switched));
        let mut out = Vec::new();
        out.extend(m.on_update(&chunk("user_message_chunk", "old", "a", "2026-10-19T22:10:22.692914+00:00")));
        out.extend(m.on_update(&chunk("agent_message_chunk", "old answer", "", "2026-10-19T22:10:26+00:00")));
        out.extend(m.on_update(&chunk("user_message_chunk", "Reply PINEAPPLE", "b", "2026-10-19T23:53:11.25+00:00")));
        out.extend(m.on_update(
            &json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Ran ls", "kind": "execute",
            "rawInput": {"command": "ls"}, "_meta": {"cognition.ai/inferenceToolName": "exec"}}),
        ));
        out.extend(
            m.on_update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed"})),
        );
        out.extend(m.on_update(&chunk("agent_message_chunk", "PINEAPPLE", "", "2026-10-19T23:53:12+00:00")));
        out.extend(m.on_update(&chunk("user_message_chunk", "again", "c", "2026-10-19T23:54:00Z")));
        out.extend(m.on_update(&chunk("agent_message_chunk", "ok", "", "2026-10-19T23:54:01Z")));
        out.extend(m.end_load());
        let ev = events(&out);
        let said: Vec<String> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::UserMessage { text, source: MessageSource::Terminal, .. } => Some(format!("> {text}")),
                AgentEvent::AgentMessage { markdown, phase: MessagePhase::Final, .. } => Some(markdown.clone()),
                AgentEvent::ToolCall { title, status: ToolStatus::Done, .. } => Some(format!("$ {title}")),
                AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. } => {
                    Some("end".into())
                }
                _ => None,
            })
            .collect();
        assert_eq!(said, ["> Reply PINEAPPLE", "$ ls", "PINEAPPLE", "end", "> again", "ok", "end"]);
        assert_eq!(m.turn(), None);
        // Without an anchor nothing is imported.
        let mut m = mapper();
        m.begin_load(None);
        assert!(m.on_update(&chunk("user_message_chunk", "x", "a", "2030-01-01T00:00:00Z")).is_empty());
        assert!(m.on_update(&chunk("agent_message_chunk", "y", "", "2030-01-01T00:00:00Z")).is_empty());
        assert!(m.end_load().is_empty());
    }

    #[test]
    fn rfc3339_times_become_ms() {
        assert_eq!(iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_ms("2026-10-19T23:53:11.255937+00:00"), Some(1_792_453_991_255));
        assert_eq!(iso_ms("2000-02-29T12:00:00-05:00"), Some(951_843_600_000));
        assert_eq!(iso_ms("2026-10-19T23:53:11.2Z"), Some(1_792_453_991_200));
        assert_eq!(iso_ms("yesterday"), None);
    }

    #[test]
    fn modes_map_both_ways_and_prompts_put_images_first() {
        assert_eq!(devin_mode(PermissionMode::Ask), "accept-edits");
        for m in DEVIN_MODES {
            assert_eq!(from_devin_mode(devin_mode(m)), Some(m));
        }
        assert_eq!(from_devin_mode("smart"), None);
        let b = prompt_blocks("look", &[("image/png".into(), "AAAA".into())]);
        assert_eq!(
            b,
            json!([{"type": "image", "mimeType": "image/png", "data": "AAAA"}, {"type": "text", "text": "look"}])
        );
        assert_eq!(prompt_blocks("", &[("image/png".into(), "A".into())]).as_array().unwrap().len(), 1);
    }

    #[test]
    fn multi_select_questions_answer_with_lists() {
        let mut m = mapper();
        m.begin_prompt("q");
        let p = json!({"mode": "form", "message": "Pick", "requestedSchema": {"type": "object", "properties": {
            "q0": {"type": "array", "title": "Fruit", "description": "Which fruit?", "items": {"anyOf": [{"const": "Apple"}, {"const": "Pear"}]}},
            "q1": {"type": "string", "title": "Why", "description": "Why?"}}, "required": ["q0"]}});
        let out = m.on_request(json!("e1"), "elicitation/create", &p);
        let AgentEvent::Request { input, .. } = &events(&out)[0].1 else { panic!() };
        assert_eq!(input["questions"][0]["multiSelect"], true);
        assert_eq!(input["questions"][1]["question"], "Why?");
        let answers = BTreeMap::from([
            ("Which fruit?".to_string(), vec!["Apple".to_string(), "Pear".to_string()]),
            ("Why?".to_string(), vec!["tasty".to_string()]),
        ]);
        let a = m.answer("e1", &Answer::Question { answers }).unwrap();
        assert_eq!(a.result, json!({"action": "accept", "content": {"q0": ["Apple", "Pear"], "q1": "tasty"}}));
        let out = m.on_request(json!("e2"), "elicitation/create", &p);
        assert!(!out.is_empty());
        assert_eq!(m.answer("e2", &Answer::Deny { message: None }).unwrap().result, json!({"action": "decline"}));
    }

    #[test]
    fn pdts_own_tools_are_allowed_without_asking() {
        let mut m = mapper();
        m.begin_prompt("go");
        m.on_update(&json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Calling get_ticket from pdt",
            "kind": "other", "status": "pending", "rawInput": {},
            "_meta": {"cognition.ai/inferenceToolName": "mcp__pdt__get_ticket"}}));
        let options = json!([{"optionId": "o1", "name": "Allow", "kind": "allow_once"},
                             {"optionId": "o2", "name": "Reject", "kind": "reject_once"}]);
        let out = m.on_request(
            json!(9),
            "session/request_permission",
            &json!({"sessionId": "s", "toolCall": {"toolCallId": "c1"}, "options": options}),
        );
        match &out[..] {
            [DevinOut::Respond { id, result }] => {
                assert_eq!(*id, json!(9));
                assert_eq!(result["outcome"]["optionId"], "o1");
            }
            other => panic!("{other:?}"),
        }
    }
}
