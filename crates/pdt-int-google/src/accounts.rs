//! Google accounts (PDT-316): one `google` connection per account. The connection holds its OAuth client — PDT's own
//! ([`crate::builtin`], PDT-360) when it names none, else the user's (its ID in the config, its secret as the
//! connection's secret, with the Picker's API key and project number) — and the Drive folder new documents go in.
//! Signing in adds the account's refresh token and email, kept as a secret of their own ([`sign_in_key`]) so editing
//! the connection never loses them. With PDT's own clients, agents have a sign-in of their own too, with the agentic
//! client ([`agent_key`]): the account's agent permission. A sign-in made on the same machine with PDT's Desktop
//! clients ([`crate::builtin::Desktop`], PDT-590) says so ([`SignIn::desktop`]): its refresh token works only with them.

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
    /// Made with PDT's Desktop client (signing in on the same machine), not its Web one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub desktop: bool,
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
    /// With PDT's Desktop client, straight back to PDT on this machine.
    pub desktop: bool,
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
/// standard or agentic one, the Desktop one for a sign-in made with it (`desktop`).
pub async fn client(
    st: &AppState,
    shared: &Shared,
    c: &Connection,
    agent: bool,
    desktop: bool,
) -> Result<Client, ProviderError> {
    let Some(id) = config_str(c, "clientId") else {
        return match &shared.built_in {
            Some(b) if desktop => b.desktop.as_ref().map(|d| d.client(agent).clone()).ok_or_else(|| {
                ProviderError::Failed(format!(
                    "the Google connection {} signed in with a client this build of PDT lacks: sign in again in \
                     Settings → Plugins → Google",
                    c.name
                ))
            }),
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
    let s = if agent {
        agent_sign_in(st, &c.id).await?.ok_or_else(|| ProviderError::Failed(no_agents(&c.name)))?
    } else {
        s
    };
    let client = client(st, shared, c, agent, s.desktop).await?;
    let refresh_token = s.refresh_token;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::REVOKE_URL;
    use pdt_forge::transport::Response;
    use pdt_forge::transport::testing::Recorded;
    use serde_json::json;

    struct Env {
        st: AppState,
        google: Arc<Recorded>,
        _tmp: tempfile::TempDir,
    }

    async fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let st = AppState::new(store, tmp.path().join("data"));
        Env { st, google: Arc::new(Recorded::new()), _tmp: tmp }
    }

    fn built_in() -> BuiltIn {
        BuiltIn::from_values([Some("std-id"), Some("std-secret"), Some("agent-id"), Some("agent-secret"), None, None])
            .unwrap()
    }

    fn shared(env: &Env, built_in: Option<BuiltIn>) -> Shared {
        Shared::new(env.google.clone(), built_in)
    }

    fn conn(id: &str, name: &str, kind: &str, config: Value) -> Connection {
        Connection {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            url: String::new(),
            config,
            has_secret: false,
            created_at: 0,
            updated_at: 0,
            last_error: None,
            last_error_at: None,
        }
    }

    /// A Google connection, with its client secret when given.
    async fn add(env: &Env, id: &str, name: &str, config: Value, secret: Option<&str>) -> Connection {
        let c = conn(id, name, KIND, config);
        env.st.store.insert_connection(&c).await.unwrap();
        if let Some(s) = secret {
            env.st.secrets.set(&format!("connection:{id}"), s).unwrap();
        }
        c
    }

    fn signed(token: &str, email: Option<&str>) -> SignIn {
        SignIn { refresh_token: token.into(), email: email.map(String::from), desktop: false }
    }

    fn failed(e: ProviderError) -> String {
        match e {
            ProviderError::Failed(m) => m,
            other => panic!("not a failure: {other:?}"),
        }
    }

    #[test]
    fn the_sign_ins_are_kept_apart_and_config_is_trimmed() {
        assert_eq!(sign_in_key("c1"), "connection:c1:google");
        assert_eq!(agent_key("c1"), "connection:c1:google-agent");
        let c = conn("c1", "Work", KIND, json!({"clientId": "  cid  ", "folder": "   ", "n": 3}));
        assert_eq!(config_str(&c, "clientId").as_deref(), Some("cid"));
        assert_eq!(config_str(&c, "folder"), None);
        assert_eq!(config_str(&c, "n"), None);
        assert_eq!(config_str(&c, "missing"), None);
        assert!(no_agents("Work").contains("Allow agents"));
    }

    #[tokio::test]
    async fn an_account_uses_pdts_clients_only_when_it_names_none() {
        let env = env().await;
        let own = conn("c1", "Own", KIND, json!({"clientId": "cid"}));
        let plain = conn("c2", "Plain", KIND, json!({"clientId": " "}));
        let with = shared(&env, Some(built_in()));
        assert!(!with.uses_built_in(&own));
        assert!(with.uses_built_in(&plain));
        assert!(!shared(&env, None).uses_built_in(&plain));
    }

    #[tokio::test]
    async fn a_sign_in_is_kept_and_read_back_beside_the_agents() {
        let env = env().await;
        assert_eq!(sign_in(&env.st, "c1").await.unwrap(), None);
        store_sign_in(&env.st, sign_in_key("c1"), &signed("r1", Some("me@x.com"))).await.unwrap();
        assert_eq!(sign_in(&env.st, "c1").await.unwrap(), Some(signed("r1", Some("me@x.com"))));
        assert_eq!(agent_sign_in(&env.st, "c1").await.unwrap(), None);
        store_sign_in(&env.st, agent_key("c1"), &signed("a1", None)).await.unwrap();
        assert_eq!(agent_sign_in(&env.st, "c1").await.unwrap(), Some(signed("a1", None)));
        // One with no email (kept before PDT kept them) reads back; one that isn't a sign-in is none.
        env.st.secrets.set(&sign_in_key("c2"), r#"{"refreshToken":"r2"}"#).unwrap();
        assert_eq!(sign_in(&env.st, "c2").await.unwrap(), Some(signed("r2", None)));
        env.st.secrets.set(&sign_in_key("c3"), "not json").unwrap();
        assert_eq!(sign_in(&env.st, "c3").await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_google_connections_come_by_name() {
        let env = env().await;
        add(&env, "c1", "work", json!({}), None).await;
        add(&env, "c2", "Home", json!({}), None).await;
        env.st.store.insert_connection(&conn("c3", "Alerts", "http", json!({}))).await.unwrap();
        let names: Vec<String> = connections(&env.st).await.unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["Home", "work"]);
    }

    #[tokio::test]
    async fn the_client_is_the_accounts_own_or_pdts() {
        let env = env().await;
        let (with, without) = (shared(&env, Some(built_in())), shared(&env, None));
        let plain = add(&env, "c1", "Plain", json!({}), None).await;
        let c = client(&env.st, &with, &plain, false, false).await.unwrap();
        assert_eq!((c.id.as_str(), c.secret.as_str()), ("std-id", "std-secret"));
        let c = client(&env.st, &with, &plain, true, false).await.unwrap();
        assert_eq!((c.id.as_str(), c.secret.as_str()), ("agent-id", "agent-secret"));
        let e = failed(client(&env.st, &without, &plain, false, false).await.unwrap_err());
        assert!(e.contains("Plain has no OAuth client ID"), "{e}");

        // Its own client, standard or agent alike, with its secret trimmed.
        let own = add(&env, "c2", "Own", json!({"clientId": "cid"}), Some("  csecret \n")).await;
        for agent in [false, true] {
            let c = client(&env.st, &with, &own, agent, false).await.unwrap();
            assert_eq!((c.id.as_str(), c.secret.as_str()), ("cid", "csecret"));
        }
        let blank = add(&env, "c3", "Blank", json!({"clientId": "cid"}), Some("   ")).await;
        let e = failed(client(&env.st, &with, &blank, false, false).await.unwrap_err());
        assert!(e.contains("Blank has no OAuth client secret"), "{e}");
        let none = add(&env, "c4", "None", json!({"clientId": "cid"}), None).await;
        assert!(
            failed(client(&env.st, &with, &none, false, false).await.unwrap_err()).contains("no OAuth client secret")
        );
    }

    #[tokio::test]
    async fn a_desktop_sign_in_refreshes_with_the_desktop_client() {
        let env = env().await;
        let desktop = crate::builtin::Desktop::from_values([Some("d-id"), Some("d-s"), Some("da-id"), Some("da-s")]);
        let sh = shared(&env, Some(BuiltIn { desktop, ..built_in() }));
        let c = add(&env, "c1", "Work", json!({}), None).await;
        let on_this_machine = SignIn { desktop: true, ..signed("user-r", None) };
        store_sign_in(&env.st, sign_in_key("c1"), &on_this_machine).await.unwrap();
        let a = account(&env.st, &sh, &c, false).await.unwrap();
        assert_eq!((a.client_id.as_str(), a.client_secret.as_str()), ("d-id", "d-s"));
        // The agents' sign-in names its own client: here the Web one, made from another device.
        store_sign_in(&env.st, agent_key("c1"), &signed("agent-r", None)).await.unwrap();
        assert_eq!(account(&env.st, &sh, &c, true).await.unwrap().client_id, "agent-id");
        store_sign_in(&env.st, agent_key("c1"), &SignIn { desktop: true, ..signed("agent-r", None) }).await.unwrap();
        assert_eq!(account(&env.st, &sh, &c, true).await.unwrap().client_id, "da-id");
        // A build without the Desktop clients asks to sign in again.
        let e = failed(account(&env.st, &shared(&env, Some(built_in())), &c, false).await.unwrap_err());
        assert!(e.contains("sign in again"), "{e}");
        // Kept only when true, so older sign-ins read back unchanged.
        assert!(!serde_json::to_string(&signed("r", None)).unwrap().contains("desktop"));
    }

    #[tokio::test]
    async fn an_agent_acts_with_its_own_sign_in_only_with_pdts_clients() {
        let env = env().await;
        let sh = shared(&env, Some(built_in()));
        let plain = add(&env, "c1", "Plain", json!({}), None).await;
        let e = failed(account(&env.st, &sh, &plain, false).await.unwrap_err());
        assert!(e.contains("Plain isn't signed in"), "{e}");

        store_sign_in(&env.st, sign_in_key("c1"), &signed("user-r", Some("me@x.com"))).await.unwrap();
        let a = account(&env.st, &sh, &plain, false).await.unwrap();
        assert_eq!((a.refresh_token.as_str(), a.client_id.as_str(), a.agent), ("user-r", "std-id", false));
        assert_eq!(sh.email("c1").as_deref(), Some("me@x.com"), "kept for the embedded editor");
        // An agent never gets the user's sign-in.
        let e = failed(account(&env.st, &sh, &plain, true).await.unwrap_err());
        assert_eq!(e, no_agents("Plain"));
        store_sign_in(&env.st, agent_key("c1"), &signed("agent-r", None)).await.unwrap();
        let a = account(&env.st, &sh, &plain, true).await.unwrap();
        assert_eq!((a.refresh_token.as_str(), a.client_id.as_str(), a.agent), ("agent-r", "agent-id", true));

        // With its own client the one sign-in is the agents' too.
        let own = add(&env, "c2", "Own", json!({"clientId": "cid"}), Some("csecret")).await;
        store_sign_in(&env.st, sign_in_key("c2"), &signed("own-r", None)).await.unwrap();
        let a = account(&env.st, &sh, &own, true).await.unwrap();
        assert_eq!((a.refresh_token.as_str(), a.client_id.as_str(), a.agent), ("own-r", "cid", false));
        assert_eq!((a.connection_id.as_str(), a.name.as_str()), ("c2", "Own"));
        assert_eq!(sh.email("c2"), None);
    }

    #[tokio::test]
    async fn an_account_by_id_must_still_be_a_google_one() {
        let env = env().await;
        let sh = shared(&env, Some(built_in()));
        add(&env, "c1", "Work", json!({}), None).await;
        store_sign_in(&env.st, sign_in_key("c1"), &signed("r", None)).await.unwrap();
        let (c, a) = account_by_id(&env.st, &sh, "c1", false).await.unwrap();
        assert_eq!((c.name.as_str(), a.refresh_token.as_str()), ("Work", "r"));
        env.st.store.insert_connection(&conn("h1", "Hook", "http", json!({}))).await.unwrap();
        for id in ["gone", "h1"] {
            let e = failed(account_by_id(&env.st, &sh, id, false).await.unwrap_err());
            assert!(e.contains("is gone from PDT"), "{id}: {e}");
        }
    }

    #[tokio::test]
    async fn signed_in_lists_the_accounts_that_can_be_used() {
        let env = env().await;
        let sh = shared(&env, Some(built_in()));
        add(&env, "c1", "B", json!({}), None).await;
        add(&env, "c2", "a", json!({}), None).await;
        add(&env, "c3", "C (not signed in)", json!({}), None).await;
        // Its own client with no secret: signed in, but unusable.
        add(&env, "c4", "D", json!({"clientId": "cid"}), None).await;
        for id in ["c1", "c2", "c4"] {
            store_sign_in(&env.st, sign_in_key(id), &signed(&format!("r-{id}"), None)).await.unwrap();
        }
        let names = |all: Vec<(Connection, Account)>| all.into_iter().map(|(c, _)| c.name).collect::<Vec<_>>();
        assert_eq!(names(signed_in(&env.st, &sh, false).await.unwrap()), ["a", "B"]);
        // For agents, only an account that lets them in.
        store_sign_in(&env.st, agent_key("c1"), &signed("agent", None)).await.unwrap();
        let agents = signed_in(&env.st, &sh, true).await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!((agents[0].0.name.as_str(), agents[0].1.refresh_token.as_str()), ("B", "agent"));
    }

    fn pending(connection_id: Option<&str>) -> Pending {
        Pending {
            connection_id: connection_id.map(String::from),
            verifier: "v".into(),
            redirect_uri: "http://127.0.0.1/api/google/callback".into(),
            client: built_in().standard,
            agent: false,
            desktop: false,
            started_ms: 0,
        }
    }

    #[tokio::test]
    async fn signing_out_takes_the_access_back_and_forgets_everything() {
        let env = env().await;
        let sh = shared(&env, Some(built_in()));
        let c = add(&env, "c1", "Work", json!({}), None).await;
        store_sign_in(&env.st, sign_in_key("c1"), &signed("user-r", Some("me@x.com"))).await.unwrap();
        store_sign_in(&env.st, agent_key("c1"), &signed("agent-r", None)).await.unwrap();
        store_sign_in(&env.st, sign_in_key("c2"), &signed("other", None)).await.unwrap();
        account(&env.st, &sh, &c, false).await.unwrap();
        {
            let mut p = sh.pending.lock().unwrap();
            p.insert("s1".into(), pending(Some("c1")));
            p.insert("s2".into(), pending(Some("c2")));
            p.insert("s3".into(), pending(None));
        }
        env.google.ok("POST", REVOKE_URL, "{}");
        sign_out(&env.st, &sh, "c1").await.unwrap();
        assert_eq!(env.google.urls(), [format!("POST {REVOKE_URL}?token=user-r")]);
        assert_eq!(sign_in(&env.st, "c1").await.unwrap(), None);
        assert_eq!(agent_sign_in(&env.st, "c1").await.unwrap(), None);
        assert_eq!(sh.email("c1"), None);
        let mut left: Vec<String> = sh.pending.lock().unwrap().keys().cloned().collect();
        left.sort();
        assert_eq!(left, ["s2", "s3"], "only its own sign-ins under way are dropped");
        assert!(sign_in(&env.st, "c2").await.unwrap().is_some(), "another account is untouched");

        // Signed out already: nothing to revoke, and it still works.
        sign_out(&env.st, &sh, "c1").await.unwrap();
        assert_eq!(env.google.sent().len(), 1);
    }

    #[tokio::test]
    async fn signing_out_works_when_google_cant_take_the_access_back() {
        let env = env().await;
        let sh = shared(&env, None);
        store_sign_in(&env.st, sign_in_key("c1"), &signed("r", None)).await.unwrap();
        env.google.on("POST", REVOKE_URL, Response::new(500, "{}"));
        sign_out(&env.st, &sh, "c1").await.unwrap();
        assert_eq!(sign_in(&env.st, "c1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn taking_the_agents_permission_away_keeps_the_users_sign_in() {
        let env = env().await;
        let sh = shared(&env, Some(built_in()));
        store_sign_in(&env.st, sign_in_key("c1"), &signed("user-r", None)).await.unwrap();
        store_sign_in(&env.st, agent_key("c1"), &signed("agent-r", None)).await.unwrap();
        forget_agents(&env.st, &sh, "c1").await.unwrap();
        assert_eq!(agent_sign_in(&env.st, "c1").await.unwrap(), None);
        assert_eq!(sign_in(&env.st, "c1").await.unwrap(), Some(signed("user-r", None)));
        assert!(env.google.sent().is_empty(), "Google isn't asked: it would sign the user out too");
    }
}
