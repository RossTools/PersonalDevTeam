//! pdt-mcp's tools (Phase 7b spec §2.2). Inputs are the API's request types where one exists (their JSON Schema from
//! schemars), and each tool runs the function the matching route runs, so the API and the tools can't drift.

use crate::error::ApiError;
use crate::integrations::Tool;
use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::Category;
use axum::Json;
use axum::extract::State;
use pdt_core::inbox::Question;
use pdt_core::integration::ToolDecl;
use pdt_core::mcp::input_schema;
use pdt_core::pm::AskTarget;
use pdt_core::review::{CommentSide, NewReviewComment};
use pdt_core::session::Session;
use pdt_core::ticket::{
    AGENTIC_LABEL, Actor, AgentBy, By, EventKind, TicketEvent, TicketSource, TicketStage, valid_comment,
};
use pdt_core::workflow::agent::OutcomeComment;
use pdt_core::workflow::api::{Author, Scope};
use pdt_core::workflow::doc::WorkflowDoc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// What a tool call knows: PDT, the calling session and the call's arguments.
pub struct Cx<'a> {
    pub st: &'a AppState,
    pub session: &'a Session,
    /// The arguments as the agent gave them: an asking tool's Inbox item shows them (PDT-256).
    pub input: &'a Value,
    /// The user allowed this very call in the Inbox: an asking tool goes ahead without asking again.
    pub confirmed: bool,
}

/// What a call PDT makes itself (not an agent's tool call) has as arguments.
pub(crate) static NO_INPUT: Value = Value::Null;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

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
struct Unwatch {
    /// The watch's id.
    id: String,
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
struct WorkflowRun {
    #[serde(default)]
    id: Option<String>,
    /// The workflow's name, when you don't know its id.
    #[serde(default)]
    name: Option<String>,
    /// The project it runs in (its key or id): its `projectId` in the run's input.
    project: String,
    /// The run's input, read as `{{trigger.x}}` (the workflow's description says what it reads).
    #[serde(default)]
    input: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowExport {
    /// The workflows' ids.
    #[serde(default)]
    ids: Vec<String>,
    /// Or their names, when you don't know the ids.
    #[serde(default)]
    names: Vec<String>,
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
    /// draft, ready, todo, in_progress, done or wont_do. Done and wont_do end its attempt; draft, ready and todo stop
    /// its work like `stop_ticket` (keeping the worktree).
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendToSession {
    /// The session's id.
    session: String,
    /// The message. PDT delivers it when the session is idle, marked as from the PM.
    text: String,
}

pub(crate) fn schema<T: JsonSchema>() -> Value {
    input_schema(crate::workflow::catalog::schema_of::<T>())
}

/// PDT's own tools, in `tools/list` order ([`crate::integrations::core`]).
pub(crate) fn defs() -> Vec<Tool> {
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
        Tool {
            decl: ToolDecl::new("list_secrets", "See the names of the secrets it may use").read_only().pm(),
            title: "List the secrets",
            description: "The secrets this session may use: their names and descriptions, never their values.",
            schema: schema::<NoArgs>,
        },
        Tool {
            decl: ToolDecl::new("request_secret", "Ask you for a secret (you type it into the Inbox, never the chat)")
                .pm(),
            title: "Ask the user for a secret",
            description: "Asks the user for a secret (an API key, a token, a password) by name: they type it into PDT's \
                          Inbox, never into this chat. You get `stored: true` once it is (at once if it already is), \
                          never the value: use it with run_with_secrets or write_secret_file. Never ask for a secret \
                          in the chat.",
            schema: schema::<crate::vault::RequestSecret>,
        },
        Tool {
            decl: ToolDecl::new(
                "run_with_secrets",
                "Run a command with secrets in its environment (it never sees them)",
            )
            .pm(),
            title: "Run a command with secrets",
            description: "Runs a command line (sh -c; cmd /C on Windows) with secrets in its environment: an env value \
                          `secret:<name>` is that secret's value. Its output comes back with every secret's value \
                          replaced by `[secret:<name>]`. Each use is recorded.",
            schema: schema::<crate::vault::RunWithSecrets>,
        },
        Tool {
            decl: ToolDecl::new("write_secret_file", "Write a file (a .env) with secrets filled in, without seeing it")
                .pm(),
            title: "Write a file with secrets",
            description: "Writes a file (a .env, a config file) from a template with `{{secret.<name>}}` where each \
                          secret goes, filled in by PDT; the content is not echoed back. Each use is recorded.",
            schema: schema::<crate::vault::WriteSecretFile>,
        },
        Tool {
            decl: ToolDecl::new("update_ticket", "Rewrite its own ticket's title and description"),
            title: "Update the ticket",
            description: "Rewrites this session's ticket: its title and/or description (as `PATCH /api/tickets/{id}`, \
                          recorded as the agent's edit). Only this session's own ticket.",
            schema: schema::<UpdateTicket>,
        },
        Tool {
            decl: ToolDecl::new("catalog_search", "Search the workflow node catalog").read_only().pm(),
            title: "Search the node catalog",
            description: "Lists workflow node kinds (triggers, conditions, actions, gates, waits) matching the \
                          words: id, title, category, description. MCP servers' tools are listed too, as \
                          `mcp:<server>/<tool>` (a `preset` mcp.tool node). catalog_get gives one kind's schemas.",
            schema: schema::<CatalogSearch>,
        },
        Tool {
            decl: ToolDecl::new("catalog_get", "Read a catalog node").read_only().pm(),
            title: "Get a node kind",
            description: "One node kind with its config and output JSON Schemas and its ports.",
            schema: schema::<CatalogGet>,
        },
        Tool {
            decl: ToolDecl::new("workflow_get", "Read a workflow").read_only().pm(),
            title: "Get a workflow",
            description: "A workflow (by id or name) with its current document and validation issues, and runInputs: \
                          the fields its Run now trigger declares (what workflow_run takes).",
            schema: schema::<WorkflowGet>,
        },
        Tool {
            decl: ToolDecl::new("workflow_validate", "Check a workflow document").read_only().pm(),
            title: "Validate a workflow",
            description: "Checks a workflow document: errors and warnings.",
            schema: schema::<crate::workflow::routes::ValidateRequest>,
        },
        Tool {
            decl: ToolDecl::new("workflow_dry_run", "Try a workflow without running it").read_only().pm(),
            title: "Dry-run a workflow",
            description: "Runs a document (or a saved workflow) against an event in memory, changing nothing: which \
                          triggers match and what each step would do.",
            schema: schema::<crate::workflow::routes::DryRunRequest>,
        },
        Tool {
            decl: ToolDecl::new("workflow_save", "Save a workflow (a new version)").pm(),
            title: "Save a workflow",
            description: "Proposes a document as a new version of a workflow (the user reviews and accepts it), or \
                          saves it as a new workflow, which starts disabled: the user reviews and enables it.",
            schema: schema::<WorkflowSave>,
        },
        Tool {
            decl: ToolDecl::new("workflow_export", "Export workflows as text to paste into another PDT")
                .read_only()
                .pm(),
            title: "Export workflows",
            description: "Workflows' current documents as one export text (format pdt-workflow): project ids become \
                          {\"$project\": \"<KEY>\"} placeholders and `requires` lists the connections, personas and \
                          projects they need. Paste it into Import on another PDT, or give it to workflow_import there.",
            schema: schema::<WorkflowExport>,
        },
        Tool {
            decl: ToolDecl::new("workflow_run", "Run an enabled workflow now").asks().pm(),
            title: "Run a workflow",
            description: "Runs an enabled workflow that has a Run now trigger, in a project, with an input its steps \
                          read as {{trigger.x}}: e.g. {\"environment\": \"staging\"} for a deploy workflow. When the \
                          trigger declares inputs (workflow_get's runInputs), the input must be those fields: defaults \
                          fill in, and anything else is refused with the list; without, its description says what it \
                          reads. The user confirms each call. Returns the run's id (list_workflow_runs follows it).",
            schema: schema::<WorkflowRun>,
        },
        Tool {
            decl: ToolDecl::new("workflow_import", "Import pasted workflows (disabled)").asks().pm(),
            title: "Import workflows",
            description: "Imports an export text (or a bare workflow document). With `check: true` it only reports \
                          the workflows, names that clash, and how each connection, persona and project they need \
                          stands here (found, missing, kind_mismatch). Then `mapping` maps each missing one to a local \
                          name (a project key to a local project's key or id) or null to leave it unmapped, and \
                          `targets` (one per workflow, in order) give scope/projectId and, on a name clash, `copy` or \
                          `versionOf` (a proposed version). New workflows start disabled. The user confirms each call.",
            schema: schema::<pdt_core::workflow::transfer::ImportRequest>,
        },
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
        Tool {
            decl: ToolDecl::new("search_notes", "Search the PM's notes and journal").read_only().pm(),
            title: "Search notes",
            description: "The PM's notes and journal entries, and the user's own notes (kind user, writtenBy \
                          \"the user\"), matching the words (full-text), host-wide and per project; every pinned \
                          note when you give no words and pinned: true.",
            schema: schema::<SearchNotes>,
        },
        Tool {
            decl: ToolDecl::new("save_note", "Save a note").pm(),
            title: "Save a note",
            description: "Adds a note (or replaces one by id): what PDT's data can't tell you, such as the user's \
                          preferences, decisions, people and rules at work. Pinned notes go in every briefing. The \
                          user's own notes (kind user) are theirs: you can't change them.",
            schema: schema::<SaveNote>,
        },
        Tool {
            decl: ToolDecl::new("delete_note", "Delete a note").destructive().pm(),
            title: "Delete a note",
            description: "Removes a note that no longer holds (not one of the user's own).",
            schema: schema::<DeleteNote>,
        },
        Tool {
            decl: ToolDecl::new("write_journal", "Write the session's journal entry").pm(),
            title: "Write the journal",
            description: "This conversation's journal entry, which later PM conversations' briefings include. Write \
                          it when PDT asks for a checkpoint or says the conversation is ending; each call replaces \
                          the entry.",
            schema: schema::<WriteJournal>,
        },
        Tool {
            decl: ToolDecl::new("list_todos", "Read your todo list").read_only().pm(),
            title: "List todos",
            description: "The user's own todo list (open ones in their order, then done ones): title, body (the user's notes on it, Markdown), done, project, \
                          linked ticket, due date (dueOn, YYYY-MM-DD, and dueTime, HH:MM, the user's local time), how it repeats \
                          (recurrence: a rule, every day/weekday/week/month, and whether the next one counts from \
                          the due date or when it's done; checking it off makes the next one) and dates. Read-only: the user edits it in PDT.",
            schema: schema::<ListTodos>,
        },
        Tool {
            decl: ToolDecl::new("watch", "Watch a ticket or workflow run").pm(),
            title: "Watch a ticket or run",
            description: "Tells this conversation when something happens to a ticket or a workflow run: PDT sends a \
                          `[PDT] Watched …` message once you are idle (several within a minute in one). `events`: \
                          `finished` (Done, Won't do, merged; a run succeeded, failed or was cancelled), `outcome` \
                          (an agent step reported), `comment` (someone else commented), `needs_you` (it waits on \
                          the user), `failed` (CI failed, the merge queue dropped it, the run failed); default \
                          finished, failed and needs_you. With `once` (default true) the watch ends when the target \
                          finishes (one already finished is refused). Use it when the user asks you to keep an eye on something, instead of checking \
                          again and again. It asks nobody: only this conversation is told.",
            schema: schema::<crate::watches::NewWatch>,
        },
        Tool {
            decl: ToolDecl::new("unwatch", "Stop watching a ticket or run").pm(),
            title: "Stop watching",
            description: "Ends one of this conversation's watches, by its id (list_watches gives them).",
            schema: schema::<Unwatch>,
        },
        Tool {
            decl: ToolDecl::new("list_watches", "List this conversation's watches").read_only().pm(),
            title: "List watches",
            description: "This conversation's watches: what each watches, what it hears about, and whether it ends \
                          once the target finishes.",
            schema: schema::<NoArgs>,
        },
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
        Tool {
            decl: ToolDecl::new("send_to_session", "Message a session").asks().pm(),
            title: "Message a session",
            description: "Sends an agent session a message, delivered when it is idle and marked as from the PM. The \
                          user confirms each call unless the project lets you act.",
            schema: schema::<SendToSession>,
        },
    ]
}

pub(crate) fn args<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    let v = if v.is_null() { json!({}) } else { v };
    serde_json::from_value(v).map_err(|e| format!("the arguments don't fit the tool's schema: {e}"))
}

pub(crate) fn api(e: ApiError) -> String {
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

/// Runs one of PDT's own tools. Err is a message for the agent.
pub(crate) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    // No secrets near untrusted input (PDT-201).
    if matches!(name, "request_secret" | "run_with_secrets" | "write_secret_file") {
        crate::untrusted::refuse_secrets(st, cx.session).await?;
    }
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
        "list_secrets" => {
            let _: NoArgs = args(a)?;
            crate::vault::list_tool(st, cx.session).await
        }
        "request_secret" => crate::vault::request_tool(st, cx.session, args(a)?).await,
        "run_with_secrets" => crate::vault::run_tool(st, cx.session, args(a)?).await,
        "write_secret_file" => crate::vault::write_file_tool(st, cx.session, args(a)?).await,
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
                .patch_ticket_by(&id, &valid, agent_by(st, cx.session).await, now_ms())
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
                (None, Some(name)) => workflow_by_name(cx, &name).await?,
                (None, None) => return Err("give the workflow's id or name".into()),
            };
            let w = crate::workflow::routes::load(st, &id).await.map_err(api)?;
            let d = crate::workflow::routes::detail(st, w).await.map_err(api)?;
            let mut v = serde_json::to_value(&d).map_err(|e| e.to_string())?;
            // What workflow_run takes: the Run now trigger's declared inputs (PDT-301).
            if let Some(n) = d.doc.nodes.iter().find(|n| n.kind == crate::workflow::nodes::trigger::MANUAL) {
                let c = st.registry().catalog().current_config(n).unwrap_or_default();
                v["runInputs"] = c.get("inputs").cloned().unwrap_or_else(|| json!([]));
            }
            Ok(v)
        }
        "workflow_export" => {
            let a: WorkflowExport = args(a)?;
            let mut ids = a.ids;
            for n in &a.names {
                ids.push(workflow_by_name(cx, n).await?);
            }
            if ids.is_empty() {
                return Err("give the workflows' ids or names".into());
            }
            let e = crate::workflow::transfer::export(st, &ids).await.map_err(api)?;
            Ok(json!({"text": pdt_core::workflow::transfer::export_text(&e), "workflows": e.workflows.len(),
                      "requires": e.requires}))
        }
        "workflow_import" => {
            let a: pdt_core::workflow::transfer::ImportRequest = args(a)?;
            if a.check {
                let c = crate::workflow::transfer::check(st, &a.text).await.map_err(api)?;
                return serde_json::to_value(c).map_err(|e| e.to_string());
            }
            confirmable(cx, name, &AskTarget::Always).await?;
            let r = crate::workflow::transfer::import(st, &a, Author::Agent).await.map_err(api)?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }
        "workflow_run" => {
            let a: WorkflowRun = args(a)?;
            let id = match (a.id, a.name) {
                (Some(id), _) => id,
                (None, Some(name)) => workflow_by_name(cx, &name).await?,
                (None, None) => return Err("give the workflow's id or name".into()),
            };
            let w = crate::workflow::routes::load(st, &id).await.map_err(api)?;
            if !w.enabled {
                return Err(format!("{} is off: the user turns it on in Workflows first", w.name));
            }
            let project = crate::pm::project_id(st, &a.project).await?;
            confirmable(cx, name, &AskTarget::Project(project.clone())).await?;
            let mut input = a.input;
            input.insert("projectId".into(), json!(project));
            let run = crate::workflow::engine::start_manual(st, &w, Value::Object(input), None).await?;
            Ok(json!({"runId": run, "workflow": w.name}))
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
                    let by =
                        cx.session.persona.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| cx.session.title.clone());
                    let by = Some(by).filter(|b| !b.trim().is_empty());
                    let s = crate::workflow::routes::SaveWorkflow { doc: Some(a.doc), note: a.note, propose: true, by };
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
                    json!({"id": t.id, "title": t.title, "body": t.body, "done": t.done, "project": t.project_key,
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
        "watch" => {
            let a: crate::watches::NewWatch = args(a)?;
            let w = crate::watches::watch(st, &a, Some(&cx.session.id)).await.map_err(api)?;
            let info = crate::watches::infos(st, vec![w]).await.map_err(api)?;
            Ok(json!({"watching": info}))
        }
        "unwatch" => {
            let a: Unwatch = args(a)?;
            crate::watches::unwatch(st, &a.id, Some(&cx.session.id)).await.map_err(api)?;
            Ok(json!({"unwatched": a.id}))
        }
        "list_watches" => {
            let _: NoArgs = args(a)?;
            let watches = st.store.watches_by(Some(&cx.session.id)).await.map_err(store)?;
            Ok(json!({"watches": crate::watches::infos(st, watches).await.map_err(api)?}))
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
                    crate::start::end_for_close(st, &id, stage, &by).await.map_err(api)?;
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
            Ok(json!({"queued": true, "messageId": m.id}))
        }
        other => Err(format!("no tool is called {other}")),
    }
}

/// A workflow's id from its name: in the session's project (with host-wide and personal ones), or for the PM anywhere.
async fn workflow_by_name(cx: &Cx<'_>, name: &str) -> Result<String, String> {
    let st = cx.st;
    st.store
        // The PM looks across every project.
        .list_workflows(match crate::pm::is_pm_session(st, cx.session).await {
            true => None,
            false => Some(&cx.session.project_id),
        })
        .await
        .map_err(store)?
        .into_iter()
        .filter(|w| !crate::workflow::builtins::hidden(&st.registry(), w))
        .find(|w| w.name.eq_ignore_ascii_case(name.trim()))
        .map(|w| w.id)
        .ok_or_else(|| format!("no workflow is called {name}"))
}

/// An asking tool from an agent that can't ask the user per call (Codex, Devin: PDT's tools run without asking
/// there) runs where the project lets the PM act; elsewhere PDT asks in its Inbox (PDT-256): the call waits there, and
/// the user's Allow runs it and tells the agent the result. Claude asks through its permission prompt (spec §4).
pub(crate) async fn confirmable(cx: &Cx<'_>, tool: &str, target: &pdt_core::pm::AskTarget) -> Result<(), String> {
    use pdt_core::session::AgentKind;
    if cx.confirmed
        || !cx.st.registry().tool_table().asks(tool)
        || cx.session.agent.unwrap_or(AgentKind::Claude) == AgentKind::Claude
    {
        return Ok(());
    }
    if crate::pm::acts_without_asking(cx.st, target).await {
        return Ok(());
    }
    let item = crate::workflow::agent::ask_confirm(cx.st, cx.session, tool, cx.input).await?;
    Err(format!(
        "{tool} needs the user's yes, so PDT put this call in its Inbox for them to Allow or Deny (item {}). Don't call \
         it again: if they allow it, PDT runs it as you gave it and sends you a message with the result; if they deny \
         it, you hear that instead. End your turn now.",
        item.id
    ))
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

/// A ticket's id from its id or key (`ABC-12`; a bare number is in `project`).
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
        None => project
            .ok_or_else(|| format!("{g} needs its project's key in front, such as ABC-{}", k.number))?
            .to_string(),
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
        .ok_or_else(|| "this session works on no ticket; name one (`ticket`: a key such as ABC-12)".to_string())
}

/// Who a session's ticket changes and comments are by (PDT-163): the agent, its persona and the ticket it works on.
pub(crate) async fn agent_by(st: &AppState, s: &Session) -> By {
    let from = match session_ticket(st, s).await {
        Some(id) => st.store.get_ticket_summary(&id).await.ok().flatten(),
        None => None,
    };
    By::agent(AgentBy {
        session_id: s.id.clone(),
        project_id: Some(s.project_id.clone()),
        persona: s.persona.as_ref().map(|p| p.name.clone()),
        from_ticket: from.as_ref().map(|t| t.key.clone()),
        from_ticket_id: from.map(|t| t.id),
    })
}

/// The ticket a session works on: its attempt's, else its agent step's.
pub(crate) async fn session_ticket(st: &AppState, s: &Session) -> Option<String> {
    if let Some(t) = &s.ticket {
        return Some(t.id.clone());
    }
    st.store.active_agent_step(&s.id).await.ok().flatten().and_then(|a| a.ticket_id)
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
             `GET <PDT's API>/documents/<id>/content` gives a doc as Markdown, a sheet as its cells."
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
