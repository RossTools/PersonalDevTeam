//! Secrets (the update token) live only in the OS keychain / credential manager (spec §3.2). Never logged, never
//! returned by the API, never written to a file or the database.

use std::collections::HashMap;
use std::sync::Mutex;

/// The keychain account holding the stable instance's GitHub update token. See [`update_token_account`].
pub const UPDATE_TOKEN: &str = "update-token";

/// The keychain account for the update token of the pdtd using `data_dir`: [`UPDATE_TOKEN`] for the stable instance's
/// default data dir (`stable_data_dir`), else `update-token:<12 hex digits of the canonical data dir>`, so a dev
/// instance or a test never reads, replaces or deletes the stable instance's token.
pub fn update_token_account(data_dir: &std::path::Path, stable_data_dir: Option<&std::path::Path>) -> String {
    let canonical = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let dir = canonical(data_dir);
    if stable_data_dir.is_some_and(|s| canonical(s) == dir) {
        return UPDATE_TOKEN.to_string();
    }
    // FNV-1a (64-bit): stable across builds and platforms, unlike `DefaultHasher`.
    let hash = dir
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3));
    format!("{UPDATE_TOKEN}:{:012x}", hash >> 16)
}

#[derive(Debug, thiserror::Error)]
#[error("the OS keychain failed: {0}")]
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

#[cfg(test)]
mod tests {
    use super::*;

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
