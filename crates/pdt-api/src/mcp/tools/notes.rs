//! The PM's notes, journal and to-dos.

use super::*;

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

pub(super) fn defs() -> Vec<Tool> {
    vec![
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
    ]
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
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
            let project = note_project(st, n.project_id.as_deref()).await;
            Ok(json!({"saved": true, "id": n.id, "title": n.title, "project": project, "pinned": n.pinned}))
        }
        "delete_note" => {
            let a: DeleteNote = args(a)?;
            pm_may_write(st, &a.id).await?;
            // What it was, for the conversation's card.
            let note = st.store.get_note(&a.id).await.map_err(store)?;
            if !st.store.delete_note(&a.id).await.map_err(store)? {
                return Err(format!("there is no note {}", a.id));
            }
            let project = note_project(st, note.as_ref().and_then(|n| n.project_id.as_deref())).await;
            Ok(json!({"deleted": true, "title": note.as_ref().map(|n| &n.title), "project": project,
                      "pinned": note.as_ref().map(|n| n.pinned)}))
        }
        "write_journal" => {
            let a: WriteJournal = args(a)?;
            let body = pdt_core::pm::valid_journal(&a.body)?;
            let n = crate::pm::write_journal(st, cx.session, &body).await.map_err(api)?;
            Ok(json!({"written": true, "id": n.id}))
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
        other => Err(format!("no tool is called {other}")),
    }
}

/// A note's project key, for the conversation's card (None: host-wide, or not found: the note is saved either way).
async fn note_project(st: &AppState, project_id: Option<&str>) -> Option<String> {
    st.store.get_project(project_id?).await.ok().flatten().map(|p| p.key)
}

/// Refuses a PM tool's change to one of the user's own notes (PDT-126); a missing note is left to the tool.
async fn pm_may_write(st: &AppState, id: &str) -> Result<(), String> {
    match st.store.get_note(id).await.map_err(store)? {
        Some(n) if !n.kind.pm_may_write() => Err(format!("note {id} is the user's own: only they can change it")),
        _ => Ok(()),
    }
}
