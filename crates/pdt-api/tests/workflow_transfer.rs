//! Export and import (PDT-155): a workflow copied from one PDT and pasted into another, through the text alone, with
//! its connection, persona and project remapped; the check report; name clashes; several workflows in one paste; a
//! bare document; and the errors for text that isn't an export.

use crate::common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};

struct Instance {
    app: axum::Router,
    _state: AppState,
    project: String,
    _tmp: tempfile::TempDir,
}

/// A PDT with one project (`key`) and an `http` connection called `alerts`.
async fn instance(key: &str, alerts: &str) -> Instance {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.path().join("data"));
    let app = router(state.clone(), None);
    let body = json!({"name": format!("Project {key}"), "key": key, "repoPath": repo.to_string_lossy()});
    let (s, p) = call(&app, "POST", "/api/projects", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let (s, c) =
        call(&app, "POST", "/api/connections", Some(json!({"name": alerts, "kind": "http", "url": "https://x"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    Instance { app, _state: state, project: p["id"].as_str().unwrap().to_string(), _tmp: tmp }
}

fn alerts_doc(project: &str, persona: &str) -> Value {
    json!({
        "name": "Review alerts",
        "description": "Reviews a moved ticket and posts the outcome.",
        "nodes": [
            {"id": "moved", "kind": "ticket.stage_changed", "version": 1, "config": {"projects": [project]}},
            {"id": "review", "kind": "agent", "version": 1, "config": {"persona": persona, "prompt": "Review it.",
                "outcomes": [{"id": "approved"}, {"id": "changes_requested"}]}},
            {"id": "post", "kind": "http.request", "version": 1, "config": {"connection": "alerts",
                "method": "POST", "url": "/alerts", "body": {"outcome": "{{steps.review.outcome}}"}}},
            {"id": "done", "kind": "end", "version": 1},
        ],
        "edges": [
            {"from": "moved", "to": "review"},
            {"from": "review", "port": "approved", "to": "post"},
            {"from": "review", "port": "changes_requested", "to": "done"},
            {"from": "post", "to": "done"},
        ],
    })
}

async fn create(i: &Instance, doc: Value, extra: Value) -> String {
    let mut body = json!({"doc": doc, "enabled": true});
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let (s, w) = call(&i.app, "POST", "/api/workflows", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

fn text(export: &Value) -> String {
    serde_json::to_string_pretty(export).unwrap()
}

async fn import(i: &Instance, body: Value) -> (StatusCode, Value) {
    call(&i.app, "POST", "/api/workflows/import", Some(body)).await
}

#[tokio::test]
async fn a_pasted_export_imports_disabled_with_its_names_remapped() {
    let a = instance("SRC", "alerts").await;
    let (s, _) = call(
        &a.app,
        "POST",
        "/api/personas",
        Some(json!({"name": "Critic", "agent": "claude", "instructions": "Be picky."})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let id = create(&a, alerts_doc(&a.project, "Critic"), json!({})).await;

    let (s, export) = call(&a.app, "GET", &format!("/api/workflows/{id}/export"), None).await;
    assert_eq!(s, StatusCode::OK, "{export}");
    assert_eq!((export["format"].as_str(), export["formatVersion"].as_u64()), (Some("pdt-workflow"), Some(1)));
    let doc = &export["workflows"][0]["doc"];
    assert_eq!(doc["nodes"][0]["config"]["projects"], json!([{"$project": "SRC"}]), "{doc}");
    assert!(!text(&export).contains(&a.project), "no project id travels");
    assert_eq!(export["requires"]["connections"], json!([{"name": "alerts", "kind": "any", "tools": []}]));
    assert_eq!(export["requires"]["personas"], json!(["Critic"]));
    assert_eq!(export["requires"]["projects"], json!([{"key": "SRC", "name": "Project SRC"}]));

    // Another PDT: a differently named connection, no Critic, another project.
    let b = instance("DST", "team-alerts").await;
    let paste = text(&export);
    let (s, c) = import(&b, json!({"text": paste, "check": true})).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["connections"][0]["status"], "missing", "{c}");
    assert_eq!(c["personas"][0]["status"], "missing");
    assert_eq!(c["projects"][0]["status"], "missing");
    assert_eq!(c["workflows"][0]["name"], "Review alerts");
    assert_eq!(c["workflows"][0]["existing"], json!([]));

    // Nothing is imported until every missing name is mapped or left unmapped.
    let (s, e) = import(&b, json!({"text": paste})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("needs_mapping")), "{e}");
    let msg = e["message"].as_str().unwrap();
    assert!(
        msg.contains("connection alerts") && msg.contains("persona Critic") && msg.contains("project SRC"),
        "{msg}"
    );
    let (_, list) = call(&b.app, "GET", "/api/workflows", None).await;
    assert!(!list.as_array().unwrap().iter().any(|w| w["name"] == "Review alerts"));

    let mapping = json!({"connections": {"alerts": "team-alerts"}, "personas": {"Critic": "Reviewer"},
        "projects": {"SRC": b.project}});
    let (s, r) = import(&b, json!({"text": paste, "mapping": mapping})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["workflows"][0]["enabled"], false, "imported disabled");
    let new_id = r["workflows"][0]["id"].as_str().unwrap();
    let (_, w) = call(&b.app, "GET", &format!("/api/workflows/{new_id}"), None).await;
    assert_eq!(w["scope"], "host");
    assert_eq!(w["doc"]["nodes"][0]["config"]["projects"], json!([b.project]));
    assert_eq!(w["doc"]["nodes"][1]["config"]["persona"], "Reviewer");
    assert_eq!(w["doc"]["nodes"][2]["config"]["connection"], "team-alerts");
    let issues = w["issues"].as_array().unwrap();
    assert!(!issues.iter().any(|i| i["severity"] == "error"), "{issues:?}");
    assert!(!issues.iter().any(|i| i["code"].as_str().unwrap().starts_with("missing_")), "{issues:?}");
    let (s, v) = call(&b.app, "POST", "/api/workflows/validate", Some(json!({"doc": w["doc"]}))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!v.as_array().unwrap().iter().any(|i| i["severity"] == "error"), "{v}");

    // Importing again offers a copy or a new version, and never overwrites by itself.
    let (_, c) = import(&b, json!({"text": paste, "check": true})).await;
    assert_eq!(c["workflows"][0]["existing"][0]["id"], new_id);
    let (s, e) = import(&b, json!({"text": paste, "mapping": mapping})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("workflow_exists")), "{e}");
    let (s, r) = import(&b, json!({"text": paste, "mapping": mapping, "targets": [{"copy": true}]})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["workflows"][0]["name"], "Review alerts (imported)");
    let (s, r) = import(&b, json!({"text": paste, "mapping": mapping, "targets": [{"versionOf": new_id}]})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!((r["workflows"][0]["id"].as_str(), r["workflows"][0]["version"].as_u64()), (Some(new_id), Some(2)));
}

#[tokio::test]
async fn unmapped_names_import_with_warnings() {
    let a = instance("SRC", "alerts").await;
    let id = create(&a, alerts_doc(&a.project, "Reviewer"), json!({"scope": "project", "projectId": a.project})).await;
    let (_, export) = call(&a.app, "GET", &format!("/api/workflows/{id}/export"), None).await;
    assert_eq!(export["workflows"][0]["project"], "SRC");
    let b = instance("DST", "team-alerts").await;
    // The Reviewer is built in on both: found, so it needs no mapping.
    let (_, c) = import(&b, json!({"text": text(&export), "check": true})).await;
    assert_eq!(c["personas"][0]["status"], "found", "{c}");
    let mapping = json!({"connections": {"alerts": null}, "projects": {"SRC": "DST"}});
    let (s, r) = import(&b, json!({"text": text(&export), "mapping": mapping})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let warnings: Vec<&str> = r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(warnings.iter().any(|w| w.contains("alerts is left unmapped")), "{warnings:?}");
    // It applies to the project its old one was mapped to.
    let w = &r["workflows"][0];
    assert_eq!((w["scope"].as_str(), w["projectId"].as_str()), (Some("project"), Some(b.project.as_str())));
    // And the workflow's page says the connection is missing.
    let (_, d) = call(&b.app, "GET", &format!("/api/workflows/{}", w["id"].as_str().unwrap()), None).await;
    let codes: Vec<&str> = d["issues"].as_array().unwrap().iter().map(|i| i["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"missing_connection"), "{codes:?}");
}

#[tokio::test]
async fn several_workflows_travel_in_one_paste() {
    let a = instance("SRC", "alerts").await;
    let mut ids = vec![];
    for n in ["One", "Two", "Three"] {
        let doc = json!({"name": n, "nodes": [{"id": "go", "kind": "manual", "version": 1},
            {"id": "end", "kind": "end", "version": 1}], "edges": [{"from": "go", "to": "end"}]});
        ids.push(create(&a, doc, json!({"scope": "personal"})).await);
    }
    let (s, export) = call(&a.app, "POST", "/api/workflows/export", Some(json!({"ids": ids}))).await;
    assert_eq!(s, StatusCode::OK, "{export}");
    assert_eq!(export["workflows"].as_array().unwrap().len(), 3);
    let b = instance("DST", "alerts").await;
    let (s, r) = import(&b, json!({"text": text(&export)})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let names: Vec<&str> = r["workflows"].as_array().unwrap().iter().map(|w| w["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["One", "Two", "Three"]);
    assert!(r["workflows"].as_array().unwrap().iter().all(|w| w["scope"] == "personal" && w["enabled"] == false));
}

#[tokio::test]
async fn a_bare_document_imports_and_bad_text_is_refused() {
    let b = instance("DST", "alerts").await;
    let doc = json!({"name": "Bare", "nodes": [{"id": "go", "kind": "manual", "version": 1},
        {"id": "post", "kind": "http.request", "version": 1, "config": {"connection": "alerts", "url": "/hi"}}],
        "edges": [{"from": "go", "to": "post"}]});
    let (s, c) = import(&b, json!({"text": doc.to_string(), "check": true})).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["connections"][0]["status"], "found", "a bare doc's names are matched too: {c}");
    let (s, r) = import(&b, json!({"text": doc.to_string()})).await;
    assert_eq!((s, r["workflows"][0]["name"].as_str()), (StatusCode::OK, Some("Bare")), "{r}");

    let (s, e) = import(&b, json!({"text": "{not json", "check": true})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_export")));
    assert!(e["message"].as_str().unwrap().contains("not valid JSON"), "{e}");
    let newer = json!({"format": "pdt-workflow", "formatVersion": 9, "workflows": []}).to_string();
    let (s, e) = import(&b, json!({"text": newer, "check": true})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(e["message"].as_str().unwrap().contains("format version 9"), "{e}");
}
