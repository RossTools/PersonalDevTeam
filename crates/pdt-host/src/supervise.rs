//! The supervision loop (spec §3.1, §3.4): start pdtd (or adopt one that is running), restart it with backoff when it
//! exits, and install the update it hands over, rolling back when the new version fails its health check.

use crate::config::HostConfig;
use crate::health::{Probe, probe};
use crate::install::Installer;
use crate::log::{log, log_path};
use pdt_core::handoff::{UpdateOutcome, UpdateRequest, UpdateResult, request_path, result_path};
use std::fs::{File, TryLockError};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
    /// A pdtd that ran this long resets the failure count.
    pub reset_after: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { initial: Duration::from_secs(1), max: Duration::from_secs(60), reset_after: Duration::from_secs(300) }
    }
}

impl Backoff {
    /// 0 after no failures; then initial, 2×, 4×… capped at `max`.
    pub fn delay(&self, failures: u32) -> Duration {
        if failures == 0 {
            return Duration::ZERO;
        }
        self.initial.saturating_mul(1u32 << (failures - 1).min(16)).min(self.max)
    }
}

/// Takes the pending update request, renaming the file so a crash loop never installs it twice.
pub fn take_request(data_dir: &Path) -> Option<UpdateRequest> {
    let path = request_path(data_dir);
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::rename(&path, path.with_extension("taken.json"));
    serde_json::from_slice(&bytes).ok()
}

/// Locks `path` (created if missing), retrying for `within` while another process holds it (ruling F10: a relaunched
/// pdt-host waits for the exiting copy). `None` when it is still held.
pub fn acquire_lock(path: &Path, within: Duration) -> std::io::Result<Option<File>> {
    let f = File::options().create(true).truncate(false).write(true).open(path)?;
    let end = Instant::now() + within;
    loop {
        match f.try_lock() {
            Ok(()) => return Ok(Some(f)),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(e)) => return Err(e),
        }
        let now = Instant::now();
        if now >= end {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100).min(end - now));
    }
}

pub fn spawn_pdtd(cfg: &HostConfig) -> std::io::Result<Child> {
    let launch_log = cfg.data_dir.join("logs").join(format!("{}-launch.log", pdt_core::brand::DAEMON_BIN));
    let _ = std::fs::create_dir_all(launch_log.parent().expect("logs dir"));
    let out = File::options().create(true).append(true).open(&launch_log)?;
    let mut cmd = Command::new(cfg.pdtd_path());
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("PDT_PORT", cfg.port.to_string())
        .env("PDT_DATA_DIR", &cfg.data_dir)
        .env("PDT_SUPERVISED", "1")
        .current_dir(&cfg.install_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out.try_clone()?))
        .stderr(Stdio::from(out));
    if cfg.dev {
        cmd.env("PDT_PROFILE", "dev");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000 | 0x0000_0200); // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
    }
    #[cfg(unix)]
    {
        // Own process group: pdtd survives pdt-host restarting itself (launchd kills the job's process group).
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()
}

fn sleep_unless_stopped(d: Duration, stop: &AtomicBool) {
    let end = Instant::now() + d;
    while !stop.load(Ordering::SeqCst) {
        let now = Instant::now();
        if now >= end {
            break;
        }
        std::thread::sleep(Duration::from_millis(50).min(end - now));
    }
}

/// Waits for our pdtd to answer health (with `version` when given). Fails early if the child exits.
fn wait_healthy(cfg: &HostConfig, child: &mut Option<Child>, version: Option<&str>, stop: &AtomicBool) -> bool {
    let end = Instant::now() + cfg.health_timeout;
    while Instant::now() < end && !stop.load(Ordering::SeqCst) {
        if let Probe::Ours { version: v } = probe(cfg.port)
            && version.is_none_or(|want| want == v)
        {
            return true;
        }
        if let Some(c) = child
            && c.try_wait().ok().flatten().is_some()
        {
            *child = None;
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn kill(child: &mut Option<Child>) {
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Returns once pdtd has gone: our child exited, or (adopted) it stopped answering three times in a row.
fn watch(cfg: &HostConfig, child: &mut Option<Child>, stop: &AtomicBool) {
    let mut misses = 0;
    while !stop.load(Ordering::SeqCst) {
        if let Some(c) = child {
            if c.try_wait().ok().flatten().is_some() {
                *child = None;
                return;
            }
        } else if matches!(probe(cfg.port), Probe::Ours { .. }) {
            misses = 0;
        } else {
            misses += 1;
            if misses >= 3 {
                return;
            }
        }
        sleep_unless_stopped(cfg.poll, stop);
    }
}

fn write_result(cfg: &HostConfig, r: &UpdateResult) {
    let path = result_path(&cfg.data_dir);
    let _ = std::fs::create_dir_all(path.parent().expect("update dir"));
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(r).expect("json"));
}

fn handle_update(
    cfg: &HostConfig,
    installer: &dyn Installer,
    child: &mut Option<Child>,
    req: UpdateRequest,
    stop: &AtomicBool,
) -> UpdateOutcome {
    // pdtd shut itself down after writing the request; make sure it is gone before files are replaced.
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        let exited = child.as_mut().is_none_or(|c| c.try_wait().ok().flatten().is_some());
        if exited && probe(cfg.port) == Probe::Free {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    kill(child);
    log(cfg, &format!("installing {} (from {})", req.version, req.from_version));
    let result = |outcome, message: String| UpdateResult {
        outcome,
        from_version: req.from_version.clone(),
        to_version: req.version.clone(),
        message,
        log: Some(log_path(cfg).to_string_lossy().into_owned()),
        at: jiff::Timestamp::now().as_millisecond(),
    };
    let r = match installer.install(&req.asset) {
        Err(e) => {
            log(cfg, &format!("install failed: {e}"));
            result(UpdateOutcome::Failed, format!("Update to v{} failed: {e}", req.version))
        }
        Ok(()) => {
            *child = spawn_pdtd(cfg).map_err(|e| log(cfg, &format!("could not start the new pdtd: {e}"))).ok();
            if wait_healthy(cfg, child, Some(&req.version), stop) {
                log(cfg, &format!("v{} is healthy", req.version));
                result(UpdateOutcome::Installed, format!("Updated to v{}", req.version))
            } else {
                log(cfg, &format!("v{} failed its health check; rolling back", req.version));
                kill(child);
                match installer.rollback() {
                    Ok(()) => result(
                        UpdateOutcome::RolledBack,
                        format!("Update to v{} failed and was rolled back", req.version),
                    ),
                    Err(e) => {
                        log(cfg, &format!("rollback failed: {e}"));
                        result(
                            UpdateOutcome::Failed,
                            format!("Update to v{} failed, and so did the rollback: {e}", req.version),
                        )
                    }
                }
            }
        }
    };
    write_result(cfg, &r);
    r.outcome
}

/// Runs until `stop` is set (tests); in production it runs for the life of the process. `after_install` is called
/// after a successful install (pdt-host's main uses it to replace itself last).
pub fn run(cfg: &HostConfig, installer: &dyn Installer, stop: &AtomicBool, after_install: &dyn Fn()) {
    let mut failures = 0u32;
    let mut child: Option<Child> = None;
    while !stop.load(Ordering::SeqCst) {
        if let Some(req) = take_request(&cfg.data_dir) {
            if handle_update(cfg, installer, &mut child, req, stop) == UpdateOutcome::Installed {
                after_install();
            }
            failures = 0;
            if child.is_none() {
                continue;
            }
        } else {
            match probe(cfg.port) {
                Probe::Ours { .. } => {}
                Probe::Foreign(why) => {
                    failures += 1;
                    log(cfg, &format!("port {} is held by something else ({why})", cfg.port));
                    sleep_unless_stopped(cfg.backoff.delay(failures), stop);
                    continue;
                }
                Probe::Free => {
                    child = match spawn_pdtd(cfg) {
                        Ok(c) => Some(c),
                        Err(e) => {
                            failures += 1;
                            log(cfg, &format!("could not start pdtd: {e}"));
                            sleep_unless_stopped(cfg.backoff.delay(failures), stop);
                            continue;
                        }
                    };
                    if !wait_healthy(cfg, &mut child, None, stop) {
                        kill(&mut child);
                        failures += 1;
                        log(cfg, "pdtd did not become healthy");
                        sleep_unless_stopped(cfg.backoff.delay(failures), stop);
                        continue;
                    }
                }
            }
        }
        let started = Instant::now();
        watch(cfg, &mut child, stop);
        if stop.load(Ordering::SeqCst) || request_path(&cfg.data_dir).exists() {
            continue;
        }
        if started.elapsed() >= cfg.backoff.reset_after {
            failures = 0;
        }
        failures += 1;
        let d = cfg.backoff.delay(failures);
        log(cfg, &format!("pdtd exited; restarting in {d:?}"));
        sleep_unless_stopped(d, stop);
    }
    kill(&mut child);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::handoff::request_path;
    use std::time::Instant;

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        let b = Backoff { initial: Duration::from_secs(1), max: Duration::from_secs(60), reset_after: Duration::ZERO };
        let d: Vec<u64> = (0..9).map(|f| b.delay(f).as_secs()).collect();
        assert_eq!(d, [0, 1, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(b.delay(1000), Duration::from_secs(60));
    }

    #[test]
    fn a_request_is_taken_once() {
        let tmp = tempfile::tempdir().unwrap();
        let req = UpdateRequest {
            version: "0.1.7".into(),
            tag: "v0.1.7".into(),
            asset: "a".into(),
            from_version: "0.1.6".into(),
            requested_at: 1,
        };
        let p = request_path(tmp.path());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, serde_json::to_vec(&req).unwrap()).unwrap();
        assert_eq!(take_request(tmp.path()), Some(req));
        assert_eq!(take_request(tmp.path()), None);
    }

    #[test]
    fn the_lock_is_retried_until_the_holder_lets_go() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("host.lock");
        let held = acquire_lock(&path, Duration::ZERO).unwrap().expect("first lock");
        let t = Instant::now();
        assert!(acquire_lock(&path, Duration::from_millis(400)).unwrap().is_none(), "still held");
        assert!(t.elapsed() >= Duration::from_millis(400), "it retried for the whole window");
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        assert!(acquire_lock(&path, Duration::from_secs(10)).unwrap().is_some(), "taken once released");
        release.join().unwrap();
    }
}
