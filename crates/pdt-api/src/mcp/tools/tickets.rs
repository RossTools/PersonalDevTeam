//! The PM's ticket tools: create, edit, link and stop tickets.

use super::*;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateTicket {
    /// The project's key (`PDT`) or id.
    project: String,
    /// One line.
    title: String,
    /// Markdown: scope, out of scope, "Done when".
    #[serde(default)]
    description: Option<String>,
    /// draft (the default), ready or todo.
    #[serde(default)]
    stage: Option<pdt_core::ticket::TicketStage>,
    /// none, low, medium, high or urgent.
    #[serde(default)]
    priority: Option<pdt_core::ticket::Priority>,
    #[serde(default)]
    labels: Option<Vec<String>>,
    /// Make it a part of this ticket (key or id).
    #[serde(default)]
    parent: Option<String>,
    /// Files to attach: absolute local paths (images: PNG, JPEG, GIF, WebP up to 10 MB; UTF-8 text such as .txt,
    /// .log, .md, .json, .csv, .yaml, .toml up to 1 MB), or `pasted:1` for the newest image the user pasted into
    /// this conversation (`pasted:2` the one before). Never secrets: dotfiles and key folders are refused.
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditTicket {
    /// The ticket's key (`ABC-12`) or id. Outside the PM, a ticket other than the session's own needs its exact key.
    ticket: String,
    #[serde(default)]
    title: Option<String>,
    /// The new description, replacing the old one: keep what still holds.
    #[serde(default)]
    description: Option<String>,
    /// draft, ready, todo, in_progress, done or wont_do. Done and wont_do end its attempt and stop its sessions and
    /// runs; wont_do also closes its pull request and removes its worktree and branch when nothing would be lost;
    /// draft, ready and todo stop its work like `stop_ticket` (keeping the worktree).
    #[serde(default)]
    stage: Option<pdt_core::ticket::TicketStage>,
    #[serde(default)]
    priority: Option<pdt_core::ticket::Priority>,
    /// Replaces every label.
    #[serde(default)]
    labels: Option<Vec<String>>,
    /// Make it a part of this ticket (key or id), or `""` to detach it from its parent.
    #[serde(default)]
    parent: Option<String>,
    /// Files to attach: absolute local paths (images: PNG, JPEG, GIF, WebP up to 10 MB; UTF-8 text such as .txt,
    /// .log, .md, .json, .csv, .yaml, .toml up to 1 MB), or `pasted:1` for the newest image the user pasted into
    /// this conversation (`pasted:2` the one before). Never secrets: dotfiles and key folders are refused.
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StopTicket {
    /// The ticket's key (`ABC-12`) or id.
    ticket: String,
    /// Where it goes: draft, ready or todo. Left out, an in-progress ticket goes to todo.
    #[serde(default)]
    stage: Option<pdt_core::ticket::TicketStage>,
    /// Also remove its worktree (uncommitted changes too) and the workspace Start made, so the next start is clean.
    #[serde(default)]
    remove_worktree: bool,
    /// With removeWorktree: delete its branch too.
    #[serde(default)]
    delete_branch: bool,
}

/// How `ticket` stands to `other` in a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum LinkAs {
    /// `ticket` blocks `other`: `other` waits until `ticket` is done.
    Blocks,
    /// `ticket` is blocked by `other`.
    BlockedBy,
    /// Related, either way round.
    Relates,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkTickets {
    /// The ticket's key (`ABC-12`) or id.
    ticket: String,
    /// blocks, blockedBy or relates: how `ticket` stands to `other`.
    kind: LinkAs,
    /// The other ticket's key or id, in the same project.
    other: String,
}

pub(super) fn defs() -> Vec<Tool> {
    vec![
        Tool {
            decl: ToolDecl::new("create_ticket", "Create tickets").asks().pm(),
            title: "Create a ticket",
            description: "Adds a ticket to a project (a draft unless you give a stage), recorded as the PM's, with \
                          `files` attached (an image the user pasted here is `pasted:1`). The user confirms each call \
                          unless the project lets you act.",
            schema: schema::<CreateTicket>,
        },
        Tool {
            decl: ToolDecl::new("edit_ticket", "Edit tickets (outside the PM, others only by exact key)").asks().pm(),
            title: "Edit a ticket",
            description: "Changes a ticket's title, description, stage, priority, labels or parent, recorded as the \
                          agent's. Outside the PM, a ticket other than the session's own needs its exact key \
                          (`ABC-12`), and one in progress or done keeps its title and description unless this \
                          session created it: comment on it instead. `files` attaches files to it (alone, or with \
                          the changes). The user confirms each call unless the project lets you act.",
            schema: schema::<EditTicket>,
        },
        Tool {
            decl: ToolDecl::new("link_tickets", "Link two tickets (blocks, relates to)").asks().pm(),
            title: "Link two tickets",
            description: "Links a ticket to another in its project: it blocks the other, is blocked by it, or relates \
                          to it. The link shows on both tickets and a blocked ticket waits on the board. Self-links \
                          and blocking cycles are refused. The user confirms each call unless the project lets you \
                          act.",
            schema: schema::<LinkTickets>,
        },
        Tool {
            decl: ToolDecl::new("unlink_tickets", "Remove a link between tickets").asks().pm(),
            title: "Unlink two tickets",
            description: "Removes a link made with link_tickets (or in the UI): the same ticket, kind and other. The \
                          user confirms each call unless the project lets you act.",
            schema: schema::<LinkTickets>,
        },
        Tool {
            decl: ToolDecl::new("stop_ticket", "Stop a ticket's work: its workflow run, sessions and attempt")
                .asks()
                .pm(),
            title: "Stop a ticket's work",
            description: "Stops the work on a ticket: cancels its workflow runs, stops its agent sessions, abandons its \
                          attempt and moves it back to To do (or the stage you give: draft, ready or todo). The \
                          worktree stays unless removeWorktree (then uncommitted changes go). Use it for work that \
                          started by mistake; a busy agent can't be told to stop with send_to_session. The user \
                          confirms each call unless the project lets you act.",
            schema: schema::<StopTicket>,
        },
    ]
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
        "create_ticket" => {
            let a: CreateTicket = args(a)?;
            let project = crate::pm::project_id(st, &a.project).await?;
            confirmable(cx, name, &pdt_core::pm::AskTarget::Project(project.clone())).await?;
            let parent = match a.parent.as_deref() {
                Some(p) => Some(resolve_ticket(st, p, Some(&project)).await?),
                None => None,
            };
            let pm = crate::pm::is_pm_session(st, cx.session).await;
            // A session reading outside input makes external tickets, as Drafts only (PDT-201).
            let triaging = crate::untrusted::triaging(st, cx.session).await;
            let agentic = a.labels.iter().flatten().any(|l| l.trim().eq_ignore_ascii_case(AGENTIC_LABEL));
            if triaging && (a.stage.is_some_and(|s| s != TicketStage::Draft) || agentic) {
                return Err(format!(
                    "this session triages input from outside PDT, so its tickets are Drafts without the \
                     {AGENTIC_LABEL} label: the user decides when they go ahead. Leave out stage and that label."
                ));
            }
            let source = match (triaging, pm) {
                (true, _) => TicketSource::External,
                (false, true) => TicketSource::Pm,
                (false, false) => TicketSource::Workflow,
            };
            let input = pdt_core::ticket::NewTicket {
                title: a.title,
                description: a.description,
                stage: Some(a.stage.unwrap_or(pdt_core::ticket::TicketStage::Draft)),
                priority: a.priority,
                labels: a.labels,
                areas: None,
                before: None,
                after: None,
                parent,
                blocked_by_previous: None,
            };
            let valid = pdt_core::ticket::ValidNewTicket { source, ..input.validate().map_err(|e| api(e.into()))? };
            let read = crate::ticket_attachments::read_files(st, cx.session, &a.files).await?;
            let id = uuid::Uuid::now_v7().to_string();
            let by_agent = agent_by(st, cx.session).await;
            let t = st
                .store
                .create_ticket_by(&id, &project, &valid, by_agent.clone(), now_ms())
                .await
                .map_err(|e| api(e.into()))?;
            let attached = crate::ticket_attachments::attach_read(st, &id, read, &by_agent).await?;
            let by = match pm {
                true => "the PM".to_string(),
                false => cx.session.persona.as_ref().map_or("an agent".into(), |p| format!("the {}", p.name)),
            };
            // `created` lets the session edit it later as its own (PDT-162).
            record_agent_note(cx, &t.summary.id, &format!("Created by {by}."), json!({"created": true})).await;
            if triaging {
                crate::untrusted::record_triage(st, cx.session, &t.summary.id).await;
            }
            Ok(json!({"created": true, "key": t.summary.key, "id": t.summary.id, "stage": t.summary.stage,
                      "attachments": crate::ticket_attachments::attached_json(st, &attached)}))
        }
        "edit_ticket" => {
            let raw = a.clone();
            let a: EditTicket = args(a)?;
            let id = resolve_ticket(st, &a.ticket, Some(&cx.session.project_id)).await?;
            let current = st.store.get_ticket(&id).await.map_err(store)?.ok_or("that ticket no longer exists")?;
            let pm = crate::pm::is_pm_session(st, cx.session).await;
            if !pm {
                let rewrites = a.title.as_deref().is_some_and(|t| t.trim() != current.summary.title)
                    || a.description.as_deref().is_some_and(|d| d.trim() != current.description.trim());
                let edit = pdt_core::ticket::AgentEdit {
                    given: &a.ticket,
                    key: &current.summary.key,
                    stage: current.summary.stage,
                    own: session_ticket(st, cx.session).await.as_deref() == Some(id.as_str()),
                    created: st.store.created_by_session(&id, &cx.session.id).await.map_err(store)?,
                    rewrites,
                };
                pdt_core::ticket::agent_may_edit(&edit)?;
            }
            confirmable(cx, name, &pdt_core::pm::AskTarget::Ticket(id.clone())).await?;
            let project = Some(current.summary.project_id.clone());
            let parent = match a.parent.as_deref().map(str::trim) {
                Some("") => Some(String::new()),
                Some(p) => Some(resolve_ticket(st, p, project.as_deref()).await?),
                None => None,
            };
            let patch = pdt_core::ticket::TicketPatch {
                title: a.title,
                description: a.description,
                stage: a.stage,
                priority: a.priority,
                labels: a.labels,
                parent,
                ..Default::default()
            };
            let valid = patch.validate().map_err(|e| api(e.into()))?;
            let unchanged = valid == Default::default();
            if unchanged && a.files.is_empty() {
                return Err("give at least one field to change, or files to attach".into());
            }
            let read = crate::ticket_attachments::read_files(st, cx.session, &a.files).await?;
            if unchanged {
                let by = agent_by(st, cx.session).await;
                let attached = crate::ticket_attachments::attach_read(st, &id, read, &by).await?;
                return Ok(json!({"updated": true, "key": current.summary.key, "title": current.summary.title,
                                 "attachments": crate::ticket_attachments::attached_json(st, &attached)}));
            }
            let since = now_ms();
            let mut by = agent_by(st, cx.session).await;
            // Letting an external ticket in (PDT-201): only the PM, with the user's yes to this very call (PDT always
            // asks it, even where the project lets the PM act). Anyone else is refused by the store.
            if pm && crate::untrusted::lets_in(&current.summary, &valid) {
                by.confirmed = cx.confirmed || st.grants.take(&cx.session.id, name, &raw);
            }
            let t = st
                .store
                .patch_ticket_by(&id, &valid, &by, since)
                .await
                .map_err(|e| api(e.into()))?
                .ok_or("that ticket no longer exists")?;
            match valid.stage {
                Some(stage) if stage.is_closed() => {
                    crate::start::close_attempt(st, &id, stage, since, &by, None).await.map_err(api)?;
                }
                // Back before work: the ticket's work stops (PDT-159).
                Some(stage) => crate::stop::after_move(st, &id, stage, since, &by).await,
                None => {}
            }
            let attached = crate::ticket_attachments::attach_read(st, &id, read, &by).await?;
            Ok(json!({"updated": true, "key": t.summary.key, "title": t.summary.title, "stage": t.summary.stage,
                      "parent": t.summary.parent.map(|p| p.key),
                      "attachments": crate::ticket_attachments::attached_json(st, &attached)}))
        }
        "link_tickets" | "unlink_tickets" => {
            let a: LinkTickets = args(a)?;
            let id = resolve_ticket(st, &a.ticket, Some(&cx.session.project_id)).await?;
            let project = st.store.get_ticket(&id).await.map_err(store)?.map(|t| t.summary.project_id);
            let other = resolve_ticket(st, &a.other, project.as_deref()).await?;
            confirmable(cx, name, &pdt_core::pm::AskTarget::Ticket(id.clone())).await?;
            let (from, to, kind) = match a.kind {
                LinkAs::Blocks => (&id, &other, pdt_core::ticket::LinkKind::Blocks),
                LinkAs::BlockedBy => (&other, &id, pdt_core::ticket::LinkKind::Blocks),
                LinkAs::Relates => (&id, &other, pdt_core::ticket::LinkKind::Relates),
            };
            let linked = name == "link_tickets";
            let by = agent_by(st, cx.session).await;
            let done = match linked {
                true => st.store.add_link_by(from, to, kind, by, now_ms()).await,
                false => st.store.remove_link_by(from, to, kind, by, now_ms()).await,
            };
            done.map_err(|e| match e {
                pdt_store::TicketError::NotFound("link") => "there is no such link between them".to_string(),
                e => api(e.into()),
            })?
            .ok_or("that ticket no longer exists")?;
            let t = st.store.get_ticket(&id).await.map_err(store)?.ok_or("that ticket no longer exists")?;
            let links: Vec<Value> = t
                .links
                .iter()
                .map(|l| {
                    let kind = match (l.kind, l.direction) {
                        (pdt_core::ticket::LinkKind::Relates, _) => "relates",
                        (_, pdt_core::ticket::LinkDirection::Outgoing) => "blocks",
                        (_, pdt_core::ticket::LinkDirection::Incoming) => "blockedBy",
                    };
                    json!({"kind": kind, "key": l.key, "title": l.title, "stage": l.stage})
                })
                .collect();
            Ok(json!({if linked { "linked" } else { "unlinked" }: true, "key": t.summary.key, "links": links}))
        }
        "stop_ticket" => {
            let a: StopTicket = args(a)?;
            let id = resolve_ticket(st, &a.ticket, Some(&cx.session.project_id)).await?;
            confirmable(cx, name, &AskTarget::Ticket(id.clone())).await?;
            let req = crate::stop::StopRequest {
                stage: a.stage,
                remove_worktree: a.remove_worktree,
                delete_branch: a.delete_branch,
            };
            let by = agent_by(st, cx.session).await;
            let task = tokio::spawn({
                let (st, id) = (st.clone(), id.clone());
                async move { crate::stop::stop_ticket(&st, &id, &req, by).await }
            });
            let r = task.await.map_err(|e| e.to_string())?.map_err(api)?;
            record_agent_change(cx, &id, "The PM stopped its work.").await;
            Ok(json!({"stopped": true, "key": r.key, "stage": r.stage, "abandonedAttempt": r.abandoned,
                      "runsCancelled": r.runs_cancelled, "sessionsStopped": r.sessions_stopped,
                      "worktreeRemoved": r.worktree_removed, "branchNote": r.branch_note}))
        }
        other => Err(format!("no tool is called {other}")),
    }
}

/// Notes on the ticket's timeline that an agent changed it through PDT's tools (principle 5: changes are on record).
async fn record_agent_change(cx: &Cx<'_>, ticket_id: &str, text: &str) {
    record_agent_note(cx, ticket_id, text, json!({})).await;
}

/// [`record_agent_change`] with more fields in the note's data.
async fn record_agent_note(cx: &Cx<'_>, ticket_id: &str, text: &str, extra: Value) {
    let by = agent_by(cx.st, cx.session).await;
    let data = (extra.as_object().is_some_and(|m| !m.is_empty())).then_some(extra);
    if let Err(e) = cx.st.store.add_comment_by(ticket_id, text, by, data, now_ms()).await {
        tracing::warn!(ticket = %ticket_id, error = %e, "could not record the PM's change on the timeline");
    }
}
