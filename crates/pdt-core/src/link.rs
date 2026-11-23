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
    now.abs_diff(at) <= MAX_SKEW_MS.unsigned_abs()
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
    /// When its agents were last checked; none while it hasn't checked them yet (just after it started), when
    /// `agents` is empty because it doesn't know.
    #[ts(type = "number | null")]
    pub agents_checked_at: Option<i64>,
    pub tools: Vec<ToolFact>,
}

/// How old an instance's agent checks may be before it checks them again: a login since shows within this long.
pub const AGENT_FACTS_TTL_MS: i64 = 5 * 60 * 1000;

/// Whether an instance's agent checks, made at `checked_at` (none yet), are due to run again.
pub fn agent_facts_stale(checked_at: Option<i64>, now: i64) -> bool {
    checked_at.is_none_or(|at| now - at >= AGENT_FACTS_TTL_MS)
}

/// The facts to keep from a heartbeat: what came, except that an instance that hasn't checked its agents yet (it
/// just restarted) keeps the agents it reported before.
pub fn merge_facts(mut new: InstanceFacts, old: Option<&InstanceFacts>) -> InstanceFacts {
    if new.agents_checked_at.is_none()
        && new.agents.is_empty()
        && let Some(old) = old
    {
        new.agents = old.agents.clone();
        new.agents_checked_at = old.agents_checked_at;
    }
    new
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

/// The longest version another instance may report.
pub const MAX_VERSION: usize = 64;

/// Whether `v` is a version another instance may report (`0.41.2`, `0.1.0-local`, `1.2.3-rc.1+abc`): 1–[`MAX_VERSION`]
/// ASCII letters, digits, `.`, `-` and `+`. Anything else is not stored or shown.
pub fn valid_version(v: &str) -> bool {
    (1..=MAX_VERSION).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
}

/// The project as this instance keeps it once joined: the owner's name and base branch, checked as this instance's
/// own would be, and the owner's key when it is a valid key free here (else one suggested from the name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedProject {
    pub name: String,
    pub base_branch: String,
    pub key: String,
}

/// [`JoinedProject`] from what the owner sent, with `taken` this instance's project keys; an error names what the
/// owner sent that can't be used.
pub fn joined_project(p: &LinkedProject, taken: &[String]) -> Result<JoinedProject, String> {
    let name = p.name.trim();
    if name.is_empty() || name.chars().count() > crate::limits::TITLE || name.chars().any(char::is_control) {
        return Err("the owner sent a project name this instance can't use".into());
    }
    if !crate::branch_name::valid_branch_name(&p.base_branch) {
        return Err(format!("the owner sent a base branch this instance can't use: {:?}", p.base_branch));
    }
    let key = match crate::project::valid_key(&p.key) {
        Ok(k) if !taken.iter().any(|t| t.eq_ignore_ascii_case(&k)) => k,
        _ => crate::project::suggest_key(name, taken),
    };
    Ok(JoinedProject { name: name.to_string(), base_branch: p.base_branch.clone(), key })
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
    /// The names of the secrets the remote instance keeps for the project itself (never their values): they win
    /// over the owner's of the same name, which then isn't forwarded (PDT-637).
    #[serde(default)]
    pub secrets: Vec<String>,
    /// The most tickets of the project the remote instance takes at once (its own setting); None: no cap of its own.
    #[serde(default)]
    pub cap: Option<u32>,
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
    /// On a remote instance: the owner no longer has the link (it removed this instance or the project while the two
    /// couldn't reach each other). Leaving the project here is all that is left to do.
    Removed,
}

/// How long without a heartbeat before a link counts as offline.
pub const OFFLINE_AFTER_MS: i64 = 90 * 1000;
/// How often the owner sends heartbeats.
pub const HEARTBEAT_EVERY_MS: i64 = 30 * 1000;
/// How long an instance with work waiting for it may be offline before the owner notifies the user.
pub const OFFLINE_NOTIFY_AFTER_MS: i64 = 2 * 60 * 60 * 1000;
/// The tickets in flight on a remote instance at once when neither end sets a cap.
pub const DEFAULT_INSTANCE_CAP: u32 = 1;

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
    /// An update the owner asks for uses this instance's own update channel, not the owner's (PDT-637).
    #[serde(default)]
    pub own_update_channel: bool,
    /// The most of the project's tickets this instance takes at once (reported to the owner); None: no cap of its own.
    #[serde(default)]
    pub cap: Option<u32>,
    /// When the owner last reached this instance, in ms.
    #[ts(type = "number | null")]
    pub last_contact_at: Option<i64>,
    /// When the owner was found to have removed this instance from the project (it couldn't tell it then), in ms.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub removed_at: Option<i64>,
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
    /// The names of the secrets it keeps for the project itself, from its last heartbeat (PDT-637).
    #[serde(default)]
    pub secrets: Vec<String>,
    /// The most tickets in flight on it at once, as the owner's project settings say; None: no cap set here.
    #[serde(default)]
    pub cap: Option<u32>,
    /// The cap the remote instance set for the project (its last heartbeat said so).
    #[serde(default)]
    pub remote_cap: Option<u32>,
    #[ts(type = "number")]
    pub created_at: i64,
}

/// What the owner's update step says a joined instance needs: None when it can stay as it is. Ends that speak another
/// protocol must update one of them (the older; [`incompatible_message`]); an instance on an older version than the
/// owner's is offered the owner's (`update win-box to v0.7.3`). Local builds' versions aren't compared.
pub fn update_needed(them: Side<'_>, owner: Side<'_>) -> Option<String> {
    if !compatible(owner.protocol, them.protocol) {
        return Some(incompatible_message(them, owner));
    }
    let (theirs, ours) = (crate::release::parse_version(them.version)?, crate::release::parse_version(owner.version)?);
    let compared = !theirs.local && !ours.local;
    (compared && ours.is_newer_than(&theirs)).then(|| format!("update {} to v{}", them.name, owner.version))
}

impl RemoteInstance {
    /// Its slot pool's size: the lower of the owner's cap and its own, else [`DEFAULT_INSTANCE_CAP`].
    pub fn pool_cap(&self) -> u32 {
        match (self.cap, self.remote_cap) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => DEFAULT_INSTANCE_CAP,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_facts_go_stale_after_the_ttl() {
        assert!(agent_facts_stale(None, 0), "never checked");
        assert!(!agent_facts_stale(Some(1_000), 1_000 + AGENT_FACTS_TTL_MS - 1));
        assert!(agent_facts_stale(Some(1_000), 1_000 + AGENT_FACTS_TTL_MS));
    }

    #[test]
    fn an_instance_that_has_not_checked_its_agents_keeps_the_ones_it_reported() {
        let claude = AgentFact { agent: "claude".into(), installed: true, logged_in: true };
        let old = InstanceFacts { agents: vec![claude.clone()], agents_checked_at: Some(5), ..Default::default() };
        let restarted = InstanceFacts { cpus: 8, ..Default::default() };
        let kept = merge_facts(restarted, Some(&old));
        assert_eq!((kept.cpus, kept.agents.clone(), kept.agents_checked_at), (8, vec![claude.clone()], Some(5)));
        assert_eq!(merge_facts(InstanceFacts::default(), None).agents, vec![], "nothing known before");
        let logged_out = AgentFact { logged_in: false, ..claude };
        let checked =
            InstanceFacts { agents: vec![logged_out.clone()], agents_checked_at: Some(9), ..Default::default() };
        assert_eq!(merge_facts(checked, Some(&old)).agents, vec![logged_out], "a new check wins");
    }

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
    fn the_update_step_names_the_instance_to_update() {
        let owner = Side { name: "studio", protocol: PROTOCOL, version: "0.7.3" };
        let same = Side { name: "win-box", protocol: PROTOCOL, version: "0.7.3" };
        assert_eq!(update_needed(same, owner), None);
        let older = Side { version: "0.7.1", ..same };
        assert_eq!(update_needed(older, owner).as_deref(), Some("update win-box to v0.7.3"));
        assert_eq!(update_needed(owner, older), None, "a newer remote instance needs nothing from this step");
        let old_protocol = Side { protocol: PROTOCOL - 1, version: "0.6.0", ..same };
        assert_eq!(
            update_needed(old_protocol, owner).as_deref(),
            Some("update win-box to v0.7.3 to link it with studio")
        );
        let new_protocol = Side { protocol: PROTOCOL + 1, version: "0.8.0", ..same };
        assert_eq!(
            update_needed(new_protocol, owner).as_deref(),
            Some("update studio to v0.8.0 to link it with win-box")
        );
        let local = Side { version: "0.1.0-local", ..same };
        assert_eq!(update_needed(local, owner), None, "a local build isn't compared");
    }

    #[test]
    fn freshness_allows_some_skew_either_way() {
        assert!(fresh(1_000, 1_000 + MAX_SKEW_MS));
        assert!(fresh(1_000 + MAX_SKEW_MS, 1_000));
        assert!(!fresh(0, MAX_SKEW_MS + 1));
        assert!(!fresh(i64::MIN, 1_000), "a far-off time is stale, not an overflow");
        assert!(!fresh(i64::MAX, -1_000));
        assert!(!fresh(i64::MIN, i64::MAX));
    }

    #[test]
    fn versions_from_another_instance() {
        for ok in ["0.41.2", "0.1.0-local", "1.2.3-rc.1+abc"] {
            assert!(valid_version(ok), "{ok}");
        }
        for bad in ["", "1.0 beta", "1.0\n", "<b>1</b>", &"9".repeat(MAX_VERSION + 1)] {
            assert!(!valid_version(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_joined_project_is_checked_and_keeps_a_usable_key() {
        let p = |key: &str, name: &str, branch: &str| LinkedProject {
            id: "o1".into(),
            key: key.into(),
            name: name.into(),
            base_branch: branch.into(),
            repo_url: None,
        };
        let taken = vec!["PDT".to_string()];
        let j = joined_project(&p("WEB", " Web app ", "main"), &taken).unwrap();
        assert_eq!(j, JoinedProject { name: "Web app".into(), base_branch: "main".into(), key: "WEB".into() });
        assert_eq!(joined_project(&p("web", "Web app", "main"), &taken).unwrap().key, "WEB", "uppercased");
        assert_eq!(joined_project(&p("pdt", "Web app", "main"), &taken).unwrap().key, "WA", "taken in another case");
        for bad in ["", "a-b", "1AB", "TOOLONGKEY11"] {
            assert_eq!(joined_project(&p(bad, "Web app", "main"), &taken).unwrap().key, "WA", "{bad:?}");
        }
        assert!(joined_project(&p("WEB", "  ", "main"), &taken).is_err());
        assert!(joined_project(&p("WEB", "a\u{1b}[2J", "main"), &taken).is_err());
        assert!(joined_project(&p("WEB", &"x".repeat(201), "main"), &taken).is_err());
        assert!(joined_project(&p("WEB", "Web", "-main"), &taken).is_err());
        assert!(joined_project(&p("WEB", "Web", "a..b"), &taken).is_err());
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
    fn an_instances_pool_is_the_lower_of_both_caps() {
        let i = RemoteInstance {
            link_id: "l".into(),
            project_id: "p".into(),
            instance_id: "w".into(),
            name: "win-box".into(),
            url: String::new(),
            public_key: String::new(),
            version: String::new(),
            protocol: PROTOCOL,
            facts: None,
            paused: false,
            last_seen_at: None,
            last_error: None,
            secrets: vec![],
            cap: None,
            remote_cap: None,
            created_at: 0,
        };
        assert_eq!(i.pool_cap(), DEFAULT_INSTANCE_CAP);
        assert_eq!(RemoteInstance { cap: Some(3), ..i.clone() }.pool_cap(), 3);
        assert_eq!(RemoteInstance { remote_cap: Some(2), ..i.clone() }.pool_cap(), 2);
        assert_eq!(RemoteInstance { cap: Some(3), remote_cap: Some(2), ..i }.pool_cap(), 2);
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
