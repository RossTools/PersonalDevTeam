use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `github.issue.opened`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubIssueOpened {
    /// The project whose repository it is, if any.
    pub project_id: Option<String>,
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub author: String,
    /// Its labels, and `from-pdt` on a report filed with Report a bug (GitHub drops the labels of someone without
    /// push access).
    pub labels: Vec<String>,
    pub created_at: String,
}

/// `github.pull_request.opened`, `github.pull_request.pushed` and `github.pull_request.closed` (PDT-268).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubPullRequest {
    /// The project whose repository it is.
    pub project_id: String,
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub author: String,
    /// `outside`, `bot` or `collaborator`.
    pub kind: crate::outside_pr::AuthorKind,
    /// The repository its branch is in (`owner/name`); none when that fork was deleted.
    pub head_repo: Option<String>,
    pub head_branch: String,
    pub head_sha: String,
    /// Its branch is in another repository.
    pub fork: bool,
    /// The branch it merges into.
    pub base: String,
    pub labels: Vec<String>,
    pub draft: bool,
    pub changed_files: u64,
    pub additions: u64,
    pub deletions: u64,
    /// For a Dependabot update, what it bumps (`major`, `minor`, `patch`), when its title says.
    pub update_type: Option<crate::outside_pr::UpdateType>,
    pub created_at: String,
    /// `pushed`: the head before the push.
    pub previous_head: Option<String>,
    /// `closed`: it was merged.
    pub merged: bool,
}

/// `github.workflow.failed` (PDT-292): one failed attempt of a watched workflow's run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubWorkflowFailed {
    /// The watching workflow and its trigger node.
    pub workflow_id: String,
    pub node_id: String,
    /// The project whose repository it is, if any.
    pub project_id: Option<String>,
    /// `owner/name`.
    pub repo: String,
    /// The workflow's name as Actions shows it (`release`) and its file (`.github/workflows/release.yml`).
    pub workflow: String,
    pub path: String,
    /// The run's id, and the attempt that failed (a re-run's is 2, 3…).
    pub run_id: String,
    pub attempt: u32,
    pub title: String,
    pub branch: String,
    /// What started it: `schedule`, `workflow_dispatch`, `push`, `pull_request`…
    pub event: String,
    pub head_sha: String,
    pub url: String,
    /// The version or tag it is for (a tag it ran on, or one in its title), when it has one.
    pub version: Option<String>,
    /// The jobs that failed.
    pub failed_jobs: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}
