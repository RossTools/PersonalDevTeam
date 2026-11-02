use crate::pr::Notice;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The attempt an event is about.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttemptRef {
    pub project_id: String,
    pub ticket_id: String,
    /// `PDT-42`.
    pub ticket_key: String,
    pub attempt_id: String,
    pub branch: String,
    pub base_branch: String,
}

/// `attempt.late_commits` (PDT-151).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LateCommitsEvent {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    pub pr: u32,
    pub url: String,
    /// The head the pull request merged at.
    pub merged_head: String,
    /// The branch's head now.
    pub head: String,
    /// The commits after `mergedHead`, newest first.
    pub commits: Vec<crate::pr::LateCommit>,
    /// One line for the Inbox: "CB-25: 1 commit pushed after PR #308 merged: cd5d0b7 …".
    pub summary: String,
    /// What PDT tells the attempt's agent.
    pub notice: Notice,
    /// The commits as a list for a ticket's description (`- cd5d0b7 Fix the build`).
    pub list: String,
}

/// How an attempt came to be merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MergeVia {
    /// Merge detection found it (git or GitHub): the usual way, after a PR merges.
    Detected,
    /// The user merged it locally in PDT (Merge…), which finishes the ticket itself.
    MergedHere,
    /// A workflow or an agent merged it locally in PDT (`attempt.merge`, or the API without `by: "user"`, PDT-181):
    /// Finish merged tickets finishes it, as a detected merge.
    Automated,
    /// The user closed the ticket (Done says the work landed, unless its PR is unmerged).
    Closed,
}

/// `attempt.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttemptEvent {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    /// `user` or `system`.
    pub actor: String,
    /// `attempt.merged` only.
    pub via: Option<MergeVia>,
    pub pr: Option<u32>,
    /// Merge… asked to finish the ticket once this merges (PDT-75): the finish gate's answer ahead of time.
    pub finish_on_merge: bool,
    pub finish_delete_branch: bool,
    /// The agent session Start opened (`attempt.started`).
    pub session_id: Option<String>,
    /// The message PDT tells the agent (`attempt.merged`).
    pub notice: Option<Notice>,
    /// The ticket's labels when it happened, so a condition can branch on them (`contains` `automerge`).
    #[serde(default)]
    pub labels: Vec<String>,
    /// The ticket's title when it happened.
    #[serde(default)]
    pub ticket_title: String,
}
