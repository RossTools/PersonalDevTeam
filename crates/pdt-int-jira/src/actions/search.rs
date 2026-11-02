use super::*;

// ---- jira.search ----

/// How many issues a search page has unless told.
const SEARCH_LIMIT: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraSearchConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// JQL (`project = ABC AND sprint in openSprints() ORDER BY rank`).
    pub jql: String,
    /// At most this many issues (default 50, at most 100).
    #[serde(default)]
    pub limit: Option<usize>,
    /// The page to read: the last page's `next`.
    #[serde(default)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraSearchOutput {
    pub issues: Vec<JiraIssueSummary>,
    /// The next page's `page`, when there are more.
    pub next: Option<String>,
    /// How many issues match, when Jira says.
    #[ts(type = "number | null")]
    pub total: Option<i64>,
}

pub struct Search;

impl Action for Search {
    type Input = JiraSearchConfig;
    type Output = JiraSearchOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_search", "Search Jira with JQL").read_only().open_world().pm(),
            "Search Jira",
            "Issues a JQL query finds, a page at a time (`limit`, and `page`: the last page's `next`): each one's \
             key, summary, status, type, assignee, priority, labels, sprint, epic, where it sits on the board \
             (`placement`) and the PDT ticket linked to it.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.search",
            "Search Jira",
            "Issues a JQL query finds: `{{steps.<id>.issues}}` is a list of {key, summary, status, assignee, sprint, \
             epic, placement, ticketId}; `{{steps.<id>.next}}` reads the next page.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraSearchConfig) -> Result<(), String> {
        if c.jql.trim().is_empty() { Err("jql is empty".into()) } else { Ok(()) }
    }

    fn approval_title(&self, c: &JiraSearchConfig) -> String {
        format!("search Jira ({}): {}", c.connection, short(&c.jql))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraSearchConfig) -> BoxFut<'a, Result<JiraSearchOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let sp = client.special().await?;
            let limit = c.limit.unwrap_or(SEARCH_LIMIT).clamp(1, 100);
            let page = client.search_page(c.jql.trim(), &list_fields(&sp), false, limit, nonblank(&c.page)).await?;
            Ok(JiraSearchOutput {
                issues: summaries(cx, &client, &sp, &page.issues).await?,
                next: page.next,
                total: page.total,
            })
        })
    }
}
