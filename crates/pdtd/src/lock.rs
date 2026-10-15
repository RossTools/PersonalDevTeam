use anyhow::Context;
use std::fs::{File, TryLockError};
use std::path::Path;

/// Takes an exclusive lock on `<data_dir>/pdtd.lock`. Hold the returned `File` for the life of the process.
pub fn lock_data_dir(data_dir: &Path) -> anyhow::Result<File> {
    let path = data_dir.join("pdtd.lock");
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening lock file {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => anyhow::bail!("data dir {} is in use by another pdtd", data_dir.display()),
        Err(TryLockError::Error(e)) => Err(e).with_context(|| format!("locking {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_lock_is_exclusive() {
        let tmp = tempfile::tempdir().unwrap();
        let _held = lock_data_dir(tmp.path()).unwrap();
        let err = lock_data_dir(tmp.path()).unwrap_err().to_string();
        assert!(err.contains("is in use by another pdtd"), "{err}");
    }
}
