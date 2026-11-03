//! Trust in the commands of a run configuration (PDT-405; the rules are `pdt_core::command_trust`). A run, a test, the
//! local forge's CI and a regeneration rule run only commands the user approved; the approval is the user's click in
//! PDT (the UI token, `untrusted::from_ui`), never an agent's call. What waits on one is in the Inbox as a notice.

use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use pdt_core::command_trust::{self, ConfigCommand};
use pdt_core::inbox::{InboxGroup, InboxItem, InboxKind, InboxStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use ts_rs::TS;

/// The Inbox notice's key for commands waiting on the user's approval.
const NOTICE_KEY: &str = "untrusted-commands";

/// The fingerprints of the project's approved commands. A project added before approvals were kept trusts its main
/// checkout's commands the first time (the user has been running them).
pub(crate) async fn trusted(st: &AppState, project_id: &str, repo_path: &str) -> Result<HashSet<String>, ApiError> {
    if st.store.has_legacy_trust(project_id).await? {
        // An invalid file runs nothing yet; the mark waits for one that can be read.
        if let Ok(c) = crate::runconfig::load_effective(st, project_id, std::path::Path::new(repo_path)).await {
            let main: Vec<String> = command_trust::of_config(&c).into_iter().map(|c| c.fingerprint).collect();
            st.store.replace_legacy_trust(project_id, &main, now_ms()).await?;
        }
    }
    Ok(st.store.trusted_commands(project_id).await?)
}

/// The commands of `commands` the user has not approved.
pub(crate) async fn untrusted(
    st: &AppState,
    project_id: &str,
    repo_path: &str,
    commands: Vec<ConfigCommand>,
) -> Result<Vec<ConfigCommand>, ApiError> {
    Ok(command_trust::untrusted(commands, &trusted(st, project_id, repo_path).await?))
}

/// Ok when the user approved all of `commands`; else `409 untrusted_commands`, with the commands (`data.commands`) for
/// PDT's web app to show before the user approves them.
pub(crate) async fn check(
    st: &AppState,
    project_id: &str,
    repo_path: &str,
    commands: Vec<ConfigCommand>,
) -> Result<(), ApiError> {
    let left = untrusted(st, project_id, repo_path, commands).await?;
    if left.is_empty() {
        return Ok(());
    }
    let message = format!(
        "{} hasn't run {} from this project's run configuration before: {}. Only the user can approve {}, in {}.",
        pdt_core::brand::SHORT_NAME,
        if left.len() == 1 { "this command" } else { "these commands" },
        command_trust::describe(&left),
        if left.len() == 1 { "it" } else { "them" },
        pdt_core::brand::SHORT_NAME,
    );
    Err(ApiError::conflict(codes::UNTRUSTED_COMMANDS, message)
        .with_data(serde_json::json!({ "projectId": project_id, "commands": left })))
}

/// [`check`] for a run's (or a test's) definition, in the project `project_id`.
pub(crate) async fn check_run(
    st: &AppState,
    project_id: &str,
    def: &pdt_core::runconfig::RunDef,
    test: bool,
) -> Result<(), ApiError> {
    let project = st.project(project_id).await?;
    check(st, project_id, &project.repo_path, vec![command_trust::of_run(def, test)]).await
}

/// Tells the user, once per command, that `what` waits on their approval of `commands`.
pub(crate) async fn notify(st: &AppState, project_id: &str, commands: &[ConfigCommand], what: &str) {
    for c in commands {
        let text = format!(
            "{what} waits on a command from the repository's {} that {} hasn't run before: {}. Look at it in the \
             project's settings, under Commands, and approve it there if you trust it.",
            pdt_core::brand::PROJECT_CONFIG_FILE,
            pdt_core::brand::SHORT_NAME,
            command_trust::describe(std::slice::from_ref(c)),
        );
        let item = InboxItem {
            id: uuid::Uuid::now_v7().to_string(),
            kind: InboxKind::Notice,
            group: InboxGroup::Notice,
            status: InboxStatus::Open,
            project_id: Some(project_id.to_string()),
            ticket_id: None,
            ticket_key: None,
            ticket_title: None,
            attempt_id: None,
            session_id: None,
            run_id: None,
            step_id: None,
            persona_id: None,
            persona_name: None,
            change_request: None,
            title: "A command waits on your approval".into(),
            body: serde_json::json!({"notice": null, "key": NOTICE_KEY, "text": text, "fingerprint": c.fingerprint}),
            choices: vec![],
            resolution: serde_json::Value::Null,
            created_at: now_ms(),
            resolved_at: None,
        };
        let dedupe = format!("{NOTICE_KEY}:{project_id}:{}", c.fingerprint);
        match st.store.add_inbox_item_once(&item, &dedupe).await {
            Ok(_) => crate::notifications::kick(st),
            Err(e) => tracing::warn!(project_id, error = %e, "could not put a notice in the Inbox"),
        }
    }
}

/// Closes the project's notices about commands the user has now approved.
async fn settle(st: &AppState, project_id: &str, approved: &HashSet<String>) {
    let Ok(items) = st.store.open_inbox_items_of(InboxKind::Notice).await else { return };
    for i in items.iter().filter(|i| {
        i.project_id.as_deref() == Some(project_id)
            && i.body["key"] == NOTICE_KEY
            && i.body["fingerprint"].as_str().is_some_and(|f| approved.contains(f))
    }) {
        let _ =
            st.store.end_inbox_item(&i.id, InboxStatus::Closed, &serde_json::json!({"approved": true}), now_ms()).await;
    }
    crate::notifications::kick(st);
}

/// `GET /api/projects/{id}/commands`: the commands of a checkout's run configuration and which wait on approval.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectCommands {
    pub commands: Vec<ConfigCommand>,
    /// The fingerprints of those the user has not approved.
    pub untrusted: Vec<String>,
    /// Why there are none when the configuration is invalid.
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Where {
    worktree_id: Option<String>,
}

pub(crate) async fn list(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(w): Query<Where>,
) -> Result<Json<ProjectCommands>, ApiError> {
    let project = st.project(&id).await?;
    crate::projects::needs_repo(&project, "commands")?;
    let (dir, _) = crate::runconfig::checkout_dir(&st, &project, w.worktree_id.as_deref()).await?;
    let config = match crate::runconfig::load_effective(&st, &id, &dir).await {
        Ok(c) => c,
        Err(e) => {
            return Ok(Json(ProjectCommands { commands: vec![], untrusted: vec![], error: Some(e.message().into()) }));
        }
    };
    let commands = command_trust::of_config(&config);
    let trusted = trusted(&st, &id, &project.repo_path).await?;
    let untrusted =
        commands.iter().filter(|c| !trusted.contains(&c.fingerprint)).map(|c| c.fingerprint.clone()).collect();
    Ok(Json(ProjectCommands { commands, untrusted, error: None }))
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrustCommands {
    pub fingerprints: Vec<String>,
}

/// `POST /api/projects/{id}/commands/trust`: the user approves these commands (only from PDT's web app).
pub(crate) async fn trust(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<TrustCommands>,
) -> Result<StatusCode, ApiError> {
    st.project(&id).await?;
    if !crate::untrusted::from_ui(&st, &headers)? {
        return Err(ApiError::forbidden(
            codes::NOT_THE_USER,
            "only the user can approve a command, in PDT; ask them to",
        ));
    }
    st.store.trust_commands(&id, &input.fingerprints, now_ms()).await?;
    settle(&st, &id, &input.fingerprints.into_iter().collect()).await;
    Ok(StatusCode::NO_CONTENT)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new().route("/projects/{id}/commands", get(list)).route("/projects/{id}/commands/trust", post(trust))
}
