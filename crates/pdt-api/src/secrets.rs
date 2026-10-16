//! Secrets (the update token) live only in the OS keychain / credential manager (spec §3.2). Never logged, never
//! returned by the API, never written to a file or the database.

use std::collections::HashMap;
use std::sync::Mutex;

/// The keychain account holding the GitHub update token.
pub const UPDATE_TOKEN: &str = "update-token";

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
