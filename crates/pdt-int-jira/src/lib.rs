//! Jira Cloud (PDT-85): the `jira` connection a project links to, its issue events (a 60 s check of linked projects'
//! issues), importing issues as tickets (the built-in for assigned ones, and any one by key), Done moving the issue,
//! and the Jira actions. pdtd's `jira` feature.

mod core;
mod nodes;
mod sensor;

use crate::core::{DEFAULT_DONE, JIRA};
use pdt_api::sdk::{AppState, BoxFut, Builtin, Integration, NodeKind, Poller};
use pdt_core::integration::{ConfigField, ConnectionKind, ProjectSettingsSchema};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::events::EventType;
use serde_json::json;

pub use crate::core::{
    JiraIssueCommented, JiraIssueCreated, JiraIssueRef, JiraIssueTransitioned, JiraIssueUpdated, JiraSettings,
    parse_time,
};
pub use crate::nodes::*;
pub use crate::sensor::{JiraImportInput, JiraImportResult, refresh_jira};

pub const JIRA_IMPORT: &str = "jira-import";
pub const JIRA_DONE: &str = "jira-done";

pub struct Jira;

impl Integration for Jira {
    fn id(&self) -> &'static str {
        "jira"
    }

    fn pollers(&self) -> Vec<Poller> {
        fn run(st: AppState) -> BoxFut<'static, ()> {
            Box::pin(async move { refresh_jira(&st).await })
        }
        vec![Poller { name: "the Jira check", run }]
    }

    fn routes(&self) -> Option<axum::Router<AppState>> {
        Some(axum::Router::new().route("/projects/{id}/jira/import", axum::routing::post(sensor::import_route)))
    }

    fn builtin_workflows(&self) -> Vec<Builtin> {
        vec![
            Builtin { id: JIRA_IMPORT, enabled: true, doc: jira_import },
            Builtin { id: JIRA_DONE, enabled: true, doc: jira_done },
        ]
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
        crate::core::EVENT_TYPES
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(JiraImport), Box::new(JiraCreate), Box::new(JiraTransition), Box::new(JiraComment)]
    }
}

fn doc(v: serde_json::Value) -> WorkflowDoc {
    serde_json::from_value(v).expect("a built-in workflow is a valid document")
}

/// The built-in "Import assigned Jira issues": an issue assigned to the user with no ticket yet becomes one.
fn jira_import() -> WorkflowDoc {
    let mine = json!([
        {"left": "{{trigger.assignedToMe}}", "op": "truthy"},
        {"left": "{{trigger.ticketId}}", "op": "missing"},
    ]);
    doc(json!({
        "name": "Import assigned Jira issues",
        "description": "An issue in a linked project's Jira scope that is assigned to you and has no ticket yet becomes \
                        a Draft ticket, linked to it. Does nothing until a project links Jira in its settings.",
        "nodes": [
            {"id": "created", "kind": "jira.issue.created", "version": 1, "position": {"x": 0, "y": 0},
             "config": {"where": mine}},
            {"id": "updated", "kind": "jira.issue.updated", "version": 1, "position": {"x": 0, "y": 140},
             "config": {"where": mine}},
            {"id": "import", "kind": "jira.import", "version": 1, "position": {"x": 320, "y": 70}, "config": {
                "key": "{{trigger.key}}", "project": "{{trigger.projectId}}"}},
        ],
        "edges": [{"from": "created", "to": "import"}, {"from": "updated", "to": "import"}],
    }))
}

/// The built-in "Done moves the Jira issue".
fn jira_done() -> WorkflowDoc {
    doc(json!({
        "name": "Done moves the Jira issue",
        "description": "Moving a ticket linked to a Jira issue to Done moves the issue to Done too (the project's done \
                        transition, in its settings). Nothing else goes to Jira unless a workflow says so.",
        "nodes": [
            {"id": "done", "kind": "ticket.stage_changed", "version": 1, "position": {"x": 0, "y": 0}, "config": {"where": [
                {"left": "{{trigger.to}}", "op": "eq", "right": "done"},
                {"left": "{{trigger.external.jira}}", "op": "exists"}]}},
            {"id": "move", "kind": "jira.transition", "version": 1, "position": {"x": 320, "y": 0}, "config": {
                "ticket": "{{trigger.ticketId}}"}},
        ],
        "edges": [{"from": "done", "to": "move"}],
    }))
}
