//! The project link between a project's owner instance and its remote instances: the protocol version, pairing
//! codes, the request signature's text, the owner's address, the facts a remote instance reports and the connection
//! state the owner shows. Pure: pdt-api signs, sends and stores.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The project link's protocol. Both ends must speak the same one; the owner refuses any other with "update <the
/// older instance> to v<the newer's version>".
pub const PROTOCOL: u32 = 1;

/// Whether an instance speaking `theirs` can be linked with one speaking `ours` (this build's is [`PROTOCOL`]).
pub fn compatible(ours: u32, theirs: u32) -> bool {
    theirs == ours
}

/// One end of a link as the other knows it: its name, protocol and version.
#[derive(Debug, Clone, Copy)]
pub struct Side<'a> {
    pub name: &'a str,
    pub protocol: u32,
    pub version: &'a str,
}

/// What to tell the user about two ends whose protocols differ: the older one has to update to the newer one's
/// version.
pub fn incompatible_message(them: Side<'_>, us: Side<'_>) -> String {
    let (older, newer) = if them.protocol < us.protocol { (them, us) } else { (us, them) };
    format!("update {} to v{} to link it with {}", older.name, newer.version, newer.name)
}

/// How long a pairing code is good for.
pub const PAIRING_CODE_TTL_MS: i64 = 10 * 60 * 1000;
/// Wrong codes a project's pairing takes before its code stops working.
pub const PAIRING_MAX_FAILURES: u32 = 5;
/// The letters a pairing code uses: no 0/O, 1/I/L, so it reads aloud and types without mistakes.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
/// Characters in a code (two groups of four): 31^8, about 2^40.
const CODE_LEN: usize = 8;

/// A pairing code from random bytes (at least [`CODE_LEN`] of them): `ABCD-EFGH`.
pub fn pairing_code(random: &[u8]) -> String {
    let chars: String =
        random.iter().take(CODE_LEN).map(|b| CODE_ALPHABET[*b as usize % CODE_ALPHABET.len()] as char).collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

/// A code as the user typed it, comparable with [`pairing_code`]'s: upper case, spaces and dashes dropped, then
/// dashed again. None when it can't be a code.
pub fn normalize_code(input: &str) -> Option<String> {
    let chars: String =
        input.chars().filter(|c| !c.is_whitespace() && *c != '-').map(|c| c.to_ascii_uppercase()).collect();
    let ok = chars.len() == CODE_LEN && chars.bytes().all(|b| CODE_ALPHABET.contains(&b));
    ok.then(|| format!("{}-{}", &chars[..4], &chars[4..]))
}

/// The headers a signed link request carries.
pub mod headers {
    /// The link (one instance joined to one project), by the id the owner gave it at pairing.
    pub const LINK: &str = "x-pdt-link";
    /// The owner's id of the project the request is for.
    pub const PROJECT: &str = "x-pdt-link-project";
    /// The sender's [`super::PROTOCOL`].
    pub const PROTOCOL: &str = "x-pdt-link-protocol";
    /// When it was signed, in ms since the epoch.
    pub const TIME: &str = "x-pdt-link-time";
    /// A value used once.
    pub const NONCE: &str = "x-pdt-link-nonce";
    /// The base64 Ed25519 signature of [`super::signed_text`].
    pub const SIGNATURE: &str = "x-pdt-link-signature";
}

/// How far a signed request's time may be from the receiver's clock.
pub const MAX_SKEW_MS: i64 = 5 * 60 * 1000;

/// What a link request's signature covers: the method, the path, the link, the project, the time, the nonce and the
/// body's SHA-256, one per line after a tag naming the scheme.
pub struct SignedRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub link: &'a str,
    pub project: &'a str,
    pub time: i64,
    pub nonce: &'a str,
    pub body: &'a [u8],
}

pub fn signed_text(r: &SignedRequest<'_>) -> String {
    use sha2::Digest;
    let digest: String = sha2::Sha256::digest(r.body).iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "pdt-link-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{digest}",
        r.method.to_ascii_uppercase(),
        r.path,
        r.link,
        r.project,
        r.time,
        r.nonce
    )
}

/// Whether a request signed at `at` is recent enough at `now`.
pub fn fresh(at: i64, now: i64) -> bool {
    (now - at).abs() <= MAX_SKEW_MS
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("enter the other instance's address, e.g. studio.example.ts.net")]
    Empty,
    #[error("{0:?} isn't an address: enter a tailnet name such as studio.example.ts.net, or an https:// address")]
    Bad(String),
    #[error("plain http is only for this machine (localhost or 127.0.0.1); use the instance's https:// address")]
    Insecure,
}

/// The base URL of another instance from what the user typed: a bare tailnet name means `https://<name>`; an
/// `https://` URL is taken as it is (with its port); plain `http://` only on this machine (tests, two instances on one
/// machine). No trailing slash, no path.
pub fn instance_url(input: &str) -> Result<String, AddressError> {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err(AddressError::Empty);
    }
    let (scheme, rest) = match s.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("https".to_string(), s),
    };
    let host_port = rest.split('/').next().unwrap_or_default();
    let bad = || AddressError::Bad(input.trim().to_string());
    if rest.contains('/') && !rest.trim_end_matches('/').eq(host_port) {
        return Err(bad());
    }
    let host = match host_port.rsplit_once(':') {
        Some((h, port)) if !h.is_empty() && port.parse::<u16>().is_ok() => h,
        Some(_) => return Err(bad()),
        None => host_port,
    };
    let host_ok = !host.is_empty()
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b))
        && !host.starts_with(['-', '.']);
    if !host_ok {
        return Err(bad());
    }
    match scheme.as_str() {
        "https" => Ok(format!("https://{}", host_port.to_ascii_lowercase())),
        "http" if matches!(host, "localhost" | "127.0.0.1") => Ok(format!("http://{host_port}")),
        "http" => Err(AddressError::Insecure),
        _ => Err(bad()),
    }
}

/// Whether two remote URLs name the same repository: the same host and path (whatever the scheme or user), or the
/// same text without a trailing `/` or `.git`.
pub fn same_repo(a: &str, b: &str) -> bool {
    match (crate::git_url::parse_remote(a), crate::git_url::parse_remote(b)) {
        (Some(x), Some(y)) => x == y,
        _ => {
            let plain = |s: &str| s.trim().trim_end_matches('/').trim_end_matches(".git").to_string();
            plain(a) == plain(b)
        }
    }
}

/// An agent on a remote instance, as it reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentFact {
    /// `claude`, `codex`, `devin` or an ACP agent's kind.
    pub agent: String,
    pub installed: bool,
    /// Signed in, as far as PDT can tell (an ACP agent counts when it is found).
    pub logged_in: bool,
}

/// A tool found on a remote instance (git, godot, MSVC…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolFact {
    pub name: String,
    /// Where it was found.
    pub path: String,
}

/// What a remote instance reports about itself with each heartbeat.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct InstanceFacts {
    /// `macos`, `windows` or `linux` (Rust's `std::env::consts::OS`).
    pub os: String,
    pub arch: String,
    pub cpus: u32,
    /// Free space on the disk holding its data folder, when it could read it.
    #[ts(type = "number | null")]
    pub disk_free_bytes: Option<u64>,
    pub agents: Vec<AgentFact>,
    pub tools: Vec<ToolFact>,
}

/// The tools a remote instance looks for, each with the programs that count as it.
pub const TOOLS: &[(&str, &[&str])] = &[
    ("git", &["git"]),
    ("godot", &["godot", "godot4", "Godot"]),
    ("msvc", &["cl"]),
    ("cargo", &["cargo"]),
    ("node", &["node"]),
    ("python", &["python3", "python"]),
];

/// An instance's name, its version and protocol: the start of every answer on the link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstanceHello {
    /// The instance's own id (it never changes).
    pub instance_id: String,
    pub name: String,
    pub version: String,
    pub protocol: u32,
}

/// The remote instance's request to join a project, sent to its owner with the code the owner showed.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PairRequest {
    pub code: String,
    pub instance: InstanceHello,
    /// Where the owner reaches the remote instance (its own address on the tailnet).
    pub url: String,
    /// The remote instance's Ed25519 public key for this link (base64): it signs its reports with it.
    pub public_key: String,
}

/// The project a remote instance joined, as its owner describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LinkedProject {
    /// The owner's id of the project.
    pub id: String,
    pub key: String,
    pub name: String,
    pub base_branch: String,
    /// `origin`'s URL on the owner, when it has one: where the remote instance clones from.
    pub repo_url: Option<String>,
}

/// The owner's answer to a pairing.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PairReply {
    pub link_id: String,
    pub project: LinkedProject,
    pub owner: InstanceHello,
    /// The owner's Ed25519 public key for this link (base64): it signs its commands with it.
    pub public_key: String,
}

/// The remote instance's answer to a heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeartbeatReply {
    pub instance: InstanceHello,
    pub facts: InstanceFacts,
    /// The user paused the project on the remote instance: it takes no work for it.
    pub paused: bool,
}

/// A link's state, as one end shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LinkState {
    /// Heard from recently.
    Connected,
    /// Connected, but paused on the remote instance.
    Paused,
    /// Not heard from for a while (or never yet).
    Offline,
    /// It speaks another protocol: one of the two must update.
    Incompatible,
}

/// How long without a heartbeat before a link counts as offline.
pub const OFFLINE_AFTER_MS: i64 = 90 * 1000;
/// How often the owner sends heartbeats.
pub const HEARTBEAT_EVERY_MS: i64 = 30 * 1000;

/// A link's state from what was last heard: never, or at `last_seen`, whether its protocol fits ours, and the pause
/// it reported.
pub fn state(last_seen: Option<i64>, compatible: bool, paused: bool, now: i64) -> LinkState {
    if !compatible {
        return LinkState::Incompatible;
    }
    match last_seen {
        Some(at) if now - at <= OFFLINE_AFTER_MS => {
            if paused {
                LinkState::Paused
            } else {
                LinkState::Connected
            }
        }
        _ => LinkState::Offline,
    }
}

/// On a remote instance: a remote project's link to its owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OwnerLink {
    /// This instance's id of the remote project.
    pub project_id: String,
    pub link_id: String,
    pub owner_project_id: String,
    pub owner_instance_id: String,
    pub owner_name: String,
    /// Where the owner is reached (its address on the tailnet).
    pub owner_url: String,
    pub owner_version: String,
    /// The owner's Ed25519 public key for this link (base64), which its commands must be signed with.
    pub owner_public_key: String,
    pub repo_url: Option<String>,
    /// Where this instance keeps the project's checkout (and its worktrees); None: the default folder.
    #[serde(default)]
    pub checkout_path: Option<String>,
    /// Paused here: the project takes no work on this instance.
    pub paused: bool,
    /// When the owner last reached this instance, in ms.
    #[ts(type = "number | null")]
    pub last_contact_at: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
}

/// On the owner: an instance that joined one of its projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteInstance {
    pub link_id: String,
    pub project_id: String,
    pub instance_id: String,
    pub name: String,
    /// Where the owner reaches it.
    pub url: String,
    /// Its Ed25519 public key for this link (base64), which its reports must be signed with.
    pub public_key: String,
    pub version: String,
    pub protocol: u32,
    /// What it reported with its last heartbeat.
    pub facts: Option<InstanceFacts>,
    /// Paused on the remote instance (its last heartbeat said so).
    pub paused: bool,
    /// Its last answer to a heartbeat, in ms.
    #[ts(type = "number | null")]
    pub last_seen_at: Option<i64>,
    /// Why its last heartbeat failed, when it did.
    pub last_error: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_two_groups_from_the_unambiguous_alphabet() {
        let code = pairing_code(&[0, 1, 2, 3, 30, 31, 200, 255, 9, 9]);
        assert_eq!(code, "ABCD-9ARH");
        assert!(!code.contains(['0', 'O', '1', 'I', 'L']));
        assert_eq!(normalize_code(" abcd 9arh "), Some(code.clone()));
        assert_eq!(normalize_code("ABCD-9ARH"), Some(code));
        assert_eq!(normalize_code("ABCD-9AR"), None, "too short");
        assert_eq!(normalize_code("ABCD-9AR0"), None, "0 is never in a code");
    }

    #[test]
    fn the_signed_text_covers_every_part() {
        let r = SignedRequest {
            method: "post",
            path: "/api/link/heartbeat",
            link: "l1",
            project: "p1",
            time: 42,
            nonce: "n",
            body: b"{}",
        };
        let text = signed_text(&r);
        assert!(text.starts_with("pdt-link-v1\nPOST\n/api/link/heartbeat\nl1\np1\n42\nn\n"), "{text}");
        assert!(text.ends_with("44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"), "sha-256 of {{}}");
        let other = signed_text(&SignedRequest { project: "p2", ..r });
        assert_ne!(text, other);
    }

    #[test]
    fn freshness_allows_some_skew_either_way() {
        assert!(fresh(1_000, 1_000 + MAX_SKEW_MS));
        assert!(fresh(1_000 + MAX_SKEW_MS, 1_000));
        assert!(!fresh(0, MAX_SKEW_MS + 1));
    }

    #[test]
    fn addresses() {
        assert_eq!(instance_url("studio.example.ts.net"), Ok("https://studio.example.ts.net".into()));
        assert_eq!(
            instance_url(" https://Studio.example.ts.net:8443/ "),
            Ok("https://studio.example.ts.net:8443".into())
        );
        assert_eq!(instance_url("http://127.0.0.1:7421"), Ok("http://127.0.0.1:7421".into()));
        assert_eq!(instance_url("http://localhost:7421"), Ok("http://localhost:7421".into()));
        assert_eq!(instance_url("http://studio.example.ts.net"), Err(AddressError::Insecure));
        assert_eq!(instance_url(""), Err(AddressError::Empty));
        assert!(matches!(instance_url("https://studio/api/x"), Err(AddressError::Bad(_))));
        assert!(matches!(instance_url("ftp://studio"), Err(AddressError::Bad(_))));
        assert!(matches!(instance_url("two words"), Err(AddressError::Bad(_))));
        assert!(matches!(instance_url("studio:port"), Err(AddressError::Bad(_))));
    }

    #[test]
    fn the_same_repository_by_any_url() {
        assert!(same_repo("git@github.com:o/val.git", "https://github.com/o/val"));
        assert!(same_repo("ssh://git@github.com/o/val.git", "https://token@github.com/o/val.git"));
        assert!(!same_repo("git@github.com:o/val.git", "git@github.com:o/other.git"));
        assert!(!same_repo("git@github.com:o/val.git", "git@gitlab.com:o/val.git"));
        assert!(same_repo("/srv/git/val.git", "/srv/git/val"));
    }

    #[test]
    fn states() {
        let now = 1_000_000;
        assert_eq!(state(None, true, false, now), LinkState::Offline);
        assert_eq!(state(Some(now - 1_000), true, false, now), LinkState::Connected);
        assert_eq!(state(Some(now - 1_000), true, true, now), LinkState::Paused);
        assert_eq!(state(Some(now - OFFLINE_AFTER_MS - 1), true, false, now), LinkState::Offline);
        assert_eq!(state(Some(now), false, false, now), LinkState::Incompatible);
    }

    #[test]
    fn the_older_side_is_told_to_update() {
        let studio = Side { name: "studio", protocol: 2, version: "0.8.0" };
        let old = Side { name: "win-box", protocol: 1, version: "0.7.0" };
        assert_eq!(incompatible_message(old, studio), "update win-box to v0.8.0 to link it with studio");
        let new = Side { name: "win-box", protocol: 3, version: "0.9.1" };
        assert_eq!(incompatible_message(new, studio), "update studio to v0.9.1 to link it with win-box");
        assert!(compatible(PROTOCOL, PROTOCOL) && !compatible(PROTOCOL, PROTOCOL + 1));
    }
}
