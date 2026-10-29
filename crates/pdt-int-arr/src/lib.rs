//! Radarr and Sonarr: their connections and the Search, Add and queue actions. pdtd's `arr` feature.

mod nodes;

use pdt_api::sdk::{Integration, NodeKind};
use pdt_core::integration::{ConfigField, ConnectionKind};

pub use crate::nodes::*;

pub struct Arr;

/// The settings both kinds have; `folder` is the root folder's example.
fn fields(folder: &str) -> Vec<ConfigField> {
    vec![
        ConfigField {
            hint: Some(
                "Used when a step adds without naming one: a profile’s name or id. Empty: the first profile.".into(),
            ),
            placeholder: Some("HD-1080p".into()),
            optional: true,
            ..ConfigField::new("qualityProfile", "Quality profile")
        },
        ConfigField {
            hint: Some("Where adds go unless a step says. Empty: the first root folder.".into()),
            placeholder: Some(folder.into()),
            optional: true,
            ..ConfigField::new("rootFolder", "Root folder")
        },
    ]
}

impl Integration for Arr {
    fn id(&self) -> &'static str {
        "arr"
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![
            ConnectionKind {
                id: "radarr".into(),
                label: "Radarr".into(),
                plural: "Radarr".into(),
                setup: "A Radarr that Search, Add and queue steps use for films.".into(),
                description: "Films: search, add, what’s downloading (the Radarr actions).".into(),
                url: true,
                url_placeholder: Some("http://192.168.1.10:7878".into()),
                secret_label: "API key".into(),
                secret_hint: "In Radarr: Settings → General → Security → API Key.".into(),
                fields: fields("/movies"),
                ..Default::default()
            },
            ConnectionKind {
                id: "sonarr".into(),
                label: "Sonarr".into(),
                plural: "Sonarr".into(),
                setup: "A Sonarr that Search, Add and queue steps use for series.".into(),
                description: "Series: search, add, what’s downloading (the Sonarr actions).".into(),
                url: true,
                url_placeholder: Some("http://192.168.1.10:8989".into()),
                secret_label: "API key".into(),
                secret_hint: "In Sonarr: Settings → General → Security → API Key.".into(),
                fields: fields("/tv"),
                ..Default::default()
            },
        ]
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        crate::nodes::kinds()
    }
}
