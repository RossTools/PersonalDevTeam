//! The GitHub REST client for updates (remote-host spec §3.2–3.3; verified fact 6) and bug reports (PDT-153: issues on
//! the release repository, with the same token). The token goes only to the API host: asset redirects are followed by
//! hand, without it (decision 13).

use futures_util::StreamExt;
use pdt_core::release::{HostOs, SUMS_ASSET, Version, parse_sums, parse_version, select_asset};
use reqwest::header::{ACCEPT, LOCATION};
use serde::Deserialize;
use sha2::Digest;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub const GITHUB_API: &str = "https://api.github.com";
const MAX_REDIRECTS: usize = 5;
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
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
struct AssetJson {
    id: u64,
    name: String,
    size: u64,
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

pub struct GitHubClient {
    http: reqwest::Client,
    api: String,
    repo: String,
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
        Self { http, api: api_base.trim_end_matches('/').to_string(), repo: repo.to_string() }
    }

    fn api_get(&self, path: &str, token: &str, accept: &str) -> reqwest::RequestBuilder {
        self.http
            .get(format!("{}/repos/{}/{path}", self.api, self.repo))
            .bearer_auth(token)
            .header(ACCEPT, accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    /// The repository (`owner/name`) this client talks to.
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// Opens an issue on the repository. The token needs Issues: write; without it GitHub answers 403 or 404.
    pub async fn create_issue(
        &self,
        token: &str,
        title: &str,
        body: &str,
        labels: &[&str],
    ) -> Result<Issue, UpdateError> {
        let res = self
            .http
            .post(format!("{}/repos/{}/issues", self.api, self.repo))
            .bearer_auth(token)
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(30))
            .json(&serde_json::json!({"title": title, "body": body, "labels": labels}))
            .send()
            .await
            .map_err(net)?;
        let text = check(res).await?.text().await.map_err(net)?;
        serde_json::from_str(&text).map_err(|e| UpdateError::Http(200, format!("an answer that is not an issue: {e}")))
    }

    pub async fn latest(&self, token: &str) -> Result<Latest, UpdateError> {
        let res = self.api_get("releases/latest", token, "application/vnd.github+json").send().await.map_err(net)?;
        let token_expires_at = res
            .headers()
            .get("github-authentication-token-expiration")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_expiry);
        let text = check(res).await?.text().await.map_err(net)?;
        let r: ReleaseJson = serde_json::from_str(&text).map_err(|e| UpdateError::BadRelease(e.to_string()))?;
        let version = parse_version(&r.tag_name)
            .filter(|v| !v.local)
            .ok_or_else(|| UpdateError::BadRelease(format!("tag {:?} is not vX.Y.Z", r.tag_name)))?;
        Ok(Latest {
            release: Release {
                tag: r.tag_name,
                version,
                notes: r.body.unwrap_or_default(),
                published_at: r.published_at,
                assets: r.assets.into_iter().map(|a| Asset { id: a.id, name: a.name, size: a.size }).collect(),
            },
            token_expires_at,
        })
    }

    /// Streams an asset to `dest`, refusing more than its announced size. The API answers 200 or a redirect (fact 6);
    /// redirects are followed without the token. On failure `dest` is removed.
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
        let mut res = self
            .api_get(&format!("releases/assets/{}", asset.id), token, "application/octet-stream")
            .send()
            .await
            .map_err(net)?;
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
