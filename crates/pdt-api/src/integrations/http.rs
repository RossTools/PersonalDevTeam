//! HTTP services: the `http` connection (an address and its token) and the HTTP request action.

use super::Integration;
use crate::workflow::catalog::NodeKind;
use pdt_core::integration::ConnectionKind;
use pdt_core::integration::{Icon, PluginCategory};

pub struct Http;

impl Integration for Http {
    fn id(&self) -> &'static str {
        "http"
    }

    fn label(&self) -> &'static str {
        "HTTP services"
    }

    fn description(&self) -> &'static str {
        "Any HTTP API with a token, for HTTP request steps."
    }

    fn category(&self) -> Option<PluginCategory> {
        Some(PluginCategory::General)
    }

    fn icon(&self) -> Icon {
        Icon { letters: "{}", color: "#6b7080" }
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind {
            id: "http".into(),
            label: "HTTP service".into(),
            plural: "HTTP services".into(),
            setup: "A service HTTP request steps call by name: its address, and a token PDT sends only to it.".into(),
            description: "A service the HTTP request action calls: its address and token.".into(),
            url: true,
            secret_label: "Token or password".into(),
            secret_hint: "Sent only to this address, the way “Send it as” says.".into(),
            auth: true,
            ..Default::default()
        }]
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(crate::workflow::nodes::http::Http)]
    }
}
