//! The generic ACP adapter (pdt-agents `adapter::acp`) with a plain profile, against `fake-agent --plain acp`: an ACP
//! agent with none of Devin's extra fields, no modes and no resume fallback; and with `--modes`, one that advertises
//! its modes.

use pdt_agents::adapter::acp::{self, AcpProfile};
use pdt_agents::adapter::{
    AdapterError, AdapterOutput, EVENT_QUEUE, EventSink, EventStream, LiveConversation, OpenSpec, Outgoing,
    SessionChange, event_channel,
};
use pdt_core::conversation::acp::PlainAcp;
use pdt_core::conversation::{AgentEvent, Answer, Delivery, MessagePhase, PermissionMode, RequestKind};
use pdt_core::session::AgentKind;
use std::time::Duration;

/// A plain ACP agent, with the `fake-agent` flag that says which modes it advertises (if any).
struct Plain(Option<&'static str>);

impl AcpProfile for Plain {
    type Extensions = PlainAcp;

    fn agent(&self) -> AgentKind {
        AgentKind::Devin
    }

    fn name(&self) -> &str {
        "Plain"
    }

    fn instructions_in_prompt(&self) -> bool {
        true
    }

    fn extensions(&self) -> PlainAcp {
        PlainAcp
    }

    fn args(&self, _spec: &OpenSpec) -> Vec<String> {
        let mut a = vec!["--plain".to_string(), "acp".into()];
        if let Some(flag) = self.0 {
            a.insert(0, flag.into());
        }
        a
    }
}

fn spec(cwd: &std::path::Path, permission_mode: PermissionMode) -> OpenSpec {
    OpenSpec {
        session_id: "sid".into(),
        external_id: None,
        resume: false,
        cwd: cwd.to_path_buf(),
        program: env!("CARGO_BIN_EXE_fake-agent").into(),
        search_path: None,
        model: None,
        permission_mode,
        env: vec![],
        env_remove: vec![],
        instructions: None,
        config_file: None,
        import_after: None,
        mcp: None,
    }
}

/// Waits for an output `f` picks something from.
async fn until<T>(rx: &mut EventStream, mut f: impl FnMut(&AdapterOutput) -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let o = rx.recv().await.expect("the adapter stopped reporting");
            if let Some(t) = f(&o) {
                return t;
            }
        }
    })
    .await
    .expect("timed out")
}

async fn answer_text(rx: &mut EventStream) -> String {
    until(rx, |o| match o {
        AdapterOutput::Event {
            event: AgentEvent::AgentMessage { markdown, phase: MessagePhase::Final, .. }, ..
        } => Some(markdown.clone()),
        _ => None,
    })
    .await
}

async fn say(live: &dyn LiveConversation, text: &str) {
    live.send(&Outgoing { text: text.into(), images: vec![] }, Delivery::Next).await.unwrap();
}

#[tokio::test]
async fn a_plain_acp_agent_converses_asks_in_pdt_and_has_no_modes_or_resume_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let (tx, mut rx): (EventSink, _) = event_channel(EVENT_QUEUE);
    let live = acp::open(Plain(None), spec(tmp.path(), PermissionMode::Ask), tx).await.unwrap();
    let session = until(&mut rx, |o| match o {
        AdapterOutput::ExternalId(id) => Some(id.clone()),
        _ => None,
    })
    .await;
    let caps = until(&mut rx, |o| match o {
        AdapterOutput::Capabilities(c) => Some(c.clone()),
        _ => None,
    })
    .await;
    assert_eq!(caps.permission_modes, PermissionMode::ALL.to_vec(), "PDT keeps every mode itself");
    assert_eq!(caps.models.len(), 2);

    // No mode map: PDT never sets the agent's mode, whatever the session's permission mode.
    say(live.as_ref(), "who").await;
    let who = answer_text(&mut rx).await;
    assert!(who.contains(&format!("session={session}")) && who.contains("mode=accept-edits"), "{who}");

    // Permission requests are answered in PDT; with no session-only option, "always" allows once.
    say(live.as_ref(), "run: ls").await;
    let (id, tool, can_always) = until(&mut rx, |o| match o {
        AdapterOutput::Event {
            event: AgentEvent::Request { id, request_kind: RequestKind::Approval, tool, can_always, .. },
            ..
        } => Some((id.clone(), tool.clone(), *can_always)),
        _ => None,
    })
    .await;
    assert_eq!((tool.as_str(), can_always), ("execute", false), "named by its kind");
    live.answer(&id, &Answer::Approve { always: true }).await.unwrap();
    assert_eq!(answer_text(&mut rx).await, "ran it (allow_once)");

    // In Bypass, PDT allows it without asking; in Plan, it refuses it.
    live.set(&SessionChange { permission_mode: Some(PermissionMode::Bypass), ..Default::default() }).await.unwrap();
    say(live.as_ref(), "run: pwd").await;
    assert_eq!(answer_text(&mut rx).await, "ran it (allow_once)");
    live.set(&SessionChange { permission_mode: Some(PermissionMode::Plan), ..Default::default() }).await.unwrap();
    say(live.as_ref(), "run: rm x").await;
    assert_eq!(answer_text(&mut rx).await, "not run (reject_once)");
    live.close().await;

    // Resuming a session the agent never saved fails: plain ACP has no fallback.
    let (tx, _rx) = event_channel(EVENT_QUEUE);
    let mut s = spec(tmp.path(), PermissionMode::Ask);
    s.external_id = Some("never-saved".into());
    s.resume = true;
    match acp::open(Plain(None), s, tx).await {
        Err(AdapterError::Refused(m)) => assert_eq!(m, "Session not found"),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("opened a session that was never saved"),
    }
}

#[tokio::test]
async fn an_agent_that_advertises_its_modes_has_them_set_and_gets_instructions_in_its_first_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let (tx, mut rx): (EventSink, _) = event_channel(EVENT_QUEUE);
    let mut s = spec(tmp.path(), PermissionMode::Plan);
    let file = tmp.path().join("instructions.md");
    s.instructions = Some(pdt_agents::adapter::SessionInstructions { text: "PDT says hello".into(), file });
    let live = acp::open(Plain(Some("--modes")), s, tx).await.unwrap();
    // The capabilities once the session is open (the model list comes before).
    let caps = until(&mut rx, |o| match o {
        AdapterOutput::Capabilities(c) if !c.permission_modes.is_empty() => Some(c.clone()),
        _ => None,
    })
    .await;
    assert_eq!(caps.permission_modes, PermissionMode::ALL.to_vec(), "the four it advertised");
    say(live.as_ref(), "who").await;
    let who = answer_text(&mut rx).await;
    assert!(who.contains("mode=plan") && who.contains("instructions=true"), "{who}");
    // The agent asks in its own modes: PDT passes the request on.
    live.set(&SessionChange { permission_mode: Some(PermissionMode::Bypass), ..Default::default() }).await.unwrap();
    say(live.as_ref(), "who").await;
    assert!(answer_text(&mut rx).await.contains("mode=bypass"));
    say(live.as_ref(), "run: ls").await;
    until(&mut rx, |o| match o {
        AdapterOutput::Event { event: AgentEvent::Request { .. }, .. } => Some(()),
        _ => None,
    })
    .await;
    live.close().await;
}

#[tokio::test]
async fn pdt_keeps_a_mode_the_agent_has_none_for_and_hands_the_ones_it_has_back() {
    let tmp = tempfile::tempdir().unwrap();
    let (tx, mut rx): (EventSink, _) = event_channel(EVENT_QUEUE);
    // Only `plan` maps (OpenCode's build/plan): Bypass, a ticket's default, is PDT's to keep.
    let live = acp::open(Plain(Some("--partial-modes")), spec(tmp.path(), PermissionMode::Bypass), tx).await.unwrap();
    let caps = until(&mut rx, |o| match o {
        AdapterOutput::Capabilities(c) if !c.permission_modes.is_empty() => Some(c.clone()),
        _ => None,
    })
    .await;
    assert_eq!(caps.permission_modes, PermissionMode::ALL.to_vec(), "PDT keeps the ones it has no mode for");
    say(live.as_ref(), "run: ls").await;
    assert_eq!(answer_text(&mut rx).await, "ran it (allow_once)", "allowed without asking");
    say(live.as_ref(), "who").await;
    assert!(answer_text(&mut rx).await.contains("mode=build"), "the agent stays in its own mode");

    // Plan is the agent's: set there, and its requests go to the user.
    live.set(&SessionChange { permission_mode: Some(PermissionMode::Plan), ..Default::default() }).await.unwrap();
    say(live.as_ref(), "who").await;
    assert!(answer_text(&mut rx).await.contains("mode=plan"));
    say(live.as_ref(), "run: ls").await;
    let id = until(&mut rx, |o| match o {
        AdapterOutput::Event { event: AgentEvent::Request { id, .. }, .. } => Some(id.clone()),
        _ => None,
    })
    .await;
    live.answer(&id, &Answer::Deny { message: None }).await.unwrap();
    assert_eq!(answer_text(&mut rx).await, "not run (reject_once)");

    // Back to Bypass: PDT keeps it again, with the agent back in the mode it started in.
    live.set(&SessionChange { permission_mode: Some(PermissionMode::Bypass), ..Default::default() }).await.unwrap();
    say(live.as_ref(), "who").await;
    assert!(answer_text(&mut rx).await.contains("mode=build"));
    say(live.as_ref(), "run: pwd").await;
    assert_eq!(answer_text(&mut rx).await, "ran it (allow_once)");
    live.close().await;
}

/// A profile whose own mode map covers only Plan, with the mode PDT keeps the others in, for an agent that edits
/// without asking (Cursor's: `plan`, and `agent` for the rest; here `fake-agent --partial-modes`'s `plan` and
/// `build`).
struct Mapped;

impl AcpProfile for Mapped {
    type Extensions = PlainAcp;

    fn agent(&self) -> AgentKind {
        AgentKind::Devin
    }

    fn name(&self) -> &str {
        "Mapped"
    }

    fn extensions(&self) -> PlainAcp {
        PlainAcp
    }

    fn args(&self, _spec: &OpenSpec) -> Vec<String> {
        vec!["--partial-modes".into(), "--plain".into(), "acp".into()]
    }

    fn modes(&self) -> Vec<PermissionMode> {
        vec![PermissionMode::Plan]
    }

    fn mode_id(&self, mode: PermissionMode) -> Option<&str> {
        (mode == PermissionMode::Plan).then_some("plan")
    }

    fn pdt_mode(&self, id: &str) -> Option<PermissionMode> {
        (id == "plan").then_some(PermissionMode::Plan)
    }

    fn base_mode(&self) -> Option<&str> {
        Some("build")
    }

    fn asks_before_edits(&self) -> bool {
        false
    }
}

#[tokio::test]
async fn a_profile_mapping_only_plan_keeps_the_rest_in_its_base_mode_and_runs_ask_as_accept_edits() {
    let tmp = tempfile::tempdir().unwrap();
    let (tx, mut rx): (EventSink, _) = event_channel(EVENT_QUEUE);
    let live = acp::open(Mapped, spec(tmp.path(), PermissionMode::Plan), tx).await.unwrap();
    let caps = until(&mut rx, |o| match o {
        AdapterOutput::Capabilities(c) if !c.permission_modes.is_empty() => Some(c.clone()),
        _ => None,
    })
    .await;
    use PermissionMode::*;
    assert_eq!(caps.permission_modes, vec![AcceptEdits, Plan, Bypass], "Plan is the profile's, the rest PDT's; no Ask");
    say(live.as_ref(), "who").await;
    assert!(answer_text(&mut rx).await.contains("mode=plan"));

    // Bypass has no mode of the agent's: PDT puts it in its base mode and allows without asking.
    live.set(&SessionChange { permission_mode: Some(PermissionMode::Bypass), ..Default::default() }).await.unwrap();
    say(live.as_ref(), "who").await;
    assert!(answer_text(&mut rx).await.contains("mode=build"));
    say(live.as_ref(), "run: ls").await;
    assert_eq!(answer_text(&mut rx).await, "ran it (allow_once)");

    // Ask: the agent would edit without asking, so the session runs as Accept edits, and says so.
    live.set(&SessionChange { permission_mode: Some(Ask), ..Default::default() }).await.unwrap();
    let mode = until(&mut rx, |o| match o {
        AdapterOutput::Info(i) => i.permission_mode,
        _ => None,
    })
    .await;
    assert_eq!(mode, AcceptEdits);
    say(live.as_ref(), "who").await;
    assert!(answer_text(&mut rx).await.contains("mode=build"));
    // Commands still go to the user.
    say(live.as_ref(), "run: ls").await;
    let id = until(&mut rx, |o| match o {
        AdapterOutput::Event { event: AgentEvent::Request { id, .. }, .. } => Some(id.clone()),
        _ => None,
    })
    .await;
    live.answer(&id, &Answer::Deny { message: None }).await.unwrap();
    assert_eq!(answer_text(&mut rx).await, "not run (reject_once)");
    live.close().await;
}
