//! Installing a release and rolling it back (spec §3.4 steps 3–4): the silent NSIS installer on Windows, a bundle swap
//! on macOS. The previous version is kept: a copy of the install folder (Windows) or the old bundle (macOS).

use crate::config::HostConfig;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long the installer (or extracting the archive) may take before it is stopped and the update rolled back.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

/// One update: `backup`, then `install`; `rollback` after a failed `install` or health check.
pub trait Installer {
    /// Keeps what is needed to restore the current version. Changes nothing that runs: after an error nothing has
    /// changed and the update is abandoned.
    fn backup(&self) -> Result<(), String>;
    /// Installs `asset`. After an error the install may be partial; the caller rolls back.
    fn install(&self, asset: &Path) -> Result<(), String>;
    /// Restores the version kept by `backup`. Succeeds when there is nothing to undo.
    fn rollback(&self) -> Result<(), String>;
    /// Where the previous version is kept, if anywhere.
    fn backup_path(&self) -> Option<PathBuf>;
}

pub fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            // A new file, never an overwrite: macOS can kill a program whose executable was rewritten in place.
            if to.is_file() {
                std::fs::remove_file(&to)?;
            }
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

/// Makes `dst` an exact copy of `src`.
pub fn replace_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.is_dir() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{} does not exist", src.display())));
    }
    if dst.exists() {
        for e in std::fs::read_dir(dst)? {
            let p = e?.path();
            if p.is_dir() { std::fs::remove_dir_all(&p)? } else { std::fs::remove_file(&p)? }
        }
    }
    copy_dir(src, dst)
}

/// Runs `cmd` to completion; after `timeout` it is stopped together with every process it started.
pub fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Result<ExitStatus, String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    #[cfg(unix)]
    {
        // Its own process group, so the whole tree can be killed.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.stdin(Stdio::null()).spawn().map_err(|e| format!("could not start {program}: {e}"))?;
    let end = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                kill_tree(&mut child);
                return Err(format!("{program} timed out after {timeout:?} and was stopped"));
            }
            Err(e) => {
                kill_tree(&mut child);
                return Err(format!("waiting for {program}: {e}"));
            }
        }
    }
}

/// Kills `child` and everything it started (`taskkill /T` on Windows, its process group on unix).
fn kill_tree(child: &mut std::process::Child) {
    let pid = child.id().to_string();
    let quiet = |c: &mut Command| {
        let _ = c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    };
    if cfg!(windows) {
        quiet(Command::new("taskkill").args(["/PID", &pid, "/T", "/F"]));
    } else {
        quiet(Command::new("kill").args(["-KILL", "--", &format!("-{pid}")]));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Tests (and the basis of the Windows backup): the asset is a folder whose contents are copied over `target`.
pub struct CopyDirInstaller {
    pub target: PathBuf,
    pub backup: PathBuf,
}

impl Installer for CopyDirInstaller {
    fn backup(&self) -> Result<(), String> {
        replace_dir(&self.target, &self.backup).map_err(|e| format!("backing up {}: {e}", self.target.display()))
    }
    fn install(&self, asset: &Path) -> Result<(), String> {
        copy_dir(asset, &self.target).map_err(|e| format!("copying the new version: {e}"))
    }
    fn rollback(&self) -> Result<(), String> {
        replace_dir(&self.backup, &self.target).map_err(|e| format!("restoring the previous version: {e}"))
    }
    fn backup_path(&self) -> Option<PathBuf> {
        Some(self.backup.clone())
    }
}

/// Used when this host cannot install updates (no installer or bundle found, ruling F9): pdt-host keeps supervising
/// pdtd and refuses every update with `reason`, before anything is touched.
pub struct Unsupported {
    pub reason: String,
}

impl Unsupported {
    fn refusal(&self) -> String {
        format!("updates are not supported on this host: {}", self.reason)
    }
}

impl Installer for Unsupported {
    fn backup(&self) -> Result<(), String> {
        Err(self.refusal())
    }
    fn install(&self, _asset: &Path) -> Result<(), String> {
        Err(self.refusal())
    }
    fn rollback(&self) -> Result<(), String> {
        Ok(())
    }
    fn backup_path(&self) -> Option<PathBuf> {
        None
    }
}

/// NSIS: `/S` silent, `/D=` the install folder (must be last and unquoted).
pub fn nsis_args(install_dir: &Path) -> Vec<String> {
    vec!["/S".into(), format!("/D={}", install_dir.display())]
}

pub struct NsisInstaller {
    pub install_dir: PathBuf,
    pub backup: PathBuf,
    pub timeout: Duration,
}

impl Installer for NsisInstaller {
    fn backup(&self) -> Result<(), String> {
        replace_dir(&self.install_dir, &self.backup).map_err(|e| format!("backing up the install folder: {e}"))
    }
    fn install(&self, asset: &Path) -> Result<(), String> {
        let _tray = TrayPause::stop(&self.install_dir);
        let status = run_with_timeout(Command::new(asset).args(nsis_args(&self.install_dir)), self.timeout)
            .map_err(|e| format!("running the installer: {e}"))?;
        if status.success() { Ok(()) } else { Err(format!("the installer failed ({status})")) }
    }
    fn rollback(&self) -> Result<(), String> {
        let _tray = TrayPause::stop(&self.install_dir);
        replace_dir(&self.backup, &self.install_dir).map_err(|e| format!("restoring the previous version: {e}"))
    }
    fn backup_path(&self) -> Option<PathBuf> {
        Some(self.backup.clone())
    }
}

/// The bundle that contains `exe_dir` (`…/X.app/Contents/MacOS` → `…/X.app`).
pub fn bundle_of(exe_dir: &Path) -> Option<PathBuf> {
    exe_dir.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

fn with_suffix(bundle: &Path, suffix: &str) -> PathBuf {
    let mut s = bundle.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

pub fn previous_bundle(bundle: &Path) -> PathBuf {
    with_suffix(bundle, ".previous")
}

pub struct BundleInstaller {
    pub bundle: PathBuf,
    pub timeout: Duration,
}

impl BundleInstaller {
    fn staging(&self) -> Result<PathBuf, String> {
        Ok(self.bundle.parent().ok_or("the bundle has no parent folder")?.join(".pdt-update-staging"))
    }

    /// Extracts the archive into `staging` and returns the `.app` in it.
    fn extract(&self, asset: &Path, staging: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(staging).map_err(|e| e.to_string())?;
        let mut tar = Command::new("tar");
        tar.arg("-xzf").arg(asset).arg("-C").arg(staging).stdout(Stdio::null()).stderr(Stdio::null());
        let ok = run_with_timeout(&mut tar, self.timeout)?;
        if !ok.success() {
            return Err("could not extract the update".into());
        }
        std::fs::read_dir(staging)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or_else(|| "the archive has no .app bundle".into())
    }
}

impl Installer for BundleInstaller {
    /// The old bundle is kept by moving it aside during `install`; here only a stale `.previous` from an earlier
    /// update is removed, so `rollback` can never restore an older version than the one running now.
    fn backup(&self) -> Result<(), String> {
        let prev = previous_bundle(&self.bundle);
        if prev.exists() {
            std::fs::remove_dir_all(&prev).map_err(|e| format!("removing {}: {e}", prev.display()))?;
        }
        Ok(())
    }
    fn install(&self, asset: &Path) -> Result<(), String> {
        let staging = self.staging()?;
        let _ = std::fs::remove_dir_all(&staging);
        let new = match self.extract(asset, &staging) {
            Ok(new) => new,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e);
            }
        };
        let _ = Command::new("xattr").args(["-dr", "com.apple.quarantine"]).arg(&new).status();
        let _tray = TrayPause::stop(&self.bundle);
        let prev = previous_bundle(&self.bundle);
        let swapped = std::fs::rename(&self.bundle, &prev)
            .map_err(|e| format!("moving the old bundle aside: {e}"))
            .and_then(|()| {
                std::fs::rename(&new, &self.bundle).map_err(|e| {
                    let _ = std::fs::rename(&prev, &self.bundle);
                    format!("moving the new bundle into place: {e}")
                })
            });
        let _ = std::fs::remove_dir_all(&staging);
        swapped
    }
    fn rollback(&self) -> Result<(), String> {
        let prev = previous_bundle(&self.bundle);
        if !prev.exists() {
            // Nothing was moved aside, so the bundle in place is still the old one.
            return if self.bundle.exists() { Ok(()) } else { Err("there is no previous bundle to restore".into()) };
        }
        let _tray = TrayPause::stop(&self.bundle);
        let failed = with_suffix(&self.bundle, ".failed");
        let _ = std::fs::remove_dir_all(&failed);
        if self.bundle.exists() {
            std::fs::rename(&self.bundle, &failed).map_err(|e| format!("moving the failed bundle aside: {e}"))?;
        }
        if let Err(e) = std::fs::rename(&prev, &self.bundle) {
            let _ = std::fs::rename(&failed, &self.bundle);
            return Err(format!("restoring the previous bundle: {e}"));
        }
        let _ = std::fs::remove_dir_all(&failed);
        Ok(())
    }
    fn backup_path(&self) -> Option<PathBuf> {
        Some(previous_bundle(&self.bundle))
    }
}

/// The installer pdt-host uses: the platform's, or `Unsupported` (with the reason logged) when updates cannot be
/// installed here (ruling F9) or the host runs the dev profile.
pub fn host_installer(cfg: &HostConfig) -> Box<dyn Installer + Send> {
    installer_for(cfg).unwrap_or_else(|reason| {
        crate::log::log(cfg, &format!("updates cannot be installed here: {reason}"));
        Box::new(Unsupported { reason })
    })
}

/// The installer for this platform, or why updates cannot be installed here. A dev host never installs: it may run
/// next to the stable app, and the stable installer must never land in a dev build's folder.
pub fn installer_for(cfg: &HostConfig) -> Result<Box<dyn Installer + Send>, String> {
    if cfg.dev {
        Err("the dev profile never installs updates".into())
    } else if cfg!(windows) {
        Ok(Box::new(NsisInstaller {
            install_dir: cfg.install_dir.clone(),
            backup: pdt_core::handoff::update_dir(&cfg.data_dir).join("previous"),
            timeout: INSTALL_TIMEOUT,
        }))
    } else if cfg!(target_os = "macos") {
        let bundle = bundle_of(&cfg.install_dir).ok_or("pdt-host is not installed from an .app bundle")?;
        Ok(Box::new(BundleInstaller { bundle, timeout: INSTALL_TIMEOUT }))
    } else {
        Err("updates are not supported on this platform".into())
    }
}

/// Stops the tray running from a place and starts it again when dropped, whichever way the install ends.
struct TrayPause<'a> {
    place: &'a Path,
    was_running: bool,
}

impl<'a> TrayPause<'a> {
    fn stop(place: &'a Path) -> Self {
        Self { place, was_running: stop_tray(place) }
    }
}

impl Drop for TrayPause<'_> {
    fn drop(&mut self) {
        if self.was_running {
            start_tray(self.place);
        }
    }
}

/// The PowerShell that stops every process running from `place` except pdtd, pdt-host, the uninstaller and the
/// process `own_pid` (pdt-host itself); it prints `yes` when it stopped one.
fn tray_stop_script(place: &Path, own_pid: u32) -> String {
    let (daemon, host) = (pdt_core::brand::DAEMON_BIN, pdt_core::brand::HOST_BIN);
    format!(
        "$p = Get-Process | Where-Object {{ $_.Id -ne {own_pid} -and $_.Path -and $_.Path.StartsWith('{}\\', [StringComparison]::OrdinalIgnoreCase) -and $_.ProcessName -notin @('{daemon}', '{host}', 'uninstall') }}; if ($p) {{ $p | Stop-Process -Force; 'yes' }}",
        place.display().to_string().replace('\'', "''")
    )
}

/// From `ps -axo pid=,args=` output: the processes running from `bundle`'s Contents/MacOS other than pdtd, pdt-host
/// and `own_pid`.
fn tray_pids(ps: &str, bundle: &Path, own_pid: u32) -> Vec<u32> {
    let prefix = format!("{}/Contents/MacOS/", bundle.display());
    ps.lines()
        .filter_map(|line| {
            let (pid, args) = line.trim().split_once(char::is_whitespace)?;
            let pid: u32 = pid.parse().ok()?;
            let rest = args.trim_start().strip_prefix(&prefix)?;
            let name = rest.split(' ').next().unwrap_or("");
            (pid != own_pid && name != pdt_core::brand::DAEMON_BIN && name != pdt_core::brand::HOST_BIN).then_some(pid)
        })
        .collect()
}

/// Stops the tray app running from `place` (it locks files on Windows and runs stale code on macOS). Never stops
/// pdtd or pdt-host. Returns whether one ran.
fn stop_tray(place: &Path) -> bool {
    let me = std::process::id();
    if cfg!(windows) {
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &tray_stop_script(place, me)])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("yes"))
            .unwrap_or(false)
    } else {
        let ps = Command::new("ps").args(["-axo", "pid=,args="]).output();
        let pids = ps.map(|o| tray_pids(&String::from_utf8_lossy(&o.stdout), place, me)).unwrap_or_default();
        for pid in &pids {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        !pids.is_empty()
    }
}

/// Starts the tray app again after an install or rollback, asking it not to open the web UI (no new browser window
/// on every update).
fn start_tray(place: &Path) {
    if cfg!(windows) {
        let daemon = pdt_core::brand::DAEMON_BIN;
        let host = pdt_core::brand::HOST_BIN;
        let app = std::fs::read_dir(place).ok().and_then(|rd| {
            rd.flatten().map(|e| e.path()).find(|p| {
                p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                    && p.file_stem()
                        .is_some_and(|s| ![daemon, host, "uninstall"].iter().any(|n| s.eq_ignore_ascii_case(n)))
            })
        });
        if let Some(app) = app {
            let mut cmd = Command::new(app);
            cmd.arg(pdt_core::handoff::TRAY_QUIET_ARG);
            for (k, _) in std::env::vars() {
                if k.starts_with("PDT_") {
                    cmd.env_remove(k);
                }
            }
            let _ = cmd.spawn();
        }
    } else {
        let _ = Command::new("open").args(open_tray_args(place)).status();
    }
}

/// `open`'s arguments for starting the tray bundle quietly.
fn open_tray_args(bundle: &Path) -> Vec<std::ffi::OsString> {
    vec![bundle.into(), "--args".into(), pdt_core::handoff::TRAY_QUIET_ARG.into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nsis_command_line() {
        let d = Path::new(r"C:\Users\me\AppData\Local\Personal Dev Team");
        assert_eq!(nsis_args(d), ["/S", r"/D=C:\Users\me\AppData\Local\Personal Dev Team"]);
    }

    #[test]
    fn bundle_paths() {
        let exe_dir = Path::new("/Users/me/Applications/Personal Dev Team.app/Contents/MacOS");
        let b = bundle_of(exe_dir).unwrap();
        assert_eq!(b, Path::new("/Users/me/Applications/Personal Dev Team.app"));
        assert_eq!(previous_bundle(&b), Path::new("/Users/me/Applications/Personal Dev Team.app.previous"));
        assert_eq!(bundle_of(Path::new("/usr/local/bin")), None);
    }

    #[test]
    fn copy_installer_backs_up_and_restores() {
        let tmp = tempfile::tempdir().unwrap();
        let (target, backup, asset) = (tmp.path().join("t"), tmp.path().join("b"), tmp.path().join("a"));
        std::fs::create_dir_all(target.join("sub")).unwrap();
        std::fs::write(target.join("v.txt"), "old").unwrap();
        std::fs::write(target.join("sub").join("x"), "1").unwrap();
        std::fs::create_dir_all(&asset).unwrap();
        std::fs::write(asset.join("v.txt"), "new").unwrap();
        let i = CopyDirInstaller { target: target.clone(), backup };
        i.backup().unwrap();
        i.install(&asset).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("v.txt")).unwrap(), "new");
        i.rollback().unwrap();
        assert_eq!(std::fs::read_to_string(target.join("v.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(target.join("sub").join("x")).unwrap(), "1");
    }

    #[test]
    fn a_missing_backup_is_never_restored_over_the_install() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("t");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("v.txt"), "current").unwrap();
        let i = CopyDirInstaller { target: target.clone(), backup: tmp.path().join("missing") };
        assert!(i.rollback().is_err());
        assert_eq!(std::fs::read_to_string(target.join("v.txt")).unwrap(), "current");
    }

    #[test]
    fn unsupported_refuses_before_touching_anything() {
        let u = Unsupported { reason: "pdt-host is not installed from an .app bundle".into() };
        let e = u.backup().unwrap_err();
        assert!(e.contains("not supported") && e.contains(".app bundle"), "{e}");
        assert!(u.install(Path::new("x")).is_err());
        assert_eq!(u.backup_path(), None);
    }

    #[test]
    fn the_tray_restarts_without_opening_the_ui() {
        let args = open_tray_args(Path::new("/Applications/P.app"));
        assert_eq!(args, ["/Applications/P.app", "--args", "--no-open"]);
    }

    #[test]
    fn the_tray_stop_never_includes_pdt_host_or_pdtd() {
        let s = tray_stop_script(Path::new(r"C:\Apps\P"), 4242);
        assert!(s.contains("$_.Id -ne 4242"), "{s}");
        assert!(s.contains("@('pdtd', 'pdt-host', 'uninstall')"), "{s}");
        let ps = "  10 /A/P.app/Contents/MacOS/Personal Dev Team --flag\n  11 /A/P.app/Contents/MacOS/pdt-host run\n  12 /A/P.app/Contents/MacOS/pdtd\n  13 /A/P.app/Contents/MacOS/Personal Dev Team\n  14 /other/thing\n";
        assert_eq!(tray_pids(ps, Path::new("/A/P.app"), 13), [10]);
    }

    #[test]
    fn bundle_rollback_restores_the_previous_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("P.app");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("v"), "new").unwrap();
        let prev = previous_bundle(&bundle);
        std::fs::create_dir_all(&prev).unwrap();
        std::fs::write(prev.join("v"), "old").unwrap();
        let i = BundleInstaller { bundle: bundle.clone(), timeout: INSTALL_TIMEOUT };
        i.rollback().unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("v")).unwrap(), "old");
        assert!(!prev.exists() && !with_suffix(&bundle, ".failed").exists());
        // Nothing moved aside: nothing to undo.
        i.rollback().unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("v")).unwrap(), "old");
    }

    #[test]
    fn a_failed_extraction_leaves_the_bundle_and_no_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("P.app");
        std::fs::create_dir_all(&bundle).unwrap();
        let junk = tmp.path().join("junk.tar.gz");
        std::fs::write(&junk, "not an archive").unwrap();
        let i = BundleInstaller { bundle: bundle.clone(), timeout: Duration::from_secs(30) };
        i.backup().unwrap();
        assert!(i.install(&junk).is_err());
        assert!(bundle.is_dir());
        assert!(!tmp.path().join(".pdt-update-staging").exists());
    }
}
