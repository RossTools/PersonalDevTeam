//! `notify` (Phase 7e spec §3.4): a `workflow.notify` notification (PDT-108), shown as the user set that kind to show
//! (a toast by default; Settings → Notifications). It logs no event, so a workflow can't notify itself in a loop.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::notification::{NewNotification, NotificationKind, NotificationTarget, Severity};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NotifyLevel {
    #[default]
    Info,
    Warning,
    Error,
}

impl NotifyLevel {
    fn severity(self) -> Severity {
        match self {
            NotifyLevel::Info => Severity::Info,
            NotifyLevel::Warning => Severity::Warning,
            NotifyLevel::Error => Severity::Error,
        }
    }
}

/// Where a `link` goes: a ticket by its key, else a URL.
fn target(link: Option<&str>) -> NotificationTarget {
    match link.map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) if pdt_core::ticket::parse_key(l).is_some_and(|k| k.key.is_some()) => {
            NotificationTarget::TicketKey { key: l.to_string() }
        }
        Some(l) => NotificationTarget::Url { url: l.to_string() },
        None => NotificationTarget::None,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct NotifyConfig {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub level: NotifyLevel,
    /// A ticket key (`PDT-42`, opens the ticket) or a URL.
    #[serde(default)]
    pub link: Option<String>,
}

pub struct Notify;

impl NodeKind for Notify {
    fn spec(&self) -> KindSpec {
        spec::<NotifyConfig, Value>(
            "notify",
            1,
            Category::Action,
            "Notify me",
            "Shows a notification in PDT (and on the desktop while PDT's page is hidden, if allowed in Settings).",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: NotifyConfig = parse(config)?;
        if c.title.trim().is_empty() {
            return Err("title is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: NotifyConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next("out", json!({"would": format!("notify: {}", c.title)})));
            }
            // One per step: a step run again after a restart updates its own notification.
            let mut n = NewNotification::new(
                NotificationKind::WorkflowNotify,
                format!("notify:{}", cx.step.id),
                c.title,
                target(c.link.as_deref()),
            );
            let from = format!("From {}", cx.workflow.name);
            n.body =
                [c.body.trim(), from.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            n.severity = c.level.severity();
            n.project_id = cx.run.project_id.clone();
            n.ticket_id = cx.run.ticket_id.clone();
            n.run_id = Some(cx.run.id.clone());
            // Nothing is made when the user turned the kind down below the Inbox.
            let id = crate::notifications::notify(cx.st, n).await.map(|n| n.id);
            Ok(Outcome::next("out", json!({"notificationId": id})))
        })
    }
}
