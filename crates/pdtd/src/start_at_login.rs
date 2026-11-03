//! Settings → This host → Start PDT at login (PDT-489), through pdt-host's own registration code.

use pdt_core::brand::Profile;
use pdt_host::config::HostConfig;
use pdt_host::register::{Dirs, Os, System, Target, bundled_host_path};
use std::path::PathBuf;

pub struct HostLogin {
    cfg: Option<HostConfig>,
    why_not: Option<String>,
    home: Option<PathBuf>,
    config: Option<PathBuf>,
}

impl HostLogin {
    /// pdt-host's login item for this pdtd: pdt-host runs from the same install folder and data folder.
    pub fn new(profile: Profile, supervised: bool, data_dir: PathBuf) -> HostLogin {
        let install_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from));
        let cfg = install_dir.and_then(|dir| HostConfig::from_env(dir).ok()).map(|mut c| {
            c.data_dir = data_dir;
            c.dev = profile == Profile::Dev;
            c
        });
        let why_not = match &cfg {
            None => Some("PDT can't tell where it is installed".into()),
            Some(_) if !supervised => Some("PDT wasn't started by its host supervisor".into()),
            Some(c) => pdt_host::login::unsupported(c, Os::current()),
        };
        HostLogin { cfg, why_not, home: std::env::home_dir(), config: dirs::config_dir() }
    }
}

impl pdt_api::host_service::StartAtLogin for HostLogin {
    fn unavailable(&self) -> Option<String> {
        self.why_not.clone()
    }

    fn registered(&self) -> bool {
        let dirs = Dirs { home: self.home.as_deref(), config: self.config.as_deref() };
        pdt_host::login::registered(Os::current(), dirs, &mut System)
    }

    fn set(&self, on: bool) -> Result<(), String> {
        let cfg = self.cfg.as_ref().ok_or("PDT can't tell where it is installed")?;
        let exe = bundled_host_path(&cfg.install_dir);
        let target = Target {
            os: Os::current(),
            exe: &exe,
            home: self.home.as_deref(),
            config: self.config.as_deref(),
            replace: false,
        };
        pdt_host::login::set(cfg, &target, on, &mut System)
    }
}
