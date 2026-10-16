//! Final review #4: the login checks see the environment agent sessions get. Its own test binary with one test,
//! because it sets variables in the process environment.
use pdt_api::{AppState, run_host_checks};
use pdt_core::host::CredentialPolicy;
use pdt_tailscale::testing::RecordingTailscale;
use std::sync::Arc;

/// A fake program that succeeds (and prints `{"loggedIn": true}`) only when `var` is set.
fn fake_needing(dir: &std::path::Path, name: &str, var: &str) -> String {
    if cfg!(windows) {
        let p = dir.join(format!("{name}.cmd"));
        let script = format!(
            "@echo off\r\nif defined {var} (\r\n  echo {{\"loggedIn\": true}}\r\n  exit /b 0\r\n)\r\necho {{\"loggedIn\": false}}\r\nexit /b 1\r\n"
        );
        std::fs::write(&p, script).unwrap();
        p.to_string_lossy().into()
    } else {
        let p = dir.join(name);
        let script = format!(
            "#!/bin/sh\nif [ -n \"${var}\" ]; then echo '{{\"loggedIn\": true}}'; exit 0; fi\necho '{{\"loggedIn\": false}}'\nexit 1\n"
        );
        std::fs::write(&p, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p.to_string_lossy().into()
    }
}

#[tokio::test]
async fn api_keys_in_pdtd_s_environment_count_only_where_sessions_get_them() {
    // SAFETY: the only test in this binary, on a current-thread runtime: no other thread reads the environment.
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", "sk-test-not-a-key");
        std::env::set_var("OPENAI_API_KEY", "sk-test-not-a-key");
    }
    let tmp = tempfile::tempdir().unwrap();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut st = AppState::new(store, tmp.path().join("data"));
    st.platform.claude_program = fake_needing(tmp.path(), "claude", "ANTHROPIC_API_KEY");
    st.platform.codex_program = fake_needing(tmp.path(), "codex", "OPENAI_API_KEY");
    st.tailscale = Arc::new(RecordingTailscale::new(Err(pdt_tailscale::TsError::NotInstalled)));

    let settings = st.store.get_host_settings().await.unwrap();
    assert_eq!(settings.agent_credentials, CredentialPolicy::Subscription, "the default");
    let c = run_host_checks(&st).await;
    assert!(c.claude.found && !c.claude.ok, "sessions get no API key under Subscription: {:?}", c.claude);
    assert!(c.codex.found && !c.codex.ok, "sessions get no API key under Subscription: {:?}", c.codex);

    let mut settings = settings;
    settings.agent_credentials = CredentialPolicy::PassThrough;
    st.store.put_host_settings(&settings).await.unwrap();
    let c = run_host_checks(&st).await;
    assert!(c.claude.ok, "PassThrough passes the key on: {:?}", c.claude);
    assert!(c.codex.ok, "PassThrough passes the key on: {:?}", c.codex);
}
