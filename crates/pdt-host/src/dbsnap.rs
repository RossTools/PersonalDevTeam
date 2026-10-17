//! The database copy that makes a rollback safe. A new version may migrate the database, and the version it rolls
//! back to refuses a migration it does not know, so pdt-host copies the database before installing an update and
//! puts that copy back when it rolls back. Only ever called while no pdtd is running.

use pdt_core::handoff::{DB_FILE, db_snapshot_dir};
use std::io;
use std::path::{Path, PathBuf};

/// The database and the files SQLite keeps beside it in WAL mode.
fn parts(dir: &Path) -> [PathBuf; 3] {
    ["", "-wal", "-shm"].map(|suffix| dir.join(format!("{DB_FILE}{suffix}")))
}

fn remove_if_present(p: &Path) -> io::Result<()> {
    match std::fs::remove_file(p) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn remove_dir_if_present(p: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(p) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Copies the database files that exist (none, for a host that has no database yet) into the snapshot folder,
/// replacing any older snapshot. Written to a temporary folder first, so a snapshot is always complete.
pub fn take(data_dir: &Path) -> io::Result<()> {
    let snap = db_snapshot_dir(data_dir);
    let tmp = snap.with_extension("tmp");
    remove_dir_if_present(&tmp)?;
    std::fs::create_dir_all(&tmp)?;
    for (from, to) in parts(data_dir).iter().zip(parts(&tmp)) {
        if from.is_file() {
            std::fs::copy(from, to)?;
        }
    }
    remove_dir_if_present(&snap)?;
    std::fs::rename(&tmp, &snap)
}

/// Puts the snapshot back (removing database files it does not have) and then deletes it. False when there is no
/// snapshot, e.g. the update failed before one was taken.
pub fn restore(data_dir: &Path) -> io::Result<bool> {
    let snap = db_snapshot_dir(data_dir);
    if !snap.is_dir() {
        return Ok(false);
    }
    for (saved, live) in parts(&snap).iter().zip(parts(data_dir)) {
        remove_if_present(&live)?;
        if saved.is_file() {
            std::fs::copy(saved, &live)?;
        }
    }
    remove_dir_if_present(&snap)?;
    Ok(true)
}

/// Deletes the snapshot once the update is in.
pub fn discard(data_dir: &Path) -> io::Result<()> {
    remove_dir_if_present(&db_snapshot_dir(data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    #[test]
    fn restore_puts_back_exactly_the_files_that_were_there() {
        let tmp = tempfile::tempdir().unwrap();
        let [db, wal, shm] = parts(tmp.path());
        write(&db, "old");
        write(&wal, "old wal");
        take(tmp.path()).unwrap();
        write(&db, "migrated");
        write(&wal, "new wal");
        write(&shm, "new shm");
        assert!(restore(tmp.path()).unwrap());
        assert_eq!(std::fs::read_to_string(&db).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(&wal).unwrap(), "old wal");
        assert!(!shm.exists());
        assert!(!db_snapshot_dir(tmp.path()).exists());
        assert!(!restore(tmp.path()).unwrap(), "nothing left to restore");
    }

    #[test]
    fn a_new_snapshot_replaces_an_old_one() {
        let tmp = tempfile::tempdir().unwrap();
        let [db, wal, _] = parts(tmp.path());
        write(&db, "first");
        write(&wal, "first wal");
        take(tmp.path()).unwrap();
        std::fs::remove_file(&wal).unwrap();
        write(&db, "second");
        take(tmp.path()).unwrap();
        write(&db, "third");
        write(&wal, "third wal");
        restore(tmp.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&db).unwrap(), "second");
        assert!(!wal.exists(), "the first snapshot's WAL is not resurrected");
    }

    #[test]
    fn discard_removes_the_snapshot_and_tolerates_none() {
        let tmp = tempfile::tempdir().unwrap();
        discard(tmp.path()).unwrap();
        take(tmp.path()).unwrap();
        assert!(db_snapshot_dir(tmp.path()).is_dir());
        discard(tmp.path()).unwrap();
        assert!(!db_snapshot_dir(tmp.path()).exists());
    }
}
