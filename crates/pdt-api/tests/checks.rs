use pdt_api::{AppState, run_host_checks};
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
    let mut st = AppState::new(store, tmp.path().join("data"));
    st.platform.claude_program = fake(tmp.path(), "claude", r#"{"loggedIn": false, "authMethod": "none"}"#, 0);
    st.platform.codex_program = fake(tmp.path(), "codex", "Logged in using ChatGPT", 0);
    st.tailscale = Arc::new(RecordingTailscale::new(Err(pdt_tailscale::TsError::NotInstalled)));
    let c = run_host_checks(&st).await;
    assert!(c.claude.found && !c.claude.ok, "{:?}", c.claude);
    assert!(c.codex.found && c.codex.ok, "{:?}", c.codex);
    assert!(c.git.found && c.git.ok, "{:?}", c.git);
    assert!(!c.tailscale.ok && c.tailscale.detail.contains("not found"));

    st.platform.claude_program = tmp.path().join("no-such-claude").to_string_lossy().into();
    st.platform.codex_program = fake(tmp.path(), "codex2", "Not logged in", 1);
    let c = run_host_checks(&st).await;
    assert!(!c.claude.found && !c.claude.ok);
    assert!(c.codex.found && !c.codex.ok);
}
