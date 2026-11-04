use crate::common;

use axum::http::StatusCode;
use common::call;
use serde_json::{Value, json};

async fn setup(tmp: &std::path::Path) -> (axum::Router, String) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    crate::common::init_repo(&repo);
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let app = pdt_api::router(common::app_state(store, tmp), None);
    let (_, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    (app, p["id"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn project_credentials_override_the_host() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid) = setup(tmp.path()).await;
    let uri = format!("/api/projects/{pid}/settings");
    let (s, v) = call(&app, "GET", &uri, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["agentCredentials"], Value::Null);
    assert_eq!(v["effectiveCredentials"], "subscription");

    let body = |c: Value| Some(json!({"agentCredentials": c, "runOverrides": null}));
    let (s, v) = call(&app, "PUT", &uri, body(json!("pass_through"))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["effectiveCredentials"], "pass_through");

    call(&app, "PUT", "/api/host/settings", Some(json!({"agentCredentials": "pass_through"}))).await;
    let (_, v) = call(&app, "PUT", &uri, body(json!("subscription"))).await;
    assert_eq!(v["effectiveCredentials"], "subscription", "a project can tighten the host policy too");
    let (_, v) = call(&app, "PUT", &uri, body(Value::Null)).await;
    assert_eq!(v["effectiveCredentials"], "pass_through", "null falls back to the host");

    let (s, _) = call(&app, "GET", "/api/projects/nope/settings", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn agent_instructions_are_kept_unless_sent() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid) = setup(tmp.path()).await;
    let uri = format!("/api/projects/{pid}/settings");
    let (_, v) = call(&app, "GET", &uri, None).await;
    assert_eq!(v["agentInstructions"], Value::Null);

    let put = |b: Value| call(&app, "PUT", &uri, Some(b));
    let (s, v) =
        put(json!({"agentCredentials": null, "runOverrides": null, "agentInstructions": "  Use make.\n"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["agentInstructions"], "Use make.", "trimmed");
    let (_, v) = put(json!({"agentCredentials": "pass_through", "runOverrides": null})).await;
    assert_eq!(v["agentInstructions"], "Use make.", "a client that leaves it out keeps it");
    let (_, v) = put(json!({"agentCredentials": null, "runOverrides": null, "agentInstructions": ""})).await;
    assert_eq!(v["agentInstructions"], Value::Null, "an empty text clears it");
    let long = "x".repeat(pdt_core::agent_context::MAX_PROJECT_INSTRUCTIONS + 1);
    let (s, v) = put(json!({"agentCredentials": null, "runOverrides": null, "agentInstructions": long})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

/// PDT-173: Settings → Plugins lists the integrations PDT knows; one this build lacks can't be enabled, and a
/// connection of its kind is refused, naming it.
#[tokio::test]
async fn integrations_this_build_lacks_are_listed_and_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup(tmp.path()).await;
    let (s, v) = call(&app, "GET", "/api/integrations", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let list = v.as_array().unwrap();
    // PDT's own plugins (PDT-228), always on, then every known one, not built.
    let (own, known): (Vec<&Value>, Vec<&Value>) = list.iter().partition(|i| i["core"] == true);
    let own_ids: Vec<&str> = own.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(own_ids, ["webhook", "github", "http", "mcp"], "{v}");
    assert!(own.iter().all(|i| i["enabled"] == true && i["status"] == "on"), "{v}");
    assert_eq!(known.len(), pdt_core::integration::KNOWN.len(), "{v}");
    assert!(known.iter().all(|i| i["built"] == false && i["enabled"] == false && i["status"] == "not_built"), "{v}");
    let (s, v) = call(&app, "PUT", "/api/integrations", Some(json!({"enabled": ["jira"]}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("built without the Jira integration"), "{v}");
    // "Test again" runs an enabled plugin's checks; one that is off or unknown has none to run.
    let (s, v) = call(&app, "POST", "/api/integrations/webhook/check", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = call(&app, "POST", "/api/integrations/jira/check", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // PDT's own are always on: naming them changes nothing.
    let (s, v) = call(&app, "PUT", "/api/integrations", Some(json!({"enabled": ["github"]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(&app, "GET", "/api/integrations/project-settings", None).await;
    assert_eq!((s, v), (StatusCode::OK, json!([])));
    let (s, v) =
        call(&app, "POST", "/api/connections", Some(json!({"name": "work", "kind": "jira", "url": "https://x"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["message"].as_str().unwrap().contains("Jira integration, which this PDT was built without"), "{v}");
    let (_, kinds) = call(&app, "GET", "/api/connections/kinds", None).await;
    let ids: Vec<&str> = kinds.as_array().unwrap().iter().map(|k| k["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["webhook", "http", "mcp"]);
}

/// PDT-251: GitHub, one of PDT's own plugins, can be turned off while no project's forge is GitHub; off, its steps and
/// triggers, built-in workflows and template are gone until it is on again, and the choice is kept.
#[tokio::test]
async fn github_turns_off_only_while_no_project_uses_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup(tmp.path()).await;
    let github = |v: &Value| v.as_array().unwrap().iter().find(|i| i["id"] == "github").unwrap().clone();
    let kinds = |app: &axum::Router| {
        let app = app.clone();
        async move {
            let (_, v) = call(&app, "GET", "/api/workflows/catalog", None).await;
            v.as_array().unwrap().iter().filter_map(|k| k["id"].as_str().map(String::from)).collect::<Vec<_>>()
        }
    };
    let (_, v) = call(&app, "GET", "/api/integrations", None).await;
    let g = github(&v);
    assert_eq!((&g["core"], &g["canTurnOff"], &g["enabled"]), (&json!(true), &json!(true), &json!(true)), "{g}");
    assert_eq!(g["forgeProjects"], json!([]), "the project has no origin");
    assert!(kinds(&app).await.iter().any(|k| k == "github.issue.opened"));

    // Leaving `off` out keeps PDT's own as they are; naming one that can't be turned off is refused.
    let (s, v) = call(&app, "PUT", "/api/integrations", Some(json!({"enabled": []}))).await;
    assert_eq!((s, github(&v)["enabled"].clone()), (StatusCode::OK, json!(true)));
    let (s, v) = call(&app, "PUT", "/api/integrations", Some(json!({"enabled": [], "off": ["mcp"]}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")), "{v}");

    let (s, v) = call(&app, "PUT", "/api/integrations", Some(json!({"enabled": [], "off": ["github"]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((github(&v)["enabled"].clone(), github(&v)["status"].clone()), (json!(false), json!("off")));
    let left = kinds(&app).await;
    assert!(!left.iter().any(|k| k.starts_with("github.")), "{left:?}");
    let (_, w) = call(&app, "GET", "/api/workflows", None).await;
    let builtins: Vec<&str> = w.as_array().unwrap().iter().filter_map(|w| w["builtin"].as_str()).collect();
    assert!(!builtins.contains(&"github-issue-done") && !builtins.contains(&"cut-release"), "{builtins:?}");
    let (s, v) = call(&app, "POST", "/api/workflow-templates/builtin:issue-triage/instantiate", Some(json!({}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["message"].as_str().unwrap().contains("GitHub plugin"), "{v}");

    // On again; then a project whose origin is on GitHub keeps it on until the project names another forge.
    let (_, v) = call(&app, "PUT", "/api/integrations", Some(json!({"enabled": [], "off": []}))).await;
    assert_eq!(github(&v)["enabled"], json!(true));
    let repo = tmp.path().join("repo");
    common::git(&repo, &["remote", "add", "origin", "https://github.com/o/r.git"]);
    let (_, v) = call(&app, "GET", "/api/integrations", None).await;
    assert_eq!(github(&v)["forgeProjects"].as_array().unwrap().len(), 1, "{v}");
    let (s, v) = call(&app, "PUT", "/api/integrations", Some(json!({"enabled": [], "off": ["github"]}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("in_use")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("the forge of P"), "{v}");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    std::fs::write(repo.join(".pdt/project.toml"), "forge = \"local\"\n").unwrap();
    let (s, v) = call(&app, "PUT", "/api/integrations", Some(json!({"enabled": [], "off": ["github"]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}
