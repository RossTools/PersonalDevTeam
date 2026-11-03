use pdt_api::run_host_checks;
use pdt_tailscale::testing::RecordingTailscale;
use std::sync::Arc;

/// A fake program printing `out` and exiting with `code`.
fn fake(dir: &std::path::Path, name: &str, out: &str, code: i32) -> String {
    if cfg!(windows) {
        let p = dir.join(format!("{name}.cmd"));
        std::fs::write(&p, format!("@echo off\r\necho {out}\r\nexit /b {code}\r\n")).unwrap();
        p.to_string_lossy().into()
    } else {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\necho '{out}'\nexit {code}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p.to_string_lossy().into()
    }
}

#[tokio::test]
async fn reports_logins_and_missing_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut st = crate::common::app_state(store, tmp.path());
    st.platform.claude_program = fake(tmp.path(), "claude", r#"{"loggedIn": false, "authMethod": "none"}"#, 0);
    st.platform.codex_program = fake(tmp.path(), "codex", "Logged in using ChatGPT", 0);
    st.platform.devin_program = fake(tmp.path(), "devin", "Not logged in.", 0);
    st.tailscale = Arc::new(RecordingTailscale::new(Err(pdt_tailscale::TsError::NotInstalled)));
    let c = run_host_checks(&st).await;
    assert!(c.claude.found && !c.claude.ok, "{:?}", c.claude);
    assert!(c.codex.found && c.codex.ok, "{:?}", c.codex);
    assert!(c.devin.found && !c.devin.ok, "{:?}", c.devin);
    assert!(c.git.found && c.git.ok, "{:?}", c.git);
    assert!(!c.tailscale.ok && c.tailscale.detail.contains("not found"));

    st.platform.claude_program = tmp.path().join("no-such-claude").to_string_lossy().into();
    st.platform.codex_program = fake(tmp.path(), "codex2", "Not logged in", 1);
    st.platform.devin_program = tmp.path().join("no-such-devin").to_string_lossy().into();
    let c = run_host_checks(&st).await;
    assert!(!c.claude.found && !c.claude.ok);
    assert!(!c.devin.found && !c.devin.ok);
    assert!(c.codex.found && !c.codex.ok);
}

/// A program that is installed but whose check never answers (here: it cannot start) is not reported missing, so its
/// agent stays offered, and the startup check knows to run again.
#[cfg(unix)]
#[tokio::test]
async fn an_unanswered_check_is_not_missing() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut st = crate::common::app_state(store, tmp.path());
    st.platform.claude_program = fake(tmp.path(), "claude", r#"{"loggedIn": true}"#, 0);
    st.platform.codex_program = fake(tmp.path(), "codex", "Logged in using ChatGPT", 0);
    let devin = tmp.path().join("devin");
    std::fs::write(&devin, "#!/no/such/interpreter\n").unwrap();
    std::fs::set_permissions(&devin, std::fs::Permissions::from_mode(0o755)).unwrap();
    st.platform.devin_program = devin.to_string_lossy().into();
    st.tailscale = Arc::new(RecordingTailscale::new(Err(pdt_tailscale::TsError::NotInstalled)));
    let c = run_host_checks(&st).await;
    assert!(c.devin.found && !c.devin.ok && c.devin.detail.starts_with("could not check devin"), "{:?}", c.devin);
    assert!(c.agents_inconclusive());

    st.platform.devin_program = fake(tmp.path(), "devin2", "Logged in (via Devin).", 0);
    let c = run_host_checks(&st).await;
    assert!(c.devin.found && c.devin.ok, "{:?}", c.devin);
    assert!(!c.agents_inconclusive());
}

/// `GET /api/host/checks` runs the checks the first time and then answers from the last run; Refresh runs them again.
#[tokio::test]
async fn get_answers_from_the_last_run_until_a_refresh() {
    use axum::http::StatusCode;
    let tmp = tempfile::tempdir().unwrap();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut st = crate::common::app_state(store, tmp.path());
    st.platform.claude_program = tmp.path().join("no-such-claude").to_string_lossy().into();
    st.platform.codex_program = fake(tmp.path(), "codex", "Logged in using ChatGPT", 0);
    st.platform.devin_program = tmp.path().join("no-such-devin").to_string_lossy().into();
    st.tailscale = Arc::new(RecordingTailscale::new(Err(pdt_tailscale::TsError::NotInstalled)));
    let app = pdt_api::router(st, None);
    let (s, first) = crate::common::call(&app, "GET", "/api/host/checks", None).await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!((first["codex"]["ok"].as_bool(), first["claude"]["found"].as_bool()), (Some(true), Some(false)));
    assert!(first["checkedAt"].as_i64().unwrap() > 0, "{first}");

    // Codex logs out: the next GET still answers from the last run.
    fake(tmp.path(), "codex", "Not logged in", 1);
    let (_, again) = crate::common::call(&app, "GET", "/api/host/checks", None).await;
    assert_eq!(again, first, "from the last run");
    let (s, fresh) = crate::common::call(&app, "POST", "/api/host/checks/refresh", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((fresh["codex"]["found"].as_bool(), fresh["codex"]["ok"].as_bool()), (Some(true), Some(false)));
    let (_, after) = crate::common::call(&app, "GET", "/api/host/checks", None).await;
    assert_eq!(after, fresh, "the refresh is the last run now");
}
