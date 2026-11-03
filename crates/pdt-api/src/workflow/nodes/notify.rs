//! `notify`: a `workflow.notify` notification (PDT-108), shown as the user set that kind to show
//! (a toast by default; Settings → Notifications), or as far as its `show` says. It can post to a chat service too
//! (PDT-368: through the plugin that declares the connection's kind a `NotificationChannel`), and only while an Inbox
//! item is still open (an escalation). It logs no event, so a workflow can't notify itself in a loop.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::notification::{NewNotification, NotificationKind, NotificationTarget, Severity, Show};
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

/// Whether a link is a web address: only `http:` and `https:`, never `javascript:`, `data:` or `file:` (a link can come
/// from a trigger's payload, PDT-416).
fn is_web_url(l: &str) -> bool {
    let l = l.to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://")
}

/// Where a `link` goes: an Inbox item (`inbox:<id>`), a ticket by its key, a web address, else nowhere.
fn target(link: Option<&str>) -> NotificationTarget {
    match link.map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) if l.starts_with("inbox:") => match l.trim_start_matches("inbox:").trim() {
            "" => NotificationTarget::None,
            id => NotificationTarget::Inbox { id: id.to_string() },
        },
        Some(l) if pdt_core::ticket::parse_key(l).is_some_and(|k| k.key.is_some()) => {
            NotificationTarget::TicketKey { key: l.to_string() }
        }
        Some(l) if is_web_url(l) => NotificationTarget::Url { url: l.to_string() },
        _ => NotificationTarget::None,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct NotifyConfig {
    pub title: String,
    #[serde(default)]
    #[schemars(extend("x-format" = "multiline"))]
    pub body: String,
    #[serde(default)]
    pub level: NotifyLevel,
    /// A ticket key (`PDT-42`, opens the ticket), `inbox:<item id>` (opens the Inbox item) or an `http(s)` URL.
    #[serde(default)]
    pub link: Option<String>,
    /// How far it reaches you: `off` (nothing in PDT: only Discord), `dot`, `inbox`, `toast` or `desktop`. Default: the
    /// level Settings → Notifications sets for Workflow notification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    #[ts(type = "\"off\" | \"dot\" | \"inbox\" | \"toast\" | \"desktop\" | null", optional)]
    pub show: Option<Show>,
    /// A chat connection to post it to as well: one of a kind a plugin posts notifications through (a Discord
    /// webhook); empty: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub chat: Option<String>,
    /// An Inbox item's id: only while it is still open (else it does nothing and goes on, `skipped` in its output).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub while_open: Option<String>,
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
            let chat = c.chat.as_deref().map(str::trim).filter(|d| !d.is_empty());
            if cx.dry_run {
                let to = chat.map(|d| format!(" (and post via {d})")).unwrap_or_default();
                return Ok(Outcome::would("out", format!("notify: {}{to}", c.title)));
            }
            if let Some(item) = c.while_open.as_deref().map(str::trim).filter(|i| !i.is_empty())
                && !crate::inbox::is_open(cx.st, item).await.map_err(|e| e.message().to_string())?
            {
                return Ok(Outcome::next("out", json!({"skipped": "the Inbox item is no longer open"})));
            }
            // One per step: a step run again after a restart updates its own notification.
            let mut n = NewNotification::new(
                NotificationKind::WorkflowNotify,
                format!("notify:{}", cx.step.id),
                c.title.clone(),
                target(c.link.as_deref()),
            );
            let from = format!("From {}", cx.workflow.name);
            n.body =
                [c.body.trim(), from.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            n.severity = c.level.severity();
            n.project_id = cx.run.project_id.clone();
            n.ticket_id = cx.run.ticket_id.clone();
            n.run_id = Some(cx.run.id.clone());
            // Nothing is made below the Inbox (the user turned the kind down, or `show` says so).
            let id = crate::notifications::notify_at(cx.st, n, c.show).await.map(|n| n.id);
            let mut out = json!({"notificationId": id});
            if let Some(connection) = chat {
                out["chat"] = post(cx, connection, &c).await?;
            }
            Ok(Outcome::next("out", out))
        })
    }
}

/// Posts the notification to a chat connection, through the node kind of the plugin that posts to its kind (its pieces
/// saved as it goes, so a step run again posts only what is left).
async fn post(cx: &StepCx<'_>, connection: &str, c: &NotifyConfig) -> Result<Value, String> {
    let conn = cx
        .st
        .store
        .connection_by_name(connection)
        .await
        .map_err(crate::workflow::err)?
        .ok_or_else(|| format!("no connection is called {connection}"))?;
    let reg = cx.st.registry();
    let channel =
        reg.notification_channels().into_iter().find(|ch| ch.connection_kind == conn.kind).ok_or_else(|| {
            format!("{} is a {} connection: no plugin that is on posts notifications through it", conn.name, conn.kind)
        })?;
    let kind = reg.catalog().shared(channel.node_kind).ok_or_else(|| format!("{} is off", channel.label))?;
    let link = c.link.as_deref().map(str::trim).filter(|l| is_web_url(l)).map(|l| format!("\n{l}"));
    let content = format!("**{}**\n{}{}", c.title.trim(), c.body.trim(), link.unwrap_or_default());
    match kind.run(cx, json!({"connection": conn.name, "content": content.trim_end()})).await? {
        Outcome::Next { output, .. } => Ok(output),
        _ => Ok(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_open_tickets_inbox_items_and_web_addresses() {
        assert!(matches!(target(Some("PDT-42")), NotificationTarget::TicketKey { key } if key == "PDT-42"));
        assert!(matches!(target(Some("inbox:abc")), NotificationTarget::Inbox { id } if id == "abc"));
        assert!(
            matches!(target(Some(" https://example.com/x ")), NotificationTarget::Url { url } if url == "https://example.com/x")
        );
        assert!(matches!(target(Some("HTTP://example.com")), NotificationTarget::Url { .. }));
        assert!(matches!(target(None), NotificationTarget::None));
        assert!(matches!(target(Some("  ")), NotificationTarget::None));
    }

    #[test]
    fn links_in_other_schemes_go_nowhere() {
        for l in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,x",
            "file:///etc/passwd",
            "httpx://a",
            "//evil.example",
            "evil.example",
        ] {
            assert!(matches!(target(Some(l)), NotificationTarget::None), "{l}");
        }
    }
}
