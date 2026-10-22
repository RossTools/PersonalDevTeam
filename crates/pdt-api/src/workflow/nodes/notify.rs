//! `notify` (Phase 7e spec §3.4): a notification in PDT. It records a `pdt.notified` event, which the web app shows as
//! a toast (and a system notification while the page is hidden, if the user allowed it). Not a trigger, so a workflow
//! can't notify itself in a loop.

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::api::NewLogEvent;
use pdt_core::workflow::events::{Notified, PDT_NOTIFIED};
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
    fn as_str(self) -> &'static str {
        match self {
            NotifyLevel::Info => "info",
            NotifyLevel::Warning => "warning",
            NotifyLevel::Error => "error",
        }
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
            let payload = Notified {
                title: c.title,
                body: c.body,
                level: c.level.as_str().into(),
                link: c.link.filter(|l| !l.trim().is_empty()),
                workflow_id: cx.workflow.id.clone(),
                workflow_name: cx.workflow.name.clone(),
                run_id: cx.run.id.clone(),
            };
            let mut e = NewLogEvent::new(PDT_NOTIFIED, format!("notify:{}", cx.step.id), "workflow", payload);
            e.project_id = cx.run.project_id.clone();
            e.ticket_id = cx.run.ticket_id.clone();
            // Once per step: a step run again after a restart finds its own notification.
            let id = match cx.st.store.emit_event(&e, now_ms()).await.map_err(|e| e.to_string())? {
                Some(ev) => ev.id,
                None => String::new(),
            };
            Ok(Outcome::next("out", json!({"eventId": id})))
        })
    }
}
