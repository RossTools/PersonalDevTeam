use pdt_core::brand::{DAEMON_BIN, Profile};
use pdt_core::network::{NetworkSettings, PORT_FROM_SETTING_ENV, network_path};
use std::path::PathBuf;
use std::time::Duration;

use crate::supervise::Backoff;

/// The only `PDT_*` variables pdt-host passes on from its own environment to pdtd (ruling A). Profile, port and data
/// dir are always set explicitly, so a dev pdt-host can never start a pdtd on the stable port or data dir.
pub const FORWARDED_ENV: [&str; 5] =
    ["PDT_WEB_DIR", "PDT_ALLOWED_HOSTS", "PDT_OWNER_LOGIN", "PDT_SECRET_STORE", "PDT_TAILSCALE_BIN"];

#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Where the installed app lives: pdtd (and the bundled pdt-host) are here.
    pub install_dir: PathBuf,
    pub data_dir: PathBuf,
    /// The port pdtd listens on (see `refresh_port`).
    pub port: u16,
    /// No `PDT_PORT`: the port follows Settings → Network (`network.json`), read again whenever pdtd is (re)started
    /// (PDT-231).
    pub port_setting: bool,
    pub dev: bool,
    /// How long a (new) pdtd gets to answer /api/health (60 s).
    pub health_timeout: Duration,
    /// How long a new version must keep running once it has answered its health check: one that crashes within
    /// this time counts as failed and is rolled back.
    pub watch: Duration,
    pub backoff: Backoff,
    /// How often a running pdtd is checked.
    pub poll: Duration,
    /// How long pdtd gets to exit after handing over an update (and again after a shutdown request).
    pub stop_timeout: Duration,
    /// The `FORWARDED_ENV` values pdt-host was started with.
    pub forward_env: Vec<(String, String)>,
    /// Why this host cannot install updates (set by `install::host_installer`); pdtd gets it as
    /// `PDT_UPDATES_UNSUPPORTED` and then never offers one.
    pub updates_unsupported: Option<String>,
}

impl HostConfig {
    /// The same rules as pdtd's config (`PDT_PROFILE`, `PDT_PORT`, `PDT_DATA_DIR`), so both agree on the port and data dir.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        default_data_root: Option<PathBuf>,
        install_dir: PathBuf,
    ) -> Result<HostConfig, String> {
        let lookup = |k: &str| lookup(k).filter(|v| !v.trim().is_empty());
        let profile = Profile::from_env_value(lookup("PDT_PROFILE").as_deref());
        let env_port: Option<u16> = match lookup("PDT_PORT") {
            Some(p) => Some(p.trim().parse().map_err(|_| format!("PDT_PORT is not a valid port: {p:?}"))?),
            None => None,
        };
        let data_dir = match lookup("PDT_DATA_DIR") {
            Some(d) => PathBuf::from(d),
            None => default_data_root
                .ok_or("could not determine a data directory; set PDT_DATA_DIR")?
                .join(profile.data_dir_name()),
        };
        let port = env_port.unwrap_or_else(|| setting_port(&data_dir, profile));
        let forward_env = FORWARDED_ENV.iter().filter_map(|k| lookup(k).map(|v| (k.to_string(), v))).collect();
        Ok(HostConfig {
            install_dir,
            data_dir,
            port,
            port_setting: env_port.is_none(),
            dev: profile == Profile::Dev,
            health_timeout: Duration::from_secs(60),
            watch: Duration::from_secs(60),
            backoff: Backoff::default(),
            poll: Duration::from_secs(2),
            stop_timeout: Duration::from_secs(30),
            forward_env,
            updates_unsupported: None,
        })
    }

    pub fn from_env(install_dir: PathBuf) -> Result<HostConfig, String> {
        Self::from_lookup(|k| std::env::var(k).ok(), dirs::data_local_dir(), install_dir)
    }

    /// Reads Settings → Network again (when the port follows it), for the next pdtd: the port a pdtd of this data
    /// dir still listens on while it answers (one started before the setting changed), else the setting's.
    pub fn refresh_port(&mut self, answers: impl Fn(u16) -> bool) {
        if !self.port_setting {
            return;
        }
        let profile = if self.dev { Profile::Dev } else { Profile::Stable };
        self.port = current_port(&self.data_dir, profile, answers);
    }

    pub fn pdtd_path(&self) -> PathBuf {
        self.install_dir.join(format!("{DAEMON_BIN}{}", std::env::consts::EXE_SUFFIX))
    }

    /// Every `PDT_*` variable pdtd is started with (all inherited ones are removed first).
    pub fn pdtd_env(&self) -> Vec<(String, String)> {
        let mut env = vec![
            ("PDT_PROFILE".to_string(), if self.dev { "dev" } else { "stable" }.to_string()),
            ("PDT_PORT".to_string(), self.port.to_string()),
            ("PDT_DATA_DIR".to_string(), self.data_dir.to_string_lossy().into_owned()),
            ("PDT_SUPERVISED".to_string(), "1".to_string()),
        ];
        if self.port_setting {
            env.push((PORT_FROM_SETTING_ENV.to_string(), "1".to_string()));
        }
        env.extend(self.forward_env.iter().filter(|(k, _)| FORWARDED_ENV.contains(&k.as_str())).cloned());
        if let Some(reason) = &self.updates_unsupported {
            env.push(("PDT_UPDATES_UNSUPPORTED".to_string(), reason.clone()));
        }
        env
    }
}

/// The port a pdtd of this data dir said it listens on (it may have exited since).
pub fn read_listening(data_dir: &std::path::Path) -> Option<u16> {
    pdt_core::network::parse_listening(&std::fs::read_to_string(pdt_core::network::listening_path(data_dir)).ok()?)
}

/// The port of the data dir's pdtd when `PDT_PORT` doesn't say: the one a running pdtd listens on while it answers
/// (started before Settings → Network changed), else the setting's, else the profile's. The tray finds pdtd by it.
pub fn current_port(data_dir: &std::path::Path, profile: Profile, answers: impl Fn(u16) -> bool) -> u16 {
    let wanted = setting_port(data_dir, profile);
    match read_listening(data_dir) {
        Some(live) if live != wanted && answers(live) => live,
        _ => wanted,
    }
}

/// The port Settings → Network gives the data dir's pdtd, or the profile's.
fn setting_port(data_dir: &std::path::Path, profile: Profile) -> u16 {
    let settings = std::fs::read(network_path(data_dir)).map(|b| NetworkSettings::parse(&b)).unwrap_or_default();
    settings.port_for(None, profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn matches_pdtd_defaults() {
        let c = HostConfig::from_lookup(lookup(&[]), Some("/d".into()), "/app".into()).unwrap();
        assert_eq!((c.port, c.data_dir.clone(), c.dev), (7420, PathBuf::from("/d").join("pdt"), false));
        let c = HostConfig::from_lookup(lookup(&[("PDT_PROFILE", "dev")]), Some("/d".into()), "/app".into()).unwrap();
        assert_eq!((c.port, c.dev), (7421, true));
        let c = HostConfig::from_lookup(lookup(&[("PDT_PORT", "9000"), ("PDT_DATA_DIR", "/x")]), None, "/app".into())
            .unwrap();
        assert_eq!((c.port, c.data_dir.clone()), (9000, PathBuf::from("/x")));
        assert!(HostConfig::from_lookup(lookup(&[("PDT_PORT", "x")]), Some("/d".into()), "/app".into()).is_err());
        assert!(c.pdtd_path().starts_with("/app"));
    }

    #[test]
    fn without_pdt_port_the_port_follows_the_network_setting() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("pdt");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(network_path(&data), r#"{"port": 7500}"#).unwrap();
        let mut c = HostConfig::from_lookup(lookup(&[]), Some(tmp.path().into()), "/app".into()).unwrap();
        assert_eq!((c.port, c.port_setting), (7500, true));
        assert!(c.pdtd_env().contains(&(PORT_FROM_SETTING_ENV.into(), "1".into())));
        std::fs::write(network_path(&data), r#"{"port": 7600}"#).unwrap();
        // A pdtd still answering on the port it started on is adopted until it exits.
        std::fs::write(pdt_core::network::listening_path(&data), "7500\n").unwrap();
        c.refresh_port(|p| p == 7500);
        assert_eq!(c.port, 7500);
        c.refresh_port(|_| false);
        assert_eq!(c.port, 7600);
        // PDT_PORT wins, and pdtd is told it is the user's.
        let mut e =
            HostConfig::from_lookup(lookup(&[("PDT_PORT", "9000")]), Some(tmp.path().into()), "/app".into()).unwrap();
        e.refresh_port(|_| false);
        assert_eq!((e.port, e.port_setting), (9000, false));
        assert!(!e.pdtd_env().iter().any(|(k, _)| k == PORT_FROM_SETTING_ENV));
    }

    #[test]
    fn pdtd_gets_an_explicit_profile_port_and_data_dir_and_only_the_forwarded_extras() {
        let env = lookup(&[
            ("PDT_PROFILE", "dev"),
            ("PDT_WEB_DIR", "/web"),
            ("PDT_ALLOWED_HOSTS", "h:1"),
            ("PDT_SUPERVISED", "0"),
            ("PDT_CLAUDE_PROGRAM", "x"),
        ]);
        let c = HostConfig::from_lookup(env, Some("/d".into()), "/app".into()).unwrap();
        let got: HashMap<String, String> = c.pdtd_env().into_iter().collect();
        let dev_dir = PathBuf::from("/d").join("pdt-dev").to_string_lossy().into_owned();
        let want: HashMap<String, String> = [
            ("PDT_PROFILE", "dev"),
            ("PDT_PORT", "7421"),
            ("PDT_DATA_DIR", dev_dir.as_str()),
            ("PDT_SUPERVISED", "1"),
            ("PDT_WEB_DIR", "/web"),
            ("PDT_ALLOWED_HOSTS", "h:1"),
            (PORT_FROM_SETTING_ENV, "1"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(got, want);
        let mut stable =
            HostConfig::from_lookup(lookup(&[("PDT_UPDATES_UNSUPPORTED", "x")]), Some("/d".into()), "/app".into())
                .unwrap();
        assert!(stable.pdtd_env().contains(&("PDT_PROFILE".into(), "stable".into())));
        assert!(!stable.pdtd_env().iter().any(|(k, _)| k == "PDT_UPDATES_UNSUPPORTED"), "never inherited");
        stable.updates_unsupported = Some("it is on a disk image".into());
        assert!(stable.pdtd_env().contains(&("PDT_UPDATES_UNSUPPORTED".into(), "it is on a disk image".into())));
    }
}
