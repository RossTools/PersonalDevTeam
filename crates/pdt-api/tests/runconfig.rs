use crate::common;

use axum::http::StatusCode;
use common::{call, git};
use serde_json::{Value, json};

async fn setup(tmp: &std::path::Path) -> (axum::Router, String, std::path::PathBuf) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(
        repo.join(".pdt").join("project.toml"),
        "[[run]]\nname = \"web\"\ncommand = \"serve $PORT\"\nweb = true\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let app = pdt_api::router(common::app_state(store, tmp), None);
    let (_, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    (app, p["id"].as_str().unwrap().to_string(), repo)
}

#[test]
fn the_config_file_path_is_the_documented_one() {
    assert_eq!(pdt_core::brand::PROJECT_CONFIG_FILE, ".pdt/project.toml");
}

#[tokio::test]
async fn effective_config_merges_file_and_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, _) = setup(tmp.path()).await;
    let uri = format!("/api/projects/{pid}/run-config");
    let (s, v) = call(&app, "GET", &uri, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["file"].as_str().unwrap().contains("serve $PORT"));
    assert!(v["filePath"].as_str().unwrap().ends_with("project.toml"));
    assert_eq!(v["effective"]["runs"][0]["web"], true);
    assert_eq!(v["error"], Value::Null);

    let overrides = "[[run]]\nname = \"web\"\ncommand = \"serve --fast $PORT\"\n";
    let (s, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{pid}/settings"),
        Some(json!({"agentCredentials": null, "runOverrides": overrides})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(&app, "GET", &uri, None).await;
    assert_eq!(v["effective"]["runs"][0]["command"], "serve --fast $PORT");
    assert_eq!(v["overrides"], overrides);
}

#[tokio::test]
async fn invalid_overrides_are_rejected_and_bad_files_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, repo) = setup(tmp.path()).await;
    let (s, v) = call(
        &app,
        "PUT",
        &format!("/api/projects/{pid}/settings"),
        Some(json!({"agentCredentials": null, "runOverrides": "[[run]\n"})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_run_config")), "{v}");
    std::fs::write(repo.join(".pdt").join("project.toml"), "[[run]]\nname = \"x\"\n").unwrap();
    let (s, v) = call(&app, "GET", &format!("/api/projects/{pid}/run-config"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["effective"], Value::Null);
    assert!(v["error"].as_str().unwrap().contains("no command"), "{v}");
}

#[tokio::test]
async fn a_project_without_a_file_has_an_empty_config() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, repo) = setup(tmp.path()).await;
    std::fs::remove_file(repo.join(".pdt").join("project.toml")).unwrap();
    let (_, v) = call(&app, "GET", &format!("/api/projects/{pid}/run-config"), None).await;
    assert_eq!(
        (v["file"].clone(), v["effective"].clone()),
        (
            Value::Null,
            json!({"runs": [], "tests": [], "areas": [], "forge": null, "ci": null, "ciProviders": null, "push": null, "buildOutput": null, "regenerate": [], "ignored": []})
        )
    );
}

#[tokio::test]
async fn unknown_sections_in_the_file_are_skipped_but_refused_in_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, repo) = setup(tmp.path()).await;
    std::fs::write(
        repo.join(".pdt").join("project.toml"),
        "[[run]]\nname = \"web\"\ncommand = \"serve $PORT\"\n[[pipeline]]\nname = \"later\"\n",
    )
    .unwrap();
    let (s, v) = call(&app, "GET", &format!("/api/projects/{pid}/run-config"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["error"], Value::Null, "{v}");
    assert_eq!(v["effective"]["runs"][0]["name"], "web");
    assert_eq!(v["effective"]["ignored"], json!(["pipeline"]));

    let (s, v) = call(
        &app,
        "PUT",
        &format!("/api/projects/{pid}/settings"),
        Some(json!({"agentCredentials": null, "runOverrides": "[[runs]]\nname = \"web\"\n"})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_run_config")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("`runs`"), "{v}");
}
