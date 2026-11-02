use super::*;

// ---- jira.board ----

/// How many of a board's issues are read unless told.
const BOARD_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraBoardConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The board: its id or name.
    pub board: String,
    /// Only issues this JQL finds too (`assignee = currentUser()`).
    #[serde(default)]
    pub jql: Option<String>,
    /// At most this many issues (default 100, at most 500).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraBoardOutput {
    #[ts(type = "number")]
    pub board: i64,
    /// Active sprints first (by status), then backlog sections, planned sprints and the backlog.
    pub issues: Vec<JiraIssueSummary>,
    /// The board's issues (that the JQL finds), also those past `limit`.
    #[ts(type = "number")]
    pub total: i64,
}

pub struct Board;

impl Action for Board {
    type Input = JiraBoardConfig;
    type Output = JiraBoardOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_board", "See where issues sit on a Jira board").read_only().open_world().pm(),
            "See a Jira board",
            "A board's issues and where each one sits (`placement`): in an active sprint in its status's column, in \
             a backlog section, planned for a sprint, or in the backlog.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.board",
            "Jira board",
            "A board's issues with where each sits: `{{steps.<id>.issues}}`, each with `placement` {area: sprint, \
             section, future or backlog; sprint; status}.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraBoardConfig) -> Result<(), String> {
        if c.board.trim().is_empty() { Err("board is empty: give its id or name".into()) } else { Ok(()) }
    }

    fn approval_title(&self, c: &JiraBoardConfig) -> String {
        format!("read board {} ({})", c.board, c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraBoardConfig) -> BoxFut<'a, Result<JiraBoardOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let sp = client.special().await?;
            let board = client.board_id(&c.board).await?;
            let mut fields = list_fields(&sp);
            fields.push("sprint".into());
            let limit = c.limit.unwrap_or(BOARD_LIMIT).clamp(1, 500);
            let mut raw = vec![];
            let mut total;
            loop {
                let (page, t) = client
                    .board_issues(board, nonblank(&c.jql), &fields, raw.len(), (limit - raw.len()).min(100))
                    .await?;
                total = t;
                let n = page.len();
                raw.extend(page);
                if n == 0 || raw.len() >= limit || raw.len() as i64 >= total {
                    break;
                }
            }
            let mut issues = summaries(cx, &client, &sp, &raw).await?;
            let order = |p: &Placement| {
                ["sprint", "section", "future", "backlog"].iter().position(|a| *a == p.area).unwrap_or(4)
            };
            issues.sort_by_key(|i| order(&i.placement));
            Ok(JiraBoardOutput { board, issues, total })
        })
    }
}
