//! The GitHub REST client for updates and bug reports (PDT-153: issues on
//! the release repository, with the same token). The token goes only to the API host: API redirects are followed on
//! that host alone (a renamed or moved repository, PDT-195), asset redirects to storage by hand, without it (decision
//! 13). The repository is pinned by its numeric id when it has one, which survives renames and transfers.

use futures_util::StreamExt;
use pdt_core::host::UpdateChannel;
use pdt_core::release::{
    Change, HostOs, SUMS_ASSET, Version, newest_on_channel, parse_change, parse_sums, parse_version, select_asset,
};
use reqwest::header::{ACCEPT, LOCATION};
use serde::Deserialize;
use sha2::Digest;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub const GITHUB_API: &str = "https://api.github.com";
const MAX_REDIRECTS: usize = 5;
/// Same-host API redirects followed before giving up (a rename, then a transfer, is two).
pub const MAX_API_HOPS: usize = 3;
/// Releases asked for per page, and the pages read at most, when listing the releases an update brings.
const RELEASES_PER_PAGE: usize = 100;
const MAX_RELEASE_PAGES: usize = 10;
/// SHA256SUMS lists a handful of assets; anything bigger is not ours.
const SUMS_MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub id: u64,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    /// Published as a GitHub pre-release (a nightly build, PDT-283).
    pub prerelease: bool,
    pub notes: String,
    /// GitHub's `published_at`, unchanged (an ISO 8601 UTC string).
    pub published_at: Option<String>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Latest {
    pub release: Release,
    /// From `GitHub-Authentication-Token-Expiration` (ms); None for tokens without an expiry.
    pub token_expires_at: Option<i64>,
}

/// The commits from an installed version to a newer one (`compare`), newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Changes {
    pub entries: Vec<Change>,
    /// Every commit in the range; more than `entries` when GitHub cut the list off (it sends 250 at most).
    pub total: u64,
    /// The range's page on github.com.
    pub url: String,
}

impl Changes {
    pub fn truncated(&self) -> bool {
        self.total > self.entries.len() as u64
    }
}

#[derive(Deserialize)]
struct CompareJson {
    total_commits: u64,
    html_url: String,
    #[serde(default)]
    commits: Vec<CompareCommitJson>,
}

#[derive(Deserialize)]
struct CompareCommitJson {
    sha: String,
    commit: CommitJson,
}

#[derive(Deserialize)]
struct CommitJson {
    message: String,
}

/// An issue `create_issue` made.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Issue {
    pub number: u64,
    /// Its page on github.com.
    #[serde(rename = "html_url")]
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum UpdateError {
    #[error("the update token was rejected (expired or revoked)")]
    Unauthorized,
    #[error("no release was found, or the token cannot see the release repository")]
    NotFound,
    #[error("could not reach GitHub: {0}")]
    Network(String),
    #[error("GitHub answered {0}: {1}")]
    Http(u16, String),
    #[error(
        "GitHub redirected the request ({0}) and PDT did not follow it: it led to another host, or on after 3 hops"
    )]
    Redirected(u16),
    #[error("the latest release is not usable: {0}")]
    BadRelease(String),
    #[error("the release has no installer for this platform")]
    NoAsset,
    #[error("the release has no SHA256SUMS file")]
    NoChecksums,
    #[error("the checksum of {0} does not match SHA256SUMS")]
    Checksum(String),
    #[error("{0} is larger than the release says")]
    TooLarge(String),
    #[error("{0}")]
    Io(String),
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

impl ReleaseJson {
    /// The release, when its tag is a release version (`vX.Y.Z` or `vX.Y.Z-exp.N`, not a local build).
    fn into_release(self) -> Option<Release> {
        let version = parse_version(&self.tag_name).filter(|v| !v.local)?;
        Some(Release {
            tag: self.tag_name,
            version,
            prerelease: self.prerelease,
            notes: self.body.unwrap_or_default(),
            published_at: self.published_at,
            assets: self.assets.into_iter().map(|a| Asset { id: a.id, name: a.name, size: a.size }).collect(),
        })
    }
}

#[derive(Deserialize)]
struct AssetJson {
    id: u64,
    name: String,
    size: u64,
}

/// When the token expires, from GitHub's `GitHub-Authentication-Token-Expiration` header.
fn expiry_of(res: &reqwest::Response) -> Option<i64> {
    res.headers().get("github-authentication-token-expiration").and_then(|v| v.to_str().ok()).and_then(parse_expiry)
}

/// `YYYY-MM-DD HH:MM:SS UTC` or `YYYY-MM-DD HH:MM:SS ±HHMM` → Unix ms.
pub fn parse_expiry(header: &str) -> Option<i64> {
    let h = header.trim();
    let h = match h.strip_suffix(" UTC") {
        Some(t) => format!("{t} +0000"),
        None => h.to_string(),
    };
    jiff::Timestamp::strptime("%Y-%m-%d %H:%M:%S %z", &h).ok().map(|t| t.as_millisecond())
}

/// Without the URL: a redirect hop's URL is a signed storage link, which must not reach statuses, API errors or logs.
fn net(e: reqwest::Error) -> UpdateError {
    UpdateError::Network(e.without_url().to_string())
}

#[derive(Deserialize)]
struct ApiMessage {
    message: String,
}

async fn check(res: reqwest::Response) -> Result<reqwest::Response, UpdateError> {
    match res.status().as_u16() {
        200..=299 => Ok(res),
        code @ 300..=399 => Err(UpdateError::Redirected(code)),
        401 => Err(UpdateError::Unauthorized),
        404 => Err(UpdateError::NotFound),
        // Only GitHub's own JSON `message` is kept: a storage host's error body can echo the signed URL.
        code => {
            let text = res.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiMessage>(&text).map(|m| m.message).unwrap_or_default();
            Err(UpdateError::Http(code, message.chars().take(200).collect()))
        }
    }
}

/// Where the client finds the repository: its name, its numeric id when known, and where a permanent redirect last
/// showed it moved to.
#[derive(Debug, Clone, PartialEq, Default)]
struct Home {
    repo: String,
    id: Option<u64>,
    moved_to: Option<String>,
}

pub struct GitHubClient {
    http: reqwest::Client,
    api: String,
    home: Mutex<Home>,
}

/// A URL for logs: no query (a signed link's signature) and no fragment.
fn bare(u: &reqwest::Url) -> String {
    let mut u = u.clone();
    u.set_query(None);
    u.set_fragment(None);
    u.to_string()
}

fn same_origin(a: &reqwest::Url, b: &reqwest::Url) -> bool {
    (a.scheme(), a.host_str(), a.port_or_known_default()) == (b.scheme(), b.host_str(), b.port_or_known_default())
}

/// The repository part of an API path (`repositories/{id}` or `repos/{owner}/{name}`), when it is one.
fn repo_base(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match parts.as_slice() {
        ["repositories", id, ..] if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => {
            Some(format!("repositories/{id}"))
        }
        ["repos", owner, name, ..] if !owner.is_empty() && !name.is_empty() => Some(format!("repos/{owner}/{name}")),
        _ => None,
    }
}

/// Errors after which the next way to the repository is worth a try: it was not there, or GitHub refused the path.
/// Never a rejected token or a network failure (a POST may have gone through).
fn try_next(e: &UpdateError) -> bool {
    matches!(e, UpdateError::NotFound | UpdateError::Http(..) | UpdateError::Redirected(_))
}

impl GitHubClient {
    pub fn new(api_base: &str, repo: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(format!("{}-updater", pdt_core::brand::SLUG))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(900))
            .build()
            .expect("an HTTP client");
        Self {
            http,
            api: api_base.trim_end_matches('/').to_string(),
            home: Mutex::new(Home { repo: repo.to_string(), ..Default::default() }),
        }
    }

    /// Pins the repository by its numeric id (`/repositories/{id}/…`), which survives renames, org renames and
    /// transfers; the `owner/name` path stays the fallback.
    pub fn with_id(self, id: Option<u64>) -> Self {
        self.home.lock().unwrap_or_else(|e| e.into_inner()).id = id;
        self
    }

    fn home(&self) -> Home {
        self.home.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The repository (`owner/name`) this client talks to.
    pub fn repo(&self) -> String {
        self.home().repo
    }

    /// Its numeric id, when pinned.
    pub fn repo_id(&self) -> Option<u64> {
        self.home().id
    }

    /// Where a permanent redirect showed the repository moved (`repositories/{id}` or `repos/{owner}/{name}`); later
    /// requests go there first.
    pub fn moved_to(&self) -> Option<String> {
        self.home().moved_to
    }

    /// Points the client at another repository (an override, or back to the built-in one). A move seen for the old
    /// one is forgotten.
    pub fn set_repo(&self, repo: &str, id: Option<u64>) {
        let mut h = self.home.lock().unwrap_or_else(|e| e.into_inner());
        if (h.repo.as_str(), h.id) != (repo, id) {
            *h = Home { repo: repo.to_string(), id, moved_to: None };
        }
    }

    /// The ways to the repository, best first: where it moved, its id, its name.
    fn bases(&self) -> Vec<String> {
        let h = self.home();
        let mut out: Vec<String> = Vec::new();
        let all = [h.moved_to, h.id.map(|id| format!("repositories/{id}")), Some(format!("repos/{}", h.repo))];
        for b in all.into_iter().flatten() {
            if !out.contains(&b) {
                out.push(b);
            }
        }
        out
    }

    /// Remembers a permanent move of the repository a request was for (logged, without query strings).
    fn note_move(&self, from: &reqwest::Url, to: &reqwest::Url) {
        let prefix =
            reqwest::Url::parse(&self.api).map(|u| u.path().trim_end_matches('/').to_string()).unwrap_or_default();
        let rel = |u: &reqwest::Url| u.path().strip_prefix(prefix.as_str()).map(repo_base).unwrap_or(None);
        let (Some(old), Some(new)) = (rel(from), rel(to)) else { return };
        if old != new {
            tracing::warn!(from = old, to = new, "the release repository moved; using its new location");
            self.home.lock().unwrap_or_else(|e| e.into_inner()).moved_to = Some(new);
        }
    }

    /// Sends an API request with the token, following redirects that stay on the API host (with the token, at most
    /// `MAX_API_HOPS`). A redirect elsewhere, or one more after that, is returned as it is: the token never leaves
    /// the API host. An asset request (`keep_redirect`) follows none: its redirect is the storage link, followed by
    /// hand without the token (a moved repository is reached through its id or the move a check saw).
    #[allow(clippy::too_many_arguments)]
    async fn send_api(
        &self,
        method: reqwest::Method,
        url: &str,
        token: &str,
        accept: &str,
        body: Option<&serde_json::Value>,
        timeout: Option<Duration>,
        keep_redirect: bool,
    ) -> Result<reqwest::Response, UpdateError> {
        let max_hops = if keep_redirect { 0 } else { MAX_API_HOPS };
        let bad = |e: String| UpdateError::BadRelease(e);
        let api = reqwest::Url::parse(&self.api).map_err(|e| bad(e.to_string()))?;
        let mut url = reqwest::Url::parse(url).map_err(|e| bad(e.to_string()))?;
        let mut hops = 0;
        loop {
            let mut b = self
                .http
                .request(method.clone(), url.clone())
                .bearer_auth(token)
                .header(ACCEPT, accept)
                .header("X-GitHub-Api-Version", "2022-11-28");
            if let Some(t) = timeout {
                b = b.timeout(t);
            }
            if let Some(j) = body {
                b = b.json(j);
            }
            let res = b.send().await.map_err(net)?;
            let status = res.status().as_u16();
            if !matches!(status, 301 | 302 | 307 | 308) || hops == max_hops {
                return Ok(res);
            }
            let Some(next) = res.headers().get(LOCATION).and_then(|v| v.to_str().ok()).and_then(|l| url.join(l).ok())
            else {
                return Ok(res);
            };
            if !same_origin(&api, &next) {
                return Ok(res);
            }
            tracing::info!(status, from = bare(&url), to = bare(&next), "GitHub redirected an API request");
            if matches!(status, 301 | 308) {
                self.note_move(&url, &next);
            }
            hops += 1;
            url = next;
        }
    }

    /// A request on the repository (`tail` after its base), by each way to it in turn until one answers. With
    /// `keep_redirect`, a redirect off the API host is handed back (an asset's storage link).
    #[allow(clippy::too_many_arguments)]
    async fn repo_send(
        &self,
        method: reqwest::Method,
        tail: &str,
        token: &str,
        accept: &str,
        body: Option<&serde_json::Value>,
        timeout: Option<Duration>,
        keep_redirect: bool,
    ) -> Result<reqwest::Response, UpdateError> {
        let bases = self.bases();
        let mut err = UpdateError::NotFound;
        for (i, base) in bases.iter().enumerate() {
            let url = format!("{}/{base}/{tail}", self.api);
            let e = match self.send_api(method.clone(), &url, token, accept, body, timeout, keep_redirect).await {
                Ok(r) if keep_redirect && r.status().is_redirection() => return Ok(r),
                Ok(r) => match check(r).await {
                    Ok(r) => return Ok(r),
                    Err(e) => e,
                },
                Err(e) => e,
            };
            if !try_next(&e) {
                return Err(e);
            }
            if i + 1 < bases.len() {
                tracing::warn!(base, error = %e, "the release repository did not answer there; trying the next way");
            }
            err = e;
        }
        Err(err)
    }

    /// Opens an issue on the repository. The token needs Issues: write; without it GitHub answers 403 or 404.
    pub async fn create_issue(
        &self,
        token: &str,
        title: &str,
        body: &str,
        labels: &[&str],
    ) -> Result<Issue, UpdateError> {
        let json = serde_json::json!({"title": title, "body": body, "labels": labels});
        let res = self
            .repo_send(
                reqwest::Method::POST,
                "issues",
                token,
                "application/vnd.github+json",
                Some(&json),
                Some(Duration::from_secs(30)),
                false,
            )
            .await?;
        let text = res.text().await.map_err(net)?;
        serde_json::from_str(&text).map_err(|e| UpdateError::Http(200, format!("an answer that is not an issue: {e}")))
    }

    async fn get(&self, tail: &str, token: &str, timeout: Option<Duration>) -> Result<reqwest::Response, UpdateError> {
        self.repo_send(reqwest::Method::GET, tail, token, "application/vnd.github+json", None, timeout, false).await
    }

    pub async fn latest(&self, token: &str) -> Result<Latest, UpdateError> {
        let res = self.get("releases/latest", token, None).await?;
        let token_expires_at = expiry_of(&res);
        let text = res.text().await.map_err(net)?;
        let r: ReleaseJson = serde_json::from_str(&text).map_err(|e| UpdateError::BadRelease(e.to_string()))?;
        let tag = r.tag_name.clone();
        let release = r
            .into_release()
            .filter(|r| !r.version.is_nightly())
            .ok_or_else(|| UpdateError::BadRelease(format!("tag {tag:?} is not vX.Y.Z")))?;
        Ok(Latest { release, token_expires_at })
    }

    /// The release a host on `channel` updates to (PDT-210, PDT-283): on stable GitHub's latest release; on nightly the
    /// newest by SemVer among the full releases and the nightly pre-releases (`vX.Y.Z-nightly.YYYYMMDD`, and the
    /// older `vX.Y.Z-exp.N`) on the first page of releases (GitHub lists them newest first; the release workflow
    /// keeps 14 nightlies), so a stable `X.Y.Z` wins over its own nightlies.
    pub async fn newest(&self, token: &str, channel: UpdateChannel) -> Result<Latest, UpdateError> {
        if channel == UpdateChannel::Stable {
            return self.latest(token).await;
        }
        let tail = format!("releases?per_page={RELEASES_PER_PAGE}");
        let res = self.get(&tail, token, Some(Duration::from_secs(30))).await?;
        let token_expires_at = expiry_of(&res);
        let text = res.text().await.map_err(net)?;
        let list: Vec<ReleaseJson> = serde_json::from_str(&text)
            .map_err(|e| UpdateError::Http(200, format!("an answer that is not a list of releases: {e}")))?;
        let mut found: Vec<Release> =
            list.into_iter().filter(|r| !r.draft).filter_map(ReleaseJson::into_release).collect();
        let keys: Vec<(Version, bool)> = found.iter().map(|r| (r.version, r.prerelease)).collect();
        let i = newest_on_channel(channel, &keys).ok_or(UpdateError::NotFound)?;
        Ok(Latest { release: found.swap_remove(i), token_expires_at })
    }

    /// The published releases after `after` up to and including `upto`, newest first (PDT-192): what an update from
    /// `after` brings. Drafts, pre-releases and tags that are not `vX.Y.Z` are left out, except `upto` itself when it
    /// is a nightly build (its notes preview everything since the last release, so earlier ones would repeat
    /// them). GitHub lists releases newest
    /// first, so the pages stop at the first one with a release no newer than `after` (10 pages at most).
    pub async fn releases_between(
        &self,
        token: &str,
        after: &Version,
        upto: &Version,
    ) -> Result<Vec<Release>, UpdateError> {
        let mut out: Vec<Release> = Vec::new();
        for page in 1..=MAX_RELEASE_PAGES {
            let tail = format!("releases?per_page={RELEASES_PER_PAGE}&page={page}");
            let res = self.get(&tail, token, Some(Duration::from_secs(30))).await?;
            let text = res.text().await.map_err(net)?;
            let list: Vec<ReleaseJson> = serde_json::from_str(&text)
                .map_err(|e| UpdateError::Http(200, format!("an answer that is not a list of releases: {e}")))?;
            let full = list.len() >= RELEASES_PER_PAGE;
            let mut reached = false;
            let listed = list.into_iter().filter(|r| !r.draft).filter_map(ReleaseJson::into_release);
            for r in listed.filter(|r| r.version == *upto || (!r.prerelease && !r.version.is_nightly())) {
                if !r.version.is_newer_than(after) {
                    reached = true;
                } else if !r.version.is_newer_than(upto) && !out.iter().any(|o| o.version == r.version) {
                    out.push(r);
                }
            }
            if reached || !full {
                break;
            }
        }
        out.sort_by_key(|r| std::cmp::Reverse(r.version.key()));
        Ok(out)
    }

    /// The commits after `base` up to `head` (tags or commits), newest first (PDT-169). GitHub sends the oldest 250
    /// of a longer range; `total` says how many there are.
    pub async fn compare(&self, token: &str, base: &str, head: &str) -> Result<Changes, UpdateError> {
        let res = self.get(&format!("compare/{base}...{head}"), token, Some(Duration::from_secs(30))).await?;
        let text = res.text().await.map_err(net)?;
        let c: CompareJson = serde_json::from_str(&text)
            .map_err(|e| UpdateError::Http(200, format!("an answer that is not a comparison: {e}")))?;
        let entries = c.commits.iter().rev().map(|k| parse_change(&k.sha, &k.commit.message)).collect();
        Ok(Changes { entries, total: c.total_commits, url: c.html_url })
    }

    /// Streams an asset to `dest`, refusing more than its announced size. The API answers 200 or a redirect (fact 6);
    /// redirects off the API host are followed without the token. On failure `dest` is removed.
    pub async fn download(&self, token: &str, asset: &Asset, dest: &Path) -> Result<(), UpdateError> {
        self.download_capped(token, asset, dest, asset.size).await
    }

    async fn download_capped(&self, token: &str, asset: &Asset, dest: &Path, max: u64) -> Result<(), UpdateError> {
        let result = self.stream_to(token, asset, dest, max).await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(dest).await;
        }
        result
    }

    async fn stream_to(&self, token: &str, asset: &Asset, dest: &Path, max: u64) -> Result<(), UpdateError> {
        let tail = format!("releases/assets/{}", asset.id);
        let mut res =
            self.repo_send(reqwest::Method::GET, &tail, token, "application/octet-stream", None, None, true).await?;
        for _ in 0..MAX_REDIRECTS {
            if !res.status().is_redirection() {
                break;
            }
            let loc = res
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| UpdateError::BadRelease("a redirect without a Location".into()))?;
            let next = res.url().join(loc).map_err(|e| UpdateError::BadRelease(e.to_string()))?;
            res = self.http.get(next).header(ACCEPT, "application/octet-stream").send().await.map_err(net)?;
        }
        let res = check(res).await?;
        let too_large = || UpdateError::TooLarge(asset.name.clone());
        if res.content_length().is_some_and(|n| n > max) {
            return Err(too_large());
        }
        let io = |e: std::io::Error| UpdateError::Io(format!("writing {}: {e}", dest.display()));
        let mut file = tokio::fs::File::create(dest).await.map_err(io)?;
        let mut body = res.bytes_stream();
        let mut written: u64 = 0;
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(net)?;
            written += chunk.len() as u64;
            if written > max {
                return Err(too_large());
            }
            file.write_all(&chunk).await.map_err(io)?;
        }
        file.flush().await.map_err(io)?;
        Ok(())
    }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// One plain file name: a single normal path component, no separators, drive prefixes or leading dot.
fn is_plain_file_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(Component::Normal(_)), None))
        && !name.contains(['/', '\\', ':'])
        && !name.starts_with('.')
}

async fn sha256_of(path: &Path) -> Result<String, UpdateError> {
    let p = path.to_path_buf();
    tokio::task::spawn_blocking(move || sha256_file(&p))
        .await
        .map_err(|e| UpdateError::Io(e.to_string()))?
        .map_err(|e| UpdateError::Io(e.to_string()))
}

/// Downloads this platform's asset of `release` into `dir` and checks it against the release's SHA256SUMS. Returns
/// the verified file; on any failure nothing is left under the asset's name. A file already there that matches
/// SHA256SUMS is reused instead of downloaded again.
pub async fn fetch_verified(
    client: &GitHubClient,
    token: &str,
    release: &Release,
    os: HostOs,
    dir: &Path,
) -> Result<PathBuf, UpdateError> {
    let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
    let name = select_asset(&names, os).ok_or(UpdateError::NoAsset)?;
    if !is_plain_file_name(name) {
        return Err(UpdateError::BadRelease(format!("asset name {name:?} is not a plain file name")));
    }
    let asset = release.assets.iter().find(|a| a.name == name).expect("selected from the list");
    let sums_asset = release.assets.iter().find(|a| a.name == SUMS_ASSET).ok_or(UpdateError::NoChecksums)?;
    tokio::fs::create_dir_all(dir).await.map_err(|e| UpdateError::Io(e.to_string()))?;
    let sums_file = dir.join(SUMS_ASSET);
    client.download_capped(token, sums_asset, &sums_file, SUMS_MAX_BYTES).await?;
    let sums = parse_sums(&tokio::fs::read_to_string(&sums_file).await.map_err(|e| UpdateError::Io(e.to_string()))?);
    let expected = sums.get(name).cloned().ok_or_else(|| UpdateError::Checksum(name.to_string()))?;
    let fin = dir.join(name);
    if tokio::fs::try_exists(&fin).await.unwrap_or(false) && sha256_of(&fin).await.is_ok_and(|h| h == expected) {
        return Ok(fin);
    }
    let _ = tokio::fs::remove_file(&fin).await;
    let part = dir.join(format!("{name}.part"));
    client.download(token, asset, &part).await?;
    let actual = match sha256_of(&part).await {
        Ok(h) => h,
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
    };
    if actual != expected {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(UpdateError::Checksum(name.to_string()));
    }
    tokio::fs::rename(&part, &fin).await.map_err(|e| UpdateError::Io(e.to_string()))?;
    Ok(fin)
}
