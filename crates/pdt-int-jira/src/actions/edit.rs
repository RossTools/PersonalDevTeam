use super::*;

// ---- jira.edit ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraEditConfig {
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: String,
    /// The issue (`ABC-123`).
    pub key: String,
    #[serde(default)]
    pub summary: Option<String>,
    /// Text (sent as Jira rich text).
    #[serde(default)]
    #[schemars(extend("x-format" = "multiline"))]
    pub description: Option<String>,
    /// Who it is assigned to: `me`, a username or account id, an email or a name; `none` unassigns it.
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    /// Its labels, all of them.
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// Its components' names, all of them.
    #[serde(default)]
    pub components: Option<Vec<String>>,
    /// More fields: by id, by a name the connection maps, or by their name in Jira; plain values or Jira's form.
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub fields: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraEditOutput {
    pub key: String,
    pub url: String,
    /// The fields it set, by id.
    pub changed: Vec<String>,
}

pub struct Edit;

impl Action for Edit {
    type Input = JiraEditConfig;
    type Output = JiraEditOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_edit", "Edit and assign Jira issues").asks().open_world().pm(),
            "Edit a Jira issue",
            "Changes an issue's summary, description, assignee (`none` unassigns), priority, labels, components, \
             and any other field by id, mapped name or name in Jira. Values are matched to what each field takes \
             (numbers, options, users) from the issue's edit screen.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.edit",
            "Edit a Jira issue",
            "Changes an issue's fields or assignee; `{{steps.<id>.changed}}` lists the fields set.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraEditConfig) -> Result<(), String> {
        key_ok(&c.key)?;
        let nothing = c.summary.is_none()
            && c.description.is_none()
            && c.assignee.is_none()
            && c.priority.is_none()
            && c.labels.is_none()
            && c.components.is_none()
            && c.fields.is_empty();
        if nothing { Err("nothing to change: give a field".into()) } else { Ok(()) }
    }

    fn approval_title(&self, c: &JiraEditConfig) -> String {
        let mut what: Vec<String> = vec![];
        if let Some(s) = &c.summary {
            what.push(format!("summary={}", short(s)));
        }
        if c.description.is_some() {
            what.push("description".into());
        }
        if let Some(a) = &c.assignee {
            what.push(format!("assignee={a}"));
        }
        if let Some(p) = &c.priority {
            what.push(format!("priority={p}"));
        }
        if let Some(l) = &c.labels {
            what.push(format!("labels={}", l.join(",")));
        }
        if let Some(l) = &c.components {
            what.push(format!("components={}", l.join(",")));
        }
        if !c.fields.is_empty() {
            what.push(fields_list(&c.fields));
        }
        format!("Jira ({}): edit {}: {}", c.connection, c.key, what.join(", "))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: JiraEditConfig) -> BoxFut<'a, Result<JiraEditOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let key = the_key(&c.key)?;
            let client = JiraClient::named(cx.st, &c.connection).await?;
            let mut given = c.fields.clone();
            if let Some(s) = &c.summary {
                given.insert("summary".into(), json!(s.trim()));
            }
            if let Some(d) = &c.description {
                given.insert("description".into(), json!(d));
            }
            if let Some(p) = &c.priority {
                given.insert("priority".into(), json!(p.trim()));
            }
            if let Some(l) = &c.labels {
                given.insert("labels".into(), json!(l));
            }
            if let Some(l) = &c.components {
                given.insert("components".into(), json!(l));
            }
            let mut changed = vec![];
            if !given.is_empty() {
                let screen = client.editmeta(&key).await?;
                let fields = shape_fields(&client, &given, Some(&screen))?;
                changed.extend(fields.keys().cloned());
                client.edit(&key, fields).await?;
            }
            if let Some(a) = c.assignee.as_deref().map(str::trim) {
                let who = match a {
                    "" | "none" | "nobody" | "unassigned" => None,
                    a => Some(client.user(a).await?),
                };
                client.assign(&key, who.as_deref()).await?;
                changed.push("assignee".into());
            }
            Ok(JiraEditOutput { url: client.browse(&key), key, changed })
        })
    }
}
