use pdt_core::handoff::{UpdateOutcome, UpdateRequest, UpdateResult, request_path, result_path};
use pdt_host::config::HostConfig;
use pdt_host::health::{Probe, probe, request_shutdown};
use pdt_host::install::{CopyDirInstaller, Installer, Unsupported};
use pdt_host::supervise::{Backoff, run};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A folder holding the fake pdtd (as `pdtd[.exe]`) and its `fake.json`.
fn fake_install(dir: &Path, version: &str, mode: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let exe = dir.join(format!("pdtd{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_fake-pdtd"), &exe).unwrap();
    std::fs::write(dir.join("fake.json"), serde_json::json!({"version": version, "mode": mode}).to_string()).unwrap();
    dir.to_path_buf()
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
    }
}

struct Running {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
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

fn read_result(data: &Path) -> UpdateResult {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(b) = std::fs::read(result_path(data))
            && let Ok(r) = serde_json::from_slice(&b)
        {
            return r;
        }
        assert!(Instant::now() < end, "no update result was written");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn request(data: &Path, version: &str, asset: &Path) {
    let req = UpdateRequest {
        version: version.into(),
        tag: format!("v{version}"),
        asset: asset.to_path_buf(),
        from_version: "0.1.5".into(),
        requested_at: 1,
    };
    let p = request_path(data);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, serde_json::to_vec(&req).unwrap()).unwrap();
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
    assert_eq!(std::fs::read_to_string(install.join("env.txt")).unwrap(), "1", "PDT_SUPERVISED is set");
    // A clean exit (not an update) is restarted too.
    request_shutdown(port).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

#[test]
fn installs_an_update_and_reports_it() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let _h = supervise(config(&install, &data, port, Duration::from_secs(10)), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    request_shutdown(port).unwrap();
    wait_version(port, "0.1.6", Duration::from_secs(30));
    let r = read_result(&data);
    assert_eq!((r.outcome, r.to_version.as_str()), (UpdateOutcome::Installed, "0.1.6"));
    assert!(!request_path(&data).exists(), "the request was taken");
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

#[test]
fn adopts_a_running_pdtd_and_takes_over_when_it_stops() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let port = free_port();
    let mut cmd = std::process::Command::new(install.join(format!("pdtd{}", std::env::consts::EXE_SUFFIX)));
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    let mut outside = cmd.env("PDT_PORT", port.to_string()).spawn().unwrap();
    wait_version(port, "0.1.5", Duration::from_secs(10));
    let _h =
        supervise(config(&install, &tmp.path().join("data"), port, Duration::from_secs(10)), tmp.path().join("backup"));
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        std::fs::read_to_string(install.join("env.txt")).unwrap(),
        "",
        "the running pdtd was adopted, not replaced"
    );
    request_shutdown(port).unwrap();
    let _ = outside.wait();
    wait_version(port, "0.1.5", Duration::from_secs(20));
}
