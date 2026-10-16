use pdt_core::brand::{DAEMON_BIN, Profile};
use std::path::PathBuf;
use std::time::Duration;

use crate::supervise::Backoff;

#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Where the installed app lives: pdtd (and the bundled pdt-host) are here.
    pub install_dir: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub dev: bool,
    /// How long a (new) pdtd gets to answer /api/health (spec §3.4: 60 s).
    pub health_timeout: Duration,
    pub backoff: Backoff,
    /// How often a running pdtd is checked.
    pub poll: Duration,
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
        Ok(HostConfig {
            install_dir,
            data_dir,
            port,
            dev: profile == Profile::Dev,
            health_timeout: Duration::from_secs(60),
            backoff: Backoff::default(),
            poll: Duration::from_secs(2),
        })
    }

    pub fn from_env(install_dir: PathBuf) -> Result<HostConfig, String> {
        Self::from_lookup(|k| std::env::var(k).ok(), dirs::data_local_dir(), install_dir)
    }

    pub fn pdtd_path(&self) -> PathBuf {
        self.install_dir.join(format!("{DAEMON_BIN}{}", std::env::consts::EXE_SUFFIX))
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
}
