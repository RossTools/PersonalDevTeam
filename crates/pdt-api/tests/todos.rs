//! PDT-125: the user's todo list over the API: add, edit, check off, reorder, link to a project or ticket, delete.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use serde_json::{Value, json};

struct Env {
    app: axum::Router,
    project: String,
    ticket: String,
    _tmp: tempfile::TempDir,
}

/// A project `P` (key `PX`) with one ticket, `PX-1`.
async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let app = pdt_api::router(pdt_api::AppState::new(store, tmp.path().join("data")), None);
    let (s, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "P", "key": "PX", "repoPath": repo.to_string_lossy()})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    let (s, t) =
        call(&app, "POST", &format!("/api/projects/{project}/tickets"), Some(json!({"title": "Fix the login"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    Env { app, project, ticket: t["id"].as_str().unwrap().to_string(), _tmp: tmp }
}

async fn add(env: &Env, body: Value) -> Value {
    let (s, v) = call(&env.app, "POST", "/api/todos", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

async fn patch(env: &Env, id: &str, body: Value) -> Value {
    let (s, v) = call(&env.app, "PATCH", &format!("/api/todos/{id}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

async fn titles(env: &Env, query: &str) -> Vec<String> {
    let (s, v) = call(&env.app, "GET", &format!("/api/todos{query}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v.as_array().unwrap().iter().map(|t| t["title"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn todos_are_added_edited_checked_off_reordered_and_deleted() {
    let env = setup().await;
    let first = add(&env, json!({"title": "  Write the plan "})).await;
    assert_eq!(
        (&first["title"], &first["done"], &first["projectId"]),
        (&json!("Write the plan"), &json!(false), &Value::Null)
    );
    let second = add(&env, json!({"title": "Call Sam"})).await;
    assert_eq!(titles(&env, "").await, ["Call Sam", "Write the plan"], "new todos go on top");

    // Edit and reorder.
    let id = first["id"].as_str().unwrap();
    let moved =
        patch(&env, id, json!({"title": "Write the spec", "position": second["position"].as_f64().unwrap() - 1.0}))
            .await;
    assert_eq!(moved["title"], "Write the spec");
    assert_eq!(titles(&env, "").await, ["Write the spec", "Call Sam"]);

    // Check off, and back.
    let done = patch(&env, id, json!({"done": true})).await;
    assert!(done["doneAt"].is_number(), "{done}");
    assert_eq!(titles(&env, "?done=true").await, ["Write the spec"]);
    assert_eq!(titles(&env, "?done=false").await, ["Call Sam"]);
    assert_eq!(titles(&env, "").await, ["Call Sam", "Write the spec"], "done ones come after the open ones");
    let open = patch(&env, id, json!({"done": false})).await;
    assert_eq!(open["doneAt"], Value::Null);

    // Bad input.
    let (s, v) = call(&env.app, "POST", "/api/todos", Some(json!({"title": "  "}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (s, _) = call(&env.app, "POST", "/api/todos", Some(json!({"title": "x", "project": "NOPE"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "PATCH", "/api/todos/missing", Some(json!({"done": true}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Delete.
    let (s, _) = call(&env.app, "DELETE", &format!("/api/todos/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&env.app, "DELETE", &format!("/api/todos/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(titles(&env, "").await, ["Call Sam"]);
}

#[tokio::test]
async fn todos_link_to_a_project_or_a_ticket_and_filter_by_project() {
    let env = setup().await;
    let loose = add(&env, json!({"title": "Buy milk"})).await;
    let by_key = add(&env, json!({"title": "Review PRs", "project": "PX"})).await;
    assert_eq!((&by_key["projectId"], &by_key["projectKey"]), (&json!(env.project), &json!("PX")));

    // A ticket brings its project along; its key and title come with the todo.
    let linked = patch(&env, loose["id"].as_str().unwrap(), json!({"ticket": "PX-1"})).await;
    assert_eq!(
        (&linked["ticketId"], &linked["ticketKey"], &linked["ticketTitle"], &linked["projectKey"]),
        (&json!(env.ticket), &json!("PX-1"), &json!("Fix the login"), &json!("PX"))
    );
    let mut by_project = titles(&env, &format!("?project={}", env.project)).await;
    by_project.sort();
    assert_eq!(by_project, ["Buy milk", "Review PRs"]);
    assert_eq!(titles(&env, "?project=PX&done=true").await, Vec::<String>::new());

    // Moving it to no project drops the ticket, which lives in one.
    let moved = patch(&env, loose["id"].as_str().unwrap(), json!({"project": ""})).await;
    assert_eq!((&moved["ticketId"], &moved["projectId"]), (&Value::Null, &Value::Null));
    patch(&env, loose["id"].as_str().unwrap(), json!({"ticket": "PX-1"})).await;
    let kept = patch(&env, loose["id"].as_str().unwrap(), json!({"project": "PX"})).await;
    assert_eq!(kept["ticketKey"], "PX-1", "the same project keeps it");

    // `""` removes a link.
    let unlinked = patch(&env, loose["id"].as_str().unwrap(), json!({"ticket": "", "project": ""})).await;
    assert_eq!((&unlinked["ticketId"], &unlinked["projectId"]), (&Value::Null, &Value::Null));
    assert_eq!(titles(&env, "?project=PX").await, ["Review PRs"]);

    let made = add(&env, json!({"title": "Ship it", "ticket": env.ticket})).await;
    assert_eq!(made["projectId"], json!(env.project));
    let (s, _) = call(
        &env.app,
        "PATCH",
        &format!("/api/todos/{}", made["id"].as_str().unwrap()),
        Some(json!({"ticket": "PX-99"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
