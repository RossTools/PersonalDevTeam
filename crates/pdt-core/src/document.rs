//! Documents (PDT-314): design docs, plans and working spreadsheets that live outside the repo, kept by a provider
//! (PDT's local one, or a plugin's, such as Google's) and linked to projects and tickets. PDT keeps a record of each
//! (title, kind, which provider holds it and the provider's reference) and its links; the content stays with the
//! provider. Pure: the types, titles, provider ids, settings and the design doc template.

use crate::ticket::{Actor, AgentBy};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The longest title kept (in characters).
pub const MAX_TITLE: usize = crate::limits::TITLE;

/// A text document or a spreadsheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DocumentKind {
    Doc,
    Sheet,
}

impl DocumentKind {
    pub const ALL: &'static [DocumentKind] = &[DocumentKind::Doc, DocumentKind::Sheet];
    pub fn as_str(self) -> &'static str {
        match self {
            DocumentKind::Doc => "doc",
            DocumentKind::Sheet => "sheet",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == s.trim())
    }
}

/// A project a document is linked to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentProject {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// A ticket a document is linked to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentTicket {
    pub id: String,
    /// `PDT-12`.
    pub key: String,
    pub title: String,
    pub project_id: String,
}

/// A document PDT knows: where it is kept, and what it is linked to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub kind: DocumentKind,
    /// The provider that keeps it: `local`, or a plugin's (`google`).
    pub provider: String,
    /// The provider's own reference to it (a file id, or a path in the documents folder).
    pub provider_ref: String,
    /// Where it opens in a browser, when the provider has a page for it.
    #[serde(default)]
    pub url: Option<String>,
    /// Who added it: the user or an agent (the PM, an agent step).
    pub actor: Actor,
    /// The agent session that added it, when an agent did.
    #[serde(default)]
    pub agent: Option<AgentBy>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    /// The projects it is linked to, by name.
    #[serde(default)]
    pub projects: Vec<DocumentProject>,
    /// The tickets it is linked to, by key.
    #[serde(default)]
    pub tickets: Vec<DocumentTicket>,
}

impl Document {
    /// Who added it, for people: `you`, the persona (`PM`, `Coder`) or `an agent`.
    pub fn added_by(&self) -> String {
        match (self.actor, &self.agent) {
            (Actor::User, _) => "you".into(),
            (_, Some(AgentBy { persona: Some(p), .. })) => p.clone(),
            _ => "an agent".into(),
        }
    }
}

/// A document open beside a conversation (PDT-317): one tab of its side panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BesideTab {
    pub document: Document,
    /// Who opened it here: the user, or the agent with `document_open_beside`.
    pub by: Actor,
    #[ts(type = "number")]
    pub opened_at: i64,
    #[ts(type = "number")]
    pub focused_at: i64,
}

/// A conversation's side panel: its tabs in order, and the one shown (the newest focused).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentsBeside {
    pub tabs: Vec<BesideTab>,
    pub active: Option<String>,
}

impl DocumentsBeside {
    pub fn new(tabs: Vec<BesideTab>) -> Self {
        let active = tabs.iter().max_by_key(|t| t.focused_at).map(|t| t.document.id.clone());
        Self { tabs, active }
    }
}

/// Which documents a list shows: each filter set narrows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentFilter {
    /// Linked to this project (its id).
    #[serde(default)]
    #[ts(optional)]
    pub project: Option<String>,
    /// Linked to this ticket (its id).
    #[serde(default)]
    #[ts(optional)]
    pub ticket: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub kind: Option<DocumentKind>,
    #[serde(default)]
    #[ts(optional)]
    pub provider: Option<String>,
    /// Words in the title (case-insensitive).
    #[serde(default)]
    #[ts(optional)]
    pub q: Option<String>,
    /// At most this many (the most recently changed); every one when left out.
    #[serde(default)]
    #[ts(optional)]
    pub limit: Option<u32>,
}

/// A title as it is kept: trimmed, control characters gone, at most [`MAX_TITLE`] characters. Blank is refused.
pub fn clean_title(title: &str) -> Result<String, &'static str> {
    let t: String = title.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        return Err("a document needs a title");
    }
    Ok(t.chars().take(MAX_TITLE).collect())
}

/// A provider's id: 1 to 32 lower-case letters, digits and `-` (`local`, `google`).
pub fn is_provider_id(s: &str) -> bool {
    (1..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The provider PDT uses for a new document: the project's choice, else the host's, else the first it has, as long as
/// the one chosen is still available (a plugin turned off falls through).
pub fn effective_provider(project: Option<&str>, host: Option<&str>, available: &[&str]) -> Option<String> {
    [project, host]
        .into_iter()
        .flatten()
        .find(|p| available.contains(p))
        .or_else(|| available.first().copied())
        .map(str::to_string)
}

/// Where local documents go: the project's folder, else the host's, else `documents` in PDT's data folder. A folder
/// that does not resolve (relative, `~` with no home) is skipped.
pub fn documents_folder(
    project: Option<&str>,
    host: Option<&str>,
    data_dir: &std::path::Path,
    home: Option<&std::path::Path>,
) -> std::path::PathBuf {
    [project, host]
        .into_iter()
        .flatten()
        .find_map(|f| crate::project::resolve_folder(f, home).ok())
        .unwrap_or_else(|| data_dir.join("documents"))
}

/// A project's documents settings (Settings → the project → Documents).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectDocumentSettings {
    /// The provider new documents go to here; none: the host's default.
    #[serde(default)]
    pub provider: Option<String>,
    /// Where its local documents go (a full path, or `~/…`); none: the host's folder.
    #[serde(default)]
    pub folder: Option<String>,
}

impl ProjectDocumentSettings {
    /// The settings as given, checked: blank clears a field.
    pub fn cleaned(&self) -> Result<Self, &'static str> {
        let provider = self.provider.as_deref().map(str::trim).filter(|p| !p.is_empty());
        if provider.is_some_and(|p| !is_provider_id(p)) {
            return Err("that isn't a document provider's id");
        }
        Ok(Self { provider: provider.map(str::to_string), folder: clean_folder(self.folder.as_deref())? })
    }
}

/// A documents folder as typed: trimmed, blank clears it, else a full path or `~/…`.
pub fn clean_folder(folder: Option<&str>) -> Result<Option<String>, &'static str> {
    let Some(f) = folder.map(str::trim).filter(|f| !f.is_empty()) else { return Ok(None) };
    if !f.starts_with('~') && !std::path::Path::new(f).is_absolute() {
        return Err("the documents folder must be a full path (or start with ~ for your home folder)");
    }
    Ok(Some(f.to_string()))
}

/// The design doc of a ticket (Create design doc): its title and first content, from a simple template.
pub fn design_doc(key: &str, title: &str) -> (String, String) {
    let name = clean_title(&format!("{key} design: {title}")).unwrap_or_else(|_| format!("{key} design"));
    let body = format!(
        "# {name}\n\n\
         Ticket: {key}\n\n\
         ## Goal\n\nWhat this change is for, and who it helps.\n\n\
         ## Scope\n\nWhat it does.\n\n\
         ## Out of scope\n\nWhat it leaves for later.\n\n\
         ## Done when\n\nHow we know it works.\n\n\
         ## Notes\n\n"
    );
    (name, body)
}

/// What a provider can do (PDT-314), so PDT offers only that. Every provider lists its documents and reads them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderCapabilities {
    /// Text documents (read as Markdown).
    pub docs: bool,
    /// Spreadsheets (read as sheets of cells with their formulas).
    pub sheets: bool,
    /// Makes new documents.
    pub create: bool,
    /// Searches the user's files to import or link (a picker).
    pub search: bool,
    /// Changes content ([`DocumentEdit`]).
    pub edit: bool,
    /// Renames a document where it is kept.
    pub rename: bool,
    /// Deletes a document where it is kept (not only PDT's record).
    pub delete: bool,
    /// Comments: list, reply, resolve.
    pub comments: bool,
    /// Has a page that can be shown inside PDT.
    pub embed: bool,
    /// Keeps charts on its sheets and reads their numbers ([`ChartView`]).
    #[serde(default)]
    pub charts: bool,
    /// Edits a document live in PDT: a doc's text saved as it is typed, a sheet in PDT's own spreadsheet editor, and
    /// everyone's changes (an agent's) streamed to the open views.
    #[serde(default)]
    pub live: bool,
}

/// A provider as the UI and agents see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderInfo {
    pub id: String,
    /// Its name for people (`Local`, `Google`).
    pub label: String,
    pub capabilities: ProviderCapabilities,
    /// The provider's own file picker in the browser, if it has one.
    #[serde(default)]
    pub picker: Option<ProviderPicker>,
}

/// A provider's file picker in the browser (PDT-378): a script of its own the import dialog loads and runs for an
/// account. `module` is an ES module exporting `pick({ account, request })`, which resolves to the references of
/// the files picked (none when cancelled); `request(path)` is a GET as PDT's web app (with its UI token).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderPicker {
    /// Its button: `{label}`, then ` ({account})` when there is more than one.
    pub label: String,
    /// The module's address.
    pub module: String,
    /// The accounts' list ([`crate::integration::SignInAccount`]): those with `picker` open it.
    pub accounts: String,
    /// What the dialog says when no account opens it, and none is signed in.
    pub help: String,
    /// What the dialog says when a signed-in account uses PDT's own sign-in (`builtIn`) and this build of PDT has no
    /// picker: nothing the user can set fixes it.
    #[serde(default)]
    pub unavailable: Option<String>,
    /// What the dialog says, with a button that edits the account's connection, when a signed-in account with its own
    /// client lacks what the picker needs.
    #[serde(default)]
    pub setup: Option<String>,
    /// Under an empty search: which files the provider's search can find without the picker.
    #[serde(default)]
    pub search_note: Option<String>,
}

/// A file a provider holds, as its search (the import picker) or create gives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderFile {
    /// The provider's reference to it.
    pub reference: String,
    pub title: String,
    pub kind: DocumentKind,
    #[serde(default)]
    pub url: Option<String>,
    /// Its version or etag, which changes whenever it does.
    #[serde(default)]
    pub version: Option<String>,
    /// When it last changed (ms since the epoch), from providers that know it.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
}

/// A provider's file as the import dialog lists it: with PDT's record of it, when PDT has one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FoundFile {
    #[serde(flatten)]
    #[ts(flatten)]
    pub file: ProviderFile,
    /// PDT's document for it, when PDT has one.
    pub document_id: Option<String>,
    /// The tickets that document is linked to (their ids).
    pub ticket_ids: Vec<String>,
}

/// One cell of a sheet: what it shows, and its formula if it has one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Cell {
    pub value: String,
    #[serde(default)]
    #[ts(optional)]
    pub formula: Option<String>,
}

/// One sheet of a spreadsheet: its rows of cells from A1, and its charts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Cell>>,
    /// Its charts, where the provider reads them.
    #[serde(default)]
    pub charts: Vec<Chart>,
}

/// A chart on a sheet: its id (to update or delete it) and what it shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Chart {
    pub id: String,
    pub spec: ChartSpec,
}

/// What kind of chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChartKind {
    /// Vertical bars.
    Column,
    /// Horizontal bars.
    Bar,
    Line,
    Area,
    Pie,
    Scatter,
}

/// A chart, the same for every provider: a kind and a data range whose first column is the categories (the x axis)
/// and whose other columns are the series, named by the range's first row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ChartSpec {
    pub kind: ChartKind,
    /// The data, as a range on the chart's sheet (`A1:C10`).
    pub data: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    /// The cell its top-left corner sits on; the provider places it when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub anchor: Option<String>,
}

/// Horizontal alignment in a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

/// Vertical alignment in a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

/// A cell format: only the fields given change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CellFormat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub underline: Option<bool>,
    /// The text colour, `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color: Option<String>,
    /// The fill colour, `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub background: Option<String>,
    /// The font size in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub size: Option<u32>,
    /// A number format (`0.00`, `#,##0`, `0%`, `yyyy-mm-dd`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub number_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub horizontal: Option<HAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub vertical: Option<VAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub wrap: Option<bool>,
}

/// A test on a cell's value, for conditional formats and filters. Values are as typed (numbers compare as numbers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export)]
pub enum CellCondition {
    GreaterThan {
        value: String,
    },
    LessThan {
        value: String,
    },
    Between {
        min: String,
        max: String,
    },
    EqualTo {
        value: String,
    },
    NotEqualTo {
        value: String,
    },
    TextContains {
        text: String,
    },
    Empty,
    NotEmpty,
    /// A formula that is true for the cells to match, written for the range's top-left cell (`=B2>C2`).
    Formula {
        formula: String,
    },
}

/// A border's line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BorderStyle {
    #[default]
    Solid,
    Thick,
    Dashed,
    Dotted,
    Double,
    /// Takes the borders away.
    None,
}

/// What a cell may hold (data validation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export)]
pub enum CellValidation {
    /// One of these values (a drop-down).
    OneOf { values: Vec<String> },
    /// A tick box.
    Checkbox,
    /// A real date.
    Date,
    /// A value that meets the condition.
    Matches { when: CellCondition },
}

/// One column to sort by: its letter (`B`), and the order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct SortKey {
    pub column: String,
    #[serde(default)]
    pub descending: bool,
}

/// A document's content as a provider reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum DocumentContent {
    Doc {
        markdown: String,
        #[serde(default)]
        version: Option<String>,
    },
    Sheet {
        sheets: Vec<Sheet>,
        #[serde(default)]
        version: Option<String>,
    },
}

impl DocumentContent {
    pub fn version(&self) -> Option<&str> {
        match self {
            DocumentContent::Doc { version, .. } | DocumentContent::Sheet { version, .. } => version.as_deref(),
        }
    }
}

/// A change to a document's content, the same for every provider: each maps them to its own API and answers
/// "unsupported" for one it can't do. Headings are matched by their text (case-insensitive, `#`s optional).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export)]
pub enum DocumentEdit {
    // ---- text documents
    /// A text document's whole content, as Markdown.
    ReplaceText {
        markdown: String,
    },
    /// Markdown added at a text document's end.
    AppendText {
        markdown: String,
    },
    /// A section's body (everything under its heading, up to the next heading of its level or higher) replaced; the
    /// heading stays.
    ReplaceSection {
        heading: String,
        markdown: String,
    },
    /// Markdown put right under a heading, before the rest of its section.
    InsertAfterHeading {
        heading: String,
        markdown: String,
    },
    /// Text found and replaced: it must appear exactly once, unless `all`.
    ReplaceString {
        find: String,
        replace: String,
        #[serde(default)]
        all: bool,
    },
    /// A row added at the end of a table: the `table`th (from 1) in the document, or under `heading`.
    TableAppendRow {
        #[serde(default)]
        #[ts(optional)]
        heading: Option<String>,
        #[serde(default)]
        #[ts(optional)]
        table: Option<u32>,
        cells: Vec<String>,
    },
    /// One cell of a table changed: `row` counts its body rows from 1, `column` is a header's text or a number from 1.
    TableSetCell {
        #[serde(default)]
        #[ts(optional)]
        heading: Option<String>,
        #[serde(default)]
        #[ts(optional)]
        table: Option<u32>,
        row: u32,
        column: String,
        value: String,
    },

    // ---- spreadsheets
    /// Values written into a sheet from `cell` (`B3`) across and down; a value starting with `=` is a formula.
    SetCells {
        sheet: String,
        cell: String,
        values: Vec<Vec<String>>,
    },
    /// A new, empty sheet.
    AddSheet {
        name: String,
    },
    RenameSheet {
        sheet: String,
        name: String,
    },
    DeleteSheet {
        sheet: String,
    },
    /// A format put on a range (`A1:C1`, or one cell): only the fields given change.
    FormatRange {
        sheet: String,
        range: String,
        format: CellFormat,
    },
    /// A format shown on the range's cells that meet the condition.
    ConditionalFormat {
        sheet: String,
        range: String,
        when: CellCondition,
        format: CellFormat,
    },
    /// A chart added; PDT gives it an id when none is given (the result lists it).
    AddChart {
        sheet: String,
        #[serde(default)]
        #[ts(optional)]
        id: Option<String>,
        chart: ChartSpec,
    },
    UpdateChart {
        id: String,
        chart: ChartSpec,
    },
    DeleteChart {
        id: String,
    },
    /// A range's rows sorted by its columns, in order; with `header` its first row stays on top.
    Sort {
        sheet: String,
        range: String,
        by: Vec<SortKey>,
        #[serde(default)]
        header: bool,
    },
    /// A filter on a range: only the rows whose `column` meets the condition show (its first row is the header).
    Filter {
        sheet: String,
        range: String,
        column: String,
        when: CellCondition,
    },
    ClearFilter {
        sheet: String,
    },
    /// Borders around a range, and between its cells with `inner`.
    SetBorders {
        sheet: String,
        range: String,
        #[serde(default)]
        style: BorderStyle,
        /// `#rrggbb`; black when left out.
        #[serde(default)]
        #[ts(optional)]
        color: Option<String>,
        #[serde(default)]
        inner: bool,
    },
    /// The width of columns (`B`, or `B:D`), in pixels.
    ColumnWidth {
        sheet: String,
        columns: String,
        width: u32,
    },
    /// The first `rows` and `columns` stay in view while scrolling (0 unfreezes).
    Freeze {
        sheet: String,
        #[serde(default)]
        rows: u32,
        #[serde(default)]
        columns: u32,
    },
    /// What may go in a range's cells; `strict` refuses anything else (else it is only flagged).
    Validate {
        sheet: String,
        range: String,
        rule: CellValidation,
        #[serde(default)]
        strict: bool,
    },
}

impl DocumentEdit {
    /// Its name, as agents give it (`replace_section`).
    pub fn op(&self) -> &'static str {
        match self {
            DocumentEdit::ReplaceText { .. } => "replace_text",
            DocumentEdit::AppendText { .. } => "append_text",
            DocumentEdit::ReplaceSection { .. } => "replace_section",
            DocumentEdit::InsertAfterHeading { .. } => "insert_after_heading",
            DocumentEdit::ReplaceString { .. } => "replace_string",
            DocumentEdit::TableAppendRow { .. } => "table_append_row",
            DocumentEdit::TableSetCell { .. } => "table_set_cell",
            DocumentEdit::SetCells { .. } => "set_cells",
            DocumentEdit::AddSheet { .. } => "add_sheet",
            DocumentEdit::RenameSheet { .. } => "rename_sheet",
            DocumentEdit::DeleteSheet { .. } => "delete_sheet",
            DocumentEdit::FormatRange { .. } => "format_range",
            DocumentEdit::ConditionalFormat { .. } => "conditional_format",
            DocumentEdit::AddChart { .. } => "add_chart",
            DocumentEdit::UpdateChart { .. } => "update_chart",
            DocumentEdit::DeleteChart { .. } => "delete_chart",
            DocumentEdit::Sort { .. } => "sort",
            DocumentEdit::Filter { .. } => "filter",
            DocumentEdit::ClearFilter { .. } => "clear_filter",
            DocumentEdit::SetBorders { .. } => "set_borders",
            DocumentEdit::ColumnWidth { .. } => "column_width",
            DocumentEdit::Freeze { .. } => "freeze",
            DocumentEdit::Validate { .. } => "validate",
        }
    }

    /// Whether it changes a text document (else a spreadsheet).
    pub fn for_docs(&self) -> bool {
        matches!(
            self,
            DocumentEdit::ReplaceText { .. }
                | DocumentEdit::AppendText { .. }
                | DocumentEdit::ReplaceSection { .. }
                | DocumentEdit::InsertAfterHeading { .. }
                | DocumentEdit::ReplaceString { .. }
                | DocumentEdit::TableAppendRow { .. }
                | DocumentEdit::TableSetCell { .. }
        )
    }
}

/// Where a comment is: a doc's heading or quoted text, or a sheet's cell or range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "on", rename_all = "snake_case")]
#[ts(export)]
pub enum CommentAnchor {
    /// Under a heading of a doc (its text, without the `#`s).
    Heading { heading: String },
    /// On some text of a doc (the text itself).
    Text { quote: String },
    /// On a sheet's cell (`B3`) or range (`B3:D9`).
    Cell { sheet: String, range: String },
}

impl CommentAnchor {
    /// The anchor as given, checked: text trimmed and not blank, a sheet's range parses.
    pub fn cleaned(self) -> Result<Self, String> {
        let blank = |s: &str, what: &str| {
            let t = s.trim();
            if t.is_empty() {
                Err(format!("a comment's {what} can't be blank"))
            } else {
                Ok(t.chars().take(500).collect())
            }
        };
        Ok(match self {
            CommentAnchor::Heading { heading } => {
                CommentAnchor::Heading { heading: blank(heading.trim_start_matches('#'), "heading")? }
            }
            CommentAnchor::Text { quote } => CommentAnchor::Text { quote: blank(&quote, "text")? },
            CommentAnchor::Cell { sheet, range } => {
                let range = range.trim().to_ascii_uppercase();
                parse_range(&range).ok_or_else(|| format!("{range} is not a cell or range"))?;
                CommentAnchor::Cell { sheet: blank(&sheet, "sheet")?, range }
            }
        })
    }

    /// What it points at, for people: `Goal`, `“some text”`, `Sheet1!B3`.
    pub fn label(&self) -> String {
        match self {
            CommentAnchor::Heading { heading } => heading.clone(),
            CommentAnchor::Text { quote } => format!("“{quote}”"),
            CommentAnchor::Cell { sheet, range } => format!("{sheet}!{range}"),
        }
    }
}

/// A new comment (`POST /api/documents/{id}/comments`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentCommentInput {
    pub body: String,
    #[serde(default)]
    #[ts(optional)]
    pub anchor: Option<CommentAnchor>,
    /// `user` from PDT's UI.
    #[serde(default)]
    #[ts(optional)]
    pub by: Option<Actor>,
}

/// A chart with the numbers it draws, read from its sheet: the categories (the data range's first column, under its
/// header row) and each series (the other columns, named by the header row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartView {
    /// The sheet it is on.
    pub sheet: String,
    pub chart: Chart,
    /// The category labels, as the cells show them.
    pub labels: Vec<String>,
    pub series: Vec<ChartSeriesView>,
}

/// One series of a chart: its name and its values (none where a cell holds no number).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartSeriesView {
    pub name: String,
    pub values: Vec<Option<f64>>,
}

/// A reply in a comment's thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommentReply {
    pub id: String,
    pub author: String,
    pub body: String,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub created_at: Option<i64>,
    /// Who wrote it, when the provider knows: the user, an agent or a workflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub actor: Option<Actor>,
}

/// A comment on a document, with its replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentComment {
    pub id: String,
    pub author: String,
    pub body: String,
    /// The text it is on, if the provider says.
    #[serde(default)]
    pub quote: Option<String>,
    /// Where it is (a heading, some text, a cell), when the provider keeps that.
    #[serde(default)]
    pub anchor: Option<CommentAnchor>,
    pub resolved: bool,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub replies: Vec<CommentReply>,
    /// Who wrote it, when the provider knows: the user, an agent or a workflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub actor: Option<Actor>,
}

/// A cell's place (`B3`) as its zero-based row and column; None for anything else.
pub fn parse_cell(cell: &str) -> Option<(usize, usize)> {
    let cell = cell.trim().to_ascii_uppercase();
    let split = cell.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = cell.split_at(split);
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    let col = letters.bytes().fold(0usize, |n, b| n * 26 + usize::from(b - b'A') + 1) - 1;
    let row: usize = digits.parse().ok().filter(|r| *r >= 1)?;
    Some((row - 1, col))
}

/// A cell's zero-based (row, column).
pub type CellAt = (usize, usize);

/// A range (`A1:C10`, or one cell `B3`) as its zero-based first and last (row, column), in order.
pub fn parse_range(range: &str) -> Option<(CellAt, CellAt)> {
    let (a, b) = match range.split_once(':') {
        Some((a, b)) => (parse_cell(a)?, parse_cell(b)?),
        None => {
            let c = parse_cell(range)?;
            (c, c)
        }
    };
    Some(((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1))))
}

/// A column's zero-based index from its letters (`B` is 1).
pub fn parse_column(column: &str) -> Option<usize> {
    parse_cell(&format!("{}1", column.trim())).map(|(_, c)| c)
}

/// A zero-based column's letters (1 is `B`).
pub fn column_name(mut col: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// A zero-based cell's name (`(2, 1)` is `B3`).
pub fn cell_name(row: usize, col: usize) -> String {
    format!("{}{}", column_name(col), row + 1)
}

#[cfg(test)]
mod anchor_tests {
    use super::*;

    #[test]
    fn anchors_are_checked() {
        let a = CommentAnchor::Cell { sheet: "Sheet1".into(), range: " b3:c4 ".into() }.cleaned().unwrap();
        assert_eq!(a.label(), "Sheet1!B3:C4");
        assert!(CommentAnchor::Cell { sheet: "S".into(), range: "x".into() }.cleaned().is_err());
        let h = CommentAnchor::Heading { heading: "## Goal ".into() }.cleaned().unwrap();
        assert_eq!(h, CommentAnchor::Heading { heading: "Goal".into() });
        assert!(CommentAnchor::Text { quote: "  ".into() }.cleaned().is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn titles_are_trimmed_and_cleaned() {
        assert_eq!(clean_title("  Plan \n for\tQ4 ").unwrap(), "Plan for Q4");
        assert!(clean_title(" \n ").is_err());
        assert_eq!(clean_title(&"x".repeat(300)).unwrap().chars().count(), MAX_TITLE);
    }

    #[test]
    fn kinds_and_provider_ids() {
        assert_eq!(DocumentKind::parse("sheet"), Some(DocumentKind::Sheet));
        assert_eq!(DocumentKind::parse("slides"), None);
        assert!(is_provider_id("local") && is_provider_id("google") && is_provider_id("my-docs2"));
        assert!(!is_provider_id("") && !is_provider_id("Google") && !is_provider_id("a/b"));
    }

    #[test]
    fn the_provider_is_the_projects_then_the_hosts_then_the_first() {
        let available = ["local", "google"];
        assert_eq!(effective_provider(Some("google"), Some("local"), &available).as_deref(), Some("google"));
        assert_eq!(effective_provider(None, Some("google"), &available).as_deref(), Some("google"));
        assert_eq!(effective_provider(Some("gone"), None, &available).as_deref(), Some("local"));
        assert_eq!(effective_provider(Some("google"), None, &[]), None);
    }

    #[test]
    fn the_folder_is_the_projects_then_the_hosts_then_the_data_folders() {
        let data = std::env::temp_dir().join("pdt-data");
        let home = std::env::temp_dir().join("home");
        let own = std::env::temp_dir().join("own");
        let own_s = own.to_string_lossy().into_owned();
        assert_eq!(documents_folder(None, None, &data, Some(&home)), data.join("documents"));
        assert_eq!(documents_folder(None, Some("~/Docs"), &data, Some(&home)), home.join("Docs"));
        assert_eq!(documents_folder(Some(&own_s), Some("~/Docs"), &data, Some(&home)), own);
        assert_eq!(documents_folder(Some("relative"), None, &data, None), data.join("documents"));
        assert_eq!(documents_folder(None, Some("~/x"), &data, None::<&Path>), data.join("documents"));
    }

    #[test]
    fn project_settings_are_checked() {
        let s = ProjectDocumentSettings { provider: Some(" google ".into()), folder: Some(" ".into()) };
        assert_eq!(s.cleaned().unwrap(), ProjectDocumentSettings { provider: Some("google".into()), folder: None });
        assert!(ProjectDocumentSettings { provider: Some("No!".into()), folder: None }.cleaned().is_err());
        assert!(clean_folder(Some("docs")).is_err());
        assert_eq!(clean_folder(Some("~/docs")).unwrap().as_deref(), Some("~/docs"));
    }

    #[test]
    fn a_design_doc_is_named_after_its_ticket() {
        let (title, body) = design_doc("PDT-7", "Faster boards");
        assert_eq!(title, "PDT-7 design: Faster boards");
        for h in ["## Goal", "## Scope", "## Out of scope", "## Done when", "## Notes"] {
            assert!(body.contains(h), "{h} in {body}");
        }
        assert!(body.starts_with("# PDT-7 design: Faster boards\n"));
    }

    #[test]
    fn cells_parse() {
        assert_eq!(parse_cell("A1"), Some((0, 0)));
        assert_eq!(parse_cell("b3"), Some((2, 1)));
        assert_eq!(parse_cell("AA10"), Some((9, 26)));
        assert_eq!(parse_cell("A0"), None);
        assert_eq!(parse_cell("1A"), None);
        assert_eq!(parse_cell("A"), None);
        assert_eq!(parse_range("C10:a1"), Some(((0, 0), (9, 2))));
        assert_eq!(parse_range("B3"), Some(((2, 1), (2, 1))));
        assert_eq!(parse_range("A1:"), None);
        assert_eq!(parse_column("AA"), Some(26));
        assert_eq!(
            (column_name(0), column_name(25), column_name(26), column_name(701)),
            ("A".into(), "Z".into(), "AA".into(), "ZZ".into())
        );
        assert_eq!(cell_name(2, 1), "B3");
    }

    #[test]
    fn who_added_it() {
        let mut d = Document {
            id: "d".into(),
            title: "T".into(),
            kind: DocumentKind::Doc,
            provider: "local".into(),
            provider_ref: "r".into(),
            url: None,
            actor: Actor::User,
            agent: None,
            created_at: 0,
            updated_at: 0,
            projects: vec![],
            tickets: vec![],
        };
        assert_eq!(d.added_by(), "you");
        d.actor = Actor::Agent;
        assert_eq!(d.added_by(), "an agent");
        d.agent = Some(AgentBy {
            session_id: "s".into(),
            project_id: None,
            persona: Some("PM".into()),
            from_ticket: None,
            from_ticket_id: None,
        });
        assert_eq!(d.added_by(), "PM");
    }
}
