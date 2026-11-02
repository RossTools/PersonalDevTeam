use super::*;

// ---- jira.check ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraCheckConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraCheckOutput {
    pub connection: String,
    /// `cloud` or `server`.
    pub deployment: String,
    pub site: String,
    /// Who the connection signs in as.
    pub user: String,
    pub user_id: String,
    /// Jira's version, when it says.
    pub version: Option<String>,
}

pub struct Check;

impl Action for Check {
    type Input = JiraCheckConfig;
    type Output = JiraCheckOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_check", "Test a Jira connection").read_only().open_world().pm(),
            "Test a Jira connection",
            "Signs in to Jira with the connection and says who it is signed in as, the site, Cloud or Server / Data \
             Center, and Jira's version. The token never appears.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.check",
            "Test Jira",
            "Signs in with a Jira connection: `{{steps.<id>.user}}` is who it is; a connection that fails fails the \
             step.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn approval_title(&self, c: &JiraCheckConfig) -> String {
        format!("test the Jira connection {}", c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraCheckConfig) -> BoxFut<'a, Result<JiraCheckOutput, String>> {
        Box::pin(async move {
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let me = client.myself_raw().await?;
            let info = client.server_info().await;
            Ok(JiraCheckOutput {
                connection: client.conn.name.clone(),
                deployment: match client.deployment {
                    Deployment::Cloud => "cloud".into(),
                    Deployment::Server => "server".into(),
                },
                site: client.site().to_string(),
                user: me["displayName"].as_str().unwrap_or_default().to_string(),
                user_id: client.deployment.user_id(&me).unwrap_or_default(),
                version: info.and_then(|i| i["version"].as_str().map(String::from)),
            })
        })
    }
}
