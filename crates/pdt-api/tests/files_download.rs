//! Downloading from the Files view (PDT-621): a file as it is, a folder as a `.zip` (`.gitignore` honoured unless
//! ignored files are asked for), never anything outside the checkout; and attachments with `?download=1`.

use crate::common;

use axum::http::{HeaderMap, StatusCode};
use common::{call, call_raw, get_bytes, git};
use pdt_api::router;
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::PathBuf;

struct Env {
    app: axum::Router,
    repo: PathBuf,
    pid: String,
    tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    std::fs::write(repo.join(".gitignore"), "build/\n").unwrap();
    std::fs::write(repo.join("src").join("main.rs"), "fn main() {}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let app = router(common::app_state(store, tmp.path()), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, repo, pid: p["id"].as_str().unwrap().into(), tmp }
}

impl Env {
    fn url(&self, query: &str) -> String {
        format!("/api/projects/{}/files/download?{query}", self.pid)
    }
}

fn header<'a>(h: &'a HeaderMap, name: &str) -> &'a str {
    h.get(name).map(|v| v.to_str().unwrap()).unwrap_or("")
}

/// The entry names of a `.zip`, and the text of one of them.
fn unzip(bytes: Vec<u8>) -> zip::ZipArchive<std::io::Cursor<Vec<u8>>> {
    zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap()
}

fn names(z: &zip::ZipArchive<std::io::Cursor<Vec<u8>>>) -> Vec<String> {
    let mut n: Vec<String> = z.file_names().map(|n| n.unwrap().into_owned()).collect();
    n.sort();
    n
}

fn read(z: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str) -> String {
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name(name).unwrap(), &mut s).unwrap();
    s
}

#[tokio::test]
async fn a_file_downloads_with_its_name_type_and_bytes() {
    let env = setup().await;
    std::fs::write(env.repo.join("src").join("Résumé v2.txt"), "bonjour\n").unwrap();
    let (s, h, body) = get_bytes(&env.app, &env.url("path=README.md")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, b"hello\n");
    assert_eq!(header(&h, "content-type"), "text/markdown");
    assert_eq!(header(&h, "content-length"), "6");
    assert_eq!(header(&h, "content-disposition"), "attachment; filename=\"README.md\"; filename*=UTF-8''README.md");
    assert_eq!(header(&h, "x-content-type-options"), "nosniff");

    let (s, h, body) = get_bytes(&env.app, &env.url("path=src/R%C3%A9sum%C3%A9%20v2.txt")).await;
    assert_eq!((s, body.as_slice()), (StatusCode::OK, b"bonjour\n".as_slice()));
    assert_eq!(header(&h, "content-type"), "text/plain");
    assert_eq!(
        header(&h, "content-disposition"),
        "attachment; filename=\"R_sum_ v2.txt\"; filename*=UTF-8''R%C3%A9sum%C3%A9%20v2.txt"
    );

    // From a ticket's worktree: its own copy.
    let (s, w) =
        call(&env.app, "POST", &format!("/api/projects/{}/worktrees", env.pid), Some(json!({"branch": "dl"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    std::fs::write(PathBuf::from(w["path"].as_str().unwrap()).join("README.md"), "worktree\n").unwrap();
    let (s, _, body) =
        get_bytes(&env.app, &env.url(&format!("path=README.md&worktreeId={}", w["id"].as_str().unwrap()))).await;
    assert_eq!((s, body.as_slice()), (StatusCode::OK, b"worktree\n".as_slice()));

    let (s, _) = call(&env.app, "GET", &env.url("path=nope.txt"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "GET", &env.url("path=nope.txt&check=1"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, body) = get_bytes(&env.app, &env.url("path=README.md&check=1")).await;
    assert_eq!((s, body.len()), (StatusCode::NO_CONTENT, 0));
    let (s, v) = call(&env.app, "GET", &env.url("path=src"), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")), "a folder goes as a zip");
    let (s, _) = call(&env.app, "GET", &env.url("path=README.md&zip=1"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a file does not");
}

#[tokio::test]
async fn nothing_outside_the_checkout_downloads() {
    let env = setup().await;
    let outside = env.tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();
    for bad in ["../outside/secret.txt", "src/../../outside/secret.txt", ".git/config", ".GIT/HEAD", "C:\\x"] {
        for zip in ["", "&zip=1"] {
            let (s, v) = call(&env.app, "GET", &env.url(&format!("path={bad}{zip}")), None).await;
            assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("path_outside")), "{bad}{zip}");
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.join("secret.txt"), env.repo.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, env.repo.join("linkdir")).unwrap();
        for bad in ["path=link.txt", "path=linkdir/secret.txt", "path=linkdir&zip=1"] {
            let (s, v) = call(&env.app, "GET", &env.url(bad), None).await;
            assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("path_outside")), "{bad}");
        }
        // Zipping the checkout leaves out what its links lead to.
        for q in ["zip=1", "zip=1&ignored=1"] {
            let (s, _, body) = get_bytes(&env.app, &env.url(q)).await;
            assert_eq!(s, StatusCode::OK);
            let z = unzip(body);
            assert!(!names(&z).iter().any(|n| n.contains("secret") || n.contains("link")), "{q}: {:?}", names(&z));
        }
    }
}

#[tokio::test]
async fn a_folder_zips_as_git_sees_it_unless_ignored_files_are_asked_for() {
    let env = setup().await;
    std::fs::create_dir_all(env.repo.join("build")).unwrap();
    std::fs::write(env.repo.join("build").join("out.bin"), [0u8, 1, 2]).unwrap();
    std::fs::write(env.repo.join("src").join("new.rs"), "// untracked\n").unwrap();
    std::fs::create_dir_all(env.repo.join("src").join("deep")).unwrap();
    std::fs::write(env.repo.join("src").join("deep").join("x.rs"), "x\n").unwrap();

    let (s, h, body) = get_bytes(&env.app, &env.url("zip=1")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(header(&h, "content-type"), "application/zip");
    assert_eq!(header(&h, "content-disposition"), "attachment; filename=\"repo.zip\"; filename*=UTF-8''repo.zip");
    let mut z = unzip(body);
    assert_eq!(
        names(&z),
        ["repo/.gitignore", "repo/README.md", "repo/src/deep/x.rs", "repo/src/main.rs", "repo/src/new.rs"]
    );
    assert_eq!(read(&mut z, "repo/src/main.rs"), "fn main() {}\n");
    let when = z.by_name("repo/src/main.rs").unwrap().last_modified().unwrap();
    assert!(when.year() > 1980, "entries keep their files' modified time: {when:?}");

    let (_, _, body) = get_bytes(&env.app, &env.url("zip=1&ignored=1")).await;
    let n = names(&unzip(body));
    assert!(n.contains(&"repo/build/out.bin".to_string()), "{n:?}");
    assert!(!n.iter().any(|n| n.contains(".git/")), "never .git: {n:?}");

    let (s, h, body) = get_bytes(&env.app, &env.url("path=src&zip=1")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(header(&h, "content-disposition").contains("filename=\"src.zip\""));
    assert_eq!(names(&unzip(body)), ["src/deep/x.rs", "src/main.rs", "src/new.rs"]);
    let (_, _, body) = get_bytes(&env.app, &env.url("path=src/deep&zip=1")).await;
    assert_eq!(names(&unzip(body)), ["deep/x.rs"]);
}

/// Unix only: the file is sparse there, where on Windows it would take 2 GB of the runner's disk.
#[cfg(unix)]
#[tokio::test]
async fn a_folder_over_the_cap_is_refused_before_anything_is_sent() {
    let env = setup().await;
    // Sparse: takes no room on disk.
    let f = std::fs::File::create(env.repo.join("huge.bin")).unwrap();
    f.set_len(2 * 1024 * 1024 * 1024 + 1).unwrap();
    drop(f);
    for q in ["zip=1", "zip=1&check=1"] {
        let (s, v) = call(&env.app, "GET", &env.url(q), None).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::PAYLOAD_TOO_LARGE, Some("too_large")), "{q}: {v}");
        assert!(v["message"].as_str().unwrap().contains("at most 2 GB"), "{v}");
    }
    let (s, _, body) = get_bytes(&env.app, &env.url("path=src&zip=1&check=1")).await;
    assert_eq!((s, body.len()), (StatusCode::NO_CONTENT, 0), "a check sends nothing");
    let (s, _, _) = get_bytes(&env.app, &env.url("path=src&zip=1")).await;
    assert_eq!(s, StatusCode::OK, "a smaller folder still downloads");
}

#[tokio::test]
async fn ticket_attachments_download_under_their_names() {
    let env = setup().await;
    let (s, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.pid), Some(json!({"title": "T"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let tid = t["id"].as_str().unwrap();
    let uri = format!("/api/tickets/{tid}/attachments?name=build%20log.txt&by=user");
    let (s, _, body) = call_raw(&env.app, "POST", &uri, b"line\n".to_vec()).await;
    assert_eq!(s, StatusCode::CREATED);
    let a: Value = serde_json::from_slice(&body).unwrap();
    let url = format!("/api/tickets/{tid}/attachments/{}", a["id"].as_str().unwrap());
    let (_, h, _) = get_bytes(&env.app, &url).await;
    assert!(header(&h, "content-disposition").starts_with("inline; "), "shown by default");
    let (s, h, body) = get_bytes(&env.app, &format!("{url}?download=1")).await;
    assert_eq!((s, body.as_slice()), (StatusCode::OK, b"line\n".as_slice()));
    assert_eq!(
        header(&h, "content-disposition"),
        "attachment; filename=\"build log.txt\"; filename*=UTF-8''build%20log.txt"
    );
}
