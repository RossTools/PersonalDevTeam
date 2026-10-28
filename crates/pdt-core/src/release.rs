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
    /// `arch` is Rust's name for the CPU (`std::env::consts::ARCH`: `x86_64`, `aarch64`); `appimage` when pdt-host runs
    /// the payload of an AppImage (its install folder holds [`APPIMAGE_MARKER`]), else the headless tarball.
    Linux {
        arch: &'static str,
        appimage: bool,
    },
    Other,
}

impl HostOs {
    /// This platform; on Linux, the tarball (see [`HostOs::current_linux`] for the AppImage).
    pub fn current() -> HostOs {
        Self::current_linux(false)
    }

    /// This platform, with `appimage` saying which Linux package is installed (ignored elsewhere).
    pub fn current_linux(appimage: bool) -> HostOs {
        if cfg!(windows) {
            HostOs::Windows
        } else if cfg!(target_os = "macos") {
            HostOs::Mac
        } else if cfg!(target_os = "linux") {
            HostOs::Linux { arch: std::env::consts::ARCH, appimage }
        } else {
            HostOs::Other
        }
    }
}

/// The CPUs Linux releases are built for (Rust's names, which the asset names use).
pub const LINUX_ARCHES: [&str; 2] = ["x86_64", "aarch64"];

/// The file in a Linux install folder that marks it as an AppImage's payload; it holds the AppImage's path.
pub const APPIMAGE_MARKER: &str = "appimage.path";

/// The headless Linux tarball for `arch`: `pdt-linux-x86_64.tar.gz` (a `pdt/` folder with pdtd, pdt-host and `web/`).
pub fn linux_tarball_name(arch: &str) -> String {
    format!("{}-linux-{arch}.tar.gz", crate::brand::SLUG)
}

/// The Linux AppImage for `arch`: `pdt-linux-x86_64.AppImage`.
pub fn linux_appimage_name(arch: &str) -> String {
    format!("{}-linux-{arch}.AppImage", crate::brand::SLUG)
}

/// The release asset each platform installs: the NSIS installer (`*-setup.exe`), the tarred `.app` bundle
/// (`*.app.tar.gz`), or on Linux the tarball or AppImage for this CPU, by exact name.
pub fn select_asset<'a>(names: &[&'a str], os: HostOs) -> Option<&'a str> {
    let suffix = |suffix: &str| names.iter().copied().find(|n| n.len() > suffix.len() && n.ends_with(suffix));
    match os {
        HostOs::Windows => suffix("-setup.exe"),
        HostOs::Mac => suffix(".app.tar.gz"),
        HostOs::Linux { arch, appimage } if LINUX_ARCHES.contains(&arch) => {
            let want = if appimage { linux_appimage_name(arch) } else { linux_tarball_name(arch) };
            names.iter().copied().find(|n| *n == want)
        }
        HostOs::Linux { .. } | HostOs::Other => None,
    }
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

/// Where the notes for an update start (PDT-169): the installed release's tag (`v0.1.741`), or the commit of a dev
/// build (`0.1.741+g<sha>`). None for a local build without a commit, or a version that is not ours.
pub fn compare_base(current: &str) -> Option<String> {
    let current = current.trim();
    if let Some((_, build)) = current.split_once('+')
        && let Some(sha) = build.strip_prefix('g')
        && sha.len() >= 7
        && sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Some(sha.to_string());
    }
    parse_version(current).filter(|v| !v.local).map(|v| format!("v{v}"))
}

/// One commit between two versions, read from its subject (merges are squashed: one per pull request).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub sha: String,
    /// The whole first line of the commit message.
    pub subject: String,
    /// The subject without its ticket key and pull request number.
    pub title: String,
    /// `PDT-167` from `PDT-167: …`.
    pub ticket: Option<String>,
    /// `177` from `… (#177)`.
    pub pr: Option<u64>,
}

/// Reads a commit: its subject is the message's first line, a leading `KEY-N:` its ticket and a trailing `(#N)` its
/// pull request.
pub fn parse_change(sha: &str, message: &str) -> Change {
    let subject = message.lines().next().unwrap_or("").trim().to_string();
    let mut rest = subject.as_str();
    let mut pr = None;
    if let Some(head) = rest.strip_suffix(')')
        && let Some((head, n)) = head.rsplit_once("(#")
        && let Ok(n) = n.parse::<u64>()
    {
        pr = Some(n);
        rest = head.trim_end();
    }
    let mut ticket = None;
    if let Some((key, after)) = rest.split_once(':')
        && let Some(k) = crate::ticket::parse_key(key)
        && let Some(project) = k.key
    {
        ticket = Some(crate::ticket::format_key(&project, k.number));
        rest = after.trim_start();
    }
    let title = if rest.is_empty() { subject.clone() } else { rest.to_string() };
    Change { sha: sha.to_string(), subject, title, ticket, pr }
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
    fn versions_order_by_semver_and_never_downgrade() {
        let v = |s| parse_version(s).unwrap();
        // Existing installs on the per-merge 0.1.x numbers move to the first deliberate release.
        assert!(v("v0.2.0").is_newer_than(&v("0.1.840")));
        assert!(!v("v0.1.840").is_newer_than(&v("0.2.0")), "never a downgrade");
        assert!(!v("v0.2.0").is_newer_than(&v("0.2.0")));
        assert!(v("0.2.1").is_newer_than(&v("0.2.0")));
        assert!(v("0.10.0").is_newer_than(&v("0.9.99")), "numeric, not lexical");
        assert!(v("1.0.0").is_newer_than(&v("0.99.99")));
        assert!(!v("0.99.99").is_newer_than(&v("1.0.0")));
        // Pre-releases are not ours: GitHub's latest release never is one, and a tag like this is refused.
        assert_eq!(parse_version("v0.3.0-rc.1"), None);
    }

    #[test]
    fn asset_selection_matches_ci_names() {
        let names = ["Personal.Dev.Team_0.1.7_x64-setup.exe", "Personal.Dev.Team.app.tar.gz", "SHA256SUMS"];
        assert_eq!(select_asset(&names, HostOs::Windows), Some("Personal.Dev.Team_0.1.7_x64-setup.exe"));
        assert_eq!(select_asset(&names, HostOs::Mac), Some("Personal.Dev.Team.app.tar.gz"));
        assert_eq!(select_asset(&names, HostOs::Other), None);
        assert_eq!(select_asset(&["-setup.exe"], HostOs::Windows), None);
        assert_eq!(select_asset(&names, HostOs::Linux { arch: "x86_64", appimage: false }), None);
    }

    #[test]
    fn linux_picks_its_own_package_and_cpu() {
        let names = [
            "Personal.Dev.Team_0.2.0_x64-setup.exe",
            "Personal.Dev.Team.app.tar.gz",
            "pdt-linux-x86_64.tar.gz",
            "pdt-linux-aarch64.tar.gz",
            "pdt-linux-x86_64.AppImage",
            "pdt-linux-aarch64.AppImage",
            "SHA256SUMS",
        ];
        let linux = |arch, appimage| select_asset(&names, HostOs::Linux { arch, appimage });
        assert_eq!(linux("x86_64", false), Some("pdt-linux-x86_64.tar.gz"));
        assert_eq!(linux("aarch64", false), Some("pdt-linux-aarch64.tar.gz"));
        assert_eq!(linux("x86_64", true), Some("pdt-linux-x86_64.AppImage"));
        assert_eq!(linux("aarch64", true), Some("pdt-linux-aarch64.AppImage"));
        assert_eq!(linux("riscv64", false), None, "no release for other CPUs");
        // The macOS tarball is never taken for the Linux one.
        assert_eq!(select_asset(&names[..2], HostOs::Linux { arch: "x86_64", appimage: false }), None);
        if cfg!(target_os = "linux") {
            assert_eq!(HostOs::current_linux(true), HostOs::Linux { arch: std::env::consts::ARCH, appimage: true });
        }
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

    #[test]
    fn compare_starts_at_the_installed_tag_or_a_dev_builds_commit() {
        assert_eq!(compare_base("0.1.741").as_deref(), Some("v0.1.741"));
        assert_eq!(compare_base("0.1.741+gabc1234def").as_deref(), Some("abc1234def"));
        assert_eq!(compare_base("0.1.0-local+g0123456").as_deref(), Some("0123456"));
        // A build suffix that is not a commit leaves the version's tag.
        assert_eq!(compare_base("0.1.3+abc").as_deref(), Some("v0.1.3"));
        assert_eq!(compare_base("0.1.3+gxyz").as_deref(), Some("v0.1.3"));
        assert_eq!(compare_base("0.1.0-local"), None);
        assert_eq!(compare_base("garbage"), None);
    }

    #[test]
    fn a_squashed_merge_names_its_ticket_and_pull_request() {
        let c = parse_change("abc", "PDT-167: PM conversations compact (#177)\n\nThe body.");
        assert_eq!(
            c,
            Change {
                sha: "abc".into(),
                subject: "PDT-167: PM conversations compact (#177)".into(),
                title: "PM conversations compact".into(),
                ticket: Some("PDT-167".into()),
                pr: Some(177),
            }
        );
        let c = parse_change("d", "Fix the build (#12)");
        assert_eq!((c.title.as_str(), c.ticket, c.pr), ("Fix the build", None, Some(12)));
        let c = parse_change("d", "pdt-3: lower-case key");
        assert_eq!((c.title.as_str(), c.ticket.as_deref(), c.pr), ("lower-case key", Some("PDT-3"), None));
        let c = parse_change("d", "Note: not a key (#x)");
        assert_eq!((c.title.as_str(), c.ticket, c.pr), ("Note: not a key (#x)", None, None));
        assert_eq!(parse_change("d", "").title, "");
    }
}
