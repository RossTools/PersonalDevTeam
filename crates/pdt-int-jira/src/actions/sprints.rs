use super::*;

// ---- jira.sprints ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraSprintsConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The board: its id or name.
    pub board: String,
    /// The sprints' states, comma-separated (default `active,future`; `closed` too for past ones).
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraSprintsOutput {
    #[ts(type = "number")]
    pub board: i64,
    pub sprints: Vec<Sprint>,
    /// The active sprints the team works in (backlog sections left out).
    pub active: Vec<Sprint>,
    /// The backlog sections (sprints the connection names as sections).
    pub sections: Vec<Sprint>,
}

pub struct Sprints;

impl Action for Sprints {
    type Input = JiraSprintsConfig;
    type Output = JiraSprintsOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_sprints", "List a Jira board's sprints").read_only().open_world().pm(),
            "List a board's sprints",
            "A board's sprints (active and planned unless `state` says): each one's id, name and state, which are \
             the active ones, and which are backlog sections rather than sprints.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.sprints",
            "Jira sprints",
            "A board's sprints: `{{steps.<id>.active}}` is the active ones (backlog sections left out), \
             `{{steps.<id>.sections}}` the backlog sections.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraSprintsConfig) -> Result<(), String> {
        if c.board.trim().is_empty() { Err("board is empty: give its id or name".into()) } else { Ok(()) }
    }

    fn approval_title(&self, c: &JiraSprintsConfig) -> String {
        format!("list the sprints of board {} ({})", c.board, c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraSprintsConfig) -> BoxFut<'a, Result<JiraSprintsOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let board = client.board_id(&c.board).await?;
            let sprints = client.sprints(board, nonblank(&c.state).unwrap_or("active,future")).await?;
            let active = sprints.iter().filter(|s| s.state == "active" && !s.backlog_section).cloned().collect();
            let sections = sprints.iter().filter(|s| s.backlog_section).cloned().collect();
            Ok(JiraSprintsOutput { board, sprints, active, sections })
        })
    }
}
