//! `send_to_session`: the PM messages a session.

use super::*;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendToSession {
    /// The session's id.
    session: String,
    /// The message. PDT delivers it when the session is idle, marked as from the PM.
    text: String,
}

pub(super) fn defs() -> Vec<Tool> {
    vec![Tool {
        decl: ToolDecl::new("send_to_session", "Message a session").asks().pm(),
        title: "Message a session",
        description: "Sends an agent session a message, delivered when it is idle and marked as from the PM. The \
                          user confirms each call unless the project lets you act.",
        schema: schema::<SendToSession>,
    }]
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
        "send_to_session" => {
            let a: SendToSession = args(a)?;
            let to = st
                .store
                .get_session(a.session.trim())
                .await
                .map_err(store)?
                .ok_or_else(|| format!("there is no session {}", a.session))?;
            if !crate::pm::takes_messages(&to) {
                return Err("that session has ended or is not an agent session".into());
            }
            if to.id == cx.session.id {
                return Err("that is this session".into());
            }
            confirmable(cx, name, &pdt_core::pm::AskTarget::Session(to.id.clone())).await?;
            let text = a.text.trim();
            if text.is_empty() {
                return Err("the message is empty".into());
            }
            let m = pdt_store::SessionMessage {
                id: uuid::Uuid::now_v7().to_string(),
                attempt_id: None,
                session_id: Some(to.id.clone()),
                ticket_id: to.ticket.as_ref().map(|t| t.id.clone()),
                dedupe: None,
                text: format!("[{}] From the project manager: {text}", pdt_core::brand::SHORT_NAME),
                timeline: Value::Null,
                run_id: None,
                step_id: None,
                status: "waiting".into(),
                told_session: None,
                created_at: now_ms(),
                resolved_at: None,
                by_user: false,
                pending_id: None,
                reopens: false,
            };
            let m = st.store.queue_session_message(&m).await.map_err(store)?;
            Ok(json!({"queued": true, "messageId": m.id, "to": to.title, "ticket": to.ticket.as_ref().map(|t| &t.key)}))
        }
        other => Err(format!("no tool is called {other}")),
    }
}
