//! Incoming webhooks: the `webhook` connection (a shared secret) and the Webhook trigger.

use super::Integration;
use crate::workflow::catalog::NodeKind;
use crate::workflow::nodes::webhook::{WEBHOOK_CONNECTION, WebhookTrigger};
use pdt_core::integration::ConnectionKind;

pub struct Webhook;

impl Integration for Webhook {
    fn id(&self) -> &'static str {
        "webhook"
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind {
            id: WEBHOOK_CONNECTION.into(),
            label: "Incoming webhook".into(),
            plural: "Incoming webhooks".into(),
            setup: "An address other machines post to, to start workflows with a Webhook trigger. Each has a secret \
                    the sender must send."
                .into(),
            description: "Lets another machine start workflows (the Webhook trigger).".into(),
            url: false,
            secret_label: "Shared secret".into(),
            secret_hint: "The sender sends it as Authorization: Bearer <secret>, or signs with it like GitHub \
                          (X-Hub-Signature-256)."
                .into(),
            generate: true,
            ..Default::default()
        }]
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(WebhookTrigger)]
    }
}
