//! Google accounts (PDT-316): one `google` connection per account. The connection holds the user's OAuth client (its
//! ID in the config, its secret as the connection's secret), the Picker's API key and project number, and the Drive
//! folder new documents go in. Signing in adds the account's refresh token and email, kept as a secret of their own
//! ([`sign_in_key`]) so editing the connection never loses them.

use crate::api::{Account, Tokens};
use pdt_api::sdk::AppState;
use pdt_api::sdk::documents::ProviderError;
use pdt_core::workflow::api::Connection;
use pdt_forge::transport::Transport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

pub const KIND: &str = "google";

/// Where an account's sign-in is kept.
pub fn sign_in_key(connection_id: &str) -> String {
    format!("connection:{connection_id}:google")
}

/// What signing in keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignIn {
    pub refresh_token: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// A sign-in under way: the browser is at Google's consent page.
#[derive(Debug, Clone)]
pub struct Pending {
    pub connection_id: String,
    pub verifier: String,
    pub redirect_uri: String,
    pub started_ms: i64,
}

/// How long a sign-in may take.
pub const PENDING_MS: i64 = 15 * 60 * 1000;

/// What the provider and the routes share: the way to Google, access tokens, the accounts' emails (for the embedded
/// editor), and sign-ins under way.
pub struct Shared {
    pub transport: Arc<dyn Transport>,
    pub tokens: Tokens,
    pub emails: RwLock<HashMap<String, String>>,
    pub pending: Mutex<HashMap<String, Pending>>,
}

impl Shared {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport, tokens: Tokens::default(), emails: Default::default(), pending: Default::default() }
    }
    pub fn email(&self, connection_id: &str) -> Option<String> {
        self.emails.read().unwrap_or_else(|e| e.into_inner()).get(connection_id).cloned()
    }
}

pub fn config_str(c: &Connection, key: &str) -> Option<String> {
    c.config.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

async fn secret(st: &AppState, key: String) -> Result<Option<String>, ProviderError> {
    let secrets = st.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.get(&key))
        .await
        .map_err(|e| ProviderError::Failed(e.to_string()))?
        .map_err(|e| ProviderError::Failed(format!("PDT couldn't read a Google sign-in from the keychain: {e}")))
}

/// The account's sign-in, if it has one.
pub async fn sign_in(st: &AppState, connection_id: &str) -> Result<Option<SignIn>, ProviderError> {
    Ok(secret(st, sign_in_key(connection_id)).await?.and_then(|s| serde_json::from_str(&s).ok()))
}

pub async fn store_sign_in(st: &AppState, connection_id: &str, s: &SignIn) -> Result<(), String> {
    let (secrets, key, value) = (st.secrets.clone(), sign_in_key(connection_id), serde_json::to_string(s).unwrap());
    tokio::task::spawn_blocking(move || secrets.set(&key, &value))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

pub async fn forget_sign_in(st: &AppState, connection_id: &str) -> Result<(), String> {
    let (secrets, key) = (st.secrets.clone(), sign_in_key(connection_id));
    tokio::task::spawn_blocking(move || secrets.delete(&key))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// The Google connections, by name.
pub async fn connections(st: &AppState) -> Result<Vec<Connection>, ProviderError> {
    let mut all: Vec<Connection> = st
        .store
        .list_connections()
        .await
        .map_err(|e| ProviderError::Failed(e.to_string()))?
        .into_iter()
        .filter(|c| c.kind == KIND)
        .collect();
    all.sort_by_key(|c| c.name.to_lowercase());
    Ok(all)
}

/// The connection's OAuth client: its ID and secret.
pub async fn client(st: &AppState, c: &Connection) -> Result<(String, String), ProviderError> {
    let id = config_str(c, "clientId")
        .ok_or_else(|| ProviderError::Failed(format!("the Google connection {} has no OAuth client ID", c.name)))?;
    let (_, s) = pdt_api::sdk::connections::connection_with_secret(st, &c.name).await.map_err(ProviderError::Failed)?;
    let s = s
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProviderError::Failed(format!("the Google connection {} has no OAuth client secret", c.name)))?;
    Ok((id, s))
}

/// The signed-in account of a connection, ready to call Google.
pub async fn account(st: &AppState, shared: &Shared, c: &Connection) -> Result<Account, ProviderError> {
    let s = sign_in(st, &c.id).await?.ok_or_else(|| {
        ProviderError::Failed(format!(
            "the Google connection {} isn't signed in: sign in in Settings → Plugins → Google",
            c.name
        ))
    })?;
    if let Some(e) = &s.email {
        shared.emails.write().unwrap_or_else(|e| e.into_inner()).insert(c.id.clone(), e.clone());
    }
    let (client_id, client_secret) = client(st, c).await?;
    Ok(Account {
        connection_id: c.id.clone(),
        name: c.name.clone(),
        client_id,
        client_secret,
        refresh_token: s.refresh_token,
    })
}

/// The account of a connection by id.
pub async fn account_by_id(st: &AppState, shared: &Shared, id: &str) -> Result<(Connection, Account), ProviderError> {
    let c = st
        .store
        .get_connection(id)
        .await
        .map_err(|e| ProviderError::Failed(e.to_string()))?
        .filter(|c| c.kind == KIND)
        .ok_or_else(|| {
            ProviderError::Failed(
                "the Google account that holds this document is gone from PDT: add it again in Settings → Plugins → \
                 Google"
                    .into(),
            )
        })?;
    let a = account(st, shared, &c).await?;
    Ok((c, a))
}

/// Every signed-in account, by connection name.
pub async fn signed_in(st: &AppState, shared: &Shared) -> Result<Vec<(Connection, Account)>, ProviderError> {
    let mut out = vec![];
    for c in connections(st).await? {
        if sign_in(st, &c.id).await?.is_some() {
            match account(st, shared, &c).await {
                Ok(a) => out.push((c, a)),
                Err(e) => tracing::debug!(connection = c.name, error = %e, "a Google account can't be used"),
            }
        }
    }
    Ok(out)
}

/// Signs an account out: Google takes PDT's access back (when it can be reached; a failure is only logged), and PDT
/// forgets the sign-in and its tokens.
pub async fn sign_out(st: &AppState, shared: &Shared, connection_id: &str) -> Result<(), String> {
    if let Some(s) = sign_in(st, connection_id).await.map_err(|e| e.to_string())? {
        let transport = shared.transport.clone();
        let revoked =
            tokio::task::spawn_blocking(move || crate::oauth::revoke(transport.as_ref(), &s.refresh_token)).await;
        if let Ok(Err(e)) = revoked {
            tracing::info!(error = %e, "Google didn't take back PDT's access; signing out anyway");
        }
    }
    forget_sign_in(st, connection_id).await?;
    shared.tokens.forget(connection_id);
    shared.emails.write().unwrap_or_else(|e| e.into_inner()).remove(connection_id);
    shared.pending.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, p| p.connection_id != connection_id);
    Ok(())
}
