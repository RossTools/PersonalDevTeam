//! Spec §3.4 step 6: when a release changes pdt-host itself, it replaces its own copy last and restarts.

use sha2::Digest;
use std::io::Read;
use std::path::{Path, PathBuf};

fn sha(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(hex::encode(h.finalize()))
}

/// The bundled copy differs from the one running.
pub fn needs_replace(running: &Path, bundled: &Path) -> bool {
    match (sha(running), sha(bundled)) {
        (Some(a), Some(b)) => a != b,
        _ => false,
    }
}

pub fn old_path(running: &Path) -> PathBuf {
    running.with_extension(if cfg!(windows) { "old.exe" } else { "old" })
}

/// Swaps in `bundled` at `running`'s path. Windows can rename a running exe but not overwrite it; unix can rename over it.
pub fn replace(running: &Path, bundled: &Path) -> std::io::Result<()> {
    if cfg!(windows) {
        let old = old_path(running);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(running, &old)?;
        if let Err(e) = std::fs::copy(bundled, running) {
            let _ = std::fs::rename(&old, running);
            return Err(e);
        }
        Ok(())
    } else {
        let new = running.with_extension("new");
        std::fs::copy(bundled, &new)?;
        std::fs::rename(&new, running)
    }
}

/// Removes the previous copy left by `replace` (best effort; called at startup).
pub fn clean_old(running: &Path) {
    let _ = std::fs::remove_file(old_path(running));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_only_when_different() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("host.bin"), tmp.path().join("bundled.bin"));
        std::fs::write(&a, "v1").unwrap();
        std::fs::write(&b, "v1").unwrap();
        assert!(!needs_replace(&a, &b));
        std::fs::write(&b, "v2").unwrap();
        assert!(needs_replace(&a, &b));
        replace(&a, &b).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "v2");
        clean_old(&a);
        assert!(!old_path(&a).exists());
        assert!(!needs_replace(&a, &tmp.path().join("missing")));
    }
}
