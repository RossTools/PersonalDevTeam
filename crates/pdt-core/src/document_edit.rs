//! Edits to a document's content (PDT-316): the provider-neutral operations PDT, agents and workflows send, which each
//! provider maps to its own API (Google's Docs and Sheets requests, the local provider's files). A provider that can't
//! do one answers that it is unsupported. Pure: the operations, and A1 ranges.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A change to a document's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum DocumentEdit {
    /// A text document's whole content, as Markdown.
    ReplaceText {
        markdown: String,
    },
    /// Markdown added at a text document's end.
    AppendText {
        markdown: String,
    },
    /// Every `find` in a text document becomes `replace` (plain text, not Markdown).
    ReplaceAll {
        find: String,
        replace: String,
        #[serde(default)]
        match_case: bool,
    },
    /// Markdown put right after the heading with this text (the first, case-insensitive), before its section's text.
    InsertAfterHeading {
        heading: String,
        markdown: String,
    },
    /// A heading's section (everything after it up to the next heading of its level or higher) becomes this Markdown.
    /// The heading stays.
    ReplaceSection {
        heading: String,
        markdown: String,
    },

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
    /// Formats the cells of `range` (`A1:C10`, `B:B`, `2:2`); only what `format` sets changes.
    FormatCells {
        sheet: String,
        range: String,
        format: CellFormat,
    },
    /// Borders around `range`, and between its cells with `inner`.
    SetBorders {
        sheet: String,
        range: String,
        #[serde(default)]
        style: BorderStyle,
        /// `#rrggbb`; black when none.
        #[serde(default)]
        #[ts(optional)]
        color: Option<String>,
        #[serde(default)]
        inner: bool,
    },
    /// The width of columns (`A`, `B:D`), in pixels.
    ColumnWidth {
        sheet: String,
        columns: String,
        width: u32,
    },
    /// Keeps the first `rows` and `columns` in view while scrolling (0 unfreezes).
    Freeze {
        sheet: String,
        #[serde(default)]
        rows: u32,
        #[serde(default)]
        columns: u32,
    },
    /// Formats the cells of `range` that meet `when`.
    ConditionalFormat {
        sheet: String,
        range: String,
        when: CellCondition,
        format: CellFormat,
    },
    /// What may be typed into the cells of `range`; `strict` refuses anything else (else it is flagged).
    Validate {
        sheet: String,
        range: String,
        rule: CellCondition,
        #[serde(default)]
        strict: bool,
    },
    /// A chart of a sheet's data.
    AddChart {
        sheet: String,
        chart: ChartSpec,
    },
    /// Sorts the rows of `range` by its columns, in order.
    Sort {
        sheet: String,
        range: String,
        by: Vec<SortKey>,
    },
    /// A filter on `range` (the user picks what it shows); `None` clears the sheet's filter.
    Filter {
        sheet: String,
        #[serde(default)]
        #[ts(optional)]
        range: Option<String>,
    },
}

impl DocumentEdit {
    /// Whether it changes a text document (else a spreadsheet).
    pub fn is_text(&self) -> bool {
        matches!(
            self,
            DocumentEdit::ReplaceText { .. }
                | DocumentEdit::AppendText { .. }
                | DocumentEdit::ReplaceAll { .. }
                | DocumentEdit::InsertAfterHeading { .. }
                | DocumentEdit::ReplaceSection { .. }
        )
    }

    /// Its `op`, as sent (`set_cells`).
    pub fn op(&self) -> &'static str {
        match self {
            DocumentEdit::ReplaceText { .. } => "replace_text",
            DocumentEdit::AppendText { .. } => "append_text",
            DocumentEdit::ReplaceAll { .. } => "replace_all",
            DocumentEdit::InsertAfterHeading { .. } => "insert_after_heading",
            DocumentEdit::ReplaceSection { .. } => "replace_section",
            DocumentEdit::SetCells { .. } => "set_cells",
            DocumentEdit::AddSheet { .. } => "add_sheet",
            DocumentEdit::RenameSheet { .. } => "rename_sheet",
            DocumentEdit::DeleteSheet { .. } => "delete_sheet",
            DocumentEdit::FormatCells { .. } => "format_cells",
            DocumentEdit::SetBorders { .. } => "set_borders",
            DocumentEdit::ColumnWidth { .. } => "column_width",
            DocumentEdit::Freeze { .. } => "freeze",
            DocumentEdit::ConditionalFormat { .. } => "conditional_format",
            DocumentEdit::Validate { .. } => "validate",
            DocumentEdit::AddChart { .. } => "add_chart",
            DocumentEdit::Sort { .. } => "sort",
            DocumentEdit::Filter { .. } => "filter",
        }
    }
}

/// How cells look. Every field is optional: only those set change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
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
    pub strikethrough: Option<bool>,
    /// In points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub font_size: Option<u32>,
    /// `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color: Option<String>,
    /// The fill, `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub align: Option<Align>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub wrap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub number_format: Option<NumberFormat>,
}

impl CellFormat {
    pub fn is_empty(&self) -> bool {
        self == &CellFormat::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// How numbers show: a kind, and a pattern in the spreadsheet's own syntax (`#,##0.00`, `yyyy-mm-dd`) if wanted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NumberFormat {
    pub kind: NumberKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pattern: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NumberKind {
    Number,
    Currency,
    Percent,
    Date,
    Time,
    DateTime,
    Scientific,
    Text,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BorderStyle {
    #[default]
    Solid,
    Thick,
    Dashed,
    Dotted,
    Double,
    /// Takes borders away.
    None,
}

/// A test on a cell (conditional formatting, validation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellCondition {
    pub kind: CellConditionKind,
    /// What it compares with: one value, two for `between`, the choices for `one_of`, the formula for `formula`
    /// (`=B2>A2`), none for `blank` and the like.
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CellConditionKind {
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
    Equal,
    NotEqual,
    Between,
    NotBetween,
    TextContains,
    TextNotContains,
    TextStartsWith,
    TextEndsWith,
    Blank,
    NotBlank,
    /// A custom formula that is true for the cells it applies to.
    Formula,
    /// One of a list (validation only: a drop-down).
    OneOf,
    /// A tick box (validation only).
    Checkbox,
    /// A real date (validation only).
    IsDate,
}

impl CellCondition {
    /// How many values it takes (`None`: any number, at least one).
    pub fn arity(&self) -> Option<usize> {
        match self.kind {
            CellConditionKind::Blank
            | CellConditionKind::NotBlank
            | CellConditionKind::Checkbox
            | CellConditionKind::IsDate => Some(0),
            CellConditionKind::Between | CellConditionKind::NotBetween => Some(2),
            CellConditionKind::OneOf => None,
            _ => Some(1),
        }
    }

    /// Err when it has the wrong number of values.
    pub fn check(&self) -> Result<(), String> {
        let n = self.values.len();
        match self.arity() {
            Some(want) if n != want => Err(format!("a {:?} condition takes {want} value(s), not {n}", self.kind)),
            None if n == 0 => Err("a one_of condition needs its choices".into()),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChartKind {
    Line,
    Bar,
    Column,
    Area,
    Pie,
    Scatter,
}

/// A chart: its kind, the range along its axis (or a pie's labels) and one range per series, all on the edit's sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartSpec {
    pub kind: ChartKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    /// `A1:A10`.
    pub domain: String,
    /// `B1:B10`, one per series (a pie takes one).
    pub series: Vec<String>,
    /// The ranges' first row is their headers (the series' names). Yes when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub headers: Option<bool>,
    /// The cell its top left corner sits on (`E2`), on `placeSheet` or the edit's sheet; a sheet of its own when
    /// neither is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub anchor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub place_sheet: Option<String>,
}

/// A column to sort by (`B`), and which way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SortKey {
    pub column: String,
    #[serde(default)]
    pub descending: bool,
}

/// A block of cells, zero-based with exclusive ends; an end left open runs to the sheet's edge (`B:B`, `2:4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRange {
    pub start_row: Option<usize>,
    pub end_row: Option<usize>,
    pub start_col: Option<usize>,
    pub end_col: Option<usize>,
}

/// A column's letters (`AA`) as its zero-based index.
pub fn parse_column(s: &str) -> Option<usize> {
    let s = s.trim().to_ascii_uppercase();
    if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    Some(s.bytes().fold(0usize, |n, b| n * 26 + usize::from(b - b'A') + 1) - 1)
}

/// A column's letters from its zero-based index (`0` → `A`, `26` → `AA`).
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

/// One end of a range: a cell (`B3`), a column (`B`) or a row (`3`), as (row, column).
fn parse_end(s: &str) -> Option<(Option<usize>, Option<usize>)> {
    let s = s.trim();
    if let Some((r, c)) = crate::document::parse_cell(s) {
        return Some((Some(r), Some(c)));
    }
    if let Some(c) = parse_column(s) {
        return Some((None, Some(c)));
    }
    let r: usize = s.parse().ok().filter(|r| *r >= 1)?;
    Some((Some(r - 1), None))
}

/// A range in A1 notation (`A1`, `A1:C10`, `B:D`, `2:5`), without a sheet name. None for anything else, or a range
/// that runs backwards.
pub fn parse_range(s: &str) -> Option<CellRange> {
    let (a, b) = s.split_once(':').unwrap_or((s, s));
    let (r1, c1) = parse_end(a)?;
    let (r2, c2) = parse_end(b)?;
    // Both ends name the same sort of place: cells, columns or rows.
    if r1.is_some() != r2.is_some() || c1.is_some() != c2.is_some() {
        return None;
    }
    if r1 > r2 || c1 > c2 {
        return None;
    }
    Some(CellRange { start_row: r1, end_row: r2.map(|r| r + 1), start_col: c1, end_col: c2.map(|c| c + 1) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ranges_parse() {
        let r = |a, b, c, d| CellRange { start_row: a, end_row: b, start_col: c, end_col: d };
        assert_eq!(parse_range("A1"), Some(r(Some(0), Some(1), Some(0), Some(1))));
        assert_eq!(parse_range("b2:c10"), Some(r(Some(1), Some(10), Some(1), Some(3))));
        assert_eq!(parse_range("B:D"), Some(r(None, None, Some(1), Some(4))));
        assert_eq!(parse_range("2:5"), Some(r(Some(1), Some(5), None, None)));
        assert_eq!(parse_range("C1:A1"), None);
        assert_eq!(parse_range("A1:B"), None);
        assert_eq!(parse_range("nope"), None);
        assert_eq!(parse_range("0:1"), None);
    }

    #[test]
    fn columns_round_trip() {
        for (i, s) in [(0, "A"), (25, "Z"), (26, "AA"), (27, "AB"), (701, "ZZ"), (702, "AAA")] {
            assert_eq!(column_name(i), s);
            assert_eq!(parse_column(s), Some(i));
        }
    }

    #[test]
    fn edits_read_as_sent() {
        let e: DocumentEdit = serde_json::from_value(json!({
            "op": "conditional_format", "sheet": "Data", "range": "B2:B20",
            "when": {"kind": "greater_than", "values": ["100"]},
            "format": {"bold": true, "background": "#ffeeee"}
        }))
        .unwrap();
        assert_eq!(e.op(), "conditional_format");
        assert!(!e.is_text());
        let e: DocumentEdit = serde_json::from_value(json!({
            "op": "add_chart", "sheet": "Data",
            "chart": {"kind": "column", "domain": "A1:A5", "series": ["B1:B5"], "anchor": "E2"}
        }))
        .unwrap();
        let DocumentEdit::AddChart { chart, .. } = e else { panic!() };
        assert_eq!((chart.kind, chart.headers), (ChartKind::Column, None));
        let e: DocumentEdit =
            serde_json::from_value(json!({"op": "replace_all", "find": "a", "replace": "b"})).unwrap();
        assert!(e.is_text());
    }

    #[test]
    fn conditions_take_their_number_of_values() {
        let c = |kind, values: &[&str]| CellCondition { kind, values: values.iter().map(|v| v.to_string()).collect() };
        assert!(c(CellConditionKind::Between, &["1", "2"]).check().is_ok());
        assert!(c(CellConditionKind::Between, &["1"]).check().is_err());
        assert!(c(CellConditionKind::Blank, &[]).check().is_ok());
        assert!(c(CellConditionKind::OneOf, &[]).check().is_err());
        assert!(c(CellConditionKind::OneOf, &["a", "b", "c"]).check().is_ok());
    }
}
