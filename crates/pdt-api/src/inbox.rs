//! The Inbox API (Phase 7b spec §6): what waits on the user. Stored items (questions, gates, stalled steps, loop
//! limits) plus the open permission requests of live conversations (approvals), most urgent first.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::conversation::{AgentEvent, RequestState};
use pdt_core::inbox::{self, InboxAnswer, InboxChoice, InboxGroup, InboxItem, InboxKind, InboxList, InboxStatus};
use pdt_core::pr::NoticeKind;
use pdt_core::ticket::AttemptState;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ListQuery {
    project: Option<String>,
}

/// `GET /api/inbox?project=`.
pub(crate) async fn list(State(st): State<AppState>, Query(q): Query<ListQuery>) -> Result<Json<InboxList>, ApiError> {
    let mut items = st.store.open_inbox_items(q.project.as_deref()).await?;
    items.extend(approvals(&st, q.project.as_deref()).await?);
    items.extend(sessions_waiting(&st, q.project.as_deref()).await?);
    items.extend(connect_forge(&st));
    decorate(&st, &mut items).await?;
    inbox::sort(&mut items);
    let total = u32::try_from(items.len()).unwrap_or(u32::MAX);
    Ok(Json(InboxList { items, total }))
}

/// Ticket keys and titles, persona names.
pub(crate) async fn decorate(st: &AppState, items: &mut [InboxItem]) -> Result<(), ApiError> {
    let mut tickets: HashMap<String, Option<(String, String)>> = HashMap::new();
    let mut personas: HashMap<String, Option<String>> = HashMap::new();
    for i in items.iter_mut() {
        if let Some(t) = &i.ticket_id {
            if !tickets.contains_key(t) {
                let found = st.store.get_ticket(t).await?.map(|x| (x.summary.key.clone(), x.summary.title.clone()));
                tickets.insert(t.clone(), found);
            }
            if let Some(Some((key, title))) = tickets.get(t) {
                i.ticket_key = Some(key.clone());
                i.ticket_title = Some(title.clone());
            }
        }
        if let Some(p) = &i.persona_id {
            if !personas.contains_key(p) {
                personas.insert(p.clone(), st.store.get_persona(p).await?.map(|x| x.name));
            }
            i.persona_name = personas.get(p).cloned().flatten();
        }
    }
    Ok(())
}

/// Sessions waiting on a permission prompt or a question, as items: their live `session.needs_you` notifications
/// (PDT-108), so viewing a session or turning the kind down takes it out. A conversation with an open approval is
/// listed under Approvals instead (the Notifier makes no row for it).
async fn sessions_waiting(st: &AppState, project: Option<&str>) -> Result<Vec<InboxItem>, ApiError> {
    let f = pdt_store::NotificationFilter {
        section: Some(pdt_core::notification::Section::NeedsYou),
        project_id: project.map(String::from),
        live: true,
        limit: 500,
        ..Default::default()
    };
    let mut out = vec![];
    for n in st.store.list_notifications(&f).await? {
        if n.kind != pdt_core::notification::NotificationKind::SessionNeedsYou {
            continue;
        }
        let Some(sid) = n.session_id.clone() else { continue };
        let Some(s) = st.store.get_session(&sid).await? else { continue };
        out.push(InboxItem {
            id: format!("session:{sid}"),
            kind: InboxKind::Session,
            group: InboxGroup::Session,
            status: InboxStatus::Open,
            project_id: Some(s.project_id.clone()),
            ticket_id: s.ticket.as_ref().map(|t| t.id.clone()),
            ticket_key: None,
            ticket_title: None,
            attempt_id: s.attempt_id.clone(),
            session_id: Some(sid),
            run_id: None,
            step_id: s.persona.as_ref().and_then(|p| p.step_id.clone()),
            persona_id: s.persona.as_ref().map(|p| p.id.clone()),
            persona_name: None,
            title: s.title.clone(),
            body: json!({"sessionTitle": s.title, "agent": s.agent, "mode": s.mode, "notificationId": n.id}),
            choices: vec![],
            resolution: Value::Null,
            created_at: n.created_at,
            resolved_at: None,
        });
    }
    Ok(out)
}

/// "Connect GitHub" (PDT-177) while the GitHub forge has no login and something (the PR poller, a merge, an agent's
/// tool) wanted it; "Reconnect GitHub" while GitHub refuses the login it has (PDT-212). Not stored: one item, which
/// goes once the user connects.
pub(crate) fn connect_forge(st: &AppState) -> Option<InboxItem> {
    if !st.github.login_needed() {
        return None;
    }
    let rejected = st.github.login_rejected_at();
    let (title, body) = match rejected {
        Some(_) => (
            "Reconnect GitHub",
            json!({"forge": "github", "reason": "rejected", "text": "GitHub no longer accepts PDT's login (the token \
was revoked or expired). Until you connect again, PDT has stopped calling GitHub: it can't follow pull requests or CI, \
merge, or let agents read CI logs. Connect GitHub again in Settings › Plugins › GitHub."}),
        ),
        None => (
            "Connect GitHub",
            json!({"forge": "github", "text": "PDT reads pull requests, CI and issues, merges and comments on \
GitHub with your GitHub login, and it has none: until you connect, it can't follow pull requests or CI, merge, or \
let agents read CI logs. Connect GitHub in Settings › Plugins › GitHub."}),
        ),
    };
    Some(InboxItem {
        id: "connect:github".into(),
        kind: InboxKind::Connect,
        group: InboxGroup::Notice,
        status: InboxStatus::Open,
        project_id: None,
        ticket_id: None,
        ticket_key: None,
        ticket_title: None,
        attempt_id: None,
        session_id: None,
        run_id: None,
        step_id: None,
        persona_id: None,
        persona_name: None,
        title: title.into(),
        body,
        choices: vec![],
        resolution: Value::Null,
        created_at: rejected.unwrap_or_else(now_ms),
        resolved_at: None,
    })
}

/// The open permission requests of live conversations, as items.
pub(crate) async fn approvals(st: &AppState, project: Option<&str>) -> Result<Vec<InboxItem>, ApiError> {
    let mut out = vec![];
    for (sid, ids) in crate::conversations::open_requests(st) {
        let Some(s) = st.store.get_session(&sid).await? else { continue };
        if project.is_some_and(|p| p != s.project_id) {
            continue;
        }
        // A persona's session works on its step's ticket.
        let step = st.store.active_agent_step(&sid).await?;
        for e in st.store.session_events_of_kind(&sid, "request").await? {
            let AgentEvent::Request { id, tool, title, description, input, diff, state, can_always, .. } = &e.event
            else {
                continue;
            };
            if *state != RequestState::Open || !ids.contains(id) {
                continue;
            }
            let mut choices = vec![InboxChoice::new("allow", "Allow", true)];
            if *can_always {
                choices.push(InboxChoice::new("allow_always", "Always allow in this session", false));
            }
            choices.push(InboxChoice::new("deny", "Deny", false));
            out.push(InboxItem {
                id: format!("approval:{sid}:{id}"),
                kind: InboxKind::Approval,
                group: InboxGroup::Approval,
                status: InboxStatus::Open,
                project_id: Some(s.project_id.clone()),
                ticket_id: s
                    .ticket
                    .as_ref()
                    .map(|t| t.id.clone())
                    .or_else(|| step.as_ref().and_then(|a| a.ticket_id.clone())),
                ticket_key: None,
                ticket_title: None,
                attempt_id: s.attempt_id.clone().or_else(|| step.as_ref().and_then(|a| a.attempt_id.clone())),
                session_id: Some(sid.clone()),
                run_id: step.as_ref().map(|a| a.run_id.clone()),
                step_id: s.persona.as_ref().and_then(|p| p.step_id.clone()),
                persona_id: s.persona.as_ref().map(|p| p.id.clone()),
                persona_name: None,
                title: if title.is_empty() { tool.clone() } else { title.clone() },
                body: json!({"requestId": id, "sessionTitle": s.title, "request": e.event, "tool": tool,
                             "description": description, "input": input, "diff": diff}),
                choices,
                resolution: Value::Null,
                created_at: e.ts,
                resolved_at: None,
            });
        }
    }
    Ok(out)
}

fn bad(m: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "cannot_answer", m)
}

async fn load(st: &AppState, id: &str) -> Result<InboxItem, ApiError> {
    let item = st.store.get_inbox_item(id).await?.ok_or_else(|| ApiError::not_found("inbox item"))?;
    if item.status != InboxStatus::Open {
        return Err(bad("this item was already answered"));
    }
    Ok(item)
}

/// `POST /api/inbox/{id}/answer`.
pub(crate) async fn answer(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<InboxAnswer>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(a) = payload?;
    if id.starts_with("approval:") {
        return Err(bad("answer a permission request in its session"));
    }
    if id.starts_with("session:") {
        return Err(bad("answer the session in its tab"));
    }
    let item = load(&st, &id).await?;
    if let Some(c) = &a.choice
        && item.kind != InboxKind::Question
        && !item.choices.iter().any(|x| &x.id == c)
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("{c} is not a choice of this item"),
        ));
    }
    let r = crate::workflow::agent::answer_item(&st, &item, &a).await.map_err(bad)?;
    crate::notifications::kick(&st);
    Ok(Json(r))
}

/// `POST /api/inbox/{id}/dismiss`: closes a question, a secret request or a stalled item without acting (its step keeps
/// waiting), or an unheard notice.
pub(crate) async fn dismiss(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let item = load(&st, &id).await?;
    if !matches!(item.kind, InboxKind::Question | InboxKind::Secret | InboxKind::Stalled | InboxKind::Notice) {
        return Err(bad(
            "only questions, secret requests, stalled steps and notices can be dismissed; answer this one",
        ));
    }
    st.store.end_inbox_item(&id, InboxStatus::Closed, &json!({"dismissed": true}), now_ms()).await?;
    crate::notifications::kick(&st);
    Ok(StatusCode::NO_CONTENT)
}

/// Closes the unheard notices whose cause went away (PDT-139): the attempt ended (merged, or a new one started) or
/// its pull request shows the notice settled (`pdt_core::pr::notice_settled`). `only`: just this attempt's notices
/// of these kinds, closed whatever the pull request shows (an event said so: CI passed, the PR merged). Never fails;
/// problems are logged.
pub(crate) async fn settle_notices(st: &AppState, only: Option<(&str, &[NoticeKind])>) {
    let items = match st.store.open_inbox_items_of(InboxKind::Notice).await {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "could not read the Inbox's notices");
            return;
        }
    };
    let mut closed = false;
    for i in items {
        let Some(attempt) = i.attempt_id.as_deref() else { continue };
        let kind: Option<NoticeKind> = i.body.get("notice").cloned().and_then(|k| serde_json::from_value(k).ok());
        let settled = match only {
            Some((a, kinds)) => a == attempt && kind.is_some_and(|k| kinds.contains(&k)),
            None => match st.store.get_attempt(attempt).await {
                Ok(None) => true,
                Ok(Some(info)) if info.attempt.state != AttemptState::Active => true,
                Ok(Some(_)) => kind.is_some_and(|k| {
                    // Unknown after a restart until the PR check looks again.
                    crate::github::watched_pr(st, attempt).is_some_and(|pr| pdt_core::pr::notice_settled(k, &pr))
                }),
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "could not read a notice's attempt");
                    false
                }
            },
        };
        if !settled {
            continue;
        }
        match st.store.end_inbox_item(&i.id, InboxStatus::Closed, &json!({"settled": true}), now_ms()).await {
            Ok(c) => closed |= c,
            Err(e) => tracing::warn!(item = %i.id, error = %e, "could not close a settled notice"),
        }
    }
    if closed {
        crate::notifications::kick(st);
    }
}
