//! `pdt-host stop` (PDT-489): stops this data folder's pdt-host and pdtd for this login session, the way the tray's
//! "Stop PDT and quit" does. Nothing is unregistered: PDT starts again at the next login, or when the app is opened.
//!
//! pdt-host goes first, so nothing starts pdtd again: macOS boots its launchd job out (KeepAlive would restart it),
//! Linux stops its systemd service (`Restart=always`), Windows (and a pdt-host the tray or a terminal started) kills
//! the process holding the data folder's lock. pdtd runs in its own process group and survives that; it is then
//! asked to shut down, which interrupts its sessions as an update does, so they resume when PDT is back.

use crate::config::HostConfig;
use crate::health::{Probe, probe};
use crate::register::{
    Os, Runner, System, host_running, launchctl, launchd_domain, stop_pdtd, stop_running_host, systemctl,
};
use pdt_core::brand::{HOST_LABEL, HOST_SYSTEMD_UNIT};

/// Stops `cfg`'s pdt-host and pdtd. The port is the one a running pdtd of this data folder listens on.
pub fn stop(cfg: &HostConfig) -> Result<(), String> {
    let mut cfg = cfg.clone();
    cfg.refresh_port(|p| matches!(probe(p), Probe::Ours { .. }));
    stop_with(&cfg, Os::current(), &mut System)
}

pub fn stop_with(cfg: &HostConfig, os: Os, runner: &mut dyn Runner) -> Result<(), String> {
    // Only the stable profile is registered; a dev pdt-host is never launchd's or systemd's.
    if !cfg.dev {
        match os {
            Os::Mac => {
                let domain = launchd_domain(runner)?;
                // Fails when launchd doesn't run it (not registered, or already stopped).
                let _ = runner.status(launchctl(["bootout".to_string(), format!("{domain}/{HOST_LABEL}")]));
            }
            Os::Linux => {
                // Stops a pdt-host systemd runs; pdtd is left (KillMode=process) and stopped below.
                let _ = runner.status(systemctl(["stop", HOST_SYSTEMD_UNIT]));
            }
            Os::Windows | Os::Other => {}
        }
    }
    if host_running(&cfg.data_dir) && !stop_running_host(cfg, os, runner) {
        return Err("pdt-host did not stop".into());
    }
    stop_pdtd(cfg);
    if matches!(probe(cfg.port), Probe::Ours { .. }) {
        return Err(format!("pdtd still answers on port {}", cfg.port));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::register::pid_path;
    use crate::register::tests::{Recorder, fake_pdtd, hold_lock, setup};

    #[test]
    fn mac_boots_the_launchd_job_out_and_nothing_else() {
        let s = setup();
        let mut r = Recorder::default();
        stop_with(&s.cfg, Os::Mac, &mut r).unwrap();
        assert_eq!(r.ran, ["id | -u", "launchctl | bootout | gui/501/dev.personaldevteam.host"]);
        assert!(!s.home.join("Library").exists(), "the login item is left alone");
    }

    #[test]
    fn linux_stops_the_service_without_disabling_it() {
        let s = setup();
        let mut r = Recorder::default();
        stop_with(&s.cfg, Os::Linux, &mut r).unwrap();
        assert_eq!(r.ran, ["systemctl | --user | stop | pdt-host.service"]);
    }

    #[test]
    fn windows_kills_the_host_then_asks_pdtd_to_shut_down() {
        let mut s = setup();
        let (port, requests) = fake_pdtd();
        s.cfg.port = port;
        let _lock = hold_lock(&s.cfg);
        std::fs::write(pid_path(&s.cfg.data_dir), "4242\n").unwrap();
        let mut r = Recorder::default();
        // The recorder kills nothing: the lock stays held, so it reports that pdt-host did not stop.
        let err = stop_with(&s.cfg, Os::Windows, &mut r).unwrap_err();
        assert!(err.contains("pdt-host did not stop"), "{err}");
        assert_eq!(r.ran, ["taskkill | /F | /PID | 4242"]);
        assert!(requests.try_iter().next().is_none(), "pdtd is left while pdt-host could restart it");
    }

    #[test]
    fn pdtd_is_shut_down_once_the_host_is_gone() {
        let mut s = setup();
        let (port, requests) = fake_pdtd();
        s.cfg.port = port;
        let mut r = Recorder::default();
        // The fake keeps answering, so it is reported; what matters is that it was asked.
        let err = stop_with(&s.cfg, Os::Windows, &mut r).unwrap_err();
        assert!(err.contains("pdtd still answers"), "{err}");
        assert!(r.ran.is_empty(), "{:?}", r.ran);
        let seen: Vec<String> = requests.try_iter().collect();
        assert!(seen.iter().any(|l| l.starts_with("POST /api/shutdown ")), "{seen:?}");
    }

    #[test]
    fn a_dev_host_is_never_launchds_or_systemds() {
        let mut s = setup();
        s.cfg.dev = true;
        for os in [Os::Mac, Os::Linux] {
            let mut r = Recorder::default();
            stop_with(&s.cfg, os, &mut r).unwrap();
            assert!(r.ran.is_empty(), "{os:?}: {:?}", r.ran);
        }
    }
}
