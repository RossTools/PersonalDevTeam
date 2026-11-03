//! Reading an agent's newline-delimited output with a bound on each line: a line longer than [`MAX_LINE`] is skipped
//! (and logged) rather than held in memory however long it grows.

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// The longest line PDT keeps from an agent (a frame can carry images and whole files).
pub const MAX_LINE: usize = 64 * 1024 * 1024;

/// Lines of a reader, each at most `max` bytes.
pub struct CappedLines<R> {
    reader: R,
    max: usize,
    buf: Vec<u8>,
}

impl<R: AsyncBufRead + Unpin> CappedLines<R> {
    pub fn new(reader: R, max: usize) -> Self {
        Self { reader, max, buf: Vec::new() }
    }

    /// The next line, without its `\n` or `\r\n`; `None` at the end or on a read error. Invalid UTF-8 is replaced.
    pub async fn next_line(&mut self) -> Option<String> {
        loop {
            self.buf.clear();
            let mut skipped = 0usize;
            let mut ended = false;
            loop {
                let avail = match self.reader.fill_buf().await {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::debug!(error = %e, "an agent's output could not be read");
                        return None;
                    }
                };
                if avail.is_empty() {
                    ended = true;
                    break;
                }
                let (part, done) = match avail.iter().position(|&b| b == b'\n') {
                    Some(i) => (&avail[..i], true),
                    None => (avail, false),
                };
                let used = part.len() + usize::from(done);
                if skipped > 0 || self.buf.len() + part.len() > self.max {
                    skipped += self.buf.len() + part.len();
                    self.buf = Vec::new();
                } else {
                    self.buf.extend_from_slice(part);
                }
                self.reader.consume(used);
                if done {
                    break;
                }
            }
            if skipped > 0 {
                tracing::warn!(bytes = skipped, max = self.max, "skipped an agent output line that was too long");
                if ended {
                    return None;
                }
                continue;
            }
            if ended && self.buf.is_empty() {
                return None;
            }
            if self.buf.last() == Some(&b'\r') {
                self.buf.pop();
            }
            return Some(String::from_utf8_lossy(&self.buf).into_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    async fn all(input: &[u8], max: usize, cap: usize) -> Vec<String> {
        let mut lines = CappedLines::new(BufReader::with_capacity(cap, input), max);
        let mut out = vec![];
        while let Some(l) = lines.next_line().await {
            out.push(l);
        }
        out
    }

    #[tokio::test]
    async fn reads_lines_like_lines_does() {
        assert_eq!(all(b"a\nbb\r\n\nlast", 10, 3).await, ["a", "bb", "", "last"]);
        assert_eq!(all(b"a\n", 10, 3).await, ["a"]);
        assert!(all(b"", 10, 3).await.is_empty());
    }

    #[tokio::test]
    async fn skips_a_line_past_the_bound_and_carries_on() {
        let input = b"ok\n0123456789abcdef\nnext\n0123456789abcdef";
        assert_eq!(all(input, 10, 4).await, ["ok", "next"]);
        assert_eq!(all(input, 10, 64).await, ["ok", "next"]);
        assert_eq!(all(b"0123456789\n", 10, 4).await, ["0123456789"], "exactly the bound is kept");
    }
}
