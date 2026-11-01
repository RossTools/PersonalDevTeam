//! `pm.update` and `pm.checkpoint` (PDT-372): the PM's daily update and journal checkpoint for its open
//! conversations, which the PM daily checkpoint built-in runs on a schedule. Journaling at a reset or when a
//! conversation ends stays PDT's own (`crate::pm`).

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::schedule::parse_every;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const PM_UPDATE: &str = "pm.update";
pub const PM_CHECKPOINT: &str = "pm.checkpoint";

fn has_ref(s: &str) -> bool {
    s.contains("{{")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct PmUpdateConfig {
    /// How long after its last update a conversation gets the next: `"1d"`, `"12h"`.
    pub after: String,
    /// Only conversations you wrote to since their last update.
    pub only_in_use: bool,
}

impl Default for PmUpdateConfig {
    fn default() -> Self {
        Self { after: "1d".into(), only_in_use: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PmUpdateSent {
    pub session_id: String,
    /// The conversation's title.
    pub title: String,
    /// The update, as Markdown.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PmUpdateOutput {
    /// The conversations that got an update.
    pub updated: Vec<PmUpdateSent>,
    /// The conversations that were due but had nothing new: brought up to date, for `pm.checkpoint`.
    pub nothing_new: Vec<String>,
    /// The newest conversation's update (empty when none was sent), to post elsewhere.
    pub text: String,
    /// Nothing was sent.
    pub quiet: bool,
}

/// `after` in ms.
fn after_ms(after: &str) -> Result<i64, String> {
    let ms = parse_every(after).map_err(|e| format!("after: {e}"))?;
    i64::try_from(ms).map_err(|_| "after is too long".into())
}

pub struct PmUpdate;

impl NodeKind for PmUpdate {
    fn spec(&self) -> KindSpec {
        spec::<PmUpdateConfig, PmUpdateOutput>(
            PM_UPDATE,
            1,
            Category::Action,
            "PM daily update",
            "Sends each open PM conversation that is due one what happened since its last update (tickets, the Inbox, \
             what other PM conversations left open) and asks it for a journal checkpoint. A conversation is due once \
             `after` has passed since its last update and, with `onlyInUse`, you wrote to it since. Updates wait for \
             a running turn to end.",
            &[("updated", "Updates sent"), ("none", "None sent")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: PmUpdateConfig = parse(config)?;
        if !has_ref(&c.after) {
            after_ms(&c.after)?;
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PmUpdateConfig = parse(&input)?;
            let after = after_ms(&c.after)?;
            let done = crate::pm::send_updates(cx.st, after, c.only_in_use, cx.dry_run)
                .await
                .map_err(|e| e.message().to_string())?;
            let text = done.updated.first().map(|u| u.text.clone()).unwrap_or_default();
            let quiet = done.updated.is_empty();
            let updated = done
                .updated
                .into_iter()
                .map(|u| PmUpdateSent { session_id: u.session_id, title: u.title, text: u.text })
                .collect();
            let out = PmUpdateOutput { updated, nothing_new: done.nothing_new, text, quiet };
            Ok(Outcome::next(if quiet { "none" } else { "updated" }, out))
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct PmCheckpointConfig {
    /// The conversations to ask (session ids, or a reference such as `{{steps.update.nothingNew}}`); empty: every
    /// open PM conversation.
    #[schemars(with = "Vec<String>")]
    #[ts(type = "Array<string> | string | null")]
    pub conversations: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PmCheckpointOutput {
    /// The conversations asked.
    pub asked: Vec<String>,
    /// None was asked.
    pub quiet: bool,
}

/// The session ids a resolved `conversations` value names; None for every open conversation.
fn conversations(v: &Value) -> Result<Option<Vec<String>>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::String(s) => Ok(Some(vec![s.trim().to_string()])),
        Value::Array(_) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("conversations: {e}")),
        other => Err(format!("conversations is not a list of session ids: {other}")),
    }
}

pub struct PmCheckpoint;

impl NodeKind for PmCheckpoint {
    fn spec(&self) -> KindSpec {
        spec::<PmCheckpointConfig, PmCheckpointOutput>(
            PM_CHECKPOINT,
            1,
            Category::Action,
            "PM journal checkpoint",
            "Asks open PM conversations to write their journal entry (what was decided, what is still open), each \
             one you wrote to since its last entry. `conversations` picks some; empty, every open one. The request \
             waits for a running turn to end, and the conversation carries on.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: PmCheckpointConfig = parse(config)?;
        if !c.conversations.as_str().is_some_and(has_ref) {
            conversations(&c.conversations)?;
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PmCheckpointConfig = parse(&input)?;
            let only = conversations(&c.conversations)?;
            // A reference that named none (no conversation had a quiet day) asks none.
            if only.as_ref().is_some_and(|ids| ids.is_empty()) {
                return Ok(Outcome::next("out", PmCheckpointOutput { asked: vec![], quiet: true }));
            }
            let asked = crate::pm::ask_checkpoints(cx.st, only.as_deref(), cx.dry_run)
                .await
                .map_err(|e| e.message().to_string())?;
            let quiet = asked.is_empty();
            Ok(Outcome::next("out", PmCheckpointOutput { asked, quiet }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn configs_are_checked() {
        let update = PmUpdate;
        assert!(update.check(&json!({})).is_ok(), "a day, conversations in use");
        assert!(update.check(&json!({"after": "12h", "onlyInUse": false})).is_ok());
        assert!(update.check(&json!({"after": "{{state.after}}"})).is_ok());
        assert!(update.check(&json!({"after": "daily"})).unwrap_err().contains("after"));
        assert!(update.check(&json!({"every": "1d"})).is_err());
        let checkpoint = PmCheckpoint;
        assert!(checkpoint.check(&json!({})).is_ok());
        assert!(checkpoint.check(&json!({"conversations": "{{steps.update.nothingNew}}"})).is_ok());
        assert!(checkpoint.check(&json!({"conversations": ["a", "b"]})).is_ok());
        assert!(checkpoint.check(&json!({"conversations": 3})).is_err());
    }

    #[test]
    fn conversations_name_some_or_every_one() {
        assert_eq!(conversations(&Value::Null).unwrap(), None);
        assert_eq!(conversations(&json!("")).unwrap(), None);
        assert_eq!(conversations(&json!([])).unwrap(), Some(vec![]), "a reference that named none");
        assert_eq!(conversations(&json!(["a"])).unwrap(), Some(vec!["a".to_string()]));
        assert_eq!(conversations(&json!("a")).unwrap(), Some(vec!["a".to_string()]));
    }
}
