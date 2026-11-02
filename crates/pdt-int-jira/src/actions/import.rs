use super::*;

// ---- jira.import ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraImportConfig {
    /// The issue (`ABC-123`, `{{trigger.key}}`).
    pub key: String,
    /// The PDT project the ticket goes in (`{{trigger.projectId}}`).
    pub project: String,
    /// A `jira` connection; default the project's.
    #[serde(default)]
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraImportOutput {
    pub ticket_id: String,
    pub key: String,
    /// A ticket already linked the issue: nothing was made.
    #[serde(default)]
    pub existed: bool,
}

pub struct JiraImport;

impl Action for JiraImport {
    type Input = JiraImportConfig;
    type Output = JiraImportOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_import", "Import a Jira issue as a ticket").asks().open_world().pm(),
            "Import a Jira issue as a ticket",
            "Makes a Draft ticket in a PDT project (`project`: its id) from a Jira issue (its summary and \
             description), linked to the issue. An issue a ticket already links is never imported again: the answer \
             names that ticket (`existed`).",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.import",
            "Import a Jira issue",
            "Makes a Draft ticket from a Jira issue (its summary and description), linked to it. When a ticket \
             already links the issue nothing is made and the step leaves by `exists`: an issue is never imported \
             twice.",
            &[("imported", "Imported"), ("exists", "Already a ticket")],
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraImportConfig) -> Result<(), String> {
        if c.project.trim().is_empty() {
            return Err("project is empty: name the PDT project ({{trigger.projectId}})".into());
        }
        key_ok(&c.key)
    }

    fn approval_title(&self, c: &JiraImportConfig) -> String {
        format!("import {} as a ticket", c.key)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraImportConfig) -> BoxFut<'a, Result<JiraImportOutput, String>> {
        Box::pin(async move {
            let r = import_issue(cx.st, &c.project, &c.key, c.connection.as_deref()).await.map_err(|e| e.message())?;
            let key = issue_key(&c.key).unwrap_or(c.key);
            Ok(match r {
                Import::Imported(ticket_id) => JiraImportOutput { ticket_id, key, existed: false },
                Import::Exists(ticket_id) => JiraImportOutput { ticket_id, key, existed: true },
            })
        })
    }

    fn port(&self, out: &JiraImportOutput) -> &'static str {
        if out.existed { "exists" } else { "imported" }
    }

    fn node_output(&self, out: JiraImportOutput) -> Value {
        let quiet = out.existed;
        let mut v = serde_json::to_value(out).unwrap_or_default();
        if quiet {
            v["quiet"] = json!(true);
        }
        v
    }
}
