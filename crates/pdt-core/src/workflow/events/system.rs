use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `update.available` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateAvailable {
    /// The new version, and the one running.
    pub version: String,
    pub current: String,
    pub tag: String,
    /// The host's update channel (`stable` or `nightly`).
    pub channel: String,
    /// A nightly build.
    pub nightly: bool,
    /// The release's page.
    pub notes_url: String,
    pub notes: String,
    pub published_at: Option<String>,
}

/// `update.installed` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInstalled {
    pub version: String,
    pub from_version: String,
    pub channel: String,
    /// The release's page.
    pub notes_url: String,
    /// Installed without the user asking (the automatic update when idle).
    pub automatic: bool,
    /// When it was installed (ms since the epoch).
    #[ts(type = "number")]
    pub at: i64,
}

/// `update.failed` (PDT-369): pdt-host could not install `version`, or rolled it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateFailed {
    /// The version that failed, and the one running (again).
    pub version: String,
    pub from_version: String,
    /// It installed but did not start, and the version before came back.
    pub rolled_back: bool,
    pub message: String,
    /// The update log's path, when there is one.
    pub log: Option<String>,
    /// When it failed (ms since the epoch).
    #[ts(type = "number")]
    pub at: i64,
}

/// `disk.low` and `disk.recovered` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiskEvent {
    /// PDT's data folder and the projects' folders on this disk.
    pub folders: Vec<String>,
    #[ts(type = "number")]
    pub free: u64,
    #[ts(type = "number")]
    pub total: u64,
    /// `ok`, `low` or `critical`, now; and before.
    pub level: String,
    pub from: String,
    /// The thresholds, in GB (0: that warning is off).
    pub low_gb: u32,
    pub critical_gb: u32,
    /// Bytes of build output in finished tickets' worktrees (as last measured): what cleaning them would free.
    #[ts(type = "number")]
    pub reclaimable: u64,
}

/// `connection.failed` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionFailed {
    /// The connection's name, or for a forge login the forge and host (`GitHub (github.com)`).
    pub name: String,
    /// The connection's kind (`http`, `jira`, `scryer` …), or the forge's (`github`, `gitlab` …).
    pub kind: String,
    /// `connection` or `forge`.
    pub source: String,
    pub error: String,
}
