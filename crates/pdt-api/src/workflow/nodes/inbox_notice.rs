//! `inbox.notice` (PDT-139): a notice nobody could be told goes to the user's Inbox, once per attempt and notice key.
//! The item closes by itself once its cause is gone (`crate::inbox::settle_notices`).

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::inbox::{InboxGroup, InboxItem, InboxKind, InboxStatus};
use pdt_core::pr::NoticeKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const KIND: &str = "inbox.notice";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct InboxNoticeConfig {
    /// The attempt (`{{trigger.attemptId}}`).
    pub attempt: String,
    /// What happened (`{{trigger.notice.text}}`).
    pub text: String,
    /// One item per attempt and key (`{{trigger.notice.key}}`).
    pub key: String,
    /// The item's title; by default it says what the trigger's notice is about.
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
}

/// The default title: the notice, and that nobody heard it.
fn title_of(kind: Option<NoticeKind>, pr: Option<u64>) -> String {
    let pr = pr.map_or("the pull request".to_string(), |n| format!("PR #{n}"));
    let what = match kind {
        Some(NoticeKind::CiFailed) => format!("CI failed on {pr}"),
        Some(NoticeKind::Conflict) => format!("{pr} conflicts with its base"),
        Some(NoticeKind::QueueLeft) => format!("{pr} left the merge queue"),
        Some(NoticeKind::Behind) => format!("{pr} is behind its base"),
        _ => format!("News on {pr}"),
    };
    let mut chars = what.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    format!("{first}{} and no agent session is open", chars.as_str())
}

pub struct InboxNotice;

impl NodeKind for InboxNotice {
    fn spec(&self) -> KindSpec {
        spec::<InboxNoticeConfig, Value>(
            KIND,
            1,
            Category::Action,
            "Put it in my Inbox",
            "Puts a notice in your Inbox (Unheard notices), linked to the attempt's ticket: once per attempt and key \
             (`skipped` after that, also once it closed). It closes by itself when its cause is gone (the pull \
             request mergeable again, CI passed, back in the queue, merged or closed, or the attempt ended).",
            &[("added", "In the Inbox"), ("skipped", "Already there")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: InboxNoticeConfig = parse(config)?;
        if c.text.trim().is_empty() {
            return Err("text is empty".into());
        }
        if c.key.trim().is_empty() {
            return Err("key is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: InboxNoticeConfig = parse(&input)?;
            let notice = cx.ctx.pointer("/trigger/notice").cloned().unwrap_or(Value::Null);
            let kind: Option<NoticeKind> = notice.get("kind").cloned().and_then(|k| serde_json::from_value(k).ok());
            let pr = notice.get("pr").and_then(Value::as_u64).or_else(|| cx.ctx.pointer("/trigger/pr")?.as_u64());
            let title = c.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| title_of(kind, pr));
            if cx.dry_run {
                return Ok(Outcome::next("added", json!({"would": format!("put in the Inbox: {title}")})));
            }
            let store = &cx.st.store;
            let info = store.get_attempt(&c.attempt).await.map_err(|e| e.to_string())?.ok_or("no such attempt")?;
            let item = InboxItem {
                id: uuid::Uuid::now_v7().to_string(),
                kind: InboxKind::Notice,
                group: InboxGroup::Notice,
                status: InboxStatus::Open,
                project_id: Some(info.project_id.clone()),
                ticket_id: Some(info.ticket_id.clone()),
                ticket_key: None,
                ticket_title: None,
                attempt_id: Some(info.attempt.id.clone()),
                session_id: None,
                run_id: Some(cx.run.id.clone()),
                // Not the step's: the item outlives the run.
                step_id: None,
                persona_id: None,
                persona_name: None,
                title,
                body: json!({
                    "notice": kind, "key": c.key, "text": c.text, "pr": pr, "url": notice.get("url"),
                    "workflow": cx.workflow.name,
                }),
                choices: vec![],
                resolution: Value::Null,
                created_at: now_ms(),
                resolved_at: None,
            };
            let key = format!("notice:{}:{}", info.attempt.id, c.key);
            match store.add_inbox_item_once(&item, &key).await.map_err(|e| e.to_string())? {
                Some(i) => {
                    crate::notifications::kick(cx.st);
                    Ok(Outcome::next("added", json!({"itemId": i.id})))
                }
                None => Ok(Outcome::next("skipped", json!({"quiet": true, "reason": "already in the Inbox"}))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_say_what_nobody_heard() {
        assert_eq!(
            title_of(Some(NoticeKind::Conflict), Some(12)),
            "PR #12 conflicts with its base and no agent session is open"
        );
        assert_eq!(title_of(None, None), "News on the pull request and no agent session is open");
        assert_eq!(
            title_of(Some(NoticeKind::Behind), None),
            "The pull request is behind its base and no agent session is open"
        );
        assert_eq!(
            title_of(Some(NoticeKind::CiFailed), None),
            "CI failed on the pull request and no agent session is open"
        );
    }
}
