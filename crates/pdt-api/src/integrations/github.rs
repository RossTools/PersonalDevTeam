//! GitHub issues: the trigger on new issues in watched repositories and the GitHub actions (PDT-221: filing an issue,
//! also the `issue_create` tool); and for cutting a release (PDT-194), starting a workflow and reading a pull request
//! or a release. (An attempt's pull request and CI are PDT's own: [`super::core`].) It can be turned off while no
//! project uses the GitHub forge (PDT-251): then its triggers, steps, built-in workflows, template and tool are left
//! out, and PDT stops calling GitHub.

use super::Integration;
use super::action::{Action, ActionCx, AnyAction, NodeFace, ToolFace, Via};
use crate::issues::{FiledIssue, NewForgeIssue};
use crate::workflow::catalog::{BoxFut, NodeKind};
use crate::workflow::nodes::github::{GithubClose, GithubComment, GithubImport};
use crate::workflow::nodes::outside_pr::{PrClose, PrMerge, PrReview, PrTicket};
use crate::workflow::nodes::release::{
    GithubChangeRequest, GithubNightly, GithubRelease, GithubWorkflowLatest, GithubWorkflowRun,
};
use pdt_core::integration::{Icon, PluginCategory, ToolDecl};
use pdt_core::pm::AskTarget;
use pdt_core::ticket::Actor;
use pdt_core::workflow::events::{EventType, GITHUB_EVENT_TYPES, GITHUB_PR_CLOSED, GITHUB_PR_OPENED, GITHUB_PR_PUSHED};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

pub struct Github;

impl Integration for Github {
    fn id(&self) -> &'static str {
        "github"
    }

    fn label(&self) -> &'static str {
        "GitHub"
    }

    fn description(&self) -> &'static str {
        "Pull requests, CI, issues and releases for projects on GitHub, as your GitHub login."
    }

    fn category(&self) -> Option<PluginCategory> {
        Some(PluginCategory::Dev)
    }

    fn icon(&self) -> Icon {
        Icon { letters: "GH", color: "#24292f" }
    }

    fn forge_kind(&self) -> Option<pdt_core::forge::ForgeKind> {
        Some(pdt_core::forge::ForgeKind::GITHUB)
    }

    fn can_turn_off(&self) -> bool {
        true
    }

    /// An issue's page on github.com or the Enterprise Server PDT signs in to, or `owner/name#12` (`#12`) in a
    /// project on GitHub.
    fn find_issue<'a>(
        &'a self,
        st: &'a crate::AppState,
        project_id: &'a str,
        given: &'a str,
    ) -> BoxFut<'a, Option<Result<crate::issue_links::FoundIssue, String>>> {
        let kind = pdt_core::forge::ForgeKind::GITHUB;
        Box::pin(async move {
            crate::issue_links::find_forge_issue(st, &kind, project_id, given, pdt_core::github_issue::issue_url).await
        })
    }

    fn read_issue<'a>(
        &'a self,
        st: &'a crate::AppState,
        link: &'a pdt_store::StoredExternalLink,
    ) -> BoxFut<'a, Option<Result<crate::issue_links::IssueNow, String>>> {
        let kind = pdt_core::forge::ForgeKind::GITHUB;
        Box::pin(async move {
            crate::issue_links::read_forge_link(st, &kind, link, pdt_core::github_issue::issue_url).await
        })
    }

    fn issue_example(&self) -> Option<&'static str> {
        Some("owner/name#12, or the issue's address")
    }

    /// "Done closes the GitHub issue".
    fn closes_issues(&self) -> bool {
        true
    }

    fn event_types(&self) -> &'static [EventType] {
        GITHUB_EVENT_TYPES
    }

    fn builtin_workflows(&self) -> Vec<crate::workflow::builtins::Builtin> {
        crate::workflow::builtins::GITHUB_BUILTINS.to_vec()
    }

    fn templates(&self) -> Vec<pdt_core::workflow::template::Template> {
        use crate::workflow::{after_release, outside_prs, templates};
        vec![
            templates::issue_triage(),
            crate::workflow::builtins::cut_release_template(),
            crate::workflow::builtins::missing_release_template(),
            templates::failed_workflow_runs(),
            templates::builtin(
                outside_prs::ID,
                pdt_core::workflow::template::Category::Issues,
                outside_prs::params(),
                outside_prs::doc(),
            ),
            templates::builtin(
                after_release::ID,
                pdt_core::workflow::template::Category::Releases,
                after_release::params(),
                after_release::doc(),
            ),
        ]
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![
            Box::new(crate::github_issues::IssueOpened),
            Box::new(crate::github_pulls::PullRequestTrigger(GITHUB_PR_OPENED)),
            Box::new(crate::github_pulls::PullRequestTrigger(GITHUB_PR_PUSHED)),
            Box::new(crate::github_pulls::PullRequestTrigger(GITHUB_PR_CLOSED)),
            Box::new(GithubImport),
            Box::new(GithubComment),
            Box::new(GithubClose),
            Box::new(PrTicket),
            Box::new(PrMerge),
            Box::new(PrReview),
            Box::new(PrClose),
            Box::new(GithubWorkflowRun),
            Box::new(GithubWorkflowLatest),
            Box::new(GithubChangeRequest),
            Box::new(GithubRelease),
            Box::new(GithubNightly),
            Box::new(crate::workflow::nodes::homebrew::HomebrewCask),
            Box::new(crate::github_workflow_runs::WorkflowFailed),
            Box::new(crate::workflow::nodes::workflow_runs::GithubWorkflowWatch),
            Box::new(crate::workflow::nodes::workflow_runs::GithubWorkflowRerun),
        ]
    }

    fn actions(&self) -> Vec<Arc<dyn AnyAction>> {
        vec![Arc::new(CreateIssue)]
    }
}

// ---- github.create_issue / issue_create

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateIssueInput {
    /// The ticket (key or id) to file as an issue: in its project's repository, with its title and description
    /// unless `title` and `body` are given. The ticket links the new issue.
    #[serde(default)]
    pub ticket: Option<String>,
    /// The project (key or id) whose repository gets the issue, when no ticket is given; an agent's own project when
    /// left out.
    #[serde(default)]
    pub project: Option<String>,
    /// The issue's title (the ticket's when left out).
    #[serde(default)]
    pub title: Option<String>,
    /// Markdown (the ticket's description when left out).
    #[serde(default)]
    pub body: Option<String>,
    /// Labels to put on it; GitHub drops ones the repository doesn't have.
    #[serde(default)]
    pub labels: Vec<String>,
}

fn given(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

pub struct CreateIssue;

impl Action for CreateIssue {
    type Input = CreateIssueInput;
    type Output = FiledIssue;

    fn tool(&self) -> ToolFace {
        ToolFace {
            decl: ToolDecl::new("issue_create", "Open an issue on the project's forge").asks().open_world().pm(),
            title: "Open an issue",
            description: "Opens an issue in a project's GitHub repository, as the user's GitHub login. With `ticket` \
                          the issue is filed from that ticket (its title and description unless given) and the \
                          ticket links it (owner/name#12). The user confirms each call.",
        }
    }

    fn node(&self) -> Option<NodeFace> {
        Some(NodeFace {
            id: "github.create_issue",
            title: "Create a GitHub issue",
            description: "Opens an issue in a project's GitHub repository, as PDT's GitHub login: from a ticket \
                          (`{{trigger.ticketId}}`; its title and description unless given, and the ticket links the \
                          issue), or in `project`'s repository. Output: `repo`, `number`, `issue` (owner/name#12), \
                          `title`, `url` and `ticketKey`.",
            ports: &[("filed", "Filed")],
        })
    }

    fn check(&self, i: &CreateIssueInput) -> Result<(), String> {
        match (given(&i.ticket), given(&i.project), given(&i.title)) {
            (None, _, None) => Err("give a title, or a ticket to file".into()),
            (Some(_), Some(_), _) => Err("give either ticket or project".into()),
            _ => Ok(()),
        }
    }

    fn approval_title(&self, i: &CreateIssueInput) -> String {
        match (given(&i.ticket), given(&i.title)) {
            (Some(t), _) => format!("file {t} as a GitHub issue"),
            (None, Some(title)) => format!("open the GitHub issue \u{201c}{title}\u{201d}"),
            (None, None) => "open a GitHub issue".into(),
        }
    }

    fn ask_target(&self, args: &Value) -> AskTarget {
        let field = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        match (field("ticket"), field("project")) {
            (Some(t), _) => AskTarget::Ticket(t.into()),
            (None, Some(p)) => AskTarget::Project(p.into()),
            (None, None) => AskTarget::Always,
        }
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: CreateIssueInput) -> BoxFut<'a, Result<FiledIssue, String>> {
        Box::pin(async move {
            self.check(&i)?;
            let st = cx.st;
            let own_project = cx.session.map(|s| s.project_id.as_str());
            let (project_id, ticket) = match given(&i.ticket) {
                Some(t) => {
                    let id = crate::mcp::tools::resolve_ticket(st, t, own_project).await?;
                    let t = st.store.get_ticket(&id).await.map_err(|e| e.to_string())?.ok_or("the ticket is gone")?;
                    (t.summary.project_id.clone(), Some(t))
                }
                None => match given(&i.project) {
                    Some(p) => (crate::pm::project_id(st, p).await?, None),
                    None => (own_project.ok_or("name the ticket or the project the issue is for")?.to_string(), None),
                },
            };
            let title = given(&i.title)
                .map(String::from)
                .or_else(|| ticket.as_ref().map(|t| t.summary.title.clone()))
                .unwrap_or_default();
            let body = match (&i.body, &ticket) {
                (Some(b), _) => b.clone(),
                (None, Some(t)) => t.description.clone(),
                (None, None) => String::new(),
            };
            let by = match (cx.via, cx.session) {
                (Via::Tool, Some(s)) => crate::mcp::tools::agent_by(st, s).await,
                _ => Actor::System.into(),
            };
            let new = NewForgeIssue { title, body, labels: i.labels, ticket_id: ticket.map(|t| t.summary.id) };
            crate::issues::file(st, &project_id, new, by).await.map_err(|e| e.message().to_string())
        })
    }
}
