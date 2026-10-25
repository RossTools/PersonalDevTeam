//! Report a bug (PDT-153): `GET /api/bug-reports/preview` says where a report would go and shows the redacted
//! diagnostics; `POST /api/bug-reports` files it, as an issue on the release repository (with the update token) or,
//! on the instance PDT is developed on, as a Draft `bug` ticket, which never makes a GitHub request. pdt-mcp's
//! `report_bug` runs the same code.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use pdt_core::bug_report::{
    self as br, BugReportRoute, BugReportSetting, Diagnostics, ISSUE_LABELS, NONCE_TTL_MS, Redactions, TICKET_LABEL,
};
use pdt_core::release::TokenState;
use pdt_update::UpdateError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;

#[derive(Debug, Clone, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewBugReport {
    /// One line saying what went wrong.
    pub title: String,
    /// What happened, what you expected, and how to make it happen again (Markdown).
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    /// Add this instance's diagnostics (version, OS, update state, the end of the pdtd log), redacted.
    #[serde(default)]
    #[ts(optional)]
    pub include_diagnostics: Option<bool>,
    /// The form's id for this report: sending the same nonce again within a minute returns the first report.
    #[serde(default)]
    #[ts(optional)]
    #[schemars(skip)]
    pub nonce: Option<String>,
}

/// Where a report went.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum BugReportFiled {
    Issue {
        #[ts(type = "number")]
        number: u64,
        url: String,
    },
    Ticket {
        key: String,
        id: String,
        project_id: String,
    },
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BugReportProject {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BugReportPreview {
    pub setting: BugReportSetting,
    pub route: BugReportRoute,
    /// The repository issues go to (`owner/name`).
    pub repo: String,
    /// The project local reports go to: the one whose origin is `repo`.
    pub project: Option<BugReportProject>,
    pub token_present: bool,
    /// What `includeDiagnostics` adds, as it would be sent (redacted).
    pub diagnostics: String,
}

/// Recent reports by nonce (when, what), so a resend within a minute doesn't file a second one. Held across a filing,
/// which also keeps two reports from racing.
pub(crate) type Nonces = std::sync::Arc<tokio::sync::Mutex<HashMap<String, (i64, BugReportFiled)>>>;

/// The first project whose origin is the release repository: the instance PDT is developed on.
async fn release_project(st: &AppState) -> Result<Option<BugReportProject>, ApiError> {
    let repo = st.update.client.repo().to_string();
    let projects = crate::projects::with_github(st.store.list_projects().await?).await;
    Ok(projects
        .into_iter()
        .find(|p| p.github_repo.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(&repo)))
        .map(|p| BugReportProject { id: p.id, key: p.key, name: p.name }))
}

fn token_word(t: &TokenState) -> &'static str {
    match t {
        TokenState::Missing => "missing",
        TokenState::Valid { .. } => "valid",
        TokenState::ExpiresSoon { .. } => "expires soon",
        TokenState::Expired => "expired or revoked",
    }
}

/// This machine's names: its hostname (and the part before the first dot) and its tailnet name.
async fn hostnames(st: &AppState) -> Vec<String> {
    let mut names: Vec<String> = ["HOSTNAME", "COMPUTERNAME"].iter().filter_map(|v| std::env::var(v).ok()).collect();
    let cmd = tokio::task::spawn_blocking(|| std::process::Command::new("hostname").output()).await;
    if let Ok(Ok(out)) = cmd
        && out.status.success()
    {
        names.push(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let remote = st.remote.lock().unwrap_or_else(|e| e.into_inner()).clone();
    names.extend(remote.dns_name.map(|d| d.trim_end_matches('.').to_string()));
    let short: Vec<String> = names.iter().filter_map(|n| n.split_once('.').map(|(s, _)| s.to_string())).collect();
    names.extend(short);
    // Longest first, so a full name goes before its short form.
    names.retain(|n| n.len() >= 2);
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    names.dedup();
    names
}

/// The newest pdtd log's last lines (`<data dir>/logs/pdtd.<date>.log`).
async fn log_tail(st: &AppState) -> Vec<String> {
    let dir = st.data_dir.join("logs");
    tokio::task::spawn_blocking(move || {
        let newest = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(&format!("{}.", pdt_core::brand::DAEMON_BIN)))
            .max()?;
        let bytes = std::fs::read(dir.join(newest)).ok()?;
        // The end is enough: a day's log can be large.
        let from = bytes.len().saturating_sub(256 * 1024);
        Some(br::tail(&String::from_utf8_lossy(&bytes[from..]), br::LOG_LINES))
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// The diagnostics as they are shown and sent: everything that could be private redacted.
async fn diagnostics(st: &AppState) -> String {
    let token = crate::update::read_token(st).await.ok().flatten();
    let mut secrets: Vec<String> = token.into_iter().collect();
    let owner = st.store.get_host_settings().await.ok().and_then(|s| s.owner_login);
    secrets.extend(owner.into_iter().chain(st.owner_from_env.clone()));
    let r = Redactions {
        secrets,
        home: st.home_dir.as_ref().map(|h| h.to_string_lossy().into_owned()),
        hostnames: hostnames(st).await,
    };
    let status = st.update.snapshot();
    let mut update =
        serde_json::to_value(status.state).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
    if let Some(e) = status.error.as_deref().or(status.disabled_reason.as_deref()) {
        update.push_str(&format!(" ({})", br::redact(e, &r)));
    }
    let d = Diagnostics {
        version: pdt_core::BUILD_VERSION.into(),
        profile: st.instance.profile.data_dir_name(),
        dev: st.instance.dev,
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        supervised: st.update.enabled,
        update,
        token: token_word(&status.token).into(),
        log_tail: log_tail(st).await.iter().map(|l| br::redact(l, &r)).collect(),
    };
    d.render()
}

pub(crate) async fn preview(State(st): State<AppState>) -> Result<Json<BugReportPreview>, ApiError> {
    let setting = st.store.get_host_settings().await?.bug_reports;
    let project = release_project(&st).await?;
    let token_present = crate::update::read_token(&st).await.ok().flatten().is_some();
    Ok(Json(BugReportPreview {
        setting,
        route: setting.route(project.is_some()),
        repo: st.update.client.repo().to_string(),
        project,
        token_present,
        diagnostics: diagnostics(&st).await,
    }))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewBugReport>, JsonRejection>,
) -> Result<Json<BugReportFiled>, ApiError> {
    let Json(r) = payload?;
    Ok(Json(file(&st, r, None).await?))
}

const SETTINGS: &str = "Settings → This host";

fn issue_error(e: UpdateError, repo: &str) -> ApiError {
    let conflict = |code, msg: String| ApiError::new(StatusCode::CONFLICT, code, msg);
    match e {
        UpdateError::Unauthorized => conflict(
            "token_rejected",
            format!("GitHub rejected the update token (expired or revoked). Replace it in {SETTINGS} → Updates."),
        ),
        UpdateError::NotFound | UpdateError::Http(403, _) => conflict(
            "token_no_issue_access",
            format!(
                "The update token can't create issues on {repo}. Give it Issues: write access, or set a new one in \
                 {SETTINGS} → Updates."
            ),
        ),
        UpdateError::Network(m) => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "github_unreachable",
            format!("Could not reach GitHub ({m}). Try again."),
        ),
        other => {
            ApiError::new(StatusCode::BAD_GATEWAY, "github_error", format!("GitHub did not take the report: {other}"))
        }
    }
}

/// Files a report where the setting says. `by_session`: an agent's `report_bug` (recorded on a local ticket).
pub(crate) async fn file(
    st: &AppState,
    r: NewBugReport,
    by_session: Option<&pdt_core::session::Session>,
) -> Result<BugReportFiled, ApiError> {
    let (title, description) = br::validate(&r.title, r.description.as_deref().unwrap_or(""))
        .map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", m))?;
    let nonce = r.nonce.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(String::from);
    let mut nonces = st.bug_report_nonces.lock().await;
    let now = now_ms();
    nonces.retain(|_, (at, _)| now - *at < NONCE_TTL_MS);
    if let Some((_, filed)) = nonce.as_ref().and_then(|n| nonces.get(n)) {
        return Ok(filed.clone());
    }
    let setting = st.store.get_host_settings().await?.bug_reports;
    let project = release_project(st).await?;
    let diagnostics = match r.include_diagnostics.unwrap_or(false) {
        true => Some(diagnostics(st).await),
        false => None,
    };
    let body = br::body(&description, diagnostics.as_deref(), pdt_core::BUILD_VERSION);
    let filed = match setting.route(project.is_some()) {
        BugReportRoute::Off => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "bug_reports_off",
                format!("Bug reports are turned off. Turn them on in {SETTINGS} → Bug reports."),
            ));
        }
        BugReportRoute::Local => {
            let Some(p) = project else {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "no_bug_project",
                    format!(
                        "Bug reports are set to local tickets, but no project here has {} as its origin. Choose \
                         GitHub in {SETTINGS} → Bug reports.",
                        st.update.client.repo()
                    ),
                ));
            };
            let input = pdt_core::ticket::NewTicket {
                title,
                description: Some(body),
                stage: Some(pdt_core::ticket::TicketStage::Draft),
                priority: None,
                labels: Some(vec![TICKET_LABEL.to_string()]),
                areas: None,
                before: None,
                after: None,
                parent: None,
                blocked_by_previous: None,
            };
            let valid = input.validate()?;
            let id = uuid::Uuid::now_v7().to_string();
            let t = st.store.create_ticket(&id, &p.id, &valid, now_ms()).await?;
            let (by, text) = match by_session {
                Some(_) => (pdt_core::ticket::Actor::Agent, "Filed by an agent with report_bug."),
                None => (pdt_core::ticket::Actor::User, "Filed with Report a bug."),
            };
            let data = by_session.map(|s| serde_json::json!({"sessionId": s.id}));
            if let Err(e) = st.store.add_comment_by(&t.summary.id, text, by, data, now_ms()).await {
                tracing::warn!(ticket = %t.summary.id, error = %e, "could not note the bug report on the ticket");
            }
            BugReportFiled::Ticket { key: t.summary.key, id: t.summary.id, project_id: p.id }
        }
        BugReportRoute::Github => {
            let Some(token) = crate::update::read_token(st).await? else {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "no_token",
                    format!("Reporting a bug needs the update token. Add it in {SETTINGS} → Updates."),
                ));
            };
            let client = &st.update.client;
            let issue = client
                .create_issue(&token, &title, &body, &ISSUE_LABELS)
                .await
                .map_err(|e| issue_error(e, client.repo()))?;
            tracing::info!(number = issue.number, "filed a bug report");
            BugReportFiled::Issue { number: issue.number, url: issue.url }
        }
    };
    if let Some(n) = nonce {
        nonces.insert(n, (now_ms(), filed.clone()));
    }
    Ok(filed)
}
