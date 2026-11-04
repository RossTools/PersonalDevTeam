//! Programs PDT installs for its built-in ACP profiles (PDT-239): Google's Antigravity ACP server, from the ACP
//! registry (pdt-core `acp_registry`). Install fetches the registry, downloads the archive for this platform from the
//! agent's own site only, checks its sha256 against the registry's, and unpacks it (nothing in it runs) into
//! `<data dir>/toolchains/<registry id>/<version>/`, beside a `pdt-install.json` saying what to launch. A path set in
//! Settings → Agents wins over it. Sign in starts the installed server and asks it to sign in (ACP `authenticate`):
//! the server opens the browser and keeps the credentials itself; PDT never sees them.

use crate::error::{ApiError, OrNotFound, codes};
use crate::routes::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::acp_agent::{AcpAgentDef, AcpFlavor};
use pdt_core::acp_registry::{self, RegistryRelease};
use pdt_core::host::HostSettings;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;

/// The manifest beside an installed program.
const MANIFEST: &str = "pdt-install.json";
/// The largest registry document PDT reads.
const MAX_REGISTRY_BYTES: u64 = 8 << 20;
/// How long the user has to finish signing in in the browser (the server's own limit is 300 s).
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(320);

/// Where PDT downloads from. [`AppState::new`] has [`Offline`]; only pdtd's production wiring installs [`Http`], so
/// tests never reach the network.
#[async_trait::async_trait]
pub trait ToolchainSource: Send + Sync {
    /// The body at `url`, at most `limit` bytes.
    async fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, String>;
}

/// No network.
pub struct Offline;

#[async_trait::async_trait]
impl ToolchainSource for Offline {
    async fn get(&self, _url: &str, _limit: u64) -> Result<Vec<u8>, String> {
        Err("this PDT has no network access for installs".into())
    }
}

/// Over HTTPS, following no redirects (the registry and the archive's site answer directly).
pub struct Http(reqwest::Client);

impl Http {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .connect_timeout(pdt_core::outbound::CONNECT_TIMEOUT)
            .timeout(pdt_core::outbound::DOWNLOAD_TIMEOUT)
            .user_agent(pdt_core::brand::user_agent(""))
            .build()
            .unwrap_or_default();
        Http(client)
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ToolchainSource for Http {
    async fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, String> {
        let mut res = self.0.get(url).send().await.map_err(|e| e.without_url().to_string())?;
        if !res.status().is_success() {
            return Err(format!("{url} answered {}", res.status()));
        }
        if res.content_length().is_some_and(|n| n > limit) {
            return Err(format!("{url} is larger than PDT downloads"));
        }
        let mut body = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(|e| e.without_url().to_string())? {
            body.extend_from_slice(&chunk);
            if body.len() as u64 > limit {
                return Err(format!("{url} is larger than PDT downloads"));
            }
        }
        Ok(body)
    }
}

/// What `pdt-install.json` records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: String,
    cmd: String,
    args: Vec<String>,
    sha256: String,
    archive: String,
}

/// A program PDT installed.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub version: String,
    pub program: PathBuf,
    pub args: Vec<String>,
}

fn root(data_dir: &FsPath, registry_id: &str) -> PathBuf {
    data_dir.join("toolchains").join(registry_id)
}

/// The newest installed version of the registry agent `registry_id`, when its program is there.
pub fn installed(data_dir: &FsPath, registry_id: &str) -> Option<Installed> {
    let entries = std::fs::read_dir(root(data_dir, registry_id)).ok()?;
    entries
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let m: Manifest = serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).ok()?).ok()?;
            let parts = acp_registry::safe_relative(&m.cmd)?;
            let program = parts.iter().fold(dir.clone(), |p, part| p.join(part));
            (acp_registry::valid_version(&m.version) && program.is_file()).then_some(Installed {
                version: m.version,
                program,
                args: m.args,
            })
        })
        .reduce(|a, b| if acp_registry::newer(&b.version, &a.version) { b } else { a })
}

/// `def` as it launches on this host: a profile PDT installs runs the path set over it in Settings → Agents, else the
/// program PDT installed (with the registry's arguments); its command stays the placeholder when neither is there.
/// `PDT_ACP_PROGRAM` (`env_program`, PDT-561) wins over all of them, for every ACP agent: tests and scratch daemons
/// point it at a stand-in, or at nothing so no ACP agent is found.
pub fn launch_def(
    data_dir: &FsPath,
    home: Option<&FsPath>,
    s: &HostSettings,
    def: AcpAgentDef,
    env_program: Option<&str>,
) -> AcpAgentDef {
    if let Some(program) = env_program {
        return AcpAgentDef { command: program.to_string(), ..def };
    }
    let Some(registry_id) = def.installer.clone() else { return def };
    let found = installed(data_dir, &registry_id);
    if let Some(path) = s.acp_programs.get(&def.id) {
        let args = found.map(|i| i.args).unwrap_or_else(|| def.args.clone());
        return AcpAgentDef { command: crate::host::expand_home(path, home), args, ..def };
    }
    match found {
        Some(i) => AcpAgentDef { command: i.program.to_string_lossy().into_owned(), args: i.args, ..def },
        None => def,
    }
}

/// Unpacks a zip archive into `dest` (which must not exist yet). Every entry must stay inside it; links are refused;
/// files the archive marks executable stay so (Unix). Runs nothing.
pub fn unpack_zip(bytes: &[u8], dest: &FsPath) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a zip archive: {e}"))?;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.to_string();
        let parts =
            acp_registry::safe_relative(&name).ok_or_else(|| format!("the archive holds a bad path: {name}"))?;
        let path = parts.iter().fold(dest.to_path_buf(), |p, part| p.join(part));
        let mode = entry.unix_mode();
        if mode.is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(format!("the archive holds a link: {name}"));
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        total += entry.size();
        if total > 4 * acp_registry::MAX_ARCHIVE_BYTES {
            return Err("the archive unpacks larger than PDT installs".into());
        }
        let mut out =
            std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let exec = mode.is_some_and(|m| m & 0o111 != 0);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Puts `release`'s archive (`bytes`, already checked) in place as `<root>/<version>/`, and removes older versions.
fn put_in_place(data_dir: &FsPath, release: &RegistryRelease, bytes: &[u8]) -> Result<Installed, String> {
    let root = root(data_dir, &release.id);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let partial = root.join(format!(".partial-{}", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        unpack_zip(bytes, &partial)?;
        let parts = acp_registry::safe_relative(&release.cmd).ok_or("the registry's command is not usable")?;
        if !parts.iter().fold(partial.clone(), |p, part| p.join(part)).is_file() {
            return Err(format!("the archive has no {}", release.cmd));
        }
        let manifest = Manifest {
            version: release.version.clone(),
            cmd: release.cmd.clone(),
            args: release.args.clone(),
            sha256: release.sha256.clone(),
            archive: release.archive.clone(),
        };
        std::fs::write(partial.join(MANIFEST), serde_json::to_vec_pretty(&manifest).unwrap_or_default())
            .map_err(|e| e.to_string())?;
        let target = root.join(&release.version);
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&partial, &target).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&partial);
    }
    result?;
    // Older versions (and leftovers of a failed install) go; one still running (Windows) stays until next time.
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy() != release.version.as_str() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    installed(data_dir, &release.id).ok_or_else(|| "the installed program was not found".into())
}

/// Installs (or updates to) the registry's newest build of `def`'s program; already the newest is a no-op.
pub async fn install(st: &AppState, def: &AcpAgentDef) -> Result<Installed, ApiError> {
    let Some(registry_id) = def.installer.clone() else {
        return Err(ApiError::conflict(codes::NOT_INSTALLABLE, format!("PDT doesn't install {}", def.label)));
    };
    let _guard = st.toolchain_lock.lock().await;
    let doc = st.toolchains.get(acp_registry::REGISTRY_URL, MAX_REGISTRY_BYTES).await.map_err(|e| {
        ApiError::upstream(codes::REGISTRY_UNAVAILABLE, format!("could not read the ACP registry: {e}"))
    })?;
    let doc: serde_json::Value = serde_json::from_slice(&doc)
        .map_err(|e| ApiError::upstream(codes::REGISTRY_UNAVAILABLE, format!("the ACP registry is not JSON: {e}")))?;
    let platform = acp_registry::platform_key().unwrap_or_else(|| std::env::consts::OS.into());
    let release = acp_registry::release(&doc, &registry_id, &platform)
        .map_err(|e| ApiError::conflict(codes::NOT_INSTALLABLE, e.to_string()))?;
    let data_dir = st.data_dir.clone();
    if let Some(i) = installed(&data_dir, &registry_id).filter(|i| i.version == release.version) {
        return Ok(i);
    }
    let bytes =
        st.toolchains.get(&release.archive, acp_registry::MAX_ARCHIVE_BYTES).await.map_err(|e| {
            ApiError::upstream(codes::DOWNLOAD_FAILED, format!("could not download {}: {e}", def.label))
        })?;
    let sha = hex::encode(sha2::Sha256::digest(&bytes));
    if sha != release.sha256 {
        return Err(ApiError::upstream(
            codes::CHECKSUM_MISMATCH,
            format!("the download of {} does not match the registry's sha256, so it was not installed", def.label),
        ));
    }
    let installed = tokio::task::spawn_blocking(move || put_in_place(&data_dir, &release, &bytes))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::internal_as(&format!("could not install {} (PDT's log says why)", def.label), e))?;
    tracing::info!(agent = %def.id, version = %installed.version, "installed an ACP agent's program");
    Ok(installed)
}

fn builtin_installable(id: &str) -> Result<AcpAgentDef, ApiError> {
    pdt_core::acp_agent::builtin_profiles()
        .into_iter()
        .find(|d| d.id == id && d.installer.is_some())
        .or_404("installable ACP agent")
}

/// `POST /api/host/acp-agents/{id}/install`: installs or updates the program; answers with Settings.
pub(crate) async fn post_install(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::host::HostSettingsView>, ApiError> {
    let def = builtin_installable(&id)?;
    install(&st, &def).await?;
    // Whether it's found is out of date.
    let s2 = st.clone();
    tokio::spawn(async move {
        crate::checks::run_host_checks(&s2).await;
    });
    Ok(Json(crate::host::view(&st).await?))
}

/// The sign-in the agent's server keeps, for a flavor PDT can tell (Antigravity's settings file); None otherwise.
pub fn signed_in(flavor: AcpFlavor, home: Option<&FsPath>) -> Option<String> {
    match flavor {
        AcpFlavor::Antigravity => {
            let gemini_home = std::env::var("GEMINI_HOME").ok();
            let path = pdt_core::conversation::antigravity::settings_path(gemini_home.as_deref(), home?);
            pdt_core::conversation::antigravity::signed_in(&std::fs::read(path).ok()?)
        }
        _ => None,
    }
}

/// The ACP sign-in method PDT asks a built-in profile's server for (Sign in); None: PDT signs none in.
fn sign_in_method(flavor: AcpFlavor) -> Option<&'static str> {
    match flavor {
        AcpFlavor::Antigravity => Some(pdt_core::conversation::antigravity::SIGN_IN_METHOD),
        _ => None,
    }
}

/// `POST /api/host/acp-agents/{id}/sign-in`: starts the agent's server and asks it to sign in (ACP `authenticate`).
/// The server opens the browser on this computer and keeps what it gets itself; this answers when the user has
/// finished there (or after about five minutes).
pub(crate) async fn post_sign_in(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::host::HostSettingsView>, ApiError> {
    let def = builtin_installable(&id)?;
    let Some(method) = sign_in_method(def.flavor) else {
        return Err(ApiError::bad_request(codes::NO_SIGN_IN, format!("{} has no sign-in", def.label)));
    };
    let s = st.store.get_host_settings().await?;
    let def = launch_def(&st.data_dir, st.home_dir.as_deref(), &s, def, st.host_env.acp_program.as_deref());
    let program = crate::host::found(&def.command, st.platform.login_path.as_deref())
        .ok_or_else(|| ApiError::conflict(codes::AGENT_NOT_FOUND, format!("{} is not installed", def.label)))?;
    let cwd = st.home_dir.clone().filter(|h| h.is_dir()).unwrap_or_else(std::env::temp_dir);
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&def.args)
        .current_dir(&cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    cmd.envs(def.env.iter().map(|e| (&e.name, &e.value)));
    let child = cmd.spawn().map_err(|e| ApiError::internal(format!("could not start {}: {e}", def.label)))?;
    let outcome = tokio::time::timeout(SIGN_IN_TIMEOUT, authenticate(child, method)).await;
    match outcome {
        Ok(Ok(())) => Ok(Json(crate::host::view(&st).await?)),
        Ok(Err(e)) => Err(ApiError::upstream(codes::SIGN_IN_FAILED, format!("{} could not sign in: {e}", def.label))),
        Err(_) => Err(ApiError::new(
            StatusCode::GATEWAY_TIMEOUT,
            codes::SIGN_IN_TIMEOUT,
            format!("{} was not signed in in time; try again", def.label),
        )),
    }
}

/// `initialize`, then `authenticate` with `method`, on a server's stdio; the child is killed when it is dropped.
async fn authenticate(mut child: tokio::process::Child, method: &str) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err("its pipes were not available".into());
    };
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let calls = [
        ("initialize", pdt_agents::adapter::acp::initialize_params()),
        ("authenticate", serde_json::json!({"methodId": method})),
    ];
    for (n, (name, params)) in calls.into_iter().enumerate() {
        let id = n + 1;
        let line = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": name, "params": params}).to_string();
        stdin.write_all(format!("{line}\n").as_bytes()).await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())?;
        loop {
            let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? else {
                return Err("it stopped".into());
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            if v.get("method").is_some() || v.get("id").and_then(|i| i.as_u64()) != Some(id as u64) {
                continue;
            }
            if let Some(e) = v.get("error") {
                let message = e.get("message").and_then(|m| m.as_str()).unwrap_or("error");
                let details = e.pointer("/data/message").and_then(|m| m.as_str());
                return Err(details.map_or(message.to_string(), |d| format!("{message}: {d}")));
            }
            break;
        }
    }
    let _ = child.start_kill();
    Ok(())
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::post;
    axum::Router::new()
        .route("/host/acp-agents/{id}/install", post(post_install))
        .route("/host/acp-agents/{id}/sign-in", post(post_sign_in))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn zip_of(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, body, mode) in files {
            let opts = zip::write::SimpleFileOptions::default().unix_permissions(*mode);
            w.start_file(*name, opts).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn unpacks_inside_its_folder_keeping_the_exec_bit() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out");
        unpack_zip(&zip_of(&[("agy_acp_server.par", b"#!/bin/sh\n", 0o755), ("lib/x.txt", b"x", 0o644)]), &dest)
            .unwrap();
        assert!(dest.join("lib").join("x.txt").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &FsPath| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dest.join("agy_acp_server.par")), 0o755);
            assert_eq!(mode(&dest.join("lib").join("x.txt")), 0o644);
        }
        let bad = unpack_zip(&zip_of(&[("../evil", b"x", 0o644)]), &tmp.path().join("bad"));
        assert!(bad.unwrap_err().contains("bad path"));
        assert!(!tmp.path().join("evil").exists());
        assert!(unpack_zip(b"not a zip", &tmp.path().join("nz")).is_err());
    }

    fn release(version: &str, bytes: &[u8]) -> RegistryRelease {
        RegistryRelease {
            id: "antigravity-acp".into(),
            version: version.into(),
            archive: "https://dl.google.com/agy-extensions/releases/x.zip".into(),
            sha256: hex::encode(sha2::Sha256::digest(bytes)),
            cmd: "agy_acp_server.par".into(),
            args: vec!["--uid=".into()],
        }
    }

    #[test]
    fn an_install_replaces_older_versions_and_a_set_path_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = zip_of(&[("agy_acp_server.par", b"#!/bin/sh\n", 0o755)]);
        let old = put_in_place(tmp.path(), &release("1.4.5", &zip), &zip).unwrap();
        assert_eq!(old.version, "1.4.5");
        let new = put_in_place(tmp.path(), &release("1.4.10", &zip), &zip).unwrap();
        assert_eq!((new.version.as_str(), new.args.as_slice()), ("1.4.10", &["--uid=".to_string()][..]));
        assert!(!old.program.exists(), "the older version is gone");
        assert_eq!(installed(tmp.path(), "antigravity-acp"), Some(new.clone()));
        // An archive without the program installs nothing.
        let empty = zip_of(&[("other", b"x", 0o644)]);
        assert!(put_in_place(tmp.path(), &release("1.5.0", &empty), &empty).unwrap_err().contains("no agy_acp_server"));
        assert_eq!(installed(tmp.path(), "antigravity-acp"), Some(new.clone()));

        let def = pdt_core::acp_agent::find(&[], "antigravity").unwrap();
        let s = HostSettings::default();
        assert_eq!(launch_def(tmp.path(), None, &s, def.clone(), None).command, new.program.to_string_lossy());
        let home = tmp.path().join("home");
        let s = HostSettings { acp_programs: [("antigravity".into(), "~/agy/server".into())].into(), ..s };
        let set = launch_def(tmp.path(), Some(&home), &s, def.clone(), None);
        assert_eq!(set.command, crate::host::expand_home("~/agy/server", Some(&home)));
        // Not installed: the placeholder, which is never found.
        let none = launch_def(&tmp.path().join("nothing"), None, &HostSettings::default(), def.clone(), None);
        assert_eq!(none.command, def.command);
        // PDT_ACP_PROGRAM wins over the path set and the installed program, and over an agent PDT doesn't install.
        let env = launch_def(tmp.path(), Some(&home), &s, def.clone(), Some("/x/stand-in"));
        assert_eq!((env.command.as_str(), env.args), ("/x/stand-in", def.args.clone()));
        let omp = pdt_core::acp_agent::find(&[], "omp").unwrap();
        assert_eq!(launch_def(tmp.path(), None, &s, omp, Some("/x/stand-in")).command, "/x/stand-in");
    }
}
