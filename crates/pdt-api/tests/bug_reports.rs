//! Report a bug (PDT-153): issues on the release repository as the GitHub login or with the update token (PDT-559),
//! the browser's bug form with neither, local Draft tickets on the instance PDT is developed on (no GitHub request),
//! errors, the nonce, and redacted diagnostics. GitHub is a local fake.

use crate::common;

use axum::http::StatusCode;
use common::issues::{FakeIssues, READ_ONLY, TOKEN};
use common::{call, git};
use serde_json::{Value, json};
use std::sync::Arc;

struct Env {
    app: axum::Router,
    state: pdt_api::AppState,
    gh: FakeIssues,
    tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let (base, gh) = FakeIssues::serve().await;
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.update = Arc::new(pdt_api::UpdateService::new(&base, "o/r", false, "0.1.0-local"));
    state.home_dir = Some(tmp.path().join("home"));
    Env { app: pdt_api::router(state.clone(), None), state, gh, tmp }
}

fn store_token(env: &Env, token: &str) {
    env.state.secrets.set(&env.state.update_token_account, token).unwrap();
}

/// A project whose repository's origin is `origin`.
async fn add_project(env: &Env, name: &str, key: &str, origin: &str) -> Value {
    let repo = env.tmp.path().join(name);
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["remote", "add", "origin", origin]);
    let body = json!({"name": name, "key": key, "repoPath": repo.to_string_lossy()});
    let (s, p) = call(&env.app, "POST", "/api/projects", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    p
}

async fn report(env: &Env, body: Value) -> (StatusCode, Value) {
    call(&env.app, "POST", "/api/bug-reports", Some(body)).await
}

#[tokio::test]
async fn another_instance_files_an_issue_with_labels_and_redacted_diagnostics() {
    let env = setup().await;
    add_project(&env, "Other", "OT", "https://github.com/someone/else.git").await;
    store_token(&env, TOKEN);
    // The log mentions the token and the home folder: neither may leave the machine.
    let home = env.tmp.path().join("home").to_string_lossy().into_owned();
    let logs = env.state.data_dir.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("pdtd.2026-10-24.log"), "old day\n").unwrap();
    std::fs::write(
        logs.join("pdtd.2026-10-25.log"),
        format!("INFO started\nWARN read {home}/work/repo/x.rs with token {TOKEN}\nINFO last line\n"),
    )
    .unwrap();

    let (s, p) = call(&env.app, "GET", "/api/bug-reports/preview", None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!((p["setting"].as_str(), p["route"].as_str()), (Some("auto"), Some("github")));
    assert_eq!((p["repo"].as_str(), p["tokenPresent"].as_bool()), (Some("o/r"), Some(true)));
    assert!(p["project"].is_null());
    let preview = p["diagnostics"].as_str().unwrap();
    let version = format!("PDT version: {}", pdt_core::BUILD_VERSION);
    assert!(preview.contains(&version) && preview.contains("INFO last line"), "{preview}");
    assert!(!preview.contains("old day"), "only the newest log");

    let (s, filed) = report(
        &env,
        json!({"title": "Board freezes", "description": "It stops.", "expected": "It moves.", "steps": "Drag a card.",
            "includeDiagnostics": true, "nonce": "n1"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{filed}");
    assert_eq!(filed, json!({"kind": "issue", "number": 101, "url": "https://github.com/o/r/issues/101"}));
    let issue = env.gh.created.lock().unwrap()[0].clone();
    assert_eq!(issue["title"], "Board freezes");
    assert_eq!(issue["labels"], json!(["bug", "from-pdt"]));
    let body = issue["body"].as_str().unwrap();
    assert!(body.starts_with("### What happened\n\nIt stops.\n\n### What you expected\n\nIt moves.\n\n"), "{body}");
    assert!(
        body.contains("### How to reproduce\n\nDrag a card.") && body.contains("<details><summary>Diagnostics"),
        "{body}"
    );
    assert!(body.contains(&format!("### OS\n\n{} (", std::env::consts::OS)), "{body}");
    assert!(body.contains("~/work/repo/x.rs") && body.contains("[redacted]"), "{body}");
    assert!(!body.contains(TOKEN) && !body.contains(&home), "{body}");

    // The same nonce again (a retry) files nothing new; without diagnostics the body has none.
    let (_, again) = report(&env, json!({"title": "Board freezes", "nonce": "n1"})).await;
    assert_eq!(again, filed);
    assert_eq!(env.gh.request_count(), 1);
    let (_, second) = report(&env, json!({"title": "Other bug", "nonce": "n2"})).await;
    assert_eq!(second["number"], 102);
    let plain = env.gh.created.lock().unwrap()[1]["body"].as_str().unwrap().to_string();
    assert!(!plain.contains("Diagnostics") && plain.contains("### OS\n\n_Not shared_"), "{plain}");
}

#[tokio::test]
async fn the_instance_with_the_release_project_files_a_draft_ticket_and_never_calls_github() {
    let env = setup().await;
    store_token(&env, TOKEN);
    add_project(&env, "Other", "OT", "https://github.com/someone/else.git").await;
    let p = add_project(&env, "PDT", "PD", "git@github.com:o/r.git").await;
    let (_, preview) = call(&env.app, "GET", "/api/bug-reports/preview", None).await;
    assert_eq!(preview["route"], "local");
    assert_eq!(preview["project"]["key"], "PD");

    let (s, filed) =
        report(&env, json!({"title": "Crash on start", "description": "Boom.", "includeDiagnostics": true})).await;
    assert_eq!(s, StatusCode::OK, "{filed}");
    assert_eq!((filed["kind"].as_str(), filed["key"].as_str()), (Some("ticket"), Some("PD-1")));
    assert_eq!(filed["projectId"], p["id"]);
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{}", filed["id"].as_str().unwrap()), None).await;
    assert_eq!((t["stage"].as_str(), t["labels"].clone()), (Some("draft"), json!(["bug"])));
    let d = t["description"].as_str().unwrap();
    assert!(d.starts_with("### What happened\n\nBoom.") && d.contains("PDT version:"), "{d}");
    assert_eq!(env.gh.request_count(), 0, "no GitHub request on the local route");

    // Choosing GitHub sends it there instead; off refuses.
    let (s, _) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"bugReports": "github"}))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, filed) = report(&env, json!({"title": "To GitHub"})).await;
    assert_eq!(filed["kind"], "issue");
    let (_, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"bugReports": "off"}))).await;
    assert_eq!(v["settings"]["bugReports"], "off");
    let (s, e) = report(&env, json!({"title": "Nope"})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("bug_reports_off")));
    assert_eq!(env.gh.request_count(), 1);
}

#[tokio::test]
async fn local_without_the_release_project_says_so() {
    let env = setup().await;
    call(&env.app, "PUT", "/api/host/settings", Some(json!({"bugReports": "local"}))).await;
    let (s, e) = report(&env, json!({"title": "x"})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("no_bug_project")));
    assert_eq!(env.gh.request_count(), 0);
}

#[tokio::test]
async fn token_problems_are_plain_errors() {
    let env = setup().await;
    let (s, e) = report(&env, json!({"title": "x"})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("no_github_login")));
    let m = e["message"].as_str().unwrap();
    assert!(m.contains("Connect GitHub") && m.contains("https://github.com/o/r/issues/new?template=bug.yml"), "{e}");
    assert_eq!(env.gh.request_count(), 0);

    store_token(&env, READ_ONLY);
    let (s, e) = report(&env, json!({"title": "x"})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("token_no_issue_access")), "{e}");
    assert!(e["message"].as_str().unwrap().contains("Connect GitHub"), "{e}");

    store_token(&env, "github_pat_revoked");
    let (s, e) = report(&env, json!({"title": "x"})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("token_rejected")), "{e}");
    assert!(!e.to_string().contains("github_pat_"), "{e}");

    let (s, e) = report(&env, json!({"title": "  "})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
}

#[tokio::test]
async fn the_github_login_files_the_issue_before_the_token_and_neither_means_the_browser() {
    let mut env = setup().await;
    let (_, p) = call(&env.app, "GET", "/api/bug-reports/preview", None).await;
    assert_eq!((p["via"].as_str(), p["githubLogin"].as_str()), (Some("browser"), None));
    assert_eq!(p["version"].as_str(), Some(pdt_core::BUILD_VERSION));
    assert!(p["os"].as_str().unwrap().starts_with(std::env::consts::OS), "{p}");
    store_token(&env, TOKEN);
    let (_, p) = call(&env.app, "GET", "/api/bug-reports/preview", None).await;
    assert_eq!(p["via"], "token");

    // Signed in to GitHub: the login files it, not the token.
    let forge = Arc::new(pdt_forge::testing::FakeForge::default());
    env.state.forges = pdt_forge::ForgeRegistry::new(vec![Arc::new(pdt_forge::testing::fake_github(&forge))]);
    env.app = pdt_api::router(env.state.clone(), None);
    let (_, p) = call(&env.app, "GET", "/api/bug-reports/preview", None).await;
    assert_eq!(p["via"], "login");
    let (s, filed) = report(&env, json!({"title": "Board freezes", "description": "It stops."})).await;
    assert_eq!(s, StatusCode::OK, "{filed}");
    assert_eq!(filed, json!({"kind": "issue", "number": 500, "url": "https://github.com/o/r/issues/500"}));
    let (repo, issue) = forge.created_issues.lock().unwrap()[0].clone();
    assert_eq!((repo.as_str(), issue.title.as_str()), ("o/r", "Board freezes"));
    assert_eq!(issue.labels, ["bug", "from-pdt"]);
    assert!(issue.body.starts_with("### What happened\n\nIt stops."), "{}", issue.body);
    assert_eq!(env.gh.request_count(), 0, "the token is not used");
}

#[tokio::test]
async fn an_unreachable_github_asks_to_try_again() {
    let tmp = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.update =
        Arc::new(pdt_api::UpdateService::new(&format!("http://127.0.0.1:{port}"), "o/r", false, "0.1.0-local"));
    state.secrets.set(&state.update_token_account, TOKEN).unwrap();
    let app = pdt_api::router(state, None);
    let (s, e) = call(&app, "POST", "/api/bug-reports", Some(json!({"title": "x", "nonce": "n"}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("github_unreachable")), "{e}");
    assert!(e["message"].as_str().unwrap().contains("Try again"));
}
