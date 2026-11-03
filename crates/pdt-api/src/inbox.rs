//! The Inbox API: what waits on the user. Stored items (questions, gates, stalled steps, loop
//! limits) plus the open permission requests of live conversations (approvals), most urgent first.

use crate::error::{ApiError, OrNotFound, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use pdt_core::conversation::{AgentEvent, RequestState};
use pdt_core::inbox::{
    self, ChangeRequestLink, InboxAnswer, InboxChoice, InboxGroup, InboxItem, InboxKind, InboxList, InboxStatus,
};
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
    items.extend(crate::mcp_oauth::inbox_items(&st).await);
    items.extend(storage(&st));
    items.extend(crate::storage::low_disk(&st).await);
    items.extend(proposals(&st, q.project.as_deref()).await?);
    decorate(&st, &mut items).await?;
    inbox::sort(&mut items);
    let total = u32::try_from(items.len()).unwrap_or(u32::MAX);
    Ok(Json(InboxList { items, total }))
}

/// What [`decorate`] reads of an item's ticket.
struct TicketFacts {
    key: String,
    title: String,
    /// Its active (else latest) attempt.
    attempt_id: Option<String>,
    /// Its external links' URLs.
    external: Vec<String>,
}

/// Ticket keys and titles, persona names, and a gate's pull request (PDT-306).
pub(crate) async fn decorate(st: &AppState, items: &mut [InboxItem]) -> Result<(), ApiError> {
    let mut tickets: HashMap<String, Option<TicketFacts>> = HashMap::new();
    let mut personas: HashMap<String, Option<String>> = HashMap::new();
    for i in items.iter_mut() {
        if let Some(t) = &i.ticket_id {
            if !tickets.contains_key(t) {
                let found = st.store.get_ticket(t).await?.map(|x| TicketFacts {
                    key: x.summary.key.clone(),
                    title: x.summary.title.clone(),
                    attempt_id: x.summary.attempt.as_ref().map(|a| a.id.clone()),
                    external: x.summary.external.iter().map(|e| e.url.clone()).collect(),
                });
                tickets.insert(t.clone(), found);
            }
            if let Some(Some(f)) = tickets.get(t) {
                i.ticket_key = Some(f.key.clone());
                i.ticket_title = Some(f.title.clone());
            }
        }
        if let Some(p) = &i.persona_id {
            if !personas.contains_key(p) {
                personas.insert(p.clone(), st.store.get_persona(p).await?.map(|x| x.name));
            }
            i.persona_name = personas.get(p).cloned().flatten();
        }
        if i.kind == InboxKind::Gate {
            // A gate opened before PDT-306 has its whole question as its title.
            if i.title.contains('\n') || i.title.chars().count() > inbox::TITLE_CHARS {
                let node = i.body.get("node").and_then(Value::as_str).unwrap_or_default();
                i.title = inbox::gate_title(node, &i.title);
            }
            let ticket = i.ticket_id.as_ref().and_then(|t| tickets.get(t)).and_then(Option::as_ref);
            i.change_request = change_request(st, i, ticket).await?;
        }
    }
    Ok(())
}

/// The pull request a gate is about: its attempt's (else its ticket's attempt's), else the one its run's trigger or
/// its ticket's external link names, when a forge in use says it is one of its change requests.
async fn change_request(
    st: &AppState,
    item: &InboxItem,
    ticket: Option<&TicketFacts>,
) -> Result<Option<ChangeRequestLink>, ApiError> {
    let plugins = st.forge_plugins();
    let plugin_of = |url: &str| {
        let host = pdt_core::git_url::parse_remote(url)?.host;
        plugins.iter().find(|p| p.claims_host(&host)).cloned()
    };
    let link = |url: &str, name: Option<&str>| ChangeRequestLink {
        url: url.to_string(),
        name: name.unwrap_or("pull request").to_string(),
    };
    let attempt = item.attempt_id.as_deref().or(ticket.and_then(|t| t.attempt_id.as_deref()));
    // The local forge's change request has no page to open.
    if let Some(pr) = attempt.and_then(|a| crate::github::attempt_pr(st, a)).filter(|p| !p.local && !p.url.is_empty()) {
        return Ok(Some(link(&pr.url, plugin_of(&pr.url).map(|p| p.change_request_name()))));
    }
    let mut urls = vec![];
    if let Some(run) = &item.run_id
        && let Some(r) = st.store.get_workflow_run(run).await?
        && let Some(u) = r.trigger.get("url").and_then(Value::as_str)
    {
        urls.push(u.to_string());
    }
    urls.extend(ticket.map(|t| t.external.clone()).unwrap_or_default());
    Ok(urls.iter().find_map(|u| {
        let p = plugin_of(u).filter(|p| p.is_change_request_url(u))?;
        Some(link(u, Some(p.change_request_name())))
    }))
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
            change_request: None,
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

/// "Connect GitHub" (PDT-177) while a forge in use (PDT-241: one item per forge plugin) has no login and something
/// (the PR poller, a merge, an agent's tool) wanted it; "Reconnect GitHub" while the forge refuses the login it has
/// (PDT-212). Not stored: one item per forge, which goes once the user connects.
pub(crate) fn connect_forge(st: &AppState) -> Vec<InboxItem> {
    st.forge_plugins()
        .iter()
        .flat_map(|p| {
            crate::forge::signed_forges(p.as_ref())
                .into_iter()
                .filter_map(|(host, f)| connect_item(p.kind().as_str(), p.label(), host.as_deref(), f.as_ref()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// PDT can't save changes (PDT-323): one item while writes fail, with what is free on the disk with PDT's data when
/// it is full. Not stored, as nothing can be; it goes once a write works again.
pub(crate) fn storage(st: &AppState) -> Option<InboxItem> {
    let f = st.store.write_failure()?;
    let full = pdt_store::is_disk_full(&f.message);
    // The data folder, or the nearest folder above it that exists.
    let free = full.then(|| st.data_dir.ancestors().find_map(|d| fs4::available_space(d).ok())).flatten();
    Some(inbox::storage_item(full, free, f.since))
}

/// `host`: an account on another host than the forge's public one (PDT-244), with an item of its own.
fn connect_item(kind: &str, label: &str, host: Option<&str>, forge: &dyn pdt_forge::Forge) -> Option<InboxItem> {
    if !forge.login_needed() {
        return None;
    }
    let rejected = forge.login_rejected_at();
    let on = host.map(|h| format!(" on {h}")).unwrap_or_default();
    let (title, mut body) = match rejected {
        Some(_) => (
            format!("Reconnect {label}{on}"),
            json!({"forge": kind, "reason": "rejected", "text": format!("{label}{on} no longer accepts PDT's login (the \
token was revoked or expired). Until you connect again, PDT has stopped calling {label}{on}: it can't follow pull \
requests or CI, merge, or let agents read CI logs. Connect {label} again in Settings › Plugins › {label}.")}),
        ),
        None => (
            format!("Connect {label}"),
            json!({"forge": kind, "text": format!("PDT reads pull requests, CI and issues, merges and comments on \
{label} with your {label} login, and it has none: until you connect, it can't follow pull requests or CI, merge, or \
let agents read CI logs. Connect {label} in Settings › Plugins › {label}.")}),
        ),
    };
    if let Some(h) = host {
        body["host"] = json!(h);
    }
    Some(InboxItem {
        id: host.map_or_else(|| format!("connect:{kind}"), |h| format!("connect:{kind}:{h}")),
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
        change_request: None,
        title,
        body,
        choices: vec![],
        resolution: Value::Null,
        created_at: rejected.unwrap_or_else(now_ms),
        resolved_at: None,
    })
}

/// Workflow versions agents proposed (PDT-320), one item per workflow: who proposed it, their note and what changed,
/// with Accept and Discard. Not stored: the item goes once the proposal is accepted, discarded or replaced. With a
/// project, the workflows that apply to it.
pub(crate) async fn proposals(st: &AppState, project: Option<&str>) -> Result<Vec<InboxItem>, ApiError> {
    let mut out = vec![];
    for w in st.store.list_workflows(project).await? {
        let Some(v) = w.proposed else { continue };
        let Some(version) = st.store.list_workflow_versions(&w.id).await?.into_iter().find(|x| x.version == v) else {
            continue;
        };
        let (Some(current), Some(proposed)) =
            (st.store.workflow_doc(&w.id, w.version).await?, st.store.workflow_doc(&w.id, v).await?)
        else {
            continue;
        };
        let summary = pdt_core::workflow::diff::diff(&current, &proposed, "", "").summary();
        let by = version.proposed_by.clone().unwrap_or_else(|| "An agent".into());
        out.push(InboxItem {
            id: inbox::proposal_id(&w.id, v),
            kind: InboxKind::Proposal,
            group: InboxGroup::Proposal,
            status: InboxStatus::Open,
            project_id: w.project_id.clone(),
            ticket_id: None,
            ticket_key: None,
            ticket_title: None,
            attempt_id: None,
            session_id: None,
            run_id: None,
            step_id: None,
            persona_id: None,
            persona_name: None,
            change_request: None,
            title: format!("{by} proposed version {v} of {}", w.name),
            body: json!({"workflowId": w.id, "workflow": w.name, "version": v, "current": w.version,
                         "by": version.proposed_by, "note": version.note, "summary": summary, "enabled": w.enabled}),
            choices: vec![
                InboxChoice::new(inbox::DISCARD, "Discard", false),
                InboxChoice::new(inbox::ACCEPT, "Accept", true),
            ],
            resolution: Value::Null,
            created_at: version.created_at,
            resolved_at: None,
        });
    }
    Ok(out)
}

/// Accept or Discard on a workflow proposal's item (PDT-320): the same as the workflow's Versions tab.
async fn answer_proposal(st: &AppState, id: &str, choice: Option<&str>) -> Result<Json<Value>, ApiError> {
    use crate::workflow::routes;
    let (workflow, version) = inbox::parse_proposal_id(id).or_404("inbox item")?;
    let path = Path((workflow.to_string(), version));
    let Json(d) = match choice {
        Some(inbox::ACCEPT) => routes::accept(State(st.clone()), path).await?,
        Some(inbox::DISCARD) => routes::discard(State(st.clone()), path).await?,
        _ => return Err(ApiError::conflict(codes::CANNOT_ANSWER, "choose Accept or Discard")),
    };
    crate::notifications::kick(st);
    Ok(Json(json!({"workflowId": workflow, "version": d.workflow.version, "accepted": choice == Some(inbox::ACCEPT)})))
}

/// Whether an Inbox item is still open (PDT-368: an escalation): a stored one by its status, a listed one (an
/// approval, a proposal, a sign-in) while it is still listed.
pub(crate) async fn is_open(st: &AppState, id: &str) -> Result<bool, ApiError> {
    if let Some(i) = st.store.get_inbox_item(id).await? {
        return Ok(i.status == pdt_core::inbox::InboxStatus::Open);
    }
    let mut listed = approvals(st, None).await?;
    listed.extend(proposals(st, None).await?);
    listed.extend(connect_forge(st));
    listed.extend(crate::mcp_oauth::inbox_items(st).await);
    Ok(listed.iter().any(|i| i.id == id))
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
                change_request: None,
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

async fn load(st: &AppState, id: &str) -> Result<InboxItem, ApiError> {
    let item = st.store.get_inbox_item(id).await?.or_404("inbox item")?;
    if item.status != InboxStatus::Open {
        return Err(ApiError::conflict(codes::CANNOT_ANSWER, "this item was already answered"));
    }
    Ok(item)
}

/// `POST /api/inbox/{id}/answer`.
pub(crate) async fn answer(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<InboxAnswer>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(a) = payload?;
    if id.starts_with("approval:") {
        return Err(ApiError::conflict(codes::CANNOT_ANSWER, "answer a permission request in its session"));
    }
    if id.starts_with("session:") {
        return Err(ApiError::conflict(codes::CANNOT_ANSWER, "answer the session in its tab"));
    }
    let by = if crate::untrusted::from_ui(&st, &headers)? { "user" } else { "agent" };
    if id.starts_with("proposal:") {
        let item = proposals(&st, None).await?.into_iter().find(|i| i.id == id);
        let r = answer_proposal(&st, &id, a.choice.as_deref()).await?;
        if let Some(item) = item {
            crate::workflow::sensors::inbox_answered(&st, &item, a.choice.as_deref(), None, by).await;
        }
        return Ok(r);
    }
    let item = load(&st, &id).await?;
    if let Some(c) = &a.choice
        && item.kind != InboxKind::Question
        && !item.choices.iter().any(|x| &x.id == c)
    {
        return Err(ApiError::invalid(format!("{c} is not a choice of this item")));
    }
    let r = match (item.kind, a.choice.as_deref()) {
        (InboxKind::Admit, Some(c)) => {
            let from_ui = crate::untrusted::from_ui(&st, &headers)?;
            crate::workflow::nodes::inbox_admit::answer(&st, &item, c, from_ui)
                .await
                .map_err(|m| ApiError::conflict(codes::CANNOT_ANSWER, m))?
        }
        (InboxKind::Admit, None) => return Err(ApiError::conflict(codes::CANNOT_ANSWER, "choose Approve or Reject")),
        (InboxKind::TicketProposal, _) => {
            let from_ui = crate::untrusted::from_ui(&st, &headers)?;
            crate::workflow::nodes::ticket_proposal::answer(&st, &item, &a, from_ui)
                .await
                .map_err(|m| ApiError::conflict(codes::CANNOT_ANSWER, m))?
        }
        // An agent can't allow its own call (PDT-256): only the user's click in PDT does.
        (InboxKind::Confirm, Some(c)) => {
            if c == inbox::ALLOW && !crate::untrusted::from_ui(&st, &headers)? {
                return Err(ApiError::conflict(
                    codes::CANNOT_ANSWER,
                    "only the user can allow this call, from PDT's Inbox",
                ));
            }
            crate::workflow::agent::answer_confirm(&st, &item, c, a.rows.as_deref())
                .await
                .map_err(|m| ApiError::conflict(codes::CANNOT_ANSWER, m))?
        }
        (InboxKind::Confirm, None) => return Err(ApiError::conflict(codes::CANNOT_ANSWER, "choose Allow or Deny")),
        _ => crate::workflow::agent::answer_item(&st, &item, &a)
            .await
            .map_err(|m| ApiError::conflict(codes::CANNOT_ANSWER, m))?,
    };
    // What was written (never a secret's value).
    let text = a.text.clone().or_else(|| a.answers.as_ref().map(|all| all.join("\n")));
    crate::workflow::sensors::inbox_answered(&st, &item, a.choice.as_deref(), text.as_deref(), by).await;
    crate::notifications::kick(&st);
    Ok(Json(r))
}

/// `POST /api/inbox/{id}/dismiss`: closes a question, a secret request or a stalled item without acting (its step keeps
/// waiting), or an unheard notice.
pub(crate) async fn dismiss(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let item = load(&st, &id).await?;
    if !matches!(item.kind, InboxKind::Question | InboxKind::Secret | InboxKind::Stalled | InboxKind::Notice) {
        return Err(ApiError::conflict(
            codes::CANNOT_ANSWER,
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

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/inbox", get(list))
        .route("/inbox/{id}/answer", post(answer))
        .route("/inbox/{id}/dismiss", post(dismiss))
}
