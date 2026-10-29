//! Buildkite (PDT-85): CI of tickets' branches (a 60 s check of each active attempt's builds, logged as `ci.*` events
//! and shown with the PR's CI), the `buildkite` connection a project names its pipelines through, and the Buildkite
//! actions. pdtd's `buildkite` feature.

mod core;
mod nodes;
mod sensor;

use crate::core::{BUILDKITE, BuildkiteSettings};
use crate::nodes::{Retry, TriggerBuild};
use pdt_api::sdk::{AppState, BoxFut, Integration, NodeKind, Poller};
use pdt_core::integration::{ConfigField, ConnectionKind, FieldType, IntegrationSettings, ProjectSettingsSchema};

pub use crate::nodes::{BuildOutput, RetryConfig, TriggerBuildConfig};
pub use crate::sensor::refresh_buildkite;

pub struct Buildkite;

impl Integration for Buildkite {
    fn id(&self) -> &'static str {
        "buildkite"
    }

    fn pollers(&self) -> Vec<Poller> {
        fn run(st: AppState) -> BoxFut<'static, ()> {
            Box::pin(async move { refresh_buildkite(&st).await })
        }
        vec![Poller { name: "the Buildkite check", run }]
    }

    fn project_ci(&self, settings: &pdt_store::ProjectSettings) -> Option<&'static str> {
        settings.integration::<BuildkiteSettings>(BUILDKITE).map(|_| "Buildkite")
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

    fn project_settings(&self) -> Option<ProjectSettingsSchema> {
        Some(ProjectSettingsSchema {
            integration: "buildkite".into(),
            label: "Buildkite".into(),
            description: "Builds of the tickets' branches show as their CI, and failures are told to the agent like \
                          GitHub's."
                .into(),
            connection_kind: BUILDKITE.into(),
            fields: vec![ConfigField {
                placeholder: Some("my-app\nmy-app-e2e".into()),
                hint: Some("The pipelines (slugs) that build this project's branches, one per line.".into()),
                r#type: Some(FieldType::Lines),
                optional: true,
                ..ConfigField::new("pipelines", "Pipelines")
            }],
        })
    }

    fn check_project_settings(&self, settings: &IntegrationSettings) -> Result<(), String> {
        let pipelines = settings.get("pipelines").and_then(|p| p.as_array()).into_iter().flatten();
        match pipelines
            .filter_map(|p| p.as_str())
            .find(|p| !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        {
            Some(p) => Err(format!("{p:?} is not a pipeline slug")),
            None => Ok(()),
        }
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(Retry), Box::new(TriggerBuild)]
    }
}
