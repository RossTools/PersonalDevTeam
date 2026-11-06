//! The PM's reading tools: the overview, tickets, sessions and workflow runs.

use super::*;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetOverview {
    /// A project's key (`PDT`) or id; every project when left out.
    #[serde(default)]
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SearchTickets {
    /// Words (prefix-matched in titles and descriptions) or a key (`ABC-12`).
    #[serde(default)]
    query: Option<String>,
    /// A project's key or id; every project when left out.
    #[serde(default)]
    project: Option<String>,
    /// Stages, comma-separated: draft, ready, todo, in_progress, done, wont_do. Open ones when left out.
    #[serde(default)]
    stage: Option<String>,
    /// At most this many (default 20, at most 100).
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetSession {
    /// The session's id.
    session: String,
    /// How many of its last messages to include (default 10, at most 50).
    #[serde(default)]
    messages: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListWorkflowRuns {
    /// A workflow's id.
    #[serde(default)]
    workflow: Option<String>,
    /// A ticket's key or id.
    #[serde(default)]
    ticket: Option<String>,
    /// running, waiting, completed, failed or cancelled.
    #[serde(default)]
    status: Option<pdt_core::workflow::api::RunStatus>,
    /// At most this many, newest first (default 10, at most 50).
    #[serde(default)]
    limit: Option<i64>,
}

pub(super) fn defs() -> Vec<Tool> {
    vec![
        Tool {
            decl: ToolDecl::new(
                "get_overview",
                "See every project's board, tickets in flight, the Inbox and running sessions",
            )
            .read_only()
            .pm(),
            title: "What's going on",
            description: "Every project (or one): open tickets per stage, tickets in progress with their branch, PR, CI \
                          and who has them, tickets waiting out the start delay before auto-start takes them, the Inbox's open \
                          items, and the agent sessions running now.",
            schema: schema::<GetOverview>,
        },
        Tool {
            decl: ToolDecl::new("search_tickets", "Search tickets in every project").read_only().pm(),
            title: "Search tickets",
            description: "Tickets matching words or a key, in every project or one, by stage: key, title, stage, \
                          priority, labels and state. get_ticket reads one in full.",
            schema: schema::<SearchTickets>,
        },
        Tool {
            decl: ToolDecl::new("get_session", "Read a session's state and last messages").read_only().pm(),
            title: "Get a session",
            description: "An agent session's state: agent, status, activity, ticket, persona and workflow step, its \
                          open requests, and its last messages.",
            schema: schema::<GetSession>,
        },
        Tool {
            decl: ToolDecl::new("list_workflow_runs", "List workflow runs and their steps").read_only().pm(),
            title: "List workflow runs",
            description: "Workflow runs (of a workflow or a ticket), newest first, with each step's status, port and \
                          error.",
            schema: schema::<ListWorkflowRuns>,
        },
    ]
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
        "get_overview" => {
            let a: GetOverview = args(a)?;
            overview(cx, a.project.as_deref()).await
        }
        "search_tickets" => {
            let a: SearchTickets = args(a)?;
            let project = match a.project.as_deref().filter(|p| !p.trim().is_empty()) {
                Some(p) => Some(crate::pm::project_id(st, p).await?),
                None => None,
            };
            let q = pdt_core::ticket::TicketQuery {
                project,
                q: a.query,
                stage: Some(a.stage.unwrap_or_else(|| "draft,ready,todo,in_progress".into())),
                limit: Some(a.limit.unwrap_or(20).clamp(1, 100)),
                ..Default::default()
            };
            let Json(page) = crate::tickets::list(State(st.clone()), Ok(axum::extract::Query(q))).await.map_err(api)?;
            let tickets: Vec<Value> = page
                .items
                .iter()
                .map(|t| {
                    let mut v = json!({"key": t.key, "id": t.id, "title": t.title, "stage": t.stage,
                                       "priority": t.priority, "labels": t.labels,
                                       "state": pdt_core::pm::ticket_state(t)});
                    if let Some(at) = t.auto_start_at {
                        v["autoStartsInSeconds"] = json!(starts_in(at));
                    }
                    v
                })
                .collect();
            Ok(json!({"tickets": tickets, "total": page.total}))
        }
        "get_session" => {
            let a: GetSession = args(a)?;
            session_state(cx, &a.session, a.messages.unwrap_or(10).clamp(1, 50)).await
        }
        "list_workflow_runs" => {
            let a: ListWorkflowRuns = args(a)?;
            let ticket_id = match a.ticket.as_deref() {
                Some(t) => Some(resolve_ticket(st, t, Some(&cx.session.project_id)).await?),
                None => None,
            };
            let f = pdt_store::RunFilter {
                workflow_id: a.workflow,
                ticket_id,
                status: a.status,
                skip_quiet: false,
                limit: a.limit.unwrap_or(10).clamp(1, 50),
            };
            let mut runs = vec![];
            for r in st.store.list_workflow_runs(&f).await.map_err(store)? {
                let steps: Vec<Value> = st
                    .store
                    .list_steps(&r.id)
                    .await
                    .map_err(store)?
                    .into_iter()
                    .map(|s| {
                        json!({"node": s.node_id, "kind": s.kind, "status": s.status, "port": s.port,
                                    "error": s.error})
                    })
                    .collect();
                runs.push(json!({"id": r.id, "workflow": r.workflow_name, "workflowId": r.workflow_id,
                                 "status": r.status, "error": r.error, "ticketId": r.ticket_id,
                                 "startedAt": r.started_at, "endedAt": r.ended_at, "steps": steps}));
            }
            Ok(json!({"runs": runs}))
        }
        other => Err(format!("no tool is called {other}")),
    }
}

/// The overview (`get_overview`): projects with their open tickets per stage, tickets in progress, the Inbox, and
/// running agent sessions.
async fn overview(cx: &Cx<'_>, project: Option<&str>) -> Result<Value, String> {
    let st = cx.st;
    let only = match project.filter(|p| !p.trim().is_empty()) {
        Some(p) => Some(crate::pm::project_id(st, p).await?),
        None => None,
    };
    let projects: Vec<pdt_core::project::Project> = st
        .store
        .list_projects()
        .await
        .map_err(store)?
        .into_iter()
        .filter(|p| only.as_ref().is_none_or(|o| *o == p.id))
        .collect();
    let keys: std::collections::HashMap<String, String> =
        projects.iter().map(|p| (p.id.clone(), p.key.clone())).collect();
    let mut out_projects = vec![];
    for p in &projects {
        let stages: serde_json::Map<String, Value> =
            crate::pm::stage_counts(st, &p.id).await.map_err(api)?.into_iter().map(|(s, n)| (s, json!(n))).collect();
        let pm_acts = st.store.get_project_settings(&p.id).await.map_err(store)?.policy.pm_acts;
        out_projects.push(json!({"key": p.key, "name": p.name, "id": p.id, "kind": p.kind, "openTickets": stages,
                                 "pmActsWithoutAsking": pm_acts}));
    }
    let in_flight: Vec<Value> = crate::pm::in_flight(st, only.as_deref())
        .await
        .map_err(api)?
        .iter()
        .map(|t| {
            json!({"key": t.key, "id": t.id, "title": t.title, "project": keys.get(&t.project_id),
                   "state": pdt_core::pm::ticket_state(t), "attemptId": t.attempt.as_ref().map(|a| &a.id)})
        })
        .collect();
    let Json(inbox) =
        crate::inbox::list(State(st.clone()), axum::extract::Query(Default::default())).await.map_err(api)?;
    let inbox: Vec<Value> = inbox
        .items
        .iter()
        .filter(|i| only.is_none() || i.project_id == only)
        .map(|i| {
            json!({"id": i.id, "group": i.group, "ticket": i.ticket_key, "title": i.title, "sessionId": i.session_id,
                   "persona": i.persona_name, "at": i.created_at})
        })
        .collect();
    let sessions: Vec<Value> = st
        .store
        .list_sessions_by_status(pdt_core::session::SessionStatus::Running)
        .await
        .map_err(store)?
        .into_iter()
        .filter(|s| s.kind == pdt_core::session::SessionKind::AgentTui && s.id != cx.session.id)
        .filter(|s| keys.contains_key(&s.project_id))
        .map(|s| {
            json!({"id": s.id, "title": s.title, "agent": s.agent, "project": keys.get(&s.project_id),
                   "ticket": s.ticket.as_ref().map(|t| &t.key), "activity": s.activity,
                   "persona": s.persona.as_ref().map(|p| &p.name), "mode": s.mode})
        })
        .collect();
    // Tickets waiting out their project's start delay before auto-start takes them (PDT-146).
    let mut auto_starting: Vec<Value> = vec![];
    for p in &projects {
        let policy = st.store.get_project_settings(&p.id).await.map_err(store)?.policy;
        if policy.auto_start == pdt_core::policy::AutoStart::Off || policy.start_delay_seconds == 0 {
            continue;
        }
        let q = pdt_core::ticket::TicketQuery {
            project: Some(p.id.clone()),
            stage: Some("todo,ready".into()),
            sort: Some(policy.auto_start_order.ticket_sort()),
            limit: Some(100),
            ..Default::default()
        };
        let Json(mut page) = crate::tickets::list(State(st.clone()), Ok(axum::extract::Query(q))).await.map_err(api)?;
        // In the order auto-start takes them (PDT-218): To do before Ready, each in the project's order.
        page.items.sort_by_key(|t| t.stage != pdt_core::ticket::TicketStage::Todo);
        auto_starting.extend(page.items.iter().filter_map(|t| {
            let at = t.auto_start_at?;
            Some(json!({"key": t.key, "id": t.id, "title": t.title, "project": p.key,
                        "autoStartsInSeconds": starts_in(at)}))
        }));
    }
    Ok(json!({"projects": out_projects, "inProgress": in_flight, "autoStarting": auto_starting, "inbox": inbox,
              "runningSessions": sessions}))
}

/// Whole seconds from now until `at` (ms), rounded up.
fn starts_in(at: i64) -> i64 {
    ((at - crate::routes::now_ms()).max(0) + 999) / 1000
}

/// `get_session`: a session's state, open requests and last messages.
async fn session_state(cx: &Cx<'_>, id: &str, messages: usize) -> Result<Value, String> {
    use pdt_core::conversation::{AgentEvent, RequestState};
    let st = cx.st;
    let s = st.store.get_session(id.trim()).await.map_err(store)?.ok_or_else(|| format!("there is no session {id}"))?;
    let project = st.store.get_project(&s.project_id).await.map_err(store)?;
    let step = st.store.active_agent_step(&s.id).await.map_err(store)?.map(|a| {
        json!({"persona": a.persona_name, "state": a.state, "outcomes": a.outcomes, "turns": a.turns,
               "runId": a.run_id})
    });
    let recent = st.store.recent_session_events(&s.id, 400).await.map_err(store)?;
    let mut open = vec![];
    let mut texts = vec![];
    for e in recent.iter().rev() {
        match &e.event {
            AgentEvent::Request { title, state: RequestState::Open, tool, .. } => {
                open.push(json!({"tool": tool, "title": title, "at": e.ts}))
            }
            AgentEvent::UserMessage { text, source, .. } if texts.len() < messages => {
                texts.push(json!({"from": source, "text": clip(text), "at": e.ts}))
            }
            AgentEvent::AgentMessage { markdown, .. } if texts.len() < messages => {
                texts.push(json!({"from": "agent", "text": clip(markdown), "at": e.ts}))
            }
            _ => {}
        }
    }
    texts.reverse();
    let mut v = serde_json::to_value(&s).map_err(|e| e.to_string())?;
    v["project"] = json!(project.map(|p| p.key));
    v["step"] = step.unwrap_or(Value::Null);
    v["openRequests"] = json!(open);
    v["lastMessages"] = json!(texts);
    Ok(v)
}

/// A message, shortened for a tool result.
fn clip(t: &str) -> String {
    const MAX: usize = 2000;
    if t.len() <= MAX {
        return t.to_string();
    }
    let mut end = MAX;
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &t[..end])
}
