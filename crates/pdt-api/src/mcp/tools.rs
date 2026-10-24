//! pdt-mcp's tools (Phase 7b spec §2.2). Inputs are the API's request types where one exists (their JSON Schema from
//! schemars), and each tool runs the function the matching route runs, so the API and the tools can't drift.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::Category;
use axum::Json;
use axum::extract::State;
use pdt_core::inbox::Question;
use pdt_core::mcp::{ToolInfo, input_schema};
use pdt_core::review::{CommentSide, NewReviewComment};
use pdt_core::session::Session;
use pdt_core::ticket::{Actor, valid_comment};
use pdt_core::workflow::agent::OutcomeComment;
use pdt_core::workflow::api::{Author, Scope};
use pdt_core::workflow::doc::WorkflowDoc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// What a tool call knows: PDT and the calling session.
pub(crate) struct Cx<'a> {
    pub st: &'a AppState,
    pub session: &'a Session,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetTicket {
    /// A ticket key (`PDT-12`) or id. Leave it out for this session's ticket.
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
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateTicket {
    /// A new title (one line).
    #[serde(default)]
    title: Option<String>,
    /// The new description (Markdown), replacing the old one: keep what still holds.
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogSearch {
    /// Words to look for in the kinds' ids, titles and descriptions; every kind when left out.
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    category: Option<Category>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogGet {
    /// The kind's id (`session.send`).
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowGet {
    #[serde(default)]
    id: Option<String>,
    /// The workflow's name, when you don't know its id.
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowSave {
    /// The workflow to save a new version of; a new (disabled) workflow when left out.
    #[serde(default)]
    id: Option<String>,
    doc: WorkflowDoc,
    /// What changed, for the version history.
    #[serde(default)]
    note: String,
    /// A new workflow's scope (default: the session's project).
    #[serde(default)]
    scope: Option<Scope>,
    #[serde(default)]
    project_id: Option<String>,
}

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
    /// Words (prefix-matched in titles and descriptions) or a key (`PDT-12`).
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SearchNotes {
    /// Words to look for in titles and bodies; every note when left out.
    #[serde(default)]
    query: Option<String>,
    /// A project's key or id: its notes and the host-wide ones.
    #[serde(default)]
    project: Option<String>,
    /// Only pinned notes (true) or unpinned ones (false).
    #[serde(default)]
    pinned: Option<bool>,
    /// note, journal or user (the user's own notes); all of them when left out.
    #[serde(default)]
    kind: Option<pdt_core::pm::NoteKind>,
    /// At most this many (default 20, at most 100).
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListTodos {
    /// A project's key or id: only its todos. Every todo when left out.
    #[serde(default)]
    project: Option<String>,
    /// Only done todos (true) or open ones (false); both when left out.
    #[serde(default)]
    done: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveNote {
    /// The note to replace; a new note when left out.
    #[serde(default)]
    id: Option<String>,
    /// A project's key or id; host-wide when left out (or "" to make an existing note host-wide).
    #[serde(default)]
    project: Option<String>,
    /// One line.
    title: String,
    /// Markdown.
    #[serde(default)]
    body: String,
    /// Pinned notes are listed in every PM session's briefing.
    #[serde(default)]
    pinned: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeleteNote {
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WriteJournal {
    /// Markdown: what you and the user were working on, decisions, threads still open.
    body: String,
}

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
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditTicket {
    /// The ticket's key (`PDT-12`) or id.
    ticket: String,
    #[serde(default)]
    title: Option<String>,
    /// The new description, replacing the old one: keep what still holds.
    #[serde(default)]
    description: Option<String>,
    /// draft, ready, todo, in_progress, done or wont_do. Done and wont_do end its attempt.
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
    /// The ticket's key (`PDT-12`) or id.
    ticket: String,
    /// blocks, blockedBy or relates: how `ticket` stands to `other`.
    kind: LinkAs,
    /// The other ticket's key or id, in the same project.
    other: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendToSession {
    /// The session's id.
    session: String,
    /// The message. PDT delivers it when the session is idle, marked as from the PM.
    text: String,
}

struct Def {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

/// Tools that change nothing.
const READ_ONLY: [&str; 13] = [
    "get_ticket",
    "catalog_search",
    "catalog_get",
    "workflow_get",
    "workflow_validate",
    "workflow_dry_run",
    "get_overview",
    "search_tickets",
    "get_session",
    "list_workflow_runs",
    "search_notes",
    "list_todos",
    "discord_get",
];

fn schema<T: JsonSchema>() -> Value {
    input_schema(crate::workflow::catalog::schema_of::<T>())
}

/// Every tool, in `ALL_TOOLS` order.
fn defs() -> Vec<Def> {
    vec![
        Def {
            name: "get_ticket",
            title: "Get the ticket",
            description: "The ticket this session works on (or another, by key): description, stage, links, attempts, \
                          comments, the outcomes earlier agent steps submitted, and, in a workflow step, what the step \
                          allows.",
            schema: schema::<GetTicket>,
        },
        Def {
            name: "submit_outcome",
            title: "Submit the step's outcome",
            description: "Ends this session's workflow step with one of the outcomes it allows; PDT moves the work on \
                          from it. Give a short summary, and line comments for a review.",
            schema: schema::<SubmitOutcome>,
        },
        Def {
            name: "ask_user",
            title: "Ask the user",
            description: "Puts questions in the user's Inbox. Their answers come back as a message; end your turn \
                          after asking. In a workflow step the step waits for them instead of stalling.",
            schema: schema::<AskUser>,
        },
        Def {
            name: "add_ticket_comment",
            title: "Comment on the ticket",
            description: "Adds a comment to the ticket's timeline (this session's ticket unless you name one).",
            schema: schema::<AddTicketComment>,
        },
        Def {
            name: "add_review_comment",
            title: "Comment on a line",
            description: "Adds a line comment to the attempt's changes (the Changes view), on the file as it is in \
                          the attempt's worktree.",
            schema: schema::<OutcomeComment>,
        },
        Def {
            name: "update_ticket",
            title: "Update the ticket",
            description: "Rewrites this session's ticket: its title and/or description (as `PATCH /api/tickets/{id}`, \
                          recorded as the agent's edit). Only this session's own ticket.",
            schema: schema::<UpdateTicket>,
        },
        Def {
            name: "catalog_search",
            title: "Search the node catalog",
            description: "Lists workflow node kinds (triggers, conditions, actions, gates, waits) matching the \
                          words: id, title, category, description. MCP servers' tools are listed too, as \
                          `mcp:<server>/<tool>` (a `preset` mcp.tool node). catalog_get gives one kind's schemas.",
            schema: schema::<CatalogSearch>,
        },
        Def {
            name: "catalog_get",
            title: "Get a node kind",
            description: "One node kind with its config and output JSON Schemas and its ports.",
            schema: schema::<CatalogGet>,
        },
        Def {
            name: "workflow_get",
            title: "Get a workflow",
            description: "A workflow (by id or name) with its current document and validation issues.",
            schema: schema::<WorkflowGet>,
        },
        Def {
            name: "workflow_validate",
            title: "Validate a workflow",
            description: "Checks a workflow document: errors and warnings.",
            schema: schema::<crate::workflow::routes::ValidateRequest>,
        },
        Def {
            name: "workflow_dry_run",
            title: "Dry-run a workflow",
            description: "Runs a document (or a saved workflow) against an event in memory, changing nothing: which \
                          triggers match and what each step would do.",
            schema: schema::<crate::workflow::routes::DryRunRequest>,
        },
        Def {
            name: "workflow_save",
            title: "Save a workflow",
            description: "Proposes a document as a new version of a workflow (the user reviews and accepts it), or \
                          saves it as a new workflow, which starts disabled: the user reviews and enables it.",
            schema: schema::<WorkflowSave>,
        },
        Def {
            name: "get_overview",
            title: "What's going on",
            description: "Every project (or one): open tickets per stage, tickets in progress with their branch, PR, CI \
                          and who has them, the Inbox's open items, and the agent sessions running now.",
            schema: schema::<GetOverview>,
        },
        Def {
            name: "search_tickets",
            title: "Search tickets",
            description: "Tickets matching words or a key, in every project or one, by stage: key, title, stage, \
                          priority, labels and state. get_ticket reads one in full.",
            schema: schema::<SearchTickets>,
        },
        Def {
            name: "get_session",
            title: "Get a session",
            description: "An agent session's state: agent, status, activity, ticket, persona and workflow step, its \
                          open requests, and its last messages.",
            schema: schema::<GetSession>,
        },
        Def {
            name: "list_workflow_runs",
            title: "List workflow runs",
            description: "Workflow runs (of a workflow or a ticket), newest first, with each step's status, port and \
                          error.",
            schema: schema::<ListWorkflowRuns>,
        },
        Def {
            name: "search_notes",
            title: "Search notes",
            description: "The PM's notes and journal entries, and the user's own notes (kind user, writtenBy \
                          \"the user\"), matching the words (full-text), host-wide and per project; every pinned \
                          note when you give no words and pinned: true.",
            schema: schema::<SearchNotes>,
        },
        Def {
            name: "save_note",
            title: "Save a note",
            description: "Adds a note (or replaces one by id): what PDT's data can't tell you, such as the user's \
                          preferences, decisions, people and rules at work. Pinned notes go in every briefing. The \
                          user's own notes (kind user) are theirs: you can't change them.",
            schema: schema::<SaveNote>,
        },
        Def {
            name: "delete_note",
            title: "Delete a note",
            description: "Removes a note that no longer holds (not one of the user's own).",
            schema: schema::<DeleteNote>,
        },
        Def {
            name: "write_journal",
            title: "Write the journal",
            description: "This session's journal entry, which the next PM session's briefing includes. Write it when \
                          PDT says the session is ending.",
            schema: schema::<WriteJournal>,
        },
        Def {
            name: "list_todos",
            title: "List todos",
            description: "The user's own todo list (open ones in their order, then done ones): title, done, project, \
                          linked ticket, due date (dueOn, YYYY-MM-DD, and dueTime, HH:MM, the user's local time), how it repeats \
                          (recurrence: a rule, every day/weekday/week/month, and whether the next one counts from \
                          the due date or when it's done; checking it off makes the next one) and dates. Read-only: the user edits it in PDT.",
            schema: schema::<ListTodos>,
        },
        Def {
            name: "create_ticket",
            title: "Create a ticket",
            description: "Adds a ticket to a project (a draft unless you give a stage), recorded as the PM's. The \
                          user confirms each call unless the project lets you act.",
            schema: schema::<CreateTicket>,
        },
        Def {
            name: "edit_ticket",
            title: "Edit a ticket",
            description: "Changes a ticket's title, description, stage, priority, labels or parent, recorded as the \
                          agent's. The user confirms each call unless the project lets you act.",
            schema: schema::<EditTicket>,
        },
        Def {
            name: "link_tickets",
            title: "Link two tickets",
            description: "Links a ticket to another in its project: it blocks the other, is blocked by it, or relates \
                          to it. The link shows on both tickets and a blocked ticket waits on the board. Self-links \
                          and blocking cycles are refused. The user confirms each call unless the project lets you \
                          act.",
            schema: schema::<LinkTickets>,
        },
        Def {
            name: "unlink_tickets",
            title: "Unlink two tickets",
            description: "Removes a link made with link_tickets (or in the UI): the same ticket, kind and other. The \
                          user confirms each call unless the project lets you act.",
            schema: schema::<LinkTickets>,
        },
        Def {
            name: "send_to_session",
            title: "Message a session",
            description: "Sends an agent session a message, delivered when it is idle and marked as from the PM. The \
                          user confirms each call unless the project lets you act.",
            schema: schema::<SendToSession>,
        },
        Def {
            name: "discord_get",
            title: "Look at the Discord server",
            description: "The Discord server a bot connection manages: its categories, channels and webhooks (with \
                          the connection each webhook is saved as), and whether you may change it without asking.",
            schema: schema::<crate::discord::DiscordGet>,
        },
        Def {
            name: "discord_change",
            title: "Change the Discord server",
            description: "Creates, renames, moves or archives (moves into Archive) channels and categories on the \
                          Discord server, in order. The user confirms each call unless the bot connection lets you \
                          act.",
            schema: schema::<crate::discord::DiscordChange>,
        },
        Def {
            name: "discord_webhook",
            title: "Make a Discord webhook",
            description: "Makes a webhook on a channel (and the channel, with createChannel) and saves it as a \
                          discord_webhook connection that discord.post steps use by name. Its URL is never shown. \
                          The user confirms each call unless the bot connection lets you act.",
            schema: schema::<crate::discord::DiscordWebhookArgs>,
        },
    ]
}

pub(crate) fn infos() -> Vec<ToolInfo> {
    defs()
        .into_iter()
        .map(|d| ToolInfo {
            name: d.name.into(),
            title: d.title.into(),
            description: d.description.into(),
            input_schema: (d.schema)(),
            annotations: json!({"title": d.title, "readOnlyHint": READ_ONLY.contains(&d.name),
                                "destructiveHint": d.name == "delete_note",
                                "openWorldHint": d.name.starts_with("discord_")}),
        })
        .collect()
}

fn args<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    let v = if v.is_null() { json!({}) } else { v };
    serde_json::from_value(v).map_err(|e| format!("the arguments don't fit the tool's schema: {e}"))
}

fn api(e: ApiError) -> String {
    e.message().to_string()
}

fn store(e: pdt_store::StoreError) -> String {
    ApiError::from(e).message().to_string()
}

/// Refuses a PM tool's change to one of the user's own notes (PDT-126); a missing note is left to the tool.
async fn pm_may_write(st: &AppState, id: &str) -> Result<(), String> {
    match st.store.get_note(id).await.map_err(store)? {
        Some(n) if !n.kind.pm_may_write() => Err(format!("note {id} is the user's own: only they can change it")),
        _ => Ok(()),
    }
}

/// Runs a tool. Err is a message for the agent.
pub(crate) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
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
            let body = valid_comment(&a.body).map_err(|e| api(e.into()))?;
            let data = json!({"sessionId": cx.session.id, "persona": cx.session.persona.as_ref().map(|p| &p.name)});
            let e = st
                .store
                .add_comment_by(&id, &body, Actor::Agent, Some(data), now_ms())
                .await
                .map_err(store)?
                .ok_or("that ticket no longer exists")?;
            Ok(json!({"commented": true, "eventId": e.id}))
        }
        "update_ticket" => {
            let a: UpdateTicket = args(a)?;
            if a.title.is_none() && a.description.is_none() {
                return Err("give a title, a description or both".into());
            }
            let id = ticket_for(cx, None).await?;
            let patch =
                pdt_core::ticket::TicketPatch { title: a.title, description: a.description, ..Default::default() };
            let valid = patch.validate().map_err(|e| api(e.into()))?;
            let t = st
                .store
                .patch_ticket_by(&id, &valid, Actor::Agent, now_ms())
                .await
                .map_err(|e| api(e.into()))?
                .ok_or("that ticket no longer exists")?;
            Ok(json!({"updated": true, "key": t.summary.key, "title": t.summary.title}))
        }
        "add_review_comment" => {
            let a: OutcomeComment = args(a)?;
            let c = add_review_comment(cx, &a).await?;
            Ok(json!({"commented": true, "commentId": c.id, "file": c.path, "line": c.line}))
        }
        "catalog_search" => {
            let a: CatalogSearch = args(a)?;
            let words: Vec<String> =
                a.query.unwrap_or_default().to_lowercase().split_whitespace().map(String::from).collect();
            let kinds: Vec<Value> = crate::workflow::catalog::listed(st)
                .into_iter()
                .filter(|k| a.category.is_none_or(|c| c == k.category))
                .filter(|k| {
                    let text = format!("{} {} {}", k.id, k.title, k.description).to_lowercase();
                    words.iter().all(|w| text.contains(w.as_str()))
                })
                .map(|k| json!({"id": k.id, "title": k.title, "category": k.category, "description": k.description}))
                .collect();
            Ok(json!({"kinds": kinds}))
        }
        "catalog_get" => {
            let a: CatalogGet = args(a)?;
            let k = crate::workflow::catalog::listed_spec(st, &a.id)
                .ok_or_else(|| format!("no node kind is called {}", a.id))?;
            serde_json::to_value(k).map_err(|e| e.to_string())
        }
        "workflow_get" => {
            let a: WorkflowGet = args(a)?;
            let id = match (a.id, a.name) {
                (Some(id), _) => id,
                (None, Some(name)) => st
                    .store
                    // The PM looks across every project.
                    .list_workflows(match crate::pm::is_pm_session(st, cx.session).await {
                        true => None,
                        false => Some(&cx.session.project_id),
                    })
                    .await
                    .map_err(store)?
                    .into_iter()
                    .find(|w| w.name.eq_ignore_ascii_case(name.trim()))
                    .map(|w| w.id)
                    .ok_or_else(|| format!("no workflow is called {name}"))?,
                (None, None) => return Err("give the workflow's id or name".into()),
            };
            let w = crate::workflow::routes::load(st, &id).await.map_err(api)?;
            let d = crate::workflow::routes::detail(st, w).await.map_err(api)?;
            serde_json::to_value(d).map_err(|e| e.to_string())
        }
        "workflow_validate" => {
            let a: crate::workflow::routes::ValidateRequest = args(a)?;
            let Json(issues) =
                crate::workflow::routes::validate_doc(State(st.clone()), Ok(Json(a))).await.map_err(api)?;
            Ok(json!({"issues": issues}))
        }
        "workflow_dry_run" => {
            let a: crate::workflow::routes::DryRunRequest = args(a)?;
            let Json(r) = crate::workflow::routes::dry_run(State(st.clone()), Ok(Json(a))).await.map_err(api)?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }
        "workflow_save" => {
            let a: WorkflowSave = args(a)?;
            let d = match a.id {
                // An agent's change to a workflow is a proposal the user reviews (Phase 7d spec §6.2).
                Some(id) => {
                    let s = crate::workflow::routes::SaveWorkflow { doc: Some(a.doc), note: a.note, propose: true };
                    let Json(d) =
                        crate::workflow::routes::save(State(st.clone()), axum::extract::Path(id), Ok(Json(s)))
                            .await
                            .map_err(api)?;
                    d
                }
                None => {
                    let scope = a.scope.unwrap_or(Scope::Project);
                    let project_id = match (scope, a.project_id) {
                        (Scope::Project, Some(p)) => Some(crate::pm::project_id(st, &p).await?),
                        // The PM is in no project: it names one.
                        (Scope::Project, None) if crate::pm::is_pm_session(st, cx.session).await => {
                            return Err("name the workflow's project (`projectId`: its key or id)".into());
                        }
                        (Scope::Project, None) => Some(cx.session.project_id.clone()),
                        _ => None,
                    };
                    crate::workflow::routes::create_workflow(
                        st,
                        &a.doc,
                        Some(scope),
                        project_id,
                        false,
                        &a.note,
                        Author::Agent,
                    )
                    .await
                    .map_err(api)?
                }
            };
            Ok(json!({"workflowId": d.workflow.id, "version": d.workflow.version, "proposed": d.workflow.proposed,
                      "enabled": d.workflow.enabled, "issues": d.issues}))
        }
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
                    json!({"key": t.key, "id": t.id, "title": t.title, "stage": t.stage, "priority": t.priority,
                           "labels": t.labels, "state": pdt_core::pm::ticket_state(t)})
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
        "search_notes" => {
            let a: SearchNotes = args(a)?;
            let project_id = match a.project.as_deref().filter(|p| !p.trim().is_empty()) {
                Some(p) => Some(crate::pm::project_id(st, p).await?),
                None => None,
            };
            let f = pdt_store::NoteFilter {
                q: a.query,
                project_id,
                kind: a.kind,
                pinned: a.pinned,
                limit: a.limit.unwrap_or(20).clamp(1, 100),
            };
            let notes: Vec<Value> = st
                .store
                .search_notes(&f)
                .await
                .map_err(store)?
                .into_iter()
                .map(|n| {
                    let by = if n.kind == pdt_core::pm::NoteKind::User { "the user" } else { "the PM" };
                    let mut v = json!(n);
                    v["writtenBy"] = json!(by);
                    v
                })
                .collect();
            Ok(json!({"notes": notes}))
        }
        "list_todos" => {
            let a: ListTodos = args(a)?;
            let f = crate::todos::filter(st, a.project.as_deref(), a.done).await.map_err(api)?;
            let todos: Vec<Value> = st
                .store
                .list_todos(&f)
                .await
                .map_err(store)?
                .into_iter()
                .map(|t| {
                    json!({"id": t.id, "title": t.title, "done": t.done, "project": t.project_key,
                           "ticket": t.ticket_key, "ticketTitle": t.ticket_title, "createdAt": t.created_at,
                           "updatedAt": t.updated_at, "doneAt": t.done_at, "dueOn": t.due_on, "dueTime": t.due_time,
                           "recurrence": t.recurrence})
                })
                .collect();
            Ok(json!({"todos": todos}))
        }
        "save_note" => {
            let a: SaveNote = args(a)?;
            let n = match a.id {
                Some(id) => {
                    pm_may_write(st, &id).await?;
                    let p = pdt_core::pm::NotePatch {
                        project: a.project,
                        title: Some(a.title),
                        body: Some(a.body),
                        pinned: a.pinned,
                    };
                    crate::pm::edit_note(st, &id, p).await.map_err(api)?
                }
                None => {
                    let n = pdt_core::pm::NewNote {
                        project: a.project,
                        title: a.title,
                        body: a.body,
                        pinned: a.pinned,
                        kind: None,
                    };
                    crate::pm::add_note(st, n).await.map_err(api)?
                }
            };
            Ok(json!({"saved": true, "id": n.id, "pinned": n.pinned}))
        }
        "delete_note" => {
            let a: DeleteNote = args(a)?;
            pm_may_write(st, &a.id).await?;
            if !st.store.delete_note(&a.id).await.map_err(store)? {
                return Err(format!("there is no note {}", a.id));
            }
            Ok(json!({"deleted": true}))
        }
        "write_journal" => {
            let a: WriteJournal = args(a)?;
            let body = pdt_core::pm::valid_journal(&a.body)?;
            let n = crate::pm::write_journal(st, cx.session, &body).await.map_err(api)?;
            Ok(json!({"written": true, "id": n.id}))
        }
        "create_ticket" => {
            let a: CreateTicket = args(a)?;
            let project = crate::pm::project_id(st, &a.project).await?;
            confirmable(cx, name, &pdt_core::pm::AskTarget::Project(project.clone())).await?;
            let parent = match a.parent.as_deref() {
                Some(p) => Some(resolve_ticket(st, p, Some(&project)).await?),
                None => None,
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
            let valid = input.validate().map_err(|e| api(e.into()))?;
            let id = uuid::Uuid::now_v7().to_string();
            let t = st.store.create_ticket(&id, &project, &valid, now_ms()).await.map_err(|e| api(e.into()))?;
            record_agent_change(cx, &t.summary.id, "Created by the PM.").await;
            Ok(json!({"created": true, "key": t.summary.key, "id": t.summary.id, "stage": t.summary.stage}))
        }
        "edit_ticket" => {
            let a: EditTicket = args(a)?;
            let id = resolve_ticket(st, &a.ticket, Some(&cx.session.project_id)).await?;
            confirmable(cx, name, &pdt_core::pm::AskTarget::Ticket(id.clone())).await?;
            let project = st.store.get_ticket(&id).await.map_err(store)?.map(|t| t.summary.project_id);
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
            if valid == Default::default() {
                return Err("give at least one field to change".into());
            }
            let t = st
                .store
                .patch_ticket_by(&id, &valid, Actor::Agent, now_ms())
                .await
                .map_err(|e| api(e.into()))?
                .ok_or("that ticket no longer exists")?;
            if let Some(stage) = valid.stage.filter(|s| s.is_closed()) {
                crate::start::end_for_close(st, &id, stage, Actor::Agent).await.map_err(api)?;
            }
            Ok(json!({"updated": true, "key": t.summary.key, "title": t.summary.title, "stage": t.summary.stage,
                      "parent": t.summary.parent.map(|p| p.key)}))
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
            let done = match linked {
                true => st.store.add_link_by(from, to, kind, Actor::Agent, now_ms()).await,
                false => st.store.remove_link_by(from, to, kind, Actor::Agent, now_ms()).await,
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
            };
            let m = st.store.queue_session_message(&m).await.map_err(store)?;
            Ok(json!({"queued": true, "messageId": m.id}))
        }
        "discord_get" => crate::discord::get(st, args(a)?).await,
        "discord_change" => {
            let a: crate::discord::DiscordChange = args(a)?;
            confirmable(cx, name, &pdt_core::pm::AskTarget::Connection(a.connection.clone())).await?;
            crate::discord::change(st, a).await
        }
        "discord_webhook" => {
            let a: crate::discord::DiscordWebhookArgs = args(a)?;
            confirmable(cx, name, &pdt_core::pm::AskTarget::Connection(a.connection.clone())).await?;
            crate::discord::webhook_tool(st, a).await
        }
        other => Err(format!("no tool is called {other}")),
    }
}

/// An asking tool from an agent that can't ask the user per call (Codex, Devin: PDT's tools run without asking
/// there) runs only where the project lets the PM act. Claude asks through its permission prompt (spec §4).
async fn confirmable(cx: &Cx<'_>, tool: &str, target: &pdt_core::pm::AskTarget) -> Result<(), String> {
    use pdt_core::session::AgentKind;
    if !pdt_core::persona::asks(tool) || cx.session.agent.unwrap_or(AgentKind::Claude) == AgentKind::Claude {
        return Ok(());
    }
    if crate::pm::acts_without_asking(cx.st, target).await {
        return Ok(());
    }
    Err(format!(
        "{tool} needs the user's yes, and PDT can only ask for it in a Claude session. Tell the user what you would \
         do so they can do it, or ask them to let the PM act (the project's settings, or the Discord bot connection's)."
    ))
}

/// Notes on the ticket's timeline that an agent changed it through PDT's tools (principle 5: changes are on record).
async fn record_agent_change(cx: &Cx<'_>, ticket_id: &str, text: &str) {
    let data = json!({"sessionId": cx.session.id, "persona": cx.session.persona.as_ref().map(|p| &p.name)});
    if let Err(e) = cx.st.store.add_comment_by(ticket_id, text, Actor::Agent, Some(data), now_ms()).await {
        tracing::warn!(ticket = %ticket_id, error = %e, "could not record the PM's change on the timeline");
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
    Ok(json!({"projects": out_projects, "inProgress": in_flight, "inbox": inbox, "runningSessions": sessions}))
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

/// A ticket's id from its id or key (`PDT-12`; a bare number is in `project`).
pub(crate) async fn resolve_ticket(st: &AppState, given: &str, project: Option<&str>) -> Result<String, String> {
    let g = given.trim();
    if st.store.get_ticket(g).await.map_err(store)?.is_some() {
        return Ok(g.to_string());
    }
    let k = pdt_core::ticket::parse_key(g).ok_or_else(|| format!("{g} is not a ticket key or id"))?;
    let project = match &k.key {
        Some(key) => st
            .store
            .list_projects()
            .await
            .map_err(store)?
            .into_iter()
            .find(|p| p.key.eq_ignore_ascii_case(key))
            .map(|p| p.id)
            .ok_or_else(|| format!("no project has the key {key}"))?,
        None => project.ok_or_else(|| format!("{g} needs its project's key, such as PDT-{}", k.number))?.to_string(),
    };
    st.store
        .ticket_id_by_number(&project, k.number)
        .await
        .map_err(store)?
        .ok_or_else(|| format!("there is no ticket {g}"))
}

/// The ticket a tool means: `given` (a key or id), else the session's (its attempt's, or its agent step's).
async fn ticket_for(cx: &Cx<'_>, given: Option<&str>) -> Result<String, String> {
    if let Some(g) = given.map(str::trim).filter(|g| !g.is_empty()) {
        return resolve_ticket(cx.st, g, Some(&cx.session.project_id)).await;
    }
    let st = cx.st;
    session_ticket(st, cx.session)
        .await
        .ok_or_else(|| "this session works on no ticket; name one (`ticket`: a key such as PDT-12)".to_string())
}

/// The ticket a session works on: its attempt's, else its agent step's.
pub(crate) async fn session_ticket(st: &AppState, s: &Session) -> Option<String> {
    if let Some(t) = &s.ticket {
        return Some(t.id.clone());
    }
    st.store.active_agent_step(&s.id).await.ok().flatten().and_then(|a| a.ticket_id)
}

async fn get_ticket(cx: &Cx<'_>, id: &str) -> Result<Value, String> {
    let st = cx.st;
    let Json(ticket) =
        crate::tickets::get_one(State(st.clone()), axum::extract::Path(id.to_string())).await.map_err(api)?;
    let comments: Vec<Value> = st
        .store
        .comments(id)
        .await
        .map_err(store)?
        .into_iter()
        .map(|(actor, body, at)| json!({"by": actor, "body": body, "at": at}))
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
    let mut v = serde_json::to_value(ticket).map_err(|e| e.to_string())?;
    v["comments"] = json!(comments);
    v["outcomes"] = json!(outcomes);
    if !step.is_null() {
        v["step"] = step;
    }
    Ok(v)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_listed_once_with_an_object_schema() {
        let names: Vec<String> = infos().into_iter().map(|t| t.name).collect();
        assert_eq!(names, pdt_core::persona::ALL_TOOLS.to_vec());
        for t in infos() {
            assert_eq!(t.input_schema["type"], "object", "{}", t.name);
            assert!(t.input_schema.get("$schema").is_none());
        }
        let s = &infos()[1].input_schema;
        assert!(s["required"].as_array().unwrap().iter().any(|r| r == "outcome"), "{s}");
    }
}
