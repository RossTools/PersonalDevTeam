//! The files pdtd and pdt-host use to hand an update over (remote-host spec §3.4). Pure: types and paths only.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ts_rs::TS;

pub fn update_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("update")
}
/// Written by pdtd just before it shuts down for an update; taken (renamed) by pdt-host.
pub fn request_path(data_dir: &Path) -> PathBuf {
    update_dir(data_dir).join("request.json")
}
/// Written by pdt-host after an install attempt; shown by pdtd until dismissed.
pub fn result_path(data_dir: &Path) -> PathBuf {
    update_dir(data_dir).join("result.json")
}
/// Written by pdt-host whenever an update to a version fails or is rolled back; pdtd never installs that version
/// automatically (ruling B). Dismissing the result leaves it; a manual update or a later successful one clears it.
pub fn skip_path(data_dir: &Path) -> PathBuf {
    update_dir(data_dir).join("skip.json")
}
/// The argument pdt-host starts the tray app with after an install or rollback: the tray then skips opening the web
/// UI, since the browser already has it open and reconnects by itself.
pub const TRAY_QUIET_ARG: &str = "--no-open";
pub fn downloads_dir(data_dir: &Path) -> PathBuf {
    update_dir(data_dir).join("downloads")
}
/// pdtd's database in its data dir. SQLite in WAL mode: `<DB_FILE>-wal` and `<DB_FILE>-shm` sit beside it.
pub const DB_FILE: &str = "pdt.db";
/// pdt-host's copy of the database, taken (with pdtd stopped) before an update is installed and put back if the
/// update is rolled back: a new version may migrate the database, and the old one cannot open a migrated one.
pub fn db_snapshot_dir(data_dir: &Path) -> PathBuf {
    update_dir(data_dir).join("db-before-update")
}

/// Written by pdt-host while it cannot start pdtd for a reason someone has to fix (its exe is missing or blocked);
/// removed once pdtd answers its health check. The tray reads it, since nothing serves the web UI meanwhile.
pub fn host_problem_path(data_dir: &Path) -> PathBuf {
    data_dir.join("host-problem.json")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostProblemKind {
    /// pdtd's exe is not there (antivirus quarantine, a half-removed install).
    PdtdMissing,
    /// The OS refused to run pdtd's exe as a virus.
    PdtdBlocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostProblem {
    pub kind: HostProblemKind,
    /// A sentence for people: what is wrong and what to do.
    pub message: String,
    /// The pdtd exe pdt-host tries to start.
    pub path: PathBuf,
    /// When pdt-host first saw it (Unix ms).
    pub since: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequest {
    pub version: String,
    pub tag: String,
    /// The verified installer (Windows) or `.app.tar.gz` (macOS).
    pub asset: PathBuf,
    pub from_version: String,
    pub requested_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UpdateOutcome {
    Installed,
    RolledBack,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateResult {
    pub outcome: UpdateOutcome,
    pub from_version: String,
    pub to_version: String,
    pub message: String,
    /// The update log's path, when there is one.
    pub log: Option<String>,
    #[ts(type = "number")]
    pub at: i64,
}

/// The version automatic updates skip (see `skip_path`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSkip {
    pub version: String,
    pub reason: String,
    pub at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_live_under_the_data_dir() {
        let d = Path::new("data");
        assert_eq!(request_path(d), d.join("update").join("request.json"));
        assert_eq!(result_path(d), d.join("update").join("result.json"));
        assert_eq!(downloads_dir(d), d.join("update").join("downloads"));
        assert_eq!(skip_path(d), d.join("update").join("skip.json"));
        assert_eq!(host_problem_path(d), d.join("host-problem.json"));
    }

    #[test]
    fn request_roundtrips_as_camel_case_json() {
        let r = UpdateRequest {
            version: "0.1.7".into(),
            tag: "v0.1.7".into(),
            asset: PathBuf::from("a.exe"),
            from_version: "0.1.6".into(),
            requested_at: 1,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"fromVersion\":\"0.1.6\""), "{j}");
        assert_eq!(serde_json::from_str::<UpdateRequest>(&j).unwrap(), r);
    }
}
