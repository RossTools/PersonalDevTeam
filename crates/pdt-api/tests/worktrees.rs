use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;
use tower::ServiceExt;

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// Router plus a registered project whose repo has one commit on main. Returns (app, project id, repo path).
async fn setup(tmp: &Path) -> (axum::Router, String, std::path::PathBuf) {
    let repo = tmp.join("my repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let app = router(AppState::new(store, tmp.join("data")), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED);
    (app, p["id"].as_str().unwrap().to_string(), repo)
}

#[tokio::test]
async fn create_and_list_worktrees_inside_the_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, repo) = setup(tmp.path()).await;
    let (s, wt) =
        call(&app, "POST", &format!("/api/projects/{pid}/worktrees"), Some(json!({"branch": "feature/x"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{wt}");
    let path = std::path::PathBuf::from(wt["path"].as_str().unwrap());
    assert!(path.is_dir());
    assert!(path.ends_with(Path::new(".worktrees").join("feature-x")));
    assert_eq!(
        std::fs::canonicalize(path.parent().unwrap().parent().unwrap()).unwrap(),
        std::fs::canonicalize(&repo).unwrap()
    );
    assert_eq!(wt["branch"], "feature/x");
    assert_eq!(wt["baseCommit"].as_str().unwrap().len(), 40);

    let status = std::process::Command::new("git").current_dir(&repo).args(["status", "--porcelain"]).output().unwrap();
    assert!(status.stdout.is_empty(), "main checkout stays clean: {}", String::from_utf8_lossy(&status.stdout));

    let (_, list) = call(&app, "GET", &format!("/api/projects/{pid}/worktrees"), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn worktree_errors_have_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, _) = setup(tmp.path()).await;
    let uri = format!("/api/projects/{pid}/worktrees");
    for (body, status, code) in [
        (json!({"branch": "  "}), StatusCode::BAD_REQUEST, "invalid_input"),
        (json!({"branch": "a b"}), StatusCode::BAD_REQUEST, "invalid_input"),
        (json!({"branch": "main"}), StatusCode::CONFLICT, "branch_exists"),
        (json!({"branch": "x", "base": "nope"}), StatusCode::BAD_REQUEST, "unknown_ref"),
    ] {
        let (s, v) = call(&app, "POST", &uri, Some(body.clone())).await;
        assert_eq!((s, v["code"].as_str().unwrap()), (status, code), "{body}");
    }
    let (s, v) = call(&app, "GET", "/api/projects/nope/worktrees", None).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::NOT_FOUND, "not_found"));
}
