//! Starting the tray app at login (PDT-148), beside pdt-host's own registration ([`crate::register`]): without it a
//! restart brings pdt-host and pdtd back but no menu bar or notification-area icon, and PDT looks like it did not
//! start. A per-user LaunchAgent on macOS (RunAtLoad, no KeepAlive: Quit means quit) and a per-user `HKCU\…\Run` value
//! on Windows. Both start the tray with [`TRAY_QUIET_ARG`], so a login opens no browser window.
//!
//! On by default; turning it off leaves a marker in the data folder, so the tray never registers itself again. The
//! LaunchAgent is only written, never bootstrapped: launchd reads it at the next login (bootstrapping it now would
//! start a second tray, and booting it out would stop one launchd started).

use crate::register::{Os, Runner};
use pdt_core::brand::{TRAY_AUTOSTART_NAME, TRAY_LABEL};
use pdt_core::handoff::TRAY_QUIET_ARG;
use std::path::{Path, PathBuf};
use std::process::Command;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

pub fn launch_agent_path(home: &Path) -> PathBuf {
    home.join("Library").join("LaunchAgents").join(format!("{TRAY_LABEL}.plist"))
}

/// The marker that says the user turned starting at login off.
pub fn off_marker(data_dir: &Path) -> PathBuf {
    data_dir.join("tray-login-off")
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn launchd_plist(program: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{program}</string>
    <string>{quiet}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#,
        label = xml(TRAY_LABEL),
        program = xml(&program.to_string_lossy()),
        quiet = xml(TRAY_QUIET_ARG),
    )
}

pub fn run_key_command(program: &Path) -> String {
    format!("\"{}\" {TRAY_QUIET_ARG}", program.display())
}

/// Where the tray registers: the platform, its own executable, the home folder (macOS) and the data folder (for the
/// off marker).
pub struct Target<'a> {
    pub os: Os,
    pub exe: &'a Path,
    pub home: Option<&'a Path>,
    pub data_dir: &'a Path,
}

/// Whether the user wants the tray started at login (the default).
pub fn wanted(data_dir: &Path) -> bool {
    !off_marker(data_dir).exists()
}

/// Whether the tray is registered at login.
pub fn registered(t: &Target, runner: &mut dyn Runner) -> bool {
    match t.os {
        Os::Mac => t.home.is_some_and(|h| launch_agent_path(h).is_file()),
        Os::Windows => {
            let mut reg = Command::new("reg");
            reg.args(["query", RUN_KEY, "/v", TRAY_AUTOSTART_NAME]);
            runner.status(reg).unwrap_or(false)
        }
        Os::Linux | Os::Other => false,
    }
}

/// Registers the tray at login unless the user turned that off: whether it is registered afterwards. Writes only
/// when something changed (macOS) or the value is refreshed in place (Windows), so it runs at every tray start.
pub fn ensure(t: &Target, runner: &mut dyn Runner) -> Result<bool, String> {
    if !wanted(t.data_dir) {
        return Ok(false);
    }
    register(t, runner).map(|()| true)
}

/// Turns starting at login on or off, and remembers the choice.
pub fn set(t: &Target, on: bool, runner: &mut dyn Runner) -> Result<(), String> {
    let marker = off_marker(t.data_dir);
    if on {
        if marker.exists() {
            std::fs::remove_file(&marker).map_err(|e| e.to_string())?;
        }
        register(t, runner)
    } else {
        std::fs::create_dir_all(t.data_dir).map_err(|e| e.to_string())?;
        std::fs::write(&marker, b"").map_err(|e| e.to_string())?;
        unregister(t.os, t.home, runner)
    }
}

fn register(t: &Target, runner: &mut dyn Runner) -> Result<(), String> {
    match t.os {
        Os::Mac => {
            let plist = launch_agent_path(t.home.ok_or("no home folder")?);
            let text = launchd_plist(t.exe);
            if std::fs::read_to_string(&plist).ok().as_deref() == Some(text.as_str()) {
                return Ok(());
            }
            std::fs::create_dir_all(plist.parent().expect("LaunchAgents")).map_err(|e| e.to_string())?;
            std::fs::write(&plist, text).map_err(|e| e.to_string())
        }
        Os::Windows => {
            let mut reg = Command::new("reg");
            reg.args(["add", RUN_KEY, "/v", TRAY_AUTOSTART_NAME, "/t", "REG_SZ", "/d"])
                .arg(run_key_command(t.exe))
                .arg("/f");
            if runner.status(reg)? { Ok(()) } else { Err("reg add failed".into()) }
        }
        Os::Linux | Os::Other => Err("starting at login is supported on macOS and Windows only".into()),
    }
}

/// Removes the tray's login registration (the choice is left as it is). `pdt-host uninstall` runs it too.
pub fn unregister(os: Os, home: Option<&Path>, runner: &mut dyn Runner) -> Result<(), String> {
    match os {
        Os::Mac => {
            let plist = launch_agent_path(home.ok_or("no home folder")?);
            match std::fs::remove_file(&plist) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        }
        Os::Windows => {
            let mut reg = Command::new("reg");
            reg.args(["delete", RUN_KEY, "/v", TRAY_AUTOSTART_NAME, "/f"]);
            // A missing value fails; it is gone either way.
            let _ = runner.status(reg)?;
            Ok(())
        }
        Os::Linux | Os::Other => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records command lines; `reg query` succeeds while `present`.
    #[derive(Default)]
    struct Recorder {
        ran: Vec<String>,
        present: bool,
    }

    impl Runner for Recorder {
        fn status(&mut self, cmd: Command) -> Result<bool, String> {
            let line = std::iter::once(cmd.get_program())
                .chain(cmd.get_args())
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" | ");
            let ok = !line.contains("| query |") || self.present;
            self.ran.push(line);
            Ok(ok)
        }
        fn stdout(&mut self, _: Command) -> Result<String, String> {
            unreachable!()
        }
        fn spawn(&mut self, _: Command) -> Result<(), String> {
            unreachable!()
        }
    }

    #[test]
    fn the_launch_agent_starts_the_tray_quietly_once() {
        let p = launchd_plist(Path::new("/Applications/A & B.app/Contents/MacOS/pdt-desktop"));
        assert!(p.contains("<key>Label</key><string>dev.personaldevteam.tray</string>"));
        assert!(p.contains("<string>/Applications/A &amp; B.app/Contents/MacOS/pdt-desktop</string>"));
        assert!(p.contains("<string>--no-open</string>") && p.contains("<key>RunAtLoad</key><true/>"));
        assert!(!p.contains("KeepAlive"), "Quit must stay quit");
        assert_eq!(
            launch_agent_path(Path::new("/Users/me")),
            Path::new("/Users/me/Library/LaunchAgents/dev.personaldevteam.tray.plist")
        );
        assert_eq!(run_key_command(Path::new(r"C:\P D T\pdt.exe")), r#""C:\P D T\pdt.exe" --no-open"#);
    }

    #[test]
    fn mac_registers_by_default_and_remembers_off() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, data) = (tmp.path().join("home"), tmp.path().join("data"));
        let exe = tmp.path().join("P.app").join("Contents").join("MacOS").join("tray");
        let t = Target { os: Os::Mac, exe: &exe, home: Some(&home), data_dir: &data };
        let mut r = Recorder::default();
        assert!(!registered(&t, &mut r));
        assert!(ensure(&t, &mut r).unwrap());
        assert!(registered(&t, &mut r));
        let plist = launch_agent_path(&home);
        assert!(std::fs::read_to_string(&plist).unwrap().contains(&*exe.to_string_lossy()));
        assert!(ensure(&t, &mut r).unwrap(), "again: nothing changes");

        set(&t, false, &mut r).unwrap();
        assert!(!plist.exists() && !wanted(&data));
        assert!(!ensure(&t, &mut r).unwrap(), "turned off stays off at the next start");
        assert!(!plist.exists());
        set(&t, true, &mut r).unwrap();
        assert!(plist.is_file() && wanted(&data));
        assert!(r.ran.is_empty(), "no launchctl: launchd reads the agent at the next login");
    }

    #[test]
    fn windows_uses_a_run_value() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = PathBuf::from(r"C:\P\pdt.exe");
        let t = Target { os: Os::Windows, exe: &exe, home: None, data_dir: tmp.path() };
        let mut r = Recorder::default();
        assert!(!registered(&t, &mut r));
        ensure(&t, &mut r).unwrap();
        set(&t, false, &mut r).unwrap();
        let run = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        assert_eq!(
            r.ran,
            [
                format!("reg | query | {run} | /v | Personal Dev Team"),
                format!(
                    r#"reg | add | {run} | /v | Personal Dev Team | /t | REG_SZ | /d | "C:\P\pdt.exe" --no-open | /f"#
                ),
                format!("reg | delete | {run} | /v | Personal Dev Team | /f"),
            ]
        );
        r.present = true;
        assert!(registered(&t, &mut r));
    }
}
