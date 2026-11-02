//! Install updates (PDT-369): what PDT does with an update a check found, under the host's update setting. By default
//! today's install when idle: wait until nothing is busy, then install. Its inputs (template params) are what
//! Settings → Updates shows: a time window, stable releases only, installing anyway after a while, the check after
//! installing and telling what changed. A failed install or health check goes to the Inbox.

use super::*;
use crate::workflow::templates::{params, unfill};
use pdt_core::workflow::template::{TemplateParam, fill, param_values};
use serde_json::{Map, Value};

/// How long a run may wait for PDT to be idle (and the window) before it fails.
const MAX_RUNTIME_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// A failed download is tried again after this, and twice as long the second time.
const RETRY_BACKOFF_MS: u64 = 10 * 60 * 1000;

/// Its inputs.
pub fn install_params() -> Vec<TemplateParam> {
    params(json!([
        {"id": "stableOnly", "title": "Stable releases only", "type": "boolean", "default": false,
         "description": "On the nightly channel, a nightly build is not installed by itself (install it from the \
                         banner)."},
        {"id": "window", "title": "Only in a time window", "type": "boolean", "default": false,
         "description": "Updates install by themselves only between two times."},
        {"id": "windowFrom", "title": "Window from", "type": "string", "default": "02:00",
         "description": "When the window opens (HH:MM, this computer's time)."},
        {"id": "windowTo", "title": "Window until", "type": "string", "default": "06:00",
         "description": "When it closes."},
        {"id": "installAnywayAfter", "title": "Install anyway after", "type": "duration", "default": 0,
         "description": "While agents work, wait this long, then install anyway (sessions resume afterwards). 0: \
                         wait for them to finish."},
        {"id": "checkAfter", "title": "Check PDT after installing", "type": "boolean", "default": true,
         "description": "A few minutes after PDT starts on the new version, check it runs that version and its \
                         background loops work; if not, it goes to the Inbox."},
        {"id": "announce", "title": "Tell me what changed", "type": "boolean", "default": false,
         "description": "A notification, linking to the release notes, once PDT runs the new version."},
        {"id": "chat", "title": "Chat channel", "type": "connection", "default": "",
         "description": "A connection to post that to as well (a Discord webhook); empty: none."},
    ]))
}

/// The document with `{{params.*}}` in it.
pub fn install_template() -> WorkflowDoc {
    let install = |force: bool| json!({"version": "{{trigger.version}}", "automatic": true, "force": force});
    let failed = |step: &str| {
        json!({"key": "update:{{trigger.version}}", "title": "PDT couldn't update to v{{trigger.version}}",
               "text": format!("{{{{steps.{step}.error}}}}\n\nInstall it from Settings → Updates to try again.")})
    };
    let retry = json!({"max": 2, "backoffMs": RETRY_BACKOFF_MS});
    doc(json!({
        "name": "Install updates",
        "description": "What PDT does with an update a check found, while updates are set to install when idle: it \
                        waits until no agent is working, then installs it, and checks PDT after it restarts. Set a \
                        time window, stable releases only, installing anyway after a while and the check in \
                        Settings → Updates. A failed install or check goes to the Inbox. Turned off, updates are \
                        only installed when you say.",
        "settings": {"maxSteps": 200, "maxRuntimeMs": MAX_RUNTIME_MS},
        "nodes": [
            {"id": "available", "kind": "update.available", "version": 1, "position": {"x": 0, "y": 0},
             "title": "An update is available"},
            {"id": "nightly_skipped", "kind": "condition", "version": 1, "position": {"x": 300, "y": 0},
             "title": "A nightly build, stable only?", "config": {"conditions": [
                {"left": "{{params.stableOnly}}", "op": "truthy"}, {"left": "{{trigger.nightly}}", "op": "truthy"}]}},
            {"id": "wait", "kind": "host.wait_idle", "version": 1, "position": {"x": 600, "y": 0},
             "title": "Wait until PDT is idle", "config": {
                "update": "{{trigger.version}}",
                "window": {"on": "{{params.window}}", "from": "{{params.windowFrom}}", "to": "{{params.windowTo}}"},
                "maxWaitMs": "{{params.installAnywayAfter}}"}},
            {"id": "install", "kind": "host.install_update", "version": 1, "position": {"x": 900, "y": -100},
             "title": "Install it", "config": install(false), "retry": retry},
            {"id": "install_anyway", "kind": "host.install_update", "version": 1, "position": {"x": 900, "y": 100},
             "title": "Install it anyway", "config": install(true), "retry": retry},
            {"id": "install_failed", "kind": "inbox.notice", "version": 1, "position": {"x": 1200, "y": -100},
             "title": "Tell me it failed", "config": failed("install")},
            {"id": "anyway_failed", "kind": "inbox.notice", "version": 1, "position": {"x": 1200, "y": 100},
             "title": "Tell me it failed", "config": failed("install_anyway")},
            {"id": "installed", "kind": "update.installed", "version": 1, "position": {"x": 0, "y": 350},
             "title": "PDT was updated"},
            {"id": "check_on", "kind": "condition", "version": 1, "position": {"x": 300, "y": 350},
             "title": "Check it?", "config": {"conditions": [{"left": "{{params.checkAfter}}", "op": "truthy"}]}},
            {"id": "health", "kind": "host.check_health", "version": 1, "position": {"x": 600, "y": 300},
             "title": "Check PDT is healthy", "config": {"version": "{{trigger.version}}"}},
            {"id": "unhealthy", "kind": "inbox.notice", "version": 1, "position": {"x": 900, "y": 250},
             "title": "Tell me what's wrong", "config": {
                "key": "update:{{trigger.version}}:health",
                "title": "PDT v{{trigger.version}} isn't healthy after updating",
                "text": "{{steps.health.error}}\n\nWhat changed: {{trigger.notesUrl}}"}},
            {"id": "announce_on", "kind": "condition", "version": 1, "position": {"x": 900, "y": 400},
             "title": "Tell me what changed?", "config": {"conditions": [
                {"left": "{{params.announce}}", "op": "truthy"}]}},
            {"id": "announce", "kind": "notify", "version": 1, "position": {"x": 1200, "y": 400},
             "title": "Tell me what changed", "config": {
                "title": "PDT updated to v{{trigger.version}}",
                "body": "From v{{trigger.fromVersion}}. See what changed in the release notes.",
                "link": "{{trigger.notesUrl}}", "chat": "{{params.chat}}"}},
            {"id": "failed", "kind": "update.failed", "version": 1, "position": {"x": 0, "y": 600},
             "title": "An update failed"},
            {"id": "failed_notice", "kind": "inbox.notice", "version": 1, "position": {"x": 300, "y": 600},
             "title": "Tell me it failed", "config": {
                "key": "update:{{trigger.version}}:failed:{{trigger.at}}",
                "title": "PDT couldn't update to v{{trigger.version}}",
                "text": "{{trigger.message}}\n\nPDT runs v{{trigger.fromVersion}}. Install it from Settings → Updates \
                         to try again."}},
            {"id": "done", "kind": "end", "version": 1, "position": {"x": 1500, "y": 200}, "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "available", "to": "nightly_skipped"},
            {"from": "nightly_skipped", "port": "true", "to": "done"},
            {"from": "nightly_skipped", "port": "false", "to": "wait"},
            {"from": "wait", "port": "idle", "to": "install"},
            {"from": "wait", "port": "timeout", "to": "install_anyway"},
            {"from": "wait", "port": "skipped", "to": "done"},
            {"from": "install", "port": "installing", "to": "done"},
            {"from": "install", "port": "busy", "to": "wait"},
            {"from": "install", "port": "skipped", "to": "done"},
            {"from": "install", "port": "error", "to": "install_failed"},
            {"from": "install_anyway", "port": "installing", "to": "done"},
            {"from": "install_anyway", "port": "busy", "to": "wait"},
            {"from": "install_anyway", "port": "skipped", "to": "done"},
            {"from": "install_anyway", "port": "error", "to": "anyway_failed"},
            {"from": "install_failed", "port": "added", "to": "done"},
            {"from": "install_failed", "port": "skipped", "to": "done"},
            {"from": "anyway_failed", "port": "added", "to": "done"},
            {"from": "anyway_failed", "port": "skipped", "to": "done"},
            {"from": "installed", "to": "check_on"},
            {"from": "check_on", "port": "true", "to": "health"},
            {"from": "check_on", "port": "false", "to": "announce_on"},
            {"from": "health", "port": "healthy", "to": "announce_on"},
            {"from": "health", "port": "unhealthy", "to": "unhealthy"},
            {"from": "unhealthy", "port": "added", "to": "done"},
            {"from": "unhealthy", "port": "skipped", "to": "done"},
            {"from": "announce_on", "port": "true", "to": "announce"},
            {"from": "announce_on", "port": "false", "to": "done"},
            {"from": "announce", "to": "done"},
            {"from": "failed", "to": "failed_notice"},
            {"from": "failed_notice", "port": "added", "to": "done"},
            {"from": "failed_notice", "port": "skipped", "to": "done"},
        ],
    }))
}

/// The inputs' defaults.
pub fn defaults() -> Map<String, Value> {
    param_values(&install_params(), &Map::new()).expect("Install updates' defaults")
}

/// The workflow filled in with `values` (checked; missing ones take their defaults).
pub fn filled(values: &Map<String, Value>) -> Result<WorkflowDoc, String> {
    let v = param_values(&install_params(), values)?;
    let d = fill(&install_template(), &v)?;
    let wait = d.node("wait").ok_or("the wait is missing")?;
    crate::workflow::nodes::host_update::check_wait(&wait.config)?;
    Ok(d)
}

/// The built-in as PDT ships it.
pub(super) fn install_updates() -> WorkflowDoc {
    filled(&Map::new()).expect("Install updates fills in")
}

/// The inputs a document was filled in with: each one read back from it, else its default.
pub fn values_of(doc: &WorkflowDoc) -> Map<String, Value> {
    let mut out = defaults();
    for (k, v) in unfill(&install_template(), doc) {
        if out.contains_key(&k) {
            out.insert(k, v);
        }
    }
    out
}

/// Whether a document is the workflow filled in with some inputs (its name and description aside): else the user
/// changed it beyond its inputs.
pub fn customized(doc: &WorkflowDoc) -> bool {
    match filled(&values_of(doc)) {
        Ok(mut d) => {
            (d.name, d.description) = (doc.name.clone(), doc.description.clone());
            d != *doc
        }
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::validate::{Severity, validate};

    /// Every port leads somewhere, every node is reached, and the defaults are today's: install when idle, at any
    /// time, waiting for agents however long; the check after installing is on, telling what changed off.
    #[test]
    fn the_defaults_install_when_idle() {
        let d = install_updates();
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        assert!(!issues.iter().any(|i| i.code == "unreachable" || i.code == "open_port"), "{issues:?}");
        assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
        let wait = &d.node("wait").unwrap().config;
        assert_eq!(wait["window"]["on"], false);
        assert_eq!(wait["maxWaitMs"], 0, "waits for agents");
        assert_eq!(wait["update"], "{{trigger.version}}");
        assert_eq!(d.node("nightly_skipped").unwrap().config["conditions"][0]["left"], false);
        assert_eq!(d.node("check_on").unwrap().config["conditions"][0]["left"], true);
        assert_eq!(d.node("announce_on").unwrap().config["conditions"][0]["left"], false);
        assert_eq!(d.node("install").unwrap().config["force"], false);
        assert_eq!(d.node("install_anyway").unwrap().config["force"], true);
        assert!(!customized(&d));
        assert_eq!(values_of(&d), defaults());
    }

    /// Changed inputs land in the document and read back; an edit beyond them is a customization.
    #[test]
    fn inputs_fill_in_and_read_back() {
        let given = json!({"window": true, "windowFrom": "01:30", "windowTo": "05:00", "installAnywayAfter": "4h",
                           "stableOnly": true, "announce": true, "chat": "team"});
        let d = filled(given.as_object().unwrap()).unwrap();
        let wait = &d.node("wait").unwrap().config;
        assert_eq!(wait["window"], json!({"on": true, "from": "01:30", "to": "05:00"}));
        assert_eq!(wait["maxWaitMs"], 4 * 3_600_000);
        assert_eq!(d.node("announce").unwrap().config["chat"], "team");
        let v = values_of(&d);
        assert_eq!(
            (v["window"].clone(), v["installAnywayAfter"].clone(), v["checkAfter"].clone()),
            (json!(true), json!(4 * 3_600_000), json!(true))
        );
        assert!(!customized(&d));
        let mut edited = d.clone();
        edited.nodes.retain(|n| n.id != "announce");
        assert!(customized(&edited));
        for bad in [json!({"windowFrom": "2am", "window": true}), json!({"installAnywayAfter": "soon"})] {
            assert!(filled(bad.as_object().unwrap()).is_err(), "{bad}");
        }
    }
}
