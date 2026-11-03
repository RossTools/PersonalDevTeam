//! Clean up storage (PDT-370; the storage loop did it before): hourly, finished tickets' worktrees are cleaned as their
//! projects' Worktrees settings say; when a disk PDT uses is almost full, pickup pauses and every finished ticket's
//! build output is cleaned at once; once there is space again, pickup resumes. Its inputs are what Settings › Storage
//! shows: the low and critical thresholds (the storage loop's `disk.low` / `disk.recovered` use them too), and what to
//! do when a disk is almost full. Measuring stays the storage loop's, and the floor that stops PDT writing to a full
//! disk stays PDT's own.

use super::inputs::InputsBuiltin;
use super::*;
use crate::workflow::templates::params;
use pdt_core::host::HostSettings;
use pdt_core::storage::{DEFAULT_CRITICAL_DISK_GB, DEFAULT_LOW_DISK_GB};
use pdt_core::workflow::template::TemplateParam;
use serde_json::{Map, Value};

pub static SETTINGS: InputsBuiltin = InputsBuiltin {
    id: CLEAN_UP_STORAGE,
    params: storage_params,
    template: storage_template,
    finish: Some(finish),
    read: None,
    extra: None,
};

/// The most a threshold can be, in GB.
const MAX_GB: u64 = 10_000;

/// Its inputs.
pub fn storage_params() -> Vec<TemplateParam> {
    params(json!([
        {"id": "lowGb", "title": "Low-disk warning", "type": "number", "default": DEFAULT_LOW_DISK_GB,
         "description": "The Inbox warns when a disk with PDT's data or a project has less free than this (GB; 0: \
                         never)."},
        {"id": "criticalGb", "title": "Almost full below", "type": "number", "default": DEFAULT_CRITICAL_DISK_GB,
         "description": "Below this (GB; 0: never) the disk is almost full, and PDT does what is set below."},
        {"id": "pausePickup", "title": "Pause pickup", "type": "boolean", "default": true,
         "description": "While a disk is almost full, auto-start starts no new tickets. Running agents carry on."},
        {"id": "cleanNow", "title": "Clean finished worktrees now", "type": "boolean", "default": true,
         "description": "When a disk is almost full, remove the build output of every Done or Won't do ticket's \
                         worktree at once, not waiting for their projects' delays."},
    ]))
}

/// The document with `{{params.*}}` in it.
pub fn storage_template() -> WorkflowDoc {
    doc(json!({
        "name": "Clean up storage",
        "description": "Every hour, cleans finished tickets' worktrees as each project's Worktrees settings say. When \
                        a disk PDT uses is almost full, pauses pickup and cleans every finished ticket's build output \
                        at once; once there is space again, pickup resumes. Set the thresholds and what happens in \
                        Settings › Storage.",
        "nodes": [
            {"id": "hourly", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 0}, "title": "Every hour",
             "config": {"every": "1h"}},
            {"id": "clean_due", "kind": "storage.clean", "version": 1, "position": {"x": 260, "y": 0},
             "title": "Clean finished worktrees that are due", "config": {"which": "due"}},
            {"id": "low", "kind": "disk.low", "version": 1, "position": {"x": 260, "y": 200},
             "title": "Disk space low"},
            {"id": "recovered", "kind": "disk.recovered", "version": 1, "position": {"x": 260, "y": 350},
             "title": "Disk space recovered"},
            {"id": "measure", "kind": "storage.measure", "version": 1, "position": {"x": 520, "y": 150},
             "title": "Check disk space", "config": {"lowGb": "{{params.lowGb}}", "criticalGb": "{{params.criticalGb}}"}},
            {"id": "pause_on", "kind": "condition", "version": 1, "position": {"x": 780, "y": 0},
             "title": "Pause pickup?", "config": {"conditions": [{"left": "{{params.pausePickup}}", "op": "truthy"}]}},
            {"id": "pause", "kind": "pickup.pause", "version": 1, "position": {"x": 1040, "y": -80},
             "title": "Pause pickup",
             "config": {"reason": "the disk is almost full ({{steps.measure.freeText}} free)"}},
            {"id": "clean_on", "kind": "condition", "version": 1, "position": {"x": 1300, "y": 0},
             "title": "Clean finished worktrees now?",
             "config": {"conditions": [{"left": "{{params.cleanNow}}", "op": "truthy"}]}},
            {"id": "clean_now", "kind": "storage.clean", "version": 1, "position": {"x": 1560, "y": -80},
             "title": "Clean every finished worktree", "config": {"which": "finished"}},
            {"id": "resume", "kind": "pickup.resume", "version": 1, "position": {"x": 780, "y": 300},
             "title": "Resume pickup"},
            {"id": "done", "kind": "end", "version": 1, "position": {"x": 1820, "y": 150}, "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "hourly", "to": "clean_due"},
            {"from": "clean_due", "to": "measure"},
            {"from": "low", "to": "measure"},
            {"from": "recovered", "to": "measure"},
            {"from": "measure", "port": "critical", "to": "pause_on"},
            {"from": "measure", "port": "low", "to": "resume"},
            {"from": "measure", "port": "ok", "to": "resume"},
            {"from": "pause_on", "port": "true", "to": "pause"},
            {"from": "pause_on", "port": "false", "to": "clean_on"},
            {"from": "pause", "to": "clean_on"},
            {"from": "clean_on", "port": "true", "to": "clean_now"},
            {"from": "clean_on", "port": "false", "to": "done"},
            {"from": "clean_now", "to": "done"},
            {"from": "resume", "port": "resumed", "to": "done"},
            {"from": "resume", "port": "not_paused", "to": "done"},
        ],
    }))
}

/// Checks the thresholds: whole numbers of GB, at most [`MAX_GB`].
fn finish(d: WorkflowDoc, v: &Map<String, Value>) -> Result<WorkflowDoc, String> {
    for (id, what) in [("lowGb", "The low-disk warning"), ("criticalGb", "The almost-full threshold")] {
        let n = v.get(id).and_then(Value::as_u64).filter(|n| *n <= MAX_GB);
        if n.is_none() {
            return Err(format!("{what} is a whole number of GB from 0 to {MAX_GB}"));
        }
    }
    Ok(d)
}

/// The thresholds (low, critical) in GB from the inputs.
pub fn thresholds(v: &Map<String, Value>) -> (u32, u32) {
    let gb = |id: &str, default: u32| {
        v.get(id).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(default)
    };
    (gb("lowGb", DEFAULT_LOW_DISK_GB), gb("criticalGb", DEFAULT_CRITICAL_DISK_GB))
}

/// The built-in as PDT ships it.
pub(super) fn clean_up_storage() -> WorkflowDoc {
    SETTINGS.shipped()
}

/// The built-in as first installed on a host whose settings held other thresholds: it keeps them.
pub(super) fn migrated(host: &HostSettings) -> WorkflowDoc {
    let mut v = Map::new();
    v.insert("lowGb".into(), json!(host.low_disk_gb));
    v.insert("criticalGb".into(), json!(host.critical_disk_gb));
    SETTINGS.filled(&v).unwrap_or_else(|_| clean_up_storage())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::validate::{Severity, validate};

    fn filled(given: Value) -> Result<WorkflowDoc, String> {
        SETTINGS.filled(given.as_object().unwrap())
    }

    /// The defaults are today's: 20 GB low, 5 GB critical, pause pickup and clean everything finished when critical;
    /// every port leads somewhere.
    #[test]
    fn the_default_is_todays_policy() {
        let d = clean_up_storage();
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        assert!(!issues.iter().any(|i| i.code == "unreachable" || i.code == "open_port"), "{issues:?}");
        assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
        let measure = &d.node("measure").unwrap().config;
        assert_eq!((measure["lowGb"].clone(), measure["criticalGb"].clone()), (json!(20), json!(5)));
        assert_eq!(d.node("hourly").unwrap().config["every"], "1h");
        assert_eq!(d.node("clean_due").unwrap().config["which"], "due");
        assert_eq!(d.node("clean_now").unwrap().config["which"], "finished");
        assert_eq!(d.node("pause_on").unwrap().config["conditions"][0]["left"], true);
        assert_eq!(d.node("clean_on").unwrap().config["conditions"][0]["left"], true);
        let v = SETTINGS.values_of(&d);
        assert_eq!(v, SETTINGS.defaults());
        assert_eq!(thresholds(&v), (20, 5));
        assert!(!SETTINGS.customized(&d));
    }

    /// Changed inputs fill in and read back; bad ones are refused; an edit beyond them is a customization.
    #[test]
    fn inputs_fill_in_and_read_back() {
        let d = filled(json!({"lowGb": "50", "criticalGb": 10, "pausePickup": false, "cleanNow": false})).unwrap();
        assert_eq!(d.node("measure").unwrap().config["criticalGb"], 10);
        assert_eq!(d.node("pause_on").unwrap().config["conditions"][0]["left"], false);
        let v = SETTINGS.values_of(&d);
        assert_eq!(thresholds(&v), (50, 10));
        assert_eq!((v["pausePickup"].clone(), v["cleanNow"].clone()), (json!(false), json!(false)));
        assert!(!SETTINGS.customized(&d));
        for bad in [json!({"lowGb": -1}), json!({"criticalGb": 2.5}), json!({"lowGb": 20000}), json!({"lowGb": "x"})] {
            assert!(filled(bad.clone()).is_err(), "{bad}");
        }
        let mut d = clean_up_storage();
        d.nodes.iter_mut().find(|n| n.id == "hourly").unwrap().config = json!({"every": "6h"});
        assert!(SETTINGS.customized(&d));
    }

    /// A host whose settings held its own thresholds keeps them.
    #[test]
    fn the_host_settings_thresholds_come_along() {
        let host = HostSettings { low_disk_gb: 40, critical_disk_gb: 0, ..Default::default() };
        let d = migrated(&host);
        assert_eq!(thresholds(&SETTINGS.values_of(&d)), (40, 0));
        assert!(!SETTINGS.customized(&d));
        assert_eq!(migrated(&HostSettings::default()), clean_up_storage());
    }
}
