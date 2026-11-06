use super::*;

// ---- jira.transition ----

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct JiraTransitionConfig {
    /// A `jira` connection (with `key`).
    #[serde(default)]
    #[schemars(extend("x-connection-kind" = "jira"))]
    pub connection: Option<String>,
    /// The issue (`ABC-123`).
    #[serde(default)]
    pub key: Option<String>,
    /// Or a ticket (`{{trigger.ticketId}}`): its Jira issue, through the connection it was linked with.
    #[serde(default)]
    pub ticket: Option<String>,
    /// A transition's id or name, or the status it leads to; with `ticket`, default its project's done mapping.
    #[serde(default)]
    pub to: Option<String>,
    /// The fields the transition's screen asks for, sent with it: by id, mapped name or name in Jira, with plain
    /// values (`{"Resolution": "Fixed", "Story Points": 3}`) or Jira's form.
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub fields: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraTransitionOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub url: Option<String>,
    /// The transition it took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub transition: Option<String>,
    /// The issue's status now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<String>,
    /// Where it was asked to go, when it couldn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<String>,
    /// The transitions open to it, when none was `to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub available: Option<Vec<String>>,
    /// Why it couldn't move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    /// Nothing to tell the user: it was there already, or can't go there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub quiet: Option<bool>,
    /// It could not move (the `unavailable` port).
    #[serde(skip)]
    #[schemars(skip)]
    pub unavailable: bool,
}

pub struct JiraTransition;

/// Where a transition goes: the connection, the issue and where to.
async fn transition_target(
    cx: &ActionCx<'_>,
    c: &JiraTransitionConfig,
) -> Result<Result<(String, String, String), JiraTransitionOutput>, String> {
    Ok(Ok(match nonblank(&c.ticket) {
        Some(ticket) => {
            let links = cx.st.store.external_links(ticket, JIRA).await.map_err(pdt_api::sdk::err)?;
            let Some(link) = links.into_iter().next() else {
                return Ok(Err(JiraTransitionOutput {
                    reason: Some("the ticket is not linked to a Jira issue".into()),
                    quiet: Some(true),
                    unavailable: true,
                    ..Default::default()
                }));
            };
            let conn = cx
                .st
                .store
                .get_connection(&link.connection_id)
                .await
                .map_err(pdt_api::sdk::err)?
                .ok_or("the Jira connection the ticket was linked with is gone")?;
            let to = match nonblank(&c.to) {
                Some(t) => t.to_string(),
                None => {
                    let summary = cx.st.store.get_ticket_summary(ticket).await.map_err(pdt_api::sdk::err)?;
                    let project = summary.map(|t| t.project_id).unwrap_or_default();
                    let s = cx.st.store.get_project_settings(&project).await.map_err(pdt_api::sdk::err)?;
                    s.integration::<JiraSettings>(JIRA).map(|j| j.done).unwrap_or_else(|| DEFAULT_DONE.into())
                }
            };
            (conn.name, link.key, to)
        }
        None => (
            nonblank(&c.connection).unwrap_or_default().to_string(),
            issue_key(nonblank(&c.key).unwrap_or_default()).ok_or("key is not an issue key")?,
            nonblank(&c.to).unwrap_or_default().to_string(),
        ),
    }))
}

impl Action for JiraTransition {
    type Input = JiraTransitionConfig;
    type Output = JiraTransitionOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("jira_transition", "Move Jira issues through their workflow").asks().open_world().pm(),
            "Move a Jira issue",
            "Moves an issue (by key, or a ticket's linked issue) through the transition `to` (an id, a name, or the \
             status it leads to), with the fields its screen asks for in the same request (`fields`: numbers, \
             choices by name). Required fields left out are named in the error; jira_transitions lists them first.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "jira.transition",
            "Move a Jira issue",
            "Moves a Jira issue (by key, or a ticket's linked issue) through the transition named `to`, or the one \
             leading to that status, with the fields its screen needs (`fields`). An issue already there counts as \
             done; no such transition from its status leaves by `unavailable`; a required field left out fails the \
             step, naming it.",
            &[("out", "Then"), ("unavailable", "Can't move there")],
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &JiraTransitionConfig) -> Result<(), String> {
        match (nonblank(&c.key), nonblank(&c.ticket)) {
            (Some(k), None) => {
                if nonblank(&c.connection).is_none() {
                    return Err("with key, name the jira connection".into());
                }
                if nonblank(&c.to).is_none() {
                    return Err("to is empty: name a transition or status".into());
                }
                key_ok(k)
            }
            (None, Some(_)) => Ok(()),
            _ => Err("give either key (with connection) or ticket".into()),
        }
    }

    fn approval_title(&self, c: &JiraTransitionConfig) -> String {
        let what =
            nonblank(&c.key).map(String::from).or_else(|| nonblank(&c.ticket).map(|t| format!("ticket {t}'s issue")));
        let mut s = format!(
            "Jira: move {} to {}",
            what.unwrap_or_else(|| "the issue".into()),
            nonblank(&c.to).unwrap_or("its done status")
        );
        if !c.fields.is_empty() {
            s.push_str(&format!(" with {}", fields_list(&c.fields)));
        }
        s
    }

    fn run<'a>(
        &'a self,
        cx: &'a ActionCx<'a>,
        c: JiraTransitionConfig,
    ) -> BoxFut<'a, Result<JiraTransitionOutput, String>> {
        Box::pin(async move {
            self.check(&c)?;
            let (connection, key, to) = match transition_target(cx, &c).await? {
                Ok(t) => t,
                Err(out) => return Ok(out),
            };
            let client = JiraClient::named(cx.st, &connection).await?;
            let url = client.browse(&key);
            let issue = client.issue(&key).await?.ok_or_else(|| format!("Jira has no issue {key}"))?;
            if issue.status.eq_ignore_ascii_case(&to) {
                return Ok(JiraTransitionOutput {
                    key: Some(key),
                    url: Some(url),
                    status: Some(issue.status),
                    quiet: Some(true),
                    ..Default::default()
                });
            }
            let list = client.transitions(&key).await?;
            let Some(t) = meta::pick_transition(&list, &to) else {
                return Ok(JiraTransitionOutput {
                    available: Some(list.iter().map(|t| t.name.clone()).collect()),
                    key: Some(key),
                    url: Some(url),
                    status: Some(issue.status),
                    to: Some(to),
                    quiet: Some(true),
                    unavailable: true,
                    ..Default::default()
                });
            };
            let fields = meta::coerce_fields(&c.fields, &client.mappings, &t.fields, client.deployment)?;
            let mut missing: Vec<MissingField> = meta::missing_required(&t.fields, &fields);
            // Jira marks a field required whenever its configuration does, even when the issue has a value already,
            // and takes the transition without it then: only the ones still empty are missing.
            if !missing.is_empty() {
                let ids: Vec<String> = missing.iter().map(|m| m.id.clone()).collect();
                let now = client.issue_raw(&key, &ids).await?.unwrap_or(Value::Null);
                missing.retain(|m| !meta::has_value(&now["fields"][&m.id]));
            }
            if !missing.is_empty() {
                return Err(format!(
                    "{key}: the transition {} needs {}. Give them in `fields` (by name or id) and try again.",
                    t.name,
                    meta::describe_missing(&missing)
                ));
            }
            client.do_transition(&key, &t.id, fields).await?;
            Ok(JiraTransitionOutput {
                key: Some(key),
                url: Some(url),
                transition: Some(t.name.clone()),
                status: Some(t.to.clone()),
                ..Default::default()
            })
        })
    }

    fn port(&self, out: &JiraTransitionOutput) -> &'static str {
        if out.unavailable { "unavailable" } else { "out" }
    }
}
