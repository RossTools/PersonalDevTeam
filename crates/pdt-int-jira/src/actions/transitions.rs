use super::*;

// ---- jira.transitions ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraTransitionsOutput {
    pub key: String,
    pub status: String,
    /// The moves open to it now, each with the fields its screen has (`required` ones must be given).
    pub transitions: Vec<Transition>,
}

pub struct Transitions;

impl Action for Transitions {
    type Input = JiraIssueConfig;
    type Output = JiraTransitionsOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_transitions", "See how a Jira issue can move").read_only().open_world().pm(),
            "See a Jira issue's transitions",
            "The transitions open to an issue now (id, name, the status each leads to) and the fields each one's \
             screen has: which are required, their type, and the values a choice allows.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.transitions",
            "Jira issue's transitions",
            "The moves open to an issue now and the fields each needs: `{{steps.<id>.transitions}}`.",
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
        format!("read {}'s transitions in Jira ({})", c.key, c.connection)
    }

    fn run<'a>(
        &'a self,
        cx: &'a ActionCx<'a>,
        c: JiraIssueConfig,
    ) -> BoxFut<'a, Result<JiraTransitionsOutput, String>> {
        Box::pin(async move {
            let key = the_key(&c.key)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let issue = client.issue(&key).await?.ok_or_else(|| format!("Jira has no issue {key}"))?;
            Ok(JiraTransitionsOutput { status: issue.status, transitions: client.transitions(&key).await?, key })
        })
    }
}
