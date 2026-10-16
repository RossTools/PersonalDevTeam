//! Releases and updates (remote-host spec §3): versions, assets, checksums, token warnings. Pure.

use serde::Serialize;
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// A local build (`X.Y.Z-local`): older than every release.
    pub local: bool,
}

impl Version {
    pub fn is_newer_than(&self, current: &Version) -> bool {
        if self.local {
            return false;
        }
        current.local || (self.major, self.minor, self.patch) > (current.major, current.minor, current.patch)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}{}", self.major, self.minor, self.patch, if self.local { "-local" } else { "" })
    }
}

/// `X.Y.Z`, `vX.Y.Z` or `X.Y.Z-local`, with an optional `+build` suffix. Other pre-releases are not ours.
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    let s = s.split('+').next()?;
    let (core, local) = match s.split_once('-') {
        Some((c, "local")) => (c, true),
        Some(_) => return None,
        None => (s, false),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = Version { major: parts.next()??, minor: parts.next()??, patch: parts.next()??, local };
    parts.next().is_none().then_some(v)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Windows,
    Mac,
    Other,
}

impl HostOs {
    pub fn current() -> HostOs {
        if cfg!(windows) {
            HostOs::Windows
        } else if cfg!(target_os = "macos") {
            HostOs::Mac
        } else {
            HostOs::Other
        }
    }
}

/// The release asset each platform installs: the NSIS installer, or the tarred `.app` bundle.
pub fn asset_suffix(os: HostOs) -> Option<&'static str> {
    match os {
        HostOs::Windows => Some("-setup.exe"),
        HostOs::Mac => Some(".app.tar.gz"),
        HostOs::Other => None,
    }
}

pub fn select_asset<'a>(names: &[&'a str], os: HostOs) -> Option<&'a str> {
    let suffix = asset_suffix(os)?;
    names.iter().copied().find(|n| n.len() > suffix.len() && n.ends_with(suffix))
}

pub const SUMS_ASSET: &str = "SHA256SUMS";

/// `sha256sum` output: `<64 hex>  <name>` or `<64 hex> *<name>` → name → lower-case hex. Other lines are ignored.
pub fn parse_sums(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (hash, rest) = l.split_once(' ')?;
            let name = rest.trim_start_matches(' ').trim_start_matches('*').trim_end();
            (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) && !name.is_empty())
                .then(|| (name.to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

pub const TOKEN_WARN_MS: i64 = 14 * 86_400_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum TokenState {
    Missing,
    Valid {
        #[ts(type = "number | null")]
        expires_at: Option<i64>,
    },
    ExpiresSoon {
        #[ts(type = "number")]
        expires_at: i64,
    },
    /// Expired, or GitHub rejected it (revoked).
    Expired,
}

/// Warns `TOKEN_WARN_MS` (14 days) ahead of expiry (spec §3.2).
pub fn token_state(present: bool, expires_at: Option<i64>, rejected: bool, now: i64) -> TokenState {
    match (present, expires_at) {
        (false, _) => TokenState::Missing,
        _ if rejected => TokenState::Expired,
        (true, Some(t)) if t <= now => TokenState::Expired,
        (true, Some(t)) if t - now <= TOKEN_WARN_MS => TokenState::ExpiresSoon { expires_at: t },
        (true, t) => TokenState::Valid { expires_at: t },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_local_builds() {
        let v = |s| parse_version(s).unwrap();
        assert_eq!(v("v0.1.42"), Version { major: 0, minor: 1, patch: 42, local: false });
        assert_eq!(v("0.1.0-local"), Version { major: 0, minor: 1, patch: 0, local: true });
        assert_eq!(v("0.1.3+abc"), Version { major: 0, minor: 1, patch: 3, local: false });
        for bad in ["", "v", "1.2", "1.2.3.4", "0.1.0-rc.1", "latest", "0.x.1"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert!(v("0.1.10").is_newer_than(&v("0.1.9")));
        assert!(!v("0.1.9").is_newer_than(&v("0.1.9")));
        assert!(v("0.1.1").is_newer_than(&v("0.1.0-local")), "any release beats a local build");
        assert!(v("0.1.1").is_newer_than(&v("0.9.9-local")));
        assert!(!v("0.9.9-local").is_newer_than(&v("0.1.1")), "a local build never counts as newer");
        assert_eq!(v("0.1.0-local").to_string(), "0.1.0-local");
        assert_eq!(v("v0.1.7").to_string(), "0.1.7");
    }

    #[test]
    fn asset_selection_matches_ci_names() {
        let names = ["Personal.Dev.Team_0.1.7_x64-setup.exe", "Personal.Dev.Team.app.tar.gz", "SHA256SUMS"];
        assert_eq!(select_asset(&names, HostOs::Windows), Some("Personal.Dev.Team_0.1.7_x64-setup.exe"));
        assert_eq!(select_asset(&names, HostOs::Mac), Some("Personal.Dev.Team.app.tar.gz"));
        assert_eq!(select_asset(&names, HostOs::Other), None);
        assert_eq!(select_asset(&["-setup.exe"], HostOs::Windows), None);
    }

    #[test]
    fn sums_file() {
        let text = "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789  a-setup.exe\n\
                    0000000000000000000000000000000000000000000000000000000000000000 *b.app.tar.gz\n\
                    garbage line\n";
        let s = parse_sums(text);
        assert_eq!(s["a-setup.exe"], "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789");
        assert_eq!(s["b.app.tar.gz"], "0".repeat(64));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn token_warnings() {
        let day = 86_400_000;
        assert_eq!(token_state(false, None, false, 0), TokenState::Missing);
        assert_eq!(token_state(true, None, false, 0), TokenState::Valid { expires_at: None });
        assert_eq!(token_state(true, Some(30 * day), false, 0), TokenState::Valid { expires_at: Some(30 * day) });
        assert_eq!(token_state(true, Some(14 * day), false, 0), TokenState::ExpiresSoon { expires_at: 14 * day });
        assert_eq!(token_state(true, Some(10), false, 20), TokenState::Expired);
        assert_eq!(
            token_state(true, Some(30 * day), true, 0),
            TokenState::Expired,
            "rejected means expired or revoked"
        );
        assert_eq!(
            serde_json::to_string(&TokenState::ExpiresSoon { expires_at: 5 }).unwrap(),
            r#"{"state":"expires_soon","expiresAt":5}"#
        );
    }
}
