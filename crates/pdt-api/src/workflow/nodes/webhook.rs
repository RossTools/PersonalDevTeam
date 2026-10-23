//! The `webhook` trigger (Phase 7e spec §2.5): fires for `webhook.received` events of its hook (a `webhook`
//! connection), which `POST /api/hooks/<name>` records (pdt-api `hooks.rs`).

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, ports, schema_of};
use pdt_core::workflow::api::{LogEvent, Workflow};
use pdt_core::workflow::cond::Condition;
use pdt_core::workflow::doc::Node;
use pdt_core::workflow::events::{WEBHOOK_RECEIVED, event_type};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// The connection kind of a webhook (its secret is the shared secret).
pub const WEBHOOK_CONNECTION: &str = "webhook";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct WebhookConfig {
    /// The webhook connection's name: requests to `POST /api/hooks/<hook>` start this workflow.
    #[schemars(extend("x-connection-kind" = "webhook"))]
    pub hook: String,
    /// Only requests for which every condition holds (`{{trigger.body.action}}` …).
    #[serde(rename = "where")]
    #[ts(rename = "where")]
    pub conditions: Vec<Condition>,
    /// The run's idempotency key; default: the request.
    pub run_key: Option<String>,
}

pub struct WebhookTrigger;

impl NodeKind for WebhookTrigger {
    fn spec(&self) -> KindSpec {
        let t = event_type(WEBHOOK_RECEIVED).expect("webhook.received is an event type");
        KindSpec {
            id: "webhook".into(),
            version: 1,
            category: Category::Trigger,
            title: t.title.into(),
            description: "Starts when another machine posts to POST /api/hooks/<hook> on this host (locally or over \
                          Tailscale) with the hook's secret: a bearer token or a GitHub X-Hub-Signature-256."
                .into(),
            config_schema: schema_of::<WebhookConfig>(),
            output_schema: serde_json::to_value((t.schema)()).unwrap_or(Value::Null),
            ports: ports(&[("out", "Then")]),
            dynamic_ports: false,
            preset: None,
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: WebhookConfig = parse(config)?;
        if c.hook.trim().is_empty() {
            return Err("hook is empty: name a webhook connection".into());
        }
        Ok(())
    }

    fn event_type(&self) -> Option<&str> {
        Some(WEBHOOK_RECEIVED)
    }

    fn accepts(&self, config: &Value, _wf: &Workflow, _node: &Node, event: &LogEvent) -> bool {
        let hook = config.get("hook").and_then(|h| h.as_str());
        hook.is_some() && event.payload.get("hook").and_then(|h| h.as_str()) == hook
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move { Ok(Outcome::Next { port: "out".into(), output: cx.run.trigger.clone() }) })
    }
}
