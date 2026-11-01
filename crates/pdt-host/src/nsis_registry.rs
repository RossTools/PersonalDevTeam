//! The registry keys the Windows installer writes, kept with the install folder's backup so a rollback puts them back
//! (PDT-357): without them Settings › Apps shows the newer version after a rollback.

use crate::register::Runner;
use pdt_core::brand::{APP_BUNDLE_ID, PRODUCT_NAME};
use std::path::Path;
use std::process::Command;

/// The keys Tauri's NSIS installer writes for a current-user install (tauri-bundler `installer.nsi`), each with the
/// file its backup is exported to: `UNINSTKEY`, the Settings › Apps entry, and `MANUPRODUCTKEY`, the install folder.
/// The manufacturer is the identifier's second part, as tauri-bundler takes it when no publisher is set.
pub fn keys() -> [(String, &'static str); 2] {
    let manufacturer = APP_BUNDLE_ID.split('.').nth(1).unwrap_or(APP_BUNDLE_ID);
    [
        (format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{PRODUCT_NAME}"), "uninstall.reg"),
        (format!(r"HKCU\Software\{manufacturer}\{PRODUCT_NAME}"), "product.reg"),
    ]
}

fn reg(args: &[&str]) -> Command {
    let mut c = Command::new("reg");
    c.args(args);
    c
}

/// Exports each key that exists into `dir` (made afresh). A missing key is not an error: it gets no file, and
/// [`restore`] removes it.
pub fn backup(dir: &Path, runner: &mut dyn Runner) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| format!("clearing {}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    for (key, file) in keys() {
        if !runner.status(reg(&["query", &key]))? {
            continue;
        }
        let to = dir.join(file).display().to_string();
        if !runner.status(reg(&["export", &key, &to, "/y"]))? {
            return Err(format!("reg export {key} failed"));
        }
    }
    Ok(())
}

/// Puts the keys back as [`backup`] found them: each is deleted, then imported from its file when it had one. Does
/// nothing when there is no backup in `dir` (one made before keys were kept).
pub fn restore(dir: &Path, runner: &mut dyn Runner) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    for (key, file) in keys() {
        // A missing key fails; it is gone either way.
        let _ = runner.status(reg(&["delete", &key, "/f"]))?;
        let from = dir.join(file);
        if from.is_file() && !runner.status(reg(&["import", &from.display().to_string()]))? {
            return Err(format!("reg import {} failed", from.display()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records command lines; `reg query` succeeds for the keys in `present`, `reg export` writes its file.
    #[derive(Default)]
    struct Recorder {
        ran: Vec<String>,
        present: Vec<String>,
    }

    impl Runner for Recorder {
        fn status(&mut self, cmd: Command) -> Result<bool, String> {
            let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
            self.ran.push(format!("reg | {}", args.join(" | ")));
            Ok(match args[0].as_str() {
                "query" => self.present.contains(&args[1]),
                "export" => std::fs::write(&args[2], "").is_ok(),
                _ => true,
            })
        }
        fn stdout(&mut self, _: Command) -> Result<String, String> {
            unreachable!()
        }
        fn spawn(&mut self, _: Command) -> Result<(), String> {
            unreachable!()
        }
    }

    const UNINSTALL: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Personal Dev Team";
    const PRODUCT: &str = r"HKCU\Software\personaldevteam\Personal Dev Team";

    #[test]
    fn the_keys_are_the_ones_tauris_installer_writes() {
        let k = keys();
        assert_eq!([k[0].0.as_str(), k[1].0.as_str()], [UNINSTALL, PRODUCT]);
    }

    #[test]
    fn backup_exports_the_keys_that_exist_and_restore_puts_them_back() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("previous-registry");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("product.reg"), "stale").unwrap();
        let mut r = Recorder { present: vec![UNINSTALL.into()], ..Default::default() };
        backup(&dir, &mut r).unwrap();
        let uninstall = dir.join("uninstall.reg").display().to_string();
        assert_eq!(
            r.ran,
            [
                format!("reg | query | {UNINSTALL}"),
                format!("reg | export | {UNINSTALL} | {uninstall} | /y"),
                format!("reg | query | {PRODUCT}"),
            ]
        );
        assert!(!dir.join("product.reg").exists(), "a stale export of a key that is gone now is removed");

        let mut r = Recorder::default();
        restore(&dir, &mut r).unwrap();
        assert_eq!(
            r.ran,
            [
                format!("reg | delete | {UNINSTALL} | /f"),
                format!("reg | import | {uninstall}"),
                format!("reg | delete | {PRODUCT} | /f"),
            ],
            "a key that was missing before the update is removed again"
        );
    }

    #[test]
    fn without_a_backup_restore_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = Recorder::default();
        restore(&tmp.path().join("none"), &mut r).unwrap();
        assert!(r.ran.is_empty());
    }
}
