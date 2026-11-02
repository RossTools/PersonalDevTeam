//! The PM's daily update and journal checkpoint.

use super::inputs::InputsBuiltin;
use super::*;
use crate::workflow::templates::params;
use pdt_core::workflow::template::TemplateParam;
use serde_json::{Map, Value};

/// The PM daily checkpoint (PDT-372; hard-coded at the end of a turn before): every hour, each open PM conversation
/// you wrote to since its last update, a day or more ago, gets what happened since (and writes a journal checkpoint);
/// one with nothing new is asked only for the checkpoint. Its inputs are what Settings → This host → PM shows
/// (PDT-384): a time of day instead of every hour, the days it runs, and a chat channel to post the update to.
/// Journaling when a conversation is reset or closed stays PDT's own.
pub static SETTINGS: InputsBuiltin = InputsBuiltin {
    id: PM_DAILY_CHECKPOINT,
    params: checkpoint_params,
    template: checkpoint_template,
    finish: Some(finish),
    read: Some(read),
};

/// The days, as the `days` input and cron name them, Monday first.
pub const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// How long after its last update a conversation gets the next, checked every hour.
const HOURLY_AFTER: &str = "1d";
/// The same at a set time: less than a day, so a slot a few seconds earlier than yesterday's still sends one.
const AT_TIME_AFTER: &str = "12h";

/// Its inputs.
pub fn checkpoint_params() -> Vec<TemplateParam> {
    params(json!([
        {"id": "atTime", "title": "At a set time", "type": "boolean", "default": false,
         "description": "Send the updates once a day at a set time; off: each hour, to a conversation whose last \
                         update was a day or more ago."},
        {"id": "time", "title": "Time", "type": "string", "default": "09:00",
         "description": "When they are sent (HH:MM, this computer's time)."},
        {"id": "days", "title": "Days", "type": "list", "default": DAYS,
         "description": "The days they are sent on (mon, tue, wed, thu, fri, sat, sun)."},
        {"id": "chat", "title": "Chat channel", "type": "connection", "default": "",
         "description": "A connection to post the daily update to as well (a Discord webhook); empty: none."},
    ]))
}

/// The document with `{{params.*}}` in it; the schedule and the wait are [`finish`]'s.
pub fn checkpoint_template() -> WorkflowDoc {
    doc(json!({
        "name": "PM daily checkpoint",
        "description": "Keeps the PM's open conversations current: each hour, a conversation you wrote to since its \
                        last update, a day or more ago, gets what happened since (tickets, the Inbox, what other PM \
                        conversations left open) and writes a journal checkpoint; on a quiet day it is asked only for \
                        the checkpoint. Set a time, the days and a chat channel to post the update to in Settings → \
                        This host → PM.",
        "nodes": [
            {"id": "hourly", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 0}, "title": "Every hour",
             "config": {"every": "1h", "catchUp": true}},
            {"id": "update", "kind": "pm.update", "version": 1, "position": {"x": 260, "y": 0},
             "title": "Send the daily updates", "config": {"after": HOURLY_AFTER, "onlyInUse": true}},
            {"id": "post_on", "kind": "condition", "version": 1, "position": {"x": 520, "y": -120},
             "title": "Post it to chat?", "config": {"conditions": [{"left": "{{params.chat}}", "op": "truthy"}]}},
            {"id": "post", "kind": "notify", "version": 1, "position": {"x": 780, "y": -120},
             "title": "Post the update", "config": {
                "title": "PM daily update", "body": "{{steps.update.text}}", "show": "off", "chat": "{{params.chat}}"}},
            {"id": "checkpoint", "kind": "pm.checkpoint", "version": 1, "position": {"x": 1040, "y": 0},
             "title": "Ask the quiet ones for a checkpoint",
             "config": {"conversations": "{{steps.update.nothingNew}}"}},
            {"id": "done", "kind": "end", "version": 1, "position": {"x": 1300, "y": 0}, "title": "Done"},
        ],
        "edges": [
            {"from": "hourly", "to": "update"},
            {"from": "update", "port": "updated", "to": "post_on"},
            {"from": "update", "port": "none", "to": "checkpoint"},
            {"from": "post_on", "port": "true", "to": "post"},
            {"from": "post_on", "port": "false", "to": "checkpoint"},
            {"from": "post", "to": "checkpoint"},
            {"from": "checkpoint", "to": "done"},
        ],
    }))
}

/// `"HH:MM"` as (hour, minute).
fn hour_minute(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// The schedule and the wait from the inputs: every hour on every day is `every: 1h` (as before Settings set it);
/// otherwise a cron expression on the days, at the time or each hour.
fn finish(mut d: WorkflowDoc, v: &Map<String, Value>) -> Result<WorkflowDoc, String> {
    let at_time = v.get("atTime").and_then(Value::as_bool).unwrap_or(false);
    let picked: Vec<String> = v
        .get("days")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str()).map(|s| s.to_ascii_lowercase()).collect())
        .unwrap_or_default();
    if let Some(bad) = picked.iter().find(|d| !DAYS.contains(&d.as_str())) {
        return Err(format!("{bad:?} is not a day (mon, tue, wed, thu, fri, sat or sun)"));
    }
    let days: Vec<&str> = DAYS.iter().copied().filter(|d| picked.iter().any(|p| p == d)).collect();
    if days.is_empty() {
        return Err("pick at least one day".into());
    }
    let time = v.get("time").and_then(Value::as_str).unwrap_or_default();
    let (h, m) = hour_minute(time).ok_or_else(|| format!("{time:?} is not a time (HH:MM)"))?;
    let time = format!("{h:02}:{m:02}");
    let dow = if days.len() == DAYS.len() { "*".to_string() } else { days.join(",") };
    let schedule = if at_time {
        json!({"cron": format!("{m} {h} * * {dow}"), "catchUp": true})
    } else if dow == "*" {
        json!({"every": "1h", "catchUp": true})
    } else {
        json!({"cron": format!("0 * * * {dow}"), "catchUp": true})
    };
    let title = match (at_time, dow == "*") {
        (false, true) => "Every hour".to_string(),
        (false, false) => format!("Every hour on {}", days.join(", ")),
        (true, true) => format!("Every day at {time}"),
        (true, false) => format!("At {time} on {}", days.join(", ")),
    };
    let node = d.nodes.iter_mut().find(|n| n.id == "hourly").ok_or("the schedule is missing")?;
    (node.config, node.title) = (schedule, title);
    let update = d.nodes.iter_mut().find(|n| n.id == "update").ok_or("the update is missing")?;
    update.config["after"] = json!(if at_time { AT_TIME_AFTER } else { HOURLY_AFTER });
    Ok(d)
}

/// The time and days from the schedule [`finish`] wrote; anything else leaves the defaults (and is customized).
fn read(d: &WorkflowDoc, out: &mut Map<String, Value>) {
    let Some(cron) = d.node("hourly").and_then(|n| n.config.get("cron")).and_then(Value::as_str) else { return };
    let f: Vec<&str> = cron.split_whitespace().collect();
    let [m, h, "*", "*", dow] = f[..] else { return };
    let days: Vec<&str> = if dow == "*" { DAYS.to_vec() } else { dow.split(',').collect() };
    out.insert("days".into(), json!(days));
    if let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) {
        out.insert("atTime".into(), json!(true));
        out.insert("time".into(), json!(format!("{h:02}:{m:02}")));
    }
}

/// The built-in as PDT ships it.
pub(super) fn pm_daily_checkpoint() -> WorkflowDoc {
    SETTINGS.shipped()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::nodes::schedule::ScheduleConfig;
    use pdt_core::workflow::validate::{Severity, validate};

    fn filled(given: Value) -> Result<WorkflowDoc, String> {
        SETTINGS.filled(given.as_object().unwrap())
    }

    fn schedule(d: &WorkflowDoc) -> ScheduleConfig {
        serde_json::from_value(d.node("hourly").unwrap().config.clone()).unwrap()
    }

    /// The default is as before Settings set it: each hour, a day after the last update, posted nowhere else.
    #[test]
    fn the_default_runs_each_hour() {
        let d = pm_daily_checkpoint();
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        assert!(!issues.iter().any(|i| i.code == "unreachable" || i.code == "open_port"), "{issues:?}");
        assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
        assert_eq!(d.node("hourly").unwrap().config, json!({"every": "1h", "catchUp": true}));
        assert_eq!(d.node("update").unwrap().config["after"], "1d");
        assert_eq!(d.node("post_on").unwrap().config["conditions"][0]["left"], "");
        assert!(!SETTINGS.customized(&d));
        assert_eq!(SETTINGS.values_of(&d), SETTINGS.defaults());
    }

    /// A time and days make a cron expression that reads back; a chat channel is posted to.
    #[test]
    fn inputs_fill_in_and_read_back() {
        let d =
            filled(json!({"atTime": true, "time": "8:30", "days": "fri, mon,tue,wed,thu", "chat": "team"})).unwrap();
        let s = schedule(&d);
        assert_eq!(s.cron.as_deref(), Some("30 8 * * mon,tue,wed,thu,fri"));
        s.schedule().unwrap();
        assert_eq!(d.node("update").unwrap().config["after"], "12h");
        assert_eq!(d.node("post").unwrap().config["chat"], "team");
        let v = SETTINGS.values_of(&d);
        assert_eq!(v["time"], "08:30");
        assert_eq!(v["days"], json!(["mon", "tue", "wed", "thu", "fri"]));
        assert_eq!((v["atTime"].clone(), v["chat"].clone()), (json!(true), json!("team")));
        assert!(!SETTINGS.customized(&d));

        let weekends = filled(json!({"days": ["sat", "sun"]})).unwrap();
        assert_eq!(schedule(&weekends).cron.as_deref(), Some("0 * * * sat,sun"));
        schedule(&weekends).schedule().unwrap();
        assert_eq!(weekends.node("update").unwrap().config["after"], "1d");
        let v = SETTINGS.values_of(&weekends);
        assert_eq!((v["atTime"].clone(), v["days"].clone()), (json!(false), json!(["sat", "sun"])));
        assert!(!SETTINGS.customized(&weekends));

        let daily = filled(json!({"atTime": true, "time": "17:05"})).unwrap();
        assert_eq!(schedule(&daily).cron.as_deref(), Some("5 17 * * *"));
        assert_eq!(SETTINGS.values_of(&daily)["days"], json!(DAYS));
    }

    /// An edit beyond the inputs, such as a schedule of one's own, is a customization; bad inputs are refused.
    #[test]
    fn edits_beyond_the_inputs_are_customized() {
        let mut d = pm_daily_checkpoint();
        d.nodes.iter_mut().find(|n| n.id == "hourly").unwrap().config = json!({"cron": "0 9 1 * *"});
        assert!(SETTINGS.customized(&d));
        let mut d = pm_daily_checkpoint();
        d.nodes.retain(|n| n.id != "post");
        assert!(SETTINGS.customized(&d));
        let mut d = pm_daily_checkpoint();
        d.name = "My checkpoint".into();
        assert!(!SETTINGS.customized(&d), "a new name is not a customization");
        for bad in [
            json!({"atTime": true, "time": "9am"}),
            json!({"time": "25:00"}),
            json!({"days": []}),
            json!({"days": "mon,someday"}),
        ] {
            assert!(filled(bad.clone()).is_err(), "{bad}");
        }
    }
}
