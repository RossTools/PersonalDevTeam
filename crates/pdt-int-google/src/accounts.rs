//! Google accounts (PDT-316): one `google` connection per account. The connection holds its OAuth client — PDT's own
//! ([`crate::builtin`], PDT-360) when it names none, else the user's (its ID in the config, its secret as the
//! connection's secret, with the Picker's API key and project number) — and the Drive folder new documents go in.
//! Signing in adds the account's refresh token and email, kept as a secret of their own ([`sign_in_key`]) so editing
//! the connection never loses them. With PDT's own clients, agents have a sign-in of their own too, with the agentic
//! client ([`agent_key`]): the account's agent permission.

use crate::api::{Account, Tokens, token_key};
use crate::builtin::{BuiltIn, Client};
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

/// Where an account's agent sign-in (its agent permission, with PDT's agentic client) is kept.
pub fn agent_key(connection_id: &str) -> String {
    format!("connection:{connection_id}:google-agent")
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
    /// The account signing in; None for a new one, added once Google says who it is.
    pub connection_id: Option<String>,
    /// PKCE's verifier: the code Google sends back is useless without it.
    pub verifier: String,
    pub redirect_uri: String,
    /// The client it signs in with.
    pub client: Client,
    /// The agent permission, with PDT's agentic client.
    pub agent: bool,
    pub started_ms: i64,
}

/// How long a sign-in may take.
pub const PENDING_MS: i64 = 15 * 60 * 1000;

/// What the provider and the routes share: the way to Google, PDT's own clients (when built with them), access
/// tokens, the accounts' emails (for the embedded editor), and sign-ins under way.
pub struct Shared {
    pub transport: Arc<dyn Transport>,
    pub built_in: Option<BuiltIn>,
    pub tokens: Tokens,
    pub emails: RwLock<HashMap<String, String>>,
    pub pending: Mutex<HashMap<String, Pending>>,
}

impl Shared {
    pub fn new(transport: Arc<dyn Transport>, built_in: Option<BuiltIn>) -> Self {
        Self { transport, built_in, tokens: Tokens::default(), emails: Default::default(), pending: Default::default() }
    }
    /// Whether the account signs in with PDT's own clients (it names no client of its own).
    pub fn uses_built_in(&self, c: &Connection) -> bool {
        self.built_in.is_some() && config_str(c, "clientId").is_none()
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
    sign_in_at(st, sign_in_key(connection_id)).await
}

/// The account's agent sign-in, if agents may use it.
pub async fn agent_sign_in(st: &AppState, connection_id: &str) -> Result<Option<SignIn>, ProviderError> {
    sign_in_at(st, agent_key(connection_id)).await
}

async fn sign_in_at(st: &AppState, key: String) -> Result<Option<SignIn>, ProviderError> {
    Ok(secret(st, key).await?.and_then(|s| serde_json::from_str(&s).ok()))
}

/// Keeps a sign-in under `key` ([`sign_in_key`] or [`agent_key`]).
pub async fn store_sign_in(st: &AppState, key: String, s: &SignIn) -> Result<(), String> {
    let (secrets, value) = (st.secrets.clone(), serde_json::to_string(s).unwrap());
    tokio::task::spawn_blocking(move || secrets.set(&key, &value))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

async fn forget(st: &AppState, key: String) -> Result<(), String> {
    let secrets = st.secrets.clone();
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

/// The OAuth client a connection signs in with: its own (the ID in its config, the secret as its secret), else PDT's
/// standard or agentic one.
pub async fn client(st: &AppState, shared: &Shared, c: &Connection, agent: bool) -> Result<Client, ProviderError> {
    let Some(id) = config_str(c, "clientId") else {
        return match &shared.built_in {
            Some(b) => Ok(if agent { b.agent.clone() } else { b.standard.clone() }),
            None => Err(ProviderError::Failed(format!(
                "the Google connection {} has no OAuth client ID (this PDT was built without its own Google sign-in)",
                c.name
            ))),
        };
    };
    let (_, s) = pdt_api::sdk::connections::connection_with_secret(st, &c.name).await.map_err(ProviderError::Failed)?;
    let secret = s
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProviderError::Failed(format!("the Google connection {} has no OAuth client secret", c.name)))?;
    Ok(Client { id, secret })
}

/// The signed-in account of a connection, ready to call Google; for an agent's document tools (`agent`), with the
/// account's agent sign-in when it uses PDT's own clients, never the user's. With its own client an account has one
/// sign-in, which agents use too.
pub async fn account(st: &AppState, shared: &Shared, c: &Connection, agent: bool) -> Result<Account, ProviderError> {
    let agent = agent && shared.uses_built_in(c);
    shared.tokens.redact_with(st.store.redactor());
    let s = sign_in(st, &c.id).await?.ok_or_else(|| {
        ProviderError::Failed(format!(
            "the Google connection {} isn't signed in: sign in in Settings → Plugins → Google",
            c.name
        ))
    })?;
    if let Some(e) = &s.email {
        shared.emails.write().unwrap_or_else(|e| e.into_inner()).insert(c.id.clone(), e.clone());
    }
    let refresh_token = if agent {
        agent_sign_in(st, &c.id).await?.ok_or_else(|| ProviderError::Failed(no_agents(&c.name)))?.refresh_token
    } else {
        s.refresh_token
    };
    let client = client(st, shared, c, agent).await?;
    Ok(Account {
        connection_id: c.id.clone(),
        name: c.name.clone(),
        client_id: client.id,
        client_secret: client.secret,
        refresh_token,
        agent,
    })
}

/// What an agent is told about an account that doesn't let agents in.
pub fn no_agents(name: &str) -> String {
    format!(
        "agents may not use the Google account {name}: the user allows them in Settings → Plugins → Google (Allow \
         agents). Ask the user to, or to make the change themselves."
    )
}

/// The account of a connection by id ([`account`]).
pub async fn account_by_id(
    st: &AppState,
    shared: &Shared,
    id: &str,
    agent: bool,
) -> Result<(Connection, Account), ProviderError> {
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
    let a = account(st, shared, &c, agent).await?;
    Ok((c, a))
}

/// Every signed-in account (with its agent permission, for an agent), by connection name.
pub async fn signed_in(
    st: &AppState,
    shared: &Shared,
    agent: bool,
) -> Result<Vec<(Connection, Account)>, ProviderError> {
    let mut out = vec![];
    for c in connections(st).await? {
        if sign_in(st, &c.id).await?.is_some() {
            match account(st, shared, &c, agent).await {
                Ok(a) => out.push((c, a)),
                Err(e) => tracing::debug!(connection = c.name, error = %e, "a Google account can't be used"),
            }
        }
    }
    Ok(out)
}

/// Takes the agents' permission away: PDT forgets the agent sign-in. Google isn't asked to revoke it, which would
/// revoke every client of PDT's project and so sign the user out too.
pub async fn forget_agents(st: &AppState, shared: &Shared, connection_id: &str) -> Result<(), String> {
    forget(st, agent_key(connection_id)).await?;
    shared.tokens.forget(&token_key(connection_id, true));
    Ok(())
}

/// Signs an account out: Google takes PDT's access back (when it can be reached; a failure is only logged), and PDT
/// forgets the sign-in, the agents' and their tokens.
pub async fn sign_out(st: &AppState, shared: &Shared, connection_id: &str) -> Result<(), String> {
    if let Some(s) = sign_in(st, connection_id).await.map_err(|e| e.to_string())? {
        let transport = shared.transport.clone();
        let revoked =
            tokio::task::spawn_blocking(move || crate::oauth::revoke(transport.as_ref(), &s.refresh_token)).await;
        if let Ok(Err(e)) = revoked {
            tracing::info!(error = %e, "Google didn't take back PDT's access; signing out anyway");
        }
    }
    forget(st, sign_in_key(connection_id)).await?;
    forget_agents(st, shared, connection_id).await?;
    shared.tokens.forget(connection_id);
    shared.emails.write().unwrap_or_else(|e| e.into_inner()).remove(connection_id);
    shared
        .pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, p| p.connection_id.as_deref() != Some(connection_id));
    Ok(())
}
