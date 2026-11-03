//! The documents' workflow nodes (PDT-318), provider-neutral: steps that make a document, add to a doc, write rows and
//! ranges into a sheet, add a chart and comment, and the `document.changed` and `document.comment_added` triggers
//! ([`super::watch`] logs their events). A step names its document by id or exact title; a dry run says what it would
//! do and changes nothing. Steps that add something (a document, text, rows, a chart, a comment) don't run again on
//! their own after being cut short, so nothing is added twice.

use super::routes::{NewDocument, apply_edits, create_document, provider_of};
use super::tools::{brief, find, read};
use super::{CommentInput, DocumentProvider};
use crate::mcp::tools::api;
use crate::routes::AppState;
use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, parses, ports, schema_of, spec,
};
use pdt_core::document::{
    ChartKind, ChartSpec, Document, DocumentContent, DocumentEdit, DocumentKind, Sheet, cell_name, parse_cell,
    parse_range,
};
use pdt_core::document_ops::used_size;
use pdt_core::document_watch::{accepts_copy, rows_of, text_from_refs};
use pdt_core::ticket::{Actor, By};
use pdt_core::workflow::api::{LogEvent, Workflow};
use pdt_core::workflow::cond::Condition;
use pdt_core::workflow::doc::Node;
use pdt_core::workflow::events::{DOCUMENT_CHANGED, DOCUMENT_COMMENT_ADDED, DocumentRef, event_type};
use pdt_core::workflow::refs::has_ref;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use ts_rs::TS;

/// The documents' node kinds, in catalog order.
pub(crate) fn kinds() -> Vec<Box<dyn NodeKind>> {
    vec![
        Box::new(DocumentTrigger(DOCUMENT_CHANGED)),
        Box::new(DocumentTrigger(DOCUMENT_COMMENT_ADDED)),
        Box::new(Create),
        Box::new(Append),
        Box::new(AppendRows),
        Box::new(SetRange),
        Box::new(AddChart),
        Box::new(Comment),
    ]
}

fn blank(s: &str) -> bool {
    s.trim().is_empty()
}

fn given(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

fn need_document(d: &str) -> Result<(), String> {
    if blank(d) { Err("document is empty: give its id or exact title".into()) } else { Ok(()) }
}

/// A cell (`B3`) or range (`A1:C10`) as a config gives it, unless it is a reference.
fn check_cell(what: &str, v: &str, range: bool) -> Result<(), String> {
    if has_ref(v) {
        return Ok(());
    }
    let ok = if range { parse_range(v).is_some() } else { parse_cell(v).is_some() };
    if ok {
        Ok(())
    } else {
        Err(format!("{what} {v:?} is not a {}", if range { "range (A1:C10)" } else { "cell (B3)" }))
    }
}

/// The document a step names, of `kind`.
async fn document_of(st: &AppState, given: &str, kind: DocumentKind) -> Result<Document, String> {
    let d = find(st, given).await?;
    if d.kind != kind {
        let what = if kind == DocumentKind::Doc { "a text document" } else { "a spreadsheet" };
        return Err(format!("{} is not {what}", d.title));
    }
    Ok(d)
}

/// A dry run's output: what the step would do, to the document when PDT finds it.
async fn would(cx: &StepCx<'_>, document: &str, what: String) -> Outcome {
    let title = match find(cx.st, document).await {
        Ok(d) => d.title,
        Err(_) => document.trim().to_string(),
    };
    Outcome::next("out", json!({"would": format!("{what} ({title})")}))
}

/// A sheet's sheets, and the one named (the first when none is).
async fn sheet_of(st: &AppState, d: &Document, name: Option<&str>) -> Result<Sheet, String> {
    let DocumentContent::Sheet { sheets, .. } = read(st, d).await? else {
        return Err(format!("{} is not a spreadsheet at its provider", d.title));
    };
    match name {
        Some(n) => sheets
            .into_iter()
            .find(|s| s.name.eq_ignore_ascii_case(n))
            .ok_or_else(|| format!("{} has no sheet called {n}", d.title)),
        None => sheets.into_iter().next().ok_or_else(|| format!("{} has no sheets", d.title)),
    }
}

/// A sheet step's cells from its `field` (filled in), with the text references brought in kept as text
/// ([`text_from_refs`]): only formulas written into the step itself run.
fn cells_of(cx: &StepCx<'_>, field: &str, filled: &Value) -> Result<Vec<Vec<String>>, String> {
    let mut rows = rows_of(filled)?;
    text_from_refs(cx.node.config.get(field).unwrap_or(&Value::Null), &mut rows);
    Ok(rows)
}

/// The range `rows` fill from `cell` (`A5:C6`).
fn filled(cell: &str, rows: &[Vec<String>]) -> String {
    let (r, c) = parse_cell(cell).unwrap_or((0, 0));
    let width = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let end = cell_name(r + rows.len().max(1) - 1, c + width - 1);
    format!("{}:{end}", cell_name(r, c))
}

// ---- document.create

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, rename = "DocumentCreateConfig")]
pub struct CreateConfig {
    pub title: String,
    /// `doc` or `sheet`.
    pub kind: DocumentKind,
    /// Link it to this project (key or id); the run's project when neither this nor `ticket` is given.
    #[serde(default)]
    #[ts(optional)]
    pub project: Option<String>,
    /// Link it to this ticket (key or id) and its project (`{{trigger.ticketId}}`).
    #[serde(default)]
    #[ts(optional)]
    pub ticket: Option<String>,
    /// The provider to keep it (`local`, `google`); the project's default when left out.
    #[serde(default)]
    #[ts(optional)]
    pub provider: Option<String>,
    /// A text document's first content.
    #[serde(default)]
    #[ts(optional)]
    #[schemars(extend("x-format" = "markdown"))]
    pub markdown: Option<String>,
}

pub struct Create;

impl NodeKind for Create {
    fn spec(&self) -> KindSpec {
        spec::<CreateConfig, Value>(
            "document.create",
            1,
            Category::Action,
            "Create a document",
            "Makes a text document (with Markdown to start it) or a spreadsheet with the project's default provider (or \
             the one named), linked to a ticket or project: the run's project when neither is given. Output: the \
             document's `id`, `title`, `kind` and `url`.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: CreateConfig = parse(config)?;
        if blank(&c.title) {
            return Err("title is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CreateConfig = parse(&input)?;
            let kind = if c.kind == DocumentKind::Doc { "document" } else { "spreadsheet" };
            if cx.dry_run {
                // Later steps' dry runs name the new document by its title.
                let title = c.title.trim();
                return Ok(Outcome::next(
                    "out",
                    json!({"would": format!("create the {kind} {title}"), "id": title, "title": title, "kind": c.kind}),
                ));
            }
            let (mut project, ticket) = (given(&c.project).map(String::from), given(&c.ticket).map(String::from));
            if project.is_none() && ticket.is_none() {
                project = cx.run.project_id.clone();
            }
            let new = NewDocument {
                title: c.title,
                kind: c.kind,
                provider: given(&c.provider).map(String::from),
                project_id: project,
                ticket_id: ticket,
                markdown: c.markdown,
                by: None,
            };
            let d = create_document(cx.st, new, &By::from(Actor::System)).await.map_err(api)?;
            Ok(Outcome::next("out", brief(&d)))
        })
    }

    /// A second run would make a second document.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }
}

// ---- document.append

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, rename = "DocumentAppendConfig")]
pub struct AppendConfig {
    /// The text document's id or exact title.
    pub document: String,
    /// What to add.
    #[schemars(extend("x-format" = "markdown"))]
    pub markdown: String,
    /// Put it right under this heading (the top of its section) instead of at the end.
    #[serde(default)]
    #[ts(optional)]
    pub heading: Option<String>,
}

pub struct Append;

impl NodeKind for Append {
    fn spec(&self) -> KindSpec {
        spec::<AppendConfig, Value>(
            "document.append",
            1,
            Category::Action,
            "Add to a document",
            "Adds Markdown to a text document: at its end, or right under a heading. Output: the new `version`.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: AppendConfig = parse(config)?;
        need_document(&c.document)?;
        if blank(&c.markdown) {
            return Err("markdown is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: AppendConfig = parse(&input)?;
            let markdown = c.markdown.trim_end().to_string();
            if markdown.trim().is_empty() {
                return Err("markdown is empty".into());
            }
            if cx.dry_run {
                let at = given(&c.heading).map_or("at the end".to_string(), |h| format!("under {h}"));
                return Ok(would(cx, &c.document, format!("add {} characters {at}", markdown.chars().count())).await);
            }
            let d = document_of(cx.st, &c.document, DocumentKind::Doc).await?;
            let edit = match given(&c.heading) {
                Some(h) => DocumentEdit::InsertAfterHeading { heading: h.into(), markdown },
                None => DocumentEdit::AppendText { markdown },
            };
            let r = apply_edits(cx.st, &d, vec![edit]).await.map_err(api)?;
            Ok(Outcome::next("out", json!({"id": d.id, "title": d.title, "version": r.version})))
        })
    }

    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }
}

// ---- sheet.append_rows

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, rename = "SheetAppendRowsConfig")]
pub struct AppendRowsConfig {
    /// The spreadsheet's id or exact title.
    pub document: String,
    /// The sheet; the first when left out.
    #[serde(default)]
    #[ts(optional)]
    pub sheet: Option<String>,
    /// The rows, each a list of cells (`[["{{trigger.ticketKey}}", "{{trigger.ticketTitle}}"]]`), or one row. A
    /// cell written here starting with `=` is a formula; text from a reference is always text.
    #[ts(type = "unknown")]
    pub rows: Value,
}

pub struct AppendRows;

impl NodeKind for AppendRows {
    fn spec(&self) -> KindSpec {
        spec::<AppendRowsConfig, Value>(
            "sheet.append_rows",
            1,
            Category::Action,
            "Add rows to a sheet",
            "Writes rows under the last used row of a sheet, from column A. Text from a reference is always text, \
             never a formula. Output: the `range` written and the new `version`.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: AppendRowsConfig = parse(config)?;
        need_document(&c.document)?;
        match &c.rows {
            Value::String(s) if has_ref(s) => Ok(()),
            rows => rows_of(rows).map(|_| ()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: AppendRowsConfig = parse(&input)?;
            let rows = cells_of(cx, "rows", &c.rows)?;
            if cx.dry_run {
                let n = rows.len();
                let what = format!("add {n} row{}: {}", if n == 1 { "" } else { "s" }, json!(rows));
                return Ok(would(cx, &c.document, what).await);
            }
            let d = document_of(cx.st, &c.document, DocumentKind::Sheet).await?;
            let sheet = sheet_of(cx.st, &d, given(&c.sheet)).await?;
            let cell = cell_name(used_size(&sheet).0, 0);
            let range = filled(&cell, &rows);
            let edit = DocumentEdit::SetCells { sheet: sheet.name.clone(), cell, values: rows };
            let r = apply_edits(cx.st, &d, vec![edit]).await.map_err(api)?;
            Ok(Outcome::next("out", json!({"id": d.id, "sheet": sheet.name, "range": range, "version": r.version})))
        })
    }

    /// A second run would add the rows again.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }
}

// ---- sheet.set_range

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, rename = "SheetSetRangeConfig")]
pub struct SetRangeConfig {
    /// The spreadsheet's id or exact title.
    pub document: String,
    /// The sheet; the first when left out.
    #[serde(default)]
    #[ts(optional)]
    pub sheet: Option<String>,
    /// The top-left cell (`B3`).
    pub cell: String,
    /// The values, a list of rows (each a list of cells) or one row, written across and down. A cell written here
    /// starting with `=` is a formula; text from a reference is always text.
    #[ts(type = "unknown")]
    pub values: Value,
}

pub struct SetRange;

impl NodeKind for SetRange {
    fn spec(&self) -> KindSpec {
        spec::<SetRangeConfig, Value>(
            "sheet.set_range",
            1,
            Category::Action,
            "Set cells in a sheet",
            "Writes values into a sheet from a cell, across and down (a formula where a value written here starts with \
             `=`; text from a reference is always text). Output: the `range` written and the new `version`.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: SetRangeConfig = parse(config)?;
        need_document(&c.document)?;
        check_cell("cell", &c.cell, false)?;
        match &c.values {
            Value::String(s) if has_ref(s) => Ok(()),
            values => rows_of(values).map(|_| ()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: SetRangeConfig = parse(&input)?;
            check_cell("cell", &c.cell, false)?;
            let rows = cells_of(cx, "values", &c.values)?;
            let cell = c.cell.trim().to_ascii_uppercase();
            let range = filled(&cell, &rows);
            if cx.dry_run {
                return Ok(would(cx, &c.document, format!("set {range} to {}", json!(rows))).await);
            }
            let d = document_of(cx.st, &c.document, DocumentKind::Sheet).await?;
            let sheet = match given(&c.sheet) {
                Some(s) => s.to_string(),
                None => sheet_of(cx.st, &d, None).await?.name,
            };
            let edit = DocumentEdit::SetCells { sheet: sheet.clone(), cell, values: rows };
            let r = apply_edits(cx.st, &d, vec![edit]).await.map_err(api)?;
            Ok(Outcome::next("out", json!({"id": d.id, "sheet": sheet, "range": range, "version": r.version})))
        })
    }
}

// ---- sheet.add_chart

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, rename = "SheetAddChartConfig")]
pub struct AddChartConfig {
    /// The spreadsheet's id or exact title.
    pub document: String,
    /// The sheet; the first when left out.
    #[serde(default)]
    #[ts(optional)]
    pub sheet: Option<String>,
    /// `column`, `bar`, `line`, `area`, `pie` or `scatter`.
    pub chart: ChartKind,
    /// The data range (`A1:C10`): its first column is the categories, its first row the series' names.
    pub data: String,
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
    /// The cell its top-left corner sits on; the provider places it when left out.
    #[serde(default)]
    #[ts(optional)]
    pub anchor: Option<String>,
}

pub struct AddChart;

impl NodeKind for AddChart {
    fn spec(&self) -> KindSpec {
        spec::<AddChartConfig, Value>(
            "sheet.add_chart",
            1,
            Category::Action,
            "Add a chart to a sheet",
            "Adds a chart of a sheet's range (its first column the categories, its first row the series' names), where \
             the provider has charts. Output: the chart's `chart` id and the new `version`.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: AddChartConfig = parse(config)?;
        need_document(&c.document)?;
        check_cell("data", &c.data, true)?;
        if let Some(a) = given(&c.anchor) {
            check_cell("anchor", a, false)?;
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: AddChartConfig = parse(&input)?;
            let kind =
                serde_json::to_value(c.chart).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
            if cx.dry_run {
                return Ok(would(cx, &c.document, format!("add a {kind} chart of {}", c.data.trim())).await);
            }
            let d = document_of(cx.st, &c.document, DocumentKind::Sheet).await?;
            let sheet = match given(&c.sheet) {
                Some(s) => s.to_string(),
                None => sheet_of(cx.st, &d, None).await?.name,
            };
            let chart = ChartSpec {
                kind: c.chart,
                data: c.data.trim().to_ascii_uppercase(),
                title: given(&c.title).map(String::from),
                anchor: given(&c.anchor).map(|a| a.to_ascii_uppercase()),
            };
            let r =
                apply_edits(cx.st, &d, vec![DocumentEdit::AddChart { sheet, id: None, chart }]).await.map_err(api)?;
            Ok(Outcome::next("out", json!({"id": d.id, "chart": r.charts.first(), "version": r.version})))
        })
    }

    /// A second run would add a second chart.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }
}

// ---- document.comment

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, rename = "DocumentCommentConfig")]
pub struct CommentConfig {
    /// The document's id or exact title (`{{trigger.documentId}}`).
    pub document: String,
    /// The comment to reply to (`{{trigger.thread}}`); a new comment on the document when left out.
    #[serde(default)]
    #[ts(optional)]
    pub comment: Option<String>,
    #[schemars(extend("x-format" = "multiline"))]
    pub body: String,
}

pub struct Comment;

impl NodeKind for Comment {
    fn spec(&self) -> KindSpec {
        spec::<CommentConfig, Value>(
            "document.comment",
            1,
            Category::Action,
            "Comment on a document",
            "Replies in a comment's thread on a document (`comment`), or adds a comment to it, as the workflow, where \
             the provider has comments. It never starts a run of `document.comment_added`. Output: the reply's or \
             comment's `id`.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: CommentConfig = parse(config)?;
        need_document(&c.document)?;
        if blank(&c.body) {
            return Err("body is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CommentConfig = parse(&input)?;
            let body = c.body.trim().to_string();
            if body.is_empty() {
                return Err("body is empty".into());
            }
            if cx.dry_run {
                let what = match given(&c.comment) {
                    Some(id) => format!("reply to comment {id}: {body}"),
                    None => format!("comment: {body}"),
                };
                return Ok(would(cx, &c.document, what).await);
            }
            let d = find(cx.st, &c.document).await?;
            let p: Arc<dyn DocumentProvider> = provider_of(cx.st, &d).map_err(api)?;
            if !p.capabilities().comments {
                return Err(format!("{} has no comments", p.label()));
            }
            let author = cx.workflow.name.clone();
            let id = match given(&c.comment) {
                Some(thread) => {
                    let r = p.reply(&d.provider_ref, thread, &body, (Actor::System, author)).await;
                    r.map_err(|e| api(e.into()))?.id
                }
                None => {
                    let new = CommentInput { body, anchor: None, author, actor: Actor::System };
                    p.comment(&d.provider_ref, new).await.map_err(|e| api(e.into()))?.id
                }
            };
            super::watch::mark_ours(cx.st, &d.id, &id).await;
            Ok(Outcome::next("out", json!({"id": id, "document": d.id})))
        })
    }

    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }
}

// ---- the triggers

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct DocumentTriggerConfig {
    /// The document to watch (its id or exact title); empty: every document linked to the workflow's projects.
    #[ts(optional)]
    pub document: Option<String>,
    /// Only these projects' documents (a host-wide workflow); empty: every project.
    pub projects: Vec<String>,
    /// Only events for which every condition holds (`{{trigger.byUser}}` …).
    #[serde(rename = "where")]
    #[ts(rename = "where")]
    pub conditions: Vec<Condition>,
    /// The run's idempotency key. Default: the event.
    pub run_key: Option<String>,
}

/// `document.changed` or `document.comment_added`: a document's event, once per change whichever projects the
/// document is linked to ([`accepts_copy`]).
pub struct DocumentTrigger(pub &'static str);

impl NodeKind for DocumentTrigger {
    fn spec(&self) -> KindSpec {
        let t = event_type(self.0).expect("a document event type");
        KindSpec {
            id: self.0.into(),
            version: 1,
            category: Category::Trigger,
            title: t.title.into(),
            description: t.description.into(),
            config_schema: schema_of::<DocumentTriggerConfig>(),
            output_schema: serde_json::to_value((t.schema)()).unwrap_or(Value::Null),
            ports: ports(&[("out", "Then")]),
            dynamic_ports: false,
            preset: None,
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<DocumentTriggerConfig>(config)
    }

    fn event_type(&self) -> Option<&str> {
        Some(self.0)
    }

    fn accepts(&self, config: &Value, wf: &Workflow, _node: &Node, event: &LogEvent) -> bool {
        let Ok(c) = parse::<DocumentTriggerConfig>(config) else { return false };
        let Ok(doc) = serde_json::from_value::<DocumentRef>(event.payload.clone()) else { return false };
        accepts_copy(wf.scope, wf.project_id.as_deref(), c.document.as_deref(), &c.projects, &doc)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move { Ok(Outcome::Next { port: "out".into(), output: cx.run.trigger.clone() }) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let ok = |k: &dyn NodeKind, c: Value| k.check(&c);
        assert!(ok(&Create, json!({"title": "Log", "kind": "sheet"})).is_ok());
        assert!(ok(&Create, json!({"title": " ", "kind": "doc"})).is_err());
        assert!(ok(&Create, json!({"title": "x", "kind": "slides"})).is_err());
        assert!(ok(&Append, json!({"document": "Notes", "markdown": "- {{trigger.ticketKey}}"})).is_ok());
        assert!(ok(&Append, json!({"document": "", "markdown": "x"})).is_err());
        assert!(ok(&AppendRows, json!({"document": "Log", "rows": [["{{trigger.ticketKey}}", "x"]]})).is_ok());
        assert!(ok(&AppendRows, json!({"document": "Log", "rows": "{{steps.rows.output}}"})).is_ok());
        assert!(ok(&AppendRows, json!({"document": "Log", "rows": {"a": 1}})).is_err());
        assert!(ok(&SetRange, json!({"document": "Log", "cell": "B2", "values": [[1, 2]]})).is_ok());
        assert!(ok(&SetRange, json!({"document": "Log", "cell": "2B", "values": [[1]]})).is_err());
        assert!(ok(&AddChart, json!({"document": "Log", "chart": "line", "data": "A1:B9"})).is_ok());
        assert!(ok(&AddChart, json!({"document": "Log", "chart": "line", "data": "A1:"})).is_err());
        assert!(ok(&AddChart, json!({"document": "Log", "chart": "donut", "data": "A1:B2"})).is_err());
        assert!(ok(&Comment, json!({"document": "{{trigger.documentId}}", "comment": "c1", "body": "Thanks"})).is_ok());
        assert!(ok(&Comment, json!({"document": "d", "body": ""})).is_err());
        let t = DocumentTrigger(DOCUMENT_CHANGED);
        assert!(ok(&t, json!({"document": "Design", "where": []})).is_ok());
        assert!(ok(&t, json!({"repos": []})).is_err());
    }

    #[test]
    fn a_fill_covers_the_rows_written() {
        assert_eq!(filled("A5", &[vec!["a".into(), "b".into()], vec!["c".into()]]), "A5:B6");
        assert_eq!(filled("C1", &[vec!["x".into()]]), "C1:C1");
    }
}
