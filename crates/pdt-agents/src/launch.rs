//! Host detection and program resolution for terminal sessions.

use crate::pty::PtyError;
use pdt_core::session::Platform;
use std::collections::HashSet;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Prints the shell's PATH with nothing around it.
const PRINT_PATH: &str = r#"printf %s "$PATH""#;
const LOGIN_PATH_TIMEOUT: Duration = Duration::from_secs(3);

pub fn detect_platform() -> Platform {
    let unix_shell = std::env::var("SHELL").ok();
    // A macOS app started by launchd has a minimal PATH, so ask the user's login shell for theirs.
    let login_path = if cfg!(target_os = "macos") {
        let shell = unix_shell.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "/bin/zsh".into());
        let path = login_path_from(&shell, &["-l", "-c", PRINT_PATH], LOGIN_PATH_TIMEOUT);
        if path.is_none() {
            tracing::warn!(shell = %shell, "could not read the login shell's PATH; using pdtd's own");
        }
        path
    } else {
        None
    };
    Platform {
        windows: cfg!(windows),
        has_pwsh: cfg!(windows) && which::which("pwsh").is_ok(),
        unix_shell,
        claude_program: "claude".into(),
        codex_program: "codex".into(),
        login_path,
        codex_home: codex_home(),
        claude_config_dir: claude_config_dir(),
    }
}

/// `$CODEX_HOME` if set and non-empty, else `<home>/.codex`.
fn codex_home() -> Option<PathBuf> {
    config_dir_from(std::env::var_os("CODEX_HOME"), std::env::home_dir(), ".codex")
}

/// `$CLAUDE_CONFIG_DIR` if set and non-empty, else `<home>/.claude`.
fn claude_config_dir() -> Option<PathBuf> {
    config_dir_from(std::env::var_os("CLAUDE_CONFIG_DIR"), std::env::home_dir(), ".claude")
}

fn config_dir_from(env: Option<std::ffi::OsString>, home: Option<PathBuf>, dot_dir: &str) -> Option<PathBuf> {
    match env.filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => home.map(|h| h.join(dot_dir)),
    }
}

/// A working folder as a comparison key: no trailing separators, and case-insensitive on Windows.
pub fn folder_key(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    let trimmed = if trimmed.is_empty() { cwd } else { trimmed };
    if cfg!(windows) { trimmed.to_lowercase() } else { trimmed.to_string() }
}

/// How long before the session's fresh launch a conversation file may have been touched and still be its own.
const ROLLOUT_MTIME_SLACK_MS: i64 = 60_000;
/// Codex stamps a conversation with its process start time, which is just after PDT's own `fresh_started_at`.
const ROLLOUT_START_BEFORE_MS: i64 = 1_000;
const ROLLOUT_START_AFTER_MS: i64 = 30_000;

/// Finds the Codex conversation created by the process that PDT launched fresh at `fresh_started_at_ms` in `cwd`.
/// Codex cannot be given a conversation id, but writes `sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl` after the
/// first turn, whose first line records the id, working folder and process start time.
///
/// A conversation that started at `t` belongs to the session whose fresh launch is the latest at or before `t`
/// (`peers_fresh_started_at_ms` are the other Codex sessions' in this folder), provided `t` is within seconds of it:
/// a Codex the user starts by hand later, or a neighbour's conversation, is never claimed. Conversations in
/// `claimed` belong to other sessions. Of the matches, the earliest is returned (the first conversation this
/// session's process started). Launches closer together than the caller allows are for the caller to refuse.
pub fn find_codex_conversation(
    codex_home: &Path,
    cwd: &Path,
    fresh_started_at_ms: i64,
    peers_fresh_started_at_ms: &[i64],
    claimed: &HashSet<String>,
) -> Option<String> {
    let want_cwd = folder_key(&cwd.to_string_lossy());
    let oldest_day = oldest_rollout_day(fresh_started_at_ms);
    let root = codex_home.join("sessions");
    let mut best: Option<(i64, String)> = None;
    let mut dirs = vec![(root.clone(), 0usize)];
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                // Rollouts live in YYYY/MM/DD folders: skip days clearly older than the launch.
                if depth == 2 && day_of(&root, &path).is_some_and(|d| oldest_day.is_some_and(|o| d < o)) {
                    continue;
                }
                dirs.push((path, depth + 1));
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !(name.starts_with("rollout-") && name.ends_with(".jsonl")) {
                continue;
            }
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            if modified_ms.is_none_or(|m| m < fresh_started_at_ms - ROLLOUT_MTIME_SLACK_MS) {
                continue;
            }
            let Some((id, folder, began)) = read_rollout_meta(&path) else { continue };
            let off = began - fresh_started_at_ms;
            let later_peer = peers_fresh_started_at_ms
                .iter()
                .any(|&p| p > fresh_started_at_ms && p <= began + ROLLOUT_START_BEFORE_MS);
            if folder_key(&folder) != want_cwd
                || !(-ROLLOUT_START_BEFORE_MS..=ROLLOUT_START_AFTER_MS).contains(&off)
                || later_peer
                || claimed.contains(&id)
            {
                continue;
            }
            if best.as_ref().is_none_or(|(b, _)| began < *b) {
                best = Some((began, id));
            }
        }
    }
    best.map(|(_, id)| id)
}

/// The day before the launch's UTC date: rollout folders are named by local date, which is at most a day off.
fn oldest_rollout_day(fresh_started_at_ms: i64) -> Option<jiff::civil::Date> {
    let day = jiff::Timestamp::from_millisecond(fresh_started_at_ms).ok()?.to_zoned(jiff::tz::TimeZone::UTC).date();
    day.yesterday().ok()
}

/// The date a `<root>/YYYY/MM/DD` folder stands for.
fn day_of(root: &Path, dir: &Path) -> Option<jiff::civil::Date> {
    let mut parts = dir.strip_prefix(root).ok()?.components().map(|c| c.as_os_str().to_str()?.parse::<i16>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    jiff::civil::Date::new(y, i8::try_from(m).ok()?, i8::try_from(d).ok()?).ok()
}

/// (conversation id, working folder, start time in ms) from a rollout file's `session_meta` first line.
fn read_rollout_meta(path: &Path) -> Option<(String, String, i64)> {
    // The first line embeds the agent's instructions and can be long, but nothing past it is needed.
    let mut line = String::new();
    std::io::BufReader::new(std::fs::File::open(path).ok()?.take(4 << 20)).read_line(&mut line).ok()?;
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let p = v.get("payload")?;
    let began = p.get("timestamp")?.as_str()?.parse::<jiff::Timestamp>().ok()?.as_millisecond();
    Some((p.get("id")?.as_str()?.to_string(), p.get("cwd")?.as_str()?.to_string(), began))
}

/// Whether Claude has saved a conversation for `session_id`. It writes `projects/<encoded cwd>/<id>.jsonl` only
/// after the first turn; the folder name is Claude's own encoding, so every project folder is checked.
pub fn claude_conversation_exists(config_dir: &Path, session_id: &str) -> bool {
    let Ok(projects) = std::fs::read_dir(config_dir.join("projects")) else {
        return false;
    };
    let file = format!("{session_id}.jsonl");
    projects.flatten().any(|d| d.path().join(&file).is_file())
}

/// Runs `shell args` and returns the last line of its stdout, or None if it fails, prints nothing or outlives `timeout`.
fn login_path_from(shell: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(shell)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = tx.send(stdout.read_to_string(&mut out).map(|_| out));
    });
    let out = rx.recv_timeout(timeout);
    if out.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().ok()?;
    // The last line: a chatty profile may print before the command runs.
    let path = out.ok()?.ok()?.trim().lines().last().unwrap_or_default().trim().to_string();
    (status.success() && !path.is_empty()).then_some(path)
}

/// Finds `program` on `search_path` (default: pdtd's PATH), or accepts a path. On Windows, `.cmd`/`.bat` shims
/// such as npm-installed CLIs cannot be started directly in a PTY, so they run through `%ComSpec% /d /c`.
pub fn resolve(program: &str, args: &[String], search_path: Option<&str>) -> Result<(PathBuf, Vec<String>), PtyError> {
    let not_found = || PtyError::ProgramNotFound(program.to_string());
    let paths = search_path.map(std::ffi::OsString::from).or_else(|| std::env::var_os("PATH"));
    let cwd = std::env::current_dir().map_err(|_| not_found())?;
    let path = which::which_in(program, paths, cwd).map_err(|_| not_found())?;
    let is_script = cfg!(windows)
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if is_script {
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
        let mut all = vec!["/d".to_string(), "/c".to_string(), path.to_string_lossy().into_owned()];
        all.extend(args.iter().cloned());
        return Ok((PathBuf::from(comspec), all));
    }
    Ok((path, args.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_program_is_reported() {
        assert!(
            matches!(resolve("pdt-definitely-missing-xyz", &[], None), Err(PtyError::ProgramNotFound(p)) if p == "pdt-definitely-missing-xyz")
        );
    }

    #[test]
    fn found_program_keeps_args() {
        let (path, args) = resolve("git", &["--version".into()], None).unwrap();
        assert!(path.is_file(), "{path:?}");
        assert_eq!(args, ["--version"]);
    }

    #[test]
    fn resolves_on_the_given_search_path_only() {
        let dir = tempfile::tempdir().unwrap();
        let name = "pdt-fake-agent-xyz";
        let file = dir.path().join(if cfg!(windows) { format!("{name}.cmd") } else { name.to_string() });
        std::fs::write(&file, if cfg!(windows) { "@echo off\r\n" } else { "#!/bin/sh\n" }).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let search = std::env::join_paths([dir.path()]).unwrap().into_string().unwrap();
        let (program, args) = resolve(name, &["--version".into()], Some(&search)).unwrap();
        let found = if cfg!(windows) { PathBuf::from(&args[2]) } else { program };
        assert_eq!(std::fs::canonicalize(found).unwrap(), std::fs::canonicalize(&file).unwrap());
        assert_eq!(args.last().unwrap(), "--version");
        assert!(matches!(resolve(name, &[], None), Err(PtyError::ProgramNotFound(_))), "not on the process PATH");
    }

    #[cfg(unix)]
    #[test]
    fn login_path_reads_the_shell_path() {
        let path = login_path_from("/bin/sh", &["-c", PRINT_PATH], Duration::from_secs(3));
        assert_eq!(path.as_deref(), std::env::var("PATH").ok().as_deref());
        assert_eq!(login_path_from("/bin/sh", &["-c", "sleep 5"], Duration::from_millis(200)), None, "times out");
        assert_eq!(login_path_from("/pdt/no/such/shell", &["-c", PRINT_PATH], Duration::from_secs(3)), None);
    }

    #[test]
    fn claude_conversation_is_found_in_any_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!claude_conversation_exists(dir.path(), "abc"), "no projects dir");
        std::fs::create_dir_all(dir.path().join("projects").join("C--some-repo")).unwrap();
        std::fs::create_dir_all(dir.path().join("projects").join("C--other")).unwrap();
        assert!(!claude_conversation_exists(dir.path(), "abc"), "no file yet");
        std::fs::write(
            dir.path().join("projects").join("C--other").join("abc.jsonl"),
            "{}
",
        )
        .unwrap();
        assert!(claude_conversation_exists(dir.path(), "abc"));
        assert!(!claude_conversation_exists(dir.path(), "abcd"), "a different id");
        // A directory with the right name is not a conversation.
        std::fs::create_dir_all(dir.path().join("projects").join("C--some-repo").join("xyz.jsonl")).unwrap();
        assert!(!claude_conversation_exists(dir.path(), "xyz"));
    }

    #[test]
    fn claude_config_dir_prefers_the_env_var() {
        let home = Some(PathBuf::from("/home/u"));
        assert_eq!(config_dir_from(Some("/cfg".into()), home.clone(), ".claude"), Some(PathBuf::from("/cfg")));
        assert_eq!(config_dir_from(Some("".into()), home.clone(), ".claude"), Some(PathBuf::from("/home/u/.claude")));
        assert_eq!(config_dir_from(None, home, ".claude"), Some(PathBuf::from("/home/u/.claude")));
        assert_eq!(config_dir_from(None, None, ".codex"), None);
    }

    /// The launch time used by the tests: now, so freshly written files pass the modified-time filter.
    fn t0() -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
    }

    fn at(base: i64, secs: f64) -> i64 {
        base + (secs * 1000.0) as i64
    }

    /// Writes a rollout file the way Codex does (a long `session_meta` first line, then more lines), in the day
    /// folder of its start time.
    fn rollout(home: &Path, id: &str, cwd: &str, began_ms: i64) -> PathBuf {
        let began = jiff::Timestamp::from_millisecond(began_ms).unwrap();
        let day = began.to_zoned(jiff::tz::TimeZone::UTC).strftime("%Y/%m/%d").to_string();
        rollout_in(home, &day, id, cwd, began_ms)
    }

    fn rollout_in(home: &Path, day: &str, id: &str, cwd: &str, began_ms: i64) -> PathBuf {
        let dir = home.join("sessions").join(day);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-2026-10-15T21-45-42-{id}.jsonl"));
        let began = jiff::Timestamp::from_millisecond(began_ms).unwrap().to_string();
        let meta = serde_json::json!({
            "timestamp": began, "type": "session_meta",
            "payload": {"id": id, "timestamp": began, "cwd": cwd, "instructions": "x".repeat(50_000)},
        });
        std::fs::write(&path, format!("{meta}\n{{\"type\":\"event_msg\"}}\n")).unwrap();
        path
    }

    fn cwd() -> &'static str {
        if cfg!(windows) { "C:\\work\\repo" } else { "/work/repo" }
    }

    fn find(home: &Path, me: i64, peers: &[i64], cwd: &str, claimed: &[&str]) -> Option<String> {
        let claimed = claimed.iter().map(|s| s.to_string()).collect();
        find_codex_conversation(home, Path::new(cwd), me, peers, &claimed)
    }

    #[test]
    fn finds_the_conversation_the_launched_process_started() {
        let dir = tempfile::tempdir().unwrap();
        let b = t0();
        rollout(dir.path(), "conv-1", cwd(), at(b, 2.5));
        assert_eq!(find(dir.path(), b, &[], cwd(), &[]).as_deref(), Some("conv-1"));
        // The same folder written with a trailing separator (and another case on Windows).
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(find(dir.path(), b, &[], &format!("{}{sep}", cwd()), &[]).as_deref(), Some("conv-1"));
        if cfg!(windows) {
            assert_eq!(find(dir.path(), b, &[], &cwd().to_uppercase(), &[]).as_deref(), Some("conv-1"));
        }
    }

    #[test]
    fn ignores_other_folders_claimed_ids_and_malformed_files() {
        let dir = tempfile::tempdir().unwrap();
        let b = t0();
        assert_eq!(find(&dir.path().join("missing"), b, &[], cwd(), &[]), None, "no codex home");
        assert_eq!(find(dir.path(), b, &[], cwd(), &[]), None, "no sessions dir");
        rollout(dir.path(), "other-folder", "/somewhere/else", at(b, 1.0));
        let day = dir.path().join("sessions").join(
            jiff::Timestamp::from_millisecond(b)
                .unwrap()
                .to_zoned(jiff::tz::TimeZone::UTC)
                .strftime("%Y/%m/%d")
                .to_string(),
        );
        std::fs::write(day.join("rollout-garbage.jsonl"), "not json\n").unwrap();
        std::fs::write(day.join("rollout-empty.jsonl"), "").unwrap();
        std::fs::write(day.join("rollout-wrong-type.jsonl"), "{\"type\":\"event_msg\",\"payload\":{}}\n").unwrap();
        std::fs::write(day.join("notes.txt"), "{}").unwrap();
        assert_eq!(find(dir.path(), b, &[], cwd(), &[]), None);
        rollout(dir.path(), "mine", cwd(), at(b, 1.0));
        assert_eq!(find(dir.path(), b, &[], cwd(), &["mine"]), None);
        assert_eq!(find(dir.path(), b, &[], cwd(), &[]).as_deref(), Some("mine"));
    }

    #[test]
    fn a_conversation_must_start_within_seconds_of_the_launch() {
        let dir = tempfile::tempdir().unwrap();
        let b = t0();
        // A codex the user started by hand a minute later, or before the launch: not this session's.
        rollout(dir.path(), "by-hand-later", cwd(), at(b, 60.0));
        rollout(dir.path(), "earlier", cwd(), at(b, -3.0));
        assert_eq!(find(dir.path(), b, &[], cwd(), &[]), None);
        rollout(dir.path(), "ok-late", cwd(), at(b, 29.0));
        rollout(dir.path(), "ok-first", cwd(), at(b, 4.0));
        assert_eq!(find(dir.path(), b, &[], cwd(), &[]).as_deref(), Some("ok-first"), "the earliest wins");
        assert_eq!(find(dir.path(), b, &[], cwd(), &["ok-first"]).as_deref(), Some("ok-late"));
    }

    #[test]
    fn a_conversation_belongs_to_the_latest_launch_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (t0(), t0() + 60_000);
        // A never took a turn; B (a minute later) did.
        rollout(dir.path(), "conv-b", cwd(), at(b, 1.0));
        assert_eq!(find(dir.path(), a, &[b], cwd(), &[]), None, "A cannot claim B's conversation");
        assert_eq!(find(dir.path(), b, &[a], cwd(), &[]).as_deref(), Some("conv-b"));
        // Launched 8 s apart, a conversation 10 s after A's launch is B's, not A's.
        let (a, b) = (t0(), t0() + 8_000);
        let dir = tempfile::tempdir().unwrap();
        rollout(dir.path(), "conv", cwd(), at(a, 10.0));
        assert_eq!(find(dir.path(), a, &[b], cwd(), &[]), None);
        assert_eq!(find(dir.path(), b, &[a], cwd(), &[]).as_deref(), Some("conv"));
        // Launches before this one do not matter.
        assert_eq!(find(dir.path(), a, &[a - 20_000], cwd(), &[]).as_deref(), Some("conv"));
    }

    #[test]
    fn old_files_and_old_day_folders_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let b = t0();
        let stale = rollout(dir.path(), "stale", cwd(), at(b, 1.0));
        let old = std::time::UNIX_EPOCH + Duration::from_millis((b - 3_600_000) as u64);
        std::fs::File::options().write(true).open(&stale).unwrap().set_modified(old).unwrap();
        assert_eq!(find(dir.path(), b, &[], cwd(), &[]), None, "not touched since before the launch");
        let other = tempfile::tempdir().unwrap();
        rollout_in(other.path(), "2001/01/01", "ancient-day", cwd(), at(b, 1.0));
        assert_eq!(find(other.path(), b, &[], cwd(), &[]), None, "day folder far older than the launch");
        rollout(other.path(), "today", cwd(), at(b, 2.0));
        assert_eq!(find(other.path(), b, &[], cwd(), &[]).as_deref(), Some("today"));
    }

    #[test]
    fn platform_defaults_to_plain_agent_names() {
        let p = detect_platform();
        assert_eq!((p.claude_program.as_str(), p.codex_program.as_str()), ("claude", "codex"));
        assert_eq!(p.windows, cfg!(windows));
    }
}
