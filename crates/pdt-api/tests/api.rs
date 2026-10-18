use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::{AppState, router, with_host_guard};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;
use tower::ServiceExt;

async fn app(tmp: &Path, web_dir: Option<std::path::PathBuf>) -> axum::Router {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    router(AppState::new(store, tmp.join("data")), web_dir)
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String) {
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
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q", "-b", "main"]).arg(dir).status().unwrap().success());
}

#[tokio::test]
async fn health_reports_product() {
    let tmp = tempfile::tempdir().unwrap();
    let (status, body) = call(&app(tmp.path(), None).await, "GET", "/api/health", None).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["name"], "Personal Dev Team");
    // `AppState::new` is a dev instance on port 0.
    assert_eq!((v["profile"].as_str(), v["port"].as_u64(), v["dev"].as_bool()), (Some("dev"), Some(0), Some(true)));
}

async fn health_of(tmp: &Path, instance: pdt_core::instance::InstanceInfo) -> Value {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.instance = instance;
    let (status, body) = call(&router(state, None), "GET", "/api/health", None).await;
    assert_eq!(status, StatusCode::OK);
    serde_json::from_str(&body).unwrap()
}

#[tokio::test]
async fn health_reports_the_instance() {
    use pdt_core::brand::{Profile, STABLE_PORT};
    use pdt_core::instance::InstanceInfo;
    // The ports are only reported; nothing is bound.
    let tmp = tempfile::tempdir().unwrap();
    let v = health_of(tmp.path(), InstanceInfo::new(Profile::Stable, STABLE_PORT)).await;
    assert_eq!(
        (v["profile"].as_str(), v["port"].as_u64(), v["dev"].as_bool()),
        (Some("stable"), Some(7420), Some(false))
    );
    let tmp = tempfile::tempdir().unwrap();
    let v = health_of(tmp.path(), InstanceInfo::new(Profile::Dev, 8512)).await;
    assert_eq!((v["profile"].as_str(), v["port"].as_u64(), v["dev"].as_bool()), (Some("dev"), Some(8512), Some(true)));
    let tmp = tempfile::tempdir().unwrap();
    let v = health_of(tmp.path(), InstanceInfo::new(Profile::Stable, 8512)).await;
    assert_eq!(v["dev"], true);
}

#[tokio::test]
async fn create_then_list_and_get_project() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("my repo é");
    init_repo(&repo);
    let app = app(tmp.path(), None).await;

    let (status, body) =
        call(&app, "POST", "/api/projects", Some(json!({"name": " PDT ", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let created: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(created["name"], "PDT");
    assert_eq!(created["baseBranch"], "main");
    assert!(created["repoPath"].as_str().unwrap().ends_with("my repo é"));

    let (_, list) = call(&app, "GET", "/api/projects", None).await;
    assert_eq!(serde_json::from_str::<Value>(&list).unwrap().as_array().unwrap().len(), 1);

    let id = created["id"].as_str().unwrap();
    let (status, _) = call(&app, "GET", &format!("/api/projects/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&app, "GET", "/api/projects/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "not_found");
}

#[tokio::test]
async fn duplicate_repo_is_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("r");
    init_repo(&repo);
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    let app = app(tmp.path(), None).await;
    let (s1, _) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "a", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s1, StatusCode::CREATED);
    let (s2, body) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "b", "repoPath": repo.join("sub").to_string_lossy()})))
            .await;
    assert_eq!(s2, StatusCode::CONFLICT, "{body}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "conflict");
}

#[tokio::test]
async fn bad_paths_and_input_are_400_with_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let app = app(tmp.path(), None).await;
    for (body, code) in [
        (json!({"name": "x", "repoPath": plain.to_string_lossy()}), "not_a_git_repo"),
        (json!({"name": "x", "repoPath": tmp.path().join("missing").to_string_lossy()}), "path_not_found"),
        (json!({"name": "  ", "repoPath": plain.to_string_lossy()}), "invalid_input"),
    ] {
        let (status, resp) = call(&app, "POST", "/api/projects", Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["code"], code);
        assert!(!v["message"].as_str().unwrap().is_empty());
    }
}

#[tokio::test]
async fn unknown_api_route_is_json_404_and_spa_falls_back_to_index() {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<html>PDT</html>").unwrap();
    let app = app(tmp.path(), Some(web)).await;

    let (status, body) = call(&app, "GET", "/api/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "not_found");

    let (status, body) = call(&app, "GET", "/board/deep/link", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("PDT"));

    // Path traversal must never serve files outside web_dir.
    std::fs::write(tmp.path().join("Cargo.toml"), "[package]\n[workspace]\n").unwrap();
    for uri in ["/..%2f..%2fCargo.toml", "/%2e%2e%5cCargo.toml", "/../Cargo.toml", "/%2e%2e/Cargo.toml"] {
        let (_, body) = call(&app, "GET", uri, None).await;
        assert!(!body.contains("[package]") && !body.contains("[workspace]"), "{uri} leaked: {body}");
    }
}

async fn call_host(app: &axum::Router, uri: &str, host: Option<&str>) -> (StatusCode, String) {
    let mut req = Request::builder().uri(uri);
    if let Some(h) = host {
        req = req.header("host", h);
    }
    let res = app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn get_with(app: &axum::Router, uri: &str, headers: &[(&str, &str)]) -> axum::http::Response<Body> {
    let mut req = Request::builder().uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
}

#[tokio::test]
async fn the_web_ui_is_compressed_and_hashed_assets_are_cached_for_good() {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path().join("web");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("index.html"), "<html>PDT</html>").unwrap();
    let js = "console.log('pdt');\n".repeat(500);
    std::fs::write(web.join("assets/index-abc123.js"), &js).unwrap();
    let app = app(tmp.path(), Some(web)).await;

    let res = get_with(&app, "/assets/index-abc123.js", &[("accept-encoding", "gzip")]).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-encoding"], "gzip");
    assert_eq!(res.headers()["cache-control"], "public, max-age=31536000, immutable");
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert!(body.len() < js.len() / 10, "{} bytes", body.len());

    // index.html (also served for app routes) is always revalidated, so a new release is picked up at once.
    for uri in ["/", "/some/app/route"] {
        let res = get_with(&app, uri, &[]).await;
        assert_eq!(res.status(), StatusCode::OK, "{uri}");
        assert_eq!(res.headers()["cache-control"], "no-cache", "{uri}");
    }
    // A client that does not accept gzip gets the plain file.
    let res = get_with(&app, "/assets/index-abc123.js", &[]).await;
    assert!(res.headers().get("content-encoding").is_none());
}

#[tokio::test]
async fn host_guard_allows_only_listed_hosts() {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<html>PDT</html>").unwrap();
    let app = with_host_guard(app(tmp.path(), Some(web)).await, vec!["127.0.0.1:7420".into(), "localhost:7420".into()]);

    assert_eq!(call_host(&app, "/api/health", Some("127.0.0.1:7420")).await.0, StatusCode::OK);
    assert_eq!(call_host(&app, "/api/health", Some("LocalHost:7420")).await.0, StatusCode::OK);
    assert_eq!(call_host(&app, "/api/health", None).await.0, StatusCode::OK);
    for uri in ["/api/health", "/board/deep/link"] {
        let (status, body) = call_host(&app, uri, Some("evil.example:7420")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["code"], "forbidden_host");
        assert_eq!(v["message"], "host evil.example:7420 is not allowed");
    }
    assert_eq!(call_host(&app, "/api/health", Some("127.0.0.1:1")).await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn malformed_body_is_json_invalid_input() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), None).await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/projects")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["code"], "invalid_input");
    assert!(!v["message"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn missing_field_is_json_invalid_input() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), None).await;
    let (status, body) = call(&app, "POST", "/api/projects", Some(json!({"name": "x"}))).await;
    assert!(status.is_client_error(), "{status}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "invalid_input");
}

#[tokio::test]
async fn wrong_method_is_json_405() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), None).await;
    let (status, body) = call(&app, "DELETE", "/api/projects", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "method_not_allowed");
}

async fn call_origin(app: &axum::Router, method: &str, origin: Option<&str>) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri("/api/health").header("host", "127.0.0.1:7420");
    if let Some(o) = origin {
        req = req.header("origin", o);
    }
    let res = app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn origin_must_match_an_allowed_host() {
    let tmp = tempfile::tempdir().unwrap();
    let app = with_host_guard(
        app(tmp.path(), None).await,
        vec!["127.0.0.1:7420".into(), "localhost:7420".into(), "box.tail.ts.net".into()],
    );
    assert_eq!(call_origin(&app, "GET", None).await.0, StatusCode::OK);
    assert_eq!(call_origin(&app, "GET", Some("http://127.0.0.1:7420")).await.0, StatusCode::OK);
    assert_eq!(call_origin(&app, "GET", Some("http://LOCALHOST:7420")).await.0, StatusCode::OK);
    assert_eq!(call_origin(&app, "GET", Some("https://box.tail.ts.net")).await.0, StatusCode::OK);
    for bad in ["http://evil.example", "null", "http://127.0.0.1:9999", "file://", "chrome-extension://abc"] {
        let (status, body) = call_origin(&app, "GET", Some(bad)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{bad}");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["code"], "forbidden_origin");
        assert_eq!(v["message"], format!("origin {bad} is not allowed"));
    }
}

#[tokio::test]
async fn health_reports_version_and_wrong_content_type_is_json() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), None).await;
    let (_, body) = call(&app, "GET", "/api/health", None).await;
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["version"], pdt_core::BUILD_VERSION);
    let req = Request::builder()
        .method("POST")
        .uri("/api/projects")
        .header("content-type", "text/plain")
        .body(Body::from("{}"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["code"], "invalid_input");
}
