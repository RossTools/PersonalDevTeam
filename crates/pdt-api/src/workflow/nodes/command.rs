//! `command` (Phase 7e spec §3.1): runs a program (no shell) or a shell script (PDT-250) with JSON on stdin and reads
//! JSON from stdout, in a scratch folder, the project's checkout or the attempt's worktree. A non-zero exit is an error
//! (so retries and the error port apply); the node's time limit kills the whole process tree.

use crate::error::ApiError;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, setup_error, spec};
use pdt_core::secret::Redactor;
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

/// The shell a `script` runs in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Shell {
    #[default]
    Sh,
    Bash,
    Zsh,
    Pwsh,
}

impl Shell {
    fn program(self) -> &'static str {
        match self {
            Shell::Sh => "sh",
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Pwsh => "pwsh",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(extend("oneOf" = [{"required": ["program"]}, {"required": ["script"]}]))]
#[ts(export)]
pub struct CommandConfig {
    /// The program: a name on PATH or a full path, run without a shell. Or leave it out and write a `script`.
    #[serde(default)]
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// A script, run as `<shell> -c <script>` instead of `program`. It can't hold `{{…}}` references (only a
    /// template's `{{params.*}}`): they would be pasted in unquoted, so a PR title could run code. Put them in `env` and
    /// use `"$NAME"` (`$env:NAME` in pwsh) in the script.
    #[serde(default)]
    #[schemars(extend("x-format" = "code", "x-language" = "sh"))]
    #[ts(optional)]
    pub script: Option<String>,
    /// The shell for `script`.
    #[serde(default)]
    pub shell: Shell,
    /// `scratch` (a fresh folder for the run), `project` (the project's main checkout), `attempt` (the attempt's
    /// worktree) or a full path.
    #[serde(default = "scratch")]
    pub cwd: String,
    /// Written to stdin as JSON (nothing when absent).
    #[serde(default)]
    #[schemars(extend("x-format" = "code", "x-language" = "json"))]
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

impl CommandConfig {
    /// What runs: the program and its arguments, or the shell with `-c` and the script.
    fn command_line(&self) -> (&str, Vec<&str>) {
        match self.script.as_deref() {
            Some(script) => (self.shell.program(), vec!["-c", script]),
            None => (self.program.as_str(), self.args.iter().map(String::as_str).collect()),
        }
    }
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
    fn needs_project(&self, config: &Value) -> bool {
        config.get("cwd").and_then(Value::as_str).is_some_and(|c| c.trim() == "project")
    }

    fn spec(&self) -> KindSpec {
        spec::<CommandConfig, CommandOutput>(
            "command",
            1,
            Category::Action,
            "Run a command",
            "Runs a program, or a shell script, with `input` as JSON on stdin and reads its stdout as JSON \
             (`{{steps.<id>.json}}`). A non-zero exit fails the step; `{\"quiet\": true}` on stdout means nothing to \
             report.",
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
        match (&c.script, c.program.trim().is_empty()) {
            (Some(_), false) => return Err("set program or script, not both".into()),
            (Some(_), true) if !c.args.is_empty() => return Err("args go with program; a script has none".into()),
            (Some(s), true) if s.trim().is_empty() => return Err("script is empty".into()),
            (None, true) => return Err("program is empty (set program or script)".into()),
            _ => {}
        }
        if let Some(script) = &c.script {
            script_refs_ok(script)?;
        }
        let cwd = c.cwd.trim();
        if !matches!(cwd, "scratch" | "project" | "attempt") && !cwd.contains("{{") && !Path::new(cwd).is_absolute() {
            return Err(format!("cwd {cwd:?} is not scratch, project, attempt or a full path"));
        }
        Ok(())
    }

    /// A workflow saved before scripts refused references would otherwise still paste outside text into the shell.
    fn check_at_run(&self, config: &Value) -> Result<(), String> {
        match config.get("script").and_then(Value::as_str) {
            Some(script) => script_refs_ok(script),
            None => Ok(()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CommandConfig = parse(&input)?;
            if cx.dry_run {
                let would = match &c.script {
                    Some(script) => format!("run a {} script in {}:\n{script}", c.shell.program(), c.cwd),
                    None => {
                        let line = std::iter::once(c.program.as_str()).chain(c.args.iter().map(String::as_str));
                        format!("run {} in {}", line.collect::<Vec<_>>().join(" "), c.cwd)
                    }
                };
                return Ok(Outcome::next("out", json!({"would": would})));
            }
            let dir = working_dir(cx, &c.cwd).await?;
            let out = run_command(&c, &dir, cx.st.store.redactor()).await?;
            Ok(Outcome::next("out", out))
        })
    }
}

/// PDT-412: a script can't paste data references into the shell. Outside text (an issue title, a webhook body) would
/// run as code; in `env` it is only ever a value. A template's parameters are filled in when it is installed, by the
/// user, so they may stay.
fn script_refs_ok(script: &str) -> Result<(), String> {
    let bad: Vec<String> = pdt_core::workflow::refs::refs_in(&Value::String(script.to_string()))
        .into_iter()
        .filter(|r| r.split('.').next() != Some("params"))
        .collect();
    let Some(first) = bad.first() else { return Ok(()) };
    let name = first
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect::<String>();
    let name = if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) { "VALUE".into() } else { name };
    Err(format!(
        "script can't use {{{{{first}}}}}: it would be pasted into the shell unquoted. Put it in env \
         (\"{name}\": \"{{{{{first}}}}}\") and use \"${name}\" in the script"
    ))
}

async fn working_dir(cx: &StepCx<'_>, cwd: &str) -> Result<PathBuf, String> {
    let st = cx.st;
    match cwd.trim() {
        "scratch" => {
            let d = scratch_dir(&st.data_dir, &cx.run.id);
            tokio::fs::create_dir_all(&d)
                .await
                .map_err(|e| ApiError::io("making the scratch folder", e).to_string())?;
            Ok(d)
        }
        "project" => {
            let id = cx.run.project_id.as_deref().ok_or_else(|| setup_error("the run has no project to run in"))?;
            let p = st
                .store
                .get_project(id)
                .await
                .map_err(crate::workflow::err)?
                .ok_or_else(|| setup_error("the run's project is gone"))?;
            Ok(PathBuf::from(p.repo_path))
        }
        "attempt" => {
            let id = cx.run.attempt_id.as_deref().ok_or("the run has no attempt")?;
            let a = st.store.get_attempt(id).await.map_err(crate::workflow::err)?.ok_or("the run's attempt is gone")?;
            a.worktree_path.map(PathBuf::from).ok_or_else(|| "the attempt has no worktree".into())
        }
        path => Ok(PathBuf::from(path)),
    }
}

/// Kills the child's whole process tree when dropped (a time limit drops the step's future).
pub(crate) struct TreeGuard(pub(crate) Option<u32>);

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
pub(crate) async fn read_capped(mut r: impl AsyncRead + Unpin, cap: usize) -> (Vec<u8>, bool) {
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

/// Runs the step's command. What it printed is redacted before it is cut to size, so a value across the cut is never
/// left half there (the store redacts the step's record again, whole values only).
pub(crate) async fn run_command(c: &CommandConfig, dir: &Path, r: &Redactor) -> Result<CommandOutput, String> {
    let (program, args) = c.command_line();
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(&args)
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
    let mut child = cmd.spawn().map_err(|e| format!("could not run {program}: {e}"))?;
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
    let status = status.map_err(crate::workflow::err)?;
    guard.0 = None;
    let stdout = r.string_cut(&String::from_utf8_lossy(&out.0), out.1);
    let stderr = tail(&r.string_cut(&String::from_utf8_lossy(&err.0), err.1), OUTPUT_CAP);
    let code = status.code().unwrap_or(-1);
    if !status.success() {
        let last: Vec<&str> = stderr.lines().rev().filter(|l| !l.trim().is_empty()).take(5).collect();
        let last: Vec<&str> = last.into_iter().rev().collect();
        let why = if last.is_empty() { tail(stdout.trim(), 500) } else { last.join("\n") };
        return Err(format!("{program} exited with {code}: {why}"));
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
