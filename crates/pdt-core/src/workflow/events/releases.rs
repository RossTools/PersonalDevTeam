use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A release asset (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleaseAssetRef {
    pub name: String,
    /// Where it downloads from.
    pub url: String,
    /// Bytes, when the forge says.
    #[ts(type = "number | null")]
    pub size: Option<u64>,
}

/// `release.published` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleasePublished {
    pub project_id: String,
    /// The forge (`github`, `gitlab` …) and the repository on it (`owner/name`).
    pub forge: String,
    pub repo: String,
    pub tag: String,
    pub name: String,
    /// Its page.
    pub url: String,
    pub prerelease: bool,
    /// When it was published (the forge's timestamp text), when known.
    pub published_at: Option<String>,
    /// Its notes (Markdown).
    pub notes: String,
    /// The start of its notes, whole lines up to about 500 characters (an announcement's).
    #[serde(default)]
    pub summary: String,
    pub assets: Vec<ReleaseAssetRef>,
}

/// The length [`ReleasePublished::summary`] is cut to.
pub const RELEASE_SUMMARY_CHARS: usize = 500;
