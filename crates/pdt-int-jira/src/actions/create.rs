use super::*;

// ---- jira.create ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraCreateConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The Jira project's key (`ABC`).
    pub project: String,
    /// Default `Task`; `Epic` makes an epic (its Epic Name is `epicName`, else the summary).
    #[serde(default = "task")]
    pub issue_type: String,
    pub summary: String,
    /// Text (sent as Jira rich text).
    #[serde(default)]
    #[schemars(extend("x-format" = "multiline"))]
    pub description: String,
    /// Who it is assigned to: `me`, a username or account id, an email or a name.
    #[serde(default)]
    pub assignee: Option<String>,
    /// A priority's name (`High`).
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Components' names.
    #[serde(default)]
    pub components: Vec<String>,
    /// The epic it goes in (`ABC-10`).
    #[serde(default)]
    pub epic: Option<String>,
    /// An epic's Epic Name (default its summary).
    #[serde(default)]
    pub epic_name: Option<String>,
    /// More fields: by id (`customfield_10016`), by a name the connection maps, or by their name in Jira, with plain
    /// values (`{"Story Points": 3, "Severity": "Sev 2"}`) or Jira's own form.
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub fields: Map<String, Value>,
}

fn task() -> String {
    "Task".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueOutput {
    pub key: String,
    pub url: String,
}

pub struct JiraCreate;

impl Action for JiraCreate {
    type Input = JiraCreateConfig;
    type Output = JiraIssueOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_create", "Create Jira issues and epics").asks().open_world().pm(),
            "Create a Jira issue",
            "Creates an issue (or an epic: `issueType` Epic) in a Jira project with its type, description, assignee, \
             priority, labels, components and epic, and any other field by id, mapped name or name in Jira. Fields \
             the create screen requires and the call leaves out are named in the error.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.create",
            "Create a Jira issue",
            "Creates an issue in a Jira project; its key is `{{steps.<id>.key}}`.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    /// What it does can't be told apart from doing it twice (PDT-131): cut short, it isn't run again on its own.
    fn rerun_safe(&self) -> bool {
        false
    }

    fn check(&self, c: &JiraCreateConfig) -> Result<(), String> {
        for (field, v) in [("connection", &c.connection), ("project", &c.project), ("summary", &c.summary)] {
            if v.trim().is_empty() {
                return Err(format!("{field} is empty"));
            }
        }
        if let Some(e) = nonblank(&c.epic) {
            key_ok(e)?;
        }
        Ok(())
    }

    fn approval_title(&self, c: &JiraCreateConfig) -> String {
        format!("Jira ({}): create a {} in {}: {}", c.connection, c.issue_type, c.project, short(&c.summary))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraCreateConfig) -> BoxFut<'a, Result<JiraIssueOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let project = c.project.trim().to_string();
            let screen = client.createmeta(&project, &c.issue_type).await?;
            let on_screen = |id: &str| screen.as_ref().is_none_or(|m| m.iter().any(|f| f.id == id));
            let mut given = c.fields.clone();
            if let Some(p) = nonblank(&c.priority) {
                given.insert("priority".into(), json!(p));
            }
            if !c.labels.is_empty() {
                given.insert("labels".into(), json!(c.labels));
            }
            if !c.components.is_empty() {
                given.insert("components".into(), json!(c.components));
            }
            let is_epic = c.issue_type.trim().eq_ignore_ascii_case("epic");
            let epic = nonblank(&c.epic).map(the_key).transpose()?;
            let mut via_agile = None;
            if is_epic || epic.is_some() {
                let sp = client.special().await?;
                if is_epic && let Some(id) = sp.epic_name.as_deref().filter(|id| on_screen(id)) {
                    given.insert(id.into(), json!(nonblank(&c.epic_name).unwrap_or(c.summary.trim())));
                }
                if let Some(e) = &epic {
                    match (sp.epic_link.as_deref().filter(|id| on_screen(id)), client.deployment) {
                        (Some(id), _) => _ = given.insert(id.into(), json!(e)),
                        (None, Deployment::Cloud) => _ = given.insert("parent".into(), json!({"key": e})),
                        (None, Deployment::Server) => via_agile = Some(e.clone()),
                    }
                }
            }
            let mut fields = shape_fields(&client, &given, screen.as_deref())?;
            if let Some(a) = nonblank(&c.assignee) {
                fields.insert("assignee".into(), client.deployment.user(&client.user(a).await?));
            }
            fields.insert("project".into(), json!({"key": project}));
            fields.insert("issuetype".into(), json!({"name": c.issue_type.trim()}));
            fields.insert("summary".into(), json!(c.summary.trim()));
            if !c.description.trim().is_empty() {
                fields.insert("description".into(), client.deployment.rich_text(&c.description));
            }
            if let Some(m) = &screen {
                let missing = meta::missing_required(m, &fields);
                if !missing.is_empty() {
                    return Err(format!(
                        "Jira needs more to create a {} in {project}: {}. Give them in `fields` (by name or id).",
                        c.issue_type,
                        meta::describe_missing(&missing)
                    ));
                }
            }
            let key = client.create(fields).await?;
            if let Some(e) = via_agile {
                client.agile_epic(Some(&e), std::slice::from_ref(&key)).await?;
            }
            Ok(JiraIssueOutput { url: client.browse(&key), key })
        })
    }
}
