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
    /// SHA256SUMS.sig (asset 6).
    sig: Arc<std::sync::Mutex<Vec<u8>>>,
    token_leaked: Arc<AtomicBool>,
    storage_hits: Arc<AtomicUsize>,
    /// The issues posted (their JSON bodies).
    issues: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    /// Another host: what `/repos/away/r/…` redirects to.
    other_base: Arc<std::sync::OnceLock<String>>,
    other_hits: Arc<AtomicUsize>,
    /// Requests on `/repos/loop/r/…`, which redirects to itself.
    loop_hits: Arc<AtomicUsize>,
    /// Requests by the repository's id (`/repositories/77/…`).
    id_hits: Arc<AtomicUsize>,
    /// The pages of `/releases` asked for.
    release_pages: Arc<std::sync::Mutex<Vec<usize>>>,
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

/// A public repository: its releases are read without a token, or with a good one; any other token is rejected.
fn readable(h: &HeaderMap) -> bool {
    !h.contains_key(header::AUTHORIZATION) || authed(h)
}

async fn latest(h: HeaderMap) -> Response {
    if !readable(&h) {
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
    if !authed(&h) {
        return axum::Json(body).into_response();
    }
    ([("github-authentication-token-expiration", "2030-01-02 03:04:05 UTC")], axum::Json(body)).into_response()
}

/// `limited/r`: the anonymous limit is used up (403, `x-ratelimit-remaining: 0`, reset at a Unix second).
async fn rate_limited() -> Response {
    let headers = [("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1900000000")];
    (StatusCode::FORBIDDEN, headers, r#"{"message":"API rate limit exceeded for 1.2.3.4."}"#).into_response()
}

/// `busy/r`: a secondary rate limit (429 with `retry-after`).
async fn secondary_limit() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [("retry-after", "60")],
        r#"{"message":"You have exceeded a secondary rate limit."}"#,
    )
        .into_response()
}

/// `creds/r`: a token GitHub refuses with 403 and "Bad credentials".
async fn forbidden_token(h: HeaderMap) -> Response {
    if h.contains_key(header::AUTHORIZATION) {
        return (StatusCode::FORBIDDEN, r#"{"message":"Bad credentials"}"#).into_response();
    }
    latest(h).await
}

/// `private/r`: 403 for another reason is an HTTP error, not a rate limit or a rejected token.
async fn forbidden_other() -> Response {
    (StatusCode::FORBIDDEN, r#"{"message":"Resource not accessible"}"#).into_response()
}

async fn asset(State(f): State<Fake>, Path(id): Path<u64>, h: HeaderMap) -> Response {
    if !readable(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if h.get(header::ACCEPT).and_then(|v| v.to_str().ok()) != Some("application/octet-stream") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if id == 3 {
        return f.sums.lock().unwrap().clone().into_response();
    }
    if id == 6 {
        return f.sig.lock().unwrap().clone().into_response();
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

async fn issue(State(f): State<Fake>, h: HeaderMap, axum::Json(body): axum::Json<serde_json::Value>) -> Response {
    if h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) == Some("Bearer read-only") {
        let msg = r#"{"message":"Resource not accessible by personal access token"}"#;
        return (StatusCode::FORBIDDEN, msg).into_response();
    }
    if !authed(&h) {
        return (StatusCode::UNAUTHORIZED, r#"{"message":"Bad credentials"}"#).into_response();
    }
    f.issues.lock().unwrap().push(body);
    let number = f.issues.lock().unwrap().len() + 40;
    let url = format!("https://github.com/o/r/issues/{number}");
    (StatusCode::CREATED, axum::Json(serde_json::json!({"number": number, "html_url": url, "state": "open"})))
        .into_response()
}

/// `old/r` was renamed: GitHub sends its API requests to the repository's id, 301 for GET and 307 for the rest.
async fn renamed(State(f): State<Fake>, method: axum::http::Method, Path(rest): Path<String>) -> Response {
    let code =
        if method == axum::http::Method::GET { StatusCode::MOVED_PERMANENTLY } else { StatusCode::TEMPORARY_REDIRECT };
    (code, [(header::LOCATION, format!("{}/repositories/77/{rest}", f.base.get().unwrap()))]).into_response()
}

async fn by_id_latest(State(f): State<Fake>, h: HeaderMap) -> Response {
    f.id_hits.fetch_add(1, Ordering::SeqCst);
    latest(h).await
}

async fn by_id_asset(State(f): State<Fake>, id: Path<u64>, h: HeaderMap) -> Response {
    f.id_hits.fetch_add(1, Ordering::SeqCst);
    asset(State(f), id, h).await
}

async fn by_id_issue(State(f): State<Fake>, h: HeaderMap, body: axum::Json<serde_json::Value>) -> Response {
    f.id_hits.fetch_add(1, Ordering::SeqCst);
    issue(State(f), h, body).await
}

/// `away/r` points at another host.
async fn away(State(f): State<Fake>) -> Response {
    let to = format!("{}/repositories/77/releases/latest", f.other_base.get().unwrap());
    (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, to)]).into_response()
}

async fn redirect_loop(State(f): State<Fake>) -> Response {
    let n = f.loop_hits.fetch_add(1, Ordering::SeqCst);
    (StatusCode::FOUND, [(header::LOCATION, format!("/repos/loop/r/releases/latest?hop={n}"))]).into_response()
}

/// Every release, newest first as GitHub lists them: two nightly pre-releases v0.2.2-nightly.20261030.2 and
/// -nightly.20261030, two older experimental ones v0.2.2-exp.2 and -exp.1, v0.2.1, a draft v0.3.0, a pre-release v0.2.2
/// (not a nightly build), v0.2.0, then the per-merge releases v0.1.840 down to v0.1.1.
fn all_releases() -> Vec<serde_json::Value> {
    let r = |tag: &str, draft: bool, prerelease: bool| {
        serde_json::json!({"tag_name": tag, "body": format!("Notes for {tag}"), "published_at": null,
                           "draft": draft, "prerelease": prerelease, "assets": []})
    };
    let mut out = vec![
        r("v0.2.2-nightly.20261030.2", false, true),
        r("v0.2.2-nightly.20261030", false, true),
        r("v0.2.2-exp.2", false, true),
        r("v0.2.2-exp.1", false, true),
        r("v0.2.1", false, false),
        r("v0.3.0", true, false),
        r("v0.2.2", false, true),
        r("v0.2.0", false, false),
    ];
    out.extend((1..=840).rev().map(|n| r(&format!("v0.1.{n}"), false, false)));
    out
}

async fn releases(
    State(f): State<Fake>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, usize>>,
    h: HeaderMap,
) -> Response {
    if !readable(&h) {
        return (StatusCode::UNAUTHORIZED, r#"{"message":"Bad credentials"}"#).into_response();
    }
    let (page, per) = (q.get("page").copied().unwrap_or(1), q.get("per_page").copied().unwrap_or(30));
    f.release_pages.lock().unwrap().push(page);
    let list: Vec<_> = all_releases().into_iter().skip((page - 1) * per).take(per).collect();
    axum::Json(list).into_response()
}

/// `v0.1.5...v0.1.7`: two squashed merges; `abc1234...v0.1.7` (a dev build): one; `v0.1.1...v0.1.7`: 300 commits, of
/// which GitHub sends the oldest 250; anything else is not found.
async fn compare(Path(range): Path<String>, h: HeaderMap) -> Response {
    if !readable(&h) {
        return (StatusCode::UNAUTHORIZED, r#"{"message":"Bad credentials"}"#).into_response();
    }
    let commit = |sha: String, message: &str| serde_json::json!({"sha": sha, "commit": {"message": message}});
    let commits: Vec<serde_json::Value> = match range.as_str() {
        "v0.1.5...v0.1.7" => vec![
            commit("a1".into(), "PDT-5: First change (#10)\n\nIts body."),
            commit("a2".into(), "PDT-6: Second change (#11)"),
        ],
        "abc1234...v0.1.7" => vec![commit("a2".into(), "PDT-6: Second change (#11)")],
        "v0.1.1...v0.1.7" => (0..250).map(|i| commit(format!("s{i}"), &format!("Change {i} (#{i})"))).collect(),
        _ => return (StatusCode::NOT_FOUND, r#"{"message":"Not Found"}"#).into_response(),
    };
    let total = if range == "v0.1.1...v0.1.7" { 300 } else { commits.len() };
    let url = format!("https://github.com/o/r/compare/{range}");
    axum::Json(serde_json::json!({"total_commits": total, "html_url": url, "commits": commits})).into_response()
}

async fn serve() -> (GitHubClient, Fake) {
    let fake = Fake {
        base: Default::default(),
        sums: Arc::new(std::sync::Mutex::new(good_sums())),
        sig: Default::default(),
        token_leaked: Default::default(),
        storage_hits: Default::default(),
        issues: Default::default(),
        other_base: Default::default(),
        other_hits: Default::default(),
        loop_hits: Default::default(),
        id_hits: Default::default(),
        release_pages: Default::default(),
    };
    let other = Router::new()
        .route(
            "/{*rest}",
            get(|State(f): State<Fake>, h: HeaderMap| async move {
                f.other_hits.fetch_add(1, Ordering::SeqCst);
                if h.contains_key(header::AUTHORIZATION) {
                    f.token_leaked.store(true, Ordering::SeqCst);
                }
                latest(h).await
            }),
        )
        .with_state(fake.clone());
    let ol = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    fake.other_base.set(format!("http://127.0.0.1:{}", ol.local_addr().unwrap().port())).unwrap();
    tokio::spawn(async move { axum::serve(ol, other).await.unwrap() });
    let app = Router::new()
        .route("/repos/o/r/releases/latest", get(latest))
        .route("/repos/o/r/releases", get(releases))
        .route("/repos/o/r/releases/assets/{id}", get(asset))
        .route("/repos/o/r/compare/{range}", get(compare))
        .route("/storage/{id}", get(storage))
        .route("/repos/o/r/issues", axum::routing::post(issue))
        .route("/repos/old/r/{*rest}", get(renamed).post(renamed))
        .route("/repositories/77/releases/latest", get(by_id_latest))
        .route("/repositories/77/releases/assets/{id}", get(by_id_asset))
        .route("/repositories/77/issues", axum::routing::post(by_id_issue))
        .route("/repos/away/r/releases/latest", get(away))
        .route("/repos/loop/r/releases/latest", get(redirect_loop))
        .route("/repos/limited/r/releases/latest", get(rate_limited))
        .route("/repos/busy/r/releases/latest", get(secondary_limit))
        .route("/repos/creds/r/releases/latest", get(forbidden_token))
        .route("/repos/private/r/releases/latest", get(forbidden_other))
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
    let l = c.latest(Some("good")).await.unwrap();
    assert_eq!((l.release.tag.as_str(), l.release.version.to_string()), ("v0.1.7", "0.1.7".to_string()));
    assert_eq!(l.release.notes, "Release notes");
    assert_eq!(l.release.assets.len(), 3);
    assert_eq!(l.token_expires_at, parse_expiry("2030-01-02 03:04:05 UTC"));
    assert!(l.token_expires_at.is_some());
}

#[tokio::test]
async fn the_releases_an_update_brings_skip_drafts_and_pre_releases() {
    let (c, fake) = serve().await;
    let v = |s| pdt_core::release::parse_version(s).unwrap();
    let got = c.releases_between(Some("good"), &v("0.1.840"), &v("0.2.1")).await.unwrap();
    let tags: Vec<_> = got.iter().map(|r| r.tag.as_str()).collect();
    assert_eq!(tags, ["v0.2.1", "v0.2.0"], "newest first, no draft or pre-release");
    assert_eq!(got[1].notes, "Notes for v0.2.0");
    assert_eq!(*fake.release_pages.lock().unwrap(), [1], "the first page reached the installed version");

    // Up to the latest release only, and across pages when the installed version is further back.
    fake.release_pages.lock().unwrap().clear();
    let got = c.releases_between(Some("good"), &v("0.1.700"), &v("0.2.0")).await.unwrap();
    assert_eq!(got.len(), 141);
    assert_eq!((got[0].tag.as_str(), got[140].tag.as_str()), ("v0.2.0", "v0.1.701"));
    assert_eq!(*fake.release_pages.lock().unwrap(), [1, 2]);

    // Nothing newer than the installed version: nothing listed.
    assert!(c.releases_between(Some("good"), &v("0.2.1"), &v("0.2.1")).await.unwrap().is_empty());
    assert_eq!(c.releases_between(Some("bad"), &v("0.1.1"), &v("0.2.1")).await, Err(UpdateError::Unauthorized));
}

#[tokio::test]
async fn each_channel_finds_its_newest_release() {
    use pdt_core::host::UpdateChannel;
    let (c, fake) = serve().await;
    let stable = c.newest(Some("good"), UpdateChannel::Stable).await.unwrap();
    assert_eq!(stable.release.tag, "v0.1.7", "stable is GitHub's latest release");
    let nightly = c.newest(Some("good"), UpdateChannel::Nightly).await.unwrap();
    assert_eq!(
        nightly.release.tag, "v0.2.2-nightly.20261030.2",
        "the day's newest nightly, above the experimental builds and not the plain pre-release v0.2.2"
    );
    assert!(nightly.release.prerelease && nightly.release.version.is_nightly());
    assert_eq!(*fake.release_pages.lock().unwrap(), [1]);
    assert_eq!(c.newest(Some("bad"), UpdateChannel::Nightly).await.unwrap_err(), UpdateError::Unauthorized);
}

#[tokio::test]
async fn a_nightly_update_brings_its_own_notes_and_the_releases_before_it() {
    let (c, _) = serve().await;
    let v = |s| pdt_core::release::parse_version(s).unwrap();
    let tags = |l: Vec<pdt_update::Release>| l.into_iter().map(|r| r.tag).collect::<Vec<_>>();
    // A nightly's notes preview everything unreleased, so its earlier builds are left out.
    assert_eq!(
        tags(c.releases_between(Some("good"), &v("0.2.0"), &v("0.2.2-nightly.20261030.2")).await.unwrap()),
        ["v0.2.2-nightly.20261030.2", "v0.2.1"]
    );
    // From an experimental build to the next nightly (a host switched from the experimental channel).
    assert_eq!(
        tags(c.releases_between(Some("good"), &v("0.2.2-exp.2"), &v("0.2.2-nightly.20261030")).await.unwrap()),
        ["v0.2.2-nightly.20261030"]
    );
    // The older experimental builds the same way.
    assert_eq!(
        tags(c.releases_between(Some("good"), &v("0.2.0"), &v("0.2.2-exp.2")).await.unwrap()),
        ["v0.2.2-exp.2", "v0.2.1"]
    );
    assert_eq!(
        tags(c.releases_between(Some("good"), &v("0.2.2-exp.1"), &v("0.2.2-exp.2")).await.unwrap()),
        ["v0.2.2-exp.2"]
    );
    // Up to a stable release, nightly and experimental builds never count.
    assert_eq!(tags(c.releases_between(Some("good"), &v("0.2.0"), &v("0.2.1")).await.unwrap()), ["v0.2.1"]);
}

#[tokio::test]
async fn changes_between_two_releases_are_newest_first() {
    let (c, _) = serve().await;
    let ch = c.compare(Some("good"), "v0.1.5", "v0.1.7").await.unwrap();
    let got: Vec<_> =
        ch.entries.iter().map(|e| (e.sha.as_str(), e.title.as_str(), e.ticket.as_deref(), e.pr)).collect();
    assert_eq!(
        got,
        [("a2", "Second change", Some("PDT-6"), Some(11)), ("a1", "First change", Some("PDT-5"), Some(10))]
    );
    assert_eq!((ch.total, ch.truncated()), (2, false));
    assert_eq!(ch.url, "https://github.com/o/r/compare/v0.1.5...v0.1.7");
}

#[tokio::test]
async fn changes_since_a_dev_builds_commit() {
    let (c, _) = serve().await;
    let ch = c.compare(Some("good"), "abc1234", "v0.1.7").await.unwrap();
    assert_eq!(ch.entries.iter().map(|e| e.sha.as_str()).collect::<Vec<_>>(), ["a2"]);
}

#[tokio::test]
async fn an_unknown_base_or_a_rejected_token_fails_the_comparison() {
    let (c, _) = serve().await;
    assert_eq!(c.compare(Some("good"), "v0.0.9", "v0.1.7").await.unwrap_err(), UpdateError::NotFound);
    assert_eq!(c.compare(Some("expired"), "v0.1.5", "v0.1.7").await.unwrap_err(), UpdateError::Unauthorized);
}

#[tokio::test]
async fn a_long_range_says_it_was_cut_off() {
    let (c, _) = serve().await;
    let ch = c.compare(Some("good"), "v0.1.1", "v0.1.7").await.unwrap();
    assert_eq!((ch.entries.len(), ch.total, ch.truncated()), (250, 300, true));
    assert_eq!(ch.entries[0].sha, "s249", "newest of those sent first");
}

#[tokio::test]
async fn a_rejected_token_is_unauthorized() {
    let (c, _) = serve().await;
    assert_eq!(c.latest(Some("expired")).await.unwrap_err(), UpdateError::Unauthorized);
}

#[tokio::test]
async fn asset_redirect_drops_the_token() {
    let (c, fake) = serve().await;
    let release = c.latest(Some("good")).await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    for (os, bytes) in [(HostOs::Windows, WIN), (HostOs::Mac, MAC)] {
        let path = fetch_verified(&c, Some("good"), &release, os, dir.path(), None).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    assert!(!fake.token_leaked.load(Ordering::SeqCst), "the storage host received the GitHub token");
}

#[tokio::test]
async fn bad_or_missing_checksums_are_rejected() {
    let (c, fake) = serve().await;
    let release = c.latest(Some("good")).await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    *fake.sums.lock().unwrap() = good_sums().replace(&sha(WIN), &"0".repeat(64));
    let e = fetch_verified(&c, Some("good"), &release, HostOs::Windows, dir.path(), None).await.unwrap_err();
    assert!(matches!(e, UpdateError::Checksum(ref n) if n.contains("setup.exe")), "{e:?}");
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(!leftovers.iter().any(|n| n.to_string_lossy().ends_with("setup.exe")), "{leftovers:?}");
    *fake.sums.lock().unwrap() = String::new();
    let e = fetch_verified(&c, Some("good"), &release, HostOs::Windows, dir.path(), None).await.unwrap_err();
    assert!(matches!(e, UpdateError::Checksum(_)), "an unlisted asset is rejected: {e:?}");
    let mut no_sums = release.clone();
    no_sums.assets.retain(|a| a.name != "SHA256SUMS");
    assert_eq!(
        fetch_verified(&c, Some("good"), &no_sums, HostOs::Windows, dir.path(), None).await.unwrap_err(),
        UpdateError::NoChecksums
    );
}

/// A signing key, its public half as `brand::RELEASE_SIGNING_KEY` holds it, and a signer for the release's message.
fn release_key(seed: u8) -> (String, impl Fn(&str, &str) -> Vec<u8>) {
    use base64::Engine;
    use p256::ecdsa::signature::Signer;
    use p256::pkcs8::EncodePublicKey;
    let sk = p256::ecdsa::SigningKey::from_slice(&[seed; 32]).unwrap();
    let der = sk.verifying_key().to_public_key_der().unwrap();
    let key = base64::engine::general_purpose::STANDARD.encode(der.as_bytes());
    let sign = move |tag: &str, sums: &str| {
        let sig: p256::ecdsa::Signature = sk.sign(&pdt_update::signed_message(tag, sums.as_bytes()));
        sig.to_der().as_bytes().to_vec()
    };
    (key, sign)
}

#[tokio::test]
async fn with_a_key_only_a_signed_sha256sums_is_trusted() {
    let (c, fake) = serve().await;
    let mut release = c.latest(Some("good")).await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    let (key, sign) = release_key(7);
    let key = Some(key.as_str());
    let win = HostOs::Windows;
    assert_eq!(
        fetch_verified(&c, None, &release, win, dir.path(), key).await.unwrap_err(),
        UpdateError::NoSignature,
        "no SHA256SUMS.sig"
    );
    release.assets.push(an_asset(6, "SHA256SUMS.sig", 72));
    *fake.sig.lock().unwrap() = sign("v0.1.7", &good_sums());
    let path = fetch_verified(&c, None, &release, win, dir.path(), key).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), WIN);
    std::fs::remove_file(&path).unwrap();

    // SHA256SUMS and the asset swapped together, with the old signature: refused before any download.
    let hits = fake.storage_hits.load(Ordering::SeqCst);
    *fake.sums.lock().unwrap() = good_sums().replace(&sha(WIN), &sha(b"trojan"));
    let e = fetch_verified(&c, None, &release, win, dir.path(), key).await.unwrap_err();
    assert!(matches!(e, UpdateError::Signature(_)), "{e:?}");
    assert_eq!(fake.storage_hits.load(Ordering::SeqCst), hits, "the asset was never downloaded");
    // Another release's signed SHA256SUMS, or one signed by another key.
    *fake.sums.lock().unwrap() = good_sums();
    *fake.sig.lock().unwrap() = sign("v0.1.6", &good_sums());
    assert!(matches!(fetch_verified(&c, None, &release, win, dir.path(), key).await, Err(UpdateError::Signature(_))));
    *fake.sig.lock().unwrap() = release_key(9).1("v0.1.7", &good_sums());
    assert!(matches!(fetch_verified(&c, None, &release, win, dir.path(), key).await, Err(UpdateError::Signature(_))));
    assert!(!dir.path().join("Personal.Dev.Team_0.1.7_x64-setup.exe").exists());
    // Without a key the signature isn't looked at.
    fetch_verified(&c, None, &release, win, dir.path(), None).await.unwrap();
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
    let mut release = c.latest(Some("good")).await.unwrap().release;
    release.assets[0].name = "../evil_x64-setup.exe".into();
    let dir = tempfile::tempdir().unwrap();
    let e = fetch_verified(&c, Some("good"), &release, HostOs::Windows, &dir.path().join("d"), None).await.unwrap_err();
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
        let e = c.download(Some("good"), &an_asset(id, "x-setup.exe", 10), &dir.path().join("x")).await.unwrap_err();
        let text = format!("{e} {e:?}");
        assert!(!text.contains("sekrit") && !text.contains("Signature") && !text.contains("/storage/"), "{text}");
        assert!(!text.contains("good"), "{text}");
        assert!(!dir.path().join("x").exists(), "a failed download leaves nothing behind");
    }
    let e = c.latest(Some("bad-token-value")).await.unwrap_err();
    assert!(!format!("{e} {e:?}").contains("bad-token-value"));
}

#[tokio::test]
async fn downloads_larger_than_announced_are_aborted() {
    let (c, fake) = serve().await;
    let mut release = c.latest(Some("good")).await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    release.assets[0].size = 3;
    let e = fetch_verified(&c, Some("good"), &release, HostOs::Windows, dir.path(), None).await.unwrap_err();
    assert!(matches!(e, UpdateError::TooLarge(_)), "{e:?}");
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(!left.iter().any(|n| n.to_string_lossy().contains("setup.exe")), "{left:?}");
    *fake.sums.lock().unwrap() = "x".repeat(70 * 1024);
    let release = c.latest(Some("good")).await.unwrap().release;
    let e = fetch_verified(&c, Some("good"), &release, HostOs::Windows, dir.path(), None).await.unwrap_err();
    assert!(matches!(e, UpdateError::TooLarge(ref n) if n == "SHA256SUMS"), "{e:?}");
}

#[tokio::test]
async fn asset_names_must_be_one_plain_component() {
    let (c, _) = serve().await;
    let release = c.latest(Some("good")).await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    for bad in ["C:evil_x64-setup.exe", "a/b_x64-setup.exe", r"a\b_x64-setup.exe", ".._x64-setup.exe"] {
        let mut r = release.clone();
        r.assets[0].name = bad.into();
        let e = fetch_verified(&c, Some("good"), &r, HostOs::Windows, &dir.path().join("d"), None).await.unwrap_err();
        assert!(matches!(e, UpdateError::BadRelease(_)), "{bad}: {e:?}");
    }
}

#[tokio::test]
async fn an_already_verified_download_is_reused() {
    let (c, fake) = serve().await;
    let release = c.latest(Some("good")).await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    let first = fetch_verified(&c, Some("good"), &release, HostOs::Windows, dir.path(), None).await.unwrap();
    let hits = fake.storage_hits.load(Ordering::SeqCst);
    let again = fetch_verified(&c, Some("good"), &release, HostOs::Windows, dir.path(), None).await.unwrap();
    assert_eq!((first, fake.storage_hits.load(Ordering::SeqCst)), (again, hits));
    std::fs::write(dir.path().join("Personal.Dev.Team_0.1.7_x64-setup.exe"), b"tampered").unwrap();
    let p = fetch_verified(&c, Some("good"), &release, HostOs::Windows, dir.path(), None).await.unwrap();
    assert_eq!(std::fs::read(p).unwrap(), WIN, "a file that no longer matches is downloaded again");
}

#[tokio::test]
async fn issues_are_created_with_title_body_and_labels() {
    let (c, fake) = serve().await;
    assert_eq!(c.repo(), "o/r");
    let i = c.create_issue("good", "It broke", "Steps", &["bug", "from-pdt"]).await.unwrap();
    assert_eq!(i, pdt_update::Issue { number: 41, url: "https://github.com/o/r/issues/41".into() });
    let posted = fake.issues.lock().unwrap().clone();
    assert_eq!(posted, vec![serde_json::json!({"title": "It broke", "body": "Steps", "labels": ["bug", "from-pdt"]})]);
}

#[tokio::test]
async fn issue_errors_say_why_and_never_hold_the_token() {
    let (c, fake) = serve().await;
    let e = c.create_issue("read-only", "t", "b", &[]).await.unwrap_err();
    assert!(matches!(e, UpdateError::Http(403, ref m) if m.contains("not accessible")), "{e:?}");
    let e = c.create_issue("bad-token-value", "t", "b", &[]).await.unwrap_err();
    assert_eq!(e, UpdateError::Unauthorized);
    assert!(!format!("{e} {e:?}").contains("bad-token-value"));
    assert!(fake.issues.lock().unwrap().is_empty());
}

async fn client(repo: &str) -> (GitHubClient, Fake) {
    let (_, fake) = serve().await;
    (GitHubClient::new(fake.base.get().unwrap(), repo), fake)
}

#[tokio::test]
async fn a_renamed_repository_is_followed_with_the_token_and_remembered() {
    let (c, fake) = client("old/r").await;
    let l = c.latest(Some("good")).await.unwrap();
    assert_eq!(l.release.tag, "v0.1.7", "the token went along to the same host");
    assert_eq!(l.token_expires_at, parse_expiry("2030-01-02 03:04:05 UTC"));
    assert_eq!(c.moved_to().as_deref(), Some("repositories/77"));
    // Later requests (an asset download too) go to where it moved, without asking the old name again.
    let dir = tempfile::tempdir().unwrap();
    let path = fetch_verified(&c, Some("good"), &l.release, HostOs::Windows, dir.path(), None).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), WIN);
    assert!(fake.id_hits.load(Ordering::SeqCst) >= 3, "latest, SHA256SUMS and the asset by id");
    assert!(!fake.token_leaked.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_redirect_to_another_host_is_not_followed_with_the_token() {
    let (c, fake) = client("away/r").await;
    let e = c.latest(Some("good")).await.unwrap_err();
    assert_eq!(e, UpdateError::Redirected(301));
    assert_eq!(fake.other_hits.load(Ordering::SeqCst), 0, "the other host never saw the request");
    assert!(!fake.token_leaked.load(Ordering::SeqCst));
    assert_eq!(c.moved_to(), None);
}

#[tokio::test]
async fn a_redirect_loop_stops_after_three_hops() {
    let (c, fake) = client("loop/r").await;
    let e = c.latest(Some("good")).await.unwrap_err();
    assert_eq!(e, UpdateError::Redirected(302));
    assert_eq!(fake.loop_hits.load(Ordering::SeqCst), 1 + pdt_update::MAX_API_HOPS);
}

#[tokio::test]
async fn the_id_path_is_used_when_set() {
    let (c, fake) = client("gone/r").await;
    let c = c.with_id(Some(77));
    assert_eq!(c.latest(Some("good")).await.unwrap().release.tag, "v0.1.7");
    assert_eq!(fake.id_hits.load(Ordering::SeqCst), 1);
    assert_eq!(c.repo_id(), Some(77));
}

#[tokio::test]
async fn an_unknown_id_falls_back_to_the_name() {
    let (c, fake) = client("o/r").await;
    let c = c.with_id(Some(99));
    assert_eq!(c.latest(Some("good")).await.unwrap().release.tag, "v0.1.7");
    assert_eq!(fake.id_hits.load(Ordering::SeqCst), 0);
    // A rejected token is not retried elsewhere.
    assert_eq!(c.latest(Some("expired")).await.unwrap_err(), UpdateError::Unauthorized);
}

#[tokio::test]
async fn bug_reports_reach_a_moved_repository() {
    let (c, fake) = client("old/r").await;
    let i = c.create_issue("good", "It broke", "Steps", &["bug"]).await.unwrap();
    assert_eq!(i.number, 41);
    assert_eq!(fake.id_hits.load(Ordering::SeqCst), 1, "the POST followed GitHub's 307 with its body");
    assert_eq!(fake.issues.lock().unwrap()[0]["title"], "It broke");
}

#[tokio::test]
async fn pointing_the_client_elsewhere_forgets_the_move() {
    let (c, _) = client("old/r").await;
    c.latest(Some("good")).await.unwrap();
    assert!(c.moved_to().is_some());
    c.set_repo("o/r", None);
    assert_eq!((c.repo(), c.moved_to()), ("o/r".to_string(), None));
    assert_eq!(c.latest(Some("good")).await.unwrap().release.tag, "v0.1.7");
}

#[tokio::test]
async fn releases_are_read_and_downloaded_without_a_token() {
    let (c, fake) = serve().await;
    let l = c.latest(None).await.unwrap();
    assert_eq!((l.release.tag.as_str(), l.token_expires_at), ("v0.1.7", None));
    assert_eq!(
        c.newest(None, pdt_core::host::UpdateChannel::Nightly).await.unwrap().release.tag,
        "v0.2.2-nightly.20261030.2"
    );
    assert_eq!(c.compare(None, "v0.1.5", "v0.1.7").await.unwrap().entries.len(), 2);
    let dir = tempfile::tempdir().unwrap();
    let path = fetch_verified(&c, None, &l.release, HostOs::Windows, dir.path(), None).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), WIN);
    assert!(!fake.token_leaked.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_used_up_rate_limit_says_when_it_resets() {
    let (c, _) = serve().await;
    c.set_repo("limited/r", None);
    assert_eq!(c.latest(None).await.unwrap_err(), UpdateError::RateLimited { reset_at: Some(1_900_000_000_000) });
    c.set_repo("busy/r", None);
    let before = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let UpdateError::RateLimited { reset_at: Some(at) } = c.latest(None).await.unwrap_err() else { panic!() };
    assert!((before + 59_000..=before + 70_000).contains(&at), "{at} vs {before}");
}

#[tokio::test]
async fn a_403_for_bad_credentials_is_a_rejected_token() {
    let (c, _) = serve().await;
    c.set_repo("creds/r", None);
    assert_eq!(c.latest(Some("good")).await.unwrap_err(), UpdateError::Unauthorized);
    assert_eq!(c.latest(None).await.unwrap().release.tag, "v0.1.7");
    c.set_repo("private/r", None);
    assert!(matches!(c.latest(None).await.unwrap_err(), UpdateError::Http(403, _)));
}
