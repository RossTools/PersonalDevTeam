use super::*;

// ---- jira.sprint ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraSprintConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The issues (`ABC-1`), up to 50.
    pub keys: Vec<String>,
    /// The sprint (or backlog section) to move them into: `active` (the board's one active sprint), a name, or an id.
    #[serde(default)]
    pub sprint: Option<String>,
    /// The board whose sprint `sprint` names (its id or name); not needed for a sprint's id.
    #[serde(default)]
    pub board: Option<String>,
    /// Move them out of their sprints, back to the backlog.
    #[serde(default)]
    pub backlog: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraSprintOutput {
    pub keys: Vec<String>,
    /// The sprint they are in now (None: the backlog).
    pub sprint: Option<Sprint>,
}

pub struct MoveToSprint;

/// The sprint a call names, on the board it names.
async fn find_sprint(client: &JiraClient, given: &str, board: Option<&str>) -> Result<Sprint, String> {
    let g = given.trim();
    if let Ok(id) = g.parse::<i64>() {
        return Ok(Sprint { id, name: g.to_string(), state: String::new(), backlog_section: false });
    }
    let board = board.ok_or_else(|| format!("name the board whose sprint {g:?} is (board)"))?;
    let all = client.sprints(client.board_id(board).await?, "active,future").await?;
    let names = |l: &[&Sprint]| l.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ");
    if g.eq_ignore_ascii_case("active") {
        let active: Vec<&Sprint> = all.iter().filter(|s| s.state == "active" && !s.backlog_section).collect();
        return match active.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(format!("board {board} has no active sprint")),
            many => Err(format!("board {board} has {} active sprints ({}); name one", many.len(), names(many))),
        };
    }
    all.iter().find(|s| s.name.eq_ignore_ascii_case(g)).cloned().ok_or_else(|| {
        let open: Vec<&Sprint> = all.iter().collect();
        format!("board {board} has no open sprint {g:?} (it has: {})", names(&open))
    })
}

impl Action for MoveToSprint {
    type Input = JiraSprintConfig;
    type Output = JiraSprintOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_sprint", "Move Jira issues between sprints and the backlog").asks().open_world().pm(),
            "Move Jira issues to a sprint or the backlog",
            "Moves issues into a sprint or backlog section (`sprint`: `active`, a name on `board`, or an id), or out \
             of their sprints to the backlog (`backlog`: true).",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.sprint",
            "Move Jira issues to a sprint",
            "Moves issues into a sprint (the board's active one, one by name or id, or a backlog section) or back to \
             the backlog.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraSprintConfig) -> Result<(), String> {
        keys_ok(&c.keys)?;
        match (nonblank(&c.sprint), c.backlog) {
            (Some(_), true) => Err("give sprint or backlog, not both".into()),
            (None, false) => Err("give the sprint to move them into, or backlog: true".into()),
            _ => Ok(()),
        }
    }

    fn approval_title(&self, c: &JiraSprintConfig) -> String {
        let to = match nonblank(&c.sprint) {
            Some(s) => format!("sprint {s}"),
            None => "the backlog".into(),
        };
        format!("Jira ({}): move {} to {to}", c.connection, c.keys.join(", "))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraSprintConfig) -> BoxFut<'a, Result<JiraSprintOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let keys = the_keys(&c.keys)?;
            if keys.len() > 50 {
                return Err("at most 50 issues at a time".into());
            }
            let client = JiraClient::named(cx.st, &c.connection).await?;
            match nonblank(&c.sprint) {
                Some(s) => {
                    let sprint = find_sprint(&client, s, nonblank(&c.board)).await?;
                    client.to_sprint(sprint.id, &keys).await?;
                    Ok(JiraSprintOutput { keys, sprint: Some(sprint) })
                }
                None => {
                    client.to_backlog(&keys).await?;
                    Ok(JiraSprintOutput { keys, sprint: None })
                }
            }
        })
    }
}
