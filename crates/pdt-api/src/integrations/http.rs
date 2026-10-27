//! HTTP services: the `http` connection (an address and its token) and the HTTP request action.

use super::Integration;
use crate::workflow::catalog::NodeKind;
use pdt_core::integration::ConnectionKind;

pub struct Http;

impl Integration for Http {
    fn id(&self) -> &'static str {
        "http"
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
