//! Host detection and program resolution for terminal sessions.

use crate::pty::PtyError;
use pdt_core::session::Platform;
use std::io::Read;
use std::path::PathBuf;
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
    }
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
    fn platform_defaults_to_plain_agent_names() {
        let p = detect_platform();
        assert_eq!((p.claude_program.as_str(), p.codex_program.as_str()), ("claude", "codex"));
        assert_eq!(p.windows, cfg!(windows));
    }
}
