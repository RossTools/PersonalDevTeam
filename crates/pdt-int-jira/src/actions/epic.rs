use super::*;

// ---- jira.epic ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraEpicConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The issues (`ABC-1`).
    pub keys: Vec<String>,
    /// The epic to put them in (`ABC-10`); empty takes them out of their epic.
    #[serde(default)]
    pub epic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraEpicOutput {
    pub keys: Vec<String>,
    /// Their epic now (None: none).
    pub epic: Option<String>,
}

pub struct SetEpic;

impl Action for SetEpic {
    type Input = JiraEpicConfig;
    type Output = JiraEpicOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_epic", "Put Jira issues in an epic or take them out").asks().open_world().pm(),
            "Set Jira issues' epic",
            "Puts issues in an epic (`epic`: its key), or takes them out of theirs (no `epic`), through the site's \
             Epic Link field (the connection's mapping, else found by its type) or, on Cloud without one, the \
             issue's parent.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.epic",
            "Set Jira issues' epic",
            "Puts issues in an epic, or takes them out of theirs (`epic` empty).",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraEpicConfig) -> Result<(), String> {
        keys_ok(&c.keys)?;
        nonblank(&c.epic).map_or(Ok(()), key_ok)
    }

    fn approval_title(&self, c: &JiraEpicConfig) -> String {
        match nonblank(&c.epic) {
            Some(e) => format!("Jira ({}): put {} in epic {e}", c.connection, c.keys.join(", ")),
            None => format!("Jira ({}): take {} out of their epic", c.connection, c.keys.join(", ")),
        }
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraEpicConfig) -> BoxFut<'a, Result<JiraEpicOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let keys = the_keys(&c.keys)?;
            let epic = nonblank(&c.epic).map(the_key).transpose()?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let sp = client.special().await?;
            match (sp.epic_link, client.deployment) {
                (Some(field), _) => {
                    for k in &keys {
                        let mut f = Map::new();
                        f.insert(field.clone(), json!(epic));
                        client.edit(k, f).await?;
                    }
                }
                (None, Deployment::Cloud) => {
                    for k in &keys {
                        let mut f = Map::new();
                        f.insert("parent".into(), epic.as_ref().map_or(Value::Null, |e| json!({"key": e})));
                        client.edit(k, f).await?;
                    }
                }
                (None, Deployment::Server) => client.agile_epic(epic.as_deref(), &keys).await?,
            }
            Ok(JiraEpicOutput { keys, epic })
        })
    }
}
