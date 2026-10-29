//! Jira (PDT-85; Server / Data Center too, PDT-258): the `jira` connection a project links to (Cloud with an email
//! and API token, or Server / Data Center with a personal access token), its issue events (a 60 s check of linked
//! projects' issues), importing issues as tickets (the built-in for assigned ones, and any one by key), Done moving the
//! issue, and the Jira actions, each a workflow node and a PM tool (search, issues, transitions with their fields,
//! boards, sprints and epics). pdtd's `jira` feature.

mod actions;
mod client;
mod core;
mod meta;
mod sensor;

use crate::core::{DEFAULT_DONE, JIRA};
use pdt_api::sdk::{AnyAction, AppState, BoxFut, Builtin, Integration, Poller};
use pdt_core::integration::{ConfigField, ConnectionKind, FieldOption, FieldType, FieldWhen, ProjectSettingsSchema};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::events::EventType;
use pdt_core::workflow::template::Category;
use serde_json::{Value, json};
use std::sync::Arc;

pub use crate::actions::*;
pub use crate::core::{
    JiraIssueCommented, JiraIssueCreated, JiraIssueRef, JiraIssueTransitioned, JiraIssueUpdated, JiraSettings,
    parse_time,
};
pub use crate::meta::{Allowed, FieldMeta, MissingField, Placement, Sprint, Transition};
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
            Builtin { id: JIRA_IMPORT, enabled: true, category: Category::Issues, doc: jira_import },
            Builtin { id: JIRA_DONE, enabled: true, category: Category::Tickets, doc: jira_done },
        ]
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        let opt = |value: &str, label: &str| FieldOption { value: value.into(), label: label.into() };
        let cloud = FieldWhen::new("deployment", "cloud");
        vec![ConnectionKind {
            id: JIRA.into(),
            label: "Jira".into(),
            plural: "Jira sites".into(),
            setup: "A Jira Cloud site or a Jira Server / Data Center instance. A project links to it in its settings: \
                    issues assigned to you become tickets, and Done moves the issue. The PM and the Jira steps search, \
                    create, edit, move and comment on issues, and manage sprints and epics."
                .into(),
            description: "Jira Cloud or Server / Data Center (projects link to it; the PM and the Jira actions)."
                .into(),
            url: true,
            url_placeholder: Some("https://your-site.atlassian.net".into()),
            url_hint: Some("Your Jira Cloud site, or your Jira server’s address (https://jira.example.com).".into()),
            fields: vec![
                ConfigField {
                    r#type: Some(FieldType::Choice),
                    options: Some(vec![opt("cloud", "Jira Cloud"), opt("server", "Jira Server / Data Center")]),
                    ..ConfigField::new("deployment", "Jira")
                },
                ConfigField {
                    placeholder: Some("you@work.com".into()),
                    hint: Some("The Atlassian account’s email; PDT signs in as it.".into()),
                    when: Some(cloud),
                    ..ConfigField::new("email", "Email")
                },
                ConfigField {
                    r#type: Some(FieldType::Env),
                    placeholder: Some("storyPoints=customfield_10016\nseverity=customfield_10200".into()),
                    hint: Some(
                        "Names for this site’s custom fields, one name=field id per line; the PM and \
                                workflows can use them in place of the ids."
                            .into(),
                    ),
                    optional: true,
                    ..ConfigField::new("fields", "Field names")
                },
                ConfigField {
                    placeholder: Some("customfield_10011".into()),
                    hint: Some("The Epic Name field’s id; found by its type when left empty.".into()),
                    optional: true,
                    ..ConfigField::new("epicNameField", "Epic Name field")
                },
                ConfigField {
                    placeholder: Some("customfield_10014".into()),
                    hint: Some(
                        "The Epic Link field’s id; found by its type when left empty (Cloud without one uses \
                                the issue’s parent)."
                            .into(),
                    ),
                    optional: true,
                    ..ConfigField::new("epicLinkField", "Epic Link field")
                },
                ConfigField {
                    placeholder: Some("customfield_10020".into()),
                    hint: Some("The Sprint field’s id; found by its type when left empty.".into()),
                    optional: true,
                    ..ConfigField::new("sprintField", "Sprint field")
                },
                ConfigField {
                    r#type: Some(FieldType::Lines),
                    placeholder: Some("Backlog: *".into()),
                    hint: Some(
                        "Sprints that are sections of the backlog rather than sprints, one name per line (* at \
                                the end for any name starting so). They are never the active sprint."
                            .into(),
                    ),
                    optional: true,
                    ..ConfigField::new("backlogSections", "Backlog sections")
                },
            ],
            secret_label: "API token or personal access token".into(),
            secret_hint: "Cloud: at id.atlassian.com → Security → Create and manage API tokens → Create API token. \
                          Server / Data Center: your profile → Personal Access Tokens → Create token (sent as a \
                          bearer token)."
                .into(),
            pm_acts: Some(
                "The PM creates, edits, moves and comments on issues, imports them and changes sprints and epics \
                 without asking"
                    .into(),
            ),
            ..Default::default()
        }]
    }

    fn check_connection_config(&self, _kind: &str, config: &Value) -> Result<(), String> {
        match config.get("deployment") {
            None | Some(Value::Null) => Ok(()),
            Some(Value::String(d)) if ["cloud", "server"].contains(&d.trim()) => Ok(()),
            Some(other) => Err(format!("deployment is cloud or server, not {other}")),
        }
    }

    fn approval_title(&self, tool: &str, input: &Value) -> Option<String> {
        actions::approval_title(tool, input)
    }

    fn pm_prompt(&self) -> Option<&'static str> {
        Some(actions::PM_PROMPT)
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

    fn actions(&self) -> Vec<Arc<dyn AnyAction>> {
        actions::actions()
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
