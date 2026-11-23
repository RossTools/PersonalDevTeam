//! Which instance of PDT this daemon is (stable or a development one), reported by `/api/health`.

use crate::brand::{Profile, STABLE_PORT};
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstanceInfo {
    pub profile: Profile,
    pub port: u16,
    /// A development instance: the dev profile, or any port other than the stable one (e.g. a "pdt" run).
    pub dev: bool,
}

impl InstanceInfo {
    pub fn new(profile: Profile, port: u16) -> Self {
        Self { profile, port, dev: profile == Profile::Dev || port != STABLE_PORT }
    }
}

/// The longest instance name.
pub const MAX_NAME: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error(
        "an instance name is 1 to 32 letters, digits, '-', '_' or '.', starting with a letter or digit (e.g. studio, win-box)"
    )]
    Bad,
}

/// An instance's name (Settings → This host, e.g. `studio`, `win-box`), trimmed: 1–[`MAX_NAME`] ASCII letters, digits,
/// `-`, `_` and `.`, starting with a letter or digit. It shows wherever an instance is mentioned, and goes in labels
/// such as `pdt:<name>`, so it has no spaces.
pub fn valid_name(input: &str) -> Result<String, NameError> {
    let s = input.trim();
    let ok = (1..=MAX_NAME).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_alphanumeric())
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    if ok { Ok(s.to_string()) } else { Err(NameError::Bad) }
}

/// The name an instance goes by before the user picks one: the machine's host name without its domain, lower case,
/// with anything a name can't hold turned into `-`; `pdt` when nothing is left.
pub fn suggest_name(host_name: &str) -> String {
    let short = host_name.trim().split('.').next().unwrap_or_default().to_ascii_lowercase();
    let mut out = String::new();
    for c in short.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' };
        if !(c == '-' && (out.is_empty() || out.ends_with('-'))) {
            out.push(c);
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(MAX_NAME).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { crate::brand::SLUG.to_string() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brand::DEV_PORT;

    #[test]
    fn stable_profile_on_stable_port_is_not_dev() {
        assert!(!InstanceInfo::new(Profile::Stable, STABLE_PORT).dev);
    }

    #[test]
    fn dev_profile_is_dev_on_any_port() {
        assert!(InstanceInfo::new(Profile::Dev, DEV_PORT).dev);
        assert!(InstanceInfo::new(Profile::Dev, STABLE_PORT).dev);
    }

    #[test]
    fn stable_profile_on_another_port_is_dev() {
        let info = InstanceInfo::new(Profile::Stable, 8512);
        assert!(info.dev);
        assert_eq!((info.profile, info.port), (Profile::Stable, 8512));
    }

    #[test]
    fn serializes_profile_lowercase() {
        let v = serde_json::to_value(InstanceInfo::new(Profile::Dev, 8512)).unwrap();
        assert_eq!(v, serde_json::json!({"profile": "dev", "port": 8512, "dev": true}));
    }

    #[test]
    fn instance_names() {
        assert_eq!(valid_name("  win-box "), Ok("win-box".into()));
        assert_eq!(valid_name("studio.2"), Ok("studio.2".into()));
        for bad in ["", "two words", "-x", ".x", "naïve", &"a".repeat(MAX_NAME + 1)] {
            assert_eq!(valid_name(bad), Err(NameError::Bad), "{bad:?}");
        }
    }

    #[test]
    fn a_name_is_suggested_from_the_host_name() {
        assert_eq!(suggest_name("Studio.local"), "studio");
        assert_eq!(suggest_name("Alex's MacBook Pro"), "alex-s-macbook-pro");
        assert_eq!(suggest_name("WIN-BOX"), "win-box");
        assert_eq!(suggest_name("--"), crate::brand::SLUG);
        assert_eq!(suggest_name(""), crate::brand::SLUG);
        let long = suggest_name(&"ab-".repeat(20));
        assert!(long.len() <= MAX_NAME && !long.ends_with('-'), "{long}");
        assert!(valid_name(&long).is_ok());
    }
}
