//! Untrusted input (PDT-201): which sessions read it, the PM's confirmed calls that may let an external ticket in, and
//! the secret tools kept away from it, and the UI token that tells the user's clicks from an agent's curl (PDT-204). The
//! Todo gate itself is pdt-core's `external_gate`, applied by the store. PDT-202: the record of outside input reaching
//! agent sessions (`session.outside_input`, on the ticket's timeline), and what outsiders wrote kept out of them.
//! Prompts fence outside text in the engine (`engine::fence_input`).

use crate::error::{ApiError, codes};
use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use pdt_core::session::Session;
use pdt_core::ticket::{TicketSource, TicketSummary};
use pdt_core::workflow::api::{LogEvent, NewLogEvent, Run};
use pdt_core::workflow::events::{self as ev, TriageCap};
use serde_json::Value;
use std::time::{Duration, Instant};

/// How long the user's yes to a call stays good for that call.
const GRANT_TTL: Duration = Duration::from_secs(10 * 60);

/// The PM's asking-tool calls waiting on the user (`(session, request)` → `(tool, input)`), and the ones the user said
/// yes to, until the call comes in.
#[derive(Default)]
pub(crate) struct Grants {
    pending: std::sync::Mutex<std::collections::HashMap<(String, String), (String, Value)>>,
    granted: std::sync::Mutex<Vec<(String, String, Value, Instant)>>,
}

impl Grants {
    /// A request PDT must not answer for the user: remember it so their yes can be matched to the call.
    pub(crate) fn hold(&self, session_id: &str, request_id: &str, tool: &str, input: &Value) {
        let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.insert((session_id.into(), request_id.into()), (tool.into(), input.clone()));
    }

    /// The user answered a request in the UI: a yes to a held one is good for one call with that input.
    pub(crate) fn answered(&self, session_id: &str, request_id: &str, approved: bool) {
        let held =
            self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&(session_id.into(), request_id.into()));
        if let (true, Some((tool, input))) = (approved, held) {
            let mut g = self.granted.lock().unwrap_or_else(|e| e.into_inner());
            g.retain(|x| x.3.elapsed() < GRANT_TTL);
            g.push((session_id.into(), tool, input, Instant::now()));
        }
    }

    /// Takes the user's yes to this call, if they gave one.
    pub(crate) fn take(&self, session_id: &str, tool: &str, input: &Value) -> bool {
        let mut g = self.granted.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|x| x.3.elapsed() < GRANT_TTL);
        match g.iter().position(|x| x.0 == session_id && x.1 == tool && same_call(&x.2, input)) {
            Some(i) => {
                g.remove(i);
                true
            }
            None => false,
        }
    }
}

/// The same call: equal arguments, leaving out ones given as null.
fn same_call(a: &Value, b: &Value) -> bool {
    let clean = |v: &Value| match v {
        Value::Object(m) => {
            Value::Object(m.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        other => other.clone(),
    };
    clean(a) == clean(b)
}

/// The header PDT's web client sends its UI token in (PDT-204).
pub(crate) const UI_TOKEN_HEADER: &str = "x-ui-token";

/// This pdtd's UI token (PDT-204), new at every start: PDT's web client fetches it and sends it with its changes, so
/// the HTTP API can tell the user's clicks from an agent's curl for the changes that let an external ticket in.
#[derive(Clone)]
pub(crate) struct UiToken(std::sync::Arc<str>);

impl Default for UiToken {
    fn default() -> Self {
        UiToken(uuid::Uuid::new_v4().simple().to_string().into())
    }
}

/// `GET /api/ui/token`: only to a browser's same-origin fetch (it sends `Sec-Fetch-Site`; a plain curl doesn't).
pub(crate) async fn ui_token(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    if headers.get("sec-fetch-site").is_none_or(|v| v.as_bytes() != b"same-origin") {
        return Err(ApiError::forbidden(codes::NOT_THE_UI, "only PDT's web app gets its UI token"));
    }
    Ok(Json(serde_json::json!({ "token": &*st.ui_token.0 })))
}

/// Whether a request came from PDT's web client: it carries this pdtd's UI token. A token from an earlier pdtd (it
/// restarted under the open page) is `403 stale_ui_token`, so the client fetches the new one and tries again.
pub fn from_ui(st: &AppState, headers: &HeaderMap) -> Result<bool, ApiError> {
    match headers.get(UI_TOKEN_HEADER) {
        None => Ok(false),
        Some(v) if v.as_bytes() == st.ui_token.0.as_bytes() => Ok(true),
        Some(_) => Err(ApiError::forbidden(codes::STALE_UI_TOKEN, "the UI token is from an earlier start")),
    }
}

/// The refusal of a change only the user may make to `key`, asked for over the HTTP API but not from PDT's web client
/// (PDT-204).
pub(crate) fn refused(key: &str) -> ApiError {
    ApiError::forbidden(
        codes::UNTRUSTED_TICKET,
        format!(
            "{key} came from outside PDT (an issue or a report), so only the user can put it in To do, start it or \
             label it {}, from PDT's board. Leave it where it is and tell the user why it should go ahead.",
            pdt_core::ticket::AGENTIC_LABEL
        ),
    )
}

/// An external ticket the user hasn't let in yet.
pub(crate) fn held_back(t: &TicketSummary) -> bool {
    t.source == TicketSource::External && !t.admitted
}

/// Whether an edit would let an external ticket in (PDT-201's Todo gate).
pub(crate) fn lets_in(t: &TicketSummary, p: &pdt_core::ticket::ValidPatch) -> bool {
    held_back(t) && pdt_core::ticket::boards(Some(t.stage), p.stage, &t.labels, p.labels.as_deref())
}

/// A change PDT's web client didn't send that would decide a ticket proposal (PDT-366): moving it out of Draft to an
/// open stage, or labelling it agentic. An agent's curl counts as the user otherwise; this refuses it as an agent's.
pub(crate) fn decides_proposal(t: &TicketSummary, p: &pdt_core::ticket::ValidPatch) -> Result<(), ApiError> {
    let boards = pdt_core::ticket::boards(Some(t.stage), p.stage, &t.labels, p.labels.as_deref());
    let moves = p.stage.is_some_and(|s| s != t.stage && !s.is_closed());
    pdt_core::ticket_proposal::agent_gate(t.proposed, pdt_core::ticket::Actor::Agent, boards || moves, &t.key)
        .map_err(|m| ApiError::forbidden(codes::TICKET_PROPOSAL, m))
}

/// Whether a pdt-mcp call (`tool`, `input`) from a session is one PDT never answers for the user (PDT-201): the PM's
/// `edit_ticket` letting an external ticket in.
pub(crate) async fn needs_the_user(st: &AppState, s: &Session, tool: &str, input: &Value) -> bool {
    #[derive(serde::Deserialize)]
    struct Edit {
        ticket: String,
        #[serde(default)]
        stage: Option<pdt_core::ticket::TicketStage>,
        #[serde(default)]
        labels: Option<Vec<String>>,
    }
    if tool != "edit_ticket" {
        return false;
    }
    let Ok(e) = serde_json::from_value::<Edit>(input.clone()) else { return false };
    let Ok(id) = crate::mcp::tools::resolve_ticket(st, &e.ticket, Some(&s.project_id)).await else { return false };
    let Ok(Some(t)) = st.store.get_ticket_summary(&id).await else { return false };
    let labels = match e.labels.as_deref().map(pdt_core::ticket::normalize_labels) {
        Some(Ok(l)) => Some(l),
        Some(Err(_)) => return false,
        None => None,
    };
    lets_in(&t, &pdt_core::ticket::ValidPatch { stage: e.stage, labels, ..Default::default() })
}

/// Whether a session reads untrusted input: it runs a triage persona, or is an agent step of a workflow run that
/// outside input started (a new issue, a webhook).
pub(crate) async fn triaging(st: &AppState, s: &Session) -> bool {
    let Some(p) = &s.persona else { return false };
    if let Ok(Some(persona)) = st.store.get_persona(&p.id).await
        && pdt_core::persona::locked_down(&persona.role)
    {
        return true;
    }
    let Some(run) = crate::workflow::agent::session_run(st, &s.id).await else { return false };
    let Ok(Some(run)) = st.store.get_workflow_run(&run).await else { return false };
    untrusted_run(st, &run).await
}

/// How many sub-workflow levels [`origin`] climbs (calls nest no deeper in practice; a cycle stops here).
const MAX_DEPTH: usize = 32;

/// The run a sub-workflow's run was started for (PDT-415): its `workflow.call` chain's first run, whose trigger is
/// what the input it was called with may hold. None for a run no step called (or whose caller is gone); the outermost
/// run still there when one further up is gone.
pub(crate) async fn origin(st: &AppState, run: &Run) -> Option<Run> {
    let mut parent = run.parent_run_id.clone()?;
    let mut last = None;
    for _ in 0..MAX_DEPTH {
        let Ok(Some(p)) = st.store.get_workflow_run(&parent).await else { break };
        let next = p.parent_run_id.clone();
        last = Some(p);
        match next {
            Some(n) => parent = n,
            None => return last,
        }
    }
    last
}

/// Whether a workflow run was started by outside input: its own trigger's, or for a sub-workflow's run its
/// [`origin`]'s (PDT-415: the input it was called with can carry it).
pub(crate) async fn untrusted_run(st: &AppState, run: &Run) -> bool {
    let root = origin(st, run).await;
    let run = root.as_ref().unwrap_or(run);
    let Some(event) = &run.trigger_event_id else { return false };
    matches!(st.store.get_log_event(event).await, Ok(Some(e)) if pdt_core::workflow::events::untrusted(&e.kind))
}

/// The secret tools a session is refused (PDT-201): one that triages, or whose ticket is external and not let in yet.
pub(crate) async fn refuse_secrets(st: &AppState, s: &Session) -> Result<(), String> {
    if triaging(st, s).await {
        return Err("this session reads input from outside PDT, so it gets no secrets".into());
    }
    if let Some(id) = crate::mcp::tools::session_ticket(st, s).await
        && let Ok(Some(t)) = st.store.get_ticket_summary(&id).await
        && held_back(&t)
    {
        return Err(format!(
            "{} came from outside PDT and the user hasn't moved it to To do yet, so its sessions get no secrets",
            t.key
        ));
    }
    Ok(())
}

/// Where a trigger's outside text came from (PDT-202), with a link: `owner/name#12`, `webhook <name>`, a Jira key.
/// None for a trigger whose event carries none.
pub(crate) fn trigger_source(reg: &crate::integrations::Registry, trigger: &Value) -> Option<(String, Option<String>)> {
    let kind = trigger.get("type").and_then(Value::as_str)?;
    if reg.event_type(kind).is_none_or(|t| t.outside.is_empty()) || pdt_core::workflow::events::by_user(trigger) {
        return None;
    }
    let s = |k: &str| trigger.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(String::from);
    let url = s("url");
    let source = match (s("repo"), trigger.get("number").and_then(Value::as_u64), s("hook"), s("key")) {
        (Some(repo), Some(n), _, _) => format!("{repo}#{n}"),
        (_, _, Some(hook), _) => format!("webhook {hook}"),
        (_, _, _, Some(key)) => key,
        _ => url.clone().unwrap_or_else(|| kind.to_string()),
    };
    Some((source, url))
}

/// [`trigger_source`] of the trigger a run's outside text came from: its own, or its [`origin`]'s for a sub-workflow's
/// run (PDT-415).
pub(crate) async fn run_source(st: &AppState, run: &Run) -> Option<(String, Option<String>)> {
    let root = origin(st, run).await;
    trigger_source(&st.registry(), &root.as_ref().unwrap_or(run).trigger)
}

/// Where an external ticket came from (its first link to another tracker), when it is one.
pub(crate) fn ticket_source(t: &TicketSummary) -> Option<(String, Option<String>)> {
    if t.source != TicketSource::External {
        return None;
    }
    Some(match t.external.first() {
        Some(l) => (l.key.clone(), Some(l.url.clone()).filter(|u| !u.is_empty())),
        None => (format!("{} (made from outside input)", t.key), None),
    })
}

/// Records that outside input went into an agent session (PDT-202): a `session.outside_input` event, and an entry on
/// the ticket's timeline when there is a ticket.
pub(crate) async fn record_input(st: &AppState, e: pdt_core::workflow::events::OutsideInput) {
    use pdt_core::ticket::{Actor, EventKind};
    use pdt_core::workflow::events::OUTSIDE_INPUT;
    let now = crate::routes::now_ms();
    let key = format!("{}:{}:{}:{}", e.session_id, e.via, e.source, e.ticket_id.as_deref().unwrap_or_default());
    let key = match &e.run_id {
        Some(r) => format!("{key}:{r}:{now}"),
        None => format!("{key}:{now}"),
    };
    tracing::info!(session = %e.session_id, source = %e.source, via = %e.via, "outside input reached an agent session");
    let log = NewLogEvent::new(OUTSIDE_INPUT, key, "pdt", &e);
    if let Err(x) = st.store.emit_event(&log, now).await {
        tracing::warn!(session = %e.session_id, error = %x, "could not log outside input");
    }
    if let Some(ticket) = &e.ticket_id {
        let data = serde_json::json!({
            "source": e.source, "url": e.url, "sessionId": e.session_id, "via": e.via, "runId": e.run_id,
            "workflowName": e.workflow_name,
        });
        if let Err(x) =
            st.store.add_ticket_event(ticket, EventKind::OutsideInput, Actor::System, None, Some(data), now).await
        {
            tracing::warn!(ticket = %ticket, error = %x, "could not put outside input on the ticket's timeline");
        }
    }
}

/// [`record_input`] for the sessions a workflow run's agent steps triaged its outside input in, onto the ticket the
/// run made from it (`ticket.create`, `github.import`).
pub(crate) async fn record_run_on_ticket(st: &AppState, run: &Run, ticket_id: &str) {
    let Some((source, url)) = run_source(st, run).await else { return };
    let mut seen = std::collections::HashSet::new();
    for a in st.store.agent_steps_of_run(&run.id).await.unwrap_or_default() {
        let Some(session) = a.session_id.filter(|s| seen.insert(s.clone())) else { continue };
        record_input(
            st,
            pdt_core::workflow::events::OutsideInput {
                project_id: run.project_id.clone(),
                ticket_id: Some(ticket_id.to_string()),
                session_id: session,
                source: source.clone(),
                url: url.clone(),
                via: "triage".into(),
                run_id: Some(run.id.clone()),
                workflow_name: Some(run.workflow_name.clone()),
            },
        )
        .await;
    }
}

/// [`record_input`] for a triage session that made a ticket itself (`create_ticket`): the input it read, onto that
/// ticket. Its source is its run's trigger, else the session's persona says only that it triages.
pub(crate) async fn record_triage(st: &AppState, s: &Session, ticket_id: &str) {
    let run = match crate::workflow::agent::session_run(st, &s.id).await {
        Some(id) => st.store.get_workflow_run(&id).await.ok().flatten(),
        None => None,
    };
    let source = match &run {
        Some(r) => run_source(st, r).await,
        None => None,
    };
    let (source, url) = source.unwrap_or_else(|| ("input from outside PDT".into(), None));
    let e = pdt_core::workflow::events::OutsideInput {
        project_id: Some(s.project_id.clone()),
        ticket_id: Some(ticket_id.to_string()),
        session_id: s.id.clone(),
        source,
        url,
        via: "triage".into(),
        run_id: run.as_ref().map(|r| r.id.clone()),
        workflow_name: run.map(|r| r.workflow_name),
    };
    record_input(st, e).await;
}

const HOUR_MS: i64 = 60 * 60 * 1000;

/// The triage cap a new issue, outside pull request (or webhook) is over (PDT-202): its author already started
/// `triage_per_author_hour` triage runs in the last hour, or its repository (webhook) `triage_per_repo_hour`. None when
/// it may be triaged. Counted in issues: one that started runs of several workflows is one.
pub(crate) async fn triage_cap(st: &AppState, e: &LogEvent, trigger: &Value) -> Option<(TriageCap, u32)> {
    let h = st.store.get_host_settings().await.unwrap_or_default();
    let since = crate::routes::now_ms() - HOUR_MS;
    let s = |k: &str| trigger.get(k).and_then(Value::as_str).filter(|v| !v.is_empty());
    let from_repo = e.kind == ev::GITHUB_ISSUE_OPENED || ev::GITHUB_PR_EVENTS.contains(&e.kind.as_str());
    let repo_field = if from_repo { "repo" } else { "hook" };
    let checks = [
        (TriageCap::Author, "author", s("author"), h.triage_per_author_hour),
        (TriageCap::Repo, repo_field, s(repo_field), h.triage_per_repo_hour),
    ];
    for (cap, field, value, limit) in checks {
        let Some(value) = value.filter(|_| limit > 0) else { continue };
        match st.store.events_with_runs_since(&e.kind, field, value, since, &e.id).await {
            Ok(n) if n >= limit => return Some((cap, limit)),
            Ok(_) => {}
            Err(x) => tracing::warn!(error = %x, "could not count the triage runs"),
        }
    }
    None
}

/// A triage run over the cap is not started (PDT-202): a `triage.skipped` event, and the hour's summary for the
/// repository in the Inbox.
pub(crate) async fn skip_triage(
    st: &AppState,
    wf: &pdt_core::workflow::api::Workflow,
    e: &LogEvent,
    trigger: &Value,
    (cap, limit): (TriageCap, u32),
) {
    let now = crate::routes::now_ms();
    let s = |k: &str| trigger.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(String::from);
    let repo = match (s("repo"), s("hook")) {
        (Some(r), _) => r,
        (None, Some(h)) => format!("webhook {h}"),
        _ => e.kind.clone(),
    };
    let skipped = ev::TriageSkipped {
        project_id: e.project_id.clone(),
        repo: repo.clone(),
        number: trigger.get("number").and_then(Value::as_u64),
        url: s("url"),
        author: s("author"),
        workflow_id: wf.id.clone(),
        workflow_name: wf.name.clone(),
        event_id: e.id.clone(),
        cap,
        limit,
    };
    tracing::info!(workflow = %wf.name, repo = %repo, author = ?skipped.author, ?cap, limit, "triage skipped: over the hourly cap");
    let log = NewLogEvent::new(ev::TRIAGE_SKIPPED, format!("{}:{}", e.id, wf.id), "engine", &skipped);
    match st.store.emit_event(&log, now).await {
        Ok(Some(_)) => {}
        Ok(None) => return,
        Err(x) => return tracing::warn!(error = %x, "could not log a skipped triage"),
    }
    // One summary per repository and hour, listing what it skipped.
    let slot = now - now.rem_euclid(HOUR_MS);
    let filter = pdt_store::EventFilter {
        kind: Some(ev::TRIAGE_SKIPPED.into()),
        since: Some(slot),
        limit: 1000,
        ..Default::default()
    };
    let all: Vec<ev::TriageSkipped> = st
        .store
        .list_log_events(&filter)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|x| serde_json::from_value::<ev::TriageSkipped>(x.payload).ok())
        .filter(|x| x.repo == repo)
        .collect();
    let lines: Vec<String> = all
        .iter()
        .map(|x| {
            let what = match (&x.url, x.number) {
                (Some(u), Some(n)) => format!("[#{n}]({u})"),
                (None, Some(n)) => format!("#{n}"),
                (Some(u), None) => u.clone(),
                (None, None) => "a delivery".into(),
            };
            let why = match x.cap {
                TriageCap::Author => {
                    format!("{} reached {} an hour", x.author.as_deref().unwrap_or("its author"), x.limit)
                }
                TriageCap::Repo => format!("the repository reached {} an hour", x.limit),
            };
            format!("- {what} for {}: {why}", x.workflow_name)
        })
        .collect();
    let n = all.len().max(1);
    let title = format!("Triage skipped {n} {} on {repo} (over the hourly cap)", if n == 1 { "run" } else { "runs" });
    let text = format!(
        "{title}. They were not triaged: run them by hand from the issue if they need it, or raise the caps in \
         Settings → Host.\n\n{}",
        lines.join("\n")
    );
    let key = format!("triage-cap:{repo}:{slot}");
    let item = pdt_core::inbox::InboxItem {
        id: uuid::Uuid::now_v7().to_string(),
        kind: pdt_core::inbox::InboxKind::Notice,
        group: pdt_core::inbox::InboxGroup::Notice,
        status: pdt_core::inbox::InboxStatus::Open,
        project_id: e.project_id.clone(),
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
        body: serde_json::json!({"notice": null, "key": key, "text": text, "workflow": wf.name}),
        choices: vec![],
        resolution: Value::Null,
        created_at: now,
        resolved_at: None,
    };
    match st.store.put_inbox_summary(&item, &key).await {
        Ok(_) => crate::notifications::kick(st),
        Err(x) => tracing::warn!(error = %x, "could not sum up skipped triage in the Inbox"),
    }
}

/// Why a step of a run started by what an outsider wrote (a pull request comment or review by someone who isn't a
/// collaborator on the repository, PDT-202) sends nothing into an agent session; None for other runs. A sub-workflow's
/// run goes by its `origin`'s trigger too (PDT-415): the input it was called with may be what the outsider wrote.
pub(crate) fn outsider_refusal(ctx: &Value, origin: Option<&Value>) -> Option<String> {
    let by_outsider = pdt_core::workflow::events::by_outsider;
    let t = origin.filter(|o| by_outsider(o)).or_else(|| ctx.get("trigger"))?;
    by_outsider(t).then(|| {
        let who = t.get("author").and_then(Value::as_str).unwrap_or("someone");
        format!(
            "{who} isn't a collaborator on the repository, so what they wrote is shown in PDT and never sent into an \
             agent session"
        )
    })
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new().route("/ui/token", get(ui_token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_yes_is_good_for_one_matching_call() {
        let g = Grants::default();
        let input = json!({"ticket": "P-1", "stage": "todo"});
        g.hold("s", "r1", "edit_ticket", &input);
        // Nothing until the user answers.
        assert!(!g.take("s", "edit_ticket", &input));
        g.answered("s", "r1", true);
        assert!(!g.take("other", "edit_ticket", &input), "another session's");
        assert!(!g.take("s", "edit_ticket", &json!({"ticket": "P-2", "stage": "todo"})), "another call");
        assert!(g.take("s", "edit_ticket", &json!({"ticket": "P-1", "stage": "todo", "title": null})));
        assert!(!g.take("s", "edit_ticket", &input), "used up");
        // A no grants nothing; an answer to a request PDT didn't hold neither.
        g.hold("s", "r2", "edit_ticket", &input);
        g.answered("s", "r2", false);
        g.answered("s", "r3", true);
        assert!(!g.take("s", "edit_ticket", &input));
    }

    #[test]
    fn outside_input_names_its_source() {
        let reg = crate::integrations::test_registry();
        let issue = json!({"type": "github.issue.opened", "repo": "o/r", "number": 12, "url": "https://x/12"});
        assert_eq!(trigger_source(reg, &issue), Some(("o/r#12".into(), Some("https://x/12".into()))));
        let hook = json!({"type": "webhook.received", "hook": "deploys", "body": {}});
        assert_eq!(trigger_source(reg, &hook), Some(("webhook deploys".into(), None)));
        assert_eq!(trigger_source(reg, &json!({"type": "ci.run_failed", "url": "u"})), None, "not outside text");
        assert_eq!(trigger_source(reg, &json!({"repo": "o/r"})), None);
    }

    #[test]
    fn what_outsiders_wrote_is_refused() {
        let by = |outside: bool| json!({"trigger": {"type": "pr.commented", "author": "eve", "outside": outside}});
        assert!(outsider_refusal(&by(true), None).is_some_and(|r| r.starts_with("eve isn't a collaborator")));
        assert_eq!(outsider_refusal(&by(false), None), None);
        assert_eq!(outsider_refusal(&json!({"trigger": {"type": "pr.commented"}}), None), None, "older events");
        // A sub-workflow's run goes by the trigger of the run that called it.
        let child = json!({"trigger": {"type": "manual", "text": "do it"}});
        let origin = by(true)["trigger"].clone();
        assert!(outsider_refusal(&child, Some(&origin)).is_some_and(|r| r.starts_with("eve isn't")));
        assert_eq!(outsider_refusal(&child, Some(&by(false)["trigger"])), None);
    }
}
