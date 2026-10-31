//! Documents (PDT-314): design docs, plans and working spreadsheets that live outside the repo, kept by a provider
//! (PDT's local one, or a plugin's, such as Google's) and linked to projects and tickets. PDT keeps a record of each
//! (title, kind, which provider holds it and the provider's reference) and its links; the content stays with the
//! provider. Pure: the types, titles, provider ids, settings and the design doc template.

use crate::ticket::{Actor, AgentBy};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The longest title kept (in characters).
pub const MAX_TITLE: usize = 200;

/// A text document or a spreadsheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
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
    /// Keeps charts on its sheets ([`ChartSpec`]).
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

/// One sheet of a spreadsheet: its rows of cells from A1.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Cell>>,
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

/// A change to a document's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum DocumentEdit {
    /// A text document's whole content, as Markdown.
    ReplaceText { markdown: String },
    /// Markdown added at a text document's end.
    AppendText { markdown: String },
    /// Values written into a sheet from `cell` (`B3`) across and down; a value starting with `=` is a formula.
    SetCells { sheet: String, cell: String, values: Vec<Vec<String>> },
    /// A new, empty sheet.
    AddSheet { name: String },
    /// Renames a sheet.
    RenameSheet { sheet: String, name: String },
    /// Deletes a sheet (not the last one).
    DeleteSheet { sheet: String },
    /// Formats a range (`A1:C3`): a style path and its value, as IronCalc names them: `font.b` / `font.i` /
    /// `font.u` / `font.strike` (`true`/`false`), `font.color` / `fill.color` (`#RRGGBB`), `num_fmt` (`#,##0.00`,
    /// `0%`), `alignment.horizontal` (`left`, `center`, `right`), `alignment.wrap_text` (`true`).
    SetStyle { sheet: String, range: String, style: String, value: String },
    /// Sorts the rows of a range by one of its columns (a letter, `B`), keeping formulas pointing at the moved rows.
    Sort {
        sheet: String,
        range: String,
        column: String,
        #[serde(default)]
        descending: bool,
    },
    /// Shows only the rows of a range whose cell in `column` shows one of `values` (hides the others); no values
    /// shows every row of the range again.
    Filter {
        sheet: String,
        range: String,
        column: String,
        #[serde(default)]
        values: Vec<String>,
    },
    /// Adds a chart, or replaces the one with its id.
    SetChart { chart: ChartSpec },
    /// Removes a chart.
    DeleteChart { id: String },
}

impl DocumentEdit {
    /// Whether it changes a sheet's charts (kept by PDT beside the file) rather than its cells.
    pub fn is_chart(&self) -> bool {
        matches!(self, DocumentEdit::SetChart { .. } | DocumentEdit::DeleteChart { .. })
    }
}

/// What a chart draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChartKind {
    Bar,
    Line,
    Area,
    Pie,
    Scatter,
}

/// One series of a chart: its name and the cells of its values (a row or column, `B2:B5`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartSeries {
    pub name: String,
    pub values: String,
}

/// A chart on a sheet, drawn by PDT from the sheet's cells and floating over it at `anchor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartSpec {
    /// Its id in the document (made up when blank).
    #[serde(default)]
    pub id: String,
    pub kind: ChartKind,
    #[serde(default)]
    pub title: String,
    pub sheet: String,
    /// The labels along the axis (a row or column, `A2:A5`).
    pub categories: String,
    pub series: Vec<ChartSeries>,
    /// The cell its top-left corner sits on (`F2`).
    pub anchor: String,
    /// Its size in pixels (480 × 300 when not given).
    #[serde(default = "chart_width")]
    pub width: u32,
    #[serde(default = "chart_height")]
    pub height: u32,
}

fn chart_width() -> u32 {
    480
}
fn chart_height() -> u32 {
    300
}

impl ChartSpec {
    /// The spec as given, checked: its ranges and anchor parse, it has a sheet and a series, and its size is sane.
    pub fn cleaned(mut self) -> Result<Self, String> {
        self.id = self.id.trim().to_string();
        if self.id.is_empty() {
            self.id = format!("chart-{}", uuid_like());
        }
        if self.id.len() > 64 || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err("a chart's id is letters, digits, - and _".into());
        }
        self.title = self.title.trim().chars().take(MAX_TITLE).collect();
        if self.sheet.trim().is_empty() {
            return Err("a chart needs a sheet".into());
        }
        parse_range(&self.categories).ok_or_else(|| format!("{} is not a range", self.categories))?;
        if self.series.is_empty() {
            return Err("a chart needs a series".into());
        }
        for s in &self.series {
            parse_range(&s.values).ok_or_else(|| format!("{} is not a range", s.values))?;
        }
        parse_cell(&self.anchor).ok_or_else(|| format!("{} is not a cell", self.anchor))?;
        self.width = self.width.clamp(160, 2000);
        self.height = self.height.clamp(120, 2000);
        Ok(self)
    }
}

/// A short id from the clock (charts' ids only need to differ within one document).
fn uuid_like() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}", n & 0xffff_ffff_ffff)
}

/// A chart with the numbers it draws, read from the sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartView {
    pub spec: ChartSpec,
    /// The category labels, as the cells show them.
    pub labels: Vec<String>,
    /// Each series' values (none where a cell holds no number).
    pub values: Vec<Vec<Option<f64>>>,
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
pub struct NewComment {
    pub body: String,
    #[serde(default)]
    #[ts(optional)]
    pub anchor: Option<CommentAnchor>,
    /// `user` from PDT's UI.
    #[serde(default)]
    #[ts(optional)]
    pub by: Option<Actor>,
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

/// A range's (`B3:D9`, or one cell `B3`) first and last cell as zero-based (row, column), in order.
pub fn parse_range(range: &str) -> Option<((usize, usize), (usize, usize))> {
    let (a, b) = range.trim().split_once(':').unwrap_or((range, range));
    let ((r1, c1), (r2, c2)) = (parse_cell(a)?, parse_cell(b)?);
    Some(((r1.min(r2), c1.min(c2)), (r1.max(r2), c1.max(c2))))
}

/// A zero-based column's letters: 0 is `A`, 26 is `AA`.
pub fn column_letters(col: usize) -> String {
    let mut n = col + 1;
    let mut out = Vec::new();
    while n > 0 {
        out.push(b'A' + ((n - 1) % 26) as u8);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// A column's letters (`B`, `aa`) as its zero-based index.
pub fn parse_column(letters: &str) -> Option<usize> {
    parse_cell(&format!("{}1", letters.trim())).map(|(_, c)| c)
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
    }

    #[test]
    fn ranges_and_columns_parse() {
        assert_eq!(parse_range("B3:D9"), Some(((2, 1), (8, 3))));
        assert_eq!(parse_range("D9:B3"), Some(((2, 1), (8, 3))));
        assert_eq!(parse_range("C2"), Some(((1, 2), (1, 2))));
        assert_eq!(parse_range("C2:"), None);
        assert_eq!(column_letters(0), "A");
        assert_eq!(column_letters(25), "Z");
        assert_eq!(column_letters(26), "AA");
        assert_eq!(column_letters(701), "ZZ");
        assert_eq!(parse_column("aa"), Some(26));
        assert_eq!(parse_column("1"), None);
    }

    #[test]
    fn charts_and_anchors_are_checked() {
        let chart = ChartSpec {
            id: " ".into(),
            kind: ChartKind::Bar,
            title: " Sales ".into(),
            sheet: "Sheet1".into(),
            categories: "A2:A5".into(),
            series: vec![ChartSeries { name: "Revenue".into(), values: "B2:B5".into() }],
            anchor: "F2".into(),
            width: 10,
            height: 300,
        };
        let c = chart.clone().cleaned().unwrap();
        assert!(c.id.starts_with("chart-"), "{}", c.id);
        assert_eq!((c.title.as_str(), c.width), ("Sales", 160));
        assert!(ChartSpec { anchor: "here".into(), ..chart.clone() }.cleaned().is_err());
        assert!(ChartSpec { series: vec![], ..chart.clone() }.cleaned().is_err());
        assert!(ChartSpec { id: "a b".into(), ..chart }.cleaned().is_err());

        let a = CommentAnchor::Cell { sheet: "Sheet1".into(), range: " b3:c4 ".into() }.cleaned().unwrap();
        assert_eq!(a.label(), "Sheet1!B3:C4");
        assert!(CommentAnchor::Cell { sheet: "S".into(), range: "x".into() }.cleaned().is_err());
        let h = CommentAnchor::Heading { heading: "## Goal ".into() }.cleaned().unwrap();
        assert_eq!(h, CommentAnchor::Heading { heading: "Goal".into() });
        assert!(CommentAnchor::Text { quote: "  ".into() }.cleaned().is_err());
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
