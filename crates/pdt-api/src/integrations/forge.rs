//! The forge tools (PDT-177): agents read their change request, its CI and failed logs, and issues, and comment, through
//! the session's project forge ([`crate::forge::project_forge`]) instead of a forge's CLI (`gh`). Each is an action
//! with only a tool face; the change request and CI default to the session's branch (its ticket attempt's, else its
//! worktree's).

use super::Integration;
use super::action::{Action, ActionCx, AnyAction, ToolFace};
use crate::forge::ProjectForge;
use crate::workflow::catalog::BoxFut;
use pdt_core::integration::ToolDecl;
use pdt_core::pr::CiState;
use pdt_forge::{ChangeComment, ChangeRequest, Forge, ForgeError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub struct ForgeTools;

impl Integration for ForgeTools {
    fn id(&self) -> &'static str {
        "forge"
    }

    fn actions(&self) -> Vec<Arc<dyn AnyAction>> {
        vec![
            Arc::new(ChangeRequestView),
            Arc::new(ChangeRequestComment),
            Arc::new(CiRuns),
            Arc::new(CiFailedLogs),
            Arc::new(IssueView),
            Arc::new(IssueComment),
        ]
    }
}

/// The calling session's project forge and branch.
struct Here {
    forge: ProjectForge,
    branch: Option<String>,
}

async fn here(cx: &ActionCx<'_>) -> Result<Here, String> {
    let s = cx.session.ok_or("only an agent session can use this tool")?;
    let st = cx.st;
    let project = st.store.get_project(&s.project_id).await.map_err(|e| e.to_string())?.ok_or("the project is gone")?;
    let forge = crate::forge::project_forge(st, &project.id, &project.repo_path)
        .await
        .ok_or("this project has no forge: its .pdt/project.toml says `forge = \"none\"`, or its origin is on no forge PDT knows")?;
    let attempt = match &s.attempt_id {
        Some(a) => st.store.get_attempt(a).await.ok().flatten().map(|a| a.attempt.branch),
        None => None,
    };
    let branch = match attempt {
        Some(b) => Some(b),
        None => match crate::workflow::agent::step_attempt_branch(st, s).await {
            Some(b) => Some(b),
            None => match &s.worktree_id {
                Some(w) => st.store.get_worktree(w).await.ok().flatten().map(|w| w.branch),
                None => None,
            },
        },
    };
    Ok(Here { forge, branch })
}

impl Here {
    fn branch(&self, given: Option<String>) -> Result<String, String> {
        given
            .filter(|b| !b.trim().is_empty())
            .or_else(|| self.branch.clone())
            .ok_or_else(|| "this session has no branch: name one".into())
    }

    /// Runs a forge call off the async runtime.
    async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Forge, &str) -> Result<T, ForgeError> + Send + 'static,
    ) -> Result<T, String> {
        let (api, repo) = (self.forge.api.clone(), self.forge.repo.clone());
        tokio::task::spawn_blocking(move || f(api.as_ref(), &repo))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }

    /// Change request `number`, else the branch's.
    async fn change_request(&self, number: Option<u32>, branch: Option<String>) -> Result<ChangeRequest, String> {
        match number {
            Some(n) => self.call(move |f, r| f.change_request_number(r, n)).await?.ok_or(format!("there is no #{n}")),
            None => {
                let b = self.branch(branch)?;
                let on = b.clone();
                self.call(move |f, r| f.change_request(r, &on)).await?.ok_or(format!("{b} has no pull request yet"))
            }
        }
    }
}

fn tool(decl: ToolDecl, title: &'static str, description: &'static str) -> ToolFace {
    ToolFace { decl, title, description }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommentOut {
    pub id: u64,
    pub author: String,
    pub bot: bool,
    pub body: String,
    pub url: String,
    /// A review comment's file and line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// A review comment (answer it with `replyTo`), not one on the conversation.
    pub review_comment: bool,
}

impl From<ChangeComment> for CommentOut {
    fn from(c: ChangeComment) -> Self {
        CommentOut {
            id: c.id,
            author: c.author,
            bot: c.is_bot,
            body: c.body,
            url: c.url,
            path: c.path,
            line: c.line,
            review_comment: c.in_review,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOut {
    pub id: u64,
    pub author: String,
    /// `approved`, `changes_requested` or `commented`.
    pub state: String,
    pub body: String,
    pub url: String,
}

fn name_of<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

// ---- change_request_view

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewInput {
    /// The pull request's number; the session's branch's when left out.
    #[serde(default)]
    pub number: Option<u32>,
    /// A branch whose pull request to show, instead of the session's.
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewOutput {
    pub number: u32,
    pub url: String,
    pub title: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    pub draft: bool,
    pub base_branch: String,
    pub head_sha: String,
    /// `mergeable`, `conflicting` or `unknown`.
    pub mergeable: String,
    pub behind: bool,
    pub auto_merge: bool,
    pub in_merge_queue: bool,
    pub comments: Vec<CommentOut>,
    pub reviews: Vec<ReviewOut>,
}

pub struct ChangeRequestView;

impl Action for ChangeRequestView {
    type Input = ViewInput;
    type Output = ViewOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("change_request_view", "Read a pull request with its comments and reviews")
                .read_only()
                .open_world(),
            "View the pull request",
            "The session's pull request (or one by number or branch) on the project's forge: its state, whether it \
             can merge, and every comment and review. Review comments have an id to answer with \
             `change_request_comment`'s `replyTo`.",
        )
    }

    fn approval_title(&self, i: &ViewInput) -> String {
        i.number.map_or_else(|| "read the pull request".into(), |n| format!("read pull request #{n}"))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: ViewInput) -> BoxFut<'a, Result<ViewOutput, String>> {
        Box::pin(async move {
            let h = here(cx).await?;
            let pr = h.change_request(i.number, i.branch).await?;
            let n = pr.number;
            let comments = h.call(move |f, r| f.change_comments(r, n)).await?;
            let reviews = h.call(move |f, r| f.change_reviews(r, n)).await?;
            Ok(ViewOutput {
                number: pr.number,
                url: pr.url,
                title: pr.title,
                state: name_of(&pr.state),
                draft: pr.draft,
                base_branch: pr.base_branch,
                head_sha: pr.head_sha,
                mergeable: name_of(&pr.mergeable),
                behind: pr.behind,
                auto_merge: pr.auto_merge,
                in_merge_queue: pr.in_merge_queue,
                comments: comments.into_iter().map(CommentOut::from).collect(),
                reviews: reviews
                    .into_iter()
                    .map(|r| ReviewOut { id: r.id, author: r.author, state: r.state, body: r.body, url: r.url })
                    .collect(),
            })
        })
    }
}

// ---- change_request_comment

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommentInput {
    /// Markdown.
    pub body: String,
    /// The pull request's number; the session's branch's when left out.
    #[serde(default)]
    pub number: Option<u32>,
    /// A review comment's id (`change_request_view`): the comment answers it in its thread instead of going on the
    /// conversation.
    #[serde(default)]
    pub reply_to: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Posted {
    /// The new comment's URL.
    pub url: String,
}

pub struct ChangeRequestComment;

impl Action for ChangeRequestComment {
    type Input = CommentInput;
    type Output = Posted;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("change_request_comment", "Comment on a pull request or answer a review comment")
                .open_world(),
            "Comment on the pull request",
            "Adds a comment to the session's pull request (or one by number), or with `replyTo` answers a review \
             comment in its thread. It is posted as the user's GitHub login.",
        )
    }

    fn check(&self, i: &CommentInput) -> Result<(), String> {
        if i.body.trim().is_empty() { Err("the comment is empty".into()) } else { Ok(()) }
    }

    fn approval_title(&self, i: &CommentInput) -> String {
        match i.reply_to {
            Some(c) => format!("answer review comment {c}"),
            None => "comment on the pull request".into(),
        }
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: CommentInput) -> BoxFut<'a, Result<Posted, String>> {
        Box::pin(async move {
            self.check(&i)?;
            let h = here(cx).await?;
            let n = match i.number {
                Some(n) => n,
                None => h.change_request(None, None).await?.number,
            };
            let body = i.body;
            let url = match i.reply_to {
                Some(c) => h.call(move |f, r| f.reply_review_comment(r, n, c, &body)).await?,
                None => h.call(move |f, r| f.comment_change_request(r, n, &body)).await?,
            };
            Ok(Posted { url })
        })
    }
}

// ---- ci_runs

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunsInput {
    /// The branch; the session's when left out.
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunOut {
    pub id: u64,
    pub attempt: u32,
    pub workflow: String,
    pub state: CiState,
    pub url: String,
    pub head_sha: String,
    pub created_at: String,
    /// The jobs that failed (failed runs only).
    pub failed_jobs: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunsOutput {
    pub branch: String,
    /// Newest first.
    pub runs: Vec<RunOut>,
}

pub struct CiRuns;

impl Action for CiRuns {
    type Input = RunsInput;
    type Output = RunsOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("ci_runs", "Read a branch's CI runs").read_only().open_world(),
            "CI runs",
            "The recent CI runs on the session's branch (or another): each run's workflow, state, commit and URL, \
             and the failed jobs of failed runs. `ci_failed_logs` reads their logs.",
        )
    }

    fn approval_title(&self, _: &RunsInput) -> String {
        "read CI runs".into()
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: RunsInput) -> BoxFut<'a, Result<RunsOutput, String>> {
        Box::pin(async move {
            let h = here(cx).await?;
            let branch = h.branch(i.branch)?;
            let b = branch.clone();
            let runs = h
                .call(move |f, r| {
                    let mut runs = f.branch_runs(r, &b)?;
                    for run in runs.iter_mut().filter(|x| x.state == CiState::Failed) {
                        run.failed_jobs = f.failed_jobs(r, run.id, run.attempt)?;
                    }
                    Ok(runs)
                })
                .await?;
            let runs = runs
                .into_iter()
                .map(|r| RunOut {
                    id: r.id,
                    attempt: r.attempt,
                    workflow: r.workflow,
                    state: r.state,
                    url: r.url,
                    head_sha: r.head_sha,
                    created_at: r.created_at,
                    failed_jobs: r.failed_jobs,
                })
                .collect();
            Ok(RunsOutput { branch, runs })
        })
    }
}

// ---- ci_failed_logs

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogsInput {
    /// A run's id (`ci_runs`); the newest failed run on the session's branch when left out.
    #[serde(default)]
    pub run_id: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct JobLogOut {
    pub name: String,
    pub url: String,
    /// The end of its log.
    pub log: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogsOutput {
    pub run_id: u64,
    pub jobs: Vec<JobLogOut>,
}

pub struct CiFailedLogs;

impl Action for CiFailedLogs {
    type Input = LogsInput;
    type Output = LogsOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("ci_failed_logs", "Read the logs of a CI run's failed jobs").read_only().open_world(),
            "Failed CI logs",
            "The failed jobs of a CI run (by id, else the newest failed run on the session's branch), each with the \
             end of its log: what to fix.",
        )
    }

    fn approval_title(&self, i: &LogsInput) -> String {
        i.run_id.map_or_else(|| "read failed CI logs".into(), |r| format!("read run {r}'s failed logs"))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: LogsInput) -> BoxFut<'a, Result<LogsOutput, String>> {
        Box::pin(async move {
            let h = here(cx).await?;
            let run_id = match i.run_id {
                Some(r) => r,
                None => {
                    let b = h.branch(None)?;
                    let on = b.clone();
                    let runs = h.call(move |f, r| f.branch_runs(r, &on)).await?;
                    runs.iter()
                        .filter(|r| r.state == CiState::Failed)
                        .max_by(|a, b| a.created_at.cmp(&b.created_at))
                        .map(|r| r.id)
                        .ok_or(format!("no CI run on {b} has failed"))?
                }
            };
            let jobs = h.call(move |f, r| f.failed_logs(r, run_id)).await?;
            let jobs = jobs.into_iter().map(|j| JobLogOut { name: j.name, url: j.url, log: j.log }).collect();
            Ok(LogsOutput { run_id, jobs })
        })
    }
}

// ---- issue_view

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueInput {
    /// The issue's number.
    pub number: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueOutput {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub body: String,
    pub author: String,
    pub labels: Vec<String>,
    pub closed: bool,
    pub created_at: String,
    pub comments: Vec<CommentOut>,
}

pub struct IssueView;

impl Action for IssueView {
    type Input = IssueInput;
    type Output = IssueOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("issue_view", "Read an issue and its comments").read_only().open_world().pm(),
            "View an issue",
            "An issue of the project's repository on its forge: title, description, labels, state and comments.",
        )
    }

    fn approval_title(&self, i: &IssueInput) -> String {
        format!("read issue #{}", i.number)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: IssueInput) -> BoxFut<'a, Result<IssueOutput, String>> {
        Box::pin(async move {
            let h = here(cx).await?;
            let n = i.number;
            let d = h.call(move |f, r| f.issue(r, n)).await?.ok_or(format!("there is no issue #{n}"))?;
            let comments = h.call(move |f, r| f.issue_comments(r, n)).await?;
            Ok(IssueOutput {
                number: d.issue.number,
                url: d.issue.url,
                title: d.issue.title,
                body: d.issue.body,
                author: d.issue.author,
                labels: d.issue.labels,
                closed: d.closed,
                created_at: d.issue.created_at,
                comments: comments.into_iter().map(CommentOut::from).collect(),
            })
        })
    }
}

// ---- issue_comment

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueCommentInput {
    /// The issue's number.
    pub number: u64,
    /// Markdown.
    pub body: String,
}

pub struct IssueComment;

impl Action for IssueComment {
    type Input = IssueCommentInput;
    type Output = Posted;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("issue_comment", "Comment on an issue").open_world(),
            "Comment on an issue",
            "Adds a comment to an issue of the project's repository, posted as the user's forge login.",
        )
    }

    fn check(&self, i: &IssueCommentInput) -> Result<(), String> {
        if i.body.trim().is_empty() { Err("the comment is empty".into()) } else { Ok(()) }
    }

    fn approval_title(&self, i: &IssueCommentInput) -> String {
        format!("comment on issue #{}", i.number)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: IssueCommentInput) -> BoxFut<'a, Result<Posted, String>> {
        Box::pin(async move {
            self.check(&i)?;
            let h = here(cx).await?;
            let (n, body) = (i.number, i.body);
            Ok(Posted { url: h.call(move |f, r| f.comment_issue(r, n, &body)).await? })
        })
    }
}
