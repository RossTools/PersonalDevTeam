//! A terminal's output kept on disk (PDT-352), so a terminal started again after pdtd restarts can show what the
//! previous process printed. The file is capped: once it reaches `cap` bytes it becomes `<file>.1` (replacing the one
//! before) and a new file starts, so the pair holds at least the last `cap` bytes and never more than twice that.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Shown between the earlier output and the new process's. It first cancels an escape sequence the earlier output
/// may end inside, then leaves the alternate screen and undoes the modes a full-screen program may have left on
/// (colours, a hidden cursor, mouse reporting, application cursor keys, bracketed paste).
pub(crate) const DIVIDER: &[u8] = b"\x18\x1b[0m\x1b[?1049l\x1b[?25h\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1l\x1b[?2004l\r\n\x1b[2m\xe2\x94\x80\xe2\x94\x80 restarted \xc2\xb7 earlier output above \xe2\x94\x80\xe2\x94\x80\x1b[0m\r\n";

fn rotated(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

/// The last `cap` bytes kept at `path` (and its rotated part), starting at a line when cut. Empty when there is none.
pub(crate) fn last_output(path: &Path, cap: usize) -> Vec<u8> {
    let current = tail(path, cap);
    let mut out = if current.len() < cap { tail(&rotated(path), cap - current.len()) } else { vec![] };
    let cut = out.len() + current.len() >= cap;
    out.extend_from_slice(&current);
    if cut {
        // Start at a line, not part way through one (or through a UTF-8 sequence or an escape sequence).
        match out.iter().position(|&b| b == b'\n') {
            Some(i) => {
                out.drain(..=i);
            }
            None => out.clear(),
        }
    }
    out
}

/// The last `n` bytes of the file at `path`, or none if it can't be read.
fn tail(path: &Path, n: usize) -> Vec<u8> {
    let Ok(mut f) = File::open(path) else { return vec![] };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(n as u64);
    let mut out = Vec::new();
    if f.seek(SeekFrom::Start(from)).is_err() || f.take(n as u64).read_to_end(&mut out).is_err() {
        return vec![];
    }
    out
}

/// Appends a terminal's output to its file, rotating it at the cap.
pub(crate) struct OutputFile {
    path: PathBuf,
    file: Option<File>,
    len: u64,
    cap: u64,
}

impl OutputFile {
    /// Opens (or creates) the file at `path` to append to, and its folder.
    pub(crate) fn open(path: PathBuf, cap: usize) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = open_append(&path)?;
        let len = file.metadata()?.len();
        Ok(Self { path, file: Some(file), len, cap: cap as u64 })
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if self.len >= self.cap {
            // Closed before the rename: Windows won't rename an open file.
            self.file.take();
            std::fs::rename(&self.path, rotated(&self.path))?;
            self.file = Some(open_append(&self.path)?);
            self.len = 0;
        }
        let Some(f) = self.file.as_mut() else { return Ok(()) };
        f.write_all(bytes)?;
        self.len += bytes.len() as u64;
        Ok(())
    }
}

/// Only the user can read it: it holds whatever the terminal showed.
fn open_append(path: &Path) -> std::io::Result<File> {
    let mut o = std::fs::OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)
}

#[cfg(test)]
mod tests {
    use super::{OutputFile, last_output, rotated};

    #[test]
    fn nothing_kept_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(last_output(&tmp.path().join("output.log"), 100).is_empty());
    }

    #[test]
    fn output_short_of_the_cap_is_kept_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s").join("output.log");
        let mut f = OutputFile::open(path.clone(), 100).unwrap();
        f.write(b"one\r\n").unwrap();
        f.write(b"two").unwrap();
        drop(f);
        // Opened again (pdtd restarted), it appends.
        OutputFile::open(path.clone(), 100).unwrap().write(b"\r\nthree").unwrap();
        assert_eq!(last_output(&path, 100), b"one\r\ntwo\r\nthree");
    }

    #[test]
    fn the_file_rotates_at_the_cap_and_the_tail_starts_at_a_line() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("output.log");
        let mut f = OutputFile::open(path.clone(), 10).unwrap();
        for line in ["aaaa\n", "bbbb\n", "cccc\n", "dddd\n", "eeee\n"] {
            f.write(line.as_bytes()).unwrap();
        }
        assert!(std::fs::metadata(&path).unwrap().len() <= 10);
        assert!(rotated(&path).is_file(), "the earlier part was kept beside it");
        assert_eq!(last_output(&path, 12), b"dddd\neeee\n", "from the first line start within the cap");
        assert_eq!(last_output(&path, 100), b"cccc\ndddd\neeee\n", "never more than the two parts");
    }
}
