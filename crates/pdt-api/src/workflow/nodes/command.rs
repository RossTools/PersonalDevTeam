//! `command` (Phase 7e spec §3.1): runs a program (no shell) with JSON on stdin and reads JSON from stdout, in a scratch
//! folder, the project's checkout or the attempt's worktree. A non-zero exit is an error (so retries and the error port
//! apply); the node's time limit kills the whole process tree.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use ts_rs::TS;

/// How much of stdout and stderr a step keeps.
pub const OUTPUT_CAP: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ParseAs {
    /// stdout is one JSON value (empty: null).
    #[default]
    Json,
    Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CommandConfig {
    /// The program: a name on PATH or a full path. No shell: use `sh -c …` / `cmd /c …` for one.
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// `scratch` (a fresh folder for the run), `project` (the project's main checkout), `attempt` (the attempt's
    /// worktree) or a full path.
    #[serde(default = "scratch")]
    pub cwd: String,
    /// Written to stdin as JSON (nothing when absent).
    #[serde(default)]
    #[ts(type = "unknown")]
    pub input: Value,
    /// Added to PDT's environment.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub parse: ParseAs,
}

fn scratch() -> String {
    "scratch".into()
}

/// What a command step outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommandOutput {
    pub exit_code: i32,
    /// stdout as JSON (with `parse: json`).
    #[ts(type = "unknown")]
    pub json: Value,
    pub stdout: String,
    /// The end of stderr.
    pub stderr: String,
    /// The JSON said `quiet: true`: nothing to report.
    pub quiet: bool,
}

pub struct Command;

/// The scratch folders of a run's command steps.
pub fn scratch_dir(data_dir: &Path, run_id: &str) -> PathBuf {
    data_dir.join("workflow-scratch").join(run_id)
}

impl NodeKind for Command {
    fn spec(&self) -> KindSpec {
        spec::<CommandConfig, CommandOutput>(
            "command",
            1,
            Category::Action,
            "Run a command",
            "Runs a program with `input` as JSON on stdin and reads its stdout as JSON (`{{steps.<id>.json}}`). A \
             non-zero exit fails the step; `{\"quiet\": true}` on stdout means nothing to report.",
            &[("out", "Then")],
            false,
        )
    }

    /// What it does can't be told apart from doing it twice (PDT-131): cut short, it isn't run again on its own.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: CommandConfig = parse(config)?;
        if c.program.trim().is_empty() {
            return Err("program is empty".into());
        }
        let cwd = c.cwd.trim();
        if !matches!(cwd, "scratch" | "project" | "attempt") && !cwd.contains("{{") && !Path::new(cwd).is_absolute() {
            return Err(format!("cwd {cwd:?} is not scratch, project, attempt or a full path"));
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CommandConfig = parse(&input)?;
            if cx.dry_run {
                let line = std::iter::once(c.program.as_str()).chain(c.args.iter().map(String::as_str));
                let line: Vec<&str> = line.collect();
                return Ok(Outcome::next("out", json!({"would": format!("run {} in {}", line.join(" "), c.cwd)})));
            }
            let dir = working_dir(cx, &c.cwd).await?;
            let out = run_command(&c, &dir).await?;
            Ok(Outcome::next("out", out))
        })
    }
}

async fn working_dir(cx: &StepCx<'_>, cwd: &str) -> Result<PathBuf, String> {
    let st = cx.st;
    match cwd.trim() {
        "scratch" => {
            let d = scratch_dir(&st.data_dir, &cx.run.id);
            tokio::fs::create_dir_all(&d).await.map_err(|e| format!("could not make {}: {e}", d.display()))?;
            Ok(d)
        }
        "project" => {
            let id = cx.run.project_id.as_deref().ok_or("the run has no project")?;
            let p = st.store.get_project(id).await.map_err(|e| e.to_string())?.ok_or("the run's project is gone")?;
            Ok(PathBuf::from(p.repo_path))
        }
        "attempt" => {
            let id = cx.run.attempt_id.as_deref().ok_or("the run has no attempt")?;
            let a = st.store.get_attempt(id).await.map_err(|e| e.to_string())?.ok_or("the run's attempt is gone")?;
            a.worktree_path.map(PathBuf::from).ok_or_else(|| "the attempt has no worktree".into())
        }
        path => Ok(PathBuf::from(path)),
    }
}

/// Kills the child's whole process tree when dropped (a time limit drops the step's future).
struct TreeGuard(Option<u32>);

impl Drop for TreeGuard {
    fn drop(&mut self) {
        let Some(pid) = self.0 else { return };
        #[cfg(unix)]
        if let Ok(pid) = i32::try_from(pid) {
            // The child leads its own process group.
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x0800_0000)
                .spawn();
        }
    }
}

/// Reads all of `r`, keeping the first `cap` bytes.
async fn read_capped(mut r: impl AsyncRead + Unpin, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut buf = [0u8; 8192];
    let mut cut = false;
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => return (kept, cut),
            Ok(n) => {
                let room = cap.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
                cut |= n > room;
            }
        }
    }
}

fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
}

pub(crate) async fn run_command(c: &CommandConfig, dir: &Path) -> Result<CommandOutput, String> {
    let mut cmd = tokio::process::Command::new(&c.program);
    cmd.args(&c.args)
        .current_dir(dir)
        .envs(&c.env)
        .stdin(if c.input.is_null() { Stdio::null() } else { Stdio::piped() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().map_err(|e| format!("could not run {}: {e}", c.program))?;
    let mut guard = TreeGuard(child.id());
    if let Some(mut stdin) = child.stdin.take() {
        let body = serde_json::to_vec(&c.input).unwrap_or_default();
        // A program that doesn't read stdin closes it early: that is not an error.
        let _ = stdin.write_all(&body).await;
        drop(stdin);
    }
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let (out, err, status) =
        tokio::join!(read_capped(stdout, OUTPUT_CAP), read_capped(stderr, 1024 * 1024), child.wait());
    let status = status.map_err(|e| e.to_string())?;
    guard.0 = None;
    let stdout = String::from_utf8_lossy(&out.0).into_owned();
    let stderr = tail(&String::from_utf8_lossy(&err.0), OUTPUT_CAP);
    let code = status.code().unwrap_or(-1);
    if !status.success() {
        let last: Vec<&str> = stderr.lines().rev().filter(|l| !l.trim().is_empty()).take(5).collect();
        let last: Vec<&str> = last.into_iter().rev().collect();
        let why = if last.is_empty() { tail(stdout.trim(), 500) } else { last.join("\n") };
        return Err(format!("{} exited with {code}: {why}", c.program));
    }
    let json = match c.parse {
        ParseAs::Text => Value::Null,
        ParseAs::Json if stdout.trim().is_empty() => Value::Null,
        ParseAs::Json if out.1 => return Err(format!("stdout is longer than {OUTPUT_CAP} bytes")),
        ParseAs::Json => serde_json::from_str(stdout.trim())
            .map_err(|e| format!("stdout is not JSON ({e}); use parse: text for plain output"))?,
    };
    let quiet = json.get("quiet").and_then(|q| q.as_bool()) == Some(true);
    Ok(CommandOutput { exit_code: code, json, stdout, stderr, quiet })
}
