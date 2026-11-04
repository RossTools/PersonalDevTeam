//! The Doctor-style checks the first-run checklist needs.

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
    /// Whether git has a name and email to commit with (`user.name`, `user.email`): merges and updates need them.
    pub git_identity: ToolCheck,
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
    let program_name = program.to_string();
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (program, login_path) = (program.to_string(), st.platform.login_path.clone());
    // Finding the program scans the search path and starting it can take a second on a loaded Windows host
    // (PDT-48): both run on the blocking pool, and only the wait for its output on the runtime.
    let started = tokio::task::spawn_blocking(move || {
        let l = pdt_agents::launch::resolve_launch(&program, &args, login_path.as_deref())
            .map_err(|e| RunError::NotFound(e.to_string()))?;
        // A Windows shim runs through cmd.exe: the program found is its third argument (`/d /c <shim>`).
        let path = if l.via_cmd { l.args[2].clone() } else { l.program.to_string_lossy().into_owned() };
        let mut cmd = tokio::process::Command::new(&l.program);
        cmd.args(&l.args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(p) = &login_path {
            cmd.env("PATH", p);
        }
        for (k, _) in std::env::vars_os() {
            if let Some(k) = k.to_str()
                && is_scrubbed(kind, k, creds)
            {
                cmd.env_remove(k);
            }
        }
        cmd.spawn().map(|child| (child, path.clone())).map_err(|e| RunError::Failed(e.to_string(), path))
    })
    .await
    .map_err(|e| RunError::Failed(e.to_string(), program_name.clone()))??;
    let (child, path) = started;
    let out = tokio::time::timeout(RUN_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| RunError::Failed(format!("timed out after {} s", RUN_TIMEOUT.as_secs()), path.clone()))?
        .map_err(|e| RunError::Failed(e.to_string(), path.clone()))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok(Ran { ok: out.status.success(), out: text.trim().to_string(), path })
}

/// How long one check may take. A CLI's first launch can be slow (it updates itself, or the OS scans a new binary),
/// and pdtd checks at startup while it restarts every interrupted session.
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

/// `claude auth status` prints JSON with `loggedIn`.
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
    let git_identity = if git.found { identity_check(st).await } else { ToolCheck::missing("git is missing".into()) };
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
    let c = HostChecks { claude, codex, devin, git, git_identity, tailscale, acp, checked_at: now_ms() };
    *st.checks.lock().unwrap_or_else(|e| e.into_inner()) = Some(c.clone());
    c
}

/// What to run when git has no identity.
const SET_IDENTITY: &str =
    "git config --global user.name \"Your Name\" and git config --global user.email \"you@example.com\"";

/// The check of git's identity: as each project's repository sees it (its own settings and the includes that apply
/// there count), or, with no project yet, in the global settings (see [`identity_verdict`]).
async fn identity_check(st: &AppState) -> ToolCheck {
    let repos: Vec<(String, std::path::PathBuf)> = match st.store.list_projects().await {
        Ok(ps) => ps.into_iter().map(|p| (p.name, std::path::PathBuf::from(p.repo_path))).collect(),
        Err(e) => {
            tracing::warn!(error = %e, "could not list the projects; checking git's global identity only");
            vec![]
        }
    };
    let read = tokio::task::spawn_blocking(move || -> Result<_, pdt_git::GitError> {
        let global = pdt_git::global_identity()?;
        // A project whose folder is gone can't say; its own page shows that.
        let projects = repos
            .into_iter()
            .filter(|(_, repo)| repo.is_dir())
            .map(|(name, repo)| Ok((name, pdt_git::repo_identity(&repo)?)))
            .collect::<Result<Vec<_>, pdt_git::GitError>>()?;
        Ok((global, projects))
    })
    .await;
    match read {
        Ok(Ok((global, projects))) => {
            let (ok, detail) = identity_verdict(&global, &projects);
            ToolCheck::answered(ok, detail)
        }
        Ok(Err(e)) => ToolCheck::answered(false, format!("could not read git's settings: {e}")),
        Err(e) => ToolCheck::answered(false, format!("could not read git's settings: {e}")),
    }
}

/// Whether git has a name and email to commit with, and what the check says. With projects: whether each project's
/// repository has both (the ones that lack them named); without: whether the global settings have them.
fn identity_verdict(global: &pdt_git::Identity, projects: &[(String, pdt_git::Identity)]) -> (bool, String) {
    let shown = |id: &pdt_git::Identity| match (&id.name, &id.email) {
        (Some(n), Some(e)) => Some(format!("{n} <{e}>")),
        _ => None,
    };
    if projects.is_empty() {
        return match shown(global) {
            Some(s) => (true, s),
            None => (false, format!("no name and email in your global git settings: run {SET_IDENTITY}")),
        };
    }
    let lacking: Vec<&str> = projects.iter().filter(|(_, id)| !id.is_complete()).map(|(n, _)| n.as_str()).collect();
    if lacking.is_empty() {
        return (true, shown(global).unwrap_or_else(|| "set in each project's repository".into()));
    }
    let which = if lacking.len() == projects.len() { "any project".to_string() } else { lacking.join(", ") };
    (false, format!("no name and email for {which}, so merges there fail or get a made-up address: run {SET_IDENTITY}"))
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

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new().route("/host/checks", get(get_checks)).route("/host/checks/refresh", post(refresh_checks))
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
    fn checks_the_identity_each_project_commits_with() {
        let id =
            |n: Option<&str>, e: Option<&str>| pdt_git::Identity { name: n.map(Into::into), email: e.map(Into::into) };
        let (ada, none) = (id(Some("Ada"), Some("a@b")), id(None, None));
        let verdict = super::identity_verdict;
        assert_eq!(verdict(&ada, &[]), (true, "Ada <a@b>".to_string()));
        let (ok, detail) = verdict(&id(Some("Ada"), None), &[]);
        assert!(!ok && detail.starts_with("no name and email in your global git settings"), "{detail}");
        assert!(detail.contains("git config --global user.email"), "{detail}");
        // Set per repository or by an include: fine, though the global settings have none.
        let work = id(Some("Ada"), Some("ada@work"));
        assert_eq!(
            verdict(&none, &[("Shop".into(), work.clone()), ("Blog".into(), ada.clone())]),
            (true, "set in each project's repository".to_string())
        );
        let (ok, detail) = verdict(&none, &[("Shop".into(), work), ("Blog".into(), none.clone())]);
        assert!(!ok && detail.starts_with("no name and email for Blog, so merges there fail"), "{detail}");
        let (_, detail) = verdict(&none, &[("Blog".into(), none.clone())]);
        assert!(detail.starts_with("no name and email for any project"), "{detail}");
    }

    #[test]
    fn reads_devin_auth_status() {
        let out = "Not logged in.\n  Credentials path: /u/.local/share/devin/credentials.toml\nRun `devin auth login`";
        assert!(!super::devin_logged_in(true, out));
        assert!(super::devin_logged_in(true, "Logged in as someone"));
        assert!(!super::devin_logged_in(false, "Logged in as someone"));
    }
}
