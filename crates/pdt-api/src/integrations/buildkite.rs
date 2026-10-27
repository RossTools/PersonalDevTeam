//! Buildkite (CI of tickets' branches): the `buildkite` connection and the Buildkite actions.

use super::Integration;
use crate::workflow::catalog::NodeKind;
use crate::workflow::nodes::buildkite::{Retry, TriggerBuild};
use pdt_core::buildkite::BUILDKITE;
use pdt_core::integration::{ConfigField, ConnectionKind};

pub struct Buildkite;

impl Integration for Buildkite {
    fn id(&self) -> &'static str {
        "buildkite"
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind {
            id: BUILDKITE.into(),
            label: "Buildkite".into(),
            plural: "Buildkite organizations".into(),
            setup: "A Buildkite organization whose pipelines build a project’s branches (named in the project’s \
                    settings)."
                .into(),
            description: "A Buildkite organization (CI of tickets’ branches; Buildkite actions).".into(),
            url: true,
            url_optional: true,
            url_placeholder: Some("https://api.buildkite.com".into()),
            url_hint: Some("Leave empty for Buildkite’s own API.".into()),
            fields: vec![ConfigField {
                placeholder: Some("acme".into()),
                hint: Some("Its slug, as in buildkite.com/<organization>.".into()),
                ..ConfigField::new("org", "Organization")
            }],
            secret_label: "API token".into(),
            secret_hint: "In Buildkite: Personal Settings → API Access Tokens → New API Access Token, with Read \
                          Builds (and Write Builds to retry or start builds)."
                .into(),
            ..Default::default()
        }]
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(Retry), Box::new(TriggerBuild)]
    }
}
