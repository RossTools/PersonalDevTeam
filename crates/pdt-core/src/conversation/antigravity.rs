//! Google's Antigravity ACP server (`agy_acp_server`, PDT-239; notes: docs/superpowers/notes/antigravity-acp.md):
//! plain ACP (`super::acp`) plus its MCP calls' server and tool in `_meta.mcp`, a command's output in `rawOutput`
//! (`{exitCode, combinedOutput}`), and an "Allow Always" that lasts only as long as its session (it keeps it in
//! memory). What a built-in profile's flavor adds ([`AcpFlavor`](crate::acp_agent::AcpFlavor)) is mapped here.

use super::acp::AcpExtensions;
use serde_json::Value;

/// What Antigravity adds to ACP.
#[derive(Debug, Clone, Copy, Default)]
pub struct AntigravityExtensions;

/// The prefix its MCP calls to pdt-mcp get from [`AntigravityExtensions::tool_name`] (the server is the `name` PDT
/// gives pdt-mcp in `mcpServers`: `crate::mcp::SERVER_NAME`).
const PDT_TOOL_PREFIX: &str = "mcp__pdt__";

/// The `rawOutput` of an approved call it ran as another one (`AntigravityExtensions::ran_elsewhere`).
const NEVER_EXECUTED: &str = "Tool call was approved but never executed.";

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty())
}

impl AcpExtensions for AntigravityExtensions {
    fn name(&self) -> &str {
        "Antigravity"
    }

    /// Its paths come back through symlinks resolved (`/private/tmp/…` on macOS).
    fn resolves_symlinks(&self) -> bool {
        true
    }

    /// An MCP call carries `_meta.mcp` `{server, tool}`: named `mcp__<server>__<tool>`, as Claude names them.
    fn tool_name(&self, update: &Value) -> Option<String> {
        let mcp = update.pointer("/_meta/mcp")?;
        Some(format!("mcp__{}__{}", s(mcp, "server")?, s(mcp, "tool")?))
    }

    fn pdt_tool_prefix(&self) -> Option<&str> {
        Some(PDT_TOOL_PREFIX)
    }

    /// An edit it asked to approve and then ran as a call of its own (`<session>:<n>`, "Running edit_file", which has
    /// no diff) ends the turn `failed` with `rawOutput` "Tool call was approved but never executed." (a new file in
    /// Ask): shown done, with the diff it asked about.
    fn ran_elsewhere(&self, update: &Value) -> bool {
        update.get("rawOutput").and_then(Value::as_str).map(str::trim) == Some(NEVER_EXECUTED)
    }

    /// A command's result is `rawOutput` `{commandLine, workingDir, exitCode, combinedOutput}`.
    fn raw_output(&self, update: &Value) -> Option<(String, Option<i32>)> {
        let out = update.get("rawOutput").filter(|o| o.is_object())?;
        let code = out.get("exitCode").and_then(Value::as_i64).and_then(|c| i32::try_from(c).ok());
        let text = out.get("combinedOutput").and_then(Value::as_str).unwrap_or_default();
        (code.is_some() || !text.trim().is_empty()).then(|| (text.trim_end().to_string(), code))
    }

    /// Its "Allow Always" (`allow_always`) is kept in the server's memory for the session only (scoped to the exact
    /// command for a shell command), and never written to its settings.
    fn session_option(&self) -> Option<&str> {
        Some("allow_always")
    }
}

/// The ACP sign-in method PDT asks the server for (Settings → Agents → Sign in): "Log in with Google". The server
/// runs the browser sign-in itself and keeps the token in its own keychain entry, apart from the `agy` CLI's.
pub const SIGN_IN_METHOD: &str = "oauth-personal";

/// The error a session gets when the server has no sign-in: `-32000` "Authentication required".
pub fn needs_sign_in(code: i64, message: &str) -> bool {
    code == -32000 && message.to_ascii_lowercase().contains("authentication required")
}

/// The server's settings file, under `GEMINI_HOME` (else `~/.gemini`): `antigravity-acp/settings.json`. PDT only reads
/// it.
pub fn settings_path(gemini_home: Option<&str>, home: &std::path::Path) -> std::path::PathBuf {
    let base = gemini_home.filter(|h| !h.trim().is_empty()).map(std::path::PathBuf::from);
    base.unwrap_or_else(|| home.join(".gemini")).join("antigravity-acp").join("settings.json")
}

/// The sign-in the server's settings say it uses (`auth.type`, which it records when it signs in and clears when it
/// signs out), or None.
pub fn signed_in(settings: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(settings).ok()?;
    let t = v.pointer("/auth/type").or_else(|| v.get("auth.type"))?.as_str()?.trim();
    (!t.is_empty()).then(|| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::acp::{AcpMapper, AcpOut};
    use super::super::{AgentEvent, Answer, PermissionMode, RequestKind, ToolStatus, TurnPhase, TurnStatus};
    use super::*;
    use crate::acp_agent::AcpFlavor;
    use serde_json::json;

    /// Google's Antigravity ACP server 1.4.5 (signed in on the free tier), recorded: `session/new`, `session/set_mode`
    /// (`auto_edit`, then `plan`, which it has not), `session/set_config_option` (a model it has, one it hasn't) and
    /// `session/load` of an id it never saved.
    const START: &str = include_str!("../../tests/fixtures/antigravity/start.jsonl");
    /// A turn in `default` mode, recorded with every request allowed once: a pdt-mcp call (`get_ticket`, to a stand-in
    /// pdt-mcp), `ls`, and a new file, which the server reports twice (an approved `create_file` frame it later fails
    /// as "never executed", and the `edit_file` that ran).
    const TURN: &str = include_str!("../../tests/fixtures/antigravity/turn.jsonl");

    fn mapper(mode: PermissionMode) -> AcpMapper<AcpFlavor> {
        let mut m = AcpMapper::with_extensions(AcpFlavor::Antigravity, "/work/repo", None, "p");
        // Ask, Accept edits and Bypass are its own modes (`acp_agent::antigravity`); PDT keeps Plan.
        m.enforce_mode((mode == PermissionMode::Plan).then_some(mode));
        m
    }

    /// Feeds the turn as the adapter would, answering requests with `answer`; returns the events and what PDT sent.
    fn feed(
        m: &mut AcpMapper<AcpFlavor>,
        mut answer: impl FnMut(&AgentEvent) -> Option<Answer>,
    ) -> (Vec<AgentEvent>, Vec<Value>) {
        let mut out = m.begin_prompt("go");
        let mut sent = Vec::new();
        for line in TURN.lines().filter(|l| !l.trim().is_empty()) {
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

    fn selected(option: &str) -> Value {
        json!({"outcome": {"outcome": "selected", "optionId": option}})
    }

    #[test]
    fn antigravity_starts_with_its_modes_models_and_saved_sessions() {
        let frames: Vec<Value> = START.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let by_id = |id: i64| frames.iter().find(|f| f["id"] == id).unwrap();
        let caps = &by_id(1)["result"]["agentCapabilities"];
        assert_eq!(
            (caps["loadSession"].as_bool(), caps["mcpCapabilities"]["http"].as_bool()),
            (Some(true), Some(true))
        );
        let methods: Vec<&str> =
            by_id(1)["result"]["authMethods"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
        assert!(methods.contains(&SIGN_IN_METHOD), "{methods:?}");
        // Its modes are the profile's map, and it starts in `default` (PDT's Ask).
        let new = &by_id(2)["result"];
        let ids: Vec<&str> =
            new["modes"]["availableModes"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
        assert_eq!(ids, ["default", "auto_edit", "yolo"]);
        assert_eq!(new["modes"]["currentModeId"], "default");
        let profile = crate::acp_agent::find(&[], "antigravity").unwrap();
        let mapped: Vec<&str> = profile.modes.iter().map(|(_, id)| id.as_str()).collect();
        assert_eq!(mapped, ids);
        assert_eq!(crate::acp_agent::advertised_modes(&new["modes"]["availableModes"]), profile.modes);
        // The model picker is a `model` config option; set_config_option takes one and refuses another.
        let model = new["configOptions"].as_array().unwrap().iter().find(|o| o["id"] == "model").unwrap();
        assert_eq!(
            (model["category"].as_str(), model["currentValue"].as_str()),
            (Some("model"), Some("gemini-3.8-flash"))
        );
        let outs = mapper(PermissionMode::Ask).on_session(new);
        let models: Vec<&AcpOut> = outs.iter().filter(|o| matches!(o, AcpOut::Models(_))).collect();
        assert!(
            matches!(&models[..], [AcpOut::Models(m)] if m.len() == 1 && m[0].value == "gemini-3.8-flash"),
            "the picker's models come from session/new's answer: {models:?}"
        );
        assert_eq!(by_id(3)["result"], json!({}), "set_mode auto_edit");
        assert_eq!(by_id(4)["error"]["message"], "Unknown mode id: plan", "no plan mode: PDT keeps Plan");
        assert!(by_id(5)["result"]["configOptions"].is_array());
        assert_eq!(by_id(6)["error"]["code"], -32602);
        // An id it never saved: -32002, so PDT starts a new session.
        assert!(
            AcpFlavor::Antigravity.session_missing(by_id(7)["error"]["code"].as_i64().unwrap(), "Session not found")
        );
    }

    #[test]
    fn an_ask_turn_runs_pdt_tools_and_asks_for_the_command_and_the_edit() {
        let mut m = mapper(PermissionMode::Ask);
        let mut asked = Vec::new();
        let (ev, sent) = feed(&mut m, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Approval, tool, can_always, .. } => {
                asked.push((tool.clone(), *can_always));
                // Its "Allow Always" is for this session only (the command, here): PDT offers it.
                Some(Answer::Approve { always: *can_always })
            }
            _ => None,
        });
        assert_eq!(asked, [("execute".to_string(), true), ("edit".to_string(), false)], "pdt-mcp calls never ask");
        assert_eq!(sent, vec![selected("allow"), selected("allow_always"), selected("allow")]);
        let AgentEvent::ToolCall { tool, status, .. } = last_tool(&ev, "f528d767ddcd414f8a8d7584fb3828b7") else {
            panic!()
        };
        assert_eq!((tool.as_str(), *status), ("mcp__pdt__get_ticket", ToolStatus::Done));
        let AgentEvent::ToolCall { title, output, exit_code, status, .. } =
            last_tool(&ev, "c73cb06b83df473ca6373aaa2730c230")
        else {
            panic!()
        };
        assert_eq!(
            (title.as_str(), output.as_deref(), *exit_code, *status),
            ("ls", Some("README.md\tskills"), Some(0), ToolStatus::Done)
        );
        // The new file's approved `create_file` ends "never executed" because it ran as `edit_file`: done, with its
        // diff, and in the turn's diff (PDT-286).
        let AgentEvent::ToolCall { diff, status, .. } = last_tool(&ev, "80c23d44dec449f3b6344563233dc1ad") else {
            panic!()
        };
        assert!(diff.as_deref().unwrap().contains("+hello world"), "{diff:?}");
        assert_eq!(*status, ToolStatus::Done);
        let AgentEvent::ToolCall { status, .. } = last_tool(&ev, ":4") else { panic!() };
        assert_eq!(*status, ToolStatus::Done);
        let turn_diff = ev.iter().find_map(|e| match e {
            AgentEvent::TurnDiff { diff } => Some(diff.as_str()),
            _ => None,
        });
        assert!(
            turn_diff.is_some_and(|d| d.contains("+++ b/hello.txt") && d.contains("+hello world")),
            "{turn_diff:?}"
        );
        assert!(matches!(
            ev.last().unwrap(),
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
    }

    #[test]
    fn plan_refuses_the_command_and_the_edit_itself() {
        let mut m = mapper(PermissionMode::Plan);
        let (_, sent) = feed(&mut m, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Approval, .. } => {
                panic!("Plan answers for the user: {e:?}")
            }
            _ => None,
        });
        assert_eq!(sent[0], selected("allow"), "pdt-mcp");
        assert!(sent[1..].iter().all(|s| s["outcome"]["optionId"] == "deny"), "{sent:?}");
    }

    #[test]
    fn a_refused_edit_that_never_ran_stays_failed() {
        // Refused for the user (Plan) or by them (Ask): "never executed" is not an edit that ran as another call.
        let mut m = mapper(PermissionMode::Plan);
        let (ev, _) = feed(&mut m, |_| None);
        let AgentEvent::ToolCall { status, .. } = last_tool(&ev, "80c23d44dec449f3b6344563233dc1ad") else { panic!() };
        assert_eq!(*status, ToolStatus::Failed);
        let mut m = mapper(PermissionMode::Ask);
        let (ev, _) = feed(&mut m, |e| match e {
            AgentEvent::Request { tool, .. } if tool == "edit" => Some(Answer::Deny { message: None }),
            AgentEvent::Request { .. } => Some(Answer::Approve { always: false }),
            _ => None,
        });
        let AgentEvent::ToolCall { status, .. } = last_tool(&ev, "80c23d44dec449f3b6344563233dc1ad") else { panic!() };
        assert_eq!(*status, ToolStatus::Failed);
        assert!(!ev.iter().any(|e| matches!(e, AgentEvent::TurnDiff { .. })), "nothing was written");
    }

    #[test]
    fn its_frames_name_mcp_calls_and_read_command_output() {
        let call = json!({"_meta": {"mcp": {"tool": "get_ticket", "server": "pdt"}, "is_mcp_tool_call": true}});
        assert_eq!(AntigravityExtensions.tool_name(&call).as_deref(), Some("mcp__pdt__get_ticket"));
        assert_eq!(AntigravityExtensions.tool_name(&json!({"title": "ls"})), None);
        let out = json!({"rawOutput": {"exitCode": 2, "combinedOutput": "no such file\r\n"}});
        assert_eq!(AntigravityExtensions.raw_output(&out), Some(("no such file".into(), Some(2))));
        assert_eq!(AntigravityExtensions.raw_output(&json!({"rawOutput": "Create hello.txt"})), None);
        assert!(AntigravityExtensions.ran_elsewhere(&json!({"rawOutput": NEVER_EXECUTED})));
        assert!(!AntigravityExtensions.ran_elsewhere(&json!({"rawOutput": "Permission denied"})));
        assert!(!AcpFlavor::Cursor.ran_elsewhere(&json!({"rawOutput": NEVER_EXECUTED})));
        assert!(AcpFlavor::Antigravity.resolves_symlinks() && !AcpFlavor::Cursor.resolves_symlinks());
        assert!(needs_sign_in(-32000, "Authentication required: No authentication method selected."));
        assert!(!needs_sign_in(-32602, "Authentication required"));
        assert!(
            AcpFlavor::Antigravity
                .refusal_hint(-32000, "Authentication required")
                .unwrap()
                .contains("Settings → Agents")
        );
        assert_eq!(AcpFlavor::Cursor.refusal_hint(-32000, "Authentication required"), None);
    }

    #[test]
    fn its_settings_say_how_it_is_signed_in() {
        assert_eq!(signed_in(br#"{"auth": {"type": "oauth-personal"}}"#).as_deref(), Some("oauth-personal"));
        assert_eq!(signed_in(b"{}"), None);
        assert_eq!(signed_in(b"not json"), None);
        let home = std::env::temp_dir().join("h");
        assert_eq!(settings_path(None, &home), home.join(".gemini").join("antigravity-acp").join("settings.json"));
        let gh = std::env::temp_dir().join("g");
        assert_eq!(settings_path(Some(&gh.to_string_lossy()), &home), gh.join("antigravity-acp").join("settings.json"));
    }
}
