use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The document an event is about (`document.*`), and the project it is for: an event is logged once for each project
/// the document is linked to (`projectIds`, in order), or once without a project when it is linked to none.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentRef {
    /// One of `projectIds`; empty for a document linked to no project.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project_id: String,
    pub document_id: String,
    pub title: String,
    /// `doc` or `sheet`.
    pub kind: String,
    /// The provider that keeps it (`local`, `google`).
    pub provider: String,
    #[serde(default)]
    pub url: Option<String>,
    /// Every project it is linked to.
    #[serde(default)]
    pub project_ids: Vec<String>,
    /// The keys of the tickets it is linked to.
    #[serde(default)]
    pub tickets: Vec<String>,
}

/// `document.changed` (PDT-318).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentChanged {
    #[serde(flatten)]
    #[ts(flatten)]
    pub document: DocumentRef,
    /// The provider's version now.
    pub version: String,
    /// The version PDT saw before.
    pub previous_version: String,
}

/// `document.comment_added` (PDT-318).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentCommentAdded {
    #[serde(flatten)]
    #[ts(flatten)]
    pub document: DocumentRef,
    /// The new comment's (or reply's) id.
    pub comment_id: String,
    /// The comment whose thread it is in: the one to reply to (the comment itself, or the one it replies to).
    pub thread: String,
    /// It is a reply in a thread, not a new comment.
    pub reply: bool,
    /// The name it shows.
    pub author: String,
    /// The user wrote it (in PDT, or as the signed-in account): its text is not outside input.
    pub by_user: bool,
    pub body: String,
    /// The text the thread is on, when the provider says.
    #[serde(default)]
    pub quote: Option<String>,
}
