use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The Inbox item an `inbox.*` event is about (PDT-364).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InboxRef {
    pub item_id: String,
    /// `question`, `gate`, `approval`, `stalled`, `ticket_proposal` …
    pub kind: String,
    /// Its group in the Inbox (`question`, `approval`, `gate`, `accept` …).
    pub group: String,
    pub title: String,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub ticket_key: Option<String>,
    /// The agent session it is from, when it is one's (a question, an approval).
    pub session_id: Option<String>,
    /// The persona whose step it is (`Coder` …).
    pub persona: Option<String>,
    /// The workflow run it waits in, and its workflow.
    pub run_id: Option<String>,
    pub workflow_id: Option<String>,
    /// When it opened (ms since the epoch), and how long ago that was when the event was logged.
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub age_ms: i64,
}

/// `inbox.created` (PDT-364).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InboxCreated {
    #[serde(flatten)]
    #[ts(flatten)]
    pub item: InboxRef,
}

/// `inbox.answered` (PDT-364).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InboxAnswered {
    #[serde(flatten)]
    #[ts(flatten)]
    pub item: InboxRef,
    /// The choice taken (`allow`, `approve`, `outcome:approved` …), when it was one.
    pub choice: Option<String>,
    /// What was written: a question's answer, a note.
    pub answer: Option<String>,
    /// `user` (in PDT) or `agent` (through the API).
    pub by: String,
}
