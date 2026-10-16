//! Run instances (remote-host spec §4.2). Pure types and rules; pdt-api drives the processes.

use crate::host::PortRange;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunKind {
    Run,
    Test,
}

impl RunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RunKind::Run => "run",
            RunKind::Test => "test",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [RunKind::Run, RunKind::Test].into_iter().find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunState {
    SettingUp,
    Starting,
    /// A web run whose port answers, or a non-web run whose command started.
    Ready,
    /// A test that is executing.
    Running,
    /// A test that exited 0.
    Passed,
    Failed,
    Stopped,
}

const ALL_STATES: [RunState; 7] = [
    RunState::SettingUp,
    RunState::Starting,
    RunState::Ready,
    RunState::Running,
    RunState::Passed,
    RunState::Failed,
    RunState::Stopped,
];

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::SettingUp => "setting_up",
            RunState::Starting => "starting",
            RunState::Ready => "ready",
            RunState::Running => "running",
            RunState::Passed => "passed",
            RunState::Failed => "failed",
            RunState::Stopped => "stopped",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        ALL_STATES.into_iter().find(|k| k.as_str() == s)
    }
    /// Has (or is about to have) a process.
    pub fn is_active(self) -> bool {
        matches!(self, RunState::SettingUp | RunState::Starting | RunState::Ready | RunState::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStep {
    Setup,
    Command,
}

impl RunStep {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStep::Setup => "setup",
            RunStep::Command => "command",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [RunStep::Setup, RunStep::Command].into_iter().find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Run {
    pub id: String,
    pub project_id: String,
    /// None means the project's main checkout.
    pub worktree_id: Option<String>,
    pub branch: String,
    /// The `[[run]]`/`[[test]]` name in the effective configuration.
    pub name: String,
    pub kind: RunKind,
    pub state: RunState,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub data_dir: Option<String>,
    /// Working directory, native separators.
    pub cwd: String,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewRun {
    /// Run in this worktree. With neither this nor `branch`, the run uses the main checkout.
    #[serde(default)]
    #[ts(optional)]
    pub worktree_id: Option<String>,
    /// Run this branch; a branch without a worktree gets one.
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
    pub name: String,
    pub kind: RunKind,
    /// Confirmed by the user: stop this run to stay within the host's maximum.
    #[serde(default)]
    #[ts(optional)]
    pub stop_run_id: Option<String>,
}

pub fn pty_id(run_id: &str, step: RunStep) -> String {
    format!("run:{run_id}:{}", step.as_str())
}

/// Kept per project, branch and run name (spec §4.2 "Data"): `<data>/runs/<project>/<branch>-<hash>/<name>`. The
/// hash of the full branch name keeps branches that flatten to the same folder (`feature/x`, `feature-x`) apart.
/// Each part is made a single plain folder name, whatever the input (names are also validated with the run
/// configuration).
pub fn run_data_dir(data_root: &Path, project_id: &str, branch: &str, name: &str) -> PathBuf {
    let branch_dir = format!("{}-{:08x}", folder_name(branch), fnv1a(branch.as_bytes()));
    data_root.join("runs").join(project_id).join(branch_dir).join(folder_name(name))
}

/// `s` as one folder name on every OS: separators, `:<>|"?*` and control characters become `-`, and a result of
/// ``, `.` or `..` gets a `_` in front.
fn folder_name(s: &str) -> String {
    let out: String = s.chars().map(|c| if c.is_control() || "/\\:<>|\"?*".contains(c) { '-' } else { c }).collect();
    if matches!(out.as_str(), "" | "." | "..") { format!("_{out}") } else { out }
}

/// 32-bit FNV-1a: small, and stable across Rust versions (unlike `DefaultHasher`), since it names folders on disk.
fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |h, b| (h ^ u32::from(*b)).wrapping_mul(0x0100_0193))
}

/// A port in `range` that no active run holds and `is_free` accepts; `prefer` first when it qualifies.
pub fn pick_port(
    range: PortRange,
    taken: &BTreeSet<u16>,
    prefer: Option<u16>,
    mut is_free: impl FnMut(u16) -> bool,
) -> Option<u16> {
    let ok =
        |p: u16, f: &mut dyn FnMut(u16) -> bool| (range.start..=range.end).contains(&p) && !taken.contains(&p) && f(p);
    if let Some(p) = prefer
        && ok(p, &mut is_free)
    {
        return Some(p);
    }
    (range.start..=range.end).find(|&p| ok(p, &mut is_free))
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepLaunch {
    pub program: String,
    pub args: Vec<String>,
    /// Windows: the text to write to `script_path` before launching.
    pub script: Option<String>,
}

/// How to run a step's command line: through a `.cmd` file on Windows (no PTY argument quoting), `sh -c` elsewhere.
///
/// On Windows the line is run with `call`: a batch file that runs a `.cmd` shim (pnpm, npm, …) without it hands
/// control to the shim for good, so the script's own `exit /b %ERRORLEVEL%` never runs and some shims' exit codes are
/// lost (ruling F22).
pub fn step_launch(windows: bool, comspec: &str, command: &str, script_path: &Path) -> StepLaunch {
    if windows {
        StepLaunch {
            program: comspec.to_string(),
            args: vec!["/d".into(), "/c".into(), script_path.to_string_lossy().into_owned()],
            script: Some(format!("@echo off\r\ncall {command}\r\nexit /b %ERRORLEVEL%\r\n")),
        }
    } else {
        StepLaunch { program: "/bin/sh".into(), args: vec!["-c".into(), command.into()], script: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::PortRange;
    use std::collections::BTreeSet;
    use std::path::Path;

    #[test]
    fn enum_strings_match_serde() {
        for s in [
            RunState::SettingUp,
            RunState::Starting,
            RunState::Ready,
            RunState::Running,
            RunState::Passed,
            RunState::Failed,
            RunState::Stopped,
        ] {
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{}\"", s.as_str()));
            assert_eq!(RunState::parse(s.as_str()), Some(s));
        }
        assert_eq!(RunState::SettingUp.as_str(), "setting_up");
        for k in [RunKind::Run, RunKind::Test] {
            assert_eq!(RunKind::parse(k.as_str()), Some(k));
        }
        let active: Vec<_> = [RunState::SettingUp, RunState::Starting, RunState::Ready, RunState::Running]
            .into_iter()
            .filter(|s| s.is_active())
            .collect();
        assert_eq!(active.len(), 4);
        assert!(![RunState::Passed, RunState::Failed, RunState::Stopped].iter().any(|s| s.is_active()));
    }

    #[test]
    fn ids_and_paths() {
        assert_eq!(pty_id("r1", RunStep::Setup), "run:r1:setup");
        assert_eq!(pty_id("r1", RunStep::Command), "run:r1:command");
        let d = run_data_dir(Path::new("data"), "p", "feature/x", "web");
        assert_eq!(d, Path::new("data").join("runs").join("p").join("feature-x-1573876a").join("web"));
        assert_ne!(
            run_data_dir(Path::new("data"), "p", "feature-x", "web"),
            d,
            "branches that flatten to the same folder name keep separate data"
        );
    }

    /// Whatever the names, the folder stays under `<data>/runs/<project>/` (configuration validation rejects such
    /// names too; this is the second line of defence).
    #[test]
    fn data_dirs_never_leave_the_runs_folder() {
        let root = Path::new("data").join("runs").join("p");
        for (branch, name) in
            [("main", ".."), ("main", "."), ("..", "web"), ("main", "a/../../b"), ("main", "c:\\x"), ("main", "")]
        {
            let d = run_data_dir(Path::new("data"), "p", branch, name);
            let rel = d.strip_prefix(&root).unwrap_or_else(|_| panic!("{d:?}"));
            let parts: Vec<_> = rel.components().collect();
            assert_eq!(parts.len(), 2, "{branch:?}/{name:?} -> {d:?}");
            assert!(parts.iter().all(|c| matches!(c, std::path::Component::Normal(_))), "{d:?}");
        }
    }

    #[test]
    fn port_picking() {
        let range = PortRange { start: 8400, end: 8403 };
        let taken: BTreeSet<u16> = [8400].into();
        assert_eq!(pick_port(range, &taken, None, |_| true), Some(8401));
        assert_eq!(pick_port(range, &taken, Some(8403), |_| true), Some(8403), "the preferred port when free");
        assert_eq!(pick_port(range, &taken, Some(8400), |_| true), Some(8401), "taken by another run");
        assert_eq!(pick_port(range, &taken, Some(9000), |_| true), Some(8401), "outside the range");
        assert_eq!(pick_port(range, &taken, None, |p| p == 8403), Some(8403), "bound by something else");
        assert_eq!(pick_port(range, &taken, None, |_| false), None);
    }

    #[test]
    fn step_launch_per_platform() {
        let w = step_launch(true, "C:\\Windows\\system32\\cmd.exe", "pnpm dev && echo ok", Path::new("C:\\d\\s.cmd"));
        assert_eq!(w.program, "C:\\Windows\\system32\\cmd.exe");
        assert_eq!(w.args, ["/d", "/c", "C:\\d\\s.cmd"]);
        // `call`, so a .cmd shim (pnpm.cmd) returns to the script instead of ending it (ruling F22).
        assert_eq!(w.script.as_deref(), Some("@echo off\r\ncall pnpm dev && echo ok\r\nexit /b %ERRORLEVEL%\r\n"));
        let u = step_launch(false, "", "pnpm dev && echo ok", Path::new("/unused"));
        assert_eq!(
            (u.program.as_str(), u.args.as_slice()),
            ("/bin/sh", &["-c".to_string(), "pnpm dev && echo ok".into()][..])
        );
        assert_eq!(u.script, None);
    }
}
