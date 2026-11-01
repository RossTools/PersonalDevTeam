#[path = "../../../test-support/port_blocks.rs"]
mod port_blocks;

use pdt_core::handoff::{
    DB_FILE, UpdateOutcome, UpdateRequest, UpdateResult, UpdateSkip, db_snapshot_dir, request_path, result_path,
    skip_path,
};
use pdt_host::config::HostConfig;
use pdt_host::health::{Probe, probe, request_shutdown};
use pdt_host::install::{CopyDirInstaller, Installer, NsisInstaller, Unsupported, host_installer};
use pdt_host::log::log_path;
use pdt_host::supervise::{Backoff, marker_path, run};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const EXE: &str = std::env::consts::EXE_SUFFIX;

/// A port no other test holds, below every OS's ephemeral range (Linux hands out 32768 and up, Windows and macOS
/// 49152 and up): a port from `bind(0)` can be handed to another test's outgoing connection while pdtd restarts, and
/// the new pdtd then cannot bind it.
fn free_port() -> u16 {
    port_blocks::claim(30000, 250, 1)
}

/// A folder holding the fake pdtd (as `pdtd[.exe]`) and its `fake.json`.
fn fake_install(dir: &Path, version: &str, mode: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_fake-pdtd"), dir.join(format!("pdtd{EXE}"))).unwrap();
    std::fs::write(dir.join("fake.json"), serde_json::json!({"version": version, "mode": mode}).to_string()).unwrap();
    dir.to_path_buf()
}

/// `fake_install`, as a version with a new migration (see fake-pdtd's "migrate").
fn fake_migrating_install(dir: &Path, version: &str, mode: &str) -> PathBuf {
    let d = fake_install(dir, version, mode);
    let cfg = serde_json::json!({"version": version, "mode": mode, "migrate": true});
    std::fs::write(d.join("fake.json"), cfg.to_string()).unwrap();
    d
}

/// The database as the old version left it: the main file and its WAL, no `-shm`.
fn old_database(data: &Path) {
    std::fs::create_dir_all(data).unwrap();
    std::fs::write(data.join(DB_FILE), "old data").unwrap();
    std::fs::write(data.join(format!("{DB_FILE}-wal")), "old wal").unwrap();
}

fn assert_old_database(data: &Path) {
    assert_eq!(read(&data.join(DB_FILE)), "old data");
    assert_eq!(read(&data.join(format!("{DB_FILE}-wal"))), "old wal");
    assert!(!data.join(format!("{DB_FILE}-shm")).exists(), "the new version's -shm was removed");
    assert!(!db_snapshot_dir(data).exists(), "the snapshot is removed once restored");
}

/// A fake NSIS installer (`setup[.exe]`) that behaves as `behaviour` says.
fn fake_installer(dir: &Path, behaviour: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let exe = dir.join(format!("setup{EXE}"));
    std::fs::copy(env!("CARGO_BIN_EXE_fake-pdtd"), &exe).unwrap();
    std::fs::write(dir.join("fake.json"), serde_json::json!({"installer": behaviour}).to_string()).unwrap();
    exe
}

/// A failing health check fails fast enough for a test, while a pdtd started on a loaded machine still has time to
/// answer.
const FAIL_FAST: Duration = Duration::from_secs(5);

/// Generous health timeout (a fresh copy of the fake pdtd can take seconds to start under load) and a short watch
/// window, which starts only once the new version answers.
fn config(install: &Path, data: &Path, port: u16) -> HostConfig {
    HostConfig {
        install_dir: install.to_path_buf(),
        data_dir: data.to_path_buf(),
        port,
        port_setting: false,
        dev: true,
        health_timeout: Duration::from_secs(30),
        watch: Duration::from_secs(3),
        backoff: Backoff {
            initial: Duration::from_millis(100),
            max: Duration::from_millis(500),
            reset_after: Duration::from_secs(5),
        },
        poll: Duration::from_millis(200),
        stop_timeout: Duration::from_secs(10),
        forward_env: Vec::new(),
        updates_unsupported: None,
    }
}

struct Running {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    fn is_running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn supervise_with(cfg: HostConfig, installer: Box<dyn Installer + Send>) -> Running {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let handle = std::thread::spawn(move || run(&cfg, installer.as_ref(), &s, &|| {}));
    Running { stop, handle: Some(handle) }
}

fn supervise(cfg: HostConfig, backup: PathBuf) -> Running {
    let installer = CopyDirInstaller { target: cfg.install_dir.clone(), backup };
    supervise_with(cfg, Box::new(installer))
}

/// A pdtd started outside pdt-host; killed when dropped.
struct Outside(std::process::Child);

impl Outside {
    fn start(install: &Path, port: u16) -> Outside {
        Outside::start_with(install, vec![("PDT_PORT".into(), port.to_string())])
    }

    /// Started with exactly `env` as its `PDT_*` variables.
    fn start_with(install: &Path, env: Vec<(String, String)>) -> Outside {
        let mut cmd = std::process::Command::new(install.join(format!("pdtd{EXE}")));
        for (k, _) in std::env::vars() {
            if k.starts_with("PDT_") {
                cmd.env_remove(k);
            }
        }
        Outside(cmd.envs(env).spawn().unwrap())
    }
}

impl Drop for Outside {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_version(port: u16, want: &str, within: Duration) {
    let end = Instant::now() + within;
    loop {
        if probe(port) == (Probe::Ours { version: want.into() }) {
            return;
        }
        assert!(Instant::now() < end, "never saw v{want} on port {port} (last: {:?})", probe(port));
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The final result: written, and the install marker gone (a provisional `installed` precedes the health check).
fn read_result(data: &Path) -> UpdateResult {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        if !marker_path(data).exists()
            && let Ok(b) = std::fs::read(result_path(data))
            && let Ok(r) = serde_json::from_slice(&b)
        {
            return r;
        }
        assert!(Instant::now() < end, "no update result was written");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn update_request(version: &str, asset: &Path) -> UpdateRequest {
    UpdateRequest {
        version: version.into(),
        tag: format!("v{version}"),
        asset: asset.to_path_buf(),
        from_version: "0.1.5".into(),
        requested_at: 1,
    }
}

/// Writes the request as pdtd does: atomically, since pdt-host may read it at any moment.
fn request(data: &Path, version: &str, asset: &Path) {
    let p = request_path(data);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(&update_request(version, asset)).unwrap()).unwrap();
    std::fs::rename(tmp, p).unwrap();
}

/// Asks the running pdtd to hand over the update just requested. A refused shutdown is fine: pdt-host may have just
/// replaced a pdtd that was slow to answer, and then takes the request itself.
fn hand_over(port: u16) {
    let _ = request_shutdown(port);
}

/// Waits for pdt-host to log `needle` (the signal that it reached a step).
fn wait_log(cfg: &HostConfig, needle: &str) {
    let end = Instant::now() + Duration::from_secs(60);
    while !read(&log_path(cfg)).contains(needle) {
        let launch = cfg.data_dir.join("logs").join("pdtd-launch.log");
        assert!(
            Instant::now() < end,
            "pdt-host never logged {needle:?}:\n{}\npdtd's output:\n{}",
            read(&log_path(cfg)),
            read(&launch)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn skip(data: &Path) -> Option<UpdateSkip> {
    serde_json::from_slice(&std::fs::read(skip_path(data)).ok()?).ok()
}

fn write_skip(data: &Path, version: &str) {
    let p = skip_path(data);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let s = UpdateSkip { version: version.into(), reason: "earlier".into(), at: 1 };
    std::fs::write(p, serde_json::to_vec(&s).unwrap()).unwrap();
}

fn process_alive(pid: u32) -> bool {
    if cfg!(windows) {
        let out =
            std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    } else {
        std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().unwrap().success()
    }
}

fn wait_for_marker_gone(data: &Path) {
    let end = Instant::now() + Duration::from_secs(30);
    while marker_path(data).exists() {
        assert!(Instant::now() < end, "the install marker was never removed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn starts_pdtd_supervised_and_restarts_it_after_a_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "crash_once");
    let port = free_port();
    let _h = supervise(config(&install, &tmp.path().join("data"), port), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    assert!(install.join("crashed-once").exists(), "the first start crashed");
    assert_eq!(read(&install.join("env.txt")), "1", "PDT_SUPERVISED is set");
    // A clean exit (not an update) is restarted too.
    request_shutdown(port).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

/// Ruling A: pdtd gets an explicit profile, port and data dir, and only the forwarded extras.
#[test]
fn pdtd_is_started_with_exactly_the_host_s_variables() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let mut cfg = config(&install, &data, port);
    cfg.forward_env = vec![("PDT_WEB_DIR".into(), "/web".into()), ("PDT_CLAUDE_PROGRAM".into(), "x".into())];
    let _h = supervise(cfg, tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    let got: HashMap<String, String> = serde_json::from_str(&read(&install.join("env.json"))).unwrap();
    let port = port.to_string();
    let data = data.to_string_lossy().into_owned();
    let want: HashMap<String, String> = [
        ("PDT_PROFILE", "dev"),
        ("PDT_PORT", port.as_str()),
        ("PDT_DATA_DIR", data.as_str()),
        ("PDT_SUPERVISED", "1"),
        ("PDT_WEB_DIR", "/web"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn installs_an_update_and_reports_it() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    write_skip(&data, "0.1.4");
    let _h = supervise(config(&install, &data, port), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    hand_over(port);
    wait_version(port, "0.1.6", Duration::from_secs(30));
    assert_eq!(read(&install.join("seen-result.txt")), "installed", "the new pdtd saw the result at startup");
    let r = read_result(&data);
    assert_eq!((r.outcome, r.to_version.as_str()), (UpdateOutcome::Installed, "0.1.6"));
    assert!(!request_path(&data).exists(), "the request was taken");
    assert_eq!(skip(&data), None, "a successful update lifts the skip");
}

#[test]
fn rolls_back_when_the_new_version_fails_its_health_check() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "unhealthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let mut cfg = config(&install, &data, port);
    cfg.health_timeout = FAIL_FAST;
    let _h = supervise(cfg, tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    assert!(r.message.contains("0.1.6"));
    wait_version(port, "0.1.5", Duration::from_secs(30));
    assert_eq!(read(&install.join("seen-result.txt")), "rolled_back", "the old pdtd saw the final result");
    assert_eq!(skip(&data).map(|s| s.version), Some("0.1.6".into()), "automatic updates skip v0.1.6");
}

/// The v0.1.42 incident: the new version migrated the database and then failed its health check. The old version
/// cannot open a database with a migration it does not know, so the rollback restores the database too.
#[test]
fn a_rollback_restores_the_database_the_new_version_migrated() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_migrating_install(&tmp.path().join("release"), "0.1.6", "unhealthy");
    let data = tmp.path().join("data");
    old_database(&data);
    let port = free_port();
    let mut cfg = config(&install, &data, port);
    cfg.health_timeout = FAIL_FAST;
    let _h = supervise(cfg, tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(30));
    assert_old_database(&data);
}

#[test]
fn a_successful_update_keeps_the_migrated_database() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_migrating_install(&tmp.path().join("release"), "0.1.6", "healthy");
    let data = tmp.path().join("data");
    old_database(&data);
    let port = free_port();
    let _h = supervise(config(&install, &data, port), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    hand_over(port);
    wait_version(port, "0.1.6", Duration::from_secs(30));
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::Installed, "{}", r.message);
    assert_eq!(read(&data.join(DB_FILE)), "migrated by 0.1.6");
    assert!(!db_snapshot_dir(&data).exists(), "the snapshot is dropped once the update is in");
}

/// A first install has no database yet: rolling back removes the one the new version created.
#[test]
fn a_rollback_without_a_database_before_removes_the_new_one() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_migrating_install(&tmp.path().join("release"), "0.1.6", "unhealthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let mut cfg = config(&install, &data, port);
    cfg.health_timeout = FAIL_FAST;
    let _h = supervise(cfg, tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    for suffix in ["", "-wal", "-shm"] {
        assert!(!data.join(format!("{DB_FILE}{suffix}")).exists(), "{DB_FILE}{suffix} was removed");
    }
}

#[test]
fn rolls_back_a_new_version_that_exits_soon_after_becoming_healthy() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "die_soon");
    let data = tmp.path().join("data");
    let port = free_port();
    let _h = supervise(config(&install, &data, port), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(30));
}

/// Review issue 1: an installer that changes files and then fails is rolled back.
#[test]
fn a_failed_installer_is_rolled_back() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let setup = fake_installer(&tmp.path().join("release"), "fail_partial");
    let data = tmp.path().join("data");
    let port = free_port();
    let installer = NsisInstaller {
        install_dir: install.clone(),
        backup: tmp.path().join("backup"),
        registry_backup: None,
        timeout: Duration::from_secs(30),
    };
    let _h = supervise_with(config(&install, &data, port), Box::new(installer));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &setup);
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    assert!(r.message.contains("installer failed"), "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(30));
    assert!(read(&install.join("fake.json")).contains("0.1.5"), "the half-written file was restored");
}

/// PDT-213: an installer that reports success but leaves the old pdtd in place (a running copy kept it open) is rolled
/// back, saying so, rather than failing a health check against the old version.
#[test]
fn an_installer_that_did_not_replace_pdtd_is_rolled_back() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let setup = fake_installer(&tmp.path().join("release"), "noop");
    let data = tmp.path().join("data");
    let port = free_port();
    let installer = NsisInstaller {
        install_dir: install.clone(),
        backup: tmp.path().join("backup"),
        registry_backup: None,
        timeout: Duration::from_secs(30),
    };
    let cfg = config(&install, &data, port);
    let _h = supervise_with(cfg.clone(), Box::new(installer));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &setup);
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    assert!(r.message.contains("the installer did not replace pdtd"), "{}", r.message);
    // Not the update's health check (pdt-host's own startup check may have logged one, which is unrelated).
    assert!(!r.message.contains("health check"), "{}", r.message);
    assert!(!read(&log_path(&cfg)).contains("failed its health check"), "{}", read(&log_path(&cfg)));
    wait_version(port, "0.1.5", Duration::from_secs(30));
}

/// Review issue 3: an installer that never finishes is stopped and the update rolled back.
#[test]
fn a_hanging_installer_times_out_and_is_rolled_back() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let setup = fake_installer(&tmp.path().join("release"), "hang");
    let data = tmp.path().join("data");
    let port = free_port();
    let installer = NsisInstaller {
        install_dir: install.clone(),
        backup: tmp.path().join("backup"),
        registry_backup: None,
        timeout: Duration::from_secs(1),
    };
    let _h = supervise_with(config(&install, &data, port), Box::new(installer));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &setup);
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    assert!(r.message.contains("timed out"), "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(30));
}

/// Ruling F9: with no installer (e.g. a dev build outside an .app), pdt-host keeps supervising and refuses updates.
#[test]
fn refuses_updates_it_cannot_install_and_keeps_supervising() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let reason = "pdt-host is not installed from an .app bundle".to_string();
    let _h = supervise_with(config(&install, &data, port), Box::new(Unsupported { reason }));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::Failed, "{}", r.message);
    assert!(r.message.contains("0.1.6") && r.message.contains(".app bundle"), "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(30));
}

/// Final review #1: a dev pdt-host never installs anything, whatever the OS or bundle: the request fails untouched.
#[test]
fn a_dev_host_never_runs_an_installer() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let setup = fake_installer(&tmp.path().join("release"), "fail_partial");
    let data = tmp.path().join("data");
    let port = free_port();
    let mut cfg = config(&install, &data, port);
    assert!(cfg.dev);
    let installer = host_installer(&mut cfg);
    assert_eq!(cfg.updates_unsupported, None, "a dev pdtd already knows it never updates");
    assert_eq!(installer.backup_path(), None, "a dev host keeps no backup: it never installs");
    let _h = supervise_with(cfg, installer);
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &setup);
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::Failed, "{}", r.message);
    assert!(r.message.contains("dev profile never installs updates"), "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(30));
    assert!(read(&install.join("fake.json")).contains("0.1.5"), "the installer never ran");
}

/// Review issue 9: a request that cannot be parsed gets a failed result, and pdtd comes back.
#[test]
fn a_malformed_request_is_reported_and_pdtd_restarted() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    write_skip(&data, "0.1.4");
    let _h = supervise(config(&install, &data, port), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    std::fs::create_dir_all(request_path(&data).parent().unwrap()).unwrap();
    std::fs::write(request_path(&data), "{garbage").unwrap();
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::Failed, "{}", r.message);
    assert!(r.message.contains("could not be read"), "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(30));
    assert_eq!(skip(&data).map(|s| s.version), Some("0.1.4".into()), "a request without a version keeps the skip");
}

/// Review issue 4: a pdtd that does not stop is asked to, and if it still runs nothing is installed.
#[test]
fn nothing_is_installed_while_pdtd_keeps_running() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "stubborn");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let _outside = Outside::start(&install, port);
    wait_version(port, "0.1.5", Duration::from_secs(30));
    let mut cfg = config(&install, &data, port);
    cfg.stop_timeout = Duration::from_secs(1);
    let _h = supervise(cfg, tmp.path().join("backup"));
    request(&data, "0.1.6", &asset);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::Failed, "{}", r.message);
    assert!(r.message.contains("did not stop"), "{}", r.message);
    assert!(read(&install.join("fake.json")).contains("0.1.5"), "the install folder was not touched");
    wait_version(port, "0.1.5", Duration::from_secs(5));
}

/// Review issue 6: an install pdt-host died in the middle of is rolled back at startup.
#[test]
fn an_interrupted_install_is_rolled_back_at_startup() {
    let tmp = tempfile::tempdir().unwrap();
    let (backup, install, data) = interrupted_install(tmp.path(), "healthy");
    let port = free_port();
    let _h = supervise(config(&install, &data, port), backup);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    assert!(r.message.contains("0.1.6") && r.message.contains("interrupted"), "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

/// An install pdt-host died in the middle of (after the new version migrated the database) restores the database
/// with the app.
#[test]
fn an_interrupted_install_restores_the_database() {
    let tmp = tempfile::tempdir().unwrap();
    let (backup, install, data) = interrupted_install(tmp.path(), "healthy");
    // As pdt-host left it: the snapshot taken before the install, and the new version's migrated database.
    let snap = db_snapshot_dir(&data);
    std::fs::create_dir_all(&snap).unwrap();
    std::fs::write(snap.join(DB_FILE), "old data").unwrap();
    std::fs::write(snap.join(format!("{DB_FILE}-wal")), "old wal").unwrap();
    std::fs::write(data.join(DB_FILE), "migrated by 0.1.6").unwrap();
    std::fs::write(data.join(format!("{DB_FILE}-shm")), "new").unwrap();
    let port = free_port();
    let _h = supervise(config(&install, &data, port), backup);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(20));
    assert_old_database(&data);
}

/// Review issue 6: a missing pdtd is restored from the previous version.
#[test]
fn a_missing_pdtd_is_restored_from_the_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let backup = fake_install(&tmp.path().join("backup"), "0.1.5", "healthy");
    let install = tmp.path().join("app");
    std::fs::create_dir_all(&install).unwrap();
    let port = free_port();
    let _h = supervise(config(&install, &tmp.path().join("data"), port), backup);
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

/// Review issue 6: with nothing runnable pdt-host keeps running and starts pdtd once it appears.
#[test]
fn with_nothing_to_run_it_keeps_trying() {
    let tmp = tempfile::tempdir().unwrap();
    let install = tmp.path().join("app");
    std::fs::create_dir_all(&install).unwrap();
    let port = free_port();
    let h = supervise(config(&install, &tmp.path().join("data"), port), tmp.path().join("none"));
    std::thread::sleep(Duration::from_secs(1));
    assert!(h.is_running(), "pdt-host did not give up");
    fake_install(&install, "0.1.5", "healthy");
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

/// A pdtd started with the variables this host would use is adopted as it is (PDT-261).
#[test]
fn adopts_a_running_pdtd_and_takes_over_when_it_stops() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let port = free_port();
    let cfg = config(&install, &tmp.path().join("data"), port);
    let mut outside = Outside::start_with(&install, cfg.pdtd_env());
    wait_version(port, "0.1.5", Duration::from_secs(30));
    std::fs::remove_file(install.join("env.json")).unwrap();
    let _h = supervise(cfg.clone(), tmp.path().join("backup"));
    std::thread::sleep(Duration::from_secs(2));
    assert!(outside.0.try_wait().unwrap().is_none(), "the running pdtd was adopted, not restarted");
    assert!(!install.join("env.json").exists(), "no pdtd was started");
    assert!(!read(&log_path(&cfg)).contains("different environment"), "{}", read(&log_path(&cfg)));
    request_shutdown(port).unwrap();
    let _ = outside.0.wait();
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

/// PDT-261: a pdtd an older pdt-host started (during an update, before this one replaced itself) with other variables
/// than this host uses is restarted once, gracefully, with this host's.
#[test]
fn restarts_an_adopted_pdtd_started_with_other_variables() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let port = free_port();
    let mut cfg = config(&install, &tmp.path().join("data"), port);
    let older = cfg.pdtd_env();
    cfg.forward_env = vec![("PDT_WEB_DIR".into(), "/web".into())];
    let mut outside = Outside::start_with(&install, older);
    wait_version(port, "0.1.5", Duration::from_secs(30));
    let _h = supervise(cfg.clone(), tmp.path().join("backup"));
    wait_log(&cfg, "pdtd was started by an older pdt-host with a different environment; restarting");
    assert!(outside.0.wait().unwrap().success(), "asked to stop, not killed");
    wait_version(port, "0.1.5", Duration::from_secs(20));
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let env: HashMap<String, String> = serde_json::from_str(&read(&install.join("env.json"))).unwrap_or_default();
        if env.get("PDT_WEB_DIR").map(String::as_str) == Some("/web") {
            break;
        }
        assert!(Instant::now() < end, "pdtd was never started with this host's variables: {env:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
    // Once: the restarted pdtd is this host's own and is left alone.
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(read(&log_path(&cfg)).matches("different environment").count(), 1, "{}", read(&log_path(&cfg)));
}

/// Review round 2: a dismissed result is not written again when the new version passes its watch window.
#[test]
fn a_result_dismissed_during_the_health_window_stays_dismissed() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let mut cfg = config(&install, &data, port);
    cfg.watch = Duration::from_secs(10); // the test acts inside it, then waits for it to end
    let _h = supervise(cfg.clone(), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    hand_over(port);
    wait_log(&cfg, "v0.1.6 answered its health check");
    assert!(marker_path(&data).exists(), "still inside the health window");
    std::fs::remove_file(result_path(&data)).unwrap(); // what Dismiss does
    wait_for_marker_gone(&data);
    assert!(!result_path(&data).exists(), "the dismissed result was not resurrected");
}

/// Review round 2: a new version shut down cleanly inside the health window is not a failed version.
#[test]
fn a_clean_shutdown_inside_the_health_window_is_not_rolled_back() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let mut cfg = config(&install, &data, port);
    cfg.watch = Duration::from_secs(60); // ends early on the clean exit
    let _h = supervise(cfg.clone(), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    hand_over(port);
    wait_log(&cfg, "v0.1.6 answered its health check");
    assert!(marker_path(&data).exists(), "still inside the health window");
    request_shutdown(port).unwrap();
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::Installed, "{}", r.message);
    wait_version(port, "0.1.6", Duration::from_secs(30));
}

/// Review round 2: on timeout the installer's children are stopped too.
#[test]
fn a_timed_out_installer_is_stopped_with_its_children() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let setup = fake_installer(&tmp.path().join("release"), "hang_tree");
    let data = tmp.path().join("data");
    let port = free_port();
    let installer = NsisInstaller {
        install_dir: install.clone(),
        backup: tmp.path().join("backup"),
        registry_backup: None,
        timeout: Duration::from_secs(30), // the installer and its child must start before it, even under load
    };
    let _h = supervise_with(config(&install, &data, port), Box::new(installer));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &setup);
    hand_over(port);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    let pid: u32 = read(&tmp.path().join("release").join("grandchild.pid")).trim().parse().expect("the child started");
    let end = Instant::now() + Duration::from_secs(10);
    while process_alive(pid) {
        assert!(Instant::now() < end, "the installer's child {pid} is still running");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn interrupted_install(tmp: &Path, mode: &str) -> (PathBuf, PathBuf, PathBuf) {
    let backup = fake_install(&tmp.join("backup"), "0.1.5", "healthy");
    let install = fake_install(&tmp.join("app"), "0.1.6", mode);
    let data = tmp.join("data");
    let marker = marker_path(&data);
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    let req = update_request("0.1.6", &tmp.join("asset"));
    std::fs::write(&marker, serde_json::json!({"request": req, "backup": backup}).to_string()).unwrap();
    (backup, install, data)
}

/// Review round 2: recovery stops a running pdtd before it restores anything.
#[test]
fn recovery_stops_a_running_pdtd_first() {
    let tmp = tempfile::tempdir().unwrap();
    let (backup, install, data) = interrupted_install(tmp.path(), "healthy");
    let port = free_port();
    let mut outside = Outside::start(&install, port);
    wait_version(port, "0.1.6", Duration::from_secs(30));
    let _h = supervise(config(&install, &data, port), backup);
    let _ = outside.0.wait();
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

/// Review round 2: a pdtd that will not stop is left alone (marker kept) and recovery runs once it has gone.
#[test]
fn recovery_waits_for_a_pdtd_that_will_not_stop() {
    let tmp = tempfile::tempdir().unwrap();
    let (backup, install, data) = interrupted_install(tmp.path(), "stubborn");
    let port = free_port();
    let outside = Outside::start(&install, port);
    wait_version(port, "0.1.6", Duration::from_secs(30));
    let mut cfg = config(&install, &data, port);
    cfg.stop_timeout = Duration::from_secs(1);
    let _h = supervise(cfg, backup);
    std::thread::sleep(Duration::from_secs(3));
    assert!(marker_path(&data).exists(), "the marker is kept");
    assert!(!result_path(&data).exists(), "nothing was rolled back yet");
    assert!(read(&install.join("fake.json")).contains("0.1.6"), "the running version's files are untouched");
    // Still serving (one probe can take seconds on a loaded runner, so this is not a 1 s check).
    wait_version(port, "0.1.6", Duration::from_secs(10));
    drop(outside);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

/// PDT-213: once another pdt-host records itself as the data folder's supervisor, this one steps down and leaves pdtd
/// running for it.
#[test]
fn steps_down_when_another_pdt_host_takes_over() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let cfg = config(&install, &data, port);
    let h = supervise(cfg.clone(), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    std::fs::write(pdt_host::register::pid_path(&data), std::process::id().to_string()).unwrap();
    std::thread::sleep(Duration::from_secs(1));
    assert!(h.is_running(), "its own record is not another pdt-host");
    let other = std::process::id().wrapping_add(1);
    std::fs::write(pdt_host::register::pid_path(&data), other.to_string()).unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    while h.is_running() {
        assert!(Instant::now() < end, "it kept supervising");
        std::thread::sleep(Duration::from_millis(50));
    }
    let log = read(&log_path(&cfg));
    assert!(log.contains(&format!("another pdt-host (process {other}) supervises this data folder now")), "{log}");
    assert_eq!(probe(port), Probe::Ours { version: "0.1.5".into() }, "pdtd was left running");
    request_shutdown(port).unwrap();
}
