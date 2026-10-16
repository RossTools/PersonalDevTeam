//! Per-host settings (spec §6). Pure: pdt-store stores them, pdt-api edits them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UpdatePolicy {
    AutoWhenIdle,
    Notify,
    Manual,
}

/// Whether agent sessions get the host's API-key variables (spec §6 "Agent credentials").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CredentialPolicy {
    /// ANTHROPIC_API_KEY and OPENAI_API_KEY are removed, so agents use the logged-in subscription.
    Subscription,
    /// Agents get the host's environment unchanged (API keys, Bedrock/Vertex variables, proxies).
    PassThrough,
}

impl CredentialPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            CredentialPolicy::Subscription => "subscription",
            CredentialPolicy::PassThrough => "pass_through",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [CredentialPolicy::Subscription, CredentialPolicy::PassThrough].into_iter().find(|c| c.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
}

pub const DEFAULT_RUN_PORTS: PortRange = PortRange { start: 8400, end: 8499 };
pub const DEFAULT_MAX_RUNS: u32 = 4;
const MAX_RUNS_LIMIT: u32 = 32;
const MAX_PORTS_IN_RANGE: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct HostSettings {
    pub remote_access: bool,
    /// The Tailscale login allowed to use this host remotely. `PDT_OWNER_LOGIN` overrides it.
    pub owner_login: Option<String>,
    pub update_policy: UpdatePolicy,
    pub run_ports: PortRange,
    pub max_concurrent_runs: u32,
    pub agent_credentials: CredentialPolicy,
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            remote_access: false,
            owner_login: None,
            update_policy: UpdatePolicy::Notify,
            run_ports: DEFAULT_RUN_PORTS,
            max_concurrent_runs: DEFAULT_MAX_RUNS,
            agent_credentials: CredentialPolicy::Subscription,
        }
    }
}

/// A partial update. `owner_login: Some("")` (or blank) clears the owner.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostSettingsPatch {
    #[serde(default)]
    #[ts(optional)]
    pub remote_access: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub owner_login: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub update_policy: Option<UpdatePolicy>,
    #[serde(default)]
    #[ts(optional)]
    pub run_ports: Option<PortRange>,
    #[serde(default)]
    #[ts(optional)]
    pub max_concurrent_runs: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub agent_credentials: Option<CredentialPolicy>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HostSettingsError {
    #[error("the run port range must lie within 1024–65535, start at or below its end, and hold at most 1000 ports")]
    BadPortRange,
    #[error("the maximum number of concurrent runs must be between 1 and 32")]
    BadMaxRuns,
    #[error("the owner login must not contain spaces or control characters")]
    BadOwner,
}

impl HostSettings {
    pub fn apply(&self, p: &HostSettingsPatch) -> Result<HostSettings, HostSettingsError> {
        let mut s = self.clone();
        if let Some(v) = p.remote_access {
            s.remote_access = v;
        }
        if let Some(raw) = &p.owner_login {
            let login = raw.trim();
            if login.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(HostSettingsError::BadOwner);
            }
            s.owner_login = (!login.is_empty()).then(|| login.to_string());
        }
        if let Some(v) = p.update_policy {
            s.update_policy = v;
        }
        if let Some(r) = p.run_ports {
            let size = u32::from(r.end).saturating_sub(u32::from(r.start)) + 1;
            if r.start < 1024 || r.start > r.end || size > MAX_PORTS_IN_RANGE {
                return Err(HostSettingsError::BadPortRange);
            }
            s.run_ports = r;
        }
        if let Some(n) = p.max_concurrent_runs {
            if !(1..=MAX_RUNS_LIMIT).contains(&n) {
                return Err(HostSettingsError::BadMaxRuns);
            }
            s.max_concurrent_runs = n;
        }
        if let Some(c) = p.agent_credentials {
            s.agent_credentials = c;
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch() -> HostSettingsPatch {
        HostSettingsPatch::default()
    }

    #[test]
    fn defaults_match_the_spec() {
        let s = HostSettings::default();
        assert!(!s.remote_access);
        assert_eq!(s.owner_login, None);
        assert_eq!(s.update_policy, UpdatePolicy::Notify);
        assert_eq!(s.run_ports, PortRange { start: 8400, end: 8499 });
        assert_eq!(s.max_concurrent_runs, 4);
        assert_eq!(s.agent_credentials, CredentialPolicy::Subscription);
    }

    #[test]
    fn missing_fields_take_defaults_when_reading_old_json() {
        let s: HostSettings = serde_json::from_str(r#"{"remoteAccess":true}"#).unwrap();
        assert!(s.remote_access);
        assert_eq!(s.max_concurrent_runs, 4);
    }

    #[test]
    fn wire_names() {
        assert_eq!(serde_json::to_string(&UpdatePolicy::AutoWhenIdle).unwrap(), r#""auto_when_idle""#);
        assert_eq!(serde_json::to_string(&CredentialPolicy::PassThrough).unwrap(), r#""pass_through""#);
        for c in [CredentialPolicy::Subscription, CredentialPolicy::PassThrough] {
            assert_eq!(CredentialPolicy::parse(c.as_str()), Some(c));
        }
        assert_eq!(CredentialPolicy::parse("api"), None);
    }

    #[test]
    fn patch_applies_only_given_fields() {
        let s = HostSettings::default();
        let p = HostSettingsPatch {
            update_policy: Some(UpdatePolicy::Manual),
            owner_login: Some("  me@example.com ".into()),
            ..patch()
        };
        let out = s.apply(&p).unwrap();
        assert_eq!(out.update_policy, UpdatePolicy::Manual);
        assert_eq!(out.owner_login.as_deref(), Some("me@example.com"));
        assert_eq!(out.run_ports, s.run_ports);
        let cleared = out.apply(&HostSettingsPatch { owner_login: Some("  ".into()), ..patch() }).unwrap();
        assert_eq!(cleared.owner_login, None);
    }

    #[test]
    fn patch_validation() {
        let s = HostSettings::default();
        let ports = |start, end| HostSettingsPatch { run_ports: Some(PortRange { start, end }), ..patch() };
        assert_eq!(s.apply(&ports(9000, 8999)), Err(HostSettingsError::BadPortRange));
        assert_eq!(s.apply(&ports(80, 90)), Err(HostSettingsError::BadPortRange));
        assert_eq!(s.apply(&ports(10000, 11000)), Err(HostSettingsError::BadPortRange), "1001 ports");
        assert!(s.apply(&ports(10000, 10999)).is_ok());
        let runs = |n| HostSettingsPatch { max_concurrent_runs: Some(n), ..patch() };
        assert_eq!(s.apply(&runs(0)), Err(HostSettingsError::BadMaxRuns));
        assert_eq!(s.apply(&runs(33)), Err(HostSettingsError::BadMaxRuns));
        assert!(s.apply(&runs(32)).is_ok());
        let owner = |o: &str| HostSettingsPatch { owner_login: Some(o.into()), ..patch() };
        assert_eq!(s.apply(&owner("two words")), Err(HostSettingsError::BadOwner));
        assert_eq!(s.apply(&owner("a\u{7}b")), Err(HostSettingsError::BadOwner));
    }
}
