//! Starting pdt-host at login (spec §3.1): a per-user launchd LaunchAgent on macOS (RunAtLoad, KeepAlive), and a
//! per-user `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value on Windows (decision 14; no admin needed).
//! pdt-host runs from a copy in the data folder, so installers can replace the install folder.
//!
//! Every command goes through a [`Runner`], so tests check the exact command lines without registering anything.

use crate::config::HostConfig;
use pdt_core::brand::{HOST_AUTOSTART_NAME, HOST_BIN, HOST_LABEL};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn installed_host_path(data_dir: &Path) -> PathBuf {
    data_dir.join("host").join(format!("{HOST_BIN}{}", std::env::consts::EXE_SUFFIX))
}

pub fn launch_agent_path(home: &Path) -> PathBuf {
    home.join("Library").join("LaunchAgents").join(format!("{HOST_LABEL}.plist"))
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn launchd_plist(program: &Path, install_dir: &Path, log: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{program}</string>
    <string>run</string>
    <string>--install-dir</string>
    <string>{dir}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        label = xml(HOST_LABEL),
        program = xml(&program.to_string_lossy()),
        dir = xml(&install_dir.to_string_lossy()),
        log = xml(&log.to_string_lossy()),
    )
}

pub fn run_key_command(program: &Path, install_dir: &Path) -> String {
    format!("\"{}\" run --install-dir \"{}\"", program.display(), install_dir.display())
}

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Other
        }
    }
}

/// Runs the commands registration needs. [`System`] runs them; tests record them.
pub trait Runner {
    /// Runs `cmd` to completion: whether it succeeded.
    fn status(&mut self, cmd: Command) -> Result<bool, String>;
    /// Runs `cmd` to completion: its standard output.
    fn stdout(&mut self, cmd: Command) -> Result<String, String>;
    /// Starts `cmd` and does not wait for it.
    fn spawn(&mut self, cmd: Command) -> Result<(), String>;
}

pub struct System;

impl Runner for System {
    fn status(&mut self, mut cmd: Command) -> Result<bool, String> {
        cmd.status().map(|s| s.success()).map_err(|e| format!("running {:?}: {e}", cmd.get_program()))
    }
    fn stdout(&mut self, mut cmd: Command) -> Result<String, String> {
        let out = cmd.output().map_err(|e| format!("running {:?}: {e}", cmd.get_program()))?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    fn spawn(&mut self, mut cmd: Command) -> Result<(), String> {
        cmd.spawn().map(|_| ()).map_err(|e| format!("starting {:?}: {e}", cmd.get_program()))
    }
}

/// Where `install` registers: the platform, the executable to copy and the home folder (macOS).
pub struct Target<'a> {
    pub os: Os,
    pub exe: &'a Path,
    pub home: Option<&'a Path>,
}

/// Whether another pdt-host holds the data folder's lock.
fn host_running(data_dir: &Path) -> bool {
    let path = data_dir.join(format!("{HOST_BIN}.lock"));
    let Ok(f) = std::fs::File::options().write(true).open(path) else { return false };
    matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

fn launchd_domain(runner: &mut dyn Runner) -> Result<String, String> {
    let mut id = Command::new("id");
    id.arg("-u");
    let uid = runner.stdout(id)?.trim().to_string();
    if uid.is_empty() {
        return Err("`id -u` printed no user id".into());
    }
    Ok(format!("gui/{uid}"))
}

fn launchctl<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut cmd = Command::new("launchctl");
    cmd.args(args.into_iter().map(Into::into));
    cmd
}

/// Copies this executable into the data folder (unless a running pdt-host owns that copy; it replaces itself after
/// updates), registers the copy to start at login, and starts it if no pdt-host is running.
pub fn install(cfg: &HostConfig) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let home = std::env::home_dir();
    install_with(cfg, &Target { os: Os::current(), exe: &exe, home: home.as_deref() }, &mut System)
}

pub fn install_with(cfg: &HostConfig, target: &Target, runner: &mut dyn Runner) -> Result<(), String> {
    if target.os == Os::Other {
        return Err("registration is supported on macOS and Windows only".into());
    }
    let program = installed_host_path(&cfg.data_dir);
    let running = host_running(&cfg.data_dir);
    // A running copy is locked (Windows) and replaces itself; a missing one is copied even then, so the registration
    // never points at nothing.
    if !running || !program.is_file() {
        std::fs::create_dir_all(program.parent().expect("host dir")).map_err(|e| e.to_string())?;
        if target.exe != program {
            std::fs::copy(target.exe, &program)
                .map_err(|e| format!("copying {} to {}: {e}", target.exe.display(), program.display()))?;
        }
    }
    match target.os {
        Os::Mac => {
            let home = target.home.ok_or("no home folder")?;
            let plist = launch_agent_path(home);
            let log = cfg.data_dir.join("logs").join(format!("{HOST_BIN}.launchd.log"));
            let _ = std::fs::create_dir_all(log.parent().expect("logs"));
            std::fs::create_dir_all(plist.parent().expect("LaunchAgents")).map_err(|e| e.to_string())?;
            std::fs::write(&plist, launchd_plist(&program, &cfg.install_dir, &log)).map_err(|e| e.to_string())?;
            if !running {
                let domain = launchd_domain(runner)?;
                // A stale registration (e.g. from an older install) fails bootstrap; it may not exist.
                let _ = runner.status(launchctl(["bootout".to_string(), format!("{domain}/{HOST_LABEL}")]));
                let bootstrap = launchctl([OsString::from("bootstrap"), domain.into(), plist.into_os_string()]);
                if !runner.status(bootstrap)? {
                    return Err("launchctl bootstrap failed".into());
                }
            }
        }
        Os::Windows => {
            let mut reg = Command::new("reg");
            reg.args(["add", RUN_KEY, "/v", HOST_AUTOSTART_NAME, "/t", "REG_SZ", "/d"])
                .arg(run_key_command(&program, &cfg.install_dir))
                .arg("/f");
            if !runner.status(reg)? {
                return Err("reg add failed".into());
            }
            if !running {
                let mut cmd = Command::new(&program);
                cmd.args(["run", "--install-dir"]).arg(&cfg.install_dir);
                // Started the way the Run key starts it: with none of this process's PDT_* variables.
                for (k, _) in std::env::vars_os() {
                    if k.to_string_lossy().starts_with("PDT_") {
                        cmd.env_remove(k);
                    }
                }
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x0800_0000 | 0x0000_0200);
                }
                runner.spawn(cmd)?;
            }
        }
        Os::Other => unreachable!(),
    }
    Ok(())
}

/// Removes the login registration. A running pdt-host keeps running until logout.
pub fn uninstall() -> Result<(), String> {
    uninstall_with(Os::current(), std::env::home_dir().as_deref(), &mut System)
}

pub fn uninstall_with(os: Os, home: Option<&Path>, runner: &mut dyn Runner) -> Result<(), String> {
    match os {
        Os::Mac => {
            let home = home.ok_or("no home folder")?;
            let domain = launchd_domain(runner)?;
            let _ = runner.status(launchctl(["bootout".to_string(), format!("{domain}/{HOST_LABEL}")]));
            let _ = std::fs::remove_file(launch_agent_path(home));
        }
        Os::Windows => {
            let mut reg = Command::new("reg");
            reg.args(["delete", RUN_KEY, "/v", HOST_AUTOSTART_NAME, "/f"]);
            let _ = runner.status(reg);
        }
        Os::Other => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn launch_agent_runs_the_data_folder_copy_at_load_and_keeps_it_alive() {
        let p = launchd_plist(
            Path::new("/Users/me/Library/Application Support/pdt/host/pdt-host"),
            Path::new("/Users/me/Applications/A & B.app/Contents/MacOS"),
            Path::new("/tmp/h.log"),
        );
        assert!(p.contains("<key>Label</key><string>dev.personaldevteam.host</string>"));
        assert!(p.contains("<string>/Users/me/Library/Application Support/pdt/host/pdt-host</string>"));
        assert!(p.contains("<string>/Users/me/Applications/A &amp; B.app/Contents/MacOS</string>"));
        assert!(p.contains("<key>RunAtLoad</key><true/>") && p.contains("<key>KeepAlive</key><true/>"));
        assert_eq!(
            launch_agent_path(Path::new("/Users/me")),
            Path::new("/Users/me/Library/LaunchAgents/dev.personaldevteam.host.plist")
        );
    }

    #[test]
    fn run_key_quotes_paths() {
        let c = run_key_command(
            Path::new(r"C:\Users\me\AppData\Local\pdt\host\pdt-host.exe"),
            Path::new(r"C:\Program Files\P D T"),
        );
        assert_eq!(
            c,
            r#""C:\Users\me\AppData\Local\pdt\host\pdt-host.exe" run --install-dir "C:\Program Files\P D T""#
        );
        assert!(installed_host_path(Path::new("d")).starts_with(Path::new("d").join("host")));
    }

    /// Records every command instead of running it.
    #[derive(Default)]
    struct Recorder {
        ran: Vec<String>,
        spawned: Vec<Command>,
        /// Results for `status`, in order (default: success).
        statuses: VecDeque<bool>,
    }

    fn line(cmd: &Command) -> String {
        std::iter::once(cmd.get_program())
            .chain(cmd.get_args())
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" | ")
    }

    impl Runner for Recorder {
        fn status(&mut self, cmd: Command) -> Result<bool, String> {
            self.ran.push(line(&cmd));
            Ok(self.statuses.pop_front().unwrap_or(true))
        }
        fn stdout(&mut self, cmd: Command) -> Result<String, String> {
            self.ran.push(line(&cmd));
            Ok("501\n".into())
        }
        fn spawn(&mut self, cmd: Command) -> Result<(), String> {
            self.ran.push(format!("spawn {}", line(&cmd)));
            self.spawned.push(cmd);
            Ok(())
        }
    }

    struct Setup {
        _tmp: tempfile::TempDir,
        cfg: HostConfig,
        exe: PathBuf,
        home: PathBuf,
    }

    fn setup() -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("app").join("pdt-host-built");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "new host").unwrap();
        let home = tmp.path().join("home");
        let cfg = HostConfig::from_lookup(
            |k| (k == "PDT_DATA_DIR").then(|| tmp.path().join("data").to_string_lossy().into_owned()),
            None,
            tmp.path().join("app"),
        )
        .unwrap();
        Setup { _tmp: tmp, cfg, exe, home }
    }

    fn target(s: &Setup, os: Os) -> Target<'_> {
        Target { os, exe: &s.exe, home: Some(&s.home) }
    }

    /// Holds the data folder's pdt-host lock, as a running pdt-host does.
    fn hold_lock(cfg: &HostConfig) -> std::fs::File {
        std::fs::create_dir_all(&cfg.data_dir).unwrap();
        let f = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(cfg.data_dir.join("pdt-host.lock"))
            .unwrap();
        f.lock().unwrap();
        f
    }

    #[test]
    fn windows_install_copies_registers_the_copy_and_starts_it_clean() {
        let s = setup();
        let mut r = Recorder::default();
        install_with(&s.cfg, &target(&s, Os::Windows), &mut r).unwrap();
        let program = installed_host_path(&s.cfg.data_dir);
        assert_eq!(std::fs::read_to_string(&program).unwrap(), "new host");
        let command = run_key_command(&program, &s.cfg.install_dir);
        let spawn = format!("spawn {} | run | --install-dir | {}", program.display(), s.cfg.install_dir.display());
        assert_eq!(
            r.ran,
            [
                format!(
                    r"reg | add | HKCU\Software\Microsoft\Windows\CurrentVersion\Run | /v | Personal Dev Team host | /t | REG_SZ | /d | {command} | /f"
                ),
                spawn,
            ]
        );
        // Every inherited PDT_* variable is removed from the started host.
        let env: Vec<_> = r.spawned[0].get_envs().collect();
        for (k, _) in std::env::vars_os().filter(|(k, _)| k.to_string_lossy().starts_with("PDT_")) {
            assert!(env.contains(&(k.as_os_str(), None)), "{k:?} is passed on");
        }
    }

    #[test]
    fn windows_install_while_a_host_runs_only_refreshes_the_registration() {
        let s = setup();
        let program = installed_host_path(&s.cfg.data_dir);
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, "running host").unwrap();
        let _lock = hold_lock(&s.cfg);
        let mut r = Recorder::default();
        install_with(&s.cfg, &target(&s, Os::Windows), &mut r).unwrap();
        assert_eq!(std::fs::read_to_string(&program).unwrap(), "running host", "a running copy replaces itself");
        assert_eq!(r.ran.len(), 1);
        assert!(r.ran[0].starts_with("reg | add | "), "{:?}", r.ran);
    }

    #[test]
    fn a_missing_copy_is_made_even_while_a_host_runs_elsewhere() {
        let s = setup();
        let _lock = hold_lock(&s.cfg);
        install_with(&s.cfg, &target(&s, Os::Windows), &mut Recorder::default()).unwrap();
        assert_eq!(std::fs::read_to_string(installed_host_path(&s.cfg.data_dir)).unwrap(), "new host");
    }

    #[test]
    fn windows_install_reports_a_failed_reg_add_and_starts_nothing() {
        let s = setup();
        let mut r = Recorder { statuses: [false].into(), ..Default::default() };
        let err = install_with(&s.cfg, &target(&s, Os::Windows), &mut r).unwrap_err();
        assert!(err.contains("reg add failed"), "{err}");
        assert!(r.spawned.is_empty());
    }

    #[test]
    fn mac_install_writes_the_agent_and_bootstraps_it() {
        let s = setup();
        let mut r = Recorder::default();
        install_with(&s.cfg, &target(&s, Os::Mac), &mut r).unwrap();
        let program = installed_host_path(&s.cfg.data_dir);
        let plist = launch_agent_path(&s.home);
        let text = std::fs::read_to_string(&plist).unwrap();
        assert!(text.contains(&format!("<string>{}</string>", program.display())), "{text}");
        assert!(text.contains(&format!("<string>{}</string>", s.cfg.install_dir.display())), "{text}");
        assert!(text.contains("pdt-host.launchd.log"), "{text}");
        assert_eq!(
            r.ran,
            [
                "id | -u".to_string(),
                "launchctl | bootout | gui/501/dev.personaldevteam.host".to_string(),
                format!("launchctl | bootstrap | gui/501 | {}", plist.display()),
            ]
        );
        assert!(r.spawned.is_empty());
    }

    #[test]
    fn mac_install_while_a_host_runs_rewrites_the_agent_without_launchctl() {
        let s = setup();
        let _lock = hold_lock(&s.cfg);
        let mut r = Recorder::default();
        install_with(&s.cfg, &target(&s, Os::Mac), &mut r).unwrap();
        assert!(launch_agent_path(&s.home).is_file());
        assert!(r.ran.is_empty(), "{:?}", r.ran);
    }

    #[test]
    fn mac_install_reports_a_failed_bootstrap() {
        let s = setup();
        // bootout may fail (nothing registered); bootstrap must not.
        let mut r = Recorder { statuses: [false, false].into(), ..Default::default() };
        let err = install_with(&s.cfg, &target(&s, Os::Mac), &mut r).unwrap_err();
        assert!(err.contains("bootstrap failed"), "{err}");
    }

    #[test]
    fn other_platforms_are_refused() {
        let s = setup();
        let mut r = Recorder::default();
        assert!(install_with(&s.cfg, &target(&s, Os::Other), &mut r).is_err());
        assert!(r.ran.is_empty());
        assert!(!installed_host_path(&s.cfg.data_dir).exists());
    }

    #[test]
    fn uninstall_removes_the_registration() {
        let s = setup();
        let mut r = Recorder::default();
        uninstall_with(Os::Windows, Some(&s.home), &mut r).unwrap();
        assert_eq!(
            r.ran,
            [r"reg | delete | HKCU\Software\Microsoft\Windows\CurrentVersion\Run | /v | Personal Dev Team host | /f"]
        );

        let plist = launch_agent_path(&s.home);
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, "x").unwrap();
        let mut r = Recorder::default();
        uninstall_with(Os::Mac, Some(&s.home), &mut r).unwrap();
        assert_eq!(r.ran, ["id | -u", "launchctl | bootout | gui/501/dev.personaldevteam.host"]);
        assert!(!plist.exists());
    }
}
