//! A terminal's output kept on disk (PDT-352), so a terminal started again after pdtd restarts can show what the
//! previous process printed. The file is capped: once it reaches `cap` bytes it becomes `<file>.1` (replacing the one
//! before) and a new file starts, so the pair holds at least the last `cap` bytes and never more than twice that.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Shown between the earlier output and the new process's: the line that says so, after [`reset`].
const DIVIDER: &[u8] =
    b"\r\n\x1b[2m\xe2\x94\x80\xe2\x94\x80 restarted \xc2\xb7 earlier output above \xe2\x94\x80\xe2\x94\x80\x1b[0m\r\n";

/// What goes between `earlier` (a terminal's kept output) and the new process's output: [`reset`], then the divider.
pub(crate) fn divider(earlier: &[u8]) -> Vec<u8> {
    let mut out = reset(earlier);
    out.extend_from_slice(DIVIDER);
    out
}

/// Undoes what `earlier` may have left on: it cancels an escape sequence it ends inside, then resets colours, a hidden
/// cursor, mouse reporting, application cursor keys and bracketed paste. It leaves the alternate screen only if
/// `earlier` was still on it (a full-screen program was running), the way that program entered it: leaving one that
/// isn't on would still restore the saved cursor (`?1049l`), often the top of the screen, and write over the earlier
/// output from there.
fn reset(earlier: &[u8]) -> Vec<u8> {
    let mut out = b"\x18\x1b[0m\x1b[?25h\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1l\x1b[?2004l".to_vec();
    if let Some(mode) = alternate_screen(earlier) {
        out.extend_from_slice(format!("\x1b[?{mode}l").as_bytes());
    }
    out
}

/// The mode (`1049`, `1047` or `47`) `bytes` last switched to the alternate screen with, if it didn't switch back.
fn alternate_screen(bytes: &[u8]) -> Option<&'static str> {
    let mut last: Option<(usize, &'static str, bool)> = None;
    for mode in ["1049", "1047", "47"] {
        for on in [true, false] {
            let seq = format!("\x1b[?{mode}{}", if on { 'h' } else { 'l' });
            if let Some(at) = bytes.windows(seq.len()).rposition(|w| w == seq.as_bytes())
                && last.is_none_or(|(l, ..)| at > l)
            {
                last = Some((at, mode, on));
            }
        }
    }
    last.filter(|&(_, _, on)| on).map(|(_, mode, _)| mode)
}

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
    use super::{OutputFile, divider, last_output, rotated};

    #[test]
    fn the_divider_leaves_the_alternate_screen_only_when_it_is_on() {
        let leaves = |earlier: &[u8]| {
            let d = divider(earlier);
            ["1049", "1047", "47"].into_iter().find(|m| {
                let seq = format!("\x1b[?{m}l");
                d.windows(seq.len()).any(|w| w == seq.as_bytes())
            })
        };
        assert_eq!(leaves(b"$ clear\r\n\x1b[H\x1b[2Jline-1\r\n$ "), None, "a shell: the cursor stays put");
        assert_eq!(leaves(b"$ vim\r\n\x1b[?1049h\x1b[Hediting"), Some("1049"), "vim was still open");
        assert_eq!(leaves(b"\x1b[?1049hvim\x1b[?1049l$ "), None, "vim had closed");
        assert_eq!(leaves(b"\x1b[?1049l\x1b[?47h"), Some("47"));
        assert!(divider(b"").ends_with("earlier output above \u{2500}\u{2500}\x1b[0m\r\n".as_bytes()));
    }

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
