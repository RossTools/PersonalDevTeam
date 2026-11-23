//! Downloading from the Files view (PDT-621): a file as it is, or a folder as a `.zip` made while it streams. Paths
//! go through the Files view's checks ([`crate::files`]): nothing outside the checkout, symlinks included, and never
//! `.git`. Neither reads a whole file into memory.

use crate::error::{ApiError, codes};
use crate::extract::ApiQuery;
use crate::files::{Checkout, FilesQuery, at, checkout_dir, contained, io_err, is_git_name, resolve};
use crate::routes::AppState;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use cap_std::fs::Dir;
use serde::Deserialize;
use std::io::Write;

/// The most a folder's files may add up to in one `.zip`.
pub const MAX_ZIP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// How much is read or zipped before it is handed to the response.
const CHUNK: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadQuery {
    #[serde(default, alias = "worktree")]
    worktree_id: Option<String>,
    #[serde(default)]
    instance_id: Option<String>,
    #[serde(default)]
    path: String,
    /// `1`: the path is a folder, sent as a `.zip`.
    #[serde(default)]
    zip: Option<String>,
    /// `1`: the `.zip` includes the files `.gitignore` leaves out.
    #[serde(default)]
    ignored: Option<String>,
    /// `1`: only check that it would download (204, or the error it would fail with), so the UI can show why before
    /// the browser saves an error as the file.
    #[serde(default)]
    check: Option<String>,
}

/// Whether a query flag is on (`1` or `true`).
pub(crate) fn flag(v: &Option<String>) -> bool {
    matches!(v.as_deref(), Some("1" | "true"))
}

/// `Content-Disposition` for `name`: `attachment` (save it) or `inline`, with a plain-ASCII `filename` and the exact
/// name as UTF-8 `filename*` (RFC 6266).
pub(crate) fn disposition(kind: &str, name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| if (c.is_ascii_graphic() && c != '"' && c != '\\') || c == ' ' { c } else { '_' })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }
        })
        .collect();
    format!("{kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

/// The headers every download carries. `identity` keeps the response compression away from it: the bytes go as they
/// are (a zip is compressed already), with their length when it is known.
fn download(name: &str, media_type: &str, len: Option<u64>, body: Body) -> Response {
    with_headers(media_type, &disposition("attachment", name), len, body)
}

/// [`download`]'s headers, with the type and `Content-Disposition` given: a remote instance's download passed on.
pub(crate) fn with_headers(media_type: &str, disposition: &str, len: Option<u64>, body: Body) -> Response {
    let mut b = Response::builder()
        .header(header::CONTENT_TYPE, media_type)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CONTENT_ENCODING, "identity")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, "default-src 'none'; sandbox")
        .header(header::CACHE_CONTROL, "no-store");
    if let Some(len) = len {
        b = b.header(header::CONTENT_LENGTH, len);
    }
    b.body(body).unwrap_or_default()
}

/// The file's type by its extension.
fn media_type(name: &str) -> String {
    mime_guess::from_path(name).first_or_octet_stream().essence_str().to_string()
}

/// The last segment of `rel`, or the checkout's own name for the whole checkout.
fn name_of(checkout: &std::path::Path, rel: &str) -> String {
    match rel.rsplit_once('/') {
        Some((_, n)) => n.to_string(),
        None if !rel.is_empty() => rel.to_string(),
        None => checkout.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "files".into()),
    }
}

/// `GET /projects/{id}/files/download?path=&worktreeId=`: the file; with `zip=1`, the folder as a `.zip` (`ignored=1`
/// to include what `.gitignore` leaves out).
pub(crate) async fn get(
    State(st): State<AppState>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<DownloadQuery>,
) -> Result<Response, ApiError> {
    let how = Download { zip: flag(&q.zip), ignored: flag(&q.ignored), check: flag(&q.check) };
    let fq = FilesQuery { worktree_id: q.worktree_id, instance_id: q.instance_id, path: q.path };
    match resolve(&st, &id, &fq).await? {
        (Checkout::Here { path, plain }, rel) => respond(path, rel, plain, how).await,
        (Checkout::Remote { instance, branch }, rel) => crate::exec::download(&st, &instance, branch, rel, how).await,
    }
}

/// How to download: a folder as a `.zip` (`ignored`: with what `.gitignore` leaves out), or (`check`) only whether it
/// would.
#[derive(Debug, Clone, Copy, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Download {
    pub zip: bool,
    pub ignored: bool,
    pub check: bool,
}

/// The download of `rel` in `checkout`: the file, or the folder as a `.zip`.
pub(crate) async fn respond(
    checkout: std::path::PathBuf,
    rel: String,
    personal: bool,
    Download { zip, ignored, check }: Download,
) -> Result<Response, ApiError> {
    if zip {
        return folder(checkout, rel, personal, ignored, check).await;
    }
    if rel.is_empty() {
        return Err(ApiError::invalid("name a file"));
    }
    let name = name_of(&checkout, &rel);
    let (file, len) = tokio::task::spawn_blocking(move || -> Result<(std::fs::File, u64), ApiError> {
        contained(&checkout, &rel)?;
        let dir = checkout_dir(&checkout)?;
        // Asked before opening: Windows refuses to open a folder as a file, as access denied.
        if dir.metadata(&rel).map_err(not_found)?.is_dir() {
            return Err(ApiError::invalid("that is a folder: download it as a .zip"));
        }
        let file = dir.open(&rel).map_err(not_found)?;
        let len = file.metadata().map_err(io_err)?.len();
        Ok((file.into_std(), len))
    })
    .await
    .map_err(ApiError::internal)??;
    if check {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let stream = tokio_util::io::ReaderStream::with_capacity(tokio::fs::File::from_std(file), CHUNK);
    Ok(download(&name, &media_type(&name), Some(len), Body::from_stream(stream)))
}

fn not_found(e: std::io::Error) -> ApiError {
    match e.kind() {
        std::io::ErrorKind::NotFound => ApiError::not_found("file"),
        _ => io_err(e),
    }
}

/// The files a folder's `.zip` holds, with their sizes: what git lists (`.gitignore` honoured), or with `all` (and in
/// a plain folder) everything in it but `.git` folders. Files a link leads out to are left out, as are linked
/// folders. Blocking.
fn zip_files(checkout: &std::path::Path, dir: &Dir, rel: &str, all: bool) -> Result<Vec<(String, u64)>, ApiError> {
    let paths = if all { walk(dir, rel) } else { pdt_git::list_files(checkout, rel)? };
    Ok(paths
        .into_iter()
        .filter(|p| !p.split('/').any(is_git_name))
        // Git lists a deleted file until it is staged; a link out fails to open through the handle.
        .filter_map(|p| dir.metadata(&p).ok().filter(|m| m.is_file()).map(|m| (p, m.len())))
        .collect())
}

/// Every file under `rel`, not following linked folders (a loop) and skipping `.git`. Blocking.
fn walk(dir: &Dir, rel: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![rel.to_string()];
    while let Some(d) = todo.pop() {
        let Ok(read) = dir.read_dir(at(&d)) else { continue };
        for e in read.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if is_git_name(&name) {
                continue;
            }
            let path = if d.is_empty() { name } else { format!("{d}/{name}") };
            match e.file_type() {
                Ok(t) if t.is_dir() => todo.push(path),
                Ok(_) => out.push(path),
                Err(_) => {}
            }
        }
    }
    out.sort();
    out
}

/// A folder as a `.zip`, zipped on the blocking pool while the response streams it. Refused up front when its files
/// add up to more than [`MAX_ZIP_BYTES`].
async fn folder(
    checkout: std::path::PathBuf,
    rel: String,
    personal: bool,
    ignored: bool,
    check: bool,
) -> Result<Response, ApiError> {
    let name = name_of(&checkout, &rel);
    let prefix = if rel.is_empty() { String::new() } else { format!("{rel}/") };
    let (dir, files) = tokio::task::spawn_blocking(move || -> Result<(Dir, Vec<(String, u64)>), ApiError> {
        contained(&checkout, &rel)?;
        let dir = checkout_dir(&checkout)?;
        if !dir.metadata(at(&rel)).map_err(not_found)?.is_dir() {
            return Err(ApiError::invalid("that is a file: download it as it is"));
        }
        let files = zip_files(&checkout, &dir, &rel, ignored || personal)?;
        let total: u64 = files.iter().map(|(_, n)| n).sum();
        if total > MAX_ZIP_BYTES {
            let hint = if ignored { ", or leave out ignored files" } else { "" };
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                codes::TOO_LARGE,
                format!(
                    "this folder holds {} of files; a download is at most 2 GB. Download a smaller folder{hint}",
                    gb(total)
                ),
            ));
        }
        Ok((dir, files))
    })
    .await
    .map_err(ApiError::internal)??;
    if check {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<Bytes>>(4);
    let root = name.clone();
    tokio::task::spawn_blocking(move || {
        let mut out = Sender { tx: tx.clone(), buf: Vec::with_capacity(CHUNK) };
        let zipped = write_zip(&mut out, &dir, &root, &prefix, &files).and_then(|()| out.flush());
        if let Err(e) = zipped {
            // A download the user stopped is not worth a line in the log.
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                tracing::warn!(error = %e, "zipping a folder for download failed");
            }
            let _ = tx.blocking_send(Err(e));
        }
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|c| (c, rx)) });
    Ok(download(&format!("{name}.zip"), "application/zip", None, Body::from_stream(stream)))
}

/// Writes the `.zip` of `files` (paths under `prefix`, named `root/<the rest>` in it) to `out`. A file that went away
/// since it was listed is left out. Blocking.
fn write_zip(
    out: &mut impl Write,
    dir: &Dir,
    root: &str,
    prefix: &str,
    files: &[(String, u64)],
) -> std::io::Result<()> {
    use zip::write::SimpleFileOptions;
    let mut zip = zip::ZipWriter::new_stream(out);
    for (path, _) in files {
        let Ok(mut file) = dir.open(path) else { continue };
        let entry = format!("{root}/{}", path.strip_prefix(prefix).unwrap_or(path));
        let mut options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        if let Ok(m) = file.metadata() {
            if let Some(t) = m.modified().ok().and_then(|t| zip_time(t.into_std())) {
                options = options.last_modified_time(t);
            }
            #[cfg(unix)]
            {
                use cap_std::fs::PermissionsExt;
                options = options.unix_permissions(m.permissions().mode() & 0o777);
            }
        }
        zip.start_file(entry, options).map_err(std::io::Error::other)?;
        std::io::copy(&mut file, &mut zip)?;
    }
    zip.finish().map_err(std::io::Error::other)?;
    Ok(())
}

/// A file's modified time as a zip entry keeps it: local time, to the even second (`None` before 1980).
fn zip_time(t: std::time::SystemTime) -> Option<zip::DateTime> {
    let t = jiff::Timestamp::try_from(t).ok()?.to_zoned(jiff::tz::TimeZone::system());
    let as_u8 = |n: i8| n as u8;
    zip::DateTime::from_date_and_time(
        u16::try_from(t.year()).ok()?,
        as_u8(t.month()),
        as_u8(t.day()),
        as_u8(t.hour()),
        as_u8(t.minute()),
        as_u8(t.second()),
    )
    .ok()
}

/// Hands what it is written to the response in chunks, waiting while the client is behind (so a slow download
/// holds a few chunks, never the whole zip). Fails with `BrokenPipe` once the client has gone.
struct Sender {
    tx: tokio::sync::mpsc::Sender<std::io::Result<Bytes>>,
    buf: Vec<u8>,
}

impl Write for Sender {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK {
            self.flush()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(std::mem::replace(&mut self.buf, Vec::with_capacity(CHUNK)));
        self.tx.blocking_send(Ok(chunk)).map_err(|_| std::io::ErrorKind::BrokenPipe.into())
    }
}

fn gb(n: u64) -> String {
    format!("{:.1} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    axum::Router::new().route("/projects/{id}/files/download", axum::routing::get(self::get))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quotes and backslashes can't be in a file name on Windows, but an attachment's name may carry them.
    #[test]
    fn a_name_is_quoted_safely_and_kept_exactly_in_filename_star() {
        assert_eq!(
            disposition("attachment", "Résumé \"v2\"\\x.txt"),
            "attachment; filename=\"R_sum_ _v2__x.txt\"; filename*=UTF-8''R%C3%A9sum%C3%A9%20%22v2%22%5Cx.txt"
        );
        assert_eq!(disposition("inline", "a.png"), "inline; filename=\"a.png\"; filename*=UTF-8''a.png");
    }
}
