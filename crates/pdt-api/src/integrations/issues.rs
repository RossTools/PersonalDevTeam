//! The issue tools: an agent reads an issue of its project's repository (`issue_view`, outside text fenced) and
//! comments on one (`issue_comment`) through the project's forge, like the change request tools ([`super::forge`]).

use super::action::{Action, ActionCx, ToolFace};
use super::forge::{CommentOut, Posted, here, tool};
use crate::workflow::catalog::BoxFut;
use pdt_core::integration::ToolDecl;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
            let fence = pdt_core::fence::Fence::random();
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
