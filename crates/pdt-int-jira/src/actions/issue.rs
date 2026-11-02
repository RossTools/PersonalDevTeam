use super::*;

// ---- jira.issue ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraIssueConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The issue (`ABC-123`).
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraCommentText {
    pub id: String,
    pub author: String,
    /// Text.
    pub body: String,
    /// ms since the epoch.
    #[ts(type = "number")]
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueDetails {
    #[serde(flatten)]
    #[ts(flatten)]
    pub issue: JiraIssueSummary,
    /// Text.
    pub description: String,
    pub reporter: Option<String>,
    pub components: Vec<String>,
    pub comments: Vec<JiraCommentText>,
    /// The connection's mapped fields, by the names it gives them, as Jira has them.
    #[ts(type = "Record<string, unknown>")]
    pub fields: Map<String, Value>,
}

pub struct GetIssue;

impl Action for GetIssue {
    type Input = JiraIssueConfig;
    type Output = JiraIssueDetails;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_issue", "Read a Jira issue").read_only().open_world().pm(),
            "Read a Jira issue",
            "One issue: its summary, description, status, type, assignee, reporter, priority, labels, components, \
             sprint, epic, where it sits on the board, its comments, the connection's mapped fields, and the PDT \
             ticket linked to it.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.issue",
            "Read a Jira issue",
            "One issue with its description and comments: `{{steps.<id>.status}}`, `{{steps.<id>.sprint.name}}`, \
             `{{steps.<id>.fields.<mapped name>}}`.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraIssueConfig) -> Result<(), String> {
        key_ok(&c.key)
    }

    fn approval_title(&self, c: &JiraIssueConfig) -> String {
        format!("read {} in Jira ({})", c.key, c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraIssueConfig) -> BoxFut<'a, Result<JiraIssueDetails, String>> {
        Box::pin(async move {
            let key = the_key(&c.key)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let sp = client.special().await?;
            let mut fields = list_fields(&sp);
            fields.extend(["description", "reporter", "components", "comment", "created", "updated"].map(String::from));
            fields.extend(client.mappings.fields.iter().map(|(_, id)| id.clone()));
            let v = client
                .issue_raw(&key, &fields)
                .await?
                .ok_or_else(|| format!("Jira has no issue {key} (or the account can't see it)"))?;
            let parsed = parse_issue(client.site(), &v).unwrap_or_default();
            let mapped: Map<String, Value> =
                client.mappings.fields.iter().map(|(name, id)| (name.clone(), v["fields"][id].clone())).collect();
            Ok(JiraIssueDetails {
                issue: summary_of(&client, &sp, &v, ticket_of(cx, &client, &key).await?),
                description: parsed.description,
                reporter: parsed.reporter,
                components: v["fields"]["components"]
                    .as_array()
                    .map(|l| l.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect())
                    .unwrap_or_default(),
                comments: parsed
                    .comments
                    .into_iter()
                    .map(|c| JiraCommentText { id: c.id, author: c.author, body: c.body, created: c.created })
                    .collect(),
                fields: mapped,
            })
        })
    }
}
