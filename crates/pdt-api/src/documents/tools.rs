//! The document tools (PDT-317): agents and the PM list, read and change documents through PDT, whichever provider
//! keeps them, and open one beside their conversation for the user to watch. Each is an action with only a tool face
//! (PDT-318 gives some a node face). Reads don't ask; writes (edits, new documents, links, comment replies and
//! resolves) ask the user unless a project the document is linked to lets the agent act. What other people wrote in a
//! document's comments comes fenced as outside input (PDT-202).

use super::routes::{
    DocumentLinkInput, NewDocument, apply_edits, create_document, document, link_of, open_beside_doc, provider_of,
    with_comments,
};
use crate::integrations::Integration;
use crate::integrations::action::{Action, ActionCx, AnyAction, ToolFace};
use crate::mcp::tools::api;
use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::BoxFut;
use pdt_core::document::{Document, DocumentComment, DocumentContent, DocumentEdit, DocumentFilter, DocumentKind};
use pdt_core::document_ops::{cells_in, headings, section, used_size};
use pdt_core::fence::Fence;
use pdt_core::integration::ToolDecl;
use pdt_core::pm::AskTarget;
use pdt_core::ticket::{Actor, By};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

/// The most cells `sheet_read` gives back at once.
const MAX_CELLS: usize = 2000;

pub struct DocumentTools;

impl Integration for DocumentTools {
    fn id(&self) -> &'static str {
        "documents"
    }

    fn actions(&self) -> Vec<Arc<dyn AnyAction>> {
        vec![
            Arc::new(DocumentsList),
            Arc::new(DocumentGet),
            Arc::new(DocRead),
            Arc::new(DocEdit),
            Arc::new(SheetRead),
            Arc::new(SheetEdit),
            Arc::new(DocumentCreate),
            Arc::new(DocumentLink),
            Arc::new(CommentsList),
            Arc::new(CommentReply),
            Arc::new(CommentResolve),
            Arc::new(DocumentOpenBeside),
        ]
    }

    fn approval_title(&self, tool: &str, input: &Value) -> Option<String> {
        let n = input.get("edits").and_then(Value::as_array).map_or(0, Vec::len);
        let title = input.get("title").and_then(Value::as_str).unwrap_or("a document");
        Some(match tool {
            "doc_edit" | "sheet_edit" if n == 1 => "Change a document".into(),
            "doc_edit" | "sheet_edit" => format!("Change a document ({n} edits)"),
            "document_create" => format!("Create {title}"),
            "document_link" if input.get("unlink").and_then(Value::as_bool) == Some(true) => "Unlink a document".into(),
            "document_link" => "Link a document".into(),
            "comment_reply" => "Reply to a document's comment".into(),
            "comment_resolve" => "Resolve a document's comment".into(),
            _ => return None,
        })
    }

    fn pm_prompt(&self) -> Option<&'static str> {
        Some(PM_PROMPT)
    }
}

/// What the PM's prompt says about the document tools.
const PM_PROMPT: &str = "- Documents (docs and sheets kept by a provider, linked to projects and tickets): \
`documents_list` finds them, `doc_read` / `sheet_read` read them, `doc_edit` / `sheet_edit` change them, \
`document_create` makes one (linked to a project or ticket), `document_open_beside` opens one beside this \
conversation so the user watches your edits land. Write specs, plans and working tables as documents, never as files \
in a repository. Edits, new documents, links and comment replies ask the user unless the project lets you act.";

fn tool(decl: ToolDecl, title: &'static str, description: &'static str) -> ToolFace {
    ToolFace { decl, title, description }
}

/// A document by id, or by its exact title when one has it.
async fn find(st: &AppState, given: &str) -> Result<Document, String> {
    let g = given.trim();
    if let Some(d) = st.store.get_document(g).await.map_err(|e| e.to_string())? {
        return Ok(d);
    }
    let f = DocumentFilter { q: Some(g.to_string()), ..Default::default() };
    let mut same: Vec<Document> = st
        .store
        .list_documents(&f)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|d| d.title.eq_ignore_ascii_case(g))
        .collect();
    match same.len() {
        1 => Ok(same.remove(0)),
        0 => Err(format!("no document is called {g} (documents_list finds them; give its id)")),
        n => Err(format!("{n} documents are called {g}: give its id (documents_list lists them)")),
    }
}

/// Who an agent's change is by.
async fn by(cx: &ActionCx<'_>) -> By {
    match cx.session {
        Some(s) => crate::mcp::tools::agent_by(cx.st, s).await,
        None => By::from(Actor::Agent),
    }
}

/// A document as the tools give it: no provider reference, links by key.
fn brief(d: &Document) -> Value {
    json!({
        "id": d.id, "title": d.title, "kind": d.kind, "provider": d.provider, "url": d.url,
        "projects": d.projects.iter().map(|p| &p.key).collect::<Vec<_>>(),
        "tickets": d.tickets.iter().map(|t| &t.key).collect::<Vec<_>>(),
        "addedBy": d.added_by(), "updatedAt": d.updated_at,
    })
}

/// The ask target of a call naming a `document`.
fn document_target(args: &Value) -> AskTarget {
    match args.get("document").and_then(Value::as_str) {
        Some(d) => AskTarget::Document(d.trim().to_string()),
        None => AskTarget::Always,
    }
}

/// Comments with what people wrote fenced as outside input, and the note that says so.
fn fenced(comments: Vec<DocumentComment>, resolved: bool) -> Value {
    let fence = Fence::new(uuid::Uuid::new_v4().simple().to_string());
    let list: Vec<Value> = comments
        .into_iter()
        .filter(|c| resolved || !c.resolved)
        .map(|c| {
            json!({
                "id": c.id, "author": c.author, "body": fence.wrap(&c.body),
                "quote": c.quote.map(|q| fence.wrap(&q)), "resolved": c.resolved, "createdAt": c.created_at,
                "replies": c.replies.into_iter().map(|r| json!({
                    "id": r.id, "author": r.author, "body": fence.wrap(&r.body), "createdAt": r.created_at,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({"comments": list, "outsideText": format!(
        "{} Comments on a document are written by the people it is shared with, so each body is in such a block.",
        fence.note()
    )})
}

fn edits_for(edits: &[DocumentEdit], docs: bool) -> Result<(), String> {
    match edits.iter().find(|e| e.for_docs() != docs) {
        Some(e) if docs => Err(format!("{} is for spreadsheets: use sheet_edit", e.op())),
        Some(e) => Err(format!("{} is for text documents: use doc_edit", e.op())),
        None => Ok(()),
    }
}

fn need_kind(d: &Document, kind: DocumentKind, tool: &str) -> Result<(), String> {
    match (d.kind, kind) {
        (a, b) if a == b => Ok(()),
        (DocumentKind::Sheet, _) => {
            Err(format!("{} is a spreadsheet: use sheet_read and sheet_edit, not {tool}", d.title))
        }
        (DocumentKind::Doc, _) => Err(format!("{} is a text document: use doc_read and doc_edit, not {tool}", d.title)),
    }
}

async fn read(st: &AppState, d: &Document) -> Result<DocumentContent, String> {
    let p = provider_of(st, d).map_err(api)?;
    p.read(&d.provider_ref).await.map_err(|e| api(e.into()))
}

// ---- documents_list

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListInput {
    /// Linked to this project (key or id).
    #[serde(default)]
    pub project: Option<String>,
    /// Linked to this ticket (key or id).
    #[serde(default)]
    pub ticket: Option<String>,
    /// `doc` or `sheet`.
    #[serde(default)]
    pub kind: Option<DocumentKind>,
    /// Words in the title.
    #[serde(default)]
    pub q: Option<String>,
}

pub struct DocumentsList;

impl Action for DocumentsList {
    type Input = ListInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("documents_list", "List documents").read_only().pm(),
            "List documents",
            "The documents PDT knows (docs and sheets, whichever provider keeps them), most recently changed first, \
             by project, ticket, kind or words in the title: id, title, kind, provider, links and who added each.",
        )
    }

    fn approval_title(&self, _: &ListInput) -> String {
        "list documents".into()
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: ListInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let st = cx.st;
            let project = match i.project.as_deref().filter(|p| !p.trim().is_empty()) {
                Some(p) => Some(crate::pm::project_id(st, p).await?),
                None => None,
            };
            let ticket = match i.ticket.as_deref().filter(|t| !t.trim().is_empty()) {
                Some(t) => Some(crate::mcp::tools::resolve_ticket(st, t, None).await?),
                None => None,
            };
            let f = DocumentFilter { project, ticket, kind: i.kind, provider: None, q: i.q };
            let list = st.store.list_documents(&f).await.map_err(|e| e.to_string())?;
            Ok(json!({"documents": list.iter().map(brief).collect::<Vec<_>>()}))
        })
    }
}

// ---- document_get

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentInput {
    /// The document's id (or its exact title).
    pub document: String,
}

pub struct DocumentGet;

impl Action for DocumentGet {
    type Input = DocumentInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("document_get", "Read a document's details and comments").read_only().pm(),
            "Get a document",
            "A document's details (title, kind, provider and what it can do, links, who added it) and its open \
             comments, when its provider has comments. doc_read or sheet_read gives its content.",
        )
    }

    fn approval_title(&self, _: &DocumentInput) -> String {
        "read a document".into()
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: DocumentInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let d = find(cx.st, &i.document).await?;
            let p = provider_of(cx.st, &d).map_err(api)?;
            let mut v = brief(&d);
            v["capabilities"] = json!(p.capabilities());
            if p.capabilities().comments {
                match p.comments(&d.provider_ref).await {
                    Ok(c) => {
                        let f = fenced(c, false);
                        v["comments"] = f["comments"].clone();
                        v["outsideText"] = f["outsideText"].clone();
                    }
                    Err(e) => v["commentsError"] = json!(e.to_string()),
                }
            }
            Ok(v)
        })
    }
}

// ---- doc_read

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocReadInput {
    /// The document's id (or its exact title).
    pub document: String,
    /// Only this section (a heading's text): the heading and everything under it.
    #[serde(default)]
    pub heading: Option<String>,
}

pub struct DocRead;

impl Action for DocRead {
    type Input = DocReadInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("doc_read", "Read a text document as Markdown").read_only().pm(),
            "Read a document",
            "A text document's content as Markdown (or one section of it, by heading), with its headings and its \
             version.",
        )
    }

    fn approval_title(&self, _: &DocReadInput) -> String {
        "read a document".into()
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: DocReadInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let d = find(cx.st, &i.document).await?;
            need_kind(&d, DocumentKind::Doc, "doc_read")?;
            let DocumentContent::Doc { markdown, version } = read(cx.st, &d).await? else {
                return Err(format!("{} is not a text document at its provider", d.title));
            };
            let hs: Vec<String> =
                headings(&markdown).into_iter().map(|(lv, t)| format!("{} {t}", "#".repeat(lv))).collect();
            let body = match i.heading.as_deref().filter(|h| !h.trim().is_empty()) {
                Some(h) => section(&markdown, h).map_err(|e| e.to_string())?,
                None => markdown,
            };
            Ok(json!({"id": d.id, "title": d.title, "markdown": body, "headings": hs, "version": version}))
        })
    }
}

// ---- doc_edit

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditInput {
    /// The document's id (or its exact title).
    pub document: String,
    /// The changes, applied in order (each an `op`).
    pub edits: Vec<DocumentEdit>,
}

pub struct DocEdit;

impl Action for DocEdit {
    type Input = EditInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("doc_edit", "Change a text document").asks().pm(),
            "Edit a document",
            "Changes a text document, in order: `replace_section` (a heading's body), `insert_after_heading`, \
             `append_text`, `replace_string` (text that appears once, or every time with all), `table_append_row` and \
             `table_set_cell` (the nth table, or the first under a heading), or `replace_text` (the whole document). \
             Returns the new version and a diff of what changed. The user confirms each call unless a project the \
             document is linked to lets you act.",
        )
    }

    fn approval_title(&self, i: &EditInput) -> String {
        format!("change a document ({} edits)", i.edits.len())
    }

    fn ask_target(&self, args: &Value) -> AskTarget {
        document_target(args)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: EditInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let d = find(cx.st, &i.document).await?;
            need_kind(&d, DocumentKind::Doc, "doc_edit")?;
            edits_for(&i.edits, true)?;
            let before = match read(cx.st, &d).await? {
                DocumentContent::Doc { markdown, .. } => markdown,
                DocumentContent::Sheet { .. } => String::new(),
            };
            let r = apply_edits(cx.st, &d, i.edits).await.map_err(api)?;
            let after = match read(cx.st, &d).await {
                Ok(DocumentContent::Doc { markdown, .. }) => markdown,
                _ => before.clone(),
            };
            Ok(json!({"version": r.version, "diff": pdt_core::document_ops::diff(&before, &after)}))
        })
    }
}

// ---- sheet_read

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SheetReadInput {
    /// The spreadsheet's id (or its exact title).
    pub document: String,
    /// The sheet to read cells from; the first when left out.
    #[serde(default)]
    pub sheet: Option<String>,
    /// A range of it (`A1:F40`); every used cell when left out.
    #[serde(default)]
    pub range: Option<String>,
}

pub struct SheetRead;

impl Action for SheetRead {
    type Input = SheetReadInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("sheet_read", "Read a spreadsheet's cells, formulas and charts").read_only().pm(),
            "Read a spreadsheet",
            "A spreadsheet's sheets (name, used size, charts) and the cells of one sheet or range: each cell's place, \
             what it shows (computed, where the provider computes formulas) and its formula. Empty cells are left out; \
             at most 2000 cells come back (`truncated` says when there were more: read a smaller range).",
        )
    }

    fn approval_title(&self, _: &SheetReadInput) -> String {
        "read a spreadsheet".into()
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: SheetReadInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let d = find(cx.st, &i.document).await?;
            need_kind(&d, DocumentKind::Sheet, "sheet_read")?;
            let DocumentContent::Sheet { sheets, version } = read(cx.st, &d).await? else {
                return Err(format!("{} is not a spreadsheet at its provider", d.title));
            };
            let list: Vec<Value> = sheets
                .iter()
                .map(|s| {
                    let (rows, columns) = used_size(s);
                    json!({"name": s.name, "rows": rows, "columns": columns, "charts": s.charts})
                })
                .collect();
            let chosen = match i.sheet.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(n) => sheets
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(n))
                    .ok_or_else(|| format!("{} has no sheet called {n}", d.title))?,
                None => sheets.first().ok_or_else(|| format!("{} has no sheets", d.title))?,
            };
            let cells = cells_in(chosen, i.range.as_deref()).map_err(|e| e.to_string())?;
            let truncated = cells.len() > MAX_CELLS;
            let cells: Vec<Value> = cells
                .into_iter()
                .take(MAX_CELLS)
                .map(|(at, c)| match c.formula {
                    Some(f) => json!({"cell": at, "value": c.value, "formula": f}),
                    None => json!({"cell": at, "value": c.value}),
                })
                .collect();
            Ok(json!({"id": d.id, "title": d.title, "version": version, "sheets": list, "sheet": chosen.name,
                      "cells": cells, "truncated": truncated}))
        })
    }
}

// ---- sheet_edit

pub struct SheetEdit;

impl Action for SheetEdit {
    type Input = EditInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("sheet_edit", "Change a spreadsheet: cells, sheets, formats, charts").asks().pm(),
            "Edit a spreadsheet",
            "Changes a spreadsheet with a batch of operations, in order: `set_cells` (values from a cell across and \
             down; `=` starts a formula), `add_sheet`, `rename_sheet`, `delete_sheet`, `format_range`, \
             `conditional_format`, `add_chart` / `update_chart` / `delete_chart` (a kind and a data range whose first \
             column is the categories and first row the series' names), `sort`, `filter` and `clear_filter`. An \
             operation the provider can't do fails with a message saying so, and the batch stops there. Returns the \
             new version and the ids of new charts. The user confirms each call unless a project the spreadsheet is \
             linked to lets you act.",
        )
    }

    fn approval_title(&self, i: &EditInput) -> String {
        format!("change a spreadsheet ({} edits)", i.edits.len())
    }

    fn ask_target(&self, args: &Value) -> AskTarget {
        document_target(args)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: EditInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let d = find(cx.st, &i.document).await?;
            need_kind(&d, DocumentKind::Sheet, "sheet_edit")?;
            edits_for(&i.edits, false)?;
            let r = apply_edits(cx.st, &d, i.edits).await.map_err(api)?;
            Ok(json!({"version": r.version, "charts": r.charts}))
        })
    }
}

// ---- document_create

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInput {
    pub title: String,
    /// `doc` or `sheet`.
    pub kind: DocumentKind,
    /// Link it to this project (key or id).
    #[serde(default)]
    pub project: Option<String>,
    /// Link it to this ticket (key or id) and its project. With neither, a session's own ticket.
    #[serde(default)]
    pub ticket: Option<String>,
    /// The provider to keep it (`local`, `google`); the project's default when left out.
    #[serde(default)]
    pub provider: Option<String>,
    /// A text document's first content.
    #[serde(default)]
    pub markdown: Option<String>,
    /// Open it beside this conversation too.
    #[serde(default)]
    pub open_beside: bool,
}

pub struct DocumentCreate;

/// The ticket a new document is linked to: the one given, else (with no project either) the session's own.
async fn create_links(cx: &ActionCx<'_>, i: &CreateInput) -> (Option<String>, Option<String>) {
    let given = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(String::from);
    let (project, ticket) = (given(&i.project), given(&i.ticket));
    if project.is_some() || ticket.is_some() {
        return (project, ticket);
    }
    let own = match cx.session {
        Some(s) => crate::mcp::tools::session_ticket(cx.st, s).await,
        None => None,
    };
    (None, own)
}

impl Action for DocumentCreate {
    type Input = CreateInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("document_create", "Create a document or spreadsheet").asks().pm(),
            "Create a document",
            "Makes a text document (with Markdown to start it) or a spreadsheet with the project's default provider \
             (or the one you name), linked to a ticket or project: the session's own ticket when you name neither. \
             Write specs and plans this way, never as files in the repository. `openBeside` opens it beside this \
             conversation. The user confirms each call unless the project lets you act.",
        )
    }

    fn approval_title(&self, i: &CreateInput) -> String {
        format!("create {}", i.title.trim())
    }

    fn ask_target(&self, args: &Value) -> AskTarget {
        let field = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
        match (field("ticket"), field("project")) {
            (Some(t), _) => AskTarget::Ticket(t.into()),
            (None, Some(p)) => AskTarget::Project(p.into()),
            // The session's own ticket, through the session's project.
            (None, None) => AskTarget::Always,
        }
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: CreateInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let (project, ticket) = create_links(cx, &i).await;
            let new = NewDocument {
                title: i.title.clone(),
                kind: i.kind,
                provider: i.provider.clone(),
                project_id: project,
                ticket_id: ticket,
                markdown: i.markdown.clone(),
                by: None,
            };
            let d = create_document(cx.st, new, &by(cx).await).await.map_err(api)?;
            let mut v = brief(&d);
            if i.open_beside
                && let Some(s) = cx.session
            {
                open_beside_doc(cx.st, &s.id, &d.id, Actor::Agent).await.map_err(api)?;
                v["openBeside"] = json!(true);
            }
            Ok(v)
        })
    }
}

// ---- document_link

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinkInput {
    /// The document's id (or its exact title).
    pub document: String,
    /// A project (key or id) to link it to.
    #[serde(default)]
    pub project: Option<String>,
    /// A ticket (key or id) to link it to.
    #[serde(default)]
    pub ticket: Option<String>,
    /// Remove the link instead.
    #[serde(default)]
    pub unlink: bool,
}

pub struct DocumentLink;

impl Action for DocumentLink {
    type Input = LinkInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("document_link", "Link a document to a project or ticket").asks().pm(),
            "Link a document",
            "Links a document to a project or a ticket (one per call), or removes that link with `unlink`. A ticket's \
             documents show on it, and agents working on it read them. The user confirms each call unless the project \
             lets you act.",
        )
    }

    fn approval_title(&self, i: &LinkInput) -> String {
        let what = i.ticket.as_deref().or(i.project.as_deref()).unwrap_or("?");
        if i.unlink { format!("unlink a document from {what}") } else { format!("link a document to {what}") }
    }

    fn ask_target(&self, args: &Value) -> AskTarget {
        let field = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
        match (field("ticket"), field("project")) {
            (Some(t), _) => AskTarget::Ticket(t.into()),
            (None, Some(p)) => AskTarget::Project(p.into()),
            (None, None) => document_target(args),
        }
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: LinkInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let st = cx.st;
            let d = find(st, &i.document).await?;
            let input = DocumentLinkInput { project_id: i.project, ticket_id: i.ticket };
            let link = link_of(st, &input).await.map_err(api)?;
            if i.unlink {
                st.store.unlink_document(&d.id, &link).await.map_err(|e| e.to_string())?;
            } else {
                st.store.link_document(&d.id, &link, now_ms()).await.map_err(|e| e.to_string())?;
            }
            Ok(brief(&document(st, &d.id).await.map_err(api)?))
        })
    }
}

// ---- comments_list

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommentsInput {
    /// The document's id (or its exact title).
    pub document: String,
    /// Resolved comments too.
    #[serde(default)]
    pub include_resolved: bool,
}

pub struct CommentsList;

impl Action for CommentsList {
    type Input = CommentsInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("comments_list", "Read a document's comments").read_only().pm(),
            "List a document's comments",
            "A document's comments (open ones, or resolved too) with their replies: id, author, the text each is on, \
             and its body, fenced as outside input since other people write them.",
        )
    }

    fn approval_title(&self, _: &CommentsInput) -> String {
        "read a document's comments".into()
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: CommentsInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let d = find(cx.st, &i.document).await?;
            let (d, p) = with_comments(cx.st, &d.id).await.map_err(api)?;
            let c = p.comments(&d.provider_ref).await.map_err(|e| api(e.into()))?;
            Ok(fenced(c, i.include_resolved))
        })
    }
}

// ---- comment_reply

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplyInput {
    /// The document's id (or its exact title).
    pub document: String,
    /// The comment's id (comments_list).
    pub comment: String,
    pub body: String,
}

pub struct CommentReply;

impl Action for CommentReply {
    type Input = ReplyInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("comment_reply", "Reply to a comment on a document").asks().pm(),
            "Reply to a comment",
            "Replies in a comment's thread on a document. The user confirms each call unless a project the document \
             is linked to lets you act.",
        )
    }

    fn approval_title(&self, _: &ReplyInput) -> String {
        "reply to a comment".into()
    }

    fn ask_target(&self, args: &Value) -> AskTarget {
        document_target(args)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: ReplyInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let body = i.body.trim();
            if body.is_empty() {
                return Err("a reply needs some text".into());
            }
            let d = find(cx.st, &i.document).await?;
            let (d, p) = with_comments(cx.st, &d.id).await.map_err(api)?;
            let r = p.reply(&d.provider_ref, i.comment.trim(), body).await.map_err(|e| api(e.into()))?;
            Ok(json!({"id": r.id, "replied": true}))
        })
    }
}

// ---- comment_resolve

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveInput {
    /// The document's id (or its exact title).
    pub document: String,
    /// The comment's id (comments_list).
    pub comment: String,
}

pub struct CommentResolve;

impl Action for CommentResolve {
    type Input = ResolveInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("comment_resolve", "Resolve a comment on a document").asks().pm(),
            "Resolve a comment",
            "Marks a comment on a document resolved. The user confirms each call unless a project the document is \
             linked to lets you act.",
        )
    }

    fn approval_title(&self, _: &ResolveInput) -> String {
        "resolve a comment".into()
    }

    fn ask_target(&self, args: &Value) -> AskTarget {
        document_target(args)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: ResolveInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let d = find(cx.st, &i.document).await?;
            let (d, p) = with_comments(cx.st, &d.id).await.map_err(api)?;
            p.resolve(&d.provider_ref, i.comment.trim()).await.map_err(|e| api(e.into()))?;
            Ok(json!({"resolved": true}))
        })
    }
}

// ---- document_open_beside

pub struct DocumentOpenBeside;

impl Action for DocumentOpenBeside {
    type Input = DocumentInput;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        tool(
            ToolDecl::new("document_open_beside", "Open a document beside the conversation").pm(),
            "Open a document beside the conversation",
            "Opens a document in a tab of the side panel beside this conversation (or shows its tab, when it is open \
             there already), so the user sees it and watches your edits land. Say so in your reply (\"it's open on \
             the right\").",
        )
    }

    fn approval_title(&self, _: &DocumentInput) -> String {
        "open a document beside the conversation".into()
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, i: DocumentInput) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let s = cx.session.ok_or("only an agent session can open a document beside its conversation")?;
            let d = find(cx.st, &i.document).await?;
            let b = open_beside_doc(cx.st, &s.id, &d.id, Actor::Agent).await.map_err(api)?;
            let tabs: Vec<&str> = b.tabs.iter().map(|t| t.document.title.as_str()).collect();
            Ok(json!({"opened": d.title, "tabs": tabs}))
        })
    }
}
