//! Devin through `devin acp`, an Agent Client Protocol server on stdio (notes: docs/superpowers/notes/devin-cli.md):
//! the generic ACP adapter (`super::acp`) with Devin's profile, its arguments, modes and resume fallback.

use super::acp::AcpProfile;
#[cfg(test)]
use super::acp::initialize_params;
use super::{AdapterError, AgentAdapter, EventSink, LiveConversation, OpenSpec};
use pdt_core::conversation::PermissionMode;
use pdt_core::conversation::devin::{DEVIN_MODES, DevinExtensions, SESSION_NOT_FOUND, devin_mode, from_devin_mode};
use pdt_core::session::AgentKind;
use serde_json::Value;

/// The `devin` arguments: `--config <file>` (the user's config plus PDT's SessionStart hook, which hands Devin PDT's
/// instructions) when there is one, then `acp`.
pub fn devin_acp_args(spec: &OpenSpec) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(file) = &spec.config_file {
        a.push("--config".to_string());
        a.push(file.to_string_lossy().into_owned());
    }
    a.push("acp".into());
    a
}

/// Devin's ACP profile.
#[derive(Debug, Clone, Copy, Default)]
pub struct DevinProfile;

impl AcpProfile for DevinProfile {
    type Extensions = DevinExtensions;

    fn agent(&self) -> AgentKind {
        AgentKind::Devin
    }

    fn name(&self) -> &'static str {
        "Devin"
    }

    fn extensions(&self) -> DevinExtensions {
        DevinExtensions
    }

    fn args(&self, spec: &OpenSpec) -> Vec<String> {
        devin_acp_args(spec)
    }

    fn modes(&self) -> Vec<PermissionMode> {
        DEVIN_MODES.to_vec()
    }

    fn mode_id(&self, mode: PermissionMode) -> Option<&'static str> {
        Some(devin_mode(mode))
    }

    fn pdt_mode(&self, id: &str) -> Option<PermissionMode> {
        from_devin_mode(id)
    }

    /// Devin saves a session only at its first prompt.
    fn resume_fallback(&self, code: i64) -> Option<String> {
        (code == SESSION_NOT_FOUND).then(|| {
            "Devin had not saved this conversation (it saves one at the first message), so this is a new one."
                .to_string()
        })
    }
}

pub struct DevinAdapter;

#[async_trait::async_trait]
impl AgentAdapter for DevinAdapter {
    fn agent(&self) -> AgentKind {
        DevinProfile.agent()
    }

    async fn open(&self, spec: OpenSpec, sink: EventSink) -> Result<Box<dyn LiveConversation>, AdapterError> {
        super::acp::open(DevinProfile, spec, sink).await
    }

    async fn draft(&self, _spec: OpenSpec, _prompt: &str, _schema: &Value) -> Result<Value, AdapterError> {
        super::acp::draft(&DevinProfile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::conversation::PermissionMode;

    #[test]
    fn args_name_the_config_before_acp() {
        let mut spec = OpenSpec {
            session_id: "sid".into(),
            external_id: None,
            resume: false,
            cwd: std::env::temp_dir(),
            program: "devin".into(),
            search_path: None,
            model: None,
            permission_mode: PermissionMode::AcceptEdits,
            env: vec![],
            env_remove: vec![],
            instructions: None,
            config_file: None,
            import_after: None,
            mcp: None,
        };
        assert_eq!(devin_acp_args(&spec), vec!["acp"]);
        let file = std::env::temp_dir().join("devin-config.json");
        spec.config_file = Some(file.clone());
        assert_eq!(
            devin_acp_args(&spec),
            vec!["--config".to_string(), file.to_string_lossy().into_owned(), "acp".into()]
        );
        let p = initialize_params();
        assert_eq!(p["clientCapabilities"]["fs"]["writeTextFile"], false);
        assert!(p["clientCapabilities"]["elicitation"]["form"].is_object());
    }
}
