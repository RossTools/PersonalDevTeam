use crate::config::HostConfig;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn log_path(cfg: &HostConfig) -> PathBuf {
    cfg.data_dir.join("logs").join(format!("{}.log", pdt_core::brand::HOST_BIN))
}

/// How big `pdt-host.log` and `pdtd-launch.log` may grow before [`roll`] moves them aside.
pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Moves `path` to `<path>.1` (replacing the one before) once it is over `max` bytes, so a log that is only ever
/// appended to keeps at most two files' worth. Never fails.
pub fn roll(path: &Path, max: u64) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max) {
        let mut old = path.as_os_str().to_owned();
        old.push(".1");
        let _ = std::fs::rename(path, old);
    }
}

/// Appends a timestamped line to `<data>/logs/pdt-host.log` and stderr. Never fails.
pub fn log(cfg: &HostConfig, msg: &str) {
    let line = format!("{} {msg}", jiff::Timestamp::now());
    eprintln!("{line}");
    let path = log_path(cfg);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    roll(&path, MAX_LOG_BYTES);
    if let Ok(mut f) = std::fs::File::options().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_log_over_the_limit_moves_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("x.log");
        let old = tmp.path().join("x.log.1");
        super::roll(&log, 10);
        assert!(!log.exists() && !old.exists(), "no log: nothing to do");
        std::fs::write(&log, "0123456789").unwrap();
        super::roll(&log, 10);
        assert!(log.exists(), "at the limit: kept");
        std::fs::write(&old, "older").unwrap();
        std::fs::write(&log, "0123456789a").unwrap();
        super::roll(&log, 10);
        assert!(!log.exists());
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "0123456789a", "replaces the one before");
    }
}
