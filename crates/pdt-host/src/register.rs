//! Starting pdt-host at login: a per-user launchd LaunchAgent on macOS (RunAtLoad, KeepAlive), and a
//! per-user `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value on Windows (no admin needed),
//! and a systemd user service on Linux (`~/.config/systemd/user/pdt-host.service`, enabled; with linger it starts at
//! boot, PDT-197).
//! On Windows pdt-host runs from a copy in the data folder, so the installer can replace the install folder while it
//! runs. On macOS it runs from the app bundle: a bundle can be swapped while its programs run, and a login item whose
//! program is rewritten after every update looks like malware hiding in a persistence item (PDT-40, SentinelOne).
//! On Linux it runs from the install folder (the tarball's, or an AppImage's payload in `<data dir>/app`), which an
//! update swaps as a whole.
//!
//! The tray app's own login item is in [`crate::tray_login`]; uninstalling removes it too.
//!
//! Every command goes through a [`Runner`], so tests check the exact command lines without registering anything.

use crate::config::HostConfig;
use crate::health::{Probe, probe, request_shutdown};
use pdt_core::brand::{
    APP_BUNDLE_ID, DAEMON_BIN, HOST_AUTOSTART_NAME, HOST_BIN, HOST_LABEL, HOST_SYSTEMD_UNIT, PRODUCT_NAME,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// The Windows copy of pdt-host in the data folder (also where older macOS versions kept one).
pub fn installed_host_path(data_dir: &Path) -> PathBuf {
    data_dir.join("host").join(format!("{HOST_BIN}{}", std::env::consts::EXE_SUFFIX))
}

/// The pdt-host in the install folder (the app bundle's on macOS).
pub fn bundled_host_path(install_dir: &Path) -> PathBuf {
    install_dir.join(format!("{HOST_BIN}{}", std::env::consts::EXE_SUFFIX))
}

/// The program the login registration starts: the bundle's (macOS) or install folder's (Linux) pdt-host, the data
/// folder copy on Windows.
pub fn registered_program(os: Os, cfg: &HostConfig) -> PathBuf {
    if os == Os::Windows { installed_host_path(&cfg.data_dir) } else { bundled_host_path(&cfg.install_dir) }
}

/// Windows: the data folder copy a `run` started from the install folder (by the tray while the copy was being
/// replaced, or by hand) hands over to, so pdt-host never runs from the folder an update replaces (PDT-213). `None`
/// when it should supervise itself: another OS, the dev profile, no copy yet (first run), or it is the copy.
pub fn copy_to_hand_over_to(os: Os, cfg: &HostConfig, exe: &Path) -> Option<PathBuf> {
    if os != Os::Windows || cfg.dev {
        return None;
    }
    let same = |a: &Path, b: &Path| match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    let copy = installed_host_path(&cfg.data_dir);
    (copy.is_file() && same(exe, &bundled_host_path(&cfg.install_dir)) && !same(exe, &copy)).then_some(copy)
}

/// The arguments the data folder copy is started with by a `run` handing over to it: this `run`'s, with the install
/// folder it resolved always named (without `--install-dir`, the copy would take its own folder for it).
pub fn hand_over_args(args: &[String], install_dir: &Path) -> Vec<OsString> {
    let mut out: Vec<OsString> = vec!["run".into()];
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        if a == "--install-dir" {
            rest.next();
        } else {
            out.push(a.into());
        }
    }
    out.extend(["--install-dir".into(), install_dir.as_os_str().to_owned()]);
    out
}

pub fn launch_agent_path(home: &Path) -> PathBuf {
    home.join("Library").join("LaunchAgents").join(format!("{HOST_LABEL}.plist"))
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// `AssociatedBundleIdentifiers` makes the app bundle the responsible code for pdt-host and the pdtd it starts: macOS
/// asks for (and lists) the Local Network permission as the app's, with its Info.plist's usage description (Apple's
/// TN3179; PDT-156). Without it a LaunchAgent is a standalone program that macOS silently denies the local network.
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
  <key>AssociatedBundleIdentifiers</key>
  <array>
    <string>{app}</string>
  </array>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        label = xml(HOST_LABEL),
        app = xml(APP_BUNDLE_ID),
        program = xml(&program.to_string_lossy()),
        dir = xml(&install_dir.to_string_lossy()),
        log = xml(&log.to_string_lossy()),
    )
}

pub fn run_key_command(program: &Path, install_dir: &Path) -> String {
    // A trailing backslash would escape the closing quote (`"C:\dir\"`); a drive root becomes `C:\.`.
    let dir = install_dir.display().to_string();
    let mut dir = dir.trim_end_matches(['\\', '/']).to_string();
    if dir.is_empty() || dir.ends_with(':') {
        dir.push_str(r"\.");
    }
    format!("\"{}\" run --install-dir \"{dir}\"", program.display())
}

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// The systemd unit runs pdt-host with this variable set to `systemd`: after an update that changes pdt-host it then
/// just exits and systemd starts the new one, as launchd does on macOS.
pub const SERVICE_ENV: &str = "PDT_HOST_SERVICE";

/// pdt-host's exit status when another pdt-host already supervises the data folder. The unit does not restart it
/// then (`RestartPreventExitStatus`), so it never loops against a pdt-host the tray started.
pub const ALREADY_RUNNING_EXIT: i32 = 3;

/// Where systemd reads the user's own units: `<config dir>/systemd/user/pdt-host.service` (`~/.config/…`).
pub fn systemd_unit_path(config_dir: &Path) -> PathBuf {
    config_dir.join("systemd").join("user").join(HOST_SYSTEMD_UNIT)
}

/// One word of an `ExecStart` line: quoted, with `\` and `"` escaped, and `%` (specifiers) and `$` (variables)
/// doubled so systemd takes them literally.
fn systemd_word(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$");
    format!("\"{escaped}\"")
}

/// The user service: started with the user's systemd (at login, or at boot with linger), restarted whenever it exits.
/// `KillMode=process`: stopping or restarting pdt-host leaves pdtd (and the agents it runs) running, and the next
/// pdt-host adopts it, as on macOS.
pub fn systemd_unit(program: &Path, install_dir: &Path) -> String {
    format!(
        "[Unit]
Description={PRODUCT_NAME} host ({HOST_BIN})

[Service]
Type=simple
ExecStart={program} run --install-dir {dir}
Environment={SERVICE_ENV}=systemd
Restart=always
RestartSec=2
RestartPreventExitStatus={ALREADY_RUNNING_EXIT}
KillMode=process

[Install]
WantedBy=default.target
",
        program = systemd_word(&program.to_string_lossy()),
        dir = systemd_word(&install_dir.to_string_lossy()),
    )
}

fn systemctl<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user").args(args.into_iter().map(Into::into));
    cmd
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "linux") {
            Os::Linux
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

/// Runs commands for real. Ones it waits for (reg, taskkill) get no console window (PDT-60).
pub struct System;

impl Runner for System {
    fn status(&mut self, mut cmd: Command) -> Result<bool, String> {
        crate::process::no_window(&mut cmd)
            .status()
            .map(|s| s.success())
            .map_err(|e| format!("running {:?}: {e}", cmd.get_program()))
    }
    fn stdout(&mut self, mut cmd: Command) -> Result<String, String> {
        let out = crate::process::no_window(&mut cmd)
            .output()
            .map_err(|e| format!("running {:?}: {e}", cmd.get_program()))?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    fn spawn(&mut self, mut cmd: Command) -> Result<(), String> {
        cmd.spawn().map(|_| ()).map_err(|e| format!("starting {:?}: {e}", cmd.get_program()))
    }
}

/// Where `install` registers: the platform, the executable to copy, the home folder (macOS) and the config folder
/// (Linux: `$XDG_CONFIG_HOME`, `~/.config`).
pub struct Target<'a> {
    pub os: Os,
    pub exe: &'a Path,
    pub home: Option<&'a Path>,
    pub config: Option<&'a Path>,
    /// macOS: take over from a pdt-host that is already running (another copy of the app, PDT-51): its launchd job is
    /// booted out, and it is stopped if it still runs, before this one is bootstrapped. pdtd keeps running (its own
    /// process group); the new pdt-host adopts it.
    pub replace: bool,
}

/// Whether another process holds the lock file at `path`.
fn locked(path: &Path) -> bool {
    let Ok(f) = std::fs::File::options().write(true).open(path) else { return false };
    matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// Whether another pdt-host holds the data folder's lock.
pub fn host_running(data_dir: &Path) -> bool {
    locked(&data_dir.join(format!("{HOST_BIN}.lock")))
}

/// The process id of the pdt-host that holds the data folder's lock (written by `run` once it has the lock; the lock
/// file itself cannot be read on Windows while it is held).
pub fn pid_path(data_dir: &Path) -> PathBuf {
    data_dir.join(format!("{HOST_BIN}.pid"))
}

pub fn write_pid(data_dir: &Path) -> std::io::Result<()> {
    std::fs::write(pid_path(data_dir), std::process::id().to_string())
}

/// The program the running pdt-host was started from (written with the pid), so the tray can tell which copy of the
/// app runs (PDT-51). A separate file: older versions parse the pid file as a bare number.
pub fn program_path(data_dir: &Path) -> PathBuf {
    data_dir.join(format!("{HOST_BIN}.program"))
}

pub fn write_program(data_dir: &Path, exe: &Path) -> std::io::Result<()> {
    std::fs::write(program_path(data_dir), exe.to_string_lossy().as_bytes())
}

/// The program of the pdt-host running for `data_dir`, when one runs and has said (older versions do not).
pub fn running_program(data_dir: &Path) -> Option<PathBuf> {
    if !host_running(data_dir) {
        return None;
    }
    let text = std::fs::read_to_string(program_path(data_dir)).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| PathBuf::from(text))
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

/// Registers pdt-host to start at login and starts it if no pdt-host is running. Windows registers a copy in the data
/// folder (made unless a running pdt-host owns it; that one replaces itself after updates); macOS registers the bundle's
/// own pdt-host and removes the copy older versions made.
pub fn install(cfg: &HostConfig, replace: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (home, config) = (std::env::home_dir(), dirs::config_dir());
    let target = Target { os: Os::current(), exe: &exe, home: home.as_deref(), config: config.as_deref(), replace };
    install_with(cfg, &target, &mut System)
}

pub fn install_with(cfg: &HostConfig, target: &Target, runner: &mut dyn Runner) -> Result<(), String> {
    if target.os == Os::Other {
        return Err("registration is supported on macOS, Windows and Linux only".into());
    }
    if cfg.dev {
        return Err("only the stable profile is started at login; unset PDT_PROFILE to register pdt-host".into());
    }
    let program = registered_program(target.os, cfg);
    let running = host_running(&cfg.data_dir);
    match target.os {
        Os::Mac => {
            if !program.is_file() {
                return Err(format!("{} does not exist", program.display()));
            }
            // A login item naming a translocated or read-only bundle breaks once that folder goes away, and that
            // pdt-host could never install an update (PDT-51).
            if let Some(reason) = crate::placement::bundle_problem(&cfg.install_dir) {
                return Err(format!("not starting this copy at login: {reason}"));
            }
            let home = target.home.ok_or("no home folder")?;
            let plist = launch_agent_path(home);
            write_launch_agent(cfg, &plist)?;
            if !running || target.replace {
                let domain = launchd_domain(runner)?;
                // A stale registration (e.g. from an older install) fails bootstrap; it may not exist. Booting it out
                // also stops the pdt-host launchd runs for it.
                let _ = runner.status(launchctl(["bootout".to_string(), format!("{domain}/{HOST_LABEL}")]));
                if running && !stop_running_host(cfg, target.os, runner) {
                    return Err("the running pdt-host did not stop".into());
                }
                let bootstrap = launchctl([OsString::from("bootstrap"), domain.into(), plist.into_os_string()]);
                if !runner.status(bootstrap)? {
                    return Err("launchctl bootstrap failed".into());
                }
                // launchd now runs the bundle's pdt-host; nothing uses an older version's copy any more.
                remove_old_copy(&cfg.data_dir);
            }
        }
        Os::Windows => {
            // A running copy is locked and replaces itself; a missing one is copied even then, so the registration
            // never points at nothing.
            if !running || !program.is_file() {
                std::fs::create_dir_all(program.parent().expect("host dir")).map_err(|e| e.to_string())?;
                if target.exe != program {
                    std::fs::copy(target.exe, &program)
                        .map_err(|e| format!("copying {} to {}: {e}", target.exe.display(), program.display()))?;
                }
            }
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
                crate::process::detached_no_window(&mut cmd);
                runner.spawn(cmd)?;
            }
        }
        Os::Linux => {
            if !program.is_file() {
                return Err(format!("{} does not exist", program.display()));
            }
            let unit = systemd_unit_path(target.config.ok_or("no config folder")?);
            std::fs::create_dir_all(unit.parent().expect("systemd/user")).map_err(|e| e.to_string())?;
            std::fs::write(&unit, systemd_unit(&program, &cfg.install_dir)).map_err(|e| e.to_string())?;
            if !runner.status(systemctl(["daemon-reload"]))? {
                return Err("systemctl --user daemon-reload failed (is there a systemd user instance? `loginctl \
                            enable-linger` starts one at boot)"
                    .into());
            }
            if running && target.replace {
                // Stopping the service stops a pdt-host systemd runs; one the tray started is killed.
                let _ = runner.status(systemctl(["stop", HOST_SYSTEMD_UNIT]));
                if !stop_running_host(cfg, target.os, runner) {
                    return Err("the running pdt-host did not stop".into());
                }
            }
            // A pdt-host that runs already (the service's, or one the tray started) keeps running: the service is
            // only enabled, for the next boot or login.
            let enable = if running && !target.replace {
                systemctl(["enable", HOST_SYSTEMD_UNIT])
            } else {
                systemctl(["enable", "--now", HOST_SYSTEMD_UNIT])
            };
            if !runner.status(enable)? {
                return Err(format!("systemctl --user enable {HOST_SYSTEMD_UNIT} failed"));
            }
        }
        Os::Other => unreachable!(),
    }
    Ok(())
}

/// Stops the pdt-host holding the data folder's lock (one launchd did not run, e.g. started by the tray) and waits up
/// to `stop_timeout` for the lock to be free: whether it is.
fn stop_running_host(cfg: &HostConfig, os: Os, runner: &mut dyn Runner) -> bool {
    let free = |within: Duration| {
        let deadline = Instant::now() + within;
        loop {
            if !host_running(&cfg.data_dir) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    if free(Duration::from_secs(2).min(cfg.stop_timeout)) {
        return true;
    }
    stop_host(cfg, os, runner);
    free(cfg.stop_timeout)
}

fn write_launch_agent(cfg: &HostConfig, plist: &Path) -> Result<(), String> {
    let log = cfg.data_dir.join("logs").join(format!("{HOST_BIN}.launchd.log"));
    let _ = std::fs::create_dir_all(log.parent().expect("logs"));
    std::fs::create_dir_all(plist.parent().expect("LaunchAgents")).map_err(|e| e.to_string())?;
    let text = launchd_plist(&bundled_host_path(&cfg.install_dir), &cfg.install_dir, &log);
    std::fs::write(plist, text).map_err(|e| e.to_string())
}

/// Removes the data folder copy of pdt-host that older macOS versions registered.
pub fn remove_old_copy(data_dir: &Path) {
    let dir = installed_host_path(data_dir).parent().expect("host dir").to_path_buf();
    let _ = std::fs::remove_dir_all(dir);
}

/// macOS, at startup: an agent written by an older version still names the data folder copy, or lacks
/// `AssociatedBundleIdentifiers` (PDT-156); it is rewritten to name the bundle's pdt-host and the app (launchd reads it
/// again at the next login, or when `install` bootstraps it). Returns whether it was rewritten. A missing agent (pdt-host started by the tray or by hand) is left missing.
pub fn migrate_launch_agent(cfg: &HostConfig, home: &Path) -> Result<bool, String> {
    let plist = launch_agent_path(home);
    let Ok(text) = std::fs::read_to_string(&plist) else { return Ok(false) };
    let program = bundled_host_path(&cfg.install_dir);
    if cfg.dev
        || !program.is_file()
        || crate::placement::bundle_problem(&cfg.install_dir).is_some()
        || (text.contains(&format!("<string>{}</string>", xml(&program.to_string_lossy())))
            && text.contains("<key>AssociatedBundleIdentifiers</key>"))
    {
        return Ok(false);
    }
    write_launch_agent(cfg, &plist)?;
    Ok(true)
}

/// Stops this data folder's pdt-host and pdtd (so the install folder can be removed), then removes the login
/// registration.
pub fn uninstall(cfg: &HostConfig) -> Result<(), String> {
    let (home, config) = (std::env::home_dir(), dirs::config_dir());
    uninstall_with(cfg, Os::current(), Dirs { home: home.as_deref(), config: config.as_deref() }, &mut System)
}

/// The user's folders `uninstall` looks in: home (macOS) and config (Linux).
#[derive(Clone, Copy, Default)]
pub struct Dirs<'a> {
    pub home: Option<&'a Path>,
    pub config: Option<&'a Path>,
}

pub fn uninstall_with(cfg: &HostConfig, os: Os, dirs: Dirs, runner: &mut dyn Runner) -> Result<(), String> {
    let home = dirs.home;
    if cfg.dev {
        return Err("only the stable profile is registered at login; unset PDT_PROFILE to unregister pdt-host".into());
    }
    // pdt-host goes first, so nothing starts pdtd again. Booting the agent out stops a launchd-run pdt-host.
    if os == Os::Mac {
        let domain = launchd_domain(runner)?;
        let _ = runner.status(launchctl(["bootout".to_string(), format!("{domain}/{HOST_LABEL}")]));
    }
    if os == Os::Linux {
        // Stops a pdt-host systemd runs; pdtd is left (KillMode=process) and stopped below.
        let _ = runner.status(systemctl(["disable", "--now", HOST_SYSTEMD_UNIT]));
    }
    stop_host(cfg, os, runner);
    stop_pdtd(cfg);
    match os {
        Os::Mac => {
            let home = home.ok_or("no home folder")?;
            let _ = std::fs::remove_file(launch_agent_path(home));
            remove_old_copy(&cfg.data_dir);
        }
        Os::Windows => {
            let mut reg = Command::new("reg");
            reg.args(["delete", RUN_KEY, "/v", HOST_AUTOSTART_NAME, "/f"]);
            let _ = runner.status(reg);
        }
        Os::Linux => {
            if let Some(config) = dirs.config {
                let _ = std::fs::remove_file(systemd_unit_path(config));
                let _ = runner.status(systemctl(["daemon-reload"]));
            }
        }
        Os::Other => {}
    }
    // The tray's own login item goes too (PDT-148); it would only start an app that is gone.
    if let Err(e) = crate::tray_login::unregister(os, home, runner) {
        eprintln!("could not remove the tray's login item: {e}");
    }
    Ok(())
}

/// Kills the pdt-host that holds `cfg`'s data folder lock, if one does (pdtd, its child, is left to stop gracefully).
fn stop_host(cfg: &HostConfig, os: Os, runner: &mut dyn Runner) {
    if !host_running(&cfg.data_dir) {
        return;
    }
    let pid = std::fs::read_to_string(pid_path(&cfg.data_dir)).ok().and_then(|s| s.trim().parse::<u32>().ok());
    let Some(pid) = pid.filter(|&p| p != std::process::id()) else { return };
    let mut kill = if os == Os::Windows { Command::new("taskkill") } else { Command::new("kill") };
    if os == Os::Windows {
        kill.args(["/F", "/PID"]);
    }
    kill.arg(pid.to_string());
    let _ = runner.status(kill);
}

/// Asks `cfg`'s pdtd to stop and waits (up to `stop_timeout`) until its port and its data folder lock are free.
fn stop_pdtd(cfg: &HostConfig) {
    if !matches!(probe(cfg.port), Probe::Ours { .. }) {
        return;
    }
    let _ = request_shutdown(cfg.port);
    let lock = cfg.data_dir.join(format!("{DAEMON_BIN}.lock"));
    let deadline = Instant::now() + cfg.stop_timeout;
    while (probe(cfg.port) != Probe::Free || locked(&lock)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn launch_agent_runs_the_program_at_load_and_keeps_it_alive() {
        let p = launchd_plist(
            Path::new("/Users/me/Applications/A & B.app/Contents/MacOS/pdt-host"),
            Path::new("/Users/me/Applications/A & B.app/Contents/MacOS"),
            Path::new("/tmp/h.log"),
        );
        assert!(p.contains("<key>Label</key><string>dev.personaldevteam.host</string>"));
        assert!(p.contains("<string>/Users/me/Applications/A &amp; B.app/Contents/MacOS/pdt-host</string>"));
        assert!(p.contains("<string>/Users/me/Applications/A &amp; B.app/Contents/MacOS</string>"));
        assert!(p.contains("<key>RunAtLoad</key><true/>") && p.contains("<key>KeepAlive</key><true/>"));
        assert!(
            p.contains(
                "<key>AssociatedBundleIdentifiers</key>\n  <array>\n    <string>dev.personaldevteam.desktop</string>"
            ),
            "{p}"
        );
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
        let p = Path::new("p.exe");
        assert_eq!(run_key_command(p, Path::new(r"C:\P D T\")), r#""p.exe" run --install-dir "C:\P D T""#);
        assert_eq!(run_key_command(p, Path::new(r"C:\")), r#""p.exe" run --install-dir "C:\.""#);
    }

    #[test]
    fn a_dev_profile_is_never_registered() {
        let mut s = setup();
        s.cfg.dev = true;
        let mut r = Recorder::default();
        let err = install_with(&s.cfg, &target(&s, Os::Windows), &mut r).unwrap_err();
        assert!(err.contains("stable profile"), "{err}");
        assert!(r.ran.is_empty());
        assert!(!installed_host_path(&s.cfg.data_dir).exists());
        assert!(uninstall_with(&s.cfg, Os::Windows, s.dirs(), &mut r).is_err());
        assert!(r.ran.is_empty());
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

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    struct Setup {
        _tmp: tempfile::TempDir,
        cfg: HostConfig,
        exe: PathBuf,
        home: PathBuf,
        config: PathBuf,
    }

    impl Setup {
        fn dirs(&self) -> Dirs<'_> {
            Dirs { home: Some(&self.home), config: Some(&self.config) }
        }
    }

    fn setup() -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("app").join("pdt-host-built");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "new host").unwrap();
        let home = tmp.path().join("home");
        let mut cfg = HostConfig::from_lookup(
            |k| (k == "PDT_DATA_DIR").then(|| tmp.path().join("data").to_string_lossy().into_owned()),
            None,
            tmp.path().join("app"),
        )
        .unwrap();
        // Never the stable port: uninstall probes it.
        cfg.port = free_port();
        cfg.stop_timeout = Duration::from_millis(300);
        let config = home.join(".config");
        Setup { _tmp: tmp, cfg, exe, home, config }
    }

    fn target(s: &Setup, os: Os) -> Target<'_> {
        Target { os, exe: &s.exe, home: Some(&s.home), config: Some(&s.config), replace: false }
    }

    #[test]
    fn windows_hands_over_from_the_install_folder_to_the_data_folder_copy() {
        let mut s = setup();
        let installed = bundled_host_path(&s.cfg.install_dir);
        let copy = installed_host_path(&s.cfg.data_dir);
        std::fs::write(&installed, "host").unwrap();
        assert_eq!(copy_to_hand_over_to(Os::Windows, &s.cfg, &installed), None, "no copy yet: it runs itself");
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, "host").unwrap();
        assert_eq!(copy_to_hand_over_to(Os::Windows, &s.cfg, &installed), Some(copy.clone()));
        assert_eq!(copy_to_hand_over_to(Os::Windows, &s.cfg, &copy), None, "the copy runs itself");
        assert_eq!(copy_to_hand_over_to(Os::Windows, &s.cfg, &s.exe), None, "a build elsewhere runs itself");
        assert_eq!(copy_to_hand_over_to(Os::Mac, &s.cfg, &installed), None);
        assert_eq!(copy_to_hand_over_to(Os::Linux, &s.cfg, &installed), None);
        s.cfg.dev = true;
        assert_eq!(copy_to_hand_over_to(Os::Windows, &s.cfg, &installed), None, "the dev profile runs itself");
    }

    #[test]
    fn the_copy_always_gets_the_install_folder() {
        let dir = std::env::temp_dir().join("P D T");
        let want = |extra: &[&str]| {
            let mut v: Vec<OsString> = vec!["run".into()];
            v.extend(extra.iter().map(OsString::from));
            v.extend(["--install-dir".into(), dir.clone().into_os_string()]);
            v
        };
        assert_eq!(hand_over_args(&[], &dir), want(&[]), "started by hand, without --install-dir");
        let given = ["--install-dir".to_string(), "elsewhere".to_string(), "--x".to_string()];
        assert_eq!(hand_over_args(&given, &dir), want(&["--x"]), "the resolved folder replaces the given one");
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

    /// The bundle's own pdt-host, as an installed app has it.
    fn bundle_host(s: &Setup) -> PathBuf {
        let p = bundled_host_path(&s.cfg.install_dir);
        std::fs::write(&p, "bundled host").unwrap();
        p
    }

    /// The data folder copy an older version registered.
    fn old_copy(s: &Setup) -> PathBuf {
        let p = installed_host_path(&s.cfg.data_dir);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "old copy").unwrap();
        p
    }

    #[test]
    fn mac_install_registers_the_bundles_host_without_copying_it() {
        let s = setup();
        let program = bundle_host(&s);
        let copy = old_copy(&s);
        let mut r = Recorder::default();
        install_with(&s.cfg, &target(&s, Os::Mac), &mut r).unwrap();
        let plist = launch_agent_path(&s.home);
        let text = std::fs::read_to_string(&plist).unwrap();
        assert!(text.contains(&format!("<string>{}</string>", program.display())), "{text}");
        assert!(text.contains(&format!("<string>{}</string>", s.cfg.install_dir.display())), "{text}");
        assert!(text.contains("pdt-host.launchd.log"), "{text}");
        assert!(!text.contains(&copy.display().to_string()), "{text}");
        assert_eq!(
            r.ran,
            [
                "id | -u".to_string(),
                "launchctl | bootout | gui/501/dev.personaldevteam.host".to_string(),
                format!("launchctl | bootstrap | gui/501 | {}", plist.display()),
            ]
        );
        assert!(r.spawned.is_empty());
        assert_eq!(std::fs::read_to_string(&program).unwrap(), "bundled host", "the bundle is never written");
        assert!(!copy.parent().unwrap().exists(), "the old copy is removed once launchd runs the bundle's");
    }

    #[test]
    fn mac_install_refuses_without_the_bundles_host() {
        let s = setup();
        let mut r = Recorder::default();
        let err = install_with(&s.cfg, &target(&s, Os::Mac), &mut r).unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
        assert!(r.ran.is_empty());
        assert!(!launch_agent_path(&s.home).exists());
        assert!(!installed_host_path(&s.cfg.data_dir).exists());
    }

    #[test]
    fn mac_install_while_a_host_runs_rewrites_the_agent_without_launchctl() {
        let s = setup();
        bundle_host(&s);
        let copy = old_copy(&s);
        let _lock = hold_lock(&s.cfg);
        let mut r = Recorder::default();
        install_with(&s.cfg, &target(&s, Os::Mac), &mut r).unwrap();
        assert!(launch_agent_path(&s.home).is_file());
        assert!(r.ran.is_empty(), "{:?}", r.ran);
        assert!(copy.is_file(), "launchd may still run the old copy until it reads the agent again");
    }

    #[test]
    fn mac_install_reports_a_failed_bootstrap() {
        let s = setup();
        bundle_host(&s);
        // bootout may fail (nothing registered); bootstrap must not.
        let mut r = Recorder { statuses: [false, false].into(), ..Default::default() };
        let err = install_with(&s.cfg, &target(&s, Os::Mac), &mut r).unwrap_err();
        assert!(err.contains("bootstrap failed"), "{err}");
    }

    #[test]
    fn an_agent_naming_the_old_copy_is_rewritten_to_name_the_bundles_host() {
        let mut s = setup();
        let program = bundle_host(&s);
        let copy = old_copy(&s);
        assert!(!migrate_launch_agent(&s.cfg, &s.home).unwrap(), "no agent: nothing is written");
        assert!(!launch_agent_path(&s.home).exists());

        let plist = launch_agent_path(&s.home);
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, launchd_plist(&copy, &s.cfg.install_dir, Path::new("l.log"))).unwrap();
        assert!(migrate_launch_agent(&s.cfg, &s.home).unwrap());
        let text = std::fs::read_to_string(&plist).unwrap();
        assert!(text.contains(&format!("<string>{}</string>", program.display())), "{text}");
        assert!(!text.contains(&copy.display().to_string()), "{text}");
        assert!(!migrate_launch_agent(&s.cfg, &s.home).unwrap(), "already the bundle's");

        // An agent from before PDT-156 names the bundle's pdt-host but not the app.
        let block = "  <key>AssociatedBundleIdentifiers</key>\n  <array>\n    <string>dev.personaldevteam.desktop</string>\n  </array>\n";
        assert!(text.contains(block), "{text}");
        let without_app = text.replace(block, "");
        std::fs::write(&plist, &without_app).unwrap();
        assert!(migrate_launch_agent(&s.cfg, &s.home).unwrap(), "{without_app}");
        assert!(std::fs::read_to_string(&plist).unwrap().contains("<key>AssociatedBundleIdentifiers</key>"));

        std::fs::write(&plist, "old").unwrap();
        s.cfg.dev = true;
        assert!(!migrate_launch_agent(&s.cfg, &s.home).unwrap(), "a dev host never touches it");
        assert_eq!(std::fs::read_to_string(&plist).unwrap(), "old");
    }

    /// Moves `s`'s install folder into an App Translocation folder, as macOS runs an app opened from Downloads.
    fn translocate(s: &mut Setup) {
        let dir = s._tmp.path().join("AppTranslocation").join("0F1E").join("d").join("P 2.app").join("Contents");
        s.cfg.install_dir = dir.join("MacOS");
        std::fs::create_dir_all(&s.cfg.install_dir).unwrap();
    }

    #[test]
    fn a_translocated_bundle_is_never_registered() {
        let mut s = setup();
        translocate(&mut s);
        bundle_host(&s);
        let mut r = Recorder::default();
        let err = install_with(&s.cfg, &target(&s, Os::Mac), &mut r).unwrap_err();
        assert!(err.contains("not starting this copy at login") && err.contains("Applications folder"), "{err}");
        assert!(r.ran.is_empty(), "{:?}", r.ran);
        assert!(!launch_agent_path(&s.home).exists());
    }

    #[test]
    fn an_agent_is_never_rewritten_to_name_a_translocated_bundle() {
        let mut s = setup();
        let copy = old_copy(&s);
        translocate(&mut s);
        bundle_host(&s);
        let plist = launch_agent_path(&s.home);
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        let text = launchd_plist(&copy, Path::new("/Applications/P.app/Contents/MacOS"), Path::new("l.log"));
        std::fs::write(&plist, &text).unwrap();
        assert!(!migrate_launch_agent(&s.cfg, &s.home).unwrap());
        assert_eq!(std::fs::read_to_string(&plist).unwrap(), text);
    }

    #[test]
    fn mac_install_replace_takes_over_from_a_running_host() {
        let s = setup();
        bundle_host(&s);
        let lock = hold_lock(&s.cfg);
        // The other copy's pdt-host lets go once launchd has booted it out.
        let let_go = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(lock);
        });
        let mut r = Recorder::default();
        install_with(&s.cfg, &Target { replace: true, ..target(&s, Os::Mac) }, &mut r).unwrap();
        let_go.join().unwrap();
        let plist = launch_agent_path(&s.home);
        assert_eq!(
            r.ran,
            [
                "id | -u".to_string(),
                "launchctl | bootout | gui/501/dev.personaldevteam.host".to_string(),
                format!("launchctl | bootstrap | gui/501 | {}", plist.display()),
            ]
        );
    }

    #[test]
    fn mac_install_replace_kills_a_host_launchd_does_not_run_and_fails_if_it_stays() {
        let s = setup();
        bundle_host(&s);
        let _lock = hold_lock(&s.cfg);
        std::fs::write(pid_path(&s.cfg.data_dir), "4242\n").unwrap();
        let mut r = Recorder::default();
        let err = install_with(&s.cfg, &Target { replace: true, ..target(&s, Os::Mac) }, &mut r).unwrap_err();
        assert!(err.contains("did not stop"), "{err}");
        assert_eq!(r.ran.last().map(String::as_str), Some("kill | 4242"), "{:?}", r.ran);
        assert!(!r.ran.iter().any(|l| l.contains("bootstrap")), "{:?}", r.ran);
    }

    #[test]
    fn the_running_hosts_program_is_known_only_while_it_runs() {
        let s = setup();
        std::fs::create_dir_all(&s.cfg.data_dir).unwrap();
        let program = Path::new("/Applications/P.app/Contents/MacOS/pdt-host");
        write_program(&s.cfg.data_dir, program).unwrap();
        assert_eq!(running_program(&s.cfg.data_dir), None, "no pdt-host holds the lock");
        let _lock = hold_lock(&s.cfg);
        assert_eq!(running_program(&s.cfg.data_dir).as_deref(), Some(program));
        std::fs::remove_file(program_path(&s.cfg.data_dir)).unwrap();
        assert_eq!(running_program(&s.cfg.data_dir), None, "an older pdt-host does not say");
    }

    #[test]
    fn mac_uninstall_removes_the_old_copy() {
        let s = setup();
        let copy = old_copy(&s);
        uninstall_with(&s.cfg, Os::Mac, s.dirs(), &mut Recorder::default()).unwrap();
        assert!(!copy.exists());
    }

    #[test]
    fn the_systemd_unit_runs_the_install_folders_host_and_survives_its_restarts() {
        let u = systemd_unit(Path::new("/home/me/opt/p d t/pdt-host"), Path::new("/home/me/opt/p d t"));
        assert!(
            u.contains("ExecStart=\"/home/me/opt/p d t/pdt-host\" run --install-dir \"/home/me/opt/p d t\"\n"),
            "{u}"
        );
        for line in [
            "Description=Personal Dev Team host (pdt-host)",
            "Type=simple",
            "Environment=PDT_HOST_SERVICE=systemd",
            "Restart=always",
            "RestartPreventExitStatus=3",
            "KillMode=process",
            "WantedBy=default.target",
        ] {
            assert!(u.lines().any(|l| l == line), "{line} missing from\n{u}");
        }
        // Specifiers, variables, quotes and backslashes are taken literally.
        let u = systemd_unit(Path::new(r#"/x/50%$HOME"q\pdt-host"#), Path::new("/x"));
        assert!(u.contains(r#"ExecStart="/x/50%%$$HOME\"q\\pdt-host" run"#), "{u}");
        assert_eq!(
            systemd_unit_path(Path::new("/home/me/.config")),
            Path::new("/home/me/.config/systemd/user/pdt-host.service")
        );
    }

    #[test]
    fn linux_install_writes_the_unit_and_enables_and_starts_it() {
        let s = setup();
        let program = bundle_host(&s);
        let mut r = Recorder::default();
        install_with(&s.cfg, &target(&s, Os::Linux), &mut r).unwrap();
        let unit = systemd_unit_path(&s.config);
        assert_eq!(std::fs::read_to_string(&unit).unwrap(), systemd_unit(&program, &s.cfg.install_dir));
        assert_eq!(
            r.ran,
            ["systemctl | --user | daemon-reload", "systemctl | --user | enable | --now | pdt-host.service"]
        );
        assert!(r.spawned.is_empty());
        assert!(!installed_host_path(&s.cfg.data_dir).exists(), "nothing is copied to the data folder");
    }

    #[test]
    fn linux_install_while_a_host_runs_only_enables_the_service() {
        let s = setup();
        bundle_host(&s);
        let _lock = hold_lock(&s.cfg);
        let mut r = Recorder::default();
        install_with(&s.cfg, &target(&s, Os::Linux), &mut r).unwrap();
        assert_eq!(r.ran, ["systemctl | --user | daemon-reload", "systemctl | --user | enable | pdt-host.service"]);
    }

    #[test]
    fn linux_install_reports_a_missing_program_or_systemd() {
        let s = setup();
        let mut r = Recorder::default();
        let err = install_with(&s.cfg, &target(&s, Os::Linux), &mut r).unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
        assert!(r.ran.is_empty() && !systemd_unit_path(&s.config).exists());

        bundle_host(&s);
        let mut r = Recorder { statuses: [false].into(), ..Default::default() };
        let err = install_with(&s.cfg, &target(&s, Os::Linux), &mut r).unwrap_err();
        assert!(err.contains("daemon-reload failed") && err.contains("enable-linger"), "{err}");
        let mut r = Recorder { statuses: [true, false].into(), ..Default::default() };
        let err = install_with(&s.cfg, &target(&s, Os::Linux), &mut r).unwrap_err();
        assert!(err.contains("enable pdt-host.service failed"), "{err}");
    }

    #[test]
    fn linux_uninstall_disables_the_service_and_removes_the_unit() {
        let s = setup();
        let unit = systemd_unit_path(&s.config);
        std::fs::create_dir_all(unit.parent().unwrap()).unwrap();
        std::fs::write(&unit, "x").unwrap();
        let mut r = Recorder::default();
        uninstall_with(&s.cfg, Os::Linux, s.dirs(), &mut r).unwrap();
        assert_eq!(
            r.ran,
            ["systemctl | --user | disable | --now | pdt-host.service", "systemctl | --user | daemon-reload"]
        );
        assert!(!unit.exists());
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
        uninstall_with(&s.cfg, Os::Windows, s.dirs(), &mut r).unwrap();
        assert_eq!(
            r.ran,
            [
                r"reg | delete | HKCU\Software\Microsoft\Windows\CurrentVersion\Run | /v | Personal Dev Team host | /f",
                r"reg | delete | HKCU\Software\Microsoft\Windows\CurrentVersion\Run | /v | Personal Dev Team | /f",
            ],
            "the tray's login item goes too"
        );

        let plist = launch_agent_path(&s.home);
        let tray = crate::tray_login::launch_agent_path(&s.home);
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, "x").unwrap();
        std::fs::write(&tray, "x").unwrap();
        let mut r = Recorder::default();
        uninstall_with(&s.cfg, Os::Mac, s.dirs(), &mut r).unwrap();
        assert_eq!(r.ran, ["id | -u", "launchctl | bootout | gui/501/dev.personaldevteam.host"]);
        assert!(!plist.exists() && !tray.exists());
    }

    /// Answers health as ours, records each request line, and accepts a shutdown.
    fn fake_pdtd() -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for mut c in l.incoming().flatten() {
                let mut buf = [0u8; 1024];
                let n = c.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string();
                let body = if req.starts_with("POST") {
                    "HTTP/1.0 202 Accepted\r\n\r\n"
                } else {
                    "HTTP/1.0 200 OK\r\n\r\n{\"name\":\"Personal Dev Team\",\"version\":\"1\"}"
                };
                let _ = c.write_all(body.as_bytes());
                let _ = tx.send(req);
            }
        });
        (port, rx)
    }

    #[test]
    fn uninstall_stops_this_data_folders_host_and_pdtd_first() {
        let mut s = setup();
        let (port, requests) = fake_pdtd();
        s.cfg.port = port;
        let lock = hold_lock(&s.cfg);
        std::fs::write(pid_path(&s.cfg.data_dir), "4242\n").unwrap();
        let mut r = Recorder::default();
        uninstall_with(&s.cfg, Os::Windows, s.dirs(), &mut r).unwrap();
        assert_eq!(r.ran[0], "taskkill | /F | /PID | 4242");
        assert!(r.ran[1].starts_with("reg | delete | "), "{:?}", r.ran);
        let seen: Vec<String> = requests.try_iter().collect();
        assert!(seen.iter().any(|l| l.starts_with("POST /api/shutdown ")), "{seen:?}");

        // No lock holder: nothing is killed.
        drop(lock);
        let mut r = Recorder::default();
        uninstall_with(&s.cfg, Os::Mac, s.dirs(), &mut r).unwrap();
        assert!(!r.ran.iter().any(|l| l.starts_with("kill")), "{:?}", r.ran);
    }
}
