//! Settings → This host's "Start PDT at login" (PDT-489): registers or unregisters pdt-host's login item
//! ([`crate::register`]) without stopping the PDT that runs now.
//!
//! On by default; turning it off leaves a marker in the data folder, so neither the tray (which registers pdt-host on
//! a first macOS or AppImage launch) nor an update's installer (which runs `install` again on Windows) registers it
//! again. Turning it on removes the marker and registers it as `install` does.

use crate::config::HostConfig;
use crate::register::{Dirs, Os, RUN_KEY, Runner, Target, install_with, launch_agent_path, systemctl};
use pdt_core::brand::{HOST_AUTOSTART_NAME, HOST_SYSTEMD_UNIT};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The marker that says the user turned starting PDT at login off.
pub fn off_marker(data_dir: &Path) -> PathBuf {
    data_dir.join("host-login-off")
}

/// Whether the user wants pdt-host started at login (the default).
pub fn wanted(data_dir: &Path) -> bool {
    !off_marker(data_dir).exists()
}

/// Why `cfg`'s pdt-host can't be started at login on `os`, or None when it can.
pub fn unsupported(cfg: &HostConfig, os: Os) -> Option<String> {
    if os == Os::Other {
        return Some("PDT starts at login on macOS, Windows and Linux only".into());
    }
    cfg.dev.then(|| "only the stable PDT starts at login".into())
}

/// Whether pdt-host is registered to start at login.
pub fn registered(os: Os, dirs: Dirs, runner: &mut dyn Runner) -> bool {
    match os {
        Os::Mac => dirs.home.is_some_and(|h| launch_agent_path(h).is_file()),
        Os::Windows => {
            let mut reg = Command::new("reg");
            reg.args(["query", RUN_KEY, "/v", HOST_AUTOSTART_NAME]);
            runner.status(reg).unwrap_or(false)
        }
        Os::Linux => runner.status(systemctl(["is-enabled", "--quiet", HOST_SYSTEMD_UNIT])).unwrap_or(false),
        Os::Other => false,
    }
}

/// Turns starting at login on or off, and remembers the choice. The running pdt-host and pdtd keep running either way.
pub fn set(cfg: &HostConfig, target: &Target, on: bool, runner: &mut dyn Runner) -> Result<(), String> {
    if let Some(why) = unsupported(cfg, target.os) {
        return Err(why);
    }
    let marker = off_marker(&cfg.data_dir);
    if on {
        if marker.exists() {
            std::fs::remove_file(&marker).map_err(|e| e.to_string())?;
        }
        return install_with(cfg, target, runner);
    }
    std::fs::create_dir_all(&cfg.data_dir).map_err(|e| e.to_string())?;
    std::fs::write(&marker, b"").map_err(|e| e.to_string())?;
    match target.os {
        // Only the agent file goes: booting it out would stop the pdt-host launchd runs now.
        Os::Mac => match std::fs::remove_file(launch_agent_path(target.home.ok_or("no home folder")?)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        },
        Os::Windows => {
            let mut reg = Command::new("reg");
            reg.args(["delete", RUN_KEY, "/v", HOST_AUTOSTART_NAME, "/f"]);
            // Fails when the value is already gone.
            let _ = runner.status(reg)?;
        }
        Os::Linux => {
            if !runner.status(systemctl(["disable", HOST_SYSTEMD_UNIT]))? {
                return Err(format!("systemctl --user disable {HOST_SYSTEMD_UNIT} failed"));
            }
        }
        Os::Other => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::register::tests::{Recorder, hold_lock, setup, target};
    use crate::register::{bundled_host_path, systemd_unit_path};

    #[test]
    fn mac_off_removes_only_the_agent_file_and_on_writes_it_again() {
        let s = setup();
        std::fs::write(bundled_host_path(&s.cfg.install_dir), "bundled host").unwrap();
        let _lock = hold_lock(&s.cfg);
        let plist = launch_agent_path(&s.home);
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, "x").unwrap();
        let mut r = Recorder::default();
        assert!(registered(Os::Mac, s.dirs(), &mut r));

        set(&s.cfg, &target(&s, Os::Mac), false, &mut r).unwrap();
        assert!(r.ran.is_empty(), "nothing is booted out: PDT keeps running: {:?}", r.ran);
        assert!(!plist.exists() && !wanted(&s.cfg.data_dir));
        assert!(!registered(Os::Mac, s.dirs(), &mut r));
        // The tray's first-launch registration and `install` leave it off.
        install_with(&s.cfg, &target(&s, Os::Mac), &mut r).unwrap();
        assert!(!plist.exists());

        set(&s.cfg, &target(&s, Os::Mac), true, &mut r).unwrap();
        assert!(plist.is_file() && wanted(&s.cfg.data_dir));
        assert!(r.ran.is_empty(), "a running pdt-host is left alone: {:?}", r.ran);
    }

    #[test]
    fn windows_switch_deletes_and_adds_the_run_value() {
        let s = setup();
        let _lock = hold_lock(&s.cfg);
        let mut r = Recorder::default();
        set(&s.cfg, &target(&s, Os::Windows), false, &mut r).unwrap();
        assert_eq!(
            r.ran,
            [r"reg | delete | HKCU\Software\Microsoft\Windows\CurrentVersion\Run | /v | Personal Dev Team host | /f"]
        );

        // An update's installer runs `install`: it is not registered again.
        let mut r = Recorder::default();
        install_with(&s.cfg, &target(&s, Os::Windows), &mut r).unwrap();
        assert!(r.ran.is_empty(), "{:?}", r.ran);

        let mut r = Recorder::default();
        set(&s.cfg, &target(&s, Os::Windows), true, &mut r).unwrap();
        assert_eq!(r.ran.len(), 1);
        assert!(r.ran[0].starts_with(
            r"reg | add | HKCU\Software\Microsoft\Windows\CurrentVersion\Run | /v | Personal Dev Team host"
        ));

        let mut r = Recorder { statuses: [false].into(), ..Default::default() };
        assert!(!registered(Os::Windows, s.dirs(), &mut r));
        assert_eq!(
            r.ran,
            [r"reg | query | HKCU\Software\Microsoft\Windows\CurrentVersion\Run | /v | Personal Dev Team host"]
        );
    }

    #[test]
    fn linux_switch_disables_and_enables_the_service() {
        let s = setup();
        std::fs::write(bundled_host_path(&s.cfg.install_dir), "host").unwrap();
        let _lock = hold_lock(&s.cfg);
        let mut r = Recorder::default();
        set(&s.cfg, &target(&s, Os::Linux), false, &mut r).unwrap();
        assert_eq!(r.ran, ["systemctl | --user | disable | pdt-host.service"]);

        let mut r = Recorder::default();
        set(&s.cfg, &target(&s, Os::Linux), true, &mut r).unwrap();
        assert_eq!(r.ran, ["systemctl | --user | daemon-reload", "systemctl | --user | enable | pdt-host.service"]);
        assert!(systemd_unit_path(&s.config).is_file());

        let mut r = Recorder::default();
        assert!(registered(Os::Linux, s.dirs(), &mut r));
        assert_eq!(r.ran, ["systemctl | --user | is-enabled | --quiet | pdt-host.service"]);
    }

    #[test]
    fn linux_install_while_off_starts_the_service_without_enabling_it() {
        let s = setup();
        std::fs::write(bundled_host_path(&s.cfg.install_dir), "host").unwrap();
        std::fs::create_dir_all(&s.cfg.data_dir).unwrap();
        std::fs::write(off_marker(&s.cfg.data_dir), "").unwrap();
        let mut r = Recorder::default();
        install_with(&s.cfg, &target(&s, Os::Linux), &mut r).unwrap();
        assert_eq!(r.ran, ["systemctl | --user | daemon-reload", "systemctl | --user | start | pdt-host.service"]);
    }

    #[test]
    fn the_dev_profile_and_other_platforms_cannot_switch() {
        let mut s = setup();
        assert!(unsupported(&s.cfg, Os::Other).is_some());
        assert!(set(&s.cfg, &target(&s, Os::Other), false, &mut Recorder::default()).is_err());
        s.cfg.dev = true;
        assert!(unsupported(&s.cfg, Os::Mac).is_some());
        let mut r = Recorder::default();
        assert!(set(&s.cfg, &target(&s, Os::Mac), false, &mut r).is_err());
        assert!(wanted(&s.cfg.data_dir) && r.ran.is_empty());
    }
}
