//! The Doctor-style checks the first-run checklist needs (spec §3.1 "Login check", §6 "First run").

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use pdt_core::host::{CredentialPolicy, HostSettings};
use pdt_core::session::{SessionKind, is_scrubbed};
use serde::Serialize;
use std::time::Duration;
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ToolCheck {
    pub found: bool,
    pub ok: bool,
    pub detail: String,
    /// Where the program was found.
    pub path: Option<String>,
    /// The program is there but its check did not finish (timed out, could not start): worth running again.
    #[serde(skip)]
    #[ts(skip)]
    pub inconclusive: bool,
}

impl ToolCheck {
    fn missing(detail: String) -> Self {
        ToolCheck { found: false, ok: false, detail, path: None, inconclusive: false }
    }

    fn answered(ok: bool, detail: String) -> Self {
        ToolCheck { found: true, ok, detail, path: None, inconclusive: false }
    }

    /// Found, but its check said nothing: never reported as missing, so the agent stays offered.
    fn unanswered(program: &str, error: String) -> Self {
        ToolCheck {
            found: true,
            ok: false,
            detail: format!("could not check {program}: {error}"),
            path: None,
            inconclusive: true,
        }
    }
}

/// Why a check's program said nothing.
enum RunError {
    /// Not on the search path.
    NotFound(String),
    /// Found (at the path), but it timed out or could not be started.
    Failed(String, String),
}

/// What a check's program said: whether it succeeded, its output, and where it was found.
struct Ran {
    ok: bool,
    out: String,
    path: String,
}

/// The check for a program that ran: `answer` reads its (success, output).
fn check(program: &str, r: Result<Ran, RunError>, answer: impl FnOnce(bool, String) -> ToolCheck) -> ToolCheck {
    let (c, path) = match r {
        Err(RunError::NotFound(e)) => return ToolCheck::missing(e),
        Err(RunError::Failed(e, path)) => (ToolCheck::unanswered(program, e), path),
        Ok(Ran { ok, out, path }) => (answer(ok, out), path),
    };
    ToolCheck { path: Some(path), ..c }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostChecks {
    pub claude: ToolCheck,
    pub codex: ToolCheck,
    /// Optional: most hosts never install Devin, so a missing one is no problem.
    pub devin: ToolCheck,
    pub git: ToolCheck,
    pub tailscale: ToolCheck,
    /// Each ACP agent's (by agent kind, `acp:<id>`): whether its command is found. PDT can't tell whether one is
    /// logged in, so a found one is `ok`.
    #[ts(type = "Record<string, ToolCheck>")]
    pub acp: std::collections::BTreeMap<String, ToolCheck>,
    #[ts(type = "number")]
    pub checked_at: i64,
}

/// Runs `program args` (resolved like a session's program) with a time limit. The
/// inherited environment is scrubbed as for a session of `kind` under `creds`, so a check sees what sessions see
/// (an API key a Subscription session never gets must not make the check report logged in).
async fn run(
    st: &AppState,
    program: &str,
    args: &[&str],
    kind: SessionKind,
    creds: CredentialPolicy,
) -> Result<Ran, RunError> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let l = pdt_agents::launch::resolve_launch(program, &args, st.platform.login_path.as_deref())
        .map_err(|e| RunError::NotFound(e.to_string()))?;
    // A Windows shim runs through cmd.exe: the program found is its third argument (`/d /c <shim>`).
    let path = if l.via_cmd { l.args[2].clone() } else { l.program.to_string_lossy().into_owned() };
    let mut cmd = tokio::process::Command::new(&l.program);
    cmd.args(&l.args).stdin(std::process::Stdio::null()).kill_on_drop(true);
    if let Some(p) = &st.platform.login_path {
        cmd.env("PATH", p);
    }
    for (k, _) in std::env::vars_os() {
        if let Some(k) = k.to_str()
            && is_scrubbed(kind, k, creds)
        {
            cmd.env_remove(k);
        }
    }
    let out = tokio::time::timeout(RUN_TIMEOUT, cmd.output())
        .await
        .map_err(|_| RunError::Failed(format!("timed out after {} s", RUN_TIMEOUT.as_secs()), path.clone()))?
        .map_err(|e| RunError::Failed(e.to_string(), path.clone()))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok(Ran { ok: out.status.success(), out: text.trim().to_string(), path })
}

/// How long one check may take. A CLI's first launch can be slow (it updates itself, or the OS scans a new binary),
/// and pdtd checks at startup while it restarts every interrupted session.
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

/// `claude auth status` prints JSON with `loggedIn` (verified fact 3).
pub(crate) fn claude_logged_in(stdout: &str) -> Option<bool> {
    let start = stdout.find('{')?;
    serde_json::from_str::<serde_json::Value>(&stdout[start..]).ok()?.get("loggedIn")?.as_bool()
}

/// `devin auth status` exits 0 either way; logged out, it says "Not logged in.".
pub(crate) fn devin_logged_in(exit_ok: bool, out: &str) -> bool {
    exit_ok && !out.to_ascii_lowercase().contains("not logged in")
}

pub async fn run_host_checks(st: &AppState) -> HostChecks {
    // The host's policy: the one agent sessions get unless their project overrides it.
    let host = match st.store.get_host_settings().await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "could not read the host settings; checking under the defaults");
            HostSettings::default()
        }
    };
    let creds = host.agent_credentials;
    let platform = st.agent_platform().await;
    let agent = |program: &str, args: &'static [&'static str]| {
        let program = program.to_string();
        async move { run(st, &program, args, SessionKind::AgentTui, creds).await }
    };
    let claude = check("claude", agent(&platform.claude_program, &["auth", "status"]).await, |_, out| {
        match claude_logged_in(&out) {
            Some(true) => ToolCheck::answered(true, "logged in".into()),
            Some(false) => ToolCheck::answered(false, "not logged in: run `claude` and log in".into()),
            None => ToolCheck::answered(false, format!("could not read `claude auth status`: {out}")),
        }
    });
    let codex = check("codex", agent(&platform.codex_program, &["login", "status"]).await, |ok, out| {
        if ok {
            ToolCheck::answered(true, out)
        } else {
            ToolCheck::answered(false, format!("not logged in: run `codex login` ({out})"))
        }
    });
    let devin = check("devin", agent(&platform.devin_program, &["auth", "status"]).await, |ok, out| {
        if devin_logged_in(ok, &out) {
            // Its status goes on to list the account and team settings; the first line says it all.
            ToolCheck::answered(true, out.lines().next().unwrap_or_default().trim().to_string())
        } else {
            ToolCheck::answered(false, format!("not logged in: run `devin auth login` ({out})"))
        }
    });
    let git = check("git", run(st, "git", &["--version"], SessionKind::Shell, creds).await, ToolCheck::answered);
    let ts = st.tailscale.clone();
    let tailscale = match tokio::task::spawn_blocking(move || ts.status()).await {
        Ok(Ok(s)) => ToolCheck::answered(true, s.dns_name),
        Ok(Err(e)) if matches!(e, pdt_tailscale::TsError::NotInstalled) => ToolCheck::missing(e.to_string()),
        Ok(Err(e)) => ToolCheck::answered(false, e.to_string()),
        Err(e) => ToolCheck::missing(e.to_string()),
    };
    let acp = crate::host::acp_agent_views(st, &host)
        .await
        .into_iter()
        .map(|v| {
            let check = match v.found {
                Some(path) => ToolCheck { path: Some(path), ..ToolCheck::answered(true, "found".into()) },
                None => ToolCheck::missing(format!("{} was not found", v.def.command)),
            };
            (v.agent.as_str().to_string(), check)
        })
        .collect();
    let c = HostChecks { claude, codex, devin, git, tailscale, acp, checked_at: now_ms() };
    *st.checks.lock().unwrap_or_else(|e| e.into_inner()) = Some(c.clone());
    c
}

impl HostChecks {
    /// Whether an agent's check did not finish, so running the checks again may tell more.
    pub fn agents_inconclusive(&self) -> bool {
        [&self.claude, &self.codex, &self.devin].iter().any(|c| c.inconclusive)
    }
}

pub(crate) async fn get_checks(State(st): State<AppState>) -> Result<Json<HostChecks>, ApiError> {
    let cached = st.checks.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Ok(Json(match cached {
        Some(c) => c,
        None => run_host_checks(&st).await,
    }))
}

pub(crate) async fn refresh_checks(State(st): State<AppState>) -> Json<HostChecks> {
    Json(run_host_checks(&st).await)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_claude_auth_status() {
        assert_eq!(super::claude_logged_in("{\n \"loggedIn\": true, \"authMethod\": \"claude.ai\"\n}"), Some(true));
        assert_eq!(super::claude_logged_in("warning\n{\"loggedIn\": false}"), Some(false));
        assert_eq!(super::claude_logged_in("Logged in"), None);
    }

    #[test]
    fn reads_devin_auth_status() {
        let out = "Not logged in.\n  Credentials path: /u/.local/share/devin/credentials.toml\nRun `devin auth login`";
        assert!(!super::devin_logged_in(true, out));
        assert!(super::devin_logged_in(true, "Logged in as someone"));
        assert!(!super::devin_logged_in(false, "Logged in as someone"));
    }
}
