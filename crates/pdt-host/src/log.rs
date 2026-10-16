use crate::config::HostConfig;
use std::io::Write;
use std::path::PathBuf;

pub fn log_path(cfg: &HostConfig) -> PathBuf {
    cfg.data_dir.join("logs").join(format!("{}.log", pdt_core::brand::HOST_BIN))
}

/// Appends a timestamped line to `<data>/logs/pdt-host.log` and stderr. Never fails.
pub fn log(cfg: &HostConfig, msg: &str) {
    let line = format!("{} {msg}", jiff::Timestamp::now());
    eprintln!("{line}");
    let path = log_path(cfg);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::File::options().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}
