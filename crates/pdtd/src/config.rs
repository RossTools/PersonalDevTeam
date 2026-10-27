use anyhow::{Context, bail};
use pdt_api::remote::HOST_HTTPS_PORT;
use pdt_core::brand::{DAEMON_BIN, Profile};
use std::path::PathBuf;
use tokio::net::TcpListener;

const VITE_DEV_PORT: u16 = 5173;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub profile: Profile,
    pub port: u16,
    pub data_dir: PathBuf,
    pub web_dir: Option<PathBuf>,
    /// Host header values accepted by the API (DNS-rebinding guard).
    pub allowed_hosts: Vec<String>,
    /// `PDT_OWNER_LOGIN`: the Tailscale login allowed in (dev instances get it from their run configuration).
    pub owner_login: Option<String>,
    /// `PDT_SECRET_STORE=memory`: keep secrets in memory (tests); otherwise the OS keychain.
    pub memory_secrets: bool,
    /// `PDT_TAILSCALE_BIN`: the tailscale CLI to use instead of the detected one (tests point it at a fake or at
    /// nothing, so they never reach the machine's Tailscale).
    pub tailscale_bin: Option<PathBuf>,
    /// `PDT_GITHUB_OFFLINE=1`: the GitHub forge never reaches the network (tests set it, so they never reach GitHub).
    pub github_offline: bool,
    /// `PDT_GITHUB_CLIENT_ID`: the OAuth App Connect GitHub signs in with, instead of the built-in one.
    pub github_client_id: Option<String>,
    /// The HTTPS port remote access claims: 443 only on the stable instance's port, else the instance's own port,
    /// so a dev instance (or any explicit `PDT_PORT`) never takes over the stable one's mapping.
    pub remote_https_port: u16,
    /// `PDT_CLAUDE_PROGRAM` / `PDT_CODEX_PROGRAM` / `PDT_DEVIN_PROGRAM`: the agent programs to launch instead of the detected ones (tests,
    /// unusual installs).
    pub claude_program: Option<String>,
    pub codex_program: Option<String>,
    pub devin_program: Option<String>,
    /// `PDT_SUPERVISED=1`: pdt-host started this pdtd. Only a supervised stable instance offers and hands off updates
    /// (`update_disabled_reason`).
    pub supervised: bool,
    /// `PDT_UPDATES_UNSUPPORTED`: why the pdt-host that started this pdtd cannot install updates (e.g. the app runs
    /// from a read-only folder, PDT-51); updates are then off with that reason.
    pub updates_unsupported: Option<String>,
    /// `PDT_SEED=demo`: fill an empty database with demo data at startup (dev instances only; `seed.rs`).
    pub seed: Option<Seed>,
}

/// What `PDT_SEED` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seed {
    Demo,
}

/// Why a dev instance never updates.
pub const DEV_UPDATE_REASON: &str = "updates apply to the installed stable app";

impl Config {
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        default_data_root: Option<PathBuf>,
        exe_dir: Option<PathBuf>,
    ) -> anyhow::Result<Config> {
        // Empty or whitespace-only values count as unset.
        let lookup = |k: &str| lookup(k).filter(|v| !v.trim().is_empty());
        let profile = Profile::from_env_value(lookup("PDT_PROFILE").as_deref());
        let port = match lookup("PDT_PORT") {
            Some(p) => p.parse::<u16>().with_context(|| format!("PDT_PORT is not a valid port: {p:?}"))?,
            None => profile.port(),
        };
        let data_dir = match lookup("PDT_DATA_DIR") {
            Some(d) => PathBuf::from(d),
            None => match default_data_root {
                Some(root) => root.join(profile.data_dir_name()),
                None => bail!("could not determine a data directory; set PDT_DATA_DIR"),
            },
        };
        // Next to the exe (Windows installer, dev layouts), or in a macOS bundle's Contents/Resources.
        let bundled_web =
            |d: PathBuf| [d.join("web"), d.join("..").join("Resources").join("web")].into_iter().find(|w| w.is_dir());
        let web_dir = lookup("PDT_WEB_DIR").map(PathBuf::from).or_else(|| exe_dir.and_then(bundled_web));
        let mut allowed_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")];
        if profile == Profile::Dev {
            // `pnpm --dir web dev` serves the UI on 5173 and proxies /api here with its own Host and Origin.
            allowed_hosts.extend([format!("localhost:{VITE_DEV_PORT}"), format!("127.0.0.1:{VITE_DEV_PORT}")]);
        }
        if let Some(extra) = lookup("PDT_ALLOWED_HOSTS") {
            allowed_hosts.extend(extra.split(',').map(str::trim).filter(|h| !h.is_empty()).map(String::from));
        }
        let owner_login = lookup("PDT_OWNER_LOGIN").map(|v| v.trim().to_string());
        let memory_secrets = lookup("PDT_SECRET_STORE").as_deref() == Some("memory");
        let tailscale_bin = lookup("PDT_TAILSCALE_BIN").map(PathBuf::from);
        let github_offline = lookup("PDT_GITHUB_OFFLINE").as_deref() == Some("1");
        let github_client_id = lookup("PDT_GITHUB_CLIENT_ID").filter(|c| !c.trim().is_empty());
        let remote_https_port = if port == Profile::Stable.port() { HOST_HTTPS_PORT } else { port };
        let claude_program = lookup("PDT_CLAUDE_PROGRAM");
        let codex_program = lookup("PDT_CODEX_PROGRAM");
        let devin_program = lookup("PDT_DEVIN_PROGRAM");
        let supervised = lookup("PDT_SUPERVISED").as_deref() == Some("1");
        let updates_unsupported = lookup("PDT_UPDATES_UNSUPPORTED").map(|v| v.trim().to_string());
        let seed = match lookup("PDT_SEED").as_deref().map(str::trim) {
            None => None,
            Some("demo") => Some(Seed::Demo),
            Some(other) => bail!("PDT_SEED must be \"demo\", not {other:?}"),
        };
        Ok(Config {
            profile,
            port,
            data_dir,
            web_dir,
            allowed_hosts,
            owner_login,
            memory_secrets,
            tailscale_bin,
            github_offline,
            github_client_id,
            remote_https_port,
            claude_program,
            codex_program,
            devin_program,
            supervised,
            updates_unsupported,
            seed,
        })
    }

    /// Why this pdtd never updates, or `None` when it may: only a stable instance under pdt-host updates. A dev
    /// instance never does, even when supervised and holding a token (final review #1).
    pub fn update_disabled_reason(&self) -> Option<String> {
        if self.profile == Profile::Dev {
            Some(DEV_UPDATE_REASON.into())
        } else if !self.supervised {
            Some(pdt_api::update::UNSUPERVISED_REASON.into())
        } else {
            self.updates_unsupported.as_ref().map(|why| format!("pdt-host cannot install updates here: {why}"))
        }
    }

    pub fn from_env() -> anyhow::Result<Config> {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from));
        Self::from_lookup(|k| std::env::var(k).ok(), dirs::data_local_dir(), exe_dir)
    }
}

pub async fn bind(port: u16) -> anyhow::Result<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => Ok(l),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            bail!("port {port} is already in use — is another {DAEMON_BIN} running? (set PDT_PORT to override)")
        }
        Err(e) => Err(e).with_context(|| format!("failed to bind 127.0.0.1:{port}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn stable_defaults() {
        let c = Config::from_lookup(lookup(&[]), Some(PathBuf::from("/data")), None).unwrap();
        assert_eq!(c.profile, Profile::Stable);
        assert_eq!(c.port, 7420);
        assert_eq!(c.data_dir, PathBuf::from("/data").join("pdt"));
        assert_eq!(c.web_dir, None);
    }

    #[test]
    fn dev_profile_and_overrides() {
        let c = Config::from_lookup(lookup(&[("PDT_PROFILE", "dev")]), Some(PathBuf::from("/data")), None).unwrap();
        assert_eq!((c.port, c.data_dir.clone()), (7421, PathBuf::from("/data").join("pdt-dev")));

        let c = Config::from_lookup(
            lookup(&[("PDT_PORT", "9999"), ("PDT_DATA_DIR", "/x"), ("PDT_WEB_DIR", "/w")]),
            None,
            None,
        )
        .unwrap();
        assert_eq!((c.port, c.data_dir, c.web_dir), (9999, PathBuf::from("/x"), Some(PathBuf::from("/w"))));
    }

    #[test]
    fn empty_env_values_are_unset() {
        let c = Config::from_lookup(
            lookup(&[
                ("PDT_DATA_DIR", ""),
                ("PDT_PORT", " "),
                ("PDT_PROFILE", "  "),
                ("PDT_WEB_DIR", ""),
                ("PDT_ALLOWED_HOSTS", " "),
            ]),
            Some(PathBuf::from("/data")),
            None,
        )
        .unwrap();
        assert_eq!((c.port, c.data_dir, c.web_dir), (7420, PathBuf::from("/data").join("pdt"), None));
        assert_eq!(c.allowed_hosts.len(), 3);
    }

    #[test]
    fn allowed_hosts_default_and_extra() {
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), None).unwrap();
        assert_eq!(c.allowed_hosts, ["127.0.0.1:7420", "localhost:7420", "[::1]:7420"]);
        let c = Config::from_lookup(
            lookup(&[("PDT_PORT", "9000"), ("PDT_ALLOWED_HOSTS", " box.tail.ts.net:9000 ,, other:1 ,")]),
            Some("/d".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            c.allowed_hosts,
            ["127.0.0.1:9000", "localhost:9000", "[::1]:9000", "box.tail.ts.net:9000", "other:1"]
        );
    }

    #[test]
    fn dev_profile_allows_the_vite_dev_server() {
        let c = Config::from_lookup(lookup(&[("PDT_PROFILE", "dev")]), Some("/d".into()), None).unwrap();
        assert!(c.allowed_hosts.contains(&"localhost:5173".to_string()));
        assert!(c.allowed_hosts.contains(&"127.0.0.1:5173".to_string()));
        let stable = Config::from_lookup(lookup(&[]), Some("/d".into()), None).unwrap();
        assert!(!stable.allowed_hosts.iter().any(|h| h.ends_with(":5173")));
    }

    #[test]
    fn owner_and_secret_store_from_env() {
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), None).unwrap();
        assert_eq!((c.owner_login, c.memory_secrets), (None, false));
        let c = Config::from_lookup(
            lookup(&[("PDT_OWNER_LOGIN", " me@example.com "), ("PDT_SECRET_STORE", "memory")]),
            Some("/d".into()),
            None,
        )
        .unwrap();
        assert_eq!((c.owner_login.as_deref(), c.memory_secrets), (Some("me@example.com"), true));
    }

    #[test]
    fn supervised_only_when_pdt_host_says_so() {
        let sup = |v: &[(&str, &str)]| Config::from_lookup(lookup(v), Some("/d".into()), None).unwrap().supervised;
        assert!(!sup(&[]));
        assert!(!sup(&[("PDT_SUPERVISED", "0")]));
        assert!(!sup(&[("PDT_SUPERVISED", "yes")]));
        assert!(sup(&[("PDT_SUPERVISED", "1")]));
    }

    #[test]
    fn only_a_supervised_stable_instance_updates() {
        let reason = |v: &[(&str, &str)]| {
            Config::from_lookup(lookup(v), Some("/d".into()), None).unwrap().update_disabled_reason()
        };
        assert_eq!(reason(&[("PDT_SUPERVISED", "1")]), None);
        assert_eq!(reason(&[]).as_deref(), Some(pdt_api::update::UNSUPERVISED_REASON));
        let dev = Some(DEV_UPDATE_REASON);
        assert_eq!(reason(&[("PDT_PROFILE", "dev"), ("PDT_SUPERVISED", "1")]).as_deref(), dev);
        assert_eq!(reason(&[("PDT_PROFILE", "dev")]).as_deref(), dev);
        assert_eq!(
            reason(&[("PDT_SUPERVISED", "1"), ("PDT_UPDATES_UNSUPPORTED", "its folder cannot be written to")])
                .as_deref(),
            Some("pdt-host cannot install updates here: its folder cannot be written to")
        );
        assert_eq!(reason(&[("PDT_SUPERVISED", "1"), ("PDT_UPDATES_UNSUPPORTED", " ")]), None);
    }

    #[test]
    fn tailscale_bin_from_env() {
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), None).unwrap();
        assert_eq!(c.tailscale_bin, None);
        let c = Config::from_lookup(lookup(&[("PDT_TAILSCALE_BIN", "/x/fake-tailscale")]), Some("/d".into()), None)
            .unwrap();
        assert_eq!(c.tailscale_bin, Some(PathBuf::from("/x/fake-tailscale")));
    }

    #[test]
    fn agent_programs_from_env() {
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), None).unwrap();
        assert_eq!((c.claude_program, c.codex_program, c.devin_program), (None, None, None));
        let c = Config::from_lookup(
            lookup(&[
                ("PDT_CLAUDE_PROGRAM", "/x/claude"),
                ("PDT_CODEX_PROGRAM", "/x/codex"),
                ("PDT_DEVIN_PROGRAM", "/x/devin"),
            ]),
            Some("/d".into()),
            None,
        )
        .unwrap();
        assert_eq!((c.claude_program.as_deref(), c.codex_program.as_deref()), (Some("/x/claude"), Some("/x/codex")));
        assert_eq!(c.devin_program.as_deref(), Some("/x/devin"));
    }

    #[test]
    fn only_the_stable_port_claims_https_443() {
        let port_of = |pairs: &[(&str, &str)]| {
            Config::from_lookup(lookup(pairs), Some("/d".into()), None).unwrap().remote_https_port
        };
        assert_eq!(port_of(&[]), 443);
        assert_eq!(port_of(&[("PDT_PROFILE", "dev")]), 7421);
        assert_eq!(port_of(&[("PDT_PORT", "9000")]), 9000);
        assert_eq!(port_of(&[("PDT_PROFILE", "dev"), ("PDT_PORT", "9001")]), 9001);
    }

    #[test]
    fn seed_from_env() {
        let seed = |v: &[(&str, &str)]| Config::from_lookup(lookup(v), Some("/d".into()), None).map(|c| c.seed);
        assert_eq!(seed(&[]).unwrap(), None);
        assert_eq!(seed(&[("PDT_SEED", "")]).unwrap(), None);
        assert_eq!(seed(&[("PDT_SEED", " demo ")]).unwrap(), Some(Seed::Demo));
        assert!(seed(&[("PDT_SEED", "lots")]).is_err());
    }

    #[test]
    fn bad_port_is_error() {
        assert!(Config::from_lookup(lookup(&[("PDT_PORT", "abc")]), Some("/d".into()), None).is_err());
    }

    #[test]
    fn web_dir_next_to_exe_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("web")).unwrap();
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(c.web_dir, Some(tmp.path().join("web")));
    }

    #[test]
    fn web_dir_inside_a_macos_app_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let macos = tmp.path().join("Contents").join("MacOS");
        let resources_web = tmp.path().join("Contents").join("Resources").join("web");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::create_dir_all(&resources_web).unwrap();
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), Some(macos)).unwrap();
        assert_eq!(
            c.web_dir.map(|d| std::fs::canonicalize(d).unwrap()),
            Some(std::fs::canonicalize(resources_web).unwrap())
        );
    }

    #[tokio::test]
    async fn port_in_use_is_reported() {
        let held = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = held.local_addr().unwrap().port();
        let err = bind(port).await.unwrap_err().to_string();
        assert!(err.contains("already in use"), "{err}");
    }
}
