//! Secrets for sessions on a remote instance (PDT-637). A remote session's `run_with_secrets` or `write_secret_file`
//! is answered by the owner (it asks the user first, as for a local one), then runs on the remote instance in the
//! session's folder: the owner sends its own values for that one call (never for a name the remote instance keeps
//! itself, which wins), and the remote instance adds the secrets it keeps for the project, never sent anywhere.
//! Forwarded values live only in this call's memory there: never stored, logged or given to its redactor, while the
//! output comes back redacted of both. Uses are recorded on the owner (forwarded ones with the instance) and on the
//! remote instance (its own, also reported to the owner by name).

use super::*;
use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use crate::vault::{RunWithSecrets, WriteSecretFile};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, Method};
use pdt_core::link::OwnerLink;
use pdt_core::secret::{self, Redactor, RemoteSecretName, RemoteSecretSet, SecretUse};
use pdt_core::session::Session;
use std::collections::HashMap;
use std::time::Duration;

/// How long a file's write may take there (a `run_with_secrets` gets its own time limit on top).
const CALL_SLACK: Duration = Duration::from_secs(60);

/// What a remote session's call does with its secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tool", content = "args", rename_all = "snake_case")]
pub(crate) enum SecretOp {
    #[serde(rename = "run_with_secrets")]
    Run(RunWithSecrets),
    #[serde(rename = "write_secret_file")]
    Write(WriteSecretFile),
}

impl SecretOp {
    fn how(&self) -> &'static str {
        match self {
            SecretOp::Run(_) => "run_with_secrets",
            SecretOp::Write(_) => "write_secret_file",
        }
    }

    fn names(&self) -> Result<Vec<String>, String> {
        match self {
            SecretOp::Run(a) => crate::vault::run_names(a),
            SecretOp::Write(a) => crate::vault::write_names(a),
        }
    }
}

/// `/link/exec/secrets`: one call of a session there, with the owner's values for the names it forwards.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SecretsReq {
    pub session_id: String,
    pub op: SecretOp,
    /// The owner's values, by name as the call wrote it: for this call only.
    pub values: HashMap<String, String>,
    /// Why the owner sent none for a name it was asked for (it has none, or this session may not use it): the
    /// remote instance says so unless it keeps that secret itself.
    pub refused: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SecretsReply {
    /// The tool's answer (redacted there), or why it failed.
    pub result: Result<Value, String>,
    /// The names it used its own secrets for.
    pub local: Vec<String>,
    /// The names it used the owner's forwarded values for.
    pub forwarded: Vec<String>,
}

// ---- the owner ----

/// Runs a remote session's secret tool on its instance: the owner's values for the names the instance doesn't keep
/// (each checked against the session as for a local one), and the uses recorded once it says which it took.
pub(crate) async fn use_secrets(st: &AppState, s: &Session, op: SecretOp, names: Vec<String>) -> Result<Value, String> {
    let inst = s.instance.as_ref().ok_or("this session runs on this instance")?;
    let i = owner::instance(st, &s.project_id, &inst.id).await.map_err(|e| e.message().to_string())?;
    let mut values = HashMap::new();
    let mut refused = HashMap::new();
    let mut found = HashMap::new();
    for n in secret::to_forward(&names, &i.secrets) {
        match crate::vault::lookup(st, &n, Some(crate::vault::session_user(s))).await {
            Ok((secret, v)) => {
                values.insert(n.clone(), v);
                found.insert(n, secret);
            }
            Err(why) => {
                refused.insert(n, why);
            }
        }
    }
    let timeout = match &op {
        SecretOp::Run(a) => crate::vault::run_limit(a) + CALL_SLACK,
        SecretOp::Write(_) => CALL_SLACK,
    };
    let how = op.how();
    let req = SecretsReq { session_id: s.id.clone(), op, values, refused };
    let reply: SecretsReply = owner::call(st, &i, SECRETS_PATH, &req, timeout)
        .await
        .map_err(|e| e.into_api(&i.name).message().to_string())?;
    let usage = crate::vault::session_usage(s, how);
    for n in &reply.forwarded {
        if let Some(secret) = found.get(n) {
            crate::vault::record_use(st, secret, &usage, Some(&i.name)).await;
        }
    }
    for n in &reply.local {
        let u = pdt_store::RemoteSecretUse {
            link_id: i.link_id.clone(),
            name: n.clone(),
            at: now_ms(),
            by: usage.by.clone(),
            how: how.into(),
            session_id: Some(s.id.clone()),
        };
        if let Err(e) = st.store.record_remote_secret_use(&u).await {
            tracing::warn!(secret = %n, instance = %i.name, error = %e, "could not record a remote secret's use");
        }
    }
    // Redacted there already; the owner's own values are taken out again here.
    reply.result.map(|v| st.store.redactor().json(&v))
}

/// `GET /api/secrets/remote`: the secrets each joined instance keeps for this instance's projects, by name.
pub(crate) async fn remote_secrets(State(st): State<AppState>) -> Result<Json<Vec<RemoteSecretSet>>, ApiError> {
    let mut out = vec![];
    for i in st.store.list_remote_instances(None).await? {
        if i.secrets.is_empty() {
            continue;
        }
        let Some(p) = st.store.get_project(&i.project_id).await? else { continue };
        let last = st.store.last_remote_secret_uses(&i.link_id).await?;
        let secrets = i
            .secrets
            .iter()
            .map(|n| {
                let used = last.iter().find(|u| u.name.eq_ignore_ascii_case(n));
                RemoteSecretName {
                    name: n.clone(),
                    last_used_at: used.map(|u| u.at),
                    last_used_by: used.map(|u| u.by.clone()),
                }
            })
            .collect();
        out.push(RemoteSecretSet {
            instance_id: i.instance_id,
            instance_name: i.name,
            project_id: p.id,
            project_key: p.key,
            project_name: p.name,
            secrets,
        });
    }
    Ok(Json(out))
}

// ---- the remote instance ----

/// The secrets this instance keeps for the remote project `project_id` (its own id of it): their names.
pub(crate) async fn own_secret_names(st: &AppState, project_id: &str) -> Result<Vec<String>, ApiError> {
    Ok(st
        .store
        .list_secrets()
        .await?
        .into_iter()
        .filter(|s| secret::for_remote_project(s, project_id))
        .map(|s| s.name)
        .collect())
}

/// Its own values for those of `names` it keeps for the project, by its secret's name.
async fn own_values(
    st: &AppState,
    link: &OwnerLink,
    names: &[String],
) -> Result<(HashMap<String, String>, Vec<pdt_core::secret::Secret>), String> {
    let mut values = HashMap::new();
    let mut used = vec![];
    for s in st.store.list_secrets().await.map_err(|e| e.to_string())? {
        if secret::for_remote_project(&s, &link.project_id) && names.iter().any(|n| n.eq_ignore_ascii_case(&s.name)) {
            values.insert(s.name.clone(), crate::vault::value(st, &s).await?);
            used.push(s);
        }
    }
    Ok((values, used))
}

/// `/link/exec/secrets`: runs the call in the session's folder here.
pub(crate) async fn secrets_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<SecretsReply>, ApiError> {
    let (link, req): (OwnerLink, SecretsReq) = remote::checked(&st, &method, &uri, &headers, &body).await?;
    let e = st
        .store
        .get_exec_session(&req.session_id)
        .await?
        .filter(|e| e.link_id == link.link_id)
        .ok_or_else(|| ApiError::conflict(codes::REMOTE_PROJECT, "that session doesn't run here"))?;
    Ok(Json(run_here(&st, &link, &e, req).await))
}

async fn run_here(st: &AppState, link: &OwnerLink, e: &pdt_store::ExecSession, req: SecretsReq) -> SecretsReply {
    let failed = |why: String| SecretsReply { result: Err(why), local: vec![], forwarded: vec![] };
    let names = match req.op.names() {
        Ok(n) => n,
        Err(why) => return failed(why),
    };
    let (own, own_secrets) = match own_values(st, link, &names).await {
        Ok(v) => v,
        Err(why) => return failed(why),
    };
    let got = match secret::remote_values(&names, &own, &req.values, &req.refused) {
        Ok(g) => g,
        Err(why) => return failed(why),
    };
    // This call's own redactor: the forwarded values never reach this instance's, which stays as it is.
    let call_redactor = Redactor::default();
    for (n, v) in &got.values {
        call_redactor.set(n, v);
    }
    let mine = st.store.redactor();
    let redact = |text: &str, cut: bool| mine.string_cut(&call_redactor.string_cut(text, cut), cut);
    let how = req.op.how();
    let result = match &req.op {
        SecretOp::Run(a) => crate::vault::run_in(&e.cwd, a, &got.values, &redact).await,
        SecretOp::Write(a) => crate::vault::write_in(&e.cwd, a, &got.values, &names).await,
    }
    .map(|v| call_redactor.json(&v))
    .map_err(|why| call_redactor.string(&why));
    for n in &got.local {
        if let Some(s) = own_secrets.iter().find(|s| s.name.eq_ignore_ascii_case(n)) {
            let u = SecretUse {
                id: 0,
                secret_id: s.id.clone(),
                at: now_ms(),
                by: e.title.clone(),
                how: how.into(),
                session_id: Some(e.session_id.clone()),
                run_id: None,
                instance: None,
            };
            if let Err(err) = st.store.record_secret_use(&u).await {
                tracing::warn!(secret = %s.name, error = %err, "could not record a secret's use");
            }
        }
    }
    // Names only, never values.
    tracing::debug!(session = %e.session_id, tool = how, local = ?got.local, forwarded = ?got.forwarded,
        ok = result.is_ok(), "a remote session's secret tool ran");
    SecretsReply { result, local: got.local, forwarded: got.forwarded }
}
