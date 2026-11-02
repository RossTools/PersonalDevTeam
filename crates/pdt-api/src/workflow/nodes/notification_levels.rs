//! `notification.levels` (PDT-368): how far each kind of notification reaches the user, and quiet hours. It is a
//! trigger that starts no runs: the Notifier reads it from the enabled "Notification rules" workflow each time it makes
//! a notification ([`crate::notifications`]), so needs-you notifications never wait on a run.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::notification::{NotificationKind, NotificationSettings, QuietHours, Show};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

pub const KIND: &str = "notification.levels";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct QuietHoursConfig {
    #[serde(default)]
    pub on: bool,
    /// `HH:MM`, local time.
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct LevelsConfig {
    /// Only these projects' notifications; empty: every project's (and the host's own).
    pub projects: Vec<String>,
    /// Kinds whose level differs from PDT's default (`inbox.question`: `off` | `dot` | `inbox` | `toast` | `desktop`).
    #[ts(type = "Record<string, string>")]
    pub levels: Map<String, Value>,
    /// No toast or desktop notification between two times; one summary when they end.
    #[ts(optional)]
    pub quiet_hours: Option<QuietHoursConfig>,
}

/// The rules a levels node sets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Levels {
    pub projects: Vec<String>,
    pub levels: NotificationSettings,
    pub quiet: Option<QuietHours>,
}

impl LevelsConfig {
    /// Checked: every kind known, every level a level the kind may take, the quiet hours times of day.
    pub fn levels(&self) -> Result<Levels, String> {
        let mut levels = NotificationSettings::default();
        for (k, v) in &self.levels {
            let kind = NotificationKind::parse(k).ok_or_else(|| format!("{k} is not a kind of notification"))?;
            let show = v.as_str().and_then(Show::parse).ok_or_else(|| format!("{k}: {v} is not a level"))?;
            levels.set(kind, Some(show))?;
        }
        let quiet = match &self.quiet_hours {
            Some(q) => {
                let hours = QuietHours::parse(&q.from, &q.to)?;
                q.on.then_some(hours)
            }
            None => None,
        };
        Ok(Levels { projects: self.projects.clone(), levels, quiet })
    }
}

pub struct NotificationLevels;

impl NodeKind for NotificationLevels {
    fn spec(&self) -> KindSpec {
        spec::<LevelsConfig, Value>(
            KIND,
            1,
            Category::Trigger,
            "Show levels",
            "How far each kind of notification reaches you (off, a dot, the Inbox, a toast, a desktop notification), \
             and quiet hours. PDT reads it from the enabled Notification rules workflow as it makes each notification; \
             it starts no runs. Kinds it doesn't list take PDT's default. Another one with `projects` sets those \
             projects' levels.",
            &[],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<LevelsConfig>(config)?.levels().map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move { Ok(Outcome::next("out", cx.run.trigger.clone())) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn levels_are_checked() {
        let c: LevelsConfig = serde_json::from_value(json!({
            "levels": {"pr.merged": "toast"}, "quietHours": {"on": true, "from": "22:00", "to": "07:00"}}))
        .unwrap();
        let l = c.levels().unwrap();
        assert_eq!(l.levels.show(NotificationKind::PrMerged), Show::Toast);
        assert_eq!(l.quiet, Some(QuietHours { from: 22 * 60, to: 7 * 60 }));
        let off: LevelsConfig =
            serde_json::from_value(json!({"quietHours": {"on": false, "from": "22:00", "to": "07:00"}})).unwrap();
        assert_eq!(off.levels().unwrap().quiet, None);
        for bad in [
            json!({"levels": {"nope": "toast"}}),
            json!({"levels": {"pr.merged": "loud"}}),
            json!({"levels": {"inbox.question": "dot"}}),
            json!({"quietHours": {"on": false, "from": "late", "to": "07:00"}}),
        ] {
            assert!(NotificationLevels.check(&bad).is_err(), "{bad}");
        }
    }
}
