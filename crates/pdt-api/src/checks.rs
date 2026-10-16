//! The Doctor-style checks the first-run checklist needs (spec §3.1 "Login check", §6 "First run").

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use std::time::Duration;
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ToolCheck {
    pub found: bool,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostChecks {
    pub claude: ToolCheck,
    pub codex: ToolCheck,
    pub git: ToolCheck,
    pub tailscale: ToolCheck,
    #[ts(type = "number")]
    pub checked_at: i64,
}

/// Runs `program args` (resolved like a session's program) with a 15 s limit: Err = not found / not runnable.
async fn run(st: &AppState, program: &str, args: &[&str]) -> Result<(bool, String), String> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (exe, args) =
        pdt_agents::launch::resolve(program, &args, st.platform.login_path.as_deref()).map_err(|e| e.to_string())?;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
    if let Some(p) = &st.platform.login_path {
        cmd.env("PATH", p);
    }
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") || k == "CLAUDECODE" {
            cmd.env_remove(k);
        }
    }
    let out = tokio::time::timeout(Duration::from_secs(15), cmd.output())
        .await
        .map_err(|_| "timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text.trim().to_string()))
}

/// `claude auth status` prints JSON with `loggedIn` (verified fact 3).
pub(crate) fn claude_logged_in(stdout: &str) -> Option<bool> {
    let start = stdout.find('{')?;
    serde_json::from_str::<serde_json::Value>(&stdout[start..]).ok()?.get("loggedIn")?.as_bool()
}

pub async fn run_host_checks(st: &AppState) -> HostChecks {
    let claude = match run(st, &st.platform.claude_program, &["auth", "status"]).await {
        Err(e) => ToolCheck { found: false, ok: false, detail: e },
        Ok((_, out)) => match claude_logged_in(&out) {
            Some(true) => ToolCheck { found: true, ok: true, detail: "logged in".into() },
            Some(false) => {
                ToolCheck { found: true, ok: false, detail: "not logged in: run `claude` and log in".into() }
            }
            None => ToolCheck { found: true, ok: false, detail: format!("could not read `claude auth status`: {out}") },
        },
    };
    let codex = match run(st, &st.platform.codex_program, &["login", "status"]).await {
        Err(e) => ToolCheck { found: false, ok: false, detail: e },
        Ok((true, out)) => ToolCheck { found: true, ok: true, detail: out },
        Ok((false, out)) => {
            ToolCheck { found: true, ok: false, detail: format!("not logged in: run `codex login` ({out})") }
        }
    };
    let git = match run(st, "git", &["--version"]).await {
        Err(e) => ToolCheck { found: false, ok: false, detail: e },
        Ok((ok, out)) => ToolCheck { found: true, ok, detail: out },
    };
    let ts = st.tailscale.clone();
    let tailscale = match tokio::task::spawn_blocking(move || ts.status()).await {
        Ok(Ok(s)) => ToolCheck { found: true, ok: true, detail: s.dns_name },
        Ok(Err(e)) => {
            ToolCheck { found: !matches!(e, pdt_tailscale::TsError::NotInstalled), ok: false, detail: e.to_string() }
        }
        Err(e) => ToolCheck { found: false, ok: false, detail: e.to_string() },
    };
    let c = HostChecks { claude, codex, git, tailscale, checked_at: now_ms() };
    *st.checks.lock().unwrap_or_else(|e| e.into_inner()) = Some(c.clone());
    c
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
}
