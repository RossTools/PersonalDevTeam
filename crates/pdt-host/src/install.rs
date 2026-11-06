//! Installing a release and rolling it back: the silent NSIS installer on Windows, a bundle swap
//! on macOS, an install folder (and AppImage) swap on Linux ([`crate::linux`]). The previous version is kept: a copy
//! of the install folder (Windows), the old bundle (macOS) or the old folder (Linux).

use crate::config::HostConfig;
use crate::process::no_window;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long the installer (or extracting the archive) may take before it is stopped and the update rolled back.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

/// One update: `prepare`, `backup`, then `install` and `verify`; `rollback` after a failed `install`, `verify` or health
/// check.
pub trait Installer {
    /// Makes sure the install can replace the files in place, before anything changes (pdtd has stopped by then).
    /// Returns what it did, for the log; after an error the update is abandoned.
    fn prepare(&self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
    /// Stops what still runs from the install folder and would keep files from being replaced (Windows); returns what
    /// it stopped, for the log. Called by `prepare`, and by pdt-host before every `rollback`.
    fn stop_strays(&self) -> Vec<String> {
        Vec::new()
    }
    /// Keeps what is needed to restore the current version. Changes nothing that runs: after an error nothing has
    /// changed and the update is abandoned.
    fn backup(&self) -> Result<(), String>;
    /// Installs `asset`. After an error the install may be partial; the caller rolls back.
    fn install(&self, asset: &Path) -> Result<(), String>;
    /// Checks that `install` really put a new version in place (it is rolled back otherwise). Only called when the
    /// update changes the version.
    fn verify(&self) -> Result<(), String> {
        Ok(())
    }
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
        quiet(no_window(Command::new("taskkill").args(["/PID", &pid, "/T", "/F"])));
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
    /// Where the registry keys the installer rewrites (the Settings › Apps entry) are exported, next to `backup`;
    /// `None` leaves the registry alone (tests).
    pub registry_backup: Option<PathBuf>,
    pub timeout: Duration,
}

impl NsisInstaller {
    fn daemon(&self) -> PathBuf {
        daemon_in(&self.install_dir)
    }
}

/// pdtd in an install folder (or its backup).
fn daemon_in(dir: &Path) -> PathBuf {
    dir.join(format!("{}{}", pdt_core::brand::DAEMON_BIN, std::env::consts::EXE_SUFFIX))
}

impl Installer for NsisInstaller {
    /// Stops any pdtd or pdt-host still running from the install folder (PDT-213: a second pdt-host's pdtd kept
    /// pdtd.exe open, so the silent installer "succeeded" without replacing it), then checks pdtd can be replaced.
    fn prepare(&self) -> Result<Vec<String>, String> {
        let stopped = self.stop_strays();
        let notes = stopped_notes(&stopped);
        if let Some(e) = wait_replaceable(&self.daemon(), Duration::from_secs(10)) {
            let stopped = if stopped.is_empty() { String::new() } else { format!(" (stopped {})", stopped.join(", ")) };
            return Err(format!("{} is in use by another program{stopped}: {e}", self.daemon().display()));
        }
        Ok(notes)
    }
    fn stop_strays(&self) -> Vec<String> {
        stop_strays(&self.install_dir)
    }
    /// Copies the install folder and exports the registry keys the installer rewrites (the Settings › Apps entry).
    fn backup(&self) -> Result<(), String> {
        replace_dir(&self.install_dir, &self.backup).map_err(|e| format!("backing up the install folder: {e}"))?;
        match &self.registry_backup {
            Some(dir) => crate::nsis_registry::backup(dir, &mut crate::register::System)
                .map_err(|e| format!("backing up the registry: {e}")),
            None => Ok(()),
        }
    }
    fn install(&self, asset: &Path) -> Result<(), String> {
        let _tray = TrayPause::stop(&self.install_dir);
        let status = run_with_timeout(no_window(Command::new(asset).args(nsis_args(&self.install_dir))), self.timeout)
            .map_err(|e| format!("running the installer: {e}"))?;
        if status.success() { Ok(()) } else { Err(format!("the installer failed ({status})")) }
    }
    fn verify(&self) -> Result<(), String> {
        unchanged_daemon(&self.install_dir, &self.backup)
    }
    fn rollback(&self) -> Result<(), String> {
        let _tray = TrayPause::stop(&self.install_dir);
        replace_dir(&self.backup, &self.install_dir).map_err(|e| format!("restoring the previous version: {e}"))?;
        match &self.registry_backup {
            Some(dir) => crate::nsis_registry::restore(dir, &mut crate::register::System)
                .map_err(|e| format!("restoring the previous version's registry keys: {e}")),
            None => Ok(()),
        }
    }
    fn backup_path(&self) -> Option<PathBuf> {
        Some(self.backup.clone())
    }
}

/// The log lines for the processes `Installer::stop_strays` stopped.
pub fn stopped_notes(stopped: &[String]) -> Vec<String> {
    stopped.iter().map(|p| format!("stopped {p}, which ran from the install folder")).collect()
}

/// An error when the installed pdtd is byte for byte the one backed up: the installer reported success without
/// replacing it (a running copy kept it open, PDT-213).
pub fn unchanged_daemon(install_dir: &Path, backup: &Path) -> Result<(), String> {
    let (now, before) = (daemon_in(install_dir), daemon_in(backup));
    match (crate::selfupdate::digest(&now), crate::selfupdate::digest(&before)) {
        (Some(a), Some(b)) if a == b => Err(format!(
            "the installer did not replace {} (it is still the previous version)",
            now.file_name().unwrap_or_default().to_string_lossy()
        )),
        _ => Ok(()),
    }
}

/// Waits (up to `within`) until `exe` can be opened for writing, as the installer must: `None` once it can (or when
/// it does not exist), else the last error. Windows refuses while a process runs from it; elsewhere a running program
/// never blocks its replacement, so this is not checked.
fn wait_replaceable(exe: &Path, within: Duration) -> Option<String> {
    if !cfg!(windows) || !exe.is_file() {
        return None;
    }
    let end = Instant::now() + within;
    loop {
        match std::fs::File::options().write(true).open(exe) {
            Ok(_) => return None,
            Err(e) if Instant::now() >= end => return Some(e.to_string()),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// The PowerShell that stops every pdtd and pdt-host running from `place` except the process `own_pid` (pdt-host
/// itself, when it runs from there), printing `<name> (process <id>)` for each.
fn stray_stop_script(place: &Path, own_pid: u32) -> String {
    let (daemon, host) = (pdt_core::brand::DAEMON_BIN, pdt_core::brand::HOST_BIN);
    format!(
        "$p = Get-Process | Where-Object {{ $_.Id -ne {own_pid} -and $_.Path -and $_.Path.StartsWith('{}\\', [StringComparison]::OrdinalIgnoreCase) -and $_.ProcessName -in @('{daemon}', '{host}') }}; foreach ($x in $p) {{ \"$($x.ProcessName) (process $($x.Id))\" }}; if ($p) {{ $p | Stop-Process -Force }}",
        place.display().to_string().replace('\'', "''")
    )
}

/// Windows: stops every pdtd and pdt-host still running from the install folder other than this process (by the time
/// an update installs, this pdt-host's own pdtd has stopped, so any left is a stray that would keep the installer from
/// replacing it). Returns the ones it stopped. Elsewhere nothing blocks replacing a running program: does nothing.
fn stop_strays(place: &Path) -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let script = stray_stop_script(place, std::process::id());
    no_window(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &script]))
        .stdin(Stdio::null())
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
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
    /// The Team ID whose Developer ID signature the new `.app` must carry (`brand::MACOS_TEAM_ID`); None checks
    /// nothing (tests, whose bundles are unsigned).
    pub team_id: Option<&'static str>,
}

/// The code requirement an update's `.app` must satisfy: Apple-anchored (a Developer ID), PDT's bundle identifier,
/// signed by `team_id`.
fn bundle_requirement(team_id: &str) -> String {
    format!(
        "anchor apple generic and identifier \"{}\" and certificate leaf[subject.OU] = \"{team_id}\"",
        pdt_core::brand::APP_BUNDLE_ID
    )
}

/// A fresh staging folder beside `target` (same file system, so the swap is a rename), removed when dropped. Its name
/// is random and only its owner can enter it (0700 on Unix): where the parent folder is writable by another user, that
/// user can't swap or edit the extracted files before they are moved into place (PDT-543).
pub(crate) fn staging_beside(target: &Path) -> Result<tempfile::TempDir, String> {
    let parent = target.parent().ok_or_else(|| format!("{} has no parent folder", target.display()))?;
    let mut b = tempfile::Builder::new();
    b.prefix(".pdt-update-staging-");
    #[cfg(unix)]
    b.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
    b.tempdir_in(parent).map_err(|e| format!("making a staging folder in {}: {e}", parent.display()))
}

/// Whether a staging folder is left in `parent`.
#[cfg(test)]
pub(crate) fn staging_left(parent: &Path) -> bool {
    std::fs::read_dir(parent)
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with(".pdt-update-staging"))
}

impl BundleInstaller {
    /// Extracts the archive into `staging` and returns the `.app` in it.
    fn extract(&self, asset: &Path, staging: &Path) -> Result<PathBuf, String> {
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

    /// Defence in depth beside the signed SHA256SUMS (PDT-531): the extracted bundle must pass `codesign --verify`
    /// (deep, strict) against [`bundle_requirement`] before it replaces the running one, since removing the
    /// quarantine attribute means Gatekeeper never looks at it.
    fn verify_signature(&self, app: &Path) -> Result<(), String> {
        let Some(team) = self.team_id else { return Ok(()) };
        let mut cs = Command::new("codesign");
        cs.args(["--verify", "--deep", "--strict"])
            .arg(format!("-R={}", bundle_requirement(team)))
            .arg(app)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if run_with_timeout(&mut cs, self.timeout)?.success() {
            Ok(())
        } else {
            Err(format!("the update's app is not signed by {}'s developer certificate", pdt_core::brand::PRODUCT_NAME))
        }
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
        let staging = staging_beside(&self.bundle)?;
        let new = self.extract(asset, staging.path())?;
        self.verify_signature(&new)?;
        let _ = Command::new("xattr").args(["-dr", "com.apple.quarantine"]).arg(&new).status();
        let _tray = TrayPause::stop(&self.bundle);
        let prev = previous_bundle(&self.bundle);
        std::fs::rename(&self.bundle, &prev).map_err(|e| format!("moving the old bundle aside: {e}")).and_then(|()| {
            std::fs::rename(&new, &self.bundle).map_err(|e| {
                let _ = std::fs::rename(&prev, &self.bundle);
                format!("moving the new bundle into place: {e}")
            })
        })
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
/// installed here (ruling F9) or the host runs the dev profile. A stable host also records the reason in
/// `cfg.updates_unsupported`, so the pdtd it starts never offers an update it would refuse (PDT-51).
pub fn host_installer(cfg: &mut HostConfig) -> Box<dyn Installer + Send> {
    installer_for(cfg).unwrap_or_else(|reason| {
        crate::log::log(cfg, &format!("updates cannot be installed here: {reason}"));
        if !cfg.dev {
            cfg.updates_unsupported = Some(reason.clone());
        }
        Box::new(Unsupported { reason })
    })
}

/// The installer for this platform, or why updates cannot be installed here. A dev host never installs: it may run
/// next to the stable app, and the stable installer must never land in a dev build's folder.
pub fn installer_for(cfg: &HostConfig) -> Result<Box<dyn Installer + Send>, String> {
    if cfg.dev {
        Err("the dev profile never installs updates".into())
    } else if cfg!(windows) {
        let update_dir = pdt_core::handoff::update_dir(&cfg.data_dir);
        Ok(Box::new(NsisInstaller {
            install_dir: cfg.install_dir.clone(),
            backup: update_dir.join("previous"),
            registry_backup: Some(update_dir.join("previous-registry")),
            timeout: INSTALL_TIMEOUT,
        }))
    } else if cfg!(target_os = "macos") {
        let bundle = bundle_of(&cfg.install_dir).ok_or("pdt-host is not installed from an .app bundle")?;
        if let Some(reason) = crate::placement::bundle_problem(&cfg.install_dir) {
            return Err(reason);
        }
        Ok(Box::new(BundleInstaller {
            bundle,
            timeout: INSTALL_TIMEOUT,
            team_id: Some(pdt_core::brand::MACOS_TEAM_ID),
        }))
    } else if cfg!(target_os = "linux") {
        linux_installer(cfg)
    } else {
        Err("updates are not supported on this platform".into())
    }
}

/// Linux: the tarball's folder, or an AppImage's payload folder (it names its AppImage). The folder is moved aside
/// during an update, so its parent must be writable.
fn linux_installer(cfg: &HostConfig) -> Result<Box<dyn Installer + Send>, String> {
    let dir = &cfg.install_dir;
    if !cfg.pdtd_path().is_file() {
        return Err(format!("{} is not an install folder (it has no pdtd)", dir.display()));
    }
    let parent = dir.parent().ok_or("the install folder has no parent folder")?;
    if !crate::linux::writable(parent) {
        return Err(format!("{} is not writable, so the install folder cannot be replaced", parent.display()));
    }
    let appimage = crate::linux::appimage_of(dir);
    if let Some(a) = appimage.as_ref().and_then(|a| a.parent())
        && !crate::linux::writable(a)
    {
        return Err(format!("{} is not writable, so the AppImage cannot be replaced", a.display()));
    }
    Ok(Box::new(crate::linux::LinuxInstaller { dir: dir.clone(), appimage, timeout: INSTALL_TIMEOUT }))
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
        no_window(Command::new("powershell.exe").args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &tray_stop_script(place, me),
        ]))
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
    fn each_update_stages_in_its_own_private_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("P.app");
        let (a, b) = (staging_beside(&target).unwrap(), staging_beside(&target).unwrap());
        assert_ne!(a.path(), b.path(), "a fixed name could be prepared by another user");
        assert_eq!(a.path().parent(), Some(tmp.path()), "beside the target, so the swap is a rename");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(a.path()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        drop((a, b));
        assert!(!staging_left(tmp.path()));
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

    #[cfg(target_os = "macos")]
    #[test]
    fn a_translocated_bundle_gets_no_installer_and_pdtd_is_told() {
        let tmp = tempfile::tempdir().unwrap();
        let place = |root: &Path| root.join("P.app").join("Contents").join("MacOS");
        let cfg = |dir: PathBuf| {
            let data = tmp.path().join("data").to_string_lossy().into_owned();
            HostConfig::from_lookup(move |k| (k == "PDT_DATA_DIR").then(|| data.clone()), None, dir).unwrap()
        };
        let fine = place(tmp.path());
        std::fs::create_dir_all(&fine).unwrap();
        assert!(installer_for(&cfg(fine)).is_ok());

        let moved = place(&tmp.path().join("AppTranslocation").join("u").join("d"));
        std::fs::create_dir_all(&moved).unwrap();
        let mut c = cfg(moved);
        let err = installer_for(&c).err().unwrap();
        assert!(err.contains("temporary read-only folder"), "{err}");
        assert_eq!(host_installer(&mut c).backup_path(), None);
        assert_eq!(c.updates_unsupported.as_deref(), Some(err.as_str()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_installs_into_a_folder_with_pdtd_and_names_its_appimage() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = |dir: PathBuf| {
            let data = tmp.path().join("data").to_string_lossy().into_owned();
            HostConfig::from_lookup(move |k| (k == "PDT_DATA_DIR").then(|| data.clone()), None, dir).unwrap()
        };
        let dir = tmp.path().join("pdt");
        std::fs::create_dir_all(&dir).unwrap();
        let err = installer_for(&cfg(dir.clone())).err().unwrap();
        assert!(err.contains("no pdtd"), "{err}");
        std::fs::write(dir.join("pdtd"), "x").unwrap();
        let i = installer_for(&cfg(dir.clone())).unwrap();
        assert_eq!(i.backup_path(), Some(tmp.path().join("pdt.previous")));
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
    fn the_stray_stop_takes_only_pdtd_and_pdt_host_from_the_install_folder() {
        let s = stray_stop_script(Path::new(r"C:\Apps\P's"), 4242);
        assert!(s.contains("$_.Id -ne 4242"), "{s}");
        assert!(s.contains(r"StartsWith('C:\Apps\P''s\'"), "{s}");
        assert!(s.contains("-in @('pdtd', 'pdt-host')"), "{s}");
        assert!(s.contains("(process $($x.Id))"), "{s}");
    }

    #[test]
    fn an_installer_that_left_pdtd_in_place_is_caught() {
        let tmp = tempfile::tempdir().unwrap();
        let (install, backup) = (tmp.path().join("app"), tmp.path().join("previous"));
        for d in [&install, &backup] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(daemon_in(d), "v1").unwrap();
        }
        let err = unchanged_daemon(&install, &backup).unwrap_err();
        assert!(err.contains("did not replace pdtd") && err.contains("previous version"), "{err}");
        std::fs::write(daemon_in(&install), "v2").unwrap();
        assert_eq!(unchanged_daemon(&install, &backup), Ok(()));
        assert_eq!(unchanged_daemon(&install, &tmp.path().join("none")), Ok(()), "no backup: nothing to compare");
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
        let i = BundleInstaller { bundle: bundle.clone(), timeout: INSTALL_TIMEOUT, team_id: None };
        i.rollback().unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("v")).unwrap(), "old");
        assert!(!prev.exists() && !with_suffix(&bundle, ".failed").exists());
        // Nothing moved aside: nothing to undo.
        i.rollback().unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("v")).unwrap(), "old");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_app_not_signed_by_the_team_is_never_swapped_in() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("P.app");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("v"), "old").unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("New.app/Contents/MacOS")).unwrap();
        std::fs::write(src.join("New.app/Contents/MacOS/x"), "trojan").unwrap();
        let archive = tmp.path().join("new.app.tar.gz");
        let tar = Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(&src).arg("New.app").status().unwrap();
        assert!(tar.success());
        let i =
            BundleInstaller { bundle: bundle.clone(), timeout: Duration::from_secs(30), team_id: Some("ABCDE12345") };
        i.backup().unwrap();
        let err = i.install(&archive).unwrap_err();
        assert!(err.contains("not signed"), "{err}");
        assert_eq!(std::fs::read_to_string(bundle.join("v")).unwrap(), "old");
        assert!(!previous_bundle(&bundle).exists() && !staging_left(tmp.path()));
        // Without a team the same bundle goes in (what the other tests rely on).
        let i = BundleInstaller { team_id: None, ..i };
        i.install(&archive).unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("Contents/MacOS/x")).unwrap(), "trojan");
    }

    #[test]
    fn a_failed_extraction_leaves_the_bundle_and_no_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("P.app");
        std::fs::create_dir_all(&bundle).unwrap();
        let junk = tmp.path().join("junk.tar.gz");
        std::fs::write(&junk, "not an archive").unwrap();
        let i = BundleInstaller { bundle: bundle.clone(), timeout: Duration::from_secs(30), team_id: None };
        i.backup().unwrap();
        assert!(i.install(&junk).is_err());
        assert!(bundle.is_dir());
        assert!(!staging_left(tmp.path()));
    }
}
