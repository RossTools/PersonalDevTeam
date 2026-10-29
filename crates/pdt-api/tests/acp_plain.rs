//! The generic ACP adapter (pdt-agents `adapter::acp`) with a plain profile, against `fake-agent --plain acp`: an ACP
//! agent with none of Devin's extra fields, no modes and no resume fallback.

use pdt_agents::adapter::acp::{self, AcpProfile};
use pdt_agents::adapter::{AdapterError, AdapterOutput, EventSink, LiveConversation, OpenSpec, Outgoing};
use pdt_core::conversation::acp::PlainAcp;
use pdt_core::conversation::{AgentEvent, Answer, Delivery, MessagePhase, PermissionMode, RequestKind};
use pdt_core::session::AgentKind;
use std::time::Duration;
use tokio::sync::mpsc;

struct Plain;

impl AcpProfile for Plain {
    type Extensions = PlainAcp;

    fn agent(&self) -> AgentKind {
        AgentKind::Devin
    }

    fn name(&self) -> &'static str {
        "Plain"
    }

    fn extensions(&self) -> PlainAcp {
        PlainAcp
    }

    fn args(&self, _spec: &OpenSpec) -> Vec<String> {
        vec!["--plain".into(), "acp".into()]
    }
}

fn spec(cwd: &std::path::Path) -> OpenSpec {
    OpenSpec {
        session_id: "sid".into(),
        external_id: None,
        resume: false,
        cwd: cwd.to_path_buf(),
        program: env!("CARGO_BIN_EXE_fake-agent").into(),
        search_path: None,
        model: None,
        permission_mode: PermissionMode::Bypass,
        env: vec![],
        env_remove: vec![],
        instructions: None,
        config_file: None,
        import_after: None,
        mcp: None,
    }
}

/// Waits for an output `f` picks something from.
async fn until<T>(
    rx: &mut mpsc::UnboundedReceiver<AdapterOutput>,
    mut f: impl FnMut(&AdapterOutput) -> Option<T>,
) -> T {
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

async fn answer_text(rx: &mut mpsc::UnboundedReceiver<AdapterOutput>) -> String {
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
    let (tx, mut rx): (EventSink, _) = mpsc::unbounded_channel();
    let live = acp::open(Plain, spec(tmp.path()), tx).await.unwrap();
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
    assert!(caps.permission_modes.is_empty(), "no modes to offer");
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
    live.close().await;

    // Resuming a session the agent never saved fails: plain ACP has no fallback.
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut s = spec(tmp.path());
    s.external_id = Some("never-saved".into());
    s.resume = true;
    match acp::open(Plain, s, tx).await {
        Err(AdapterError::Refused(m)) => assert_eq!(m, "Session not found"),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("opened a session that was never saved"),
    }
}
