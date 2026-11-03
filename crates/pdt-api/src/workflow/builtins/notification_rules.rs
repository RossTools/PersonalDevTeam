//! Notification rules (PDT-368): how far each kind of notification reaches the user, quiet hours, and the rules that
//! are off until turned on: escalating an Inbox item left unanswered, and posting questions, approvals and failed runs
//! to a chat service. Its inputs (template params, and the levels) are what Settings → Notifications shows.

use super::*;
use crate::workflow::nodes::notification_levels::LevelsConfig;
use crate::workflow::templates::{params, unfill};
use pdt_core::workflow::template::{TemplateParam, fill, param_values};
use serde_json::{Map, Value};

/// The input that holds the levels: kinds whose level differs from PDT's default. It is no template param (its value
/// is a map), so it is checked by the levels node.
pub const LEVELS: &str = "levels";
/// The node that holds the levels and quiet hours.
pub const LEVELS_NODE: &str = "levels";
/// How long an Inbox item waits before it is escalated, by default.
pub const ESCALATE_AFTER_MS: u64 = 2 * 60 * 60 * 1000;

/// Its inputs (but the levels).
pub fn rules_params() -> Vec<TemplateParam> {
    params(json!([
        {"id": "escalate", "title": "Escalate what waits", "type": "boolean", "default": false,
         "description": "An Inbox item still unanswered after a while comes back as a desktop notification (and to \
                         the chat channel, if one is set)."},
        {"id": "escalateAfter", "title": "Escalate after", "type": "duration", "default": ESCALATE_AFTER_MS,
         "description": "How long an Inbox item waits unanswered before it is escalated."},
        {"id": "quietHours", "title": "Quiet hours", "type": "boolean", "default": false,
         "description": "No toasts or desktop notifications between two times; they wait in the Inbox, and one \
                         summary comes when quiet hours end."},
        {"id": "quietFrom", "title": "Quiet from", "type": "string", "default": "22:00",
         "description": "When quiet hours start (HH:MM, this computer's time)."},
        {"id": "quietTo", "title": "Quiet until", "type": "string", "default": "07:00",
         "description": "When they end."},
        {"id": "chat", "title": "Chat channel", "type": "connection", "default": "",
         "description": "A connection to post to (a Discord webhook); empty: none."},
        {"id": "chatQuestions", "title": "Post questions", "type": "boolean", "default": true,
         "description": "An agent asks you something."},
        {"id": "chatApprovals", "title": "Post approvals", "type": "boolean", "default": true,
         "description": "A conversation asks to use a tool, or asks you to confirm."},
        {"id": "chatFailedRuns", "title": "Post failed runs", "type": "boolean", "default": true,
         "description": "A workflow run fails."},
    ]))
}

/// The document with `{{params.*}}` in it.
pub fn rules_template() -> WorkflowDoc {
    let chat_on = json!({"left": "{{params.chat}}", "op": "truthy"});
    let item = |kinds: Value, on: &str| {
        json!({"where": [chat_on, {"left": format!("{{{{params.{on}}}}}"), "op": "truthy"},
                         {"left": "{{trigger.kind}}", "op": "in", "right": kinds}]})
    };
    doc(json!({
        "name": "Notification rules",
        "description": "How far each kind of notification reaches you, and quiet hours (the Show levels step). Turn \
                        on escalating what waits unanswered, and posting questions, approvals and failed runs to a \
                        chat channel. Change them in Settings → Notifications. Turned off, every kind shows at PDT's \
                        default level.",
        "nodes": [
            {"id": LEVELS_NODE, "kind": "notification.levels", "version": 1, "position": {"x": 0, "y": -200},
             "title": "Show levels", "config": {"levels": "{{params.levels}}", "quietHours": {
                "on": "{{params.quietHours}}", "from": "{{params.quietFrom}}", "to": "{{params.quietTo}}"}}},
            {"id": "waiting", "kind": "inbox.created", "version": 1, "position": {"x": 0, "y": 0},
             "title": "Something waits on you",
             "config": {"where": [{"left": "{{params.escalate}}", "op": "truthy"}]}},
            {"id": "answered", "kind": "wait", "version": 1, "position": {"x": 300, "y": 0},
             "title": "Answered in time?", "config": {
                "event": {"type": "inbox.answered", "where": [
                    {"left": "{{event.itemId}}", "op": "eq", "right": "{{trigger.itemId}}"}]},
                "timeoutMs": "{{params.escalateAfter}}"}},
            {"id": "escalate", "kind": "notify", "version": 1, "position": {"x": 600, "y": 0},
             "title": "Escalate", "config": {
                "title": "Still waiting: {{trigger.title}}", "body": "Unanswered in the Inbox.",
                "level": "warning", "show": "desktop", "chat": "{{params.chat}}",
                "link": "inbox:{{trigger.itemId}}", "whileOpen": "{{trigger.itemId}}"}},
            {"id": "question", "kind": "inbox.created", "version": 1, "position": {"x": 0, "y": 200},
             "title": "A question", "config": item(json!(["question", "secret"]), "chatQuestions")},
            {"id": "approval", "kind": "inbox.created", "version": 1, "position": {"x": 0, "y": 350},
             "title": "An approval", "config": item(json!(["approval", "confirm"]), "chatApprovals")},
            {"id": "post_item", "kind": "notify", "version": 1, "position": {"x": 300, "y": 275},
             "title": "Post it", "config": {
                "title": "{{trigger.title}}", "body": "Waits on you in PDT's Inbox.", "show": "off",
                "chat": "{{params.chat}}", "link": "inbox:{{trigger.itemId}}"}},
            {"id": "failed", "kind": "run.failed", "version": 1, "position": {"x": 0, "y": 500},
             "title": "A run failed", "config": {"where": [chat_on, {"left": "{{params.chatFailedRuns}}", "op": "truthy"}]}},
            {"id": "post_failed", "kind": "notify", "version": 1, "position": {"x": 300, "y": 500},
             "title": "Post it", "config": {
                "title": "{{trigger.workflowName}} failed", "body": "{{trigger.error}}", "level": "error",
                "show": "off", "chat": "{{params.chat}}"}},
            {"id": "done", "kind": "end", "version": 1, "position": {"x": 900, "y": 200}, "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "waiting", "to": "answered"},
            {"from": "answered", "port": "timeout", "to": "escalate"},
            {"from": "answered", "port": "event", "to": "done"},
            {"from": "answered", "port": "done", "to": "done"},
            {"from": "escalate", "to": "done"},
            {"from": "question", "to": "post_item"},
            {"from": "approval", "to": "post_item"},
            {"from": "post_item", "to": "done"},
            {"from": "failed", "to": "post_failed"},
            {"from": "post_failed", "to": "done"},
        ],
    }))
}

/// The inputs' defaults (no level changed).
pub fn defaults() -> Map<String, Value> {
    let mut v = param_values(&rules_params(), &Map::new()).expect("the notification rules' defaults");
    v.insert(LEVELS.into(), json!({}));
    v
}

/// The workflow filled in with `values` (checked; missing ones take their defaults).
pub fn filled(values: &Map<String, Value>) -> Result<WorkflowDoc, String> {
    let mut given = values.clone();
    let levels = given.remove(LEVELS).filter(|l| !l.is_null()).unwrap_or_else(|| json!({}));
    if !levels.is_object() {
        return Err("levels is a map of kinds to levels".into());
    }
    let mut v = param_values(&rules_params(), &given)?;
    v.insert(LEVELS.into(), levels);
    let d = fill(&rules_template(), &v)?;
    let node = d.node(LEVELS_NODE).ok_or("the levels are missing")?;
    serde_json::from_value::<LevelsConfig>(node.config.clone()).map_err(crate::workflow::err)?.levels()?;
    Ok(d)
}

/// The built-in as PDT ships it.
pub(super) fn notification_rules() -> WorkflowDoc {
    filled(&Map::new()).expect("the notification rules fill in")
}

/// The inputs a document was filled in with: each one read back from it, else its default.
pub fn values_of(doc: &WorkflowDoc) -> Map<String, Value> {
    let mut out = defaults();
    for (k, v) in unfill(&rules_template(), doc) {
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

/// The built-in as first installed: the levels the user had set in Settings before they were the workflow's.
pub(super) fn migrated(old_levels: &Value) -> WorkflowDoc {
    let levels = pdt_core::notification::NotificationSettings::from_json(old_levels).to_json();
    let mut v = Map::new();
    v.insert(LEVELS.into(), levels);
    filled(&v).unwrap_or_else(|_| notification_rules())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::validate::{Severity, validate};

    /// Every port leads somewhere, every node is reached, and nothing is turned on.
    #[test]
    fn the_defaults_change_nothing() {
        let d = notification_rules();
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        assert!(!issues.iter().any(|i| i.code == "unreachable" || i.code == "open_port"), "{issues:?}");
        assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
        let levels = &d.node(LEVELS_NODE).unwrap().config;
        assert_eq!(levels["levels"], json!({}), "every kind at PDT's default");
        assert_eq!(levels["quietHours"], json!({"on": false, "from": "22:00", "to": "07:00"}));
        assert_eq!(d.node("waiting").unwrap().config["where"][0]["left"], false, "no escalation");
        assert_eq!(d.node("answered").unwrap().config["timeoutMs"], ESCALATE_AFTER_MS);
        assert_eq!(d.node("question").unwrap().config["where"][0]["left"], "", "no chat channel");
        assert!(!customized(&d));
        assert_eq!(values_of(&d), defaults());
    }

    /// Changed inputs land in the document and read back; an edit beyond them is a customization.
    #[test]
    fn inputs_fill_in_and_read_back() {
        let given = json!({"levels": {"pr.merged": "toast"}, "escalate": true, "escalateAfter": "30m",
                           "quietHours": true, "quietFrom": "23:00", "chat": "team", "chatApprovals": false});
        let d = filled(given.as_object().unwrap()).unwrap();
        let levels = &d.node(LEVELS_NODE).unwrap().config;
        assert_eq!(levels["levels"], json!({"pr.merged": "toast"}));
        assert_eq!(levels["quietHours"], json!({"on": true, "from": "23:00", "to": "07:00"}));
        assert_eq!(d.node("answered").unwrap().config["timeoutMs"], 30 * 60 * 1000);
        assert_eq!(d.node("escalate").unwrap().config["chat"], "team");
        assert_eq!(d.node("approval").unwrap().config["where"][1]["left"], false);
        let v = values_of(&d);
        assert_eq!((v["escalate"].clone(), v["chat"].clone()), (json!(true), json!("team")));
        assert_eq!(v["levels"], json!({"pr.merged": "toast"}));
        assert!(!customized(&d));
        let mut edited = d.clone();
        edited.nodes.retain(|n| n.id != "post_failed");
        assert!(customized(&edited));
        for bad in [json!({"quietFrom": "late"}), json!({"levels": {"inbox.question": "off"}}), json!({"levels": 3})] {
            assert!(filled(bad.as_object().unwrap()).is_err(), "{bad}");
        }
    }

    /// The levels the user had changed come along when the workflow is first installed.
    #[test]
    fn changed_levels_carry_over() {
        let d = migrated(&json!({"pr.merged": "desktop", "inbox.gate": "inbox", "nope": "off"}));
        assert_eq!(
            d.node(LEVELS_NODE).unwrap().config["levels"],
            json!({"pr.merged": "desktop", "inbox.gate": "inbox"})
        );
        assert!(!customized(&d));
        assert_eq!(migrated(&Value::Null), notification_rules());
    }
}
