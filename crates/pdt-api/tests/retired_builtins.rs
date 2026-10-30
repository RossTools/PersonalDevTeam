//! PDT-295: PDT never special-cases its own repository. The built-ins it shipped for its own development are gone: a
//! host that has one keeps it as its own workflow, as it was, and no host gets one, whatever its projects are.

mod common;
mod retired;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};

struct Env {
    app: axum::Router,
    state: AppState,
    _tmp: tempfile::TempDir,
}

/// A host with a project on PDT's own repository and these retired built-ins stored, each enabled or not, as an
/// upgraded host has them.
async fn setup(stored: &[(&str, bool)]) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    let origin = format!("https://github.com/{}.git", pdt_core::brand::RELEASE_REPO);
    git(&repo, &["remote", "add", "origin", &origin]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    for (id, enabled) in stored {
        let wf = retired::install(&store, id, *enabled).await;
        store.set_workflow_state(&wf, "seen", &json!(["o/r#1"]), false, 2).await.unwrap();
    }
    let state = AppState::new(store, tmp.path().join("data"));
    let app = router(state.clone(), None);
    let (s, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "PDT", "key": "PX", "repoPath": repo.to_string_lossy()})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, _tmp: tmp }
}

async fn workflows(env: &Env) -> Vec<Value> {
    let (_, all) = call(&env.app, "GET", "/api/workflows", None).await;
    all.as_array().unwrap().clone()
}

async fn versions(env: &Env, wf: &str) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/workflows/{wf}/versions"), None).await;
    v.as_array().unwrap().clone()
}

#[tokio::test]
async fn an_upgraded_host_keeps_them_as_its_own_workflows() {
    let stored: Vec<(&str, bool)> = retired::ALL.iter().map(|id| (*id, *id != retired::PR_REVIEW)).collect();
    let env = setup(&stored).await;
    pdt_api::run_workflows(&env.state).await;
    for (id, enabled) in &stored {
        let wf = retired::id(id);
        let (s, w) = call(&env.app, "GET", &format!("/api/workflows/{wf}"), None).await;
        assert_eq!(s, StatusCode::OK, "{id}: {w}");
        assert!(w["builtin"].is_null(), "no longer a built-in: {w}");
        assert_eq!(w["enabled"], *enabled, "{id}");
        assert_eq!(w["scope"], "host", "{id}");
        assert_eq!(w["name"].as_str(), Some(retired::doc(id).name.as_str()), "{id}");
        let errors: Vec<&Value> = w["issues"].as_array().unwrap().iter().filter(|i| i["severity"] == "error").collect();
        assert!(errors.is_empty(), "{id}: {errors:?}");
        let (_, state) = call(&env.app, "GET", &format!("/api/workflows/{wf}/state"), None).await;
        assert_eq!(state["seen"], json!(["o/r#1"]), "{id}: its state stays");
        // Its document is the one it had, but for nodes on an older version of their kind.
        let doc: pdt_core::workflow::doc::WorkflowDoc = serde_json::from_value(w["doc"].clone()).unwrap();
        if *id == retired::NIGHTLY_MISSING {
            assert_eq!(versions(&env, &wf).await.len(), 2, "{id}");
            let watch = doc.node("watch").unwrap();
            assert_eq!(watch.version, 2);
            // PDT's tag scheme and due time, written out in the generic watch's config.
            let c = &watch.config;
            assert_eq!(c["repo"], pdt_core::brand::RELEASE_REPO);
            assert_eq!((&c["workflow"], &c["branch"]), (&json!("release.yml"), &json!("main")));
            assert_eq!((&c["tagPattern"], &c["dueBy"]), (&json!("*-nightly.{date}*"), &json!("10:00")));
            assert_eq!(c["onlyIfChanged"], true);
            assert!(c.get("dueHour").is_none(), "{c}");
        } else {
            assert_eq!(doc, retired::doc(id), "{id}");
            assert_eq!(versions(&env, &wf).await.len(), 1, "{id}");
        }
    }
    // Nothing is installed in their place, and the next start changes nothing.
    let names = |all: &[Value]| all.iter().map(|w| w["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let before = names(&workflows(&env).await);
    let again = AppState::new(env.state.store.clone(), env._tmp.path().join("data"));
    pdt_api::run_workflows(&again).await;
    assert_eq!(names(&workflows(&env).await), before);
    assert_eq!(versions(&env, &retired::id(retired::NIGHTLY_MISSING)).await.len(), 2);
    assert!(workflows(&env).await.iter().all(|w| !retired::ALL.contains(&w["builtin"].as_str().unwrap_or(""))));
    // The user's own now: it can be deleted.
    let (s, v) = call(&env.app, "DELETE", &format!("/api/workflows/{}", retired::id(retired::NIGHTLY)), None).await;
    assert!(s.is_success(), "{s}: {v}");
}

#[tokio::test]
async fn a_host_with_pdts_repository_gets_none_of_them() {
    let env = setup(&[]).await;
    pdt_api::run_workflows(&env.state).await;
    let all = workflows(&env).await;
    assert!(!all.is_empty(), "the built-ins are installed");
    for w in &all {
        let name = w["name"].as_str().unwrap();
        assert!(!name.starts_with("PDT "), "{name}");
        assert!(!retired::ALL.contains(&w["builtin"].as_str().unwrap_or("")), "{w}");
    }
    let (_, templates) = call(&env.app, "GET", "/api/workflow-templates", None).await;
    for id in retired::ALL {
        assert!(!templates.to_string().contains(&format!("builtin:{id}\"")), "{id} offered");
        let (s, _) = call(&env.app, "GET", &format!("/api/workflow-templates/builtin:{id}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{id}");
    }
}

/// One the user edited is left as they saved it: its version 1 watch runs as it did (the engine migrates it as it
/// reads it), and PDT writes no version of its own.
#[tokio::test]
async fn an_edited_one_keeps_the_users_document() {
    let env = setup(&[(retired::NIGHTLY_MISSING, true)]).await;
    let wf = retired::id(retired::NIGHTLY_MISSING);
    let mut mine = retired::doc(retired::NIGHTLY_MISSING);
    mine.name = "My missing nightly".into();
    env.state.store.save_workflow_version(&wf, &mine, pdt_core::workflow::api::Author::User, "mine", 3).await.unwrap();
    pdt_api::run_workflows(&env.state).await;
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf}"), None).await;
    assert!(w["builtin"].is_null(), "{w}");
    let doc: pdt_core::workflow::doc::WorkflowDoc = serde_json::from_value(w["doc"].clone()).unwrap();
    assert_eq!(doc, mine);
    assert_eq!(versions(&env, &wf).await.len(), 2);
    let errors: Vec<&Value> = w["issues"].as_array().unwrap().iter().filter(|i| i["severity"] == "error").collect();
    assert!(errors.is_empty(), "{errors:?}");
}
