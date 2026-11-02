//! The forge tools (PDT-177): agents read their change request, its CI and failed logs, and issues, and comment, through
//! the session's project forge ([`crate::forge::project_forge`]) instead of a forge's CLI (`gh`). Each is an action
//! with only a tool face; the change request and CI default to the session's branch (its ticket attempt's, else its
//! worktree's). PDT-185: an agent opens its branch's change request, and asks for its attempt's merge again once a
//! merge queue dropped it.

use super::Integration;
use super::action::{Action, ActionCx, AnyAction, ToolFace};
use crate::forge::ProjectForge;
use crate::workflow::catalog::BoxFut;
use pdt_core::integration::ToolDecl;
use pdt_core::policy::Merger;
use pdt_core::pr::{CiRun, CiState, PrState};
use pdt_core::review::{MergeOutcome, MergeRequest};
use pdt_forge::{ChangeComment, ChangeRequest, CiProvider, Forge, ForgeError, NewChangeRequest};
use pdt_store::AttemptInfo;
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
            Arc::new(ChangeRequestDiff),
            Arc::new(ChangeRequestComment),
            Arc::new(ChangeRequestOpen),
            Arc::new(ChangeRequestMerge),
            Arc::new(CiRuns),
            Arc::new(CiFailedLogs),
            Arc::new(IssueView),
            Arc::new(IssueComment),
        ]
    }
}

/// The calling session's project forge and branch, and its own ticket attempt (not a workflow step's: a Reviewer's
/// session reads that attempt's CI but opens and merges nothing).
struct Here {
    forge: ProjectForge,
    /// The project's CI providers (PDT-242).
    ci: Vec<Arc<dyn CiProvider>>,
    branch: Option<String>,
    /// The branch is the session's own (its attempt's or its worktree's), not a workflow step's attempt's.
    own_branch: bool,
    attempt: Option<AttemptInfo>,
}

async fn here(cx: &ActionCx<'_>) -> Result<Here, String> {
    let s = cx.session.ok_or("only an agent session can use this tool")?;
    let st = cx.st;
    let project = st.store.get_project(&s.project_id).await.map_err(|e| e.to_string())?.ok_or("the project is gone")?;
    let forge = crate::forge::project_forge(st, &project.id, &project.repo_path)
        .await
        .ok_or("this project has no forge: its .pdt/project.toml says `forge = \"none\"`, or its origin is on no forge PDT knows")?;
    let attempt = match &s.attempt_id {
        Some(a) => st.store.get_attempt(a).await.ok().flatten(),
        None => None,
    };
    let (branch, own_branch) = match attempt.as_ref().map(|a| a.attempt.branch.clone()) {
        Some(b) => (Some(b), true),
        None => match crate::workflow::agent::step_attempt_branch(st, s).await {
            Some(b) => (Some(b), false),
            None => match &s.worktree_id {
                Some(w) => (st.store.get_worktree(w).await.ok().flatten().map(|w| w.branch), true),
                None => (None, true),
            },
        },
    };
    let ci = crate::ci::project_ci(st, &project.id, &project.repo_path, Some(&forge)).await;
    Ok(Here { forge, ci, branch, own_branch, attempt })
}

impl Here {
    fn branch(&self, given: Option<String>) -> Result<String, String> {
        given
            .filter(|b| !b.trim().is_empty())
            .or_else(|| self.branch.clone())
            .ok_or_else(|| "this session has no branch: name one".into())
    }

    /// The forge's name for the repository.
    fn repo(&self) -> String {
        self.forge.repo.clone()
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
    /// Comments and reviews by people who aren't collaborators on the repository, left out (PDT-202: the ticket's
    /// timeline in PDT shows them).
    #[serde(skip_serializing_if = "is_zero")]
    pub left_out: u32,
    /// How to read the fenced blocks, when there are any: bots' comments and reviews came from outside PDT (a bot can
    /// relay what anyone wrote).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outside_text: Option<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
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
             can merge, and every comment and review by the repository's collaborators and bots (others' are left \
             out: `leftOut` counts them; bots' text is fenced as outside input). Review comments have an id to answer with `change_request_comment`'s \
             `replyTo`.",
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
            let left_out = u32::try_from(
                comments.iter().filter(|c| !(c.collaborator || c.is_bot)).count()
                    + reviews.iter().filter(|r| !(r.collaborator || r.is_bot)).count(),
            )
            .unwrap_or(u32::MAX);
            // A bot's text is outside text (PDT-415), whatever its role: it can relay what an outsider wrote.
            let fence = pdt_core::fence::Fence::new(uuid::Uuid::new_v4().simple().to_string());
            let fenced = |bot: bool, body: String| if bot { fence.wrap(&body) } else { body };
            let comments: Vec<CommentOut> = comments
                .into_iter()
                .filter(|c| c.collaborator || c.is_bot)
                .map(|c| {
                    let mut out = CommentOut::from(c);
                    out.body = fenced(out.bot, out.body);
                    out
                })
                .collect();
            let reviews: Vec<ReviewOut> = reviews
                .into_iter()
                .filter(|r| r.collaborator || r.is_bot)
                .map(|r| ReviewOut {
                    id: r.id,
                    author: r.author,
                    state: r.state,
                    body: fenced(r.is_bot, r.body),
                    url: r.url,
                })
                .collect();
            let mut bodies = comments.iter().map(|c| &c.body).chain(reviews.iter().map(|r| &r.body));
            let outside_text = bodies.any(|b| fence.used_in(b)).then(|| fence.note());
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
                left_out,
                comments,
                reviews,
                outside_text,
            })
        })
    }
}

// ---- change_request_diff

/// The most diff text [`ChangeRequestDiff`] gives (characters); the files after it are listed without their diff.
pub const MAX_DIFF: usize = 150_000;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiffInput {
    /// The pull request's number.
    pub number: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiffFileOut {
    pub path: String,
    /// `added`, `modified`, `removed`, `renamed`, …
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_path: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiffOutput {
    pub number: u32,
    pub url: String,
    /// Fenced: whoever opened the pull request wrote it.
    pub title: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    pub draft: bool,
    pub base_branch: String,
    pub head_sha: String,
    /// `mergeable`, `conflicting` or `unknown`.
    pub mergeable: String,
    pub files: Vec<DiffFileOut>,
    /// Every file's diff, one `diff --git` block each, fenced (it is outside input). A binary file, or one too large
    /// for the forge to show, says so instead.
    pub diff: String,
    /// The diff was cut at [`MAX_DIFF`] characters: the later files are in `files` only.
    pub truncated: bool,
    /// How to read the fenced blocks.
    pub outside_text: String,
}

/// The files' diffs as one text, at most `max` characters (whole files only), and whether some were left out.
pub(crate) fn diff_text(files: &[pdt_forge::ChangeFile], max: usize) -> (String, bool) {
    let mut out = String::new();
    for f in files {
        let from = f.previous_path.as_deref().unwrap_or(&f.path);
        let mut block = format!("diff --git a/{from} b/{} ({}, +{} -{})\n", f.path, f.status, f.additions, f.deletions);
        match f.patch.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => {
                block.push_str(p);
                block.push('\n');
            }
            None if f.status == "renamed" && f.additions + f.deletions == 0 => block.push_str("(renamed only)\n"),
            None => block.push_str("(no diff: a binary file, or too large for the forge to show)\n"),
        }
        if out.chars().count() + block.chars().count() > max {
            return (out, true);
        }
        out.push_str(&block);
    }
    (out, false)
}

pub struct ChangeRequestDiff;

impl Action for ChangeRequestDiff {
    type Input = DiffInput;
    type Output = DiffOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("change_request_diff", "Read a pull request's diff through the forge")
                .read_only()
                .open_world(),
            "Read a pull request's diff",
            "A pull request of the project's repository by number, read through the forge's API (nothing is checked \
             out or run): its state, base, head commit, whether it can merge, its files and their diffs. The title \
             and the diff are fenced as outside input.",
        )
    }

    fn approval_title(&self, i: &DiffInput) -> String {
        format!("read pull request #{}'s diff", i.number)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: DiffInput) -> BoxFut<'a, Result<DiffOutput, String>> {
        Box::pin(async move {
            let h = here(cx).await?;
            let pr = h.change_request(Some(i.number), None).await?;
            let n = pr.number;
            let files = h.call(move |f, r| f.change_request_files(r, n)).await?;
            let (diff, truncated) = diff_text(&files, MAX_DIFF);
            // A pull request from outside is outside input (PDT-202): what its author wrote is fenced.
            let fence = pdt_core::fence::Fence::new(uuid::Uuid::new_v4().simple().to_string());
            Ok(DiffOutput {
                outside_text: fence.note(),
                number: pr.number,
                url: pr.url,
                title: fence.wrap(&pr.title),
                state: name_of(&pr.state),
                draft: pr.draft,
                base_branch: pr.base_branch,
                head_sha: pr.head_sha,
                mergeable: name_of(&pr.mergeable),
                files: files
                    .into_iter()
                    .map(|f| DiffFileOut {
                        path: f.path,
                        status: f.status,
                        additions: f.additions,
                        deletions: f.deletions,
                        previous_path: f.previous_path,
                    })
                    .collect(),
                diff: fence.wrap(&diff),
                truncated,
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
                .open_world()
                .public(),
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

// ---- change_request_open

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenInput {
    pub title: String,
    /// Markdown: what changed and why, and how it was checked.
    #[serde(default)]
    pub body: String,
    /// The branch it merges into; the ticket attempt's base when left out.
    #[serde(default)]
    pub base: Option<String>,
    /// Open it as a draft.
    #[serde(default)]
    pub draft: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpenOutput {
    pub number: u32,
    pub url: String,
    pub branch: String,
    pub base: String,
    /// The branch already had an open pull request: that one, nothing new was opened.
    pub existing: bool,
}

pub struct ChangeRequestOpen;

impl Action for ChangeRequestOpen {
    type Input = OpenInput;
    type Output = OpenOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("change_request_open", "Open a pull request for the session's branch").open_world().public(),
            "Open a pull request",
            "Opens a pull request from the session's branch (push it first) into the ticket attempt's base branch, or \
             `base`, with a title and a Markdown body; `draft` opens it as a draft. A branch that already has an open \
             one gets that one back. It is opened as the user's GitHub login.",
        )
    }

    fn check(&self, i: &OpenInput) -> Result<(), String> {
        if i.title.trim().is_empty() { Err("the pull request needs a title".into()) } else { Ok(()) }
    }

    fn approval_title(&self, i: &OpenInput) -> String {
        format!("open a pull request: {}", i.title.trim())
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: OpenInput) -> BoxFut<'a, Result<OpenOutput, String>> {
        Box::pin(async move {
            self.check(&i)?;
            let h = here(cx).await?;
            if !h.own_branch {
                return Err("only the session working on the branch opens its pull request".into());
            }
            let branch = h.branch(None)?;
            let base = i
                .base
                .map(|b| b.trim().to_string())
                .filter(|b| !b.is_empty())
                .or_else(|| h.attempt.as_ref().map(|a| a.attempt.base_branch.clone()))
                .ok_or("this session has no ticket attempt: name the `base` branch")?;
            if base == branch {
                return Err(format!("{branch} can't merge into itself: name another `base`"));
            }
            let on = branch.clone();
            if let Some(pr) = h.call(move |f, r| f.change_request(r, &on)).await?.filter(|p| p.state == PrState::Open) {
                return Ok(OpenOutput { number: pr.number, url: pr.url, branch, base: pr.base_branch, existing: true });
            }
            let new = NewChangeRequest {
                head: branch.clone(),
                base: base.clone(),
                title: i.title.trim().to_string(),
                body: i.body,
                draft: i.draft,
            };
            let opened = h.call(move |f, r| f.open_change_request(r, &new)).await?;
            // PDT tracks it (CI, notices, Merge…) from now, not from the next poll.
            if let Some(a) = &h.attempt {
                crate::github::refresh_attempt(cx.st, a).await;
            }
            Ok(OpenOutput { number: opened.number, url: opened.url, branch, base, existing: false })
        })
    }
}

// ---- change_request_merge

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeInput {}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutput {
    pub number: u32,
    /// `queued`: set to merge (auto-merge or the merge queue); `merged`; `already`: it was set to merge already;
    /// `waitingForCi`: CI on its head hasn't passed yet.
    pub outcome: String,
    pub message: String,
}

pub struct ChangeRequestMerge;

impl Action for ChangeRequestMerge {
    type Input = MergeInput;
    type Output = MergeOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new(
                "change_request_merge",
                "Ask again for the merge of the session's pull request after it dropped out (a merge queue, failed CI)",
            )
            .open_world(),
            "Merge the pull request again",
            "Sets the session's ticket attempt's pull request to merge again (auto-merge, back into the merge \
             queue, or merged once CI passes where the repository has no auto-merge) once you have fixed and pushed what made it drop out. Only for a pull request that was already \
             set to merge: the user or the project's workflow decides the first merge. The project's merge policy \
             applies (it may wait for CI on the push).",
        )
    }

    fn approval_title(&self, _: &MergeInput) -> String {
        "set the pull request to merge again".into()
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, _: MergeInput) -> BoxFut<'a, Result<MergeOutput, String>> {
        Box::pin(async move {
            let h = here(cx).await?;
            let info = h.attempt.as_ref().ok_or("only a ticket attempt's own session can merge its pull request")?;
            crate::github::refresh_attempt(cx.st, info).await;
            let pr = crate::github::watched_pr(cx.st, &info.attempt.id)
                .filter(|p| p.state == PrState::Open)
                .ok_or(format!("{} has no open pull request", info.attempt.branch))?;
            // Read again: the refresh may have recorded auto-merge or the queue.
            let info = cx
                .st
                .store
                .get_attempt(&info.attempt.id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("the attempt is gone")?;
            if !crate::github::merge_requested(&info, &pr) {
                return Err(format!(
                    "PR #{} was never set to merge (or someone took it out by hand): the user merges it (Merge… on the \
                     ticket), or the project's workflow does",
                    pr.number
                ));
            }
            let n = pr.number;
            if pr.auto_merge || pr.in_merge_queue {
                let message = format!("PR #{n} is set to merge already");
                return Ok(MergeOutput { number: n, outcome: "already".into(), message });
            }
            match crate::review::merge_attempt(cx.st, &info.attempt.id, &MergeRequest::default(), Merger::Automated)
                .await
                .map_err(|e| e.message().to_string())?
            {
                Ok(r) => {
                    let (outcome, message) = match r.outcome {
                        MergeOutcome::Merged => ("merged", format!("PR #{n} merged")),
                        MergeOutcome::Queued => ("queued", format!("PR #{n} is set to merge again")),
                    };
                    Ok(MergeOutput { number: n, outcome: outcome.into(), message })
                }
                Err(r) if r.code == "ci_pending" => {
                    // Still set to merge, so CI passing merges it (PR notices, PDT-104; PDT itself, PDT-297).
                    if let Err(e) = cx.st.store.set_attempt_merge_requested(&info.attempt.id).await {
                        tracing::warn!(attempt = %info.attempt.id, error = %e, "could not record a merge waiting for CI");
                    }
                    let message = format!(
                        "{}. Ask again once CI on the push passes (`ci_runs`); PDT also sets it to merge when it \
                         passes and no workflow step is on the ticket",
                        r.reason
                    );
                    Ok(MergeOutput { number: n, outcome: "waitingForCi".into(), message })
                }
                Err(r) => Err(r.reason),
            }
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
    /// Only this CI provider's runs (`github_actions`, `buildkite`, `local`); every provider of the project's when
    /// left out.
    #[serde(default)]
    pub provider: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunOut {
    /// Its CI provider.
    pub provider: String,
    pub id: String,
    pub attempt: u32,
    /// Its workflow (or pipeline).
    pub name: String,
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
    /// Newest first, each provider's in turn.
    pub runs: Vec<RunOut>,
    /// Providers whose runs couldn't be read, and why.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

pub struct CiRuns;

/// The project's CI providers, or only `provider`.
fn providers_of(h: &Here, provider: Option<&str>) -> Result<Vec<Arc<dyn CiProvider>>, String> {
    let all = h.ci.clone();
    let Some(id) = provider.map(str::trim).filter(|p| !p.is_empty()) else {
        return if all.is_empty() { Err("this project has no CI provider".into()) } else { Ok(all) };
    };
    let names = all.iter().map(|p| p.id().to_string()).collect::<Vec<_>>().join(", ");
    match all.into_iter().find(|p| p.id() == id) {
        Some(p) => Ok(vec![p]),
        None => Err(format!("this project has no CI provider {id} (it has: {names})")),
    }
}

/// `p`'s runs on `branch`, newest first, failed jobs filled in.
fn runs_of(p: &dyn CiProvider, repo: &str, branch: &str) -> Result<Vec<CiRun>, ForgeError> {
    let mut runs = p.runs(repo, branch, None)?;
    for run in runs.iter_mut().filter(|x| x.state == CiState::Failed && x.failed_jobs.is_empty()) {
        run.failed_jobs = pdt_forge::ci::failed_job_names(p, repo, run)?;
    }
    runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(runs)
}

impl Action for CiRuns {
    type Input = RunsInput;
    type Output = RunsOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("ci_runs", "Read a branch's CI runs").read_only().open_world(),
            "CI runs",
            "The recent CI runs on the session's branch (or another), from each of the project's CI providers (or \
             one): each run's provider, workflow, state, commit and URL, and the failed jobs of failed runs. \
             `ci_failed_logs` reads their logs.",
        )
    }

    fn approval_title(&self, _: &RunsInput) -> String {
        "read CI runs".into()
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: RunsInput) -> BoxFut<'a, Result<RunsOutput, String>> {
        Box::pin(async move {
            let h = here(cx).await?;
            let branch = h.branch(i.branch)?;
            let providers = providers_of(&h, i.provider.as_deref())?;
            let (repo, b) = (h.repo(), branch.clone());
            let read = tokio::task::spawn_blocking(move || {
                providers.iter().map(|p| (p.label().to_string(), runs_of(p.as_ref(), &repo, &b))).collect::<Vec<_>>()
            })
            .await
            .map_err(|e| e.to_string())?;
            let (mut runs, mut errors) = (vec![], vec![]);
            for (label, got) in read {
                match got {
                    Ok(r) => runs.extend(r),
                    Err(e) => errors.push(format!("{label}: {e}")),
                }
            }
            if runs.is_empty() && !errors.is_empty() {
                return Err(errors.join("; "));
            }
            let runs = runs
                .into_iter()
                .map(|r| RunOut {
                    provider: r.provider,
                    id: r.id,
                    attempt: r.attempt,
                    name: r.name,
                    state: r.state,
                    url: r.url,
                    head_sha: r.head_sha,
                    created_at: r.created_at,
                    failed_jobs: r.failed_jobs,
                })
                .collect();
            Ok(RunsOutput { branch, runs, errors })
        })
    }
}

// ---- ci_failed_logs

/// A run id: text, or a number as GitHub Actions' were.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(inline)]
pub enum RunId {
    Number(u64),
    Text(String),
}

impl RunId {
    fn text(&self) -> String {
        match self {
            RunId::Number(n) => n.to_string(),
            RunId::Text(s) => s.trim().to_string(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogsInput {
    /// A run's id (`ci_runs`); the newest failed run on the session's branch when left out.
    #[serde(default)]
    pub run_id: Option<RunId>,
    /// The run's CI provider (`ci_runs` names it); GitHub Actions, or the only provider, when left out.
    #[serde(default)]
    pub provider: Option<String>,
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
    pub provider: String,
    pub run_id: String,
    pub jobs: Vec<JobLogOut>,
}

pub struct CiFailedLogs;

/// The failed run to read: `id` of `provider` (looked up on the branch, for its attempt), else the newest failed run
/// on the branch of any of `providers`.
fn failed_run(
    providers: &[Arc<dyn CiProvider>],
    repo: &str,
    branch: Option<&str>,
    id: Option<&str>,
) -> Result<(Arc<dyn CiProvider>, CiRun), String> {
    let read = |p: &Arc<dyn CiProvider>| match branch {
        Some(b) => p.runs(repo, b, None).map_err(|e| format!("{}: {e}", p.label())),
        None => Ok(vec![]),
    };
    if let Some(id) = id {
        let p = match providers {
            [one] => one.clone(),
            _ => providers
                .iter()
                .find(|p| p.id() == pdt_core::pr::GITHUB_ACTIONS)
                .cloned()
                .ok_or("name the run's provider (`ci_runs` gives it)")?,
        };
        let known = read(&p).unwrap_or_default().into_iter().find(|r| r.id == id);
        // A run no longer among the branch's recent ones: its first attempt.
        let run = known.unwrap_or_else(|| CiRun {
            provider: p.id().to_string(),
            id: id.to_string(),
            attempt: 1,
            name: String::new(),
            state: CiState::Failed,
            url: String::new(),
            head_sha: String::new(),
            branch: branch.unwrap_or_default().to_string(),
            created_at: String::new(),
            failed_jobs: vec![],
        });
        return Ok((p, run));
    }
    let b = branch.ok_or("this session has no branch: name a run")?;
    let mut newest: Option<(Arc<dyn CiProvider>, CiRun)> = None;
    for p in providers {
        for r in read(p)?.into_iter().filter(|r| r.state == CiState::Failed) {
            if newest.as_ref().is_none_or(|(_, n)| r.created_at > n.created_at) {
                newest = Some((p.clone(), r));
            }
        }
    }
    newest.ok_or(format!("no CI run on {b} has failed"))
}

impl Action for CiFailedLogs {
    type Input = LogsInput;
    type Output = LogsOutput;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("ci_failed_logs", "Read the logs of a CI run's failed jobs").read_only().open_world(),
            "Failed CI logs",
            "The failed jobs of a CI run (by provider and id, else the newest failed run on the session's branch), \
             each with the end of its log: what to fix.",
        )
    }

    fn approval_title(&self, i: &LogsInput) -> String {
        i.run_id
            .as_ref()
            .map_or_else(|| "read failed CI logs".into(), |r| format!("read run {}'s failed logs", r.text()))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: LogsInput) -> BoxFut<'a, Result<LogsOutput, String>> {
        Box::pin(async move {
            let h = here(cx).await?;
            let providers = providers_of(&h, i.provider.as_deref())?;
            let (repo, branch, id) = (h.repo(), h.branch(None).ok(), i.run_id.map(|r| r.text()));
            tokio::task::spawn_blocking(move || {
                let (p, run) = failed_run(&providers, &repo, branch.as_deref(), id.as_deref())?;
                if !p.capabilities().logs {
                    return Err(format!("{} has no logs to read: {}", p.label(), run.url));
                }
                let failed = p.jobs(&repo, &run).map_err(|e| e.to_string())?;
                let mut jobs = vec![];
                for j in failed.into_iter().filter(|j| j.state == CiState::Failed) {
                    let log = p.job_log_tail(&repo, &run, &j, pdt_forge::ci::LOG_LINES).map_err(|e| e.to_string())?;
                    jobs.push(JobLogOut { name: j.name, url: j.url, log });
                }
                Ok(LogsOutput { provider: p.id().to_string(), run_id: run.id, jobs })
            })
            .await
            .map_err(|e| e.to_string())?
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
    /// How to read the fenced blocks: the issue's title and text and outsiders' and bots' comments came from outside
    /// PDT.
    pub outside_text: String,
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
            // An issue is outside input (PDT-202): its title and text, and outsiders' and bots' comments, are fenced.
            let fence = pdt_core::fence::Fence::new(uuid::Uuid::new_v4().simple().to_string());
            let comments: Vec<CommentOut> = comments
                .into_iter()
                .map(|c| {
                    // A bot's text is outside text too (PDT-415): it can relay what an outsider wrote.
                    let inside = c.collaborator && !c.is_bot;
                    let mut out = CommentOut::from(c);
                    if !inside {
                        out.body = fence.wrap(&out.body);
                    }
                    out
                })
                .collect();
            Ok(IssueOutput {
                outside_text: fence.note(),
                number: d.issue.number,
                url: d.issue.url,
                title: fence.wrap(&d.issue.title),
                body: fence.wrap(&d.issue.body),
                author: d.issue.author,
                labels: d.issue.labels,
                closed: d.closed,
                created_at: d.issue.created_at,
                comments,
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
            ToolDecl::new("issue_comment", "Comment on an issue").open_world().public(),
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
