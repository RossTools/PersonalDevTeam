//! Jira Cloud (PDT-85): the `jira` connection, its issue events and the Jira actions.

use super::Integration;
use crate::workflow::catalog::NodeKind;
use crate::workflow::nodes::jira::{JiraComment, JiraCreate, JiraImport, JiraTransition};
use pdt_core::integration::{ConfigField, ConnectionKind, ProjectSettingsSchema};
use pdt_core::jira::{DEFAULT_DONE, JIRA};
use pdt_core::workflow::events::EventType;

pub struct Jira;

impl Integration for Jira {
    fn id(&self) -> &'static str {
        "jira"
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind {
            id: JIRA.into(),
            label: "Jira Cloud".into(),
            plural: "Jira Cloud sites".into(),
            setup: "A Jira Cloud site a project links to in its settings: issues assigned to you become tickets, and \
                    Done moves the issue."
                .into(),
            description: "A Jira Cloud site (projects link to it; Jira actions).".into(),
            url: true,
            url_placeholder: Some("https://your-site.atlassian.net".into()),
            url_hint: Some("Your Jira Cloud site.".into()),
            fields: vec![ConfigField {
                placeholder: Some("you@work.com".into()),
                hint: Some("The Atlassian account’s email; PDT signs in as it.".into()),
                ..ConfigField::new("email", "Email")
            }],
            secret_label: "API token".into(),
            secret_hint: "At id.atlassian.com → Security → Create and manage API tokens → Create API token.".into(),
            ..Default::default()
        }]
    }

    fn project_settings(&self) -> Option<ProjectSettingsSchema> {
        Some(ProjectSettingsSchema {
            integration: "jira".into(),
            label: "Jira".into(),
            description:
                "Issues assigned to you in this scope become Draft tickets (checked every minute), and moving \
                          a linked ticket to Done moves its issue."
                    .into(),
            connection_kind: JIRA.into(),
            fields: vec![
                ConfigField {
                    placeholder: Some("project = ABC".into()),
                    hint: Some("JQL for this project's issues. Empty: every issue the account can see.".into()),
                    optional: true,
                    ..ConfigField::new("scope", "Jira issues")
                },
                ConfigField {
                    hint: Some("The transition (or status) a ticket moved to Done moves its issue to.".into()),
                    default: Some(DEFAULT_DONE.into()),
                    ..ConfigField::new("done", "Done in Jira")
                },
            ],
        })
    }

    fn event_types(&self) -> &'static [EventType] {
        pdt_core::jira::EVENT_TYPES
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(JiraImport), Box::new(JiraCreate), Box::new(JiraTransition), Box::new(JiraComment)]
    }
}
