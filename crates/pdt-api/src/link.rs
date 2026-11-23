//! The project link (PDT-631): how a project's owner instance and its remote instances talk. Pairing (the remote
//! instance sends the code its owner showed, and the two swap Ed25519 public keys for that one project), signed
//! requests either way (each end keeps its private key for the link in its keychain and checks the other's
//! signature), the protocol version, and the owner's heartbeats with the facts the remote instance reports.
//!
//! Every link request also passes the identity guard: over Tailscale it must carry the host owner's login. The
//! signature then says which link, and so which project, it is for: a remote instance takes commands for a project
//! only from that project's owner. Pure rules (codes, the signed text, addresses, states) are in pdt-core `link`.

use crate::error::{ApiError, codes};
use crate::extract::ApiJson;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, Method, StatusCode};
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use pdt_core::link::{
    self, HeartbeatReply, InstanceFacts, InstanceHello, PairReply, PairRequest, Side, SignedRequest, headers as h,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// The host value holding this instance's id, made on first use.
const INSTANCE_ID_KEY: &str = "instance.id";

/// How long one link call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a heartbeat that asks for fresh facts waits for the agent checks before it answers with the last ones,
/// well inside [`CALL_TIMEOUT`].
const REFRESH_WAIT: Duration = Duration::from_secs(6);

/// How long after pairing the owner sends its first heartbeat.
const FIRST_HEARTBEAT_AFTER: Duration = Duration::from_secs(2);

/// The link paths, under `/api`.
pub const PAIR_PATH: &str = "/api/link/pair";
pub const HEARTBEAT_PATH: &str = "/api/link/heartbeat";
pub const REVOKE_PATH: &str = "/api/link/revoke";
pub const LEAVE_PATH: &str = "/api/link/leave";
/// A remote instance asks its owner whether it still has the link (PDT-702): while the owner's heartbeats don't come.
pub const STATUS_PATH: &str = "/api/link/status";
/// The owner asks a joined instance to update (PDT-637); it is taken whatever protocol the owner speaks.
pub const UPDATE_PATH: &str = "/api/link/update";

/// A pairing code a project's owner is showing.
#[derive(Debug, Clone)]
pub(crate) struct Pending {
    pub(crate) code: String,
    pub(crate) expires_at: i64,
}

/// What the link keeps in memory: the pairing codes on show, the nonces seen lately, and its HTTP client.
pub struct LinkRuntime {
    /// By project id: one code per project at a time.
    pairing: Mutex<HashMap<String, Pending>>,
    /// Wrong codes since the codes on show were made; at [`link::PAIRING_MAX_FAILURES`] they all stop working.
    failures: AtomicU32,
    /// Nonces of signed requests taken, with when they arrived, so none is taken twice.
    nonces: Mutex<HashMap<String, i64>>,
    /// The protocol this instance speaks: [`link::PROTOCOL`] (a test sets another to stand for another version).
    protocol: AtomicU32,
    /// Every call to the other end fails as unreachable (tests: the link dropped).
    offline: std::sync::atomic::AtomicBool,
    /// Added before each call to the other end, in ms (tests: a tailnet's round trip).
    latency_ms: AtomicU64,
    /// Calls made to other instances since this process started, by path (tests count round trips).
    calls: Mutex<HashMap<String, u64>>,
    client: reqwest::Client,
}

impl Default for LinkRuntime {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .timeout(CALL_TIMEOUT)
            .connect_timeout(CALL_TIMEOUT)
            .user_agent(pdt_core::brand::user_agent(""))
            .build()
            .unwrap_or_default();
        Self {
            pairing: Default::default(),
            failures: AtomicU32::new(0),
            nonces: Default::default(),
            protocol: AtomicU32::new(link::PROTOCOL),
            offline: Default::default(),
            latency_ms: AtomicU64::new(0),
            calls: Default::default(),
            client,
        }
    }
}

impl LinkRuntime {
    pub fn protocol(&self) -> u32 {
        self.protocol.load(Ordering::SeqCst)
    }

    /// Makes this instance speak another protocol (tests: an instance of another version).
    pub fn set_protocol(&self, p: u32) {
        self.protocol.store(p, Ordering::SeqCst);
    }

    /// Drops (true) or restores the link from this end: while dropped, every call to another instance fails as
    /// unreachable (tests).
    pub fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::SeqCst);
    }

    /// Adds `latency` before every call to another instance (tests: a link slower than loopback).
    pub fn set_latency(&self, latency: Duration) {
        self.latency_ms.store(latency.as_millis() as u64, Ordering::SeqCst);
    }

    /// How many calls this instance has made to others on `path` (`/api/link/…`).
    pub fn calls_to(&self, path: &str) -> u64 {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).get(path).copied().unwrap_or(0)
    }

    /// A new code for `project_id`, replacing the one it had.
    pub(crate) fn new_code(&self, project_id: &str, now: i64) -> Pending {
        let mut p = self.pairing.lock().unwrap_or_else(|e| e.into_inner());
        p.retain(|_, c| c.expires_at > now);
        if p.is_empty() {
            self.failures.store(0, Ordering::SeqCst);
        }
        let code = Pending { code: link::pairing_code(&random_bytes()), expires_at: now + link::PAIRING_CODE_TTL_MS };
        p.insert(project_id.to_string(), code.clone());
        code
    }

    pub(crate) fn code_for(&self, project_id: &str, now: i64) -> Option<Pending> {
        let p = self.pairing.lock().unwrap_or_else(|e| e.into_inner());
        p.get(project_id).filter(|c| c.expires_at > now).cloned()
    }

    pub(crate) fn cancel_code(&self, project_id: &str) {
        self.pairing.lock().unwrap_or_else(|e| e.into_inner()).remove(project_id);
    }

    /// Takes the code `code` (once): the project it is for. A wrong one counts against every code on show.
    fn take_code(&self, code: &str, now: i64) -> Option<String> {
        let mut p = self.pairing.lock().unwrap_or_else(|e| e.into_inner());
        p.retain(|_, c| c.expires_at > now);
        let found = p.iter().find(|(_, c)| c.code == code).map(|(id, _)| id.clone());
        match found {
            Some(id) => {
                p.remove(&id);
                Some(id)
            }
            None => {
                if self.failures.fetch_add(1, Ordering::SeqCst) + 1 >= link::PAIRING_MAX_FAILURES {
                    p.clear();
                }
                None
            }
        }
    }

    /// Records `nonce`; false when it was seen already.
    fn first_use(&self, nonce: &str, now: i64) -> bool {
        let mut n = self.nonces.lock().unwrap_or_else(|e| e.into_inner());
        n.retain(|_, at| now - *at <= 2 * link::MAX_SKEW_MS);
        n.insert(nonce.to_string(), now).is_none()
    }
}

/// 32 random bytes: the OS's randomness (through UUID v4s), hashed together.
fn random_bytes() -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    for _ in 0..3 {
        hasher.update(uuid::Uuid::new_v4().as_bytes());
    }
    hasher.finalize().into()
}

/// A new key for one end of one link.
pub(crate) fn new_key() -> SigningKey {
    SigningKey::from_bytes(&random_bytes())
}

pub(crate) fn public_key(k: &SigningKey) -> String {
    B64.encode(k.verifying_key().to_bytes())
}

fn parse_public_key(s: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = B64.decode(s.trim()).ok()?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

/// The keychain account of this end's private key for `link_id`. Link ids are unique, so two instances on one
/// machine never share one.
fn key_account(link_id: &str) -> String {
    format!("project-link:{link_id}")
}

pub(crate) async fn save_key(st: &AppState, link_id: &str, key: &SigningKey) -> Result<(), ApiError> {
    let (secrets, account, value) = (st.secrets.clone(), key_account(link_id), B64.encode(key.to_bytes()));
    tokio::task::spawn_blocking(move || secrets.set(&account, &value))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::keychain)
}

pub(crate) async fn load_key(st: &AppState, link_id: &str) -> Result<SigningKey, ApiError> {
    let (secrets, account) = (st.secrets.clone(), key_account(link_id));
    let raw = tokio::task::spawn_blocking(move || secrets.get(&account))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::keychain)?;
    let bytes: Option<[u8; 32]> = raw.and_then(|r| B64.decode(r.trim()).ok()).and_then(|b| b.try_into().ok());
    match bytes {
        Some(b) => Ok(SigningKey::from_bytes(&b)),
        None => Err(ApiError::keychain("this instance's key for the project link is missing from the keychain")),
    }
}

/// This end's key for `link_id`, to tell the other end it is removed; None (logged) when it can't be read.
pub(crate) async fn load_key_for_revoke(st: &AppState, link_id: &str) -> Option<SigningKey> {
    load_key(st, link_id).await.inspect_err(|e| tracing::warn!(error = %e, "could not read a link's key")).ok()
}

/// Forgets this end's key for `link_id` (revoking the link here). A failure is logged: the link's row is gone
/// anyway, so the key no longer signs anything anyone accepts.
pub(crate) async fn delete_key(st: &AppState, link_id: &str) {
    let (secrets, account) = (st.secrets.clone(), key_account(link_id));
    match tokio::task::spawn_blocking(move || secrets.delete(&account)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(error = %e, link = link_id, "could not delete a project link's key"),
        Err(e) => tracing::warn!(error = %e, link = link_id, "could not delete a project link's key"),
    }
}

/// This instance's id: made once, never changed.
pub(crate) async fn instance_id(st: &AppState) -> Result<String, ApiError> {
    if let Some(id) = st.store.get_host_value(INSTANCE_ID_KEY).await?.and_then(|v| v.as_str().map(String::from)) {
        return Ok(id);
    }
    let _guard = st.settings_lock.lock().await;
    if let Some(id) = st.store.get_host_value(INSTANCE_ID_KEY).await?.and_then(|v| v.as_str().map(String::from)) {
        return Ok(id);
    }
    let id = uuid::Uuid::now_v7().to_string();
    st.store.put_host_value(INSTANCE_ID_KEY, &Value::String(id.clone())).await?;
    Ok(id)
}

/// The machine's host name (for the name an instance goes by before the user picks one).
pub(crate) fn machine_host_name() -> String {
    if let Some(n) = std::env::var_os("COMPUTERNAME").filter(|n| !n.is_empty()) {
        return n.to_string_lossy().into_owned();
    }
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for its length; gethostname writes at most that many bytes.
        let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if r == 0 {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
    }
    String::new()
}

/// The name this instance goes by (Settings → This host).
pub(crate) async fn instance_name(st: &AppState) -> Result<String, ApiError> {
    Ok(st.store.get_host_settings().await?.instance_name_or(&machine_host_name()))
}

/// This instance's own address on the tailnet (remote access), where another instance reaches it.
pub(crate) fn own_url(st: &AppState) -> Option<String> {
    let r = st.remote.lock().unwrap_or_else(|e| e.into_inner()).clone();
    r.active.then_some(r.url).flatten().map(|u| u.trim_end_matches('/').to_string())
}

/// This instance as the other end of a link sees it.
pub(crate) async fn hello(st: &AppState) -> Result<InstanceHello, ApiError> {
    Ok(InstanceHello {
        instance_id: instance_id(st).await?,
        name: instance_name(st).await?,
        version: pdt_core::BUILD_VERSION.to_string(),
        protocol: st.link.protocol(),
    })
}

/// The headers that sign a link request (`POST path` with `body`) for `link_id` and the owner's project `project`.
pub fn sign_headers(
    key: &SigningKey,
    path: &str,
    link_id: &str,
    project: &str,
    body: &[u8],
    protocol: u32,
    now: i64,
) -> Vec<(&'static str, String)> {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let text = link::signed_text(&SignedRequest {
        method: "POST",
        path,
        link: link_id,
        project,
        time: now,
        nonce: &nonce,
        body,
    });
    let sig = B64.encode(key.sign(text.as_bytes()).to_bytes());
    vec![
        (h::LINK, link_id.to_string()),
        (h::PROJECT, project.to_string()),
        (h::PROTOCOL, protocol.to_string()),
        (h::TIME, now.to_string()),
        (h::NONCE, nonce),
        (h::SIGNATURE, sig),
    ]
}

/// Why a call to the other end failed.
#[derive(Debug)]
pub(crate) enum PeerError {
    /// It could not be reached, or didn't answer in time.
    Unreachable(String),
    /// It answered with an error: its `{code, message, data}`.
    Refused { status: u16, code: String, message: String, data: Option<Value> },
}

impl PeerError {
    /// The error for this instance's own API, naming `who` (the other instance).
    pub(crate) fn into_api(self, who: &str) -> ApiError {
        match self {
            PeerError::Unreachable(e) => {
                ApiError::upstream(codes::LINK_UNREACHABLE, format!("couldn't reach {who}: {e}"))
            }
            PeerError::Refused { code, message, .. } if code == codes::PAIRING_CODE => {
                ApiError::forbidden(codes::PAIRING_CODE, message)
            }
            PeerError::Refused { code, message, .. } if code == codes::LINK_INCOMPATIBLE => {
                ApiError::conflict(codes::LINK_INCOMPATIBLE, message)
            }
            PeerError::Refused { code, message, .. } if code == codes::REMOTE_PROJECT => {
                ApiError::conflict(codes::REMOTE_PROJECT, message)
            }
            PeerError::Refused { status, message, .. } => {
                ApiError::upstream(codes::LINK_UNREACHABLE, format!("{who} refused it ({status}): {message}"))
            }
        }
    }

    /// The error for this instance's own API as the other instance gave it (its status, code and message), for a call
    /// made on the user's behalf (Files, Changes) whose errors mean the same on both ends: a file changed on disk, a
    /// path outside the checkout. A code this instance doesn't know, or an unreachable instance, is [`into_api`].
    ///
    /// [`into_api`]: PeerError::into_api
    pub(crate) fn passed_on(self, who: &str) -> ApiError {
        if let PeerError::Refused { status, code, message, data } = &self
            && let Some(code) = PASSED_ON.iter().copied().find(|c| *c == code)
            && let Ok(status) = StatusCode::from_u16(*status)
            && status.is_client_error()
        {
            let e = ApiError::new(status, code, message.clone());
            return match data {
                Some(d) => e.with_data(d.clone()),
                None => e,
            };
        }
        self.into_api(who)
    }

    pub(crate) fn text(&self) -> String {
        match self {
            PeerError::Unreachable(e) => format!("unreachable: {e}"),
            PeerError::Refused { message, .. } => message.clone(),
        }
    }
}

/// The codes [`PeerError::passed_on`] passes on: what the Files and Changes views act on.
const PASSED_ON: &[&str] = &[
    codes::BAD_PATTERN,
    codes::FILE_CHANGED,
    codes::FILE_EXISTS,
    codes::INVALID_INPUT,
    codes::NOT_EDITABLE,
    codes::NOT_FOUND,
    codes::PATH_OUTSIDE,
    codes::PERMISSION_DENIED,
    codes::TOO_LARGE,
    codes::WORKTREE_MISSING,
    codes::WORKTREE_PATH_UNSAFE,
];

/// POSTs `body` to `base_url` + `path`, signed for `link_id` when `key` is given, and reads the JSON answer
/// (`Value::Null` for an empty one).
pub(crate) async fn post<T: DeserializeOwned>(
    st: &AppState,
    base_url: &str,
    path: &str,
    signed: Option<(&SigningKey, &str, &str)>,
    body: &impl Serialize,
) -> Result<T, PeerError> {
    post_within(st, CALL_TIMEOUT, base_url, path, signed, body).await
}

/// [`post`], for a call that may take up to `timeout` (an agent starting, a fetch, a tool call waiting on the user).
pub(crate) async fn post_within<T: DeserializeOwned>(
    st: &AppState,
    timeout: Duration,
    base_url: &str,
    path: &str,
    signed: Option<(&SigningKey, &str, &str)>,
    body: &impl Serialize,
) -> Result<T, PeerError> {
    let res = send(st, timeout, base_url, path, signed, body).await?;
    let status = res.status();
    let text = res.text().await.map_err(|e| PeerError::Unreachable(short_error(&e)))?;
    let text = if text.trim().is_empty() { "null" } else { &text };
    serde_json::from_str(text).map_err(|e| PeerError::Refused {
        status: status.as_u16(),
        code: String::new(),
        message: format!("an answer this instance can't read: {e}"),
        data: None,
    })
}

/// POSTs `body` signed (as [`post_within`]) and hands back the other end's successful response as it is, unread: a
/// download streams through without being held here.
pub(crate) async fn send(
    st: &AppState,
    timeout: Duration,
    base_url: &str,
    path: &str,
    signed: Option<(&SigningKey, &str, &str)>,
    body: &impl Serialize,
) -> Result<reqwest::Response, PeerError> {
    if st.link.offline.load(Ordering::SeqCst) {
        return Err(PeerError::Unreachable("the link is down".into()));
    }
    *st.link.calls.lock().unwrap_or_else(|e| e.into_inner()).entry(path.to_string()).or_default() += 1;
    let latency = st.link.latency_ms.load(Ordering::SeqCst);
    if latency > 0 {
        tokio::time::sleep(Duration::from_millis(latency)).await;
    }
    let bytes = serde_json::to_vec(body).expect("a link body serializes");
    let mut req = st
        .link
        .client
        .post(format!("{base_url}{path}"))
        .timeout(timeout)
        .header("content-type", "application/json")
        .body(bytes.clone());
    if let Some((key, link_id, project)) = signed {
        for (k, v) in sign_headers(key, path, link_id, project, &bytes, st.link.protocol(), now_ms()) {
            req = req.header(k, v);
        }
    }
    let res = req.send().await.map_err(|e| PeerError::Unreachable(short_error(&e)))?;
    let status = res.status();
    if status.is_success() {
        return Ok(res);
    }
    let text = res.text().await.map_err(|e| PeerError::Unreachable(short_error(&e)))?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Err(PeerError::Refused {
        status: status.as_u16(),
        code: v["code"].as_str().unwrap_or_default().to_string(),
        message: v["message"].as_str().map(String::from).unwrap_or_else(|| text.chars().take(200).collect()),
        data: v.get("data").cloned(),
    })
}

fn short_error(e: &reqwest::Error) -> String {
    use std::error::Error;
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        msg = s.to_string();
        src = s.source();
    }
    if e.is_timeout() { "it didn't answer in time".into() } else { msg }
}

fn refused(why: impl Into<String>) -> ApiError {
    ApiError::forbidden(codes::LINK_REFUSED, why)
}

/// A signed link request's headers, read but not yet checked against a link.
pub(crate) struct SignedParts {
    pub(crate) link_id: String,
    project: String,
    time: i64,
    nonce: String,
    signature: Signature,
}

impl SignedParts {
    /// Reads the headers and checks what needs no link: the protocol and the time.
    pub(crate) async fn read(st: &AppState, headers: &HeaderMap) -> Result<Self, ApiError> {
        Self::read_with(st, headers, false).await
    }

    /// [`read`](Self::read), taking any protocol when `any_protocol`: the owner's request to update this instance,
    /// which must get through when the two ends speak different ones (PDT-637).
    async fn read_with(st: &AppState, headers: &HeaderMap, any_protocol: bool) -> Result<Self, ApiError> {
        let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
        let (Some(link_id), Some(project), Some(protocol), Some(time), Some(nonce), Some(sig)) =
            (get(h::LINK), get(h::PROJECT), get(h::PROTOCOL), get(h::TIME), get(h::NONCE), get(h::SIGNATURE))
        else {
            return Err(refused("not a signed project-link request"));
        };
        let protocol: u32 = protocol.parse().map_err(|_| refused("a project-link protocol that isn't a number"))?;
        if !any_protocol && !link::compatible(st.link.protocol(), protocol) {
            let me = hello(st).await?;
            return Err(ApiError::conflict(
                codes::LINK_INCOMPATIBLE,
                format!("{} speaks project-link protocol {} (v{}), not {protocol}", me.name, me.protocol, me.version),
            )
            .with_data(json!({ "instance": me })));
        }
        let time: i64 = time.parse().map_err(|_| refused("a request time that isn't a number"))?;
        if !link::fresh(time, now_ms()) {
            return Err(refused("the request's time is too far from this instance's clock"));
        }
        let signature = B64
            .decode(sig.trim())
            .ok()
            .and_then(|b| Signature::from_slice(&b).ok())
            .ok_or_else(|| refused("a signature that can't be read"))?;
        Ok(Self { link_id, project, time, nonce, signature })
    }

    /// Checks the request against its link: the owner's project the link is for, and the other end's public key.
    pub(crate) fn check(
        &self,
        st: &AppState,
        method: &Method,
        path: &str,
        body: &[u8],
        link_project: &str,
        peer_key: &str,
    ) -> Result<(), ApiError> {
        if self.project != link_project {
            return Err(refused("that link isn't for this project"));
        }
        let key = parse_public_key(peer_key).ok_or_else(|| refused("the link's key can't be read"))?;
        let text = link::signed_text(&SignedRequest {
            method: method.as_str(),
            path,
            link: &self.link_id,
            project: &self.project,
            time: self.time,
            nonce: &self.nonce,
            body,
        });
        key.verify_strict(text.as_bytes(), &self.signature)
            .map_err(|_| refused("the signature doesn't match the link's key"))?;
        if !st.link.first_use(&self.nonce, now_ms()) {
            return Err(refused("that request was already taken"));
        }
        Ok(())
    }
}

/// The owner checks a pairing: the code it showed, the remote instance's protocol, and that it isn't the owner itself.
/// It then makes its key for the link, keeps the remote instance's, and joins it to the project. A remote instance
/// that pairs again (with a new code) replaces its old link.
async fn pair(State(st): State<AppState>, ApiJson(req): ApiJson<PairRequest>) -> Result<Json<PairReply>, ApiError> {
    let me = hello(&st).await?;
    if !link::compatible(me.protocol, req.instance.protocol) {
        let them = Side { name: &req.instance.name, protocol: req.instance.protocol, version: &req.instance.version };
        let us = Side { name: &me.name, protocol: me.protocol, version: &me.version };
        return Err(ApiError::conflict(codes::LINK_INCOMPATIBLE, link::incompatible_message(them, us)));
    }
    let wrong =
        || ApiError::forbidden(codes::PAIRING_CODE, "that pairing code is wrong or has expired: show a new one");
    let code = link::normalize_code(&req.code).ok_or_else(wrong)?;
    let project_id = st.link.take_code(&code, now_ms()).ok_or_else(wrong)?;
    let project = st.project(&project_id).await?;
    if req.instance.instance_id == me.instance_id {
        return Err(ApiError::conflict(
            codes::REMOTE_PROJECT,
            format!("{} is this instance: pair another one", me.name),
        ));
    }
    let url = link::instance_url(&req.url).map_err(|e| ApiError::invalid(e.to_string()))?;
    parse_public_key(&req.public_key).ok_or_else(|| ApiError::invalid("the remote instance's key can't be read"))?;
    let name = pdt_core::instance::valid_name(&req.instance.name).map_err(|e| ApiError::invalid(e.to_string()))?;

    for old in st.store.list_remote_instances(Some(&project.id)).await? {
        if old.instance_id == req.instance.instance_id {
            st.store.delete_remote_instance(&old.link_id).await?;
            delete_key(&st, &old.link_id).await;
        }
    }
    let link_id = uuid::Uuid::now_v7().to_string();
    let key = new_key();
    save_key(&st, &link_id, &key).await?;
    let instance = pdt_core::link::RemoteInstance {
        link_id: link_id.clone(),
        project_id: project.id.clone(),
        instance_id: req.instance.instance_id.clone(),
        name,
        url,
        public_key: req.public_key.clone(),
        version: req.instance.version.clone(),
        protocol: req.instance.protocol,
        facts: None,
        paused: false,
        last_seen_at: None,
        last_error: None,
        secrets: vec![],
        cap: None,
        remote_cap: None,
        created_at: now_ms(),
    };
    if let Err(e) = st.store.insert_remote_instance(&instance).await {
        delete_key(&st, &link_id).await;
        return Err(e.into());
    }
    let repo = std::path::PathBuf::from(&project.repo_path);
    let repo_url =
        tokio::task::spawn_blocking(move || pdt_git::remote_url(&repo, "origin")).await.map_err(ApiError::internal)?;
    tracing::info!(project = %project.key, instance = %instance.name, link = %link_id, "a remote instance joined");
    // Heard from soon (once the remote instance has saved the link this answer gives it), so the list shows it
    // connected without waiting for the next heartbeat.
    let bg = st.clone();
    let joined = instance.clone();
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_HEARTBEAT_AFTER).await;
        heartbeat(&bg, &joined, false).await
    });
    Ok(Json(PairReply {
        link_id,
        project: pdt_core::link::LinkedProject {
            id: project.id,
            key: project.key,
            name: project.name,
            base_branch: project.base_branch,
            repo_url,
        },
        owner: me,
        public_key: public_key(&key),
    }))
}

/// What the owner sends with a heartbeat: itself, and whether it wants the agents checked again now (Check now).
#[derive(Debug, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeartbeatBody {
    owner: Option<InstanceHello>,
    #[serde(default)]
    refresh: bool,
}

/// A remote project's request from its owner, checked: the link it names is one of this instance's remote
/// projects, for the project it names, and signed with that owner's key.
pub(crate) async fn from_owner(
    st: &AppState,
    method: &Method,
    uri: &OriginalUri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<pdt_core::link::OwnerLink, ApiError> {
    from_owner_with(st, method, uri, headers, body, false).await
}

/// [`from_owner`], whatever protocol the owner speaks: only for its request to update this instance.
pub(crate) async fn from_owner_any_protocol(
    st: &AppState,
    method: &Method,
    uri: &OriginalUri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<pdt_core::link::OwnerLink, ApiError> {
    from_owner_with(st, method, uri, headers, body, true).await
}

async fn from_owner_with(
    st: &AppState,
    method: &Method,
    uri: &OriginalUri,
    headers: &HeaderMap,
    body: &[u8],
    any_protocol: bool,
) -> Result<pdt_core::link::OwnerLink, ApiError> {
    crate::host_guard::optional_json::<Value>(headers, body)?;
    let parts = SignedParts::read_with(st, headers, any_protocol).await?;
    let link = st
        .store
        .owner_link_by_id(&parts.link_id)
        .await?
        .ok_or_else(|| ApiError::forbidden(codes::LINK_REVOKED, "this instance isn't paired on that link"))?;
    parts.check(st, method, uri.path(), body, &link.owner_project_id, &link.owner_public_key)?;
    Ok(link)
}

/// A report from one of this instance's remote instances, checked: the link it names is a joined instance's, and it
/// is signed with that instance's key.
pub(crate) async fn from_remote(
    st: &AppState,
    method: &Method,
    uri: &OriginalUri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<pdt_core::link::RemoteInstance, ApiError> {
    crate::host_guard::optional_json::<Value>(headers, body)?;
    let parts = SignedParts::read(st, headers).await?;
    let instance =
        st.store.get_remote_instance(&parts.link_id).await?.ok_or_else(|| {
            ApiError::forbidden(codes::LINK_REVOKED, "this instance has no remote instance on that link")
        })?;
    parts.check(st, method, uri.path(), body, &instance.project_id, &instance.public_key)?;
    Ok(instance)
}

/// The remote instance answers its owner's heartbeat with itself and its facts.
async fn heartbeat_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<HeartbeatReply>, ApiError> {
    let link = from_owner(&st, &method, &uri, &headers, &body).await?;
    let sent: HeartbeatBody = crate::host_guard::optional_json(&headers, &body)?;
    let owner_version = sent.owner.map(|o| o.version);
    st.store.owner_contact(&link.link_id, now_ms(), owner_version.as_deref()).await?;
    let secrets = crate::exec::own_secret_names(&st, &link.project_id).await?;
    Ok(Json(HeartbeatReply {
        instance: hello(&st).await?,
        facts: facts(&st, if sent.refresh { Agents::Fresh(REFRESH_WAIT) } else { Agents::Cached }).await,
        paused: link.paused,
        secrets,
        cap: link.cap,
    }))
}

/// The owner removed this instance from the project: the remote project goes, with this end's key.
async fn revoke_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let link = from_owner(&st, &method, &uri, &headers, &body).await?;
    crate::exec::close_link(&st, &link.link_id).await;
    st.store.delete_project(&link.project_id).await?;
    delete_key(&st, &link.link_id).await;
    tracing::info!(owner = %link.owner_name, link = %link.link_id, "the owner removed this instance from a project");
    Ok(StatusCode::NO_CONTENT)
}

/// A remote instance asks whether this instance still has its link: answered when it does, refused (`link_revoked`)
/// when it was removed.
async fn status_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    from_remote(&st, &method, &uri, &headers, &body).await?;
    Ok(Json(json!({})))
}

/// Whether a call to the other end of a link got through: it answered, or it no longer has the link (so the link is
/// over at both ends all the same). Anything else is logged as `what`.
pub(crate) fn got_through<T>(r: &Result<T, PeerError>, who: &str, what: &str) -> bool {
    match r {
        Ok(_) => true,
        Err(PeerError::Refused { code, .. }) if code == codes::LINK_REVOKED => true,
        Err(e) => {
            tracing::warn!(other = %who, error = %e.text(), "{what}");
            false
        }
    }
}

/// The owner tells a joined instance it is removed from `project_id` (so it drops the remote project and its key),
/// then forgets its own key for the link. True when the instance was told; an instance that can't be reached is
/// removed here all the same, and finds out once it asks ([`check_owners`]).
pub(crate) async fn revoke(st: &AppState, i: &pdt_core::link::RemoteInstance, project_id: &str) -> bool {
    let told = match load_key_for_revoke(st, &i.link_id).await {
        Some(key) => {
            let r: Result<Value, _> =
                post(st, &i.url, REVOKE_PATH, Some((&key, &i.link_id, project_id)), &json!({})).await;
            got_through(&r, &i.name, "could not tell a removed remote instance")
        }
        None => {
            tracing::warn!(instance = %i.name, "no key to tell a removed remote instance with");
            false
        }
    };
    delete_key(st, &i.link_id).await;
    told
}

/// A remote instance asks the owners it hasn't heard from lately whether they still have its links; one that says
/// it doesn't removed this instance while it couldn't tell it, and the remote project says so (the `project links`
/// loop).
pub async fn check_owners(st: &AppState) {
    let projects = match st.store.list_remote_projects().await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "could not list the remote projects");
            return;
        }
    };
    let now = now_ms();
    let quiet = projects.into_iter().map(|(_, l)| l).filter(|l| {
        l.removed_at.is_none() && link::state(l.last_contact_at, true, false, now) == link::LinkState::Offline
    });
    futures_util::future::join_all(quiet.map(|l| async move {
        let Ok(key) = load_key(st, &l.link_id).await else { return };
        let r: Result<Value, _> =
            post(st, &l.owner_url, STATUS_PATH, Some((&key, &l.link_id, &l.owner_project_id)), &json!({})).await;
        if let Err(PeerError::Refused { code, .. }) = r
            && code == codes::LINK_REVOKED
        {
            tracing::info!(owner = %l.owner_name, link = %l.link_id, "the owner removed this instance from a project");
            if let Err(e) = st.store.set_owner_link_removed(&l.link_id, now_ms()).await {
                tracing::warn!(error = %e, "could not record a removed remote project");
            }
        }
    }))
    .await;
}

/// A remote instance left one of this instance's projects: it is no longer joined, and the owner's key goes.
async fn leave_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let instance = from_remote(&st, &method, &uri, &headers, &body).await?;
    st.store.delete_remote_instance(&instance.link_id).await?;
    delete_key(&st, &instance.link_id).await;
    tracing::info!(instance = %instance.name, link = %instance.link_id, "a remote instance left a project");
    Ok(StatusCode::NO_CONTENT)
}

/// Which agent checks [`facts`] reports.
pub(crate) enum Agents {
    /// The last ones, never waiting: none before the first checks finish, and checks older than
    /// [`link::AGENT_FACTS_TTL_MS`] run again in the background.
    Cached,
    /// Checks run now, waiting for them up to this long (else the last ones).
    Fresh(Duration),
}

/// What this instance reports about itself: its OS, CPUs and free disk, its agents (from the host checks) and the
/// tools on its PATH. It never runs the agent checks inline (they can take far longer than a link call may).
pub(crate) async fn facts(st: &AppState, which: Agents) -> InstanceFacts {
    let checks = match which {
        Agents::Fresh(wait) => crate::checks::refreshed(st, wait).await,
        Agents::Cached => {
            let c = crate::checks::cached(st);
            if link::agent_facts_stale(c.as_ref().map(|c| c.checked_at), now_ms()) {
                crate::checks::refresh_in_background(st);
            }
            c
        }
    };
    let agents = checks.as_ref().map(agent_facts).unwrap_or_default();
    let search = st.platform.login_path.clone();
    let tools = tokio::task::spawn_blocking(move || {
        link::TOOLS
            .iter()
            .filter_map(|(name, programs)| {
                programs
                    .iter()
                    .find_map(|p| crate::host::found(p, search.as_deref()))
                    .map(|path| pdt_core::link::ToolFact { name: (*name).to_string(), path })
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    InstanceFacts {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        cpus: std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(0),
        disk_free_bytes: (st.disk_space)(&st.data_dir).map(|d| d.free),
        agents,
        agents_checked_at: checks.map(|c| c.checked_at),
        tools,
    }
}

/// The agents the host checks found, and whether each is logged in.
fn agent_facts(checks: &crate::checks::HostChecks) -> Vec<pdt_core::link::AgentFact> {
    let mut agents = vec![
        pdt_core::link::AgentFact {
            agent: "claude".into(),
            installed: checks.claude.found,
            logged_in: checks.claude.usable(),
        },
        pdt_core::link::AgentFact {
            agent: "codex".into(),
            installed: checks.codex.found,
            logged_in: checks.codex.usable(),
        },
        pdt_core::link::AgentFact {
            agent: "devin".into(),
            installed: checks.devin.found,
            logged_in: checks.devin.usable(),
        },
    ];
    agents.extend(checks.acp.iter().map(|(kind, c)| pdt_core::link::AgentFact {
        agent: kind.clone(),
        installed: c.found,
        logged_in: c.usable(),
    }));
    agents
}

/// The owner's heartbeat to one joined instance: it records what came back (or why nothing did). With `refresh`
/// (Check now) the instance checks its agents again before it answers.
pub(crate) async fn heartbeat(st: &AppState, i: &pdt_core::link::RemoteInstance, refresh: bool) {
    let outcome = heartbeat_once(st, i, refresh).await;
    let write = match &outcome {
        Ok(reply) => {
            let facts = link::merge_facts(reply.facts.clone(), i.facts.as_ref());
            st.store
                .record_heartbeat(
                    &i.link_id,
                    pdt_store::Heard::Answer {
                        name: &reply.instance.name,
                        version: &reply.instance.version,
                        protocol: reply.instance.protocol,
                        facts: &facts,
                        paused: reply.paused,
                        secrets: &reply.secrets,
                        cap: reply.cap,
                        at: now_ms(),
                    },
                )
                .await
        }
        Err((error, protocol)) => {
            let protocol = protocol.as_ref().map(|(p, v)| (*p, v.as_str()));
            st.store.record_heartbeat(&i.link_id, pdt_store::Heard::Failed { error, protocol }).await
        }
    };
    if let Err(e) = write {
        tracing::warn!(error = %e, instance = %i.name, "could not record a heartbeat");
    }
}

type Failure = (String, Option<(u32, String)>);

async fn heartbeat_once(
    st: &AppState,
    i: &pdt_core::link::RemoteInstance,
    refresh: bool,
) -> Result<HeartbeatReply, Failure> {
    let key = load_key(st, &i.link_id).await.map_err(|e| (e.message().to_string(), None))?;
    let project =
        st.store.get_project(&i.project_id).await.ok().flatten().ok_or(("no such project".to_string(), None))?;
    let me = hello(st).await.map_err(|e| (e.message().to_string(), None))?;
    let body = HeartbeatBody { owner: Some(me.clone()), refresh };
    let us = Side { name: &me.name, protocol: me.protocol, version: &me.version };
    let reply: HeartbeatReply =
        match post(st, &i.url, HEARTBEAT_PATH, Some((&key, &i.link_id, &project.id)), &body).await {
            Ok(r) => r,
            Err(PeerError::Refused { code, data: Some(data), .. }) if code == codes::LINK_INCOMPATIBLE => {
                let them: Option<InstanceHello> = serde_json::from_value(data["instance"].clone()).ok();
                return Err(match them {
                    Some(t) => {
                        let side = Side { name: &t.name, protocol: t.protocol, version: &t.version };
                        (link::incompatible_message(side, us), Some((t.protocol, t.version.clone())))
                    }
                    None => ("it speaks another project-link protocol".into(), None),
                });
            }
            Err(PeerError::Refused { code, .. }) if code == codes::LINK_REVOKED => {
                return Err((format!("{} left the project; remove it here too", i.name), None));
            }
            Err(e) => return Err((e.text(), None)),
        };
    if !link::compatible(me.protocol, reply.instance.protocol) {
        let t = &reply.instance;
        let side = Side { name: &t.name, protocol: t.protocol, version: &t.version };
        return Err((link::incompatible_message(side, us), Some((t.protocol, t.version.clone()))));
    }
    Ok(reply)
}

/// The owner's heartbeats to every joined instance, side by side (the `project links` loop, and tests).
pub async fn heartbeat_all(st: &AppState) {
    let instances = match st.store.list_remote_instances(None).await {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "could not list the remote instances");
            return;
        }
    };
    futures_util::future::join_all(instances.iter().map(|i| heartbeat(st, i, false))).await;
    match st.store.list_remote_instances(None).await {
        Ok(heard) => crate::offline_instances::check(st, &heard).await,
        Err(e) => tracing::warn!(error = %e, "could not list the remote instances"),
    }
}

/// The link's own routes (the other instance calls them); `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::post;
    axum::Router::new()
        .route("/link/pair", post(pair))
        .route("/link/heartbeat", post(heartbeat_route))
        .route("/link/revoke", post(revoke_route))
        .route("/link/leave", post(leave_route))
        .route("/link/status", post(status_route))
        .route("/link/update", post(crate::update_instances::link_route))
        .merge(crate::exec::router())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_works_once_and_wrong_ones_use_it_up() {
        let rt = LinkRuntime::default();
        let c = rt.new_code("p", 0);
        assert_eq!(rt.take_code(&c.code, 1), Some("p".into()));
        assert_eq!(rt.take_code(&c.code, 1), None, "once");

        let c = rt.new_code("p", 0);
        assert_eq!(rt.take_code(&c.code, link::PAIRING_CODE_TTL_MS + 1), None, "expired");

        let c = rt.new_code("p", 0);
        for _ in 0..link::PAIRING_MAX_FAILURES {
            assert_eq!(rt.take_code("AAAA-AAAA", 1), None);
        }
        assert_eq!(rt.take_code(&c.code, 1), None, "too many wrong codes");
    }

    #[test]
    fn nonces_are_taken_once() {
        let rt = LinkRuntime::default();
        assert!(rt.first_use("n", 0));
        assert!(!rt.first_use("n", 1));
        assert!(rt.first_use("n", 3 * link::MAX_SKEW_MS), "forgotten once its time is past");
    }

    #[test]
    fn keys_round_trip_and_sign() {
        let k = new_key();
        let pk = parse_public_key(&public_key(&k)).unwrap();
        let sig = k.sign(b"hello");
        assert!(pk.verify_strict(b"hello", &sig).is_ok());
        assert!(pk.verify_strict(b"other", &sig).is_err());
        assert!(parse_public_key("not base64").is_none());
    }
}
