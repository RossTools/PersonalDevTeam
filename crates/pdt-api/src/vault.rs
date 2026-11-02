//! Secrets agents can use but never see (PDT-158). The user keeps them in Settings → Secrets or types one into the
//! Inbox when an agent asks (`request_secret`); the values live in the OS keychain (account `secret:<id>`) and in the
//! store's [`Redactor`](pdt_core::secret::Redactor), which replaces them with `[secret:<name>]` in everything PDT
//! stores, shows or logs. Agents use them through `run_with_secrets` and `write_secret_file`, workflows through
//! `{{secret.<name>}}` and `{"$secret": "<name>"}`, filled in at the moment of the call; every use is recorded.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::inbox::{InboxAnswer, InboxItem, InboxKind};
use pdt_core::secret::{self, NewSecret, Secret, SecretPatch, SecretUse, User};
use pdt_core::session::Session;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// The keychain account of a secret's value.
fn account(id: &str) -> String {
    format!("secret:{id}")
}

fn keychain_failed(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "keychain_failed", e.to_string())
}

async fn keychain_get(st: &AppState, s: &Secret) -> Result<Option<String>, String> {
    let (secrets, acct, what) = (st.secrets.clone(), account(&s.id), format!("the secret {}", s.name));
    tokio::task::spawn_blocking(move || secrets.get(&acct).map_err(|e| e.naming(&acct, &what)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

async fn keychain_set(st: &AppState, id: &str, value: &str) -> Result<(), ApiError> {
    let (secrets, acct, value) = (st.secrets.clone(), account(id), value.to_string());
    tokio::task::spawn_blocking(move || secrets.set(&acct, &value))
        .await
        .map_err(ApiError::internal)?
        .map_err(keychain_failed)
}

async fn keychain_delete(st: &AppState, id: &str) -> Result<(), ApiError> {
    let (secrets, acct) = (st.secrets.clone(), account(id));
    tokio::task::spawn_blocking(move || secrets.delete(&acct))
        .await
        .map_err(ApiError::internal)?
        .map_err(keychain_failed)
}

/// Reads every secret's value into the redactor, so they are redacted from the start. pdtd runs it in the background
/// as it starts (the keychain may be slow or ask); a value it can't read now is read when first used.
pub async fn load_known(st: &AppState) {
    let list = match st.store.list_secrets().await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "could not list the secrets");
            return;
        }
    };
    for s in list {
        match keychain_get(st, &s).await {
            Ok(Some(v)) => st.store.redactor().set(&s.name, &v),
            Ok(None) => tracing::warn!(secret = %s.name, "a secret has no value in the keychain"),
            Err(e) => tracing::warn!(secret = %s.name, error = %e, "could not read a secret from the keychain"),
        }
    }
}

/// A secret's value: known already, else from the keychain (and known from then on).
async fn value(st: &AppState, s: &Secret) -> Result<String, String> {
    if let Some(v) = st.store.redactor().value(&s.name) {
        return Ok(v);
    }
    let v = keychain_get(st, s)
        .await?
        .ok_or_else(|| format!("the secret {} has no value: set it in Settings → Secrets", s.name))?;
    st.store.redactor().set(&s.name, &v);
    Ok(v)
}

async fn put_value(st: &AppState, s: &Secret, value: &str) -> Result<(), ApiError> {
    keychain_set(st, &s.id, value).await?;
    st.store.redactor().set(&s.name, value);
    Ok(())
}

/// Who used a secret and how, for the audit list.
pub(crate) struct Usage {
    pub by: String,
    pub how: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
}

/// The values of the secrets `names` (as written), for `who`; each use is recorded. Err names the first secret that
/// is missing or that `who` may not use.
pub(crate) async fn values_for(
    st: &AppState,
    names: &[String],
    who: User<'_>,
    usage: &Usage,
) -> Result<HashMap<String, String>, String> {
    values_checked(st, names, Some(who), usage).await
}

/// [`values_for`] for the user's own use (Settings → MCP servers → Test): no project or persona limit applies.
pub(crate) async fn values_for_user(
    st: &AppState,
    names: &[String],
    usage: &Usage,
) -> Result<HashMap<String, String>, String> {
    values_checked(st, names, None, usage).await
}

async fn values_checked(
    st: &AppState,
    names: &[String],
    who: Option<User<'_>>,
    usage: &Usage,
) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    for n in names {
        if out.contains_key(n) {
            continue;
        }
        let s = st.store.secret_by_name(n).await.map_err(|e| e.to_string())?.ok_or_else(|| {
            format!("no secret is called {n}: the user adds it in Settings → Secrets (agents ask with request_secret)")
        })?;
        if who.is_some_and(|w| !s.allows(w)) {
            return Err(format!(
                "the secret {} may not be used here: the user can allow it in Settings → Secrets",
                s.name
            ));
        }
        let v = value(st, &s).await?;
        let u = SecretUse {
            id: 0,
            secret_id: s.id.clone(),
            at: now_ms(),
            by: usage.by.clone(),
            how: usage.how.clone(),
            session_id: usage.session_id.clone(),
            run_id: usage.run_id.clone(),
        };
        if let Err(e) = st.store.record_secret_use(&u).await {
            tracing::warn!(secret = %s.name, error = %e, "could not record a secret's use");
        }
        out.insert(n.clone(), v);
    }
    Ok(out)
}

fn session_user(s: &Session) -> User<'_> {
    User::Agent { project: &s.project_id, persona: s.persona.as_ref().map(|p| p.id.as_str()) }
}

fn session_usage(s: &Session, how: &str) -> Usage {
    Usage { by: s.title.clone(), how: how.into(), session_id: Some(s.id.clone()), run_id: None }
}

// ---- the Settings page ----

fn invalid(m: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", m)
}

async fn load(st: &AppState, id: &str) -> Result<Secret, ApiError> {
    st.store.get_secret(id).await?.ok_or_else(|| ApiError::not_found("secret"))
}

/// Project and persona ids, each checked to exist.
async fn access(
    st: &AppState,
    projects: Vec<String>,
    personas: Vec<String>,
) -> Result<(Vec<String>, Vec<String>), ApiError> {
    let clean = |l: Vec<String>| {
        let mut l: Vec<String> = l.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        l.sort();
        l.dedup();
        l
    };
    let (projects, personas) = (clean(projects), clean(personas));
    for p in &projects {
        if st.store.get_project(p).await?.is_none() {
            return Err(invalid(format!("no project has the id {p}")));
        }
    }
    for p in &personas {
        if st.store.get_persona(p).await?.is_none() {
            return Err(invalid(format!("no persona has the id {p}")));
        }
    }
    Ok((projects, personas))
}

/// `GET /api/secrets`: names, access and last use, never values.
pub(crate) async fn list(State(st): State<AppState>) -> Result<Json<Vec<Secret>>, ApiError> {
    Ok(Json(st.store.list_secrets().await?))
}

/// `GET /api/secrets/store`: where values are kept (the keychain, or the data folder's file on Linux without one).
pub(crate) async fn store_info(State(st): State<AppState>) -> Json<crate::secrets::SecretStoreInfo> {
    Json(st.secret_store.clone())
}

/// `POST /api/secrets`.
pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewSecret>, JsonRejection>,
) -> Result<(StatusCode, Json<Secret>), ApiError> {
    let Json(n) = payload?;
    let s = new_secret(&st, n).await?;
    Ok((StatusCode::CREATED, Json(s)))
}

async fn new_secret(st: &AppState, n: NewSecret) -> Result<Secret, ApiError> {
    let name = secret::valid_name(&n.name).map_err(invalid)?;
    secret::valid_value(&n.value).map_err(invalid)?;
    let description = secret::valid_description(n.description.as_deref().unwrap_or_default()).map_err(invalid)?;
    let (projects, personas) = access(st, n.projects.unwrap_or_default(), n.personas.unwrap_or_default()).await?;
    if st.store.secret_by_name(&name).await?.is_some() {
        return Err(exists(&name));
    }
    let now = now_ms();
    let s = Secret {
        id: uuid::Uuid::now_v7().to_string(),
        name,
        description,
        projects,
        personas,
        created_at: now,
        updated_at: now,
        last_used_at: None,
        last_used_by: None,
    };
    // The value first: a row is never there without one.
    put_value(st, &s, &n.value).await?;
    if let Err(e) = st.store.insert_secret(&s).await {
        let _ = keychain_delete(st, &s.id).await;
        st.store.redactor().remove(&s.name);
        return Err(match e {
            pdt_store::StoreError::Conflict(_) => exists(&s.name),
            other => other.into(),
        });
    }
    Ok(s)
}

fn exists(name: &str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "secret_exists", format!("a secret is already called {name}"))
}

/// `PATCH /api/secrets/{id}`: its description, who may use it, or a new value.
pub(crate) async fn patch(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<SecretPatch>, JsonRejection>,
) -> Result<Json<Secret>, ApiError> {
    let Json(p) = payload?;
    let mut s = load(&st, &id).await?;
    if let Some(d) = &p.description {
        s.description = secret::valid_description(d).map_err(invalid)?;
    }
    if p.projects.is_some() || p.personas.is_some() {
        let projects = p.projects.unwrap_or_else(|| s.projects.clone());
        let personas = p.personas.unwrap_or_else(|| s.personas.clone());
        (s.projects, s.personas) = access(&st, projects, personas).await?;
    }
    if let Some(v) = &p.value {
        secret::valid_value(v).map_err(invalid)?;
        put_value(&st, &s, v).await?;
    }
    s.updated_at = now_ms();
    st.store.update_secret(&s).await?;
    Ok(Json(s))
}

/// `DELETE /api/secrets/{id}`: the value leaves the keychain, the row and its uses go.
pub(crate) async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let s = load(&st, &id).await?;
    keychain_delete(&st, &s.id).await?;
    st.store.delete_secret(&s.id).await?;
    st.store.redactor().remove(&s.name);
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/secrets/{id}/uses`: the newest 200 uses.
pub(crate) async fn uses(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<SecretUse>>, ApiError> {
    let s = load(&st, &id).await?;
    Ok(Json(st.store.secret_uses(&s.id, 200).await?))
}

pub(crate) fn router() -> axum::Router<AppState> {
    use axum::routing::get;
    axum::Router::new()
        .route("/secrets", get(list).post(create))
        .route("/secrets/store", get(store_info))
        .route("/secrets/{id}", axum::routing::patch(patch).delete(delete))
        .route("/secrets/{id}/uses", get(uses))
}

// ---- pdt-mcp's tools ----

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RequestSecret {
    /// The secret's name: letters, digits, `_` and `-` (`NPM_TOKEN`).
    pub name: String,
    /// What it is for, for the user: one sentence.
    pub purpose: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RunWithSecrets {
    /// A command line, run by the shell (`sh -c` on macOS and Linux, `cmd /C` on Windows). Secrets go in `env`,
    /// never in the command.
    pub command: String,
    /// Environment variables for it: `"secret:<name>"` is that secret's value, any other value is used as it is.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// The folder it runs in: the session's when left out; a relative path is under the session's.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Seconds before it is stopped (default 120, at most 1800).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WriteSecretFile {
    /// The file to write (made or replaced): a relative path is under the session's folder.
    pub path: String,
    /// Its content, with `{{secret.<name>}}` where a secret's value goes (`NPM_TOKEN={{secret.NPM_TOKEN}}`).
    pub template: String,
}

/// `list_secrets`: the secrets this session may use.
pub(crate) async fn list_tool(st: &AppState, s: &Session) -> Result<Value, String> {
    let all = st.store.list_secrets().await.map_err(|e| e.to_string())?;
    let mine: Vec<Value> = all
        .iter()
        .filter(|x| x.allows(session_user(s)))
        .map(|x| json!({"name": x.name, "description": x.description}))
        .collect();
    Ok(json!({"secrets": mine,
              "use": "run_with_secrets (env: {\"VAR\": \"secret:<name>\"}) or write_secret_file ({{secret.<name>}}); \
                      request_secret asks the user for one that is missing."}))
}

/// `request_secret`: one the session may use already is `stored`; otherwise the user is asked for it in the Inbox.
pub(crate) async fn request_tool(st: &AppState, s: &Session, a: RequestSecret) -> Result<Value, String> {
    let name = secret::valid_name(&a.name)?;
    let purpose = a.purpose.trim();
    if purpose.is_empty() {
        return Err("say what the secret is for (purpose): the user reads it".into());
    }
    if let Some(x) = st.store.secret_by_name(&name).await.map_err(|e| e.to_string())? {
        if x.allows(session_user(s)) {
            return Ok(json!({"name": x.name, "stored": true}));
        }
        return Err(format!(
            "the secret {} exists but this session may not use it: the user can allow it in Settings → Secrets",
            x.name
        ));
    }
    let title = format!("Secret {name}: {purpose}");
    let body = json!({"name": name, "purpose": purpose, "sessionTitle": s.title});
    let item = crate::workflow::agent::ask(st, s, InboxKind::Secret, title, body).await?;
    Ok(json!({"name": name, "requested": true, "inboxItemId": item.id,
              "message": "The user enters the value in PDT's Inbox, never in this chat. You will get a message once it \
                          is stored (never the value); end your turn now."}))
}

/// The Inbox's answer to a `request_secret`: the value goes to the keychain, the agent hears only that it is stored.
pub(crate) async fn answer_request(st: &AppState, item: &InboxItem, answer: &InboxAnswer) -> Result<Value, String> {
    let v = answer.secret.as_deref().ok_or("enter the secret's value")?;
    secret::valid_value(v)?;
    let name = item.body.get("name").and_then(Value::as_str).ok_or("the request names no secret")?.to_string();
    let purpose = item.body.get("purpose").and_then(Value::as_str).unwrap_or_default();
    // Its session must still be there to tell.
    crate::workflow::agent::asker(st, item).await?;
    let api = |e: ApiError| e.message().to_string();
    let s = match st.store.secret_by_name(&name).await.map_err(|e| e.to_string())? {
        // Added meanwhile: the user's entry replaces its value.
        Some(s) => {
            put_value(st, &s, v).await.map_err(api)?;
            s
        }
        None => {
            let n = NewSecret {
                name: name.clone(),
                description: Some(purpose.chars().take(500).collect()),
                // The asking project's agents and workflows; the user widens it in Settings → Secrets.
                projects: Some(item.project_id.iter().cloned().collect()),
                personas: None,
                value: v.to_string(),
            };
            new_secret(st, n).await.map_err(api)?
        }
    };
    let text = format!(
        "The secret {0} is stored. Use it with run_with_secrets (env: {{\"VAR\": \"secret:{0}\"}}) or write_secret_file \
         ({{{{secret.{0}}}}}); its value is never shown to you.",
        s.name
    );
    crate::workflow::agent::reply(st, item, &text, json!({"name": s.name, "stored": true})).await?;
    Ok(json!({"name": s.name, "stored": true}))
}

/// A folder or file a secret tool uses (PDT-404): inside the session's folder ([`secret::checkout_path`]), and what of
/// it is already on disk resolves inside it too and into no `.git` folder, so a symlink can't lead it elsewhere.
fn session_path(s: &Session, given: Option<&str>) -> Result<PathBuf, String> {
    let base = PathBuf::from(&s.cwd);
    let path = secret::checkout_path(&base, given)?;
    let root = std::fs::canonicalize(&base).map_err(|e| format!("this session's folder {}: {e}", base.display()))?;
    let mut on_disk = path.as_path();
    while std::fs::symlink_metadata(on_disk).is_err() {
        on_disk = on_disk.parent().ok_or_else(|| format!("{} is not in this session's folder", path.display()))?;
    }
    inside(&root, on_disk, &path)?;
    Ok(path)
}

/// `on_disk` (`path` or a folder it is in) resolves inside `root` and into no `.git` folder.
fn inside(root: &std::path::Path, on_disk: &std::path::Path, path: &std::path::Path) -> Result<(), String> {
    let real = std::fs::canonicalize(on_disk).map_err(|e| format!("{}: {e}", on_disk.display()))?;
    let rel = real.strip_prefix(root).map_err(|_| format!("{} leads outside this session's folder", path.display()))?;
    if rel.components().any(|c| c.as_os_str().eq_ignore_ascii_case(".git")) {
        return Err(format!("{} leads into a .git folder: secret tools never write there", path.display()));
    }
    Ok(())
}

/// How much of stdout and of stderr `run_with_secrets` gives back.
const OUTPUT_CAP: usize = 64 * 1024;
const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 1800;

/// What `run_with_secrets` refuses before it asks the user: the secrets it names and the folder it runs in.
pub(crate) fn check_run(s: &Session, a: &RunWithSecrets) -> Result<(Vec<String>, PathBuf), String> {
    if a.command.trim().is_empty() {
        return Err("the command is empty".into());
    }
    if !secret::template_names(&a.command).is_empty() {
        return Err("secrets go in env (\"VAR\": \"secret:<name>\"), never in the command line".into());
    }
    let names: Vec<String> = a.env.values().filter_map(|v| secret::env_ref(v)).map(String::from).collect();
    // It is for using secrets, not a way round the agent's own shell and its sandbox (PDT-404).
    if names.is_empty() {
        return Err("name a secret in env (\"VAR\": \"secret:<name>\"): run_with_secrets runs only commands that use \
                    one; run anything else with your own shell"
            .into());
    }
    let dir = session_path(s, a.cwd.as_deref())?;
    if !dir.is_dir() {
        return Err(format!("{} is not a folder", dir.display()));
    }
    Ok((names, dir))
}

/// `run_with_secrets`: the command with the secrets in its environment; what it printed comes back redacted.
pub(crate) async fn run_tool(st: &AppState, s: &Session, a: RunWithSecrets) -> Result<Value, String> {
    let (names, dir) = check_run(s, &a)?;
    let values = values_for(st, &names, session_user(s), &session_usage(s, "run_with_secrets")).await?;
    let env: BTreeMap<String, String> = a
        .env
        .iter()
        .map(|(k, v)| (k.clone(), secret::env_ref(v).and_then(|n| values.get(n)).cloned().unwrap_or_else(|| v.clone())))
        .collect();
    let limit =
        std::time::Duration::from_secs(a.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).clamp(1, MAX_TIMEOUT_SECS));
    let out = run_shell(&a.command, &env, &dir, limit).await?;
    let r = st.store.redactor();
    Ok(json!({
        "exitCode": out.code,
        "timedOut": out.timed_out,
        "stdout": r.string(&out.stdout),
        "stderr": r.string(&out.stderr),
        "truncated": out.truncated,
        "secrets": values.keys().collect::<Vec<_>>(),
    }))
}

struct ShellOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    truncated: bool,
    timed_out: bool,
}

async fn run_shell(
    command: &str,
    env: &BTreeMap<String, String>,
    dir: &std::path::Path,
    limit: std::time::Duration,
) -> Result<ShellOutput, String> {
    use crate::workflow::nodes::command::{TreeGuard, read_capped};
    use std::process::Stdio;
    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    cmd.current_dir(dir)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().map_err(|e| format!("could not run the command: {e}"))?;
    let mut guard = TreeGuard(child.id());
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let all = async { tokio::join!(read_capped(stdout, OUTPUT_CAP), read_capped(stderr, OUTPUT_CAP), child.wait()) };
    match tokio::time::timeout(limit, all).await {
        Ok((out, err, status)) => {
            let status = status.map_err(|e| e.to_string())?;
            guard.0 = None;
            Ok(ShellOutput {
                code: status.code(),
                stdout: String::from_utf8_lossy(&out.0).into_owned(),
                stderr: String::from_utf8_lossy(&err.0).into_owned(),
                truncated: out.1 || err.1,
                timed_out: false,
            })
        }
        // Dropping the guard kills the process tree.
        Err(_) => Ok(ShellOutput {
            code: None,
            stdout: String::new(),
            stderr: format!("stopped after {} s", limit.as_secs()),
            truncated: false,
            timed_out: true,
        }),
    }
}

/// What `write_secret_file` refuses before it asks the user: the secrets it names and the file it writes.
pub(crate) fn check_write(s: &Session, a: &WriteSecretFile) -> Result<(Vec<String>, PathBuf), String> {
    if a.path.trim().is_empty() {
        return Err("the path is empty".into());
    }
    let names = secret::template_names(&a.template);
    // It is for files with secrets in, not a way round the agent's own edits and its sandbox (PDT-404).
    if names.is_empty() {
        return Err("name a secret in the template ({{secret.<name>}}): write_secret_file writes only files with one \
                    in; write anything else with your own tools"
            .into());
    }
    Ok((names, session_path(s, Some(&a.path))?))
}

/// `write_secret_file`: the template with its secrets filled in, written to the file (owner-only on macOS and
/// Linux); nothing of the content comes back.
pub(crate) async fn write_file_tool(st: &AppState, s: &Session, a: WriteSecretFile) -> Result<Value, String> {
    let (names, path) = check_write(s, &a)?;
    let values = values_for(st, &names, session_user(s), &session_usage(s, "write_secret_file")).await?;
    let text = secret::fill_template(&a.template, &values);
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| format!("could not make {}: {e}", dir.display()))?;
        // Checked again now the folders are there.
        let root = std::fs::canonicalize(&s.cwd).map_err(|e| format!("this session's folder {}: {e}", s.cwd))?;
        inside(&root, dir, &path)?;
    }
    write_private(&path, text.as_bytes()).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(json!({"path": path.display().to_string(), "bytes": text.len(), "secrets": names}))
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // Never through a symlink (PDT-404): it would write wherever that points.
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        // An existing file keeps its mode on open: make it owner-only too.
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(std::io::Error::other("it is a symlink"));
        }
        std::fs::write(path, bytes)
    }
}

// ---- workflows ----

/// A step's input with its secrets filled in, just before its node runs (the stored input keeps the references).
/// Only the references the node's `config` itself holds count: they are filled in as the config is resolved again
/// against the run's context, so a reference that comes in as data (a trigger's payload, a ticket's text, an earlier
/// step's output) is neither filled nor an error (`secret::resolve_with_secrets`). `{{secret.<name>}}` must name a
/// secret; `{"$secret": "<name>"}` naming none is left for the node (a connection, for an integration's nodes). Nodes that
/// send text to an agent take no secrets. Without any, `input` is used as it is.
pub(crate) async fn fill_step(
    cx: &crate::workflow::catalog::StepCx<'_>,
    config: &Value,
    input: Value,
) -> Result<Value, String> {
    let named = secret::names_in(config);
    if named.templates.is_empty() && named.refs.is_empty() {
        return Ok(input);
    }
    let st = cx.st;
    let mut names = named.templates.clone();
    for r in &named.refs {
        if st.store.secret_by_name(r).await.map_err(|e| e.to_string())?.is_some() {
            names.push(r.clone());
        }
    }
    if names.is_empty() {
        return Ok(input);
    }
    let kind = &cx.step.kind;
    let to_agent = kind == "session.send"
        || st.registry().catalog().spec(kind).is_some_and(|s| s.category == crate::workflow::catalog::Category::Agent);
    if to_agent {
        return Err(format!("a {kind} step sends its text to an agent: secrets can't go in it"));
    }
    let host;
    let project = match cx.run.project_id.as_deref().or(cx.workflow.project_id.as_deref()) {
        Some(p) => p,
        None => {
            host = st.store.host_project().await.map_err(|e| e.to_string())?.map(|p| p.id).unwrap_or_default();
            host.as_str()
        }
    };
    let usage = Usage {
        by: format!("workflow {}", cx.run.workflow_name),
        how: kind.clone(),
        session_id: None,
        run_id: Some(cx.run.id.clone()),
    };
    let values = values_for(st, &names, User::Workflow { project }, &usage).await?;
    Ok(secret::resolve_with_secrets(config, cx.ctx, &["event"], &values))
}
