use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `webhook.received`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WebhookReceived {
    /// The webhook connection's name.
    pub hook: String,
    pub method: String,
    #[ts(type = "Record<string, string>")]
    pub query: std::collections::BTreeMap<String, String>,
    /// `content-type`, `user-agent` and the `x-*` headers, names in lower case.
    #[ts(type = "Record<string, string>")]
    pub headers: std::collections::BTreeMap<String, String>,
    /// Parsed JSON, else the text.
    #[ts(type = "unknown")]
    pub body: serde_json::Value,
}

/// `session.outside_input` (PDT-202): outside text went into an agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OutsideInput {
    pub project_id: Option<String>,
    /// The ticket the session works on, or the one made from the input; none for a triage that made none.
    pub ticket_id: Option<String>,
    pub session_id: String,
    /// Where the text came from: `owner/name#12`, a comment's URL, `webhook <name>`, `jira ABC-1`.
    pub source: String,
    /// A link to it, when there is one.
    pub url: Option<String>,
    /// How it went in: `prompt` (an agent step's prompt), `message` (`session.send`), `ticket` (the session works
    /// on an external ticket, whose description is outside text), `triage` (the triage session that made the
    /// ticket).
    pub via: String,
    pub run_id: Option<String>,
    pub workflow_name: Option<String>,
}

/// Why triage skipped an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TriageCap {
    /// Its author had reached the cap per author.
    Author,
    /// Its repository (or webhook) had reached the cap per repository.
    Repo,
}

/// `triage.skipped` (PDT-202).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TriageSkipped {
    pub project_id: Option<String>,
    /// `owner/name`, or `webhook <name>`.
    pub repo: String,
    /// The issue's, when it is one.
    pub number: Option<u64>,
    pub url: Option<String>,
    pub author: Option<String>,
    pub workflow_id: String,
    pub workflow_name: String,
    /// The event that would have started the run.
    pub event_id: String,
    pub cap: TriageCap,
    /// The cap reached (runs an hour).
    pub limit: u32,
}
