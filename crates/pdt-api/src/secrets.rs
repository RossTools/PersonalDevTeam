//! Secrets (the update token, the GitHub login) live only in the OS keychain / credential manager (spec §3.2). Never logged, never
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

/// What a keychain call that ran out of time reports (after "the OS keychain failed: ").
pub const KEYCHAIN_STALLED: &str =
    "it did not answer in time (macOS may be waiting for someone at this Mac to allow access to the update token)";

/// Bounds every call to `inner` by `timeout`, so a keychain waiting on an approval prompt nobody sees cannot hang
/// pdtd. A call that times out keeps running on its own thread; until it returns, further calls fail at once
/// instead of piling up more stuck threads.
pub struct TimeLimited {
    inner: std::sync::Arc<dyn SecretStore>,
    timeout: std::time::Duration,
    busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl TimeLimited {
    pub fn new(inner: std::sync::Arc<dyn SecretStore>, timeout: std::time::Duration) -> Self {
        Self { inner, timeout, busy: Default::default() }
    }

    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn SecretStore) -> Result<T, SecretError> + Send + 'static,
    ) -> Result<T, SecretError> {
        use std::sync::atomic::Ordering;
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(SecretError(KEYCHAIN_STALLED.into()));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (inner, busy) = (self.inner.clone(), self.busy.clone());
        std::thread::spawn(move || {
            let r = f(inner.as_ref());
            busy.store(false, Ordering::SeqCst);
            let _ = tx.send(r);
        });
        rx.recv_timeout(self.timeout).unwrap_or_else(|_| Err(SecretError(KEYCHAIN_STALLED.into())))
    }
}

impl SecretStore for TimeLimited {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        let name = name.to_string();
        self.call(move |s| s.get(&name))
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        let (name, value) = (name.to_string(), value.to_string());
        self.call(move |s| s.set(&name, &value))
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        let name = name.to_string();
        self.call(move |s| s.delete(&name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// A store whose `get` waits until `release` is sent (a keychain showing a prompt).
    struct Blocking(Mutex<std::sync::mpsc::Receiver<()>>);
    impl SecretStore for Blocking {
        fn get(&self, _: &str) -> Result<Option<String>, SecretError> {
            let _ = self.0.lock().unwrap().recv();
            Ok(Some("tok".into()))
        }
        fn set(&self, _: &str, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
    }

    #[test]
    fn a_stalled_keychain_times_out_and_later_calls_fail_fast_until_it_answers() {
        let (release, rx) = std::sync::mpsc::channel();
        // A 1 s limit leaves room to tell "failed at once" from "waited for the limit" on a loaded CI runner.
        let s = TimeLimited::new(Arc::new(Blocking(Mutex::new(rx))), Duration::from_secs(1));
        let t = Instant::now();
        assert_eq!(s.get("a").unwrap_err().0, KEYCHAIN_STALLED);
        assert!(t.elapsed() < Duration::from_secs(5));
        let t = Instant::now();
        assert_eq!(s.get("a").unwrap_err().0, KEYCHAIN_STALLED, "still stuck: no second thread");
        assert!(t.elapsed() < Duration::from_millis(500), "fails at once while the first call is stuck");
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while s.busy.load(std::sync::atomic::Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        release.send(()).unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("tok"), "answers again once the prompt is dealt with");
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
}
