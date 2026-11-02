use super::*;

// ---- jira.comment ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraCommentConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    pub key: String,
    /// Text (sent as Jira rich text: a paragraph per blank-line-separated block).
    #[schemars(extend("x-format" = "multiline"))]
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraCommentOutput {
    pub key: String,
    pub comment_id: String,
}

pub struct JiraComment;

impl Action for JiraComment {
    type Input = JiraCommentConfig;
    type Output = JiraCommentOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_comment", "Comment on Jira issues").asks().open_world().pm(),
            "Comment on a Jira issue",
            "Adds a comment (text: a paragraph per blank-line-separated block) to a Jira issue.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.comment",
            "Comment on a Jira issue",
            "Adds a comment to a Jira issue; its id is `{{steps.<id>.commentId}}`.",
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

    fn check(&self, c: &JiraCommentConfig) -> Result<(), String> {
        if c.connection.trim().is_empty() {
            return Err("connection is empty: name a jira connection".into());
        }
        if c.body.trim().is_empty() {
            return Err("body is empty".into());
        }
        key_ok(&c.key)
    }

    fn approval_title(&self, c: &JiraCommentConfig) -> String {
        format!("Jira ({}): comment on {}: {}", c.connection, c.key, short(&c.body))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraCommentConfig) -> BoxFut<'a, Result<JiraCommentOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let key = the_key(&c.key)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let comment_id = client.comment(&key, &c.body).await?;
            Ok(JiraCommentOutput { key, comment_id })
        })
    }
}
