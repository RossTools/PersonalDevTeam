//! PDT-100: built-in personas follow newer templates field by field. A field the user changed stays theirs, every
//! other field follows; `GET /api/personas/{id}/template` shows the template for "Load PDT's template".

use crate::common;

use axum::http::StatusCode;
use common::call;
use pdt_api::router;
use pdt_core::persona::{PersonaAuthor, PersonaSetup};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;

/// One daemon run on the data in `dir`: built-ins are installed or updated on its first persona request.
async fn run(dir: &Path) -> (axum::Router, Store) {
    let store = Store::open(&dir.join("pdt.db")).await.unwrap();
    let state = common::app_state(store.clone(), dir);
    (router(state, None), store)
}

async fn persona(app: &axum::Router, builtin: &str) -> Value {
    let (_, all) = call(app, "GET", "/api/personas", None).await;
    all.as_array().unwrap().iter().find(|p| p["builtin"] == builtin).unwrap().clone()
}

async fn versions(app: &axum::Router, id: &str) -> Vec<Value> {
    let (_, v) = call(app, "GET", &format!("/api/personas/{id}/versions"), None).await;
    v.as_array().unwrap().clone()
}

/// Makes `id` look as if an older PDT had installed `old` as its template.
async fn older_template(store: &Store, id: &str, old: &PersonaSetup) {
    store.save_persona_version(id, old, PersonaAuthor::Builtin, "updated by PDT", 1).await.unwrap();
    store.set_persona_template(id, old).await.unwrap();
}

#[tokio::test]
async fn edited_built_ins_keep_what_the_user_changed_and_follow_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, store) = run(tmp.path()).await;
    let pm = persona(&app, "pm").await;
    let reviewer = persona(&app, "reviewer").await;
    let (pm_id, reviewer_id) = (pm["id"].as_str().unwrap(), reviewer["id"].as_str().unwrap());
    let new_pm: PersonaSetup = serde_json::from_value(pm.clone()).unwrap();
    let new_reviewer: PersonaSetup = serde_json::from_value(reviewer.clone()).unwrap();

    // An older PDT's templates; the user then changed only the PM's model, and the Reviewer's instructions.
    let old_pm = PersonaSetup { instructions: "Old PM.".into(), ..new_pm.clone() };
    older_template(&store, pm_id, &old_pm).await;
    let mine = json!({"agent": "claude", "model": "opus", "instructions": "Old PM.", "permissionMode": "ask",
                      "tools": new_pm.tools, "note": "my model"});
    let (s, v) = call(&app, "PUT", &format!("/api/personas/{pm_id}"), Some(mine)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let old_reviewer = PersonaSetup {
        instructions: "Old review.".into(),
        permission_mode: Some(pdt_core::conversation::PermissionMode::Bypass),
        ..new_reviewer.clone()
    };
    older_template(&store, reviewer_id, &old_reviewer).await;
    let mut mine = serde_json::to_value(&old_reviewer).unwrap();
    mine["instructions"] = json!("My review.");
    call(&app, "PUT", &format!("/api/personas/{reviewer_id}"), Some(mine)).await;

    // What "Load PDT's template" shows.
    let (s, t) = call(&app, "GET", &format!("/api/personas/{pm_id}/template"), None).await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["setup"]["instructions"], new_pm.instructions.as_str());
    assert_eq!(t["diff"]["changed"], json!(["model", "instructions"]));

    // The next daemon run brings them up to the current templates.
    let (app, _store) = run(tmp.path()).await;
    let pm = persona(&app, "pm").await;
    assert_eq!((pm["model"].as_str(), pm["instructions"].as_str()), (Some("opus"), Some(new_pm.instructions.as_str())));
    let v = versions(&app, pm_id).await;
    assert_eq!((v[0]["author"].as_str(), v[0]["note"].as_str()), (Some("builtin"), Some("updated by PDT")));
    let reviewer = persona(&app, "reviewer").await;
    assert_eq!(reviewer["instructions"], "My review.", "the user's edit stays");
    assert_eq!(reviewer["permissionMode"], json!(new_reviewer.permission_mode), "the rest follows");
    let v = versions(&app, reviewer_id).await;
    assert_eq!(v[0]["note"], "updated by PDT; kept your instructions");

    // Once up to date, later runs add nothing.
    let n = versions(&app, pm_id).await.len();
    let (app, _store) = run(tmp.path()).await;
    persona(&app, "pm").await;
    assert_eq!(versions(&app, pm_id).await.len(), n);

    // Only built-ins have a template.
    let (_, p) =
        call(&app, "POST", "/api/personas", Some(json!({"name": "Mine", "agent": "claude", "instructions": "Hi."})))
            .await;
    let (s, e) = call(&app, "GET", &format!("/api/personas/{}/template", p["id"].as_str().unwrap()), None).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_builtin")));
}

/// `GET /api/personas/{id}/versions/{v}`: each saved version as it was; one that was never saved is not found.
#[tokio::test]
async fn each_version_reads_as_it_was_saved() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _store) = run(tmp.path()).await;
    let (s, p) =
        call(&app, "POST", "/api/personas", Some(json!({"name": "Mine", "agent": "claude", "instructions": "First."})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let id = p["id"].as_str().unwrap();
    let edit = json!({"agent": "claude", "instructions": "Second.", "note": "reworded"});
    let (s, v) = call(&app, "PUT", &format!("/api/personas/{id}"), Some(edit)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["version"], 2, "{v}");

    let (s, v1) = call(&app, "GET", &format!("/api/personas/{id}/versions/1"), None).await;
    assert_eq!(s, StatusCode::OK, "{v1}");
    assert_eq!((v1["version"].as_u64(), v1["instructions"].as_str()), (Some(1), Some("First.")), "{v1}");
    assert_eq!((v1["personaId"].as_str(), v1["author"].as_str()), (Some(id), Some("user")));
    let (_, v2) = call(&app, "GET", &format!("/api/personas/{id}/versions/2"), None).await;
    assert_eq!((v2["instructions"].as_str(), v2["note"].as_str()), (Some("Second."), Some("reworded")), "{v2}");

    let (s, _) = call(&app, "GET", &format!("/api/personas/{id}/versions/3"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, "GET", "/api/personas/nope/versions/1", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, "GET", &format!("/api/personas/{id}/versions/latest"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a version is a number");
}
