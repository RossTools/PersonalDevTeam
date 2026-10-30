//! ACP agents defined as data (PDT-234; pdt-core `acp_agent`): a profile that ships with PDT or one the user added
//! in Settings → Agents, run through the generic ACP adapter (`super::acp`) as plain ACP. Their command, arguments
//! and environment come from the definition; their modes from its mode map, else from the `availableModes` they
//! advertise, else PDT keeps the mode itself (also the modes a built-in profile's map leaves out). PDT's instructions
//! go in front of a new session's first prompt. What a built-in profile's agent adds to ACP comes from its flavor
//! (`pdt_core::conversation::cursor`).

use super::acp::AcpProfile;
use super::{AdapterError, AgentAdapter, EventSink, LiveConversation, OpenSpec};
use pdt_core::acp_agent::{AcpAgentDef, AcpFlavor};
use pdt_core::conversation::PermissionMode;
use pdt_core::session::AgentKind;
use serde_json::Value;

/// A defined ACP agent's profile.
#[derive(Debug, Clone)]
pub struct DefinedProfile {
    pub kind: AgentKind,
    pub def: AcpAgentDef,
}

impl AcpProfile for DefinedProfile {
    type Extensions = AcpFlavor;

    fn agent(&self) -> AgentKind {
        self.kind
    }

    fn name(&self) -> &str {
        &self.def.label
    }

    fn extensions(&self) -> AcpFlavor {
        self.def.flavor
    }

    fn args(&self, _spec: &OpenSpec) -> Vec<String> {
        self.def.args.clone()
    }

    fn env(&self, _spec: &OpenSpec) -> Vec<(String, String)> {
        self.def.env.iter().map(|e| (e.name.clone(), e.value.clone())).collect()
    }

    fn modes(&self) -> Vec<PermissionMode> {
        self.def.modes.iter().map(|(m, _)| *m).collect()
    }

    fn mode_id(&self, mode: PermissionMode) -> Option<&str> {
        self.def.modes.iter().find(|(m, _)| *m == mode).map(|(_, id)| id.as_str())
    }

    fn pdt_mode(&self, id: &str) -> Option<PermissionMode> {
        self.def.modes.iter().find(|(_, i)| i == id).map(|(m, _)| *m)
    }

    fn resume_fallback(&self, code: i64, message: &str) -> Option<String> {
        self.def
            .flavor
            .session_missing(code, message)
            .then(|| format!("{} had no saved conversation to resume, so this is a new one.", self.def.label))
    }

    fn base_mode(&self) -> Option<&str> {
        self.def.base_mode.as_deref()
    }

    fn instructions_in_prompt(&self) -> bool {
        true
    }
}

/// The adapter for a defined ACP agent.
pub struct DefinedAdapter(pub DefinedProfile);

impl DefinedAdapter {
    /// The adapter for `def`, when its id is a valid agent kind.
    pub fn new(def: AcpAgentDef) -> Option<Self> {
        Some(DefinedAdapter(DefinedProfile { kind: def.kind()?, def }))
    }
}

#[async_trait::async_trait]
impl AgentAdapter for DefinedAdapter {
    fn agent(&self) -> AgentKind {
        self.0.kind
    }

    async fn open(&self, spec: OpenSpec, sink: EventSink) -> Result<Box<dyn LiveConversation>, AdapterError> {
        super::acp::open(self.0.clone(), spec, sink).await
    }

    async fn draft(&self, _spec: OpenSpec, _prompt: &str, _schema: &Value) -> Result<Value, AdapterError> {
        super::acp::draft(&self.0)
    }
}
