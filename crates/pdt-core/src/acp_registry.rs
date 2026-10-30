//! The ACP registry (PDT-239): the list of ACP agents at agentclientprotocol.com, which says where a built-in profile's
//! program is published (an archive per platform, with its sha256, and the command inside it). PDT installs those
//! programs itself (pdt-api `toolchains`); this module reads the registry and checks what it says before anything is
//! downloaded or unpacked. No I/O.

use serde_json::Value;

/// Where PDT reads the registry.
pub const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// The only place an archive may come from, by registry id: its host and path prefix (https only).
const SOURCES: &[(&str, &str, &str)] =
    &[(crate::acp_agent::ANTIGRAVITY_REGISTRY_ID, "dl.google.com", "/agy-extensions/releases/")];

/// The largest archive PDT downloads.
pub const MAX_ARCHIVE_BYTES: u64 = 1 << 30;

/// One platform's build of a registry agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryRelease {
    pub id: String,
    pub version: String,
    /// The archive's URL (checked: https, from the agent's own source).
    pub archive: String,
    /// The archive's sha256, lowercase hex.
    pub sha256: String,
    /// The program in the archive, a path relative to its root (`agy_acp_server.par`).
    pub cmd: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("the ACP registry has no agent `{0}`")]
    NoAgent(String),
    #[error("the ACP registry has no build of {0} for this platform ({1})")]
    NoBuild(String, String),
    #[error("the ACP registry's entry for {0} is not usable: {1}")]
    Bad(String, &'static str),
}

/// The registry's name for the platform PDT runs on (`darwin-aarch64`), or None for one it has no name for.
pub fn platform_key() -> Option<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        _ => return None,
    };
    Some(format!("{os}-{arch}"))
}

/// A version that is safe as a folder name: letters, digits, `.`, `-`, `_` and `+`, not starting with `.`.
pub fn valid_version(v: &str) -> bool {
    (1..=64).contains(&v.len())
        && !v.starts_with('.')
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}

/// A path inside an archive that stays inside the folder it unpacks to: relative, no `..`, no drive or root, no
/// backslashes, no NUL. `./` in front is dropped. Returns its parts.
pub fn safe_relative(path: &str) -> Option<Vec<&str>> {
    let path = path.strip_prefix("./").unwrap_or(path);
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('\0') || path.contains(':') {
        return None;
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    (!parts.is_empty() && parts.iter().all(|p| *p != "..")).then_some(parts)
}

/// Whether `url` is an https URL on the agent's own source.
fn from_source(id: &str, url: &str) -> bool {
    let Some((_, host, prefix)) = SOURCES.iter().find(|(i, ..)| *i == id) else { return false };
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let Some((h, path)) = rest.split_once('/') else { return false };
    h.eq_ignore_ascii_case(host)
        && format!("/{path}").starts_with(prefix)
        && !path.contains("..")
        && !path.contains(['?', '#', '\\'])
}

/// The build of agent `id` for `platform` in the registry `doc`, checked.
pub fn release(doc: &Value, id: &str, platform: &str) -> Result<RegistryRelease, RegistryError> {
    let agent = doc
        .get("agents")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|a| a.get("id").and_then(Value::as_str) == Some(id)))
        .ok_or_else(|| RegistryError::NoAgent(id.into()))?;
    let name = agent.get("name").and_then(Value::as_str).unwrap_or(id).to_string();
    let bad = |why| RegistryError::Bad(name.clone(), why);
    let version = agent.get("version").and_then(Value::as_str).unwrap_or_default();
    if !valid_version(version) {
        return Err(bad("its version"));
    }
    let build = agent
        .pointer("/distribution/binary")
        .and_then(|b| b.get(platform))
        .ok_or_else(|| RegistryError::NoBuild(name.clone(), platform.into()))?;
    let archive = build.get("archive").and_then(Value::as_str).unwrap_or_default();
    if !from_source(id, archive) {
        return Err(bad("its archive is not from the agent's own download site"));
    }
    let sha256 = build.get("sha256").and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad("its sha256"));
    }
    let cmd = build.get("cmd").and_then(Value::as_str).unwrap_or_default();
    let Some(cmd) = safe_relative(cmd).map(|p| p.join("/")) else { return Err(bad("its command")) };
    let args = match build.get("args") {
        None => Vec::new(),
        Some(a) => a
            .as_array()
            .and_then(|a| a.iter().map(|x| x.as_str().map(String::from)).collect::<Option<Vec<_>>>())
            .filter(|a| a.len() <= 16 && a.iter().all(|x| !x.contains('\0')))
            .ok_or_else(|| bad("its arguments"))?,
    };
    Ok(RegistryRelease { id: id.into(), version: version.into(), archive: archive.into(), sha256, cmd, args })
}

/// Compares two dotted versions by their numeric parts (`1.4.10` > `1.4.5`); parts that aren't numbers compare as
/// text.
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<(u64, String)> {
        v.split(['.', '-', '+']).map(|p| (p.parse::<u64>().unwrap_or(0), p.to_string())).collect()
    };
    parts(a) > parts(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = crate::acp_agent::ANTIGRAVITY_REGISTRY_ID;
    const SHA: &str = "886fff7767d08994ca39eb02c3d01a7532c7325a94aa33aaf52d52068733284f";

    fn doc(archive: &str, sha: &str, cmd: &str) -> Value {
        json!({"version": "1.0.0", "agents": [
            {"id": "other", "version": "1", "distribution": {}},
            {"id": ID, "name": "Google Antigravity", "version": "1.4.5", "distribution": {"binary": {
                "darwin-aarch64": {"archive": archive, "cmd": cmd, "sha256": sha},
                "linux-x86_64": {"archive": archive, "cmd": cmd, "args": ["--uid="], "sha256": sha}
            }}}
        ]})
    }

    const ZIP: &str = "https://dl.google.com/agy-extensions/releases/macos/agy-acp-server-1.4.5-darwin-arm64.zip";

    #[test]
    fn reads_a_platforms_build_from_the_registry() {
        let r = release(&doc(ZIP, &SHA.to_uppercase(), "./agy_acp_server.par"), ID, "darwin-aarch64").unwrap();
        assert_eq!(
            r,
            RegistryRelease {
                id: ID.into(),
                version: "1.4.5".into(),
                archive: ZIP.into(),
                sha256: SHA.into(),
                cmd: "agy_acp_server.par".into(),
                args: vec![],
            }
        );
        let linux = release(&doc(ZIP, SHA, "./agy_acp_server.par"), ID, "linux-x86_64").unwrap();
        assert_eq!(linux.args, vec!["--uid=".to_string()]);
        assert_eq!(
            release(&doc(ZIP, SHA, "x"), ID, "windows-aarch64").unwrap_err(),
            RegistryError::NoBuild("Google Antigravity".into(), "windows-aarch64".into())
        );
        assert_eq!(
            release(&doc(ZIP, SHA, "x"), "nope", "darwin-aarch64").unwrap_err(),
            RegistryError::NoAgent("nope".into())
        );
    }

    #[test]
    fn an_archive_comes_only_from_the_agents_own_site() {
        let fails = |url: &str| release(&doc(url, SHA, "./a"), ID, "darwin-aarch64").is_err();
        assert!(fails("http://dl.google.com/agy-extensions/releases/a.zip"), "https only");
        assert!(fails("https://evil.example/agy-extensions/releases/a.zip"));
        assert!(fails("https://dl.google.com.evil.example/agy-extensions/releases/a.zip"));
        assert!(fails("https://dl.google.com/other/a.zip"));
        assert!(fails("https://dl.google.com/agy-extensions/releases/../../x.zip"));
        assert!(fails("https://user@dl.google.com/agy-extensions/releases/a.zip"));
        assert!(!fails("https://DL.google.com/agy-extensions/releases/a.zip"));
        // Only agents PDT knows the source of.
        let mut d = doc(ZIP, SHA, "./a");
        d["agents"][1]["id"] = json!("someone-else");
        assert!(release(&d, "someone-else", "darwin-aarch64").is_err());
    }

    #[test]
    fn the_sha_command_and_version_are_checked() {
        assert!(release(&doc(ZIP, "abc", "./a"), ID, "darwin-aarch64").is_err());
        assert!(release(&doc(ZIP, SHA, "../a"), ID, "darwin-aarch64").is_err());
        assert!(release(&doc(ZIP, SHA, "/bin/sh"), ID, "darwin-aarch64").is_err());
        let mut d = doc(ZIP, SHA, "./a");
        d["agents"][1]["version"] = json!("../1");
        assert!(release(&d, ID, "darwin-aarch64").is_err());
    }

    #[test]
    fn archive_paths_stay_inside_their_folder() {
        assert_eq!(safe_relative("./a/b"), Some(vec!["a", "b"]));
        assert_eq!(safe_relative("a//./b/"), Some(vec!["a", "b"]));
        for bad in ["", "/etc/x", "../x", "a/../../x", "a\\b", "C:/x", "./", "a\0"] {
            assert_eq!(safe_relative(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("1.4.10", "1.4.5"));
        assert!(!newer("1.4.5", "1.4.5"));
        assert!(!newer("1.3.9", "1.4.0"));
        assert!(valid_version("1.4.5") && !valid_version(".x") && !valid_version("1/2"));
        assert!(platform_key().is_some());
    }
}
