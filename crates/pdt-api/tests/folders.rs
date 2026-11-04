//! The folder picker's API (PDT-5): `GET /api/host/folders` and `POST /api/host/folders`.
use crate::common;

use axum::http::StatusCode;
use common::call;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{MAIN_SEPARATOR as SEP, Path, PathBuf};

/// A router whose home folder is `<tmp>/home`, holding `work/` (with a git repo `app` and a plain folder `notes`),
/// `.hidden/`, `Zeta/`, `alpha/` and a file.
async fn setup(tmp: &Path) -> (axum::Router, AppState, PathBuf) {
    let home = tmp.join("home");
    for d in ["work/app", "work/notes", ".hidden", "Zeta", "alpha"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    std::fs::write(home.join("file.txt"), "x").unwrap();
    crate::common::init_repo(&home.join("work/app"));
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp);
    state.home_dir = Some(home.clone());
    (router(state.clone(), None), state, home)
}

fn uri(path: &Path, extra: &str) -> String {
    format!("/api/host/folders?path={}{extra}", urlencode(&path.to_string_lossy()))
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn names(v: &Value) -> Vec<&str> {
    v["folders"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect()
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[tokio::test]
async fn lists_the_home_folder_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, home) = setup(tmp.path()).await;
    let (st, v) = call(&app, "GET", "/api/host/folders", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["path"], s(&home));
    assert_eq!(v["display"], "~");
    assert_eq!(v["sep"], SEP.to_string());
    assert_eq!(names(&v), ["alpha", "work", "Zeta"], "folders only, hidden left out, sorted ignoring case");
    assert_eq!(v["crumbs"], json!([{"name": "~", "path": s(&home)}]));
    assert_eq!(v["places"][0], json!({"name": "Home", "path": s(&home)}));
    assert_eq!(v["parent"], s(home.parent().unwrap()));
    assert_eq!(v["truncated"], false);

    let (_, tilde) = call(&app, "GET", "/api/host/folders?path=~", None).await;
    assert_eq!(tilde["path"], s(&home));
}

#[tokio::test]
async fn shows_hidden_folders_on_request() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, home) = setup(tmp.path()).await;
    let (_, v) = call(&app, "GET", &uri(&home, "&hidden=true"), None).await;
    assert_eq!(names(&v), [".hidden", "alpha", "work", "Zeta"]);
    assert_eq!(v["folders"][0]["hidden"], true);
    assert_eq!(v["folders"][1]["hidden"], false);
}

#[tokio::test]
async fn marks_git_repos_and_projects() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, home) = setup(tmp.path()).await;
    let (st, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "App", "repoPath": "~/work/app", "key": "APP"}))).await;
    assert_eq!(st, StatusCode::CREATED, "Add project takes a ~ path: {p}");

    let work = format!("~{SEP}work");
    let (st, v) = call(&app, "GET", &format!("/api/host/folders?path={}", urlencode(&work)), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["display"], work);
    assert_eq!(v["crumbs"], json!([{"name": "~", "path": s(&home)}, {"name": "work", "path": s(&home.join("work"))}]));
    let app_dir = &v["folders"][0];
    assert_eq!(app_dir["name"], "app");
    assert_eq!(app_dir["path"], s(&home.join("work").join("app")));
    assert_eq!(app_dir["display"], format!("~{SEP}work{SEP}app"));
    assert_eq!(app_dir["git"], true);
    assert_eq!(app_dir["project"], json!({"id": p["id"], "name": "App", "key": "APP"}));
    assert_eq!(v["folders"][1]["git"], false);
    assert_eq!(v["folders"][1]["project"], Value::Null);
}

#[tokio::test]
async fn offers_the_projects_folder_once_it_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, home) = setup(tmp.path()).await;
    let work = s(&home.join("work"));
    call(&app, "PUT", "/api/host/settings", Some(json!({"projectsDir": work}))).await;
    let (_, v) = call(&app, "GET", "/api/host/folders", None).await;
    assert_eq!(v["places"][1], json!({"name": "Projects folder", "path": work}));

    call(&app, "PUT", "/api/host/settings", Some(json!({"projectsDir": s(&home.join("nope"))}))).await;
    let (_, v) = call(&app, "GET", "/api/host/folders", None).await;
    assert!(v["places"].as_array().unwrap().iter().all(|p| p["name"] != "Projects folder"), "{v}");
}

#[tokio::test]
async fn reports_bad_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, home) = setup(tmp.path()).await;
    let (st, v) = call(&app, "GET", &uri(&home.join("missing"), ""), None).await;
    assert_eq!((st, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("folder_missing")), "{v}");
    assert_eq!(v["message"], format!("~{SEP}missing does not exist"));
    let (st, v) = call(&app, "GET", &uri(&home.join("file.txt"), ""), None).await;
    assert_eq!((st, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("not_a_folder")), "{v}");
    let (st, v) = call(&app, "GET", "/api/host/folders?path=work", None).await;
    assert_eq!((st, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_folder")), "{v}");
}

#[tokio::test]
async fn nearest_lists_the_closest_existing_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, home) = setup(tmp.path()).await;
    let (st, v) = call(&app, "GET", &uri(&home.join("work").join("new").join("deeper"), "&nearest=true"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["path"], s(&home.join("work")));
}

#[tokio::test]
async fn normalizes_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, home) = setup(tmp.path()).await;
    let messy = format!("{}{SEP}.{SEP}work{SEP}app{SEP}..{SEP}", s(&home));
    let (st, v) = call(&app, "GET", &format!("/api/host/folders?path={}", urlencode(&messy)), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["path"], s(&home.join("work")));
}

#[cfg(unix)]
#[tokio::test]
async fn follows_links_to_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, home) = setup(tmp.path()).await;
    std::os::unix::fs::symlink(home.join("work"), home.join("linked")).unwrap();
    std::os::unix::fs::symlink(home.join("file.txt"), home.join("filelink")).unwrap();
    let (_, v) = call(&app, "GET", &uri(&home, ""), None).await;
    assert_eq!(names(&v), ["alpha", "linked", "work", "Zeta"]);
}

#[tokio::test]
async fn creates_a_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, home) = setup(tmp.path()).await;
    let (st, v) = call(&app, "POST", "/api/host/folders", Some(json!({"parent": "~", "name": " new app "}))).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["path"], s(&home.join("new app")));
    assert_eq!(v["display"], format!("~{SEP}new app"));
    assert!(home.join("new app").is_dir());

    let (st, v) = call(&app, "POST", "/api/host/folders", Some(json!({"parent": "~", "name": "new app"}))).await;
    assert_eq!((st, v["code"].as_str()), (StatusCode::CONFLICT, Some("folder_exists")), "{v}");
    let (st, v) = call(&app, "POST", "/api/host/folders", Some(json!({"parent": "~", "name": "a/b"}))).await;
    assert_eq!((st, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_folder")), "{v}");
    let (st, v) = call(&app, "POST", "/api/host/folders", Some(json!({"parent": "~", "name": ".."}))).await;
    assert_eq!((st, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_folder")), "{v}");
    let missing = s(&home.join("missing"));
    let (st, v) = call(&app, "POST", "/api/host/folders", Some(json!({"parent": missing, "name": "x"}))).await;
    assert_eq!((st, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("folder_missing")), "{v}");
    assert!(!home.join("missing").exists(), "only one level is ever created");
}
