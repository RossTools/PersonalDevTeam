//! An agent's stderr, kept as a bounded tail for the error the user sees when the process ends.

use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::task::JoinHandle;

/// How much of an agent's stderr PDT keeps (the end of it).
pub const STDERR_TAIL: usize = 4096;

/// Reads `stderr` to its end on a task, keeping its last `cap` bytes (at a character boundary) in the returned string.
/// A character split between two reads is joined, not replaced; invalid UTF-8 becomes `�`.
pub fn stderr_tail<R: AsyncRead + Unpin + Send + 'static>(
    mut stderr: R,
    cap: usize,
) -> (Arc<Mutex<String>>, JoinHandle<()>) {
    let tail = Arc::new(Mutex::new(String::new()));
    let task = {
        let tail = tail.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            let mut pending = Vec::new();
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                pending.extend_from_slice(&buf[..n]);
                let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                decode(&mut pending, &mut t);
                keep_tail(&mut t, cap);
            }
            if !pending.is_empty() {
                let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                t.push_str(&String::from_utf8_lossy(&pending));
                keep_tail(&mut t, cap);
            }
        })
    };
    (tail, task)
}

/// Moves the complete UTF-8 in `pending` onto `out` (invalid bytes as `�`), leaving an incomplete character at its end.
fn decode(pending: &mut Vec<u8>, out: &mut String) {
    let mut start = 0;
    loop {
        match std::str::from_utf8(&pending[start..]) {
            Ok(s) => {
                out.push_str(s);
                start = pending.len();
                break;
            }
            Err(e) => {
                let valid = start + e.valid_up_to();
                out.push_str(std::str::from_utf8(&pending[start..valid]).unwrap_or_default());
                match e.error_len() {
                    Some(bad) => {
                        out.push(char::REPLACEMENT_CHARACTER);
                        start = valid + bad;
                    }
                    None => {
                        start = valid;
                        break;
                    }
                }
            }
        }
    }
    pending.drain(..start);
}

/// Drops the start of `text` so at most its last `cap` bytes remain, cutting at a character boundary.
pub fn keep_tail(text: &mut String, cap: usize) {
    if text.len() > cap {
        let mut cut = text.len() - cap;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        text.drain(..cut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    async fn tail_of(chunks: &[&[u8]], cap: usize) -> String {
        let (mut w, r) = tokio::io::duplex(64);
        let (tail, task) = stderr_tail(r, cap);
        for c in chunks {
            w.write_all(c).await.unwrap();
            w.flush().await.unwrap();
            tokio::task::yield_now().await;
        }
        drop(w);
        task.await.unwrap();
        tail.lock().unwrap().clone()
    }

    #[test]
    fn decode_keeps_a_split_character_for_the_next_read() {
        let euro = "€".as_bytes();
        let mut pending = b"a".to_vec();
        pending.extend_from_slice(&euro[..2]);
        let mut out = String::new();
        decode(&mut pending, &mut out);
        assert_eq!(out, "a");
        assert_eq!(pending, &euro[..2]);
        pending.extend_from_slice(&euro[2..]);
        pending.push(b'b');
        decode(&mut pending, &mut out);
        assert_eq!(out, "a€b");
        assert!(pending.is_empty());
    }

    #[test]
    fn decode_replaces_invalid_bytes_and_goes_on() {
        let mut pending = b"a\xffb\xc3".to_vec();
        let mut out = String::new();
        decode(&mut pending, &mut out);
        assert_eq!(out, "a\u{FFFD}b");
        assert_eq!(pending, b"\xc3");
    }

    #[test]
    fn keep_tail_cuts_at_a_character_boundary() {
        let mut s = "ab€cd".to_string();
        keep_tail(&mut s, 4);
        // The last 4 bytes start inside '€'; the cut moves past it.
        assert_eq!(s, "cd");
        let mut short = "abc".to_string();
        keep_tail(&mut short, 4);
        assert_eq!(short, "abc");
    }

    #[tokio::test]
    async fn a_character_split_between_reads_is_whole() {
        let e = "é".as_bytes();
        let got = tail_of(&[b"bad ", &e[..1], &[e[1]], b" thing"], STDERR_TAIL).await;
        assert_eq!(got, "bad é thing");
    }

    #[tokio::test]
    async fn keeps_only_the_end() {
        let got = tail_of(&[b"0123456789", b"abcdef"], 8).await;
        assert_eq!(got, "89abcdef");
    }

    #[tokio::test]
    async fn an_unfinished_character_at_the_end_is_replaced() {
        let got = tail_of(&[b"oops \xe2\x82"], STDERR_TAIL).await;
        assert_eq!(got, "oops \u{FFFD}");
    }
}
