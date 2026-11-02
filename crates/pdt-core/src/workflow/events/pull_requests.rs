use super::AttemptRef;
use crate::pr::Notice;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `pr.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrEvent {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    pub pr: u32,
    pub url: String,
    pub head_sha: String,
    pub auto_merge: bool,
    pub in_merge_queue: bool,
    pub notice: Notice,
    /// The pull request was set to merge (Merge…, `attempt.merge`, auto-merge or the merge queue) and hasn't merged
    /// (PDT-104): PR notices gets it merging again.
    #[serde(default)]
    pub merge_requested: bool,
    /// A workflow's merge of the attempt was refused for something its agent can fix (a conflict, uncommitted
    /// changes), and the pull request is still open (PDT-139): PR notices puts the Coder on its later failures.
    #[serde(default)]
    pub merge_refused: bool,
    /// An agent step is working on the attempt (a workflow already has an agent on it).
    #[serde(default)]
    pub agent_working: bool,
}

/// `pr.commented`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrCommented {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    pub pr: u32,
    /// The comment.
    pub url: String,
    #[ts(type = "number")]
    pub comment_id: u64,
    pub author: String,
    pub body: String,
    /// A review comment's file and line.
    pub path: Option<String>,
    pub line: Option<u32>,
    /// On a line, in a review (else on the conversation).
    pub in_review: bool,
    /// Written by the user themself (the account PDT is logged in to GitHub as).
    pub by_owner: bool,
    pub is_bot: bool,
    /// Written by someone who isn't a collaborator on the repository (not its owner, a member or a collaborator, nor
    /// a bot installed on it; PDT-202): shown in PDT, never sent into an agent session.
    #[serde(default)]
    pub outside: bool,
    pub notice: Notice,
}

/// `pr.reviewed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrReviewed {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    pub pr: u32,
    pub url: String,
    #[ts(type = "number")]
    pub review_id: u64,
    pub author: String,
    /// `approved`, `changes_requested` or `commented`.
    pub state: String,
    pub body: String,
    pub by_owner: bool,
    pub is_bot: bool,
    /// As on `pr.commented`.
    #[serde(default)]
    pub outside: bool,
    pub notice: Notice,
}
