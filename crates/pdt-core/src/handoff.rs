//! The files pdtd and pdt-host use to hand an update over. Pure: types and paths only.

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
/// Where pdt-host moves the request it took (`take_request`): the next pdtd reads whether its update was automatic.
pub fn taken_request_path(data_dir: &Path) -> PathBuf {
    request_path(data_dir).with_extension("taken.json")
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequest {
    pub version: String,
    pub tag: String,
    /// The verified installer (Windows) or `.app.tar.gz` (macOS).
    pub asset: PathBuf,
    pub from_version: String,
    pub requested_at: i64,
    /// pdtd installs it by itself (the automatic update when idle), not because the user asked (PDT-365).
    #[serde(default)]
    pub automatic: bool,
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

/// Written by pdtd at startup: its version and the `PDT_*` variables it was started with (`StartedEnv`). pdt-host
/// reads it when it adopts a running pdtd, and restarts one started with other variables than it would use (PDT-261:
/// an older pdt-host started it during an update).
pub fn started_env_path(data_dir: &Path) -> PathBuf {
    data_dir.join("pdtd-env.json")
}

/// What `started_env_path` holds. The values are only fingerprinted: a `PDT_*` variable may hold a token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedEnv {
    /// The version of the pdtd that wrote it (a record left by another version says nothing about the running one).
    pub version: String,
    /// The variables' names, sorted.
    pub names: Vec<String>,
    /// `env_fingerprint` of the variables.
    pub fingerprint: String,
}

impl StartedEnv {
    pub fn new(version: &str, vars: impl IntoIterator<Item = (String, String)>) -> StartedEnv {
        let vars: std::collections::BTreeMap<String, String> = vars.into_iter().collect();
        StartedEnv {
            version: version.to_string(),
            names: vars.keys().cloned().collect(),
            fingerprint: env_fingerprint(vars),
        }
    }

    /// Whether the record is of a pdtd of `version` started with exactly `vars`.
    pub fn matches(&self, version: &str, vars: impl IntoIterator<Item = (String, String)>) -> bool {
        self.version == version && self.fingerprint == env_fingerprint(vars)
    }
}

/// A fingerprint of a set of variables, the same whatever their order (64-bit FNV-1a, stable across builds).
pub fn env_fingerprint(vars: impl IntoIterator<Item = (String, String)>) -> String {
    let vars: std::collections::BTreeMap<String, String> = vars.into_iter().collect();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (k, v) in &vars {
        for b in k.bytes().chain(*b"=").chain(v.bytes()).chain(*b"\0") {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_started_env_matches_only_the_same_version_and_variables() {
        let rec = StartedEnv::new("0.3.0", vars(&[("PDT_PORT", "7420"), ("PDT_PORT_FROM_SETTING", "1")]));
        assert_eq!(rec.names, ["PDT_PORT", "PDT_PORT_FROM_SETTING"]);
        assert!(rec.matches("0.3.0", vars(&[("PDT_PORT_FROM_SETTING", "1"), ("PDT_PORT", "7420")])), "any order");
        assert!(!rec.matches("0.3.0", vars(&[("PDT_PORT", "7420")])), "a variable missing");
        assert!(!rec.matches("0.3.0", vars(&[("PDT_PORT", "7421"), ("PDT_PORT_FROM_SETTING", "1")])), "a value");
        assert!(!rec.matches("0.2.0", vars(&[("PDT_PORT", "7420"), ("PDT_PORT_FROM_SETTING", "1")])), "a version");
        // Name and value boundaries count: `A=BC` is not `AB=C`.
        assert_ne!(env_fingerprint(vars(&[("A", "BC")])), env_fingerprint(vars(&[("AB", "C")])));
        let j = serde_json::to_string(&rec).unwrap();
        assert!(!j.contains("7420"), "values are not written: {j}");
        assert_eq!(serde_json::from_str::<StartedEnv>(&j).unwrap(), rec);
    }

    #[test]
    fn paths_live_under_the_data_dir() {
        let d = Path::new("data");
        assert_eq!(request_path(d), d.join("update").join("request.json"));
        assert_eq!(result_path(d), d.join("update").join("result.json"));
        assert_eq!(downloads_dir(d), d.join("update").join("downloads"));
        assert_eq!(skip_path(d), d.join("update").join("skip.json"));
        assert_eq!(taken_request_path(d), d.join("update").join("request.taken.json"));
    }

    #[test]
    fn request_roundtrips_as_camel_case_json() {
        let r = UpdateRequest {
            version: "0.1.7".into(),
            tag: "v0.1.7".into(),
            asset: PathBuf::from("a.exe"),
            from_version: "0.1.6".into(),
            requested_at: 1,
            automatic: true,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"fromVersion\":\"0.1.6\""), "{j}");
        assert_eq!(serde_json::from_str::<UpdateRequest>(&j).unwrap(), r);
        // An older pdtd's request has no `automatic`: it was the user's.
        let old = j.replace(",\"automatic\":true", "");
        assert!(!serde_json::from_str::<UpdateRequest>(&old).unwrap().automatic, "{old}");
    }
}
