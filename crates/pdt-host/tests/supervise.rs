use pdt_core::handoff::{UpdateOutcome, UpdateRequest, UpdateResult, UpdateSkip, request_path, result_path, skip_path};
use pdt_host::config::HostConfig;
use pdt_host::health::{Probe, probe, request_shutdown};
use pdt_host::install::{CopyDirInstaller, Installer, NsisInstaller, Unsupported, host_installer};
use pdt_host::supervise::{Backoff, marker_path, run};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const EXE: &str = std::env::consts::EXE_SUFFIX;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A folder holding the fake pdtd (as `pdtd[.exe]`) and its `fake.json`.
fn fake_install(dir: &Path, version: &str, mode: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_fake-pdtd"), dir.join(format!("pdtd{EXE}"))).unwrap();
    std::fs::write(dir.join("fake.json"), serde_json::json!({"version": version, "mode": mode}).to_string()).unwrap();
    dir.to_path_buf()
}

/// A fake NSIS installer (`setup[.exe]`) that behaves as `behaviour` says.
fn fake_installer(dir: &Path, behaviour: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let exe = dir.join(format!("setup{EXE}"));
    std::fs::copy(env!("CARGO_BIN_EXE_fake-pdtd"), &exe).unwrap();
    std::fs::write(dir.join("fake.json"), serde_json::json!({"installer": behaviour}).to_string()).unwrap();
    exe
}

fn config(install: &Path, data: &Path, port: u16, health: Duration) -> HostConfig {
    HostConfig {
        install_dir: install.to_path_buf(),
        data_dir: data.to_path_buf(),
        port,
        dev: true,
        health_timeout: health,
        backoff: Backoff {
            initial: Duration::from_millis(100),
            max: Duration::from_millis(500),
            reset_after: Duration::from_secs(5),
        },
        poll: Duration::from_millis(200),
        stop_timeout: Duration::from_secs(10),
        forward_env: Vec::new(),
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
        let mut cmd = std::process::Command::new(install.join(format!("pdtd{EXE}")));
        for (k, _) in std::env::vars() {
            if k.starts_with("PDT_") {
                cmd.env_remove(k);
            }
        }
        Outside(cmd.env("PDT_PORT", port.to_string()).spawn().unwrap())
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

fn request(data: &Path, version: &str, asset: &Path) {
    let p = request_path(data);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, serde_json::to_vec(&update_request(version, asset)).unwrap()).unwrap();
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
    let _h =
        supervise(config(&install, &tmp.path().join("data"), port, Duration::from_secs(10)), tmp.path().join("backup"));
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
    let mut cfg = config(&install, &data, port, Duration::from_secs(10));
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
    let _h = supervise(config(&install, &data, port, Duration::from_secs(3)), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    request_shutdown(port).unwrap();
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
    let _h = supervise(config(&install, &data, port, Duration::from_secs(3)), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    request_shutdown(port).unwrap();
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    assert!(r.message.contains("0.1.6"));
    wait_version(port, "0.1.5", Duration::from_secs(30));
    assert_eq!(read(&install.join("seen-result.txt")), "rolled_back", "the old pdtd saw the final result");
    assert_eq!(skip(&data).map(|s| s.version), Some("0.1.6".into()), "automatic updates skip v0.1.6");
}

#[test]
fn rolls_back_a_new_version_that_exits_soon_after_becoming_healthy() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "die_soon");
    let data = tmp.path().join("data");
    let port = free_port();
    let _h = supervise(config(&install, &data, port, Duration::from_secs(5)), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    request_shutdown(port).unwrap();
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
        timeout: Duration::from_secs(30),
    };
    let _h = supervise_with(config(&install, &data, port, Duration::from_secs(5)), Box::new(installer));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &setup);
    request_shutdown(port).unwrap();
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    assert!(r.message.contains("installer failed"), "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(30));
    assert!(read(&install.join("fake.json")).contains("0.1.5"), "the half-written file was restored");
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
        timeout: Duration::from_secs(1),
    };
    let _h = supervise_with(config(&install, &data, port, Duration::from_secs(5)), Box::new(installer));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &setup);
    request_shutdown(port).unwrap();
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
    let _h = supervise_with(config(&install, &data, port, Duration::from_secs(10)), Box::new(Unsupported { reason }));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    request_shutdown(port).unwrap();
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
    let cfg = config(&install, &data, port, Duration::from_secs(10));
    assert!(cfg.dev);
    let installer = host_installer(&cfg);
    assert_eq!(installer.backup_path(), None, "a dev host keeps no backup: it never installs");
    let _h = supervise_with(cfg, installer);
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &setup);
    request_shutdown(port).unwrap();
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
    let _h = supervise(config(&install, &data, port, Duration::from_secs(10)), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    std::fs::create_dir_all(request_path(&data).parent().unwrap()).unwrap();
    std::fs::write(request_path(&data), "{garbage").unwrap();
    request_shutdown(port).unwrap();
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
    wait_version(port, "0.1.5", Duration::from_secs(10));
    let mut cfg = config(&install, &data, port, Duration::from_secs(10));
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
    let _h = supervise(config(&install, &data, port, Duration::from_secs(10)), backup);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    assert!(r.message.contains("0.1.6") && r.message.contains("interrupted"), "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

/// Review issue 6: a missing pdtd is restored from the previous version.
#[test]
fn a_missing_pdtd_is_restored_from_the_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let backup = fake_install(&tmp.path().join("backup"), "0.1.5", "healthy");
    let install = tmp.path().join("app");
    std::fs::create_dir_all(&install).unwrap();
    let port = free_port();
    let _h = supervise(config(&install, &tmp.path().join("data"), port, Duration::from_secs(10)), backup);
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

/// Review issue 6: with nothing runnable pdt-host keeps running and starts pdtd once it appears.
#[test]
fn with_nothing_to_run_it_keeps_trying() {
    let tmp = tempfile::tempdir().unwrap();
    let install = tmp.path().join("app");
    std::fs::create_dir_all(&install).unwrap();
    let port = free_port();
    let h =
        supervise(config(&install, &tmp.path().join("data"), port, Duration::from_secs(10)), tmp.path().join("none"));
    std::thread::sleep(Duration::from_secs(1));
    assert!(h.is_running(), "pdt-host did not give up");
    fake_install(&install, "0.1.5", "healthy");
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

#[test]
fn adopts_a_running_pdtd_and_takes_over_when_it_stops() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let port = free_port();
    let mut outside = Outside::start(&install, port);
    wait_version(port, "0.1.5", Duration::from_secs(10));
    let _h =
        supervise(config(&install, &tmp.path().join("data"), port, Duration::from_secs(10)), tmp.path().join("backup"));
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(read(&install.join("env.txt")), "", "the running pdtd was adopted, not replaced");
    request_shutdown(port).unwrap();
    let _ = outside.0.wait();
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

/// Review round 2: a dismissed result is not written again when the new version passes its watch window.
#[test]
fn a_result_dismissed_during_the_health_window_stays_dismissed() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let _h = supervise(config(&install, &data, port, Duration::from_secs(4)), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    request_shutdown(port).unwrap();
    wait_version(port, "0.1.6", Duration::from_secs(30));
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
    let _h = supervise(config(&install, &data, port, Duration::from_secs(5)), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    request_shutdown(port).unwrap();
    wait_version(port, "0.1.6", Duration::from_secs(30));
    std::thread::sleep(Duration::from_secs(1)); // pdt-host's own health poll (every 200 ms) has seen it too
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
        timeout: Duration::from_secs(2),
    };
    let _h = supervise_with(config(&install, &data, port, Duration::from_secs(5)), Box::new(installer));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &setup);
    request_shutdown(port).unwrap();
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
    wait_version(port, "0.1.6", Duration::from_secs(10));
    let _h = supervise(config(&install, &data, port, Duration::from_secs(10)), backup);
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
    wait_version(port, "0.1.6", Duration::from_secs(10));
    let mut cfg = config(&install, &data, port, Duration::from_secs(10));
    cfg.stop_timeout = Duration::from_secs(1);
    let _h = supervise(cfg, backup);
    std::thread::sleep(Duration::from_secs(3));
    assert!(marker_path(&data).exists(), "the marker is kept");
    assert!(!result_path(&data).exists(), "nothing was rolled back yet");
    assert!(read(&install.join("fake.json")).contains("0.1.6"), "the running version's files are untouched");
    wait_version(port, "0.1.6", Duration::from_secs(1));
    drop(outside);
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    wait_version(port, "0.1.5", Duration::from_secs(20));
}
