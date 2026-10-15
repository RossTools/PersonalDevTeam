use anyhow::{Context, bail};
use pdt_core::brand::{DAEMON_BIN, Profile};
use std::path::PathBuf;
use tokio::net::TcpListener;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub profile: Profile,
    pub port: u16,
    pub data_dir: PathBuf,
    pub web_dir: Option<PathBuf>,
    /// Host header values accepted by the API (DNS-rebinding guard).
    pub allowed_hosts: Vec<String>,
}

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
        let web_dir =
            lookup("PDT_WEB_DIR").map(PathBuf::from).or_else(|| exe_dir.map(|d| d.join("web")).filter(|d| d.is_dir()));
        let mut allowed_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        if let Some(extra) = lookup("PDT_ALLOWED_HOSTS") {
            allowed_hosts.extend(extra.split(',').map(str::trim).filter(|h| !h.is_empty()).map(String::from));
        }
        Ok(Config { profile, port, data_dir, web_dir, allowed_hosts })
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
        assert_eq!(c.allowed_hosts.len(), 2);
    }

    #[test]
    fn allowed_hosts_default_and_extra() {
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), None).unwrap();
        assert_eq!(c.allowed_hosts, ["127.0.0.1:7420", "localhost:7420"]);
        let c = Config::from_lookup(
            lookup(&[("PDT_PORT", "9000"), ("PDT_ALLOWED_HOSTS", " box.tail.ts.net:9000 ,, other:1 ,")]),
            Some("/d".into()),
            None,
        )
        .unwrap();
        assert_eq!(c.allowed_hosts, ["127.0.0.1:9000", "localhost:9000", "box.tail.ts.net:9000", "other:1"]);
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

    #[tokio::test]
    async fn port_in_use_is_reported() {
        let held = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = held.local_addr().unwrap().port();
        let err = bind(port).await.unwrap_err().to_string();
        assert!(err.contains("already in use"), "{err}");
    }
}
