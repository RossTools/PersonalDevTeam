use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pdt_core::release::HostOs;
use pdt_update::{GitHubClient, UpdateError, fetch_verified, parse_expiry};
use sha2::Digest;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone)]
struct Fake {
    base: Arc<std::sync::OnceLock<String>>,
    sums: Arc<std::sync::Mutex<String>>,
    token_leaked: Arc<AtomicBool>,
    storage_hits: Arc<AtomicUsize>,
}

const WIN: &[u8] = b"windows installer bytes";
const MAC: &[u8] = b"mac bundle bytes";

fn sha(b: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(b))
}

fn good_sums() -> String {
    format!("{}  Personal.Dev.Team_0.1.7_x64-setup.exe\n{}  Personal.Dev.Team.app.tar.gz\n", sha(WIN), sha(MAC))
}

fn authed(h: &HeaderMap) -> bool {
    h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) == Some("Bearer good")
}

async fn latest(h: HeaderMap) -> Response {
    if !authed(&h) {
        return (StatusCode::UNAUTHORIZED, r#"{"message":"Bad credentials"}"#).into_response();
    }
    let body = serde_json::json!({
        "tag_name": "v0.1.7", "body": "Release notes", "published_at": "2026-10-15T00:00:00Z",
        "assets": [
            {"id": 1, "name": "Personal.Dev.Team_0.1.7_x64-setup.exe", "size": WIN.len()},
            {"id": 2, "name": "Personal.Dev.Team.app.tar.gz", "size": MAC.len()},
            {"id": 3, "name": "SHA256SUMS", "size": 1}
        ]
    });
    ([("github-authentication-token-expiration", "2030-01-02 03:04:05 UTC")], axum::Json(body)).into_response()
}

async fn asset(State(f): State<Fake>, Path(id): Path<u64>, h: HeaderMap) -> Response {
    if !authed(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if h.get(header::ACCEPT).and_then(|v| v.to_str().ok()) != Some("application/octet-stream") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if id == 3 {
        return f.sums.lock().unwrap().clone().into_response();
    }
    if id == 4 {
        // A signed storage URL on a port nothing listens on: the hop fails with a network error.
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let to = format!("http://127.0.0.1:{port}/storage/4?X-Amz-Signature=sekrit");
        return (StatusCode::FOUND, [(header::LOCATION, to)]).into_response();
    }
    let to = format!("{}/storage/{id}", f.base.get().unwrap());
    (StatusCode::FOUND, [(header::LOCATION, to)]).into_response()
}

async fn storage(State(f): State<Fake>, Path(id): Path<u64>, h: HeaderMap) -> Response {
    if h.contains_key(header::AUTHORIZATION) {
        f.token_leaked.store(true, Ordering::SeqCst);
        return StatusCode::BAD_REQUEST.into_response();
    }
    f.storage_hits.fetch_add(1, Ordering::SeqCst);
    if id == 5 {
        let body = format!("<Error>{}/storage/5?X-Amz-Signature=sekrit</Error>", f.base.get().unwrap());
        return (StatusCode::FORBIDDEN, body).into_response();
    }
    (if id == 1 { WIN } else { MAC }).to_vec().into_response()
}

async fn serve() -> (GitHubClient, Fake) {
    let fake = Fake {
        base: Default::default(),
        sums: Arc::new(std::sync::Mutex::new(good_sums())),
        token_leaked: Default::default(),
        storage_hits: Default::default(),
    };
    let app = Router::new()
        .route("/repos/o/r/releases/latest", get(latest))
        .route("/repos/o/r/releases/assets/{id}", get(asset))
        .route("/storage/{id}", get(storage))
        .with_state(fake.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    fake.base.set(base.clone()).unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (GitHubClient::new(&base, "o/r"), fake)
}

#[tokio::test]
async fn latest_release_and_token_expiry() {
    let (c, _) = serve().await;
    let l = c.latest("good").await.unwrap();
    assert_eq!((l.release.tag.as_str(), l.release.version.to_string()), ("v0.1.7", "0.1.7".to_string()));
    assert_eq!(l.release.notes, "Release notes");
    assert_eq!(l.release.assets.len(), 3);
    assert_eq!(l.token_expires_at, parse_expiry("2030-01-02 03:04:05 UTC"));
    assert!(l.token_expires_at.is_some());
}

#[tokio::test]
async fn a_rejected_token_is_unauthorized() {
    let (c, _) = serve().await;
    assert_eq!(c.latest("expired").await.unwrap_err(), UpdateError::Unauthorized);
}

#[tokio::test]
async fn asset_redirect_drops_the_token() {
    let (c, fake) = serve().await;
    let release = c.latest("good").await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    for (os, bytes) in [(HostOs::Windows, WIN), (HostOs::Mac, MAC)] {
        let path = fetch_verified(&c, "good", &release, os, dir.path()).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    assert!(!fake.token_leaked.load(Ordering::SeqCst), "the storage host received the GitHub token");
}

#[tokio::test]
async fn bad_or_missing_checksums_are_rejected() {
    let (c, fake) = serve().await;
    let release = c.latest("good").await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    *fake.sums.lock().unwrap() = good_sums().replace(&sha(WIN), &"0".repeat(64));
    let e = fetch_verified(&c, "good", &release, HostOs::Windows, dir.path()).await.unwrap_err();
    assert!(matches!(e, UpdateError::Checksum(ref n) if n.contains("setup.exe")), "{e:?}");
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(!leftovers.iter().any(|n| n.to_string_lossy().ends_with("setup.exe")), "{leftovers:?}");
    *fake.sums.lock().unwrap() = String::new();
    let e = fetch_verified(&c, "good", &release, HostOs::Windows, dir.path()).await.unwrap_err();
    assert!(matches!(e, UpdateError::Checksum(_)), "an unlisted asset is rejected: {e:?}");
    let mut no_sums = release.clone();
    no_sums.assets.retain(|a| a.name != "SHA256SUMS");
    assert_eq!(
        fetch_verified(&c, "good", &no_sums, HostOs::Windows, dir.path()).await.unwrap_err(),
        UpdateError::NoChecksums
    );
}

#[test]
fn expiry_header_formats() {
    let utc = parse_expiry("2030-01-02 03:04:05 UTC").unwrap();
    assert_eq!(parse_expiry("2030-01-02 05:04:05 +0200"), Some(utc));
    assert_eq!(parse_expiry("2030-01-01 22:04:05 -0500"), Some(utc));
    assert_eq!(parse_expiry("soon"), None);
}

#[tokio::test]
async fn asset_names_cannot_escape_the_download_folder() {
    let (c, _) = serve().await;
    let mut release = c.latest("good").await.unwrap().release;
    release.assets[0].name = "../evil_x64-setup.exe".into();
    let dir = tempfile::tempdir().unwrap();
    let e = fetch_verified(&c, "good", &release, HostOs::Windows, &dir.path().join("d")).await.unwrap_err();
    assert!(matches!(e, UpdateError::BadRelease(_)), "{e:?}");
    assert!(!dir.path().join("evil_x64-setup.exe").exists());
}

fn an_asset(id: u64, name: &str, size: u64) -> pdt_update::Asset {
    pdt_update::Asset { id, name: name.into(), size }
}

#[tokio::test]
async fn failed_hops_never_reveal_the_signed_url_or_the_token() {
    let (c, _) = serve().await;
    let dir = tempfile::tempdir().unwrap();
    for id in [4, 5] {
        let e = c.download("good", &an_asset(id, "x-setup.exe", 10), &dir.path().join("x")).await.unwrap_err();
        let text = format!("{e} {e:?}");
        assert!(!text.contains("sekrit") && !text.contains("Signature") && !text.contains("/storage/"), "{text}");
        assert!(!text.contains("good"), "{text}");
        assert!(!dir.path().join("x").exists(), "a failed download leaves nothing behind");
    }
    let e = c.latest("bad-token-value").await.unwrap_err();
    assert!(!format!("{e} {e:?}").contains("bad-token-value"));
}

#[tokio::test]
async fn downloads_larger_than_announced_are_aborted() {
    let (c, fake) = serve().await;
    let mut release = c.latest("good").await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    release.assets[0].size = 3;
    let e = fetch_verified(&c, "good", &release, HostOs::Windows, dir.path()).await.unwrap_err();
    assert!(matches!(e, UpdateError::TooLarge(_)), "{e:?}");
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(!left.iter().any(|n| n.to_string_lossy().contains("setup.exe")), "{left:?}");
    *fake.sums.lock().unwrap() = "x".repeat(70 * 1024);
    let release = c.latest("good").await.unwrap().release;
    let e = fetch_verified(&c, "good", &release, HostOs::Windows, dir.path()).await.unwrap_err();
    assert!(matches!(e, UpdateError::TooLarge(ref n) if n == "SHA256SUMS"), "{e:?}");
}

#[tokio::test]
async fn asset_names_must_be_one_plain_component() {
    let (c, _) = serve().await;
    let release = c.latest("good").await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    for bad in ["C:evil_x64-setup.exe", "a/b_x64-setup.exe", r"a\b_x64-setup.exe", ".._x64-setup.exe"] {
        let mut r = release.clone();
        r.assets[0].name = bad.into();
        let e = fetch_verified(&c, "good", &r, HostOs::Windows, &dir.path().join("d")).await.unwrap_err();
        assert!(matches!(e, UpdateError::BadRelease(_)), "{bad}: {e:?}");
    }
}

#[tokio::test]
async fn an_already_verified_download_is_reused() {
    let (c, fake) = serve().await;
    let release = c.latest("good").await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    let first = fetch_verified(&c, "good", &release, HostOs::Windows, dir.path()).await.unwrap();
    let hits = fake.storage_hits.load(Ordering::SeqCst);
    let again = fetch_verified(&c, "good", &release, HostOs::Windows, dir.path()).await.unwrap();
    assert_eq!((first, fake.storage_hits.load(Ordering::SeqCst)), (again, hits));
    std::fs::write(dir.path().join("Personal.Dev.Team_0.1.7_x64-setup.exe"), b"tampered").unwrap();
    let p = fetch_verified(&c, "good", &release, HostOs::Windows, dir.path()).await.unwrap();
    assert_eq!(std::fs::read(p).unwrap(), WIN, "a file that no longer matches is downloaded again");
}
