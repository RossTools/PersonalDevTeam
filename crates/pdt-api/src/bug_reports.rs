//! Report a bug (PDT-153): `GET /api/bug-reports/preview` says where a report would go and shows the redacted
//! diagnostics; `POST /api/bug-reports` files it, as an issue on the release repository or, on the instance PDT is
//! developed on, as a Draft `bug` ticket, which never makes a GitHub request. An issue is filed as the user's GitHub
//! login (Connect GitHub), else with the update token (PDT-559: updates need none, so most hosts have neither); with
//! neither, the dialog opens GitHub's bug form filled in ([`BugReportVia::Browser`]). pdt-mcp's `report_bug` runs the
//! same code.

use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use pdt_core::bug_report::{
    self as br, BugReportRoute, BugReportSetting, Diagnostics, ISSUE_LABELS, NONCE_TTL_MS, Redactions, TICKET_LABEL,
};
use pdt_core::forge::ForgeKind;
use pdt_core::release::TokenState;
use pdt_forge::{Forge, ForgeError, NewIssue};
use pdt_github::login::GITHUB_HOST;
use pdt_update::UpdateError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Clone, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewBugReport {
    /// One line saying what went wrong.
    pub title: String,
    /// What happened (Markdown).
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    /// What you expected to happen instead (Markdown).
    #[serde(default)]
    #[ts(optional)]
    pub expected: Option<String>,
    /// How to make it happen again: the steps (Markdown).
    #[serde(default)]
    #[ts(optional)]
    pub steps: Option<String>,
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

/// How an issue on the release repository is filed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BugReportVia {
    /// As the user's GitHub login on github.com.
    Login,
    /// With the update token.
    Token,
    /// By the user, on GitHub's bug form: PDT has neither a login nor a token to file it with.
    Browser,
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
    /// How an issue is filed (on the GitHub route).
    pub via: BugReportVia,
    /// Who the GitHub login signs in as, when `via` is `login`.
    pub github_login: Option<String>,
    /// This build's version and OS, for GitHub's bug form when `via` is `browser`.
    pub version: String,
    pub os: String,
    /// What `includeDiagnostics` adds, as it would be sent (redacted).
    pub diagnostics: String,
}

/// Recent reports by nonce (when, what), so a resend within a minute doesn't file a second one. Held across a filing,
/// which also keeps two reports from racing.
pub(crate) type Nonces = std::sync::Arc<tokio::sync::Mutex<HashMap<String, (i64, BugReportFiled)>>>;

/// The first project whose origin is the release repository: the instance PDT is developed on.
async fn release_project(st: &AppState) -> Result<Option<BugReportProject>, ApiError> {
    let repo = st.update.client.repo();
    for p in st.store.list_projects().await?.into_iter().filter(|p| p.has_repo()) {
        if crate::github::github_repo(&p.repo_path).await.is_some_and(|r| r.eq_ignore_ascii_case(&repo)) {
            return Ok(Some(BugReportProject { id: p.id, key: p.key, name: p.name }));
        }
    }
    Ok(None)
}

/// The GitHub forge as the user's login on github.com, while the GitHub plugin is on and GitHub accepts it, with who
/// it signs in as. A forge with no login of its own (a test's fake) is always signed in.
fn github_login(st: &AppState) -> Option<(Arc<dyn Forge>, Option<String>)> {
    let plugin = st.forge_plugin(&ForgeKind::GITHUB)?;
    let Some(login) = plugin.login() else { return Some((plugin.forge(), None)) };
    let account = login.accounts().into_iter().find(|a| a.host == GITHUB_HOST && a.rejected_at.is_none())?;
    Some((plugin.forge_on(GITHUB_HOST), Some(account.account.login)))
}

/// What an issue would be filed with now.
enum Filer {
    /// The GitHub forge as the user's login, and who it signs in as.
    Login(Arc<dyn Forge>, Option<String>),
    Token(String),
    None,
}

impl Filer {
    /// The login, else the update token.
    async fn find(st: &AppState) -> Filer {
        if let Some((forge, name)) = github_login(st) {
            return Filer::Login(forge, name);
        }
        match crate::update::read_token(st).await.ok().flatten() {
            Some(t) => Filer::Token(t),
            None => Filer::None,
        }
    }

    fn via(&self) -> BugReportVia {
        match self {
            Filer::Login(..) => BugReportVia::Login,
            Filer::Token(_) => BugReportVia::Token,
            Filer::None => BugReportVia::Browser,
        }
    }
}

/// This machine's OS as a report gives it.
fn os() -> String {
    format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
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
    crate::update::refresh_settings(&st).await;
    let setting = st.store.get_host_settings().await?.bug_reports;
    let project = release_project(&st).await?;
    let token_present = crate::update::read_token(&st).await.ok().flatten().is_some();
    let filer = Filer::find(&st).await;
    Ok(Json(BugReportPreview {
        setting,
        route: setting.route(project.is_some()),
        repo: st.update.client.repo(),
        project,
        token_present,
        via: filer.via(),
        github_login: match filer {
            Filer::Login(_, name) => name,
            _ => None,
        },
        version: pdt_core::BUILD_VERSION.into(),
        os: os(),
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
    match e {
        UpdateError::Unauthorized => ApiError::conflict(
            codes::TOKEN_REJECTED,
            format!("GitHub rejected the update token (expired or revoked). Replace it in {SETTINGS} → Updates."),
        ),
        UpdateError::NotFound | UpdateError::Http(403, _) => ApiError::conflict(
            codes::TOKEN_NO_ISSUE_ACCESS,
            format!(
                "The update token can't create issues on {repo}. Connect GitHub in Settings › Plugins › GitHub to \
                 file reports as your GitHub login instead."
            ),
        ),
        UpdateError::Network(m) => {
            ApiError::upstream(codes::GITHUB_UNREACHABLE, format!("Could not reach GitHub ({m}). Try again."))
        }
        other => ApiError::upstream(codes::GITHUB_ERROR, format!("GitHub did not take the report: {other}")),
    }
}

fn login_error(e: ForgeError, repo: &str) -> ApiError {
    match e {
        ForgeError::Unavailable(m) => ApiError::unavailable(codes::FORGE_UNAVAILABLE, m),
        ForgeError::Unsupported(m) => ApiError::bad_request(codes::UNSUPPORTED, m),
        ForgeError::Failed(m) | ForgeError::NotReady(m) => {
            ApiError::upstream(codes::GITHUB_ERROR, format!("GitHub did not take the report on {repo}: {m}"))
        }
    }
}

/// Files a report where the setting says. `by_session`: an agent's `report_bug` (recorded on a local ticket).
pub(crate) async fn file(
    st: &AppState,
    r: NewBugReport,
    by_session: Option<&pdt_core::session::Session>,
) -> Result<BugReportFiled, ApiError> {
    let text = |s: &Option<String>| s.clone().unwrap_or_default();
    let report = br::Report { what_happened: text(&r.description), expected: text(&r.expected), steps: text(&r.steps) };
    let (title, report) = br::validate(&r.title, &report).map_err(ApiError::invalid)?;
    let nonce = r.nonce.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(String::from);
    let mut nonces = st.bug_report_nonces.lock().await;
    let now = now_ms();
    nonces.retain(|_, (at, _)| now - *at < NONCE_TTL_MS);
    if let Some((_, filed)) = nonce.as_ref().and_then(|n| nonces.get(n)) {
        return Ok(filed.clone());
    }
    crate::update::refresh_settings(st).await;
    let setting = st.store.get_host_settings().await?.bug_reports;
    let project = release_project(st).await?;
    let diagnostics = match r.include_diagnostics.unwrap_or(false) {
        true => Some(diagnostics(st).await),
        false => None,
    };
    // The OS is part of the diagnostics: shared only with them.
    let os = diagnostics.as_ref().map(|_| os());
    let body = br::body(&report, diagnostics.as_deref(), pdt_core::BUILD_VERSION, os.as_deref());
    let filed = match setting.route(project.is_some()) {
        BugReportRoute::Off => {
            return Err(ApiError::conflict(
                codes::BUG_REPORTS_OFF,
                format!("Bug reports are turned off. Turn them on in {SETTINGS} → Bug reports."),
            ));
        }
        BugReportRoute::Local => {
            let Some(p) = project else {
                return Err(ApiError::conflict(
                    codes::NO_BUG_PROJECT,
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
            let mut valid = input.validate()?;
            if by_session.is_some() {
                valid.source = pdt_core::ticket::TicketSource::Workflow;
            }
            let id = uuid::Uuid::now_v7().to_string();
            let (by, text) = match by_session {
                Some(s) => (crate::mcp::tools::agent_by(st, s).await, "Filed by an agent with report_bug."),
                None => (pdt_core::ticket::Actor::User.into(), "Filed with Report a bug."),
            };
            let t = st.store.create_ticket_by(&id, &p.id, &valid, &by, now_ms()).await?;
            if let Err(e) = st.store.add_comment_by(&t.summary.id, text, by, None, now_ms()).await {
                tracing::warn!(ticket = %t.summary.id, error = %e, "could not note the bug report on the ticket");
            }
            BugReportFiled::Ticket { key: t.summary.key, id: t.summary.id, project_id: p.id }
        }
        BugReportRoute::Github => {
            let client = &st.update.client;
            let repo = client.repo();
            let issue = match Filer::find(st).await {
                Filer::Login(forge, _) => {
                    let labels = ISSUE_LABELS.iter().map(|l| l.to_string()).collect();
                    let new = NewIssue { title, body, labels };
                    let r = repo.clone();
                    let issue = tokio::task::spawn_blocking(move || forge.create_issue(&r, &new))
                        .await
                        .map_err(ApiError::internal)?
                        .map_err(|e| login_error(e, &repo))?;
                    pdt_update::Issue { number: issue.number, url: issue.url }
                }
                Filer::Token(token) => client
                    .create_issue(&token, &title, &body, &ISSUE_LABELS)
                    .await
                    .map_err(|e| issue_error(e, &repo))?,
                Filer::None => {
                    return Err(ApiError::conflict(
                        codes::NO_GITHUB_LOGIN,
                        format!(
                            "PDT has no GitHub login to file the report with. Connect GitHub in Settings › Plugins › \
                             GitHub, or open the bug form at https://github.com/{repo}/issues/new?template=bug.yml"
                        ),
                    ));
                }
            };
            tracing::info!(number = issue.number, "filed a bug report");
            BugReportFiled::Issue { number: issue.number, url: issue.url }
        }
    };
    if let Some(n) = nonce {
        nonces.insert(n, (now_ms(), filed.clone()));
    }
    Ok(filed)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new().route("/bug-reports", post(create)).route("/bug-reports/preview", get(preview))
}
