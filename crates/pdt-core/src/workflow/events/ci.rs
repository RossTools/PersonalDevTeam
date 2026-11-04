use super::AttemptRef;
use crate::pr::{CiState, Notice};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CiRunRef {
    /// Its CI provider (`github_actions`, `buildkite`, `local`).
    #[serde(default)]
    pub provider: String,
    /// The provider's id for it (PDT-242: text; events logged before were numbers).
    #[serde(deserialize_with = "id_text")]
    pub id: String,
    pub attempt: u32,
    /// Its workflow or pipeline.
    #[serde(alias = "workflow")]
    pub name: String,
    pub state: CiState,
    pub url: String,
    pub failed_jobs: Vec<String>,
}

/// A run id as text, from text or a number.
fn id_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        other => Err(serde::de::Error::custom(format!("a run id is text, not {other}"))),
    }
}

/// `ci.run_failed`, `ci.run_passed`, `ci.run_cancelled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CiEvent {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    /// The CI provider whose runs these are: `github_actions`, `local`, or a CI integration's id (`buildkite`).
    pub provider: String,
    pub pr: Option<u32>,
    pub head_sha: String,
    /// The run to look at.
    pub url: String,
    pub runs: Vec<CiRunRef>,
    /// The failed jobs, all runs together.
    pub jobs: Vec<String>,
    /// The message PDT tells the agent (failures only).
    pub notice: Option<Notice>,
    /// The pull request's auto-merge is on (false without one).
    #[serde(default)]
    pub auto_merge: bool,
    /// The pull request is in the merge queue.
    #[serde(default)]
    pub in_merge_queue: bool,
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
    /// A workflow's merge of the attempt waits for this CI (PDT-151): it handles a failure itself.
    #[serde(default)]
    pub merge_waiting: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ci_run_logged_before_providers_still_reads() {
        let old = serde_json::json!({"id": 391, "attempt": 2, "workflow": "ci", "state": "failed", "url": "u",
            "failedJobs": []});
        let r: CiRunRef = serde_json::from_value(old).unwrap();
        assert_eq!((r.provider.as_str(), r.id.as_str(), r.name.as_str()), ("", "391", "ci"));
        let new = serde_json::to_value(&CiRunRef { provider: "buildkite".into(), id: "app/3".into(), ..r }).unwrap();
        assert_eq!((new["id"].as_str(), new["name"].as_str()), (Some("app/3"), Some("ci")));
        assert_eq!(serde_json::from_value::<CiRunRef>(new).unwrap().provider, "buildkite");
    }
}
