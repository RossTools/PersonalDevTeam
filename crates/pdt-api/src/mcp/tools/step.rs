//! The tools a workflow step's agent works with: its ticket, its outcome, questions and comments.

use super::*;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetTicket {
    /// A ticket key (`ABC-12`) or id. Leave it out for this session's ticket.
    #[serde(default)]
    ticket: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SubmitOutcome {
    /// One of the outcomes this step allows.
    pub outcome: String,
    /// What you did and found, in a few sentences: the next step (or the user) reads it.
    pub summary: String,
    /// Line comments on the attempt's changes (a review's findings).
    #[serde(default)]
    pub comments: Vec<OutcomeComment>,
    /// Fields the step's prompt asks for (an issue number, a ticket key), for the workflow's later steps.
    #[serde(default)]
    pub data: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AskUser {
    /// Short, concrete questions; give choices where a few answers are likely.
    questions: Vec<Question>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddTicketComment {
    /// Markdown. It is recorded as the agent's (`NewComment` without `by`: an agent never writes as the user).
    body: String,
    /// A ticket key or id; this session's ticket when left out.
    #[serde(default)]
    ticket: Option<String>,
    /// Files to attach to the comment (images show in it, text files as links): absolute local paths (images: PNG,
    /// JPEG, GIF, WebP up to 10 MB; UTF-8 text such as .txt, .log, .md, .json, .csv, .yaml, .toml up to 1 MB), or
    /// `pasted:1` for the newest image the user pasted into
    /// this conversation (`pasted:2` the one before). Never secrets: dotfiles and key folders are refused.
    #[serde(default)]
    files: Vec<String>,
}

pub(super) fn defs() -> Vec<Tool> {
    vec![
        Tool {
            decl: ToolDecl::new("get_ticket", "Read the ticket, its comments and attempt").read_only().pm(),
            title: "Get the ticket",
            description: "The ticket this session works on (or another, by key): description, stage, links, attempts, \
                          comments, attachments (each with a local file path to open), linked documents, the outcomes \
                          earlier agent steps submitted, and, in a workflow step, what the step allows.",
            schema: schema::<GetTicket>,
        },
        Tool {
            decl: ToolDecl::new("submit_outcome", "Finish the step with an outcome and a summary").pm(),
            title: "Submit the step's outcome",
            description: "Ends this session's workflow step with one of the outcomes it allows; PDT moves the work on \
                          from it. Give a short summary, line comments for a review, and `data` when the step's prompt asks for fields.",
            schema: schema::<SubmitOutcome>,
        },
        Tool {
            decl: ToolDecl::new("ask_user", "Ask you questions (they wait in the Inbox)").pm(),
            title: "Ask the user",
            description: "Puts questions in the user's Inbox. Their answers come back as a message; end your turn \
                          after asking. In a workflow step the step waits for them instead of stalling.",
            schema: schema::<AskUser>,
        },
        Tool {
            decl: ToolDecl::new("add_ticket_comment", "Comment on the ticket").pm(),
            title: "Comment on the ticket",
            description: "Adds a comment to the ticket's timeline (this session's ticket unless you name one), with \
                          files attached by local path if you give `files` (a screenshot shows in the comment).",
            schema: schema::<AddTicketComment>,
        },
        Tool {
            decl: ToolDecl::new("add_review_comment", "Comment on a line of the attempt's changes"),
            title: "Comment on a line",
            description: "Adds a line comment to the attempt's changes (the Changes view), on the file as it is in \
                          the attempt's worktree.",
            schema: schema::<OutcomeComment>,
        },
    ]
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
        "get_ticket" => {
            let a: GetTicket = args(a)?;
            let id = ticket_for(cx, a.ticket.as_deref()).await?;
            get_ticket(cx, &id).await
        }
        "submit_outcome" => {
            let a: SubmitOutcome = args(a)?;
            crate::workflow::agent::submit_outcome(st, cx.session, a).await
        }
        "ask_user" => {
            let a: AskUser = args(a)?;
            crate::workflow::agent::ask_user(st, cx.session, a.questions).await
        }
        "add_ticket_comment" => {
            let a: AddTicketComment = args(a)?;
            let id = ticket_for(cx, a.ticket.as_deref()).await?;
            let mut body = valid_comment(&a.body).map_err(|e| api(e.into()))?;
            let by = agent_by(st, cx.session).await;
            let read = crate::ticket_attachments::read_files(st, cx.session, &a.files).await?;
            let attached = crate::ticket_attachments::attach_read(st, &id, read, &by).await?;
            let mut data = None;
            if !attached.is_empty() {
                let refs: Vec<String> = attached.iter().map(|a| a.markdown()).collect();
                body = format!("{body}\n\n{}", refs.join("\n"));
                data = Some(json!({"attachments": attached.iter().map(|a| &a.id).collect::<Vec<_>>()}));
            }
            let e = st
                .store
                .add_comment_by(&id, &body, by, data, now_ms())
                .await
                .map_err(store)?
                .ok_or("that ticket no longer exists")?;
            let ids: Vec<String> = attached.iter().map(|a| a.id.clone()).collect();
            st.store.set_attachments_event(&ids, &e.id).await.map_err(store)?;
            Ok(json!({"commented": true, "eventId": e.id,
                      "attachments": crate::ticket_attachments::attached_json(st, &attached)}))
        }
        "add_review_comment" => {
            let a: OutcomeComment = args(a)?;
            let c = add_review_comment(cx, &a).await?;
            Ok(json!({"commented": true, "commentId": c.id, "file": c.path, "line": c.line}))
        }
        other => Err(format!("no tool is called {other}")),
    }
}

/// The changes `get_ticket` lists as a ticket's history, and how many (the newest).
const HISTORY: &[EventKind] = &[
    EventKind::Created,
    EventKind::Edited,
    EventKind::Moved,
    EventKind::Linked,
    EventKind::Unlinked,
    EventKind::Parted,
    EventKind::Waiting,
    EventKind::Started,
    EventKind::Merged,
    EventKind::Abandoned,
];

const HISTORY_LEN: u32 = 50;

/// Who made an event and when, for `get_ticket`: `by` is the agent's persona (`agent` when it had none or nothing is
/// known), `user` or `system`, with the agent's session and the ticket it works on (PDT-163).
fn who(e: &TicketEvent) -> Value {
    let mut v = json!({"by": e.actor, "at": e.created_at});
    if let Some(a) = &e.agent {
        if e.actor == Actor::Agent
            && let Some(p) = &a.persona
        {
            v["by"] = json!(p);
        }
        v["sessionId"] = json!(a.session_id);
        if let Some(t) = &a.from_ticket {
            v["fromTicket"] = json!(t);
        }
    }
    v
}

async fn get_ticket(cx: &Cx<'_>, id: &str) -> Result<Value, String> {
    let st = cx.st;
    let Json(ticket) =
        crate::tickets::get_one(State(st.clone()), axum::extract::Path(id.to_string())).await.map_err(api)?;
    let comments: Vec<Value> = st
        .store
        .events_of(id, &[EventKind::Comment], 1000)
        .await
        .map_err(store)?
        .iter()
        .filter(|e| e.body.is_some())
        .map(|e| {
            let mut v = who(e);
            v["body"] = json!(e.body);
            v
        })
        .collect();
    let history: Vec<Value> = st
        .store
        .events_of(id, HISTORY, HISTORY_LEN)
        .await
        .map_err(store)?
        .iter()
        .map(|e| {
            let mut v = who(e);
            v["kind"] = json!(e.kind);
            if let Some(Value::Object(mut d)) = e.data.clone() {
                d.remove("agent");
                if !d.is_empty() {
                    v["data"] = Value::Object(d);
                }
            }
            v
        })
        .collect();
    let outcomes: Vec<Value> = st
        .store
        .ticket_outcomes(id)
        .await
        .map_err(store)?
        .into_iter()
        .map(|a| {
            json!({"persona": a.persona_name, "outcome": a.outcome, "summary": a.summary, "comments": a.comments,
                   "by": if a.by_user { "user" } else { "agent" }, "at": a.updated_at})
        })
        .collect();
    let step = match st.store.active_agent_step(&cx.session.id).await.map_err(store)? {
        Some(a) if a.ticket_id.as_deref() == Some(id) => {
            json!({"persona": a.persona_name, "outcomes": a.outcomes, "state": a.state})
        }
        _ => Value::Null,
    };
    // An external ticket's description is what someone outside PDT wrote (an imported issue, PDT-202): fenced.
    let external = ticket.summary.source == TicketSource::External;
    let fence = pdt_core::fence::Fence::new(uuid::Uuid::new_v4().simple().to_string());
    let outside = external.then(|| (fence.wrap(&ticket.description), fence.note()));
    let attachments = attachments_of(st, id, external.then_some(&fence)).await?;
    let mut v = serde_json::to_value(ticket).map_err(|e| e.to_string())?;
    if let Some((description, note)) = outside {
        v["description"] = json!(description);
        v["outsideText"] = json!(format!(
            "{note} This ticket came from outside PDT, so its description (and any text file attached to it) is in \
             such a block (the tags are not part of it)."
        ));
    }
    if !attachments.is_empty() {
        v["attachments"] = json!(attachments);
        v["attachmentsNote"] = json!(
            "The ticket has attachments: open the images and files that matter (each `path` is a local file you can \
             read; an image opens with your file-reading or image-viewing tool)."
        );
    }
    let documents = crate::documents::routes::for_agent(st, id).await.map_err(api)?;
    if !documents.is_empty() {
        v["documents"] = json!(documents);
        v["documentsNote"] = json!(
            "Documents are linked to this ticket. Read its design doc (and any other that matters) before you start: \
             doc_read gives a doc as Markdown, sheet_read a sheet's cells."
        );
    }
    v["comments"] = json!(comments);
    v["history"] = json!(history);
    v["outcomes"] = json!(outcomes);
    if !step.is_null() {
        v["step"] = step;
    }
    Ok(v)
}

/// A ticket's attachments for `get_ticket`: name, type, size, who added it and when, and the local file's path. On
/// an external ticket (`fence` given) a text file's content comes fenced as outside input instead of its path.
async fn attachments_of(st: &AppState, id: &str, fence: Option<&pdt_core::fence::Fence>) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for a in st.store.ticket_attachments(id).await.map_err(store)? {
        let path = crate::ticket_attachments::path_of(st, &a);
        let by = match (&a.agent, a.actor) {
            (Some(g), Actor::Agent) => g.persona.clone().unwrap_or_else(|| "agent".into()),
            (_, actor) => actor.as_str().to_string(),
        };
        let mut v = json!({"id": a.id, "name": a.name, "mediaType": a.media_type, "size": a.size, "by": by,
                           "at": a.created_at});
        match fence {
            Some(f) if !a.is_image() => {
                let text = tokio::fs::read_to_string(&path).await.unwrap_or_default();
                v["text"] = json!(f.wrap(&text));
            }
            _ => v["path"] = json!(path),
        }
        out.push(v);
    }
    Ok(out)
}

/// A review comment on the session's attempt's worktree, by its persona (or `agent`), already sent.
pub(crate) async fn add_review_comment(
    cx: &Cx<'_>,
    c: &OutcomeComment,
) -> Result<pdt_core::review::ReviewComment, String> {
    let st = cx.st;
    let attempt = match &cx.session.attempt_id {
        Some(a) => Some(a.clone()),
        None => st.store.active_agent_step(&cx.session.id).await.map_err(store)?.and_then(|a| a.attempt_id),
    };
    let attempt = attempt.ok_or("this session works on no attempt, so there are no changes to comment on")?;
    let info = st.store.get_attempt(&attempt).await.map_err(store)?.ok_or("the attempt is gone")?;
    let wt_id = info.attempt.worktree_id.clone().ok_or("the attempt has no worktree")?;
    let wt = st.store.get_worktree(&wt_id).await.map_err(store)?.ok_or("the attempt's worktree is gone")?;
    let end = c.end_line.unwrap_or(c.line).max(c.line);
    let file = crate::files::clean_path(&c.file).map_err(api)?;
    let text = std::fs::read_to_string(std::path::Path::new(&wt.path).join(&file)).unwrap_or_default();
    let line_text: String = text
        .lines()
        .skip(usize::try_from(c.line.saturating_sub(1)).unwrap_or(0))
        .take(usize::try_from(end - c.line + 1).unwrap_or(1))
        .collect::<Vec<_>>()
        .join("\n");
    let input = NewReviewComment {
        path: file,
        side: CommentSide::New,
        line: c.line,
        end_line: Some(end),
        line_text,
        body: c.body.clone(),
        parent_id: None,
    };
    let author = cx.session.persona.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "agent".into());
    crate::review::add_comment(st, &wt_id, input, &author, true).await.map_err(api)
}
