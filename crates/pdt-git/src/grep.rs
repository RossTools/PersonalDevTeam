//! Content search in a checkout (`git grep`), for the Files view: tracked and untracked files that aren't ignored,
//! binary files skipped. The output is read as it comes, so a cap or the deadline stops git and keeps what was found.

use crate::GitError;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// What to look for and how much of it to keep.
#[derive(Debug, Clone)]
pub struct GrepQuery {
    pub pattern: String,
    pub match_case: bool,
    pub whole_word: bool,
    /// An extended regular expression (`-E`); otherwise the pattern is taken literally (`-F`).
    pub regex: bool,
    /// Comma-separated globs (`crates/**/*.rs`, `web`, `!*.test.ts`); one without a `/` matches at any depth, and
    /// `!` leaves files out.
    pub glob: String,
    /// A plain folder, not a repository (`--no-index`).
    pub no_index: bool,
    pub max_matches: usize,
    pub max_per_file: usize,
    pub timeout: Duration,
    /// Longer lines are cut to this many characters around the first match.
    pub max_line_chars: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrepLine {
    /// 1-based.
    pub line: u32,
    /// The line, or the part of it around the first match when it is long.
    pub text: String,
    /// Where `text` starts in the line, in UTF-16 units (as JavaScript counts): 0 unless the start was cut.
    pub offset: u32,
    /// Whether the end of the line was cut.
    pub cut_end: bool,
    /// The matches in `text`, as `[start, end)` in UTF-16 units.
    pub ranges: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrepFile {
    /// Relative to the checkout, `/`-separated.
    pub path: String,
    /// The matching lines kept (at most `max_per_file`).
    pub lines: Vec<GrepLine>,
    /// Matching lines seen in the file, kept or not.
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GrepResult {
    pub files: Vec<GrepFile>,
    /// Some matches were left out: a cap was reached.
    pub capped: bool,
    /// The deadline passed and git was stopped.
    pub timed_out: bool,
}

/// How git marks a match (`color.grep.matchSelected`), and the end of any colour.
const MATCH_START: &[u8] = b"\x1b[7m";
const COLOR_END: &[u8] = b"\x1b[m";

/// The pathspecs for `GrepQuery::glob`: each glob matches files, and everything inside folders, by that name.
fn pathspecs(glob: &str) -> Vec<String> {
    let mut out = Vec::new();
    for g in glob.split(',').map(str::trim) {
        let (magic, g) = match g.strip_prefix('!') {
            Some(rest) => ("glob,exclude", rest.trim()),
            None => ("glob", g),
        };
        let g = g.trim_start_matches("./").trim_matches('/');
        if g.is_empty() {
            continue;
        }
        let g = if g.contains('/') { g.to_string() } else { format!("**/{g}") };
        out.push(format!(":({magic}){g}"));
        out.push(format!(":({magic}){g}/**"));
    }
    out
}

/// Searches `checkout` for `q.pattern`. Blocking. A pattern git can't read is `GitError::BadPattern`.
pub fn grep(checkout: &Path, q: &GrepQuery) -> Result<GrepResult, GitError> {
    if q.pattern.is_empty() {
        return Ok(GrepResult::default());
    }
    let mut cmd = Command::new("git");
    cmd.env("LC_ALL", "C").env("LANGUAGE", "C").env("GIT_OPTIONAL_LOCKS", "0");
    for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"] {
        cmd.env_remove(var);
    }
    cmd.args(crate::PDT_GIT_CONFIG).args(["-c", "core.quotepath=off"]);
    // Only matches are coloured, so the colour codes mark exactly where they are.
    for slot in ["filename", "linenumber", "column", "separator", "selected", "context", "function", "matchContext"] {
        cmd.args(["-c", &format!("color.grep.{slot}=normal")]);
    }
    cmd.args(["-c", "color.grep.matchSelected=reverse"]);
    cmd.arg("-C").arg(checkout).arg("grep");
    if q.no_index {
        cmd.arg("--no-index");
    } else {
        cmd.arg("--untracked");
    }
    cmd.args(["--exclude-standard", "-n", "--no-column", "-I", "--null", "--color=always", "--full-name"]);
    cmd.arg(if q.regex { "-E" } else { "-F" });
    if !q.match_case {
        cmd.arg("-i");
    }
    if q.whole_word {
        cmd.arg("-w");
    }
    cmd.arg("-e").arg(&q.pattern).arg("--").args(pathspecs(&q.glob));
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { GitError::GitMissing } else { GitError::Command(e.to_string()) }
    })?;

    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(256);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) if tx.send(line).is_err() => break,
                Ok(_) => {}
            }
        }
    });
    let errs = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let deadline = Instant::now() + q.timeout;
    let mut out = GrepResult::default();
    let mut kept = 0usize;
    let mut stopped = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let next = if left.is_zero() { Err(mpsc::RecvTimeoutError::Timeout) } else { rx.recv_timeout(left) };
        let raw = match next {
            Ok(raw) => raw,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                out.timed_out = true;
                stopped = true;
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let Some((path, line, text)) = split_record(&raw) else { continue };
        if out.files.last().is_none_or(|f| f.path != path) {
            out.files.push(GrepFile { path, lines: Vec::new(), count: 0 });
        }
        let file = out.files.last_mut().expect("just pushed");
        file.count += 1;
        if file.lines.len() >= q.max_per_file {
            out.capped = true;
            continue;
        }
        file.lines.push(shown_line(line, text, q.max_line_chars));
        kept += 1;
        if kept >= q.max_matches {
            out.capped = true;
            stopped = true;
            break;
        }
    }
    if stopped {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| GitError::Command(e.to_string()))?;
    let stderr = errs.join().unwrap_or_default();
    // 0: matches, 1: none. Anything else, unless PDT stopped it, is git refusing the search.
    if !stopped && !matches!(status.code(), Some(0 | 1)) {
        let said = stderr.trim();
        let said = said.strip_prefix("fatal: ").unwrap_or(said);
        if said.contains("-e option") || said.contains("pathspec") || said.contains("outside repository") {
            return Err(GitError::BadPattern(said.to_string()));
        }
        return Err(GitError::Command(said.to_string()));
    }
    Ok(out)
}

/// `path\0line\0text\n` into its parts (the text still coloured).
fn split_record(raw: &[u8]) -> Option<(String, u32, &[u8])> {
    let raw = raw.strip_suffix(b"\n").unwrap_or(raw);
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    let mut parts = raw.splitn(3, |b| *b == 0);
    let path = String::from_utf8_lossy(parts.next()?).into_owned();
    let line = std::str::from_utf8(parts.next()?).ok()?.parse().ok()?;
    Some((path, line, parts.next()?))
}

/// A coloured line as plain text and the byte ranges of its matches.
fn uncolor(text: &[u8]) -> (Vec<u8>, Vec<(usize, usize)>) {
    let mut plain = Vec::with_capacity(text.len());
    let mut ranges = Vec::new();
    let mut open: Option<usize> = None;
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with(MATCH_START) {
            open = Some(plain.len());
            i += MATCH_START.len();
        } else if text[i..].starts_with(COLOR_END) {
            if let Some(start) = open.take()
                && start < plain.len()
            {
                ranges.push((start, plain.len()));
            }
            i += COLOR_END.len();
        } else {
            plain.push(text[i]);
            i += 1;
        }
    }
    (plain, ranges)
}

/// The line as the view shows it: plain text, cut around the first match when longer than `max_chars`, with the
/// matches in UTF-16 units.
fn shown_line(line: u32, colored: &[u8], max_chars: usize) -> GrepLine {
    let (plain, byte_ranges) = uncolor(colored);
    // Byte offsets into char offsets: walk the (lossily decoded) text once.
    let text = String::from_utf8_lossy(&plain);
    let lossless = matches!(text, std::borrow::Cow::Borrowed(_));
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let char_at = |byte: usize| chars.partition_point(|(b, _)| *b < byte);
    let ranges: Vec<(usize, usize)> =
        if lossless { byte_ranges.iter().map(|&(s, e)| (char_at(s), char_at(e))).collect() } else { Vec::new() };
    let (start, end) = if chars.len() <= max_chars {
        (0, chars.len())
    } else {
        let first = ranges.first().map_or(0, |r| r.0);
        let start = first.saturating_sub(max_chars / 5).min(chars.len() - max_chars);
        (start, start + max_chars)
    };
    let utf16 = |from: usize, to: usize| chars[from..to].iter().map(|(_, c)| c.len_utf16() as u32).sum::<u32>();
    let shown: String = chars[start..end].iter().map(|(_, c)| c).collect();
    let ranges = ranges
        .iter()
        .filter(|&&(s, e)| e > start && s < end)
        .map(|&(s, e)| (utf16(start, s.max(start)), utf16(start, e.min(end))))
        .collect();
    GrepLine { line, text: shown, offset: utf16(0, start), cut_end: end < chars.len(), ranges }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").current_dir(dir).args(args).status().unwrap().success();
        assert!(ok, "git {args:?}");
    }

    fn query(pattern: &str) -> GrepQuery {
        GrepQuery {
            pattern: pattern.into(),
            match_case: false,
            whole_word: false,
            regex: false,
            glob: String::new(),
            no_index: false,
            max_matches: 2000,
            max_per_file: 50,
            timeout: Duration::from_secs(10),
            max_line_chars: 300,
        }
    }

    fn repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q", "-b", "main"]);
        let w = |p: &str, t: &[u8]| {
            let f = tmp.path().join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, t).unwrap();
        };
        w(".gitignore", b"ignored.txt\n");
        w("src/a.rs", b"fn needle() {}\nlet x = Needle;\nlet needles = 2;\n");
        w("src/b.ts", b"const needle = 1;\n");
        w("ignored.txt", b"needle\n");
        w("blob.bin", b"needle\0\x01\x02");
        w("long.txt", format!("{}needle{}\n", "a".repeat(500), "b".repeat(500)).as_bytes());
        git(tmp.path(), &["add", "src/a.rs"]);
        tmp
    }

    fn paths(r: &GrepResult) -> Vec<&str> {
        r.files.iter().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn finds_tracked_and_untracked_text_but_not_ignored_or_binary_files() {
        let tmp = repo();
        let r = grep(tmp.path(), &query("needle")).unwrap();
        assert_eq!(paths(&r), ["long.txt", "src/a.rs", "src/b.ts"]);
        let a = &r.files[1];
        assert_eq!(a.count, 3);
        assert_eq!(
            a.lines[0],
            GrepLine { line: 1, text: "fn needle() {}".into(), offset: 0, cut_end: false, ranges: vec![(3, 9)] }
        );
        assert_eq!(a.lines[1].ranges, [(8, 14)], "case is ignored by default");
        assert!(!r.capped && !r.timed_out);
    }

    #[test]
    fn options_narrow_the_search() {
        let tmp = repo();
        let mut q = query("needle");
        q.match_case = true;
        q.whole_word = true;
        let r = grep(tmp.path(), &q).unwrap();
        let a = r.files.iter().find(|f| f.path == "src/a.rs").unwrap();
        assert_eq!(a.lines.iter().map(|l| l.line).collect::<Vec<_>>(), [1]);

        let mut q = query("need(le|les) =");
        q.regex = true;
        let r = grep(tmp.path(), &q).unwrap();
        assert_eq!(paths(&r), ["src/a.rs", "src/b.ts"]);
        assert_eq!(r.files[0].lines[0].ranges, [(4, 13)]);

        let mut q = query("needle");
        q.glob = "*.ts".into();
        assert_eq!(paths(&grep(tmp.path(), &q).unwrap()), ["src/b.ts"]);
        q.glob = "src, !*.ts".into();
        assert_eq!(paths(&grep(tmp.path(), &q).unwrap()), ["src/a.rs"]);
        q.glob = "src/**/*.rs".into();
        assert_eq!(paths(&grep(tmp.path(), &q).unwrap()), ["src/a.rs"]);
    }

    #[test]
    fn a_literal_search_takes_regex_characters_as_they_are() {
        let tmp = repo();
        assert!(grep(tmp.path(), &query("needle.")).unwrap().files.is_empty());
        assert_eq!(paths(&grep(tmp.path(), &query("needle()")).unwrap()), ["src/a.rs"]);
    }

    #[test]
    fn a_bad_regex_is_its_own_error() {
        let tmp = repo();
        let mut q = query("needle(");
        q.regex = true;
        assert!(matches!(grep(tmp.path(), &q), Err(GitError::BadPattern(_))));
    }

    #[test]
    fn caps_keep_the_first_matches() {
        let tmp = repo();
        let mut q = query("needle");
        q.max_per_file = 1;
        let r = grep(tmp.path(), &q).unwrap();
        let a = r.files.iter().find(|f| f.path == "src/a.rs").unwrap();
        assert_eq!((a.lines.len(), a.count), (1, 3));
        assert!(r.capped);

        q.max_per_file = 50;
        q.max_matches = 2;
        let r = grep(tmp.path(), &q).unwrap();
        assert_eq!(r.files.iter().map(|f| f.lines.len()).sum::<usize>(), 2);
        assert!(r.capped && !r.timed_out);
    }

    #[test]
    fn the_deadline_stops_git() {
        let tmp = repo();
        let mut q = query("needle");
        q.timeout = Duration::ZERO;
        let r = grep(tmp.path(), &q).unwrap();
        assert!(r.timed_out);
    }

    #[test]
    fn long_lines_are_cut_around_the_match() {
        let tmp = repo();
        let mut q = query("needle");
        q.max_line_chars = 100;
        let r = grep(tmp.path(), &q).unwrap();
        let l = &r.files[0].lines[0];
        assert_eq!(l.text.chars().count(), 100);
        assert_eq!(l.offset, 480);
        assert!(l.cut_end);
        assert_eq!(l.ranges, [(20, 26)]);
        assert_eq!(&l.text[20..26], "needle");
    }

    #[test]
    fn ranges_count_utf16_units() {
        let l = shown_line(1, "é😀 \x1b[7mneedle\x1b[m".as_bytes(), 300);
        assert_eq!(l.ranges, [(4, 10)]);
    }

    #[test]
    fn a_plain_folder_is_searched_without_git() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("notes.md"), "a needle here\n").unwrap();
        let mut q = query("needle");
        q.no_index = true;
        assert_eq!(paths(&grep(tmp.path(), &q).unwrap()), ["notes.md"]);
    }

    #[test]
    fn globs_become_pathspecs() {
        assert_eq!(pathspecs(""), Vec::<String>::new());
        assert_eq!(
            pathspecs("*.rs, !web/"),
            [":(glob)**/*.rs", ":(glob)**/*.rs/**", ":(glob,exclude)**/web", ":(glob,exclude)**/web/**"]
        );
        assert_eq!(pathspecs(" ./crates/x "), [":(glob)crates/x", ":(glob)crates/x/**"]);
    }
}
