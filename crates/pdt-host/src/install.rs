//! Installing a release and rolling it back (spec §3.4 steps 3–4): the silent NSIS installer on Windows, a bundle swap
//! on macOS. The previous version is kept: a copy of the install folder (Windows) or the old bundle (macOS).

use crate::config::HostConfig;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub trait Installer {
    fn install(&self, asset: &Path) -> Result<(), String>;
    fn rollback(&self) -> Result<(), String>;
}

pub fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

/// Makes `dst` an exact copy of `src`.
pub fn replace_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    if dst.exists() {
        for e in std::fs::read_dir(dst)? {
            let p = e?.path();
            if p.is_dir() { std::fs::remove_dir_all(&p)? } else { std::fs::remove_file(&p)? }
        }
    }
    copy_dir(src, dst)
}

/// Tests (and the basis of the Windows backup): the asset is a folder whose contents are copied over `target`.
pub struct CopyDirInstaller {
    pub target: PathBuf,
    pub backup: PathBuf,
}

impl Installer for CopyDirInstaller {
    fn install(&self, asset: &Path) -> Result<(), String> {
        replace_dir(&self.target, &self.backup).map_err(|e| format!("backing up {}: {e}", self.target.display()))?;
        copy_dir(asset, &self.target).map_err(|e| format!("copying the new version: {e}"))
    }
    fn rollback(&self) -> Result<(), String> {
        replace_dir(&self.backup, &self.target).map_err(|e| format!("restoring the previous version: {e}"))
    }
}

/// Used when this host cannot install updates (no installer or bundle found, ruling F9): pdt-host keeps supervising
/// pdtd and refuses every update with `reason`.
pub struct Unsupported {
    pub reason: String,
}

impl Installer for Unsupported {
    fn install(&self, _asset: &Path) -> Result<(), String> {
        Err(format!("updates are not supported on this host: {}", self.reason))
    }
    fn rollback(&self) -> Result<(), String> {
        Err("nothing was installed, so there is nothing to roll back".into())
    }
}

/// NSIS: `/S` silent, `/D=` the install folder (must be last and unquoted).
pub fn nsis_args(install_dir: &Path) -> Vec<String> {
    vec!["/S".into(), format!("/D={}", install_dir.display())]
}

pub struct NsisInstaller {
    pub install_dir: PathBuf,
    pub backup: PathBuf,
}

impl Installer for NsisInstaller {
    fn install(&self, asset: &Path) -> Result<(), String> {
        let tray = stop_tray(&self.install_dir);
        replace_dir(&self.install_dir, &self.backup).map_err(|e| format!("backing up the install folder: {e}"))?;
        let status = Command::new(asset)
            .args(nsis_args(&self.install_dir))
            .stdin(Stdio::null())
            .status()
            .map_err(|e| format!("running the installer: {e}"))?;
        if tray {
            start_tray(&self.install_dir);
        }
        if status.success() { Ok(()) } else { Err(format!("the installer failed ({status})")) }
    }
    fn rollback(&self) -> Result<(), String> {
        let tray = stop_tray(&self.install_dir);
        let r =
            replace_dir(&self.backup, &self.install_dir).map_err(|e| format!("restoring the previous version: {e}"));
        if tray {
            start_tray(&self.install_dir);
        }
        r
    }
}

/// The bundle that contains `exe_dir` (`…/X.app/Contents/MacOS` → `…/X.app`).
pub fn bundle_of(exe_dir: &Path) -> Option<PathBuf> {
    exe_dir.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

pub fn previous_bundle(bundle: &Path) -> PathBuf {
    let mut s = bundle.as_os_str().to_owned();
    s.push(".previous");
    PathBuf::from(s)
}

pub struct BundleInstaller {
    pub bundle: PathBuf,
}

impl Installer for BundleInstaller {
    fn install(&self, asset: &Path) -> Result<(), String> {
        let parent = self.bundle.parent().ok_or("the bundle has no parent folder")?;
        let staging = parent.join(".pdt-update-staging");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
        let ok =
            Command::new("tar").arg("-xzf").arg(asset).arg("-C").arg(&staging).status().map_err(|e| e.to_string())?;
        if !ok.success() {
            return Err("could not extract the update".into());
        }
        let new = std::fs::read_dir(&staging)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or("the archive has no .app bundle")?;
        let _ = Command::new("xattr").args(["-dr", "com.apple.quarantine"]).arg(&new).status();
        let tray = stop_tray(&self.bundle);
        let prev = previous_bundle(&self.bundle);
        let _ = std::fs::remove_dir_all(&prev);
        std::fs::rename(&self.bundle, &prev).map_err(|e| format!("moving the old bundle aside: {e}"))?;
        if let Err(e) = std::fs::rename(&new, &self.bundle) {
            let _ = std::fs::rename(&prev, &self.bundle);
            return Err(format!("moving the new bundle into place: {e}"));
        }
        let _ = std::fs::remove_dir_all(&staging);
        if tray {
            start_tray(&self.bundle);
        }
        Ok(())
    }
    fn rollback(&self) -> Result<(), String> {
        let prev = previous_bundle(&self.bundle);
        if !prev.exists() {
            return Err("there is no previous bundle to restore".into());
        }
        let tray = stop_tray(&self.bundle);
        std::fs::remove_dir_all(&self.bundle).map_err(|e| e.to_string())?;
        std::fs::rename(&prev, &self.bundle).map_err(|e| e.to_string())?;
        if tray {
            start_tray(&self.bundle);
        }
        Ok(())
    }
}

/// The installer for this platform, or why updates cannot be installed here (pdt-host then uses `Unsupported`).
pub fn installer_for(cfg: &HostConfig) -> Result<Box<dyn Installer>, String> {
    if cfg!(windows) {
        Ok(Box::new(NsisInstaller {
            install_dir: cfg.install_dir.clone(),
            backup: pdt_core::handoff::update_dir(&cfg.data_dir).join("previous"),
        }))
    } else if cfg!(target_os = "macos") {
        let bundle = bundle_of(&cfg.install_dir).ok_or("pdt-host is not installed from an .app bundle")?;
        Ok(Box::new(BundleInstaller { bundle }))
    } else {
        Err("updates are not supported on this platform".into())
    }
}

/// Stops the tray app running from `place` (it locks files on Windows and runs stale code on macOS). pdtd has already
/// exited and pdt-host runs from the data folder, so everything else there is the tray. Returns whether one ran.
fn stop_tray(place: &Path) -> bool {
    let daemon = pdt_core::brand::DAEMON_BIN;
    if cfg!(windows) {
        let script = format!(
            "$p = Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith('{}\\', [StringComparison]::OrdinalIgnoreCase) -and $_.ProcessName -notin @('{daemon}', 'uninstall') }}; if ($p) {{ $p | Stop-Process -Force; 'yes' }}",
            place.display().to_string().replace('\'', "''")
        );
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("yes"))
            .unwrap_or(false)
    } else {
        let pattern = format!("{}/Contents/MacOS/", place.display());
        let found =
            Command::new("pgrep").args(["-f", &pattern]).output().map(|o| !o.stdout.is_empty()).unwrap_or(false);
        if found {
            let _ = Command::new("pkill").args(["-f", &pattern]).status();
        }
        found
    }
}

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
            for (k, _) in std::env::vars() {
                if k.starts_with("PDT_") {
                    cmd.env_remove(k);
                }
            }
            let _ = cmd.spawn();
        }
    } else {
        let _ = Command::new("open").arg(place).status();
    }
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
        i.install(&asset).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("v.txt")).unwrap(), "new");
        i.rollback().unwrap();
        assert_eq!(std::fs::read_to_string(target.join("v.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(target.join("sub").join("x")).unwrap(), "1");
    }

    #[test]
    fn unsupported_refuses_with_its_reason() {
        let u = Unsupported { reason: "pdt-host is not installed from an .app bundle".into() };
        let e = u.install(Path::new("x")).unwrap_err();
        assert!(e.contains("not supported") && e.contains(".app bundle"), "{e}");
        assert!(u.rollback().is_err());
    }
}
