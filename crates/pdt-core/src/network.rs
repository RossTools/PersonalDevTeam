//! Where PDT listens (PDT-231): its port and the extra host names its UI may be opened from, set in Settings →
//! Network. Kept in a file of the data dir (`network.json`), not the database: pdtd binds its port before it opens
//! the database, and pdt-host and the tray need the port without one. A set `PDT_PORT` or `PDT_ALLOWED_HOSTS` wins
//! over its setting. Changes apply when pdtd next starts. Pure: types, paths and rules only.

use crate::brand::Profile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ts_rs::TS;

/// The settings file in the data dir.
pub fn network_path(data_dir: &Path) -> PathBuf {
    data_dir.join("network.json")
}

/// Set (to `1`) by pdt-host when the `PDT_PORT` it starts pdtd with is the setting's (or the default), not one from
/// the user's environment: pdtd then shows the port as a setting, not as set by `PDT_PORT`.
pub const PORT_FROM_SETTING_ENV: &str = "PDT_PORT_FROM_SETTING";

/// Written by pdtd once it listens: the port it took. pdt-host and the tray find a pdtd started before the setting
/// changed by it.
pub fn listening_path(data_dir: &Path) -> PathBuf {
    data_dir.join("listening-port")
}

/// The port in the listening-port file's text, if it holds one.
pub fn parse_listening(text: &str) -> Option<u16> {
    text.trim().parse().ok()
}

/// The lowest port the setting takes (below it needs privileges on most systems).
pub const MIN_PORT: u16 = 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct NetworkSettings {
    /// The port of the web UI and API; the profile's own (7420, or 7421 for dev) when unset.
    pub port: Option<u16>,
    /// Host names (with a port when it isn't the default one) the UI may be opened from, besides 127.0.0.1 and
    /// localhost on its port and the tailnet name remote access adds.
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetworkError {
    #[error("the port must be between 1024 and 65535")]
    BadPort,
    #[error("{0:?} is not a host name: give a name or address, with :port if needed, and nothing else")]
    BadHost(String),
}

impl NetworkSettings {
    /// The stored settings, or the defaults when the file is missing or unreadable (a broken file never stops pdtd
    /// from starting; Settings shows the defaults and saving writes it again).
    pub fn parse(bytes: &[u8]) -> NetworkSettings {
        serde_json::from_slice(bytes).unwrap_or_default()
    }

    /// Checked and tidied: hosts trimmed, blank and repeated ones dropped.
    pub fn validated(self) -> Result<NetworkSettings, NetworkError> {
        if self.port.is_some_and(|p| p < MIN_PORT) {
            return Err(NetworkError::BadPort);
        }
        let mut hosts: Vec<String> = Vec::new();
        for h in self.allowed_hosts.iter().map(|h| h.trim()).filter(|h| !h.is_empty()) {
            if !is_host(h) {
                return Err(NetworkError::BadHost(h.to_string()));
            }
            if !hosts.iter().any(|x| x.eq_ignore_ascii_case(h)) {
                hosts.push(h.to_string());
            }
        }
        Ok(NetworkSettings { port: self.port, allowed_hosts: hosts })
    }

    /// The port pdtd listens on: `PDT_PORT` when set, else the setting, else the profile's.
    pub fn port_for(&self, env_port: Option<u16>, profile: Profile) -> u16 {
        env_port.or(self.port).unwrap_or_else(|| profile.port())
    }
}

/// A `Host` header value: a name or address, optionally `:port`, with no scheme, path, spaces or commas.
fn is_host(h: &str) -> bool {
    h.len() <= 260
        && !h.contains("://")
        && h.chars().all(|c| c.is_ascii_alphanumeric() || "-._:[]".contains(c))
        && h.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '[')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_broken_file_reads_as_the_defaults() {
        assert_eq!(NetworkSettings::parse(b""), NetworkSettings::default());
        assert_eq!(NetworkSettings::parse(b"{not json"), NetworkSettings::default());
        let s = NetworkSettings::parse(br#"{"port": 7500, "allowedHosts": ["pdt.local"]}"#);
        assert_eq!((s.port, s.allowed_hosts), (Some(7500), vec!["pdt.local".to_string()]));
        assert_eq!(NetworkSettings::parse(br#"{"port": 7500}"#).allowed_hosts, Vec::<String>::new());
    }

    #[test]
    fn the_environment_wins_then_the_setting_then_the_profile() {
        let set = NetworkSettings { port: Some(7600), ..Default::default() };
        assert_eq!(set.port_for(Some(9000), Profile::Stable), 9000);
        assert_eq!(set.port_for(None, Profile::Stable), 7600);
        assert_eq!(NetworkSettings::default().port_for(None, Profile::Stable), 7420);
        assert_eq!(NetworkSettings::default().port_for(None, Profile::Dev), 7421);
    }

    #[test]
    fn validation_checks_the_port_and_tidies_the_hosts() {
        let s = |port, hosts: &[&str]| {
            NetworkSettings { port, allowed_hosts: hosts.iter().map(|h| h.to_string()).collect() }.validated()
        };
        assert_eq!(s(Some(80), &[]), Err(NetworkError::BadPort));
        assert_eq!(
            s(Some(7500), &[" pdt.local ", "", "PDT.local", "box.tail.ts.net:7500", "[::1]:7500", "10.0.0.2"])
                .unwrap()
                .allowed_hosts,
            ["pdt.local", "box.tail.ts.net:7500", "[::1]:7500", "10.0.0.2"]
        );
        for bad in ["http://pdt.local", "pdt.local/x", "two words", "a,b", ":7420"] {
            assert_eq!(s(None, &[bad]), Err(NetworkError::BadHost(bad.into())), "{bad}");
        }
    }
}
