//! API keys in pdtd's environment reach only what the credential policy lets them: the login checks see the
//! environment agent sessions get (final review #4), and a conversation under the subscription policy gets none. Its
//! own test binary with one test, because it sets variables in the process environment.
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use pdt_api::{AppState, router, run_host_checks};
use pdt_core::host::CredentialPolicy;
use pdt_tailscale::testing::RecordingTailscale;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

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
async fn api_keys_in_pdtd_s_environment_reach_only_where_the_policy_lets_them() {
    // SAFETY: the only test in this binary, on a current-thread runtime: no other thread reads the environment.
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", "sk-test-not-a-key");
        std::env::set_var("OPENAI_API_KEY", "sk-test-not-a-key");
    }
    login_checks_count_keys_only_where_sessions_get_them().await;
    a_conversation_gets_no_api_key_under_the_subscription().await;
}

async fn login_checks_count_keys_only_where_sessions_get_them() {
    let tmp = tempfile::tempdir().unwrap();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut st = AppState::new(store, tmp.path().join("data"));
    st.platform.claude_program = fake_needing(tmp.path(), "claude", "ANTHROPIC_API_KEY");
    st.platform.codex_program = fake_needing(tmp.path(), "codex", "OPENAI_API_KEY");
    st.platform.devin_program = tmp.path().join("no-such-devin").to_string_lossy().into();
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

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

async fn a_conversation_gets_no_api_key_under_the_subscription() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut st = AppState::new(store, tmp.path().join("data"));
    st.platform.claude_program = env!("CARGO_BIN_EXE_fake-agent").to_string();
    st.platform.login_path = None;
    st.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    st.platform.codex_home = Some(tmp.path().join("codex-home"));
    st.home_dir = Some(tmp.path().to_path_buf());
    let app = router(st.clone(), None);
    let (s, p) = call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo}))).await;
    assert_eq!(s, 201, "{p}");
    let body = json!({"projectId": p["id"], "kind": "agent_tui", "agent": "claude", "cols": 80, "rows": 24,
        "initialPrompt": "env: ANTHROPIC_API_KEY"});
    let (s, v) = call(&app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, 201, "{v}");
    let id = v["id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let said = loop {
        let (_, all) = call(&app, "GET", &format!("/api/sessions/{id}/events?after=0"), None).await;
        let all = all.as_array().unwrap().clone();
        if all.iter().any(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended") {
            break all.into_iter().filter(|e| e["event"]["kind"] == "agent_message").collect::<Vec<_>>();
        }
        assert!(Instant::now() < deadline, "the turn never ended: {all:#?}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(said.len(), 1, "{said:#?}");
    assert_eq!(said[0]["event"]["markdown"], "ANTHROPIC_API_KEY is unset");
    call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    pdt_api::shutdown_conversations(&st).await;
}
