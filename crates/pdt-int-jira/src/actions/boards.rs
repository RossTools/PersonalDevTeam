use super::*;

// ---- jira.boards ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraBoardsConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// Only a project's boards (its key).
    #[serde(default)]
    pub project: Option<String>,
    /// Only boards whose name has these words.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraBoardInfo {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    /// `scrum` or `kanban`.
    pub r#type: String,
    /// Its project's key.
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraBoardsOutput {
    pub boards: Vec<JiraBoardInfo>,
}

pub struct Boards;

impl Action for Boards {
    type Input = JiraBoardsConfig;
    type Output = JiraBoardsOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_boards", "List Jira boards").read_only().open_world().pm(),
            "List Jira boards",
            "Jira's boards (a project's, or by name): each one's id, name, type and project.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.boards",
            "Jira boards",
            "Jira's boards: `{{steps.<id>.boards}}` is a list of {id, name, type, project}.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn approval_title(&self, c: &JiraBoardsConfig) -> String {
        format!("list Jira boards ({})", c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraBoardsConfig) -> BoxFut<'a, Result<JiraBoardsOutput, String>> {
        Box::pin(async move {
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let list = client.boards(nonblank(&c.project), nonblank(&c.name)).await?;
            Ok(JiraBoardsOutput {
                boards: list
                    .iter()
                    .map(|b| JiraBoardInfo {
                        id: b["id"].as_i64().unwrap_or_default(),
                        name: b["name"].as_str().unwrap_or_default().to_string(),
                        r#type: b["type"].as_str().unwrap_or_default().to_string(),
                        project: b["location"]["projectKey"].as_str().map(String::from),
                    })
                    .collect(),
            })
        })
    }
}
