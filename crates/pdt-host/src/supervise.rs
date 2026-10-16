//! The supervision loop (spec §3.1, §3.4): start pdtd (or adopt one that is running), restart it with backoff when it
//! exits, and install the update it hands over, rolling back when the new version fails its health check.

use crate::config::HostConfig;
use crate::health::{Probe, probe, request_shutdown};
use crate::install::Installer;
use crate::log::{log, log_path};
use pdt_core::handoff::{UpdateOutcome, UpdateRequest, UpdateResult, request_path, result_path, update_dir};
use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
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

/// Exists while an update is being installed (written after the backup, removed once the result is written). Found
/// at startup, it means pdt-host died mid-install: the update is rolled back.
pub fn marker_path(data_dir: &Path) -> PathBuf {
    update_dir(data_dir).join("installing.json")
}

#[derive(Debug, PartialEq)]
pub enum TakenRequest {
    None,
    Request(UpdateRequest),
    /// The request was taken but could not be parsed.
    Malformed(String),
    /// The request file could not be read or removed; it is left alone (installing it could repeat forever).
    Stuck(String),
}

/// Takes the pending update request, renaming (or else deleting) the file so a crash loop never installs it twice.
pub fn take_request(data_dir: &Path) -> TakenRequest {
    let path = request_path(data_dir);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return TakenRequest::None,
        Err(e) => return TakenRequest::Stuck(format!("could not read {}: {e}", path.display())),
    };
    if let Err(rename) = std::fs::rename(&path, path.with_extension("taken.json"))
        && let Err(remove) = std::fs::remove_file(&path)
    {
        return TakenRequest::Stuck(format!("could not take {} ({rename}; {remove})", path.display()));
    }
    match serde_json::from_slice(&bytes) {
        Ok(r) => TakenRequest::Request(r),
        Err(e) => TakenRequest::Malformed(e.to_string()),
    }
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

/// Starts pdtd with exactly `cfg.pdtd_env()` as its `PDT_*` variables (ruling A).
pub fn spawn_pdtd(cfg: &HostConfig) -> std::io::Result<Child> {
    let launch_log = cfg.data_dir.join("logs").join(format!("{}-launch.log", pdt_core::brand::DAEMON_BIN));
    let _ = std::fs::create_dir_all(launch_log.parent().expect("logs dir"));
    let out = File::options().create(true).append(true).open(&launch_log)?;
    let mut cmd = Command::new(cfg.pdtd_path());
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    cmd.envs(cfg.pdtd_env())
        .current_dir(&cfg.install_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out.try_clone()?))
        .stderr(Stdio::from(out));
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

fn stopped(stop: &AtomicBool) -> bool {
    stop.load(Ordering::SeqCst)
}

fn sleep_unless_stopped(d: Duration, stop: &AtomicBool) {
    let end = Instant::now() + d;
    while !stopped(stop) {
        let now = Instant::now();
        if now >= end {
            break;
        }
        std::thread::sleep(Duration::from_millis(50).min(end - now));
    }
}

/// Whether our child has exited (it is then forgotten).
fn reap(child: &mut Option<Child>) -> bool {
    if let Some(c) = child
        && c.try_wait().ok().flatten().is_some()
    {
        *child = None;
        return true;
    }
    false
}

/// Waits for our pdtd to answer health (with `version` when given). Fails early if the child exits.
fn wait_healthy(cfg: &HostConfig, child: &mut Option<Child>, version: Option<&str>, stop: &AtomicBool) -> bool {
    let end = Instant::now() + cfg.health_timeout;
    while Instant::now() < end && !stopped(stop) {
        if let Probe::Ours { version: v } = probe(cfg.port)
            && version.is_none_or(|want| want == v)
        {
            return true;
        }
        if reap(child) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// False when the child exits before `until`.
fn stays_up(child: &mut Option<Child>, until: Instant, stop: &AtomicBool) -> bool {
    while Instant::now() < until && !stopped(stop) {
        if reap(child) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

/// Waits until our child (if any) has exited and the port is free.
fn wait_gone(cfg: &HostConfig, child: &mut Option<Child>, within: Duration, stop: &AtomicBool) -> bool {
    let end = Instant::now() + within;
    loop {
        reap(child);
        if child.is_none() && probe(cfg.port) == Probe::Free {
            return true;
        }
        if Instant::now() >= end || stopped(stop) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn kill(child: &mut Option<Child>) {
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Returns once pdtd has gone (our child exited, or an adopted one stopped answering three times in a row), or, when
/// `notice_requests`, once an update request appears.
fn watch(cfg: &HostConfig, child: &mut Option<Child>, stop: &AtomicBool, notice_requests: bool) {
    let mut misses = 0;
    while !stopped(stop) {
        if notice_requests && request_path(&cfg.data_dir).exists() {
            return;
        }
        if child.is_some() {
            if reap(child) {
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

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("a parent"))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

fn write_result(cfg: &HostConfig, r: &UpdateResult) {
    if let Err(e) = write_atomic(&result_path(&cfg.data_dir), &serde_json::to_vec_pretty(r).expect("json")) {
        log(cfg, &format!("could not write the update result: {e}"));
    }
}

fn update_result(cfg: &HostConfig, outcome: UpdateOutcome, from: &str, to: &str, message: String) -> UpdateResult {
    UpdateResult {
        outcome,
        from_version: from.into(),
        to_version: to.into(),
        message,
        log: Some(log_path(cfg).to_string_lossy().into_owned()),
        at: jiff::Timestamp::now().as_millisecond(),
    }
}

/// Rolls back after the new version failed (`why`); the outcome and message to report.
fn roll_back(cfg: &HostConfig, installer: &dyn Installer, version: &str, why: &str) -> (UpdateOutcome, String) {
    log(cfg, &format!("v{version} failed ({why}); rolling back"));
    match installer.rollback() {
        Ok(()) => (UpdateOutcome::RolledBack, format!("Update to v{version} failed ({why}) and was rolled back")),
        Err(e) => {
            log(cfg, &format!("rollback failed: {e}"));
            (UpdateOutcome::Failed, format!("Update to v{version} failed ({why}), and so did the rollback: {e}"))
        }
    }
}

fn handle_update(
    cfg: &HostConfig,
    installer: &dyn Installer,
    child: &mut Option<Child>,
    req: UpdateRequest,
    stop: &AtomicBool,
) -> UpdateOutcome {
    let v = req.version.as_str();
    let finish = |outcome: UpdateOutcome, message: String| {
        write_result(cfg, &update_result(cfg, outcome, &req.from_version, v, message));
        let _ = std::fs::remove_file(marker_path(&cfg.data_dir));
        outcome
    };
    // pdtd shuts itself down after writing the request; nothing is touched until it is gone.
    if !wait_gone(cfg, child, cfg.stop_timeout, stop) && !stopped(stop) {
        if matches!(probe(cfg.port), Probe::Ours { .. }) {
            log(cfg, "pdtd is still running after handing over an update; asking it to stop");
            if let Err(e) = request_shutdown(cfg.port) {
                log(cfg, &format!("the shutdown request failed: {e}"));
            }
        }
        wait_gone(cfg, child, cfg.stop_timeout, stop);
    }
    if child.is_some() || probe(cfg.port) != Probe::Free {
        log(cfg, &format!("pdtd did not stop; v{v} was not installed"));
        return finish(
            UpdateOutcome::Failed,
            format!("Update to v{v} was not installed: the running version did not stop"),
        );
    }
    log(cfg, &format!("installing {v} (from {})", req.from_version));
    if let Err(e) = installer.backup() {
        log(cfg, &format!("install failed: {e}"));
        return finish(UpdateOutcome::Failed, format!("Update to v{v} failed: {e}"));
    }
    let marker = serde_json::json!({ "request": &req, "backup": installer.backup_path() });
    if let Err(e) = write_atomic(&marker_path(&cfg.data_dir), marker.to_string().as_bytes()) {
        log(cfg, &format!("could not write the install marker: {e}"));
        return finish(UpdateOutcome::Failed, format!("Update to v{v} failed: could not record the install: {e}"));
    }
    if let Err(e) = installer.install(&req.asset) {
        let (outcome, message) = roll_back(cfg, installer, v, &e);
        return finish(outcome, message);
    }
    // The new pdtd reads the result at startup; it is overwritten if the new version is rolled back.
    write_result(cfg, &update_result(cfg, UpdateOutcome::Installed, &req.from_version, v, format!("Updated to v{v}")));
    let started = Instant::now();
    *child = match spawn_pdtd(cfg) {
        Ok(c) => Some(c),
        Err(e) => {
            let (outcome, message) = roll_back(cfg, installer, v, &format!("it could not be started: {e}"));
            return finish(outcome, message);
        }
    };
    let failure = if !wait_healthy(cfg, child, Some(v), stop) {
        Some("it failed its health check")
    } else if !stays_up(child, started + cfg.health_timeout, stop) {
        Some("it exited soon after starting")
    } else {
        None
    };
    if let Some(why) = failure {
        kill(child);
        let (outcome, message) = roll_back(cfg, installer, v, why);
        return finish(outcome, message);
    }
    log(cfg, &format!("v{v} is healthy"));
    finish(UpdateOutcome::Installed, format!("Updated to v{v}"))
}

/// At startup: rolls back an install that pdt-host died in the middle of, and restores the previous version when
/// pdtd is missing.
fn recover(cfg: &HostConfig, installer: &dyn Installer) {
    let marker = marker_path(&cfg.data_dir);
    if let Ok(bytes) = std::fs::read(&marker) {
        let req = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| serde_json::from_value::<UpdateRequest>(v["request"].clone()).ok());
        let (from, to) = req.map(|r| (r.from_version, r.version)).unwrap_or_default();
        log(cfg, &format!("an update to v{to} was interrupted; rolling back"));
        let (outcome, message) = match installer.rollback() {
            Ok(()) => (UpdateOutcome::RolledBack, format!("Update to v{to} was interrupted and was rolled back")),
            Err(e) => {
                log(cfg, &format!("rollback failed: {e}"));
                (UpdateOutcome::Failed, format!("Update to v{to} was interrupted, and the rollback failed: {e}"))
            }
        };
        write_result(cfg, &update_result(cfg, outcome, &from, &to, message));
        let _ = std::fs::remove_file(&marker);
    }
    if !cfg.pdtd_path().is_file() {
        match installer.backup_path() {
            Some(b) if b.exists() => {
                log(cfg, &format!("{} is missing; restoring the previous version", cfg.pdtd_path().display()));
                if let Err(e) = installer.rollback() {
                    log(cfg, &format!("restoring the previous version failed: {e}"));
                }
            }
            _ => log(cfg, &format!("{} is missing; retrying until it appears", cfg.pdtd_path().display())),
        }
    }
}

/// Runs until `stop` is set (tests); in production it runs for the life of the process. `after_install` is called
/// after a successful install (pdt-host's main uses it to replace itself last).
pub fn run(cfg: &HostConfig, installer: &dyn Installer, stop: &AtomicBool, after_install: &dyn Fn()) {
    recover(cfg, installer);
    let mut failures = 0u32;
    let mut child: Option<Child> = None;
    // Set when a request file cannot be removed: it is then left alone until pdt-host restarts.
    let mut ignore_requests = false;
    while !stopped(stop) {
        let taken = if ignore_requests { TakenRequest::None } else { take_request(&cfg.data_dir) };
        match taken {
            TakenRequest::Request(req) => {
                if handle_update(cfg, installer, &mut child, req, stop) == UpdateOutcome::Installed {
                    after_install();
                }
                failures = 0;
                if child.is_none() {
                    continue;
                }
            }
            TakenRequest::Malformed(e) => {
                log(cfg, &format!("the update request could not be read: {e}"));
                let message = format!("The update request could not be read: {e}");
                write_result(cfg, &update_result(cfg, UpdateOutcome::Failed, "", "", message));
                continue;
            }
            TakenRequest::Stuck(e) => {
                log(cfg, &format!("{e}; update requests are ignored until pdt-host restarts"));
                ignore_requests = true;
                continue;
            }
            TakenRequest::None => match probe(cfg.port) {
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
            },
        }
        let started = Instant::now();
        watch(cfg, &mut child, stop, !ignore_requests);
        if stopped(stop) || (!ignore_requests && request_path(&cfg.data_dir).exists()) {
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
        assert_eq!(take_request(tmp.path()), TakenRequest::Request(req));
        assert_eq!(take_request(tmp.path()), TakenRequest::None);
    }

    #[test]
    fn a_malformed_request_is_taken_and_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let p = request_path(tmp.path());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{not json").unwrap();
        assert!(matches!(take_request(tmp.path()), TakenRequest::Malformed(_)));
        assert!(!p.exists());
    }

    #[test]
    fn an_unreadable_request_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let p = request_path(tmp.path());
        std::fs::create_dir_all(&p).unwrap(); // a folder where the file should be
        assert!(matches!(take_request(tmp.path()), TakenRequest::Stuck(_)));
        assert!(p.exists());
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
