//! Linux packages (PDT-197): the headless tarball (a `pdt/` folder with pdtd, pdt-host and `web/`) and the AppImage.
//! Both run pdt-host and pdtd from a plain install folder: the tarball's wherever it was unpacked, the AppImage's
//! payload copied to `<data dir>/app` (a mounted AppImage disappears when its process exits, and an update must
//! change the pdtd pdt-host starts next). That folder holds [`APPIMAGE_MARKER`] with the AppImage's path when it came
//! from one.
//!
//! An update swaps the whole folder, as macOS swaps the `.app`: the old one is moved aside to `<dir>.previous` and
//! the new one moved into place; for the AppImage the AppImage file is swapped the same way (`<file>.previous`).

use crate::install::{Installer, copy_dir, run_with_timeout};
use pdt_core::brand::{DAEMON_BIN, HOST_BIN};
use pdt_core::release::APPIMAGE_MARKER;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Where an AppImage's payload is installed: `<data dir>/app`.
pub fn appimage_install_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("app")
}

/// The AppImage an install folder came from, if it came from one.
pub fn appimage_of(install_dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(install_dir.join(APPIMAGE_MARKER)).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// The web UI beside the programs in `bin_dir`: `web/` (the tarball), or the AppImage's resources
/// (`usr/bin` → `usr/lib/<app>/web`).
pub fn payload_web(bin_dir: &Path) -> Option<PathBuf> {
    let beside = bin_dir.join("web");
    if beside.join("index.html").is_file() {
        return Some(beside);
    }
    let lib = bin_dir.parent()?.join("lib");
    let mut found: Vec<PathBuf> = std::fs::read_dir(lib)
        .ok()?
        .flatten()
        .map(|e| e.path().join("web"))
        .filter(|w| w.join("index.html").is_file())
        .collect();
    found.sort();
    found.into_iter().next()
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

pub fn previous(p: &Path) -> PathBuf {
    with_suffix(p, ".previous")
}

/// Makes `dest` an install folder from the programs in `bin_dir`: pdtd, pdt-host and the web UI, and the AppImage's
/// path when `appimage` is given. `dest` must not exist.
pub fn assemble_payload(bin_dir: &Path, dest: &Path, appimage: Option<&Path>) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("creating {}: {e}", dest.display()))?;
    for name in [DAEMON_BIN, HOST_BIN] {
        let from = bin_dir.join(name);
        // std::fs::copy keeps the permission bits, so they stay executable.
        std::fs::copy(&from, dest.join(name)).map_err(|e| format!("copying {}: {e}", from.display()))?;
    }
    let web = payload_web(bin_dir).ok_or_else(|| format!("no web UI beside {}", bin_dir.display()))?;
    copy_dir(&web, &dest.join("web")).map_err(|e| format!("copying the web UI: {e}"))?;
    if let Some(a) = appimage {
        std::fs::write(dest.join(APPIMAGE_MARKER), a.to_string_lossy().as_bytes())
            .map_err(|e| format!("recording the AppImage: {e}"))?;
    }
    Ok(())
}

/// `pdt-host install --appimage <file>`, run from the mounted AppImage (`bin_dir`): copies its payload to
/// `install_dir`, replacing what is there (no pdt-host may be running from it).
pub fn install_payload(bin_dir: &Path, install_dir: &Path, appimage: &Path) -> Result<(), String> {
    let staged = with_suffix(install_dir, ".new");
    let _ = std::fs::remove_dir_all(&staged);
    if let Err(e) = assemble_payload(bin_dir, &staged, Some(appimage)) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    if install_dir.exists() {
        std::fs::remove_dir_all(install_dir).map_err(|e| format!("removing {}: {e}", install_dir.display()))?;
    }
    std::fs::rename(&staged, install_dir).map_err(|e| format!("moving the app into {}: {e}", install_dir.display()))
}

/// Whether pdt-host can create files in `dir` (an update moves the install folder within it).
pub fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".{HOST_BIN}-write-test-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

#[cfg(unix)]
fn make_executable(p: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Moves `current` to `old` and `new` to `current`; puts `current` back if the second move fails. A missing
/// `current` is simply replaced.
fn swap(current: &Path, new: &Path, old: &Path) -> Result<(), String> {
    let had = current.exists();
    if had {
        std::fs::rename(current, old).map_err(|e| format!("moving {} aside: {e}", current.display()))?;
    }
    std::fs::rename(new, current).map_err(|e| {
        if had {
            let _ = std::fs::rename(old, current);
        }
        format!("moving the new version to {}: {e}", current.display())
    })
}

/// Puts `old` back at `current`, keeping `current` aside until that worked. Nothing to do without `old`.
fn restore(current: &Path, old: &Path) -> Result<(), String> {
    if !old.exists() {
        return Ok(());
    }
    let failed = with_suffix(current, ".failed");
    let remove = |p: &Path| if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
    let _ = remove(&failed);
    if current.exists() {
        std::fs::rename(current, &failed).map_err(|e| format!("moving the failed version aside: {e}"))?;
    }
    if let Err(e) = std::fs::rename(old, current) {
        let _ = std::fs::rename(&failed, current);
        return Err(format!("restoring {}: {e}", current.display()));
    }
    let _ = remove(&failed);
    Ok(())
}

/// The Linux updater: the tarball (`appimage: None`) or an AppImage (`appimage`: the file to replace).
pub struct LinuxInstaller {
    /// The install folder pdt-host and pdtd run from.
    pub dir: PathBuf,
    pub appimage: Option<PathBuf>,
    pub timeout: Duration,
}

impl LinuxInstaller {
    fn staging(&self) -> Result<PathBuf, String> {
        Ok(self.dir.parent().ok_or("the install folder has no parent folder")?.join(".pdt-update-staging"))
    }

    /// The tarball's folder: the one top-level folder in it that holds pdtd.
    fn unpack_tarball(&self, asset: &Path, staging: &Path) -> Result<PathBuf, String> {
        let mut tar = Command::new("tar");
        tar.arg("-xzf").arg(asset).arg("-C").arg(staging).stdout(Stdio::null()).stderr(Stdio::null());
        if !run_with_timeout(&mut tar, self.timeout)?.success() {
            return Err("could not extract the update".into());
        }
        std::fs::read_dir(staging)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.join(DAEMON_BIN).is_file())
            .ok_or_else(|| format!("the archive has no folder with {DAEMON_BIN}"))
    }

    /// Extracts the new AppImage (`--appimage-extract`, which needs no FUSE) and assembles its payload; returns the
    /// payload folder and the AppImage, made executable, beside the one it replaces.
    fn unpack_appimage(&self, asset: &Path, staging: &Path, appimage: &Path) -> Result<(PathBuf, PathBuf), String> {
        let image = staging.join("new.AppImage");
        std::fs::copy(asset, &image).map_err(|e| format!("copying the AppImage: {e}"))?;
        make_executable(&image).map_err(|e| e.to_string())?;
        let mut extract = Command::new(&image);
        extract.arg("--appimage-extract").current_dir(staging).stdout(Stdio::null()).stderr(Stdio::null());
        if !run_with_timeout(&mut extract, self.timeout)?.success() {
            return Err("could not extract the AppImage".into());
        }
        let payload = staging.join("app");
        assemble_payload(&staging.join("squashfs-root").join("usr").join("bin"), &payload, Some(appimage))?;
        let beside = with_suffix(appimage, ".new");
        let _ = std::fs::remove_file(&beside);
        std::fs::copy(&image, &beside).map_err(|e| format!("copying the AppImage to {}: {e}", beside.display()))?;
        make_executable(&beside).map_err(|e| e.to_string())?;
        Ok((payload, beside))
    }
}

impl Installer for LinuxInstaller {
    /// The old version is kept by moving it aside during `install`; here only a stale `.previous` from an earlier
    /// update is removed, so `rollback` never restores an older version than the one running now.
    fn backup(&self) -> Result<(), String> {
        let prev = previous(&self.dir);
        if prev.exists() {
            std::fs::remove_dir_all(&prev).map_err(|e| format!("removing {}: {e}", prev.display()))?;
        }
        if let Some(a) = &self.appimage {
            let prev = previous(a);
            if prev.exists() {
                std::fs::remove_file(&prev).map_err(|e| format!("removing {}: {e}", prev.display()))?;
            }
        }
        Ok(())
    }

    fn install(&self, asset: &Path) -> Result<(), String> {
        let staging = self.staging()?;
        let _ = std::fs::remove_dir_all(&staging);
        let staged = std::fs::create_dir_all(&staging).map_err(|e| e.to_string()).and_then(|()| match &self.appimage {
            None => self.unpack_tarball(asset, &staging).map(|dir| (dir, None)),
            Some(a) => self.unpack_appimage(asset, &staging, a).map(|(dir, image)| (dir, Some(image))),
        });
        let result = staged.and_then(|(new, image)| {
            swap(&self.dir, &new, &previous(&self.dir))?;
            if let (Some(a), Some(image)) = (&self.appimage, image) {
                swap(a, &image, &previous(a))?;
            }
            Ok(())
        });
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    fn rollback(&self) -> Result<(), String> {
        if !previous(&self.dir).exists() && !self.dir.exists() {
            return Err("there is no previous version to restore".into());
        }
        let dir = restore(&self.dir, &previous(&self.dir));
        let image = self.appimage.as_ref().map_or(Ok(()), |a| {
            let _ = std::fs::remove_file(with_suffix(a, ".new"));
            restore(a, &previous(a))
        });
        dir.and(image)
    }

    fn backup_path(&self) -> Option<PathBuf> {
        Some(previous(&self.dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tarball-style install folder at `dir` reporting `version`.
    fn folder(dir: &Path, version: &str) {
        std::fs::create_dir_all(dir.join("web")).unwrap();
        std::fs::write(dir.join(DAEMON_BIN), format!("pdtd {version}")).unwrap();
        std::fs::write(dir.join(HOST_BIN), format!("host {version}")).unwrap();
        std::fs::write(dir.join("web").join("index.html"), version).unwrap();
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn appimage_payload_is_assembled_from_the_mounted_image() {
        let tmp = tempfile::tempdir().unwrap();
        let usr = tmp.path().join("mount").join("usr");
        let bin = usr.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(DAEMON_BIN), "pdtd").unwrap();
        std::fs::write(bin.join(HOST_BIN), "host").unwrap();
        std::fs::write(bin.join("personal-dev-team"), "tray").unwrap();
        let web = usr.join("lib").join("Personal Dev Team").join("web");
        std::fs::create_dir_all(web.join("assets")).unwrap();
        std::fs::write(web.join("index.html"), "ui").unwrap();
        std::fs::write(web.join("assets").join("a.js"), "js").unwrap();
        assert_eq!(payload_web(&bin), Some(web));

        let app = appimage_install_dir(&tmp.path().join("data"));
        let image = tmp.path().join("Apps").join("pdt-linux-x86_64.AppImage");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("stale"), "x").unwrap();
        install_payload(&bin, &app, &image).unwrap();
        assert_eq!(read(&app.join(DAEMON_BIN)), "pdtd");
        assert_eq!(read(&app.join(HOST_BIN)), "host");
        assert_eq!(read(&app.join("web").join("assets").join("a.js")), "js");
        assert!(!app.join("personal-dev-team").exists() && !app.join("stale").exists());
        assert_eq!(appimage_of(&app), Some(image));
        assert!(!with_suffix(&app, ".new").exists());

        // A tarball folder is not an AppImage's.
        let plain = tmp.path().join("pdt");
        folder(&plain, "1");
        assert_eq!(appimage_of(&plain), None);
        assert_eq!(payload_web(&plain), Some(plain.join("web")));
    }

    #[test]
    fn a_missing_web_ui_fails_the_payload_and_leaves_the_install() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(DAEMON_BIN), "pdtd").unwrap();
        std::fs::write(bin.join(HOST_BIN), "host").unwrap();
        let app = tmp.path().join("app");
        folder(&app, "1");
        let err = install_payload(&bin, &app, Path::new("/x.AppImage")).unwrap_err();
        assert!(err.contains("no web UI"), "{err}");
        assert_eq!(read(&app.join(DAEMON_BIN)), "pdtd 1");
        assert!(!with_suffix(&app, ".new").exists());
    }

    /// `pdt/` (pdtd, pdt-host, web) tarred as the release's tarball is.
    #[cfg(unix)]
    fn tarball(tmp: &Path, version: &str) -> PathBuf {
        let src = tmp.join(format!("src-{version}"));
        folder(&src.join("pdt"), version);
        let out = tmp.join(format!("pdt-linux-{version}.tar.gz"));
        assert!(Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(&src).arg("pdt").status().unwrap().success());
        out
    }

    #[cfg(unix)]
    #[test]
    fn tarball_update_swaps_the_folder_and_rolls_back() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("opt").join("pdt");
        folder(&dir, "1");
        let i = LinuxInstaller { dir: dir.clone(), appimage: None, timeout: Duration::from_secs(30) };
        // A stale previous version from an earlier update is never restored.
        folder(&previous(&dir), "0");
        i.backup().unwrap();
        assert!(!previous(&dir).exists());

        i.install(&tarball(tmp.path(), "2")).unwrap();
        assert_eq!(read(&dir.join(DAEMON_BIN)), "pdtd 2");
        assert_eq!(read(&dir.join("web").join("index.html")), "2");
        assert_eq!(read(&previous(&dir).join(DAEMON_BIN)), "pdtd 1");
        assert!(!tmp.path().join("opt").join(".pdt-update-staging").exists());
        assert_eq!(i.backup_path(), Some(previous(&dir)));

        i.rollback().unwrap();
        assert_eq!(read(&dir.join(DAEMON_BIN)), "pdtd 1");
        assert!(!previous(&dir).exists() && !with_suffix(&dir, ".failed").exists());
        // Nothing moved aside: nothing to undo.
        i.rollback().unwrap();
        assert_eq!(read(&dir.join(DAEMON_BIN)), "pdtd 1");
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_tarball_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pdt");
        folder(&dir, "1");
        let junk = tmp.path().join("junk.tar.gz");
        std::fs::write(&junk, "not an archive").unwrap();
        let i = LinuxInstaller { dir: dir.clone(), appimage: None, timeout: Duration::from_secs(30) };
        i.backup().unwrap();
        assert!(i.install(&junk).is_err());
        assert_eq!(read(&dir.join(DAEMON_BIN)), "pdtd 1");
        assert!(!previous(&dir).exists() && !tmp.path().join(".pdt-update-staging").exists());
        i.rollback().unwrap();
        assert_eq!(read(&dir.join(DAEMON_BIN)), "pdtd 1");
    }

    /// A stand-in AppImage: a script that, given `--appimage-extract`, writes the layout a Tauri AppImage has.
    #[cfg(unix)]
    fn fake_appimage(tmp: &Path, version: &str) -> PathBuf {
        let p = tmp.join(format!("new-{version}.AppImage"));
        let script = format!(
            "#!/bin/sh\n[ \"$1\" = --appimage-extract ] || exit 1\n\
             mkdir -p squashfs-root/usr/bin \"squashfs-root/usr/lib/Personal Dev Team/web\"\n\
             echo 'pdtd {version}' > squashfs-root/usr/bin/{DAEMON_BIN}\n\
             echo 'host {version}' > squashfs-root/usr/bin/{HOST_BIN}\n\
             echo '{version}' > \"squashfs-root/usr/lib/Personal Dev Team/web/index.html\"\n\
             # image {version}\n"
        );
        std::fs::write(&p, script).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn appimage_update_swaps_the_payload_and_the_file_and_rolls_back_both() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = appimage_install_dir(&tmp.path().join("data"));
        let image = tmp.path().join("Applications").join("pdt-linux-x86_64.AppImage");
        std::fs::create_dir_all(image.parent().unwrap()).unwrap();
        std::fs::write(&image, "image 1").unwrap();
        folder(&dir, "1");
        std::fs::write(dir.join(APPIMAGE_MARKER), image.to_string_lossy().as_bytes()).unwrap();
        let i = LinuxInstaller { dir: dir.clone(), appimage: Some(image.clone()), timeout: Duration::from_secs(30) };
        i.backup().unwrap();
        i.install(&fake_appimage(tmp.path(), "2")).unwrap();
        assert_eq!(read(&dir.join(DAEMON_BIN)), "pdtd 2\n");
        assert_eq!(read(&dir.join("web").join("index.html")), "2\n");
        assert_eq!(appimage_of(&dir), Some(image.clone()), "the new payload still names the AppImage");
        assert!(read(&image).contains("# image 2"));
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&image).unwrap().permissions().mode() & 0o111, 0o111, "executable");
        }
        assert_eq!(read(&previous(&image)), "image 1");

        i.rollback().unwrap();
        assert_eq!(read(&dir.join(DAEMON_BIN)), "pdtd 1");
        assert_eq!(read(&image), "image 1");
        assert!(!previous(&image).exists() && !previous(&dir).exists());
    }
}
