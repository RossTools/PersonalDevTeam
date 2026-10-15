//! Host detection and program resolution for terminal sessions.

use crate::pty::PtyError;
use pdt_core::session::Platform;
use std::path::PathBuf;

pub fn detect_platform() -> Platform {
    Platform {
        windows: cfg!(windows),
        has_pwsh: cfg!(windows) && which::which("pwsh").is_ok(),
        unix_shell: std::env::var("SHELL").ok(),
        claude_program: "claude".into(),
        codex_program: "codex".into(),
    }
}

/// Finds `program` on PATH (or accepts a path). On Windows, `.cmd`/`.bat` shims such as npm-installed CLIs
/// cannot be started directly in a PTY, so they run through `%ComSpec% /d /c`.
pub fn resolve(program: &str, args: &[String]) -> Result<(PathBuf, Vec<String>), PtyError> {
    let path = which::which(program).map_err(|_| PtyError::ProgramNotFound(program.to_string()))?;
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
            matches!(resolve("pdt-definitely-missing-xyz", &[]), Err(PtyError::ProgramNotFound(p)) if p == "pdt-definitely-missing-xyz")
        );
    }

    #[test]
    fn found_program_keeps_args() {
        let (path, args) = resolve("git", &["--version".into()]).unwrap();
        assert!(path.is_file(), "{path:?}");
        assert_eq!(args, ["--version"]);
    }

    #[test]
    fn platform_defaults_to_plain_agent_names() {
        let p = detect_platform();
        assert_eq!((p.claude_program.as_str(), p.codex_program.as_str()), ("claude", "codex"));
        assert_eq!(p.windows, cfg!(windows));
    }
}
