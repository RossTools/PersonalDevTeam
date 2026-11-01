//! `update_ticket`: a session rewrites its own ticket.

use super::*;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateTicket {
    /// A new title (one line).
    #[serde(default)]
    title: Option<String>,
    /// The new description (Markdown), replacing the old one: keep what still holds.
    #[serde(default)]
    description: Option<String>,
    /// The parts of the project it touches, from its areas (`projectAreas` in `get_ticket`); replaces every area.
    /// "Pick up tickets" keeps tickets in the same busy areas from running at once.
    #[serde(default)]
    areas: Option<Vec<String>>,
}

pub(super) fn defs() -> Vec<Tool> {
    vec![Tool {
        decl: ToolDecl::new("update_ticket", "Rewrite its own ticket's title, description and areas"),
        title: "Update the ticket",
        description: "Rewrites this session's ticket: its title, description and/or areas (as `PATCH \
                          /api/tickets/{id}`, recorded as the agent's edit). Only this session's own ticket.",
        schema: schema::<UpdateTicket>,
    }]
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
        "update_ticket" => {
            let a: UpdateTicket = args(a)?;
            if a.title.is_none() && a.description.is_none() && a.areas.is_none() {
                return Err("give a title, a description, areas or any of them".into());
            }
            let id = ticket_for(cx, None).await?;
            let areas = checked_areas(st, &cx.session.project_id, a.areas).await?;
            let patch = pdt_core::ticket::TicketPatch {
                title: a.title,
                description: a.description,
                areas,
                ..Default::default()
            };
            let valid = patch.validate().map_err(|e| api(e.into()))?;
            let t = st
                .store
                .patch_ticket_by(&id, &valid, agent_by(st, cx.session).await, now_ms())
                .await
                .map_err(|e| api(e.into()))?
                .ok_or("that ticket no longer exists")?;
            Ok(json!({"updated": true, "key": t.summary.key, "title": t.summary.title, "areas": t.summary.areas}))
        }
        other => Err(format!("no tool is called {other}")),
    }
}
