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
pub fn downloads_dir(data_dir: &Path) -> PathBuf {
    update_dir(data_dir).join("downloads")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_live_under_the_data_dir() {
        let d = Path::new("data");
        assert_eq!(request_path(d), d.join("update").join("request.json"));
        assert_eq!(result_path(d), d.join("update").join("result.json"));
        assert_eq!(downloads_dir(d), d.join("update").join("downloads"));
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
