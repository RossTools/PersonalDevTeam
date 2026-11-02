//! Secrets (the update token, the forge logins) live only in the OS keychain / credential manager (spec §3.2). Never logged, never
//! returned by the API, never written to the database. The one exception is Linux without a Secret Service (a headless
//! machine, PDT-197): there they go to a file in the data folder only the user can read ([`FileSecretStore`]), and
//! Settings → Secrets says so. macOS and Windows never fall back.

use std::collections::HashMap;
use std::sync::Mutex;

/// The keychain account holding the stable instance's GitHub update token. See [`update_token_account`].
pub const UPDATE_TOKEN: &str = "update-token";

/// The keychain account for the update token of the pdtd using `data_dir`: [`UPDATE_TOKEN`] for the stable instance's
/// default data dir (`stable_data_dir`), else `update-token:<12 hex digits of the canonical data dir>`, so a dev
/// instance or a test never reads, replaces or deletes the stable instance's token.
pub fn update_token_account(data_dir: &std::path::Path, stable_data_dir: Option<&std::path::Path>) -> String {
    instance_account(UPDATE_TOKEN, data_dir, stable_data_dir)
}

/// The keychain account holding the stable instance's GitHub login (PDT-177). See [`github_login_account`].
pub const GITHUB_LOGIN: &str = "github-login";

/// The keychain account for the GitHub login of the pdtd using `data_dir`, kept apart per instance as the update
/// token's is ([`update_token_account`]).
pub fn github_login_account(data_dir: &std::path::Path, stable_data_dir: Option<&std::path::Path>) -> String {
    instance_account(GITHUB_LOGIN, data_dir, stable_data_dir)
}

/// The keychain account holding the stable instance's forge logins (PDT-244): every account on every forge host, as
/// one JSON `pdt_forge::login::AccountStore`. See [`forge_logins_account`].
pub const FORGE_LOGINS: &str = "forge-logins";

/// The keychain account for the forge logins of the pdtd using `data_dir`, kept apart per instance as the update
/// token's is. The first read copies an old [`GITHUB_LOGIN`] into it (and leaves that one, for an older PDT).
pub fn forge_logins_account(data_dir: &std::path::Path, stable_data_dir: Option<&std::path::Path>) -> String {
    instance_account(FORGE_LOGINS, data_dir, stable_data_dir)
}

/// `name` for the stable instance's default data dir, else `name:<12 hex digits of the canonical data dir>`.
fn instance_account(name: &str, data_dir: &std::path::Path, stable_data_dir: Option<&std::path::Path>) -> String {
    let canonical = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let dir = canonical(data_dir);
    if stable_data_dir.is_some_and(|s| canonical(s) == dir) {
        return name.to_string();
    }
    // FNV-1a (64-bit): stable across builds and platforms, unlike `DefaultHasher`.
    let hash = dir
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3));
    format!("{name}:{:012x}", hash >> 16)
}

/// How every keychain failure message starts (so a later successful read can clear it).
pub const KEYCHAIN_FAILED: &str = "the OS keychain failed";

#[derive(Debug, thiserror::Error)]
#[error("{KEYCHAIN_FAILED}: {0}")]
pub struct SecretError(pub String);

/// Blocking: call from `spawn_blocking` (macOS may show a keychain prompt).
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError>;
    /// Deleting a secret that does not exist succeeds.
    fn delete(&self, name: &str) -> Result<(), SecretError>;
}

/// For tests and `PDT_SECRET_STORE=memory`.
#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(name.into(), value.into());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(name);
        Ok(())
    }
}

/// Windows Credential Manager / macOS login keychain, through `keyring` (v1 API, verified fact 7).
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: &str) -> Self {
        Self { service: service.into() }
    }

    fn entry(&self, name: &str) -> Result<keyring::v1::Entry, SecretError> {
        keyring::v1::Entry::new(&self.service, name).map_err(|e| SecretError(e.to_string()))
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        match self.entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::v1::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError(e.to_string())),
        }
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        self.entry(name)?.set_password(value).map_err(|e| SecretError(e.to_string()))
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::v1::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError(e.to_string())),
        }
    }
}

impl KeyringStore {
    /// Whether the keychain answers: reading an entry that does not exist succeeds (with nothing). On Linux this fails
    /// when no Secret Service (gnome-keyring, KWallet) is reachable over D-Bus.
    pub fn reachable(&self) -> bool {
        self.get("pdt-store-probe").is_ok()
    }
}

/// Where this instance keeps secrets (Settings → Secrets shows it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SecretBackend {
    /// The OS keychain: macOS login keychain, Windows Credential Manager, a Secret Service on Linux.
    Keychain,
    /// `secrets.json` in the data folder, readable only by the user (Linux without a Secret Service).
    File,
    /// In memory, gone when pdtd stops (tests, `PDT_SECRET_STORE=memory`, rehearsals).
    Memory,
}

/// `GET /api/secrets/store`.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SecretStoreInfo {
    pub backend: SecretBackend,
    /// The file, for [`SecretBackend::File`].
    pub path: Option<String>,
    /// Why the store may not work right now: the keychain this instance uses did not answer at startup (Linux, a
    /// Secret Service that is not up or locked). Calls keep trying it; Settings → Secrets shows this.
    pub problem: Option<String>,
}

/// What [`choose_backend`] knows of this data folder on Linux: which store it used before, so the choice never flips
/// between restarts (secrets saved in one store would look lost in the other).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoreHistory {
    /// `secrets.json` exists: secrets went to the file.
    pub file: bool,
    /// [`keychain_marker`] exists: secrets went to the Secret Service.
    pub keychain: bool,
}

/// Which store pdtd uses. `forced` is `PDT_SECRET_STORE`: `memory` anywhere, `file` on Linux only. macOS and Windows:
/// always the keychain. Linux sticks to the store it used before (`history`; the file wins if both exist, since
/// that is where secrets went last), and only a data folder that has used neither asks `secret_service`: the
/// keychain when one answers, else the file. A keychain used before that does not answer now is kept (and reported),
/// never swapped for the file.
pub fn choose_backend(
    linux: bool,
    forced: Option<&str>,
    history: StoreHistory,
    secret_service: impl FnOnce() -> bool,
) -> SecretBackend {
    match forced {
        Some("memory") => SecretBackend::Memory,
        Some("file") if linux => SecretBackend::File,
        _ if !linux => SecretBackend::Keychain,
        _ if history.file => SecretBackend::File,
        _ if history.keychain => SecretBackend::Keychain,
        _ if secret_service() => SecretBackend::Keychain,
        _ => SecretBackend::File,
    }
}

/// Written on Linux once the Secret Service is chosen, so a later start that cannot reach it keeps using it instead of
/// falling back to the file: `<data dir>/secrets.keychain`.
pub fn keychain_marker(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("secrets.keychain")
}

/// The file [`FileSecretStore`] uses: `<data dir>/secrets.json`.
pub fn secrets_file(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("secrets.json")
}

/// Secrets in a JSON file (name → value) that only the user can read (0600), for Linux hosts without a Secret
/// Service. Each change rewrites it whole: written to a temporary file with those permissions, then renamed over it.
pub struct FileSecretStore {
    path: std::path::PathBuf,
    lock: Mutex<()>,
}

impl FileSecretStore {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path, lock: Mutex::new(()) }
    }

    fn err(&self, what: &str, e: impl std::fmt::Display) -> SecretError {
        SecretError(format!("{what} {}: {e}", self.path.display()))
    }

    fn read(&self) -> Result<std::collections::BTreeMap<String, String>, SecretError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| self.err("reading", e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(e) => Err(self.err("reading", e)),
        }
    }

    fn write(&self, map: &std::collections::BTreeMap<String, String>) -> Result<(), SecretError> {
        use std::io::Write;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| self.err("writing", e))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&tmp).map_err(|e| self.err("writing", e))?;
        let bytes = serde_json::to_vec_pretty(map).expect("json");
        f.write_all(&bytes).and_then(|()| f.sync_all()).map_err(|e| self.err("writing", e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| self.err("writing", e))
    }

    fn change(
        &self,
        f: impl FnOnce(&mut std::collections::BTreeMap<String, String>) -> bool,
    ) -> Result<(), SecretError> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.read()?;
        if f(&mut map) { self.write(&map) } else { Ok(()) }
    }

    /// Creates the (empty) file if it is missing, so the next start keeps choosing it ([`StoreHistory::file`]).
    pub fn ensure(&self) -> Result<(), SecretError> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.path.exists() { Ok(()) } else { self.write(&Default::default()) }
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        Ok(self.read()?.get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        self.change(|m| m.insert(name.into(), value.into()).as_deref() != Some(value))
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        self.change(|m| m.remove(name).is_some())
    }
}

/// What a keychain call reports when it runs out of time (after "the OS keychain failed: "), naming the secret
/// (`what`, e.g. [`secret_label`]). Only a call that really timed out, or one refused because an earlier call for the
/// same secret is still stuck, says this: a keychain that is merely busy with other calls never does.
pub fn stalled(what: &str) -> SecretError {
    SecretError(format!("it did not answer in time for {what}{}", waiting_hint("it")))
}

/// Why a keychain call may hang, after its message: macOS asks someone at the Mac to allow access. Other keychains
/// never ask, so there is nothing to suggest (PDT-213: Windows said the same).
fn waiting_hint(what: &str) -> String {
    if cfg!(target_os = "macos") {
        format!(" (macOS may be waiting for someone at this Mac to allow access to {what})")
    } else {
        String::new()
    }
}

/// How [`TimeLimited`] names the secret in `account` in its errors: the update token, the GitHub login, or (callers
/// that know its name say which, with [`SecretError::naming`]) a connection's secret or a vault secret.
pub fn secret_label(account: &str) -> &'static str {
    let is = |name: &str| account == name || account.strip_prefix(name).is_some_and(|r| r.starts_with(':'));
    if is(UPDATE_TOKEN) {
        "the update token"
    } else if is(GITHUB_LOGIN) {
        "the GitHub login"
    } else if is(FORGE_LOGINS) {
        "the forge logins"
    } else if account.starts_with("connection:") {
        "a connection's secret"
    } else {
        "a secret"
    }
}

impl SecretError {
    /// Names the secret in `account` as `what` instead of its generic [`secret_label`] (e.g. the connection's name).
    pub fn naming(self, account: &str, what: &str) -> SecretError {
        SecretError(self.0.replacen(secret_label(account), what, 1))
    }
}

/// How many calls may be stuck past their timeout at once (each holds a thread) before every further call fails at
/// once instead of starting another.
const MAX_STUCK: usize = 4;

/// Bounds every call to `inner` by `timeout`, so a keychain waiting on an approval prompt nobody sees cannot hang
/// pdtd. Calls run side by side, each on its own thread with its own limit. A call that times out keeps running;
/// until it returns, further calls for the same secret fail at once instead of piling up more stuck threads, and
/// once [`MAX_STUCK`] calls are stuck every call does. Calls for other secrets are never refused for being busy.
pub struct TimeLimited {
    inner: std::sync::Arc<dyn SecretStore>,
    timeout: std::time::Duration,
    /// Calls past their timeout and still running, by account.
    stuck: std::sync::Arc<Mutex<HashMap<String, usize>>>,
}

impl TimeLimited {
    pub fn new(inner: std::sync::Arc<dyn SecretStore>, timeout: std::time::Duration) -> Self {
        Self { inner, timeout, stuck: Default::default() }
    }

    #[cfg(test)]
    fn stuck_calls(&self) -> usize {
        self.stuck.lock().unwrap().values().sum()
    }

    fn call<T: Send + 'static>(
        &self,
        account: &str,
        f: impl FnOnce(&dyn SecretStore) -> Result<T, SecretError> + Send + 'static,
    ) -> Result<T, SecretError> {
        use std::sync::atomic::{AtomicU8, Ordering};
        const RUNNING: u8 = 0;
        const DONE: u8 = 1;
        const STUCK: u8 = 2;
        {
            let stuck = self.stuck.lock().unwrap_or_else(|e| e.into_inner());
            if stuck.contains_key(account) {
                return Err(stalled(secret_label(account)));
            }
            let n: usize = stuck.values().sum();
            if n >= MAX_STUCK {
                return Err(SecretError(format!(
                    "{n} earlier calls have not answered yet{}",
                    waiting_hint("a secret")
                )));
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let state = std::sync::Arc::new(AtomicU8::new(RUNNING));
        let (inner, stuck, name) = (self.inner.clone(), self.stuck.clone(), account.to_string());
        let thread_state = state.clone();
        std::thread::spawn(move || {
            let r = f(inner.as_ref());
            // Counted as stuck (under the lock, before this swap could see it): it no longer is.
            if thread_state.swap(DONE, Ordering::SeqCst) == STUCK {
                let mut stuck = stuck.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(n) = stuck.get_mut(&name) {
                    *n -= 1;
                    if *n == 0 {
                        stuck.remove(&name);
                    }
                }
            }
            let _ = tx.send(r);
        });
        match rx.recv_timeout(self.timeout) {
            Ok(r) => r,
            Err(_) => {
                let mut stuck = self.stuck.lock().unwrap_or_else(|e| e.into_inner());
                if state.compare_exchange(RUNNING, STUCK, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    *stuck.entry(account.to_string()).or_default() += 1;
                    drop(stuck);
                    Err(stalled(secret_label(account)))
                } else {
                    // It answered just as the limit passed.
                    drop(stuck);
                    rx.recv().unwrap_or_else(|_| Err(stalled(secret_label(account))))
                }
            }
        }
    }
}

impl SecretStore for TimeLimited {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        let n = name.to_string();
        self.call(name, move |s| s.get(&n))
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        let (n, value) = (name.to_string(), value.to_string());
        self.call(name, move |s| s.set(&n, &value))
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        let n = name.to_string();
        self.call(name, move |s| s.delete(&n))
    }
}

/// Keeps what `inner` read in memory, so pollers (the update banner, Settings, connections' sensors) do not ask the
/// keychain on every call. Every change goes through pdtd (each instance has its own accounts), so a value is
/// replaced on `set`, forgotten on `delete` (the GitHub login's reconnect or sign-out), and dropped on a failed
/// change. Failed reads are never kept: the next call asks again.
pub struct Cached {
    inner: std::sync::Arc<dyn SecretStore>,
    cache: Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    values: HashMap<String, Option<String>>,
    /// Bumped by every change, so a read that started before one does not put back the old value.
    generation: u64,
}

impl Cached {
    pub fn new(inner: std::sync::Arc<dyn SecretStore>) -> Self {
        Self { inner, cache: Default::default() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn changed(&self, name: &str, r: &Result<(), SecretError>, value: Option<&str>) {
        let mut c = self.lock();
        c.generation += 1;
        match r {
            Ok(()) => c.values.insert(name.to_string(), value.map(str::to_string)),
            Err(_) => c.values.remove(name),
        };
    }
}

impl SecretStore for Cached {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        let generation = {
            let c = self.lock();
            if let Some(v) = c.values.get(name) {
                return Ok(v.clone());
            }
            c.generation
        };
        let v = self.inner.get(name)?;
        let mut c = self.lock();
        if c.generation == generation {
            c.values.insert(name.to_string(), v.clone());
        }
        Ok(v)
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        let r = self.inner.set(name, value);
        self.changed(name, &r, Some(value));
        r
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        let r = self.inner.delete(name);
        self.changed(name, &r, None);
        r
    }
}

/// Gives every value PDT reads from or writes to `inner` to the redactor, so the secrets PDT keeps for itself (the
/// update token, the forge logins, connections' secrets and sign-ins) are kept out of what it stores, shows and logs
/// as the user's secrets are. A value holding JSON (a login with its tokens) gives the strings under its token, secret
/// and password fields. The user's secrets (`secret:<id>`) are left to the vault, which knows them by name.
pub struct Redacting {
    inner: std::sync::Arc<dyn SecretStore>,
    redactor: std::sync::Arc<pdt_core::secret::Redactor>,
}

impl Redacting {
    pub fn new(inner: std::sync::Arc<dyn SecretStore>, redactor: std::sync::Arc<pdt_core::secret::Redactor>) -> Self {
        Self { inner, redactor }
    }

    fn learn(&self, account: &str, value: &str) {
        if account.starts_with("secret:") {
            return;
        }
        let label = redaction_label(account);
        match serde_json::from_str::<serde_json::Value>(value) {
            Ok(v @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => {
                let mut found = vec![];
                credential_strings(&v, "", &mut found);
                for (path, s) in found {
                    self.redactor.set_labelled(&format!("{account}/{path}"), &s, label);
                }
            }
            _ => self.redactor.set_labelled(account, value, label),
        }
    }
}

/// What a redacted value of PDT's own reads as (`[secret:<label>]`).
fn redaction_label(account: &str) -> &'static str {
    let is = |name: &str| account == name || account.strip_prefix(name).is_some_and(|r| r.starts_with(':'));
    if is(UPDATE_TOKEN) {
        "update-token"
    } else if is(GITHUB_LOGIN) || is(FORGE_LOGINS) {
        "forge-login"
    } else if account.starts_with("mcp-oauth:") {
        "mcp-sign-in"
    } else if account.starts_with("connection:") {
        "connection"
    } else {
        "pdt"
    }
}

/// The strings under credential fields (`token`, `access_token`, `refreshToken`, `client_secret`, `password`…) in `v`,
/// with their paths; not addresses, types or times (`token_endpoint`, `token_type`, `expires_at`), and not short
/// ones (`"bearer"`).
fn credential_strings(v: &serde_json::Value, path: &str, out: &mut Vec<(String, String)>) {
    let credential = |key: &str| {
        let k = key.to_ascii_lowercase();
        ["token", "secret", "password"].iter().any(|w| k.contains(w))
            && !["endpoint", "url", "uri", "type", "expires", "_at"].iter().any(|w| k.contains(w))
    };
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                match x {
                    serde_json::Value::String(s) if credential(k) && s.chars().count() >= 12 => {
                        out.push((p, s.clone()))
                    }
                    _ => credential_strings(x, &p, out),
                }
            }
        }
        serde_json::Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                credential_strings(x, &format!("{path}.{i}"), out);
            }
        }
        _ => {}
    }
}

impl SecretStore for Redacting {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        let v = self.inner.get(name)?;
        if let Some(v) = &v {
            self.learn(name, v);
        }
        Ok(v)
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        self.inner.set(name, value)?;
        self.learn(name, value);
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        self.inner.delete(name)?;
        // The value stays known: it may still be in a step's output or a log.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// A store whose `get` of `blocked` waits until `release` is sent (a keychain showing a prompt); other accounts
    /// answer at once.
    struct Blocking {
        blocked: &'static str,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl SecretStore for Blocking {
        fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
            if name == self.blocked {
                let _ = self.release.lock().unwrap().recv();
            }
            Ok(Some("tok".into()))
        }
        fn set(&self, _: &str, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
    }

    fn wait_unstuck(s: &TimeLimited) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while s.stuck_calls() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(s.stuck_calls(), 0);
    }

    #[test]
    fn a_stalled_keychain_times_out_naming_its_secret_and_only_then_fails_fast() {
        let (release, rx) = std::sync::mpsc::channel();
        let store = Blocking { blocked: UPDATE_TOKEN, release: Mutex::new(rx) };
        // A 1 s limit leaves room to tell "failed at once" from "waited for the limit" on a loaded CI runner.
        let s = TimeLimited::new(Arc::new(store), Duration::from_secs(1));
        let t = Instant::now();
        let e = s.get(UPDATE_TOKEN).unwrap_err().0;
        assert!(t.elapsed() >= Duration::from_secs(1), "waited for the limit");
        assert!(t.elapsed() < Duration::from_secs(5));
        assert!(e.contains("did not answer in time for the update token"), "{e}");
        let t = Instant::now();
        let e = s.get(UPDATE_TOKEN).unwrap_err().0;
        assert!(e.contains("the update token"), "{e}");
        assert!(t.elapsed() < Duration::from_millis(500), "fails at once while the first call is stuck");
        assert_eq!(s.stuck_calls(), 1, "no second thread");
        assert_eq!(s.get(GITHUB_LOGIN).unwrap().as_deref(), Some("tok"), "other secrets still answer");
        release.send(()).unwrap();
        wait_unstuck(&s);
        release.send(()).unwrap();
        assert_eq!(s.get(UPDATE_TOKEN).unwrap().as_deref(), Some("tok"), "answers again once the prompt is dealt with");
    }

    /// A store whose `get` returns only once two calls are inside it at the same time.
    struct Together(std::sync::Barrier);
    impl SecretStore for Together {
        fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
            self.0.wait();
            Ok(Some(name.to_string()))
        }
        fn set(&self, _: &str, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
    }

    #[test]
    fn overlapping_reads_of_different_secrets_both_succeed() {
        let s = Arc::new(TimeLimited::new(Arc::new(Together(std::sync::Barrier::new(2))), Duration::from_secs(5)));
        let other = s.clone();
        let t = std::thread::spawn(move || other.get(GITHUB_LOGIN));
        assert_eq!(s.get(UPDATE_TOKEN).unwrap().as_deref(), Some(UPDATE_TOKEN));
        assert_eq!(t.join().unwrap().unwrap().as_deref(), Some(GITHUB_LOGIN));
        assert_eq!(s.stuck_calls(), 0);
    }

    /// Every `get` waits until `release` is dropped.
    struct AllBlocked(Mutex<std::sync::mpsc::Receiver<()>>);
    impl SecretStore for AllBlocked {
        fn get(&self, _: &str) -> Result<Option<String>, SecretError> {
            let _ = self.0.lock().unwrap().recv();
            Ok(None)
        }
        fn set(&self, _: &str, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
    }

    #[test]
    fn too_many_stuck_calls_make_every_call_fail_fast() {
        let (release, rx) = std::sync::mpsc::channel::<()>();
        let s = TimeLimited::new(Arc::new(AllBlocked(Mutex::new(rx))), Duration::from_millis(100));
        for i in 0..MAX_STUCK {
            assert!(s.get(&format!("secret:{i}")).is_err());
        }
        assert_eq!(s.stuck_calls(), MAX_STUCK);
        let t = Instant::now();
        let e = s.get("secret:new").unwrap_err().0;
        assert!(t.elapsed() < Duration::from_millis(500) && e.contains("earlier calls have not answered"), "{e}");
        drop(release);
        wait_unstuck(&s);
    }

    #[test]
    fn stalled_errors_name_the_secret() {
        assert_eq!(secret_label("update-token:0123456789ab"), "the update token");
        assert_eq!(secret_label(GITHUB_LOGIN), "the GitHub login");
        assert_eq!(secret_label("github-loginx"), "a secret");
        assert_eq!(secret_label("forge-logins:0123456789ab"), "the forge logins");
        let e = stalled(secret_label("connection:1")).naming("connection:1", "the secret of the connection jira");
        assert!(e.0.contains("did not answer in time for the secret of the connection jira"), "{}", e.0);
        assert_eq!(
            e.0.contains("macOS"),
            cfg!(target_os = "macos"),
            "only macOS asks someone to allow access: {}",
            e.0
        );
    }

    /// Counts the calls that reach it.
    #[derive(Default)]
    struct Counting {
        inner: MemorySecretStore,
        gets: std::sync::atomic::AtomicUsize,
    }
    impl SecretStore for Counting {
        fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
            self.gets.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.get(name)
        }
        fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
            self.inner.set(name, value)
        }
        fn delete(&self, name: &str) -> Result<(), SecretError> {
            self.inner.delete(name)
        }
    }

    #[test]
    fn a_cached_secret_survives_polling_and_a_change_replaces_it() {
        let inner = Arc::new(Counting::default());
        inner.inner.set(GITHUB_LOGIN, "old").unwrap();
        let s = Cached::new(inner.clone());
        let gets = || inner.gets.load(std::sync::atomic::Ordering::SeqCst);
        for _ in 0..10 {
            assert_eq!(s.get(GITHUB_LOGIN).unwrap().as_deref(), Some("old"));
        }
        assert_eq!(gets(), 1, "polling reads the keychain once");
        s.set(GITHUB_LOGIN, "new").unwrap();
        assert_eq!(s.get(GITHUB_LOGIN).unwrap().as_deref(), Some("new"), "reconnecting replaces it");
        s.delete(GITHUB_LOGIN).unwrap();
        assert_eq!(s.get(GITHUB_LOGIN).unwrap(), None, "signing out forgets it");
        assert_eq!(gets(), 1);
        assert_eq!(s.get(UPDATE_TOKEN).unwrap(), None);
        assert_eq!(s.get(UPDATE_TOKEN).unwrap(), None);
        assert_eq!(gets(), 2, "a missing secret is remembered too");
    }

    #[test]
    fn failed_reads_are_not_cached() {
        let (release, rx) = std::sync::mpsc::channel();
        let store = Blocking { blocked: UPDATE_TOKEN, release: Mutex::new(rx) };
        let limited = Arc::new(TimeLimited::new(Arc::new(store), Duration::from_millis(200)));
        let s = Cached::new(limited.clone());
        assert!(s.get(UPDATE_TOKEN).is_err());
        release.send(()).unwrap();
        wait_unstuck(&limited);
        release.send(()).unwrap();
        assert_eq!(s.get(UPDATE_TOKEN).unwrap().as_deref(), Some("tok"), "asked again");
    }

    #[test]
    fn time_limited_passes_calls_through() {
        let s = TimeLimited::new(Arc::new(MemorySecretStore::default()), Duration::from_secs(5));
        s.set("a", "1").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("1"));
        s.delete("a").unwrap();
        assert_eq!(s.get("a").unwrap(), None);
    }

    #[test]
    fn token_accounts_are_per_data_dir() {
        let tmp = std::env::temp_dir();
        let stable = tmp.join("pdt-acct-stable");
        assert_eq!(update_token_account(&stable, Some(&stable)), UPDATE_TOKEN);
        let other = update_token_account(&tmp.join("pdt-acct-other"), Some(&stable));
        assert!(other.starts_with("update-token:") && other.len() == "update-token:".len() + 12, "{other}");
        assert_eq!(other, update_token_account(&tmp.join("pdt-acct-other"), Some(&stable)), "stable across calls");
        assert_ne!(other, update_token_account(&tmp.join("pdt-acct-third"), Some(&stable)));
        assert_ne!(update_token_account(&stable, None), UPDATE_TOKEN, "no stable dir known: never the shared account");
    }

    #[test]
    fn memory_store_roundtrip() {
        let s = MemorySecretStore::default();
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "1").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("1"));
        s.delete("a").unwrap();
        s.delete("a").unwrap();
        assert_eq!(s.get("a").unwrap(), None);
    }

    #[test]
    fn only_linux_without_a_secret_service_falls_back_to_the_file() {
        let asked = std::cell::Cell::new(false);
        let probe = |found: bool| {
            asked.set(true);
            found
        };
        let new = StoreHistory::default();
        let file = StoreHistory { file: true, keychain: false };
        // macOS and Windows: the keychain, never asked about and never a file, even when told or found.
        assert_eq!(choose_backend(false, None, new, || probe(false)), SecretBackend::Keychain);
        assert_eq!(choose_backend(false, Some("file"), new, || probe(false)), SecretBackend::Keychain);
        assert_eq!(choose_backend(false, None, file, || probe(false)), SecretBackend::Keychain);
        assert!(!asked.get(), "only Linux looks for a Secret Service");
        assert_eq!(choose_backend(true, None, new, || probe(true)), SecretBackend::Keychain);
        assert_eq!(choose_backend(true, None, new, || probe(false)), SecretBackend::File);
        assert_eq!(choose_backend(true, Some("file"), new, || true), SecretBackend::File);
        for linux in [true, false] {
            assert_eq!(choose_backend(linux, Some("memory"), file, || true), SecretBackend::Memory);
        }
    }

    #[test]
    fn linux_keeps_the_store_it_used_before() {
        let never = || -> bool { panic!("a data folder with a store never asks again") };
        // The file stays the store even once a Secret Service appears (a desktop login after a headless start).
        let file = StoreHistory { file: true, keychain: false };
        assert_eq!(choose_backend(true, None, file, never), SecretBackend::File);
        // The keychain stays the store even when it does not answer (gnome-keyring not up or locked yet).
        let keychain = StoreHistory { file: false, keychain: true };
        assert_eq!(choose_backend(true, None, keychain, never), SecretBackend::Keychain);
        // Both (the file was forced once): the file, where secrets went last.
        assert_eq!(choose_backend(true, None, StoreHistory { file: true, keychain: true }, never), SecretBackend::File);
        // Memory for tests overrides any history.
        assert_eq!(choose_backend(true, Some("memory"), keychain, never), SecretBackend::Memory);
    }

    #[test]
    fn an_ensured_file_store_exists_empty_and_keeps_its_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let path = secrets_file(tmp.path());
        let s = FileSecretStore::new(path.clone());
        s.ensure().unwrap();
        assert!(path.is_file());
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "1").unwrap();
        s.ensure().unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("1"), "ensure never empties it");
    }

    #[test]
    fn file_store_roundtrip_is_private_to_the_user() {
        let tmp = tempfile::tempdir().unwrap();
        let path = secrets_file(&tmp.path().join("data"));
        let s = FileSecretStore::new(path.clone());
        assert_eq!(s.get("a").unwrap(), None);
        s.delete("a").unwrap();
        assert!(!path.exists(), "nothing is written until there is a secret");
        s.set("a", "1").unwrap();
        s.set("b", "2").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Another instance on the same file sees them (pdtd restarted).
        let again = FileSecretStore::new(path.clone());
        assert_eq!(again.get("b").unwrap().as_deref(), Some("2"));
        again.delete("a").unwrap();
        assert_eq!(s.get("a").unwrap(), None);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::write(&path, "not json").unwrap();
        assert!(s.get("b").is_err(), "a damaged file is an error, never an empty store");
    }

    /// Touches the real keychain: run by hand only (`cargo test -p pdt-api keyring -- --ignored`).
    #[test]
    #[ignore]
    fn keyring_store_roundtrip() {
        let s = KeyringStore::new("dev.personaldevteam.test");
        s.set("probe", "v").unwrap();
        assert_eq!(s.get("probe").unwrap().as_deref(), Some("v"));
        s.delete("probe").unwrap();
        assert_eq!(s.get("probe").unwrap(), None);
    }

    #[test]
    fn values_pdt_keeps_for_itself_are_redacted() {
        let r = Arc::new(pdt_core::secret::Redactor::default());
        let s = Redacting::new(Arc::new(MemorySecretStore::default()), r.clone());
        s.set("connection:c1", "conn-secret-value").unwrap();
        let login = serde_json::json!({"accounts": {"github:github.com": {
            "token": "gho_0123456789abcdef", "api_url": "https://api.github.com", "login": "octocat",
            "refresh": {"refresh_token": "ghr_0123456789abcdef", "token_endpoint": "https://github.com/x/token-path"},
        }}});
        s.set(FORGE_LOGINS, &login.to_string()).unwrap();
        let text =
            "a conn-secret-value b gho_0123456789abcdef c ghr_0123456789abcdef octocat https://github.com/x/token-path";
        assert_eq!(
            r.string(text),
            "a [secret:connection] b [secret:forge-login] c [secret:forge-login] octocat https://github.com/x/token-path"
        );
        // A value read (pdtd starting with the keychain full) is learnt too; the vault's own are left to it.
        let inner = Arc::new(MemorySecretStore::default());
        inner.set(UPDATE_TOKEN, "github_pat_update_0123").unwrap();
        inner.set("secret:s1", "vault-value-1").unwrap();
        let s = Redacting::new(inner, r.clone());
        s.get(UPDATE_TOKEN).unwrap();
        s.get("secret:s1").unwrap();
        assert_eq!(r.string("github_pat_update_0123 vault-value-1"), "[secret:update-token] vault-value-1");
        // A new login replaces the old one's tokens.
        let s = Redacting::new(Arc::new(MemorySecretStore::default()), r.clone());
        s.set("connection:c1", "conn-secret-value-2").unwrap();
        assert_eq!(r.string("conn-secret-value"), "conn-secret-value");
    }
}
