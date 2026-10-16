use pdt_core::brand::{DAEMON_BIN, Profile};
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
    pub port: u16,
    pub dev: bool,
    /// How long a (new) pdtd gets to answer /api/health (spec §3.4: 60 s). A new version that exits within this
    /// time of being installed also counts as failed.
    pub health_timeout: Duration,
    pub backoff: Backoff,
    /// How often a running pdtd is checked.
    pub poll: Duration,
    /// How long pdtd gets to exit after handing over an update (and again after a shutdown request).
    pub stop_timeout: Duration,
    /// The `FORWARDED_ENV` values pdt-host was started with.
    pub forward_env: Vec<(String, String)>,
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
        let port = match lookup("PDT_PORT") {
            Some(p) => p.trim().parse().map_err(|_| format!("PDT_PORT is not a valid port: {p:?}"))?,
            None => profile.port(),
        };
        let data_dir = match lookup("PDT_DATA_DIR") {
            Some(d) => PathBuf::from(d),
            None => default_data_root
                .ok_or("could not determine a data directory; set PDT_DATA_DIR")?
                .join(profile.data_dir_name()),
        };
        let forward_env = FORWARDED_ENV.iter().filter_map(|k| lookup(k).map(|v| (k.to_string(), v))).collect();
        Ok(HostConfig {
            install_dir,
            data_dir,
            port,
            dev: profile == Profile::Dev,
            health_timeout: Duration::from_secs(60),
            backoff: Backoff::default(),
            poll: Duration::from_secs(2),
            stop_timeout: Duration::from_secs(30),
            forward_env,
        })
    }

    pub fn from_env(install_dir: PathBuf) -> Result<HostConfig, String> {
        Self::from_lookup(|k| std::env::var(k).ok(), dirs::data_local_dir(), install_dir)
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
        env.extend(self.forward_env.iter().filter(|(k, _)| FORWARDED_ENV.contains(&k.as_str())).cloned());
        env
    }
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
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(got, want);
        let stable = HostConfig::from_lookup(lookup(&[]), Some("/d".into()), "/app".into()).unwrap();
        assert!(stable.pdtd_env().contains(&("PDT_PROFILE".into(), "stable".into())));
    }
}
