//! PDT's local sheets (PDT-315) on IronCalc, the engine the browser's editor runs too (`@ironcalc/workbook`, the same
//! version: the two exchange IronCalc's bytes and diffs, see [`IRONCALC_VERSION`]). A sheet's file of record is a
//! `.xlsx`; this crate loads and saves it, reads cells with their computed values, applies PDT's edits (values,
//! formats, sheets, sort and filter), imports `.csv`, exports `.csv`, and passes IronCalc's sync diffs in and out
//! ([`Workbook::flush`], [`Workbook::apply_diffs`]).
//!
//! Formulas are recalculated on load (a file's cached values are not trusted) and after every edit.

use ironcalc::base::UserModel;
use ironcalc::base::cell::CellValue;
use ironcalc::base::cf_types::CfRuleInput;
use ironcalc::base::expressions::types::Area;
use ironcalc::base::types::{Color, Dxf, DxfFont};
use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use pdt_core::document::{
    BorderStyle, Cell, CellCondition, CellFormat, HAlign, MAX_COLUMNS, MAX_ROWS, Sheet, SortKey, VAlign, cell_name,
    column_name, parse_column, parse_range,
};
use std::io::Cursor;
use std::path::Path;

mod limits;
pub use limits::{MAX_CELLS, MAX_FORMULA_CHARS, MAX_FORMULA_DEPTH, MAX_FORMULA_OPERATORS, MAX_UNPACKED, check_formula};

/// A cell's text for a CSV file: text starting with `=`, `+`, `-`, `@`, a tab or a carriage return gets a leading `'`,
/// so a spreadsheet opening the file shows it rather than running it as a formula (PDT-542). A `number` (`-3`) stays.
pub fn csv_cell(value: &str, number: bool) -> std::borrow::Cow<'_, str> {
    if !number && value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}").into()
    } else {
        value.into()
    }
}

/// Rows of shown text as CSV, quoted where needed and each cell through [`csv_cell`]; `number` says whether a cell's
/// text is a number (a provider that only gives text decides by parsing it).
pub fn write_csv<R: IntoIterator<Item = Vec<String>>>(rows: R, number: impl Fn(&str) -> bool) -> Result<String> {
    let mut w = csv::WriterBuilder::new().flexible(true).from_writer(Vec::new());
    for row in rows {
        let line: Vec<_> = row.iter().map(|v| csv_cell(v, number(v))).collect();
        w.write_record(line.iter().map(|v| v.as_bytes())).map_err(|e| e.to_string())?;
    }
    let bytes = w.into_inner().map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// The IronCalc version in pdtd; the web UI's `@ironcalc/workbook` must be the same (a test checks).
pub const IRONCALC_VERSION: &str = "0.9.1";

const LOCALE: &str = "en";
const TZ: &str = "UTC";
const LANGUAGE: &str = "en";

/// The most rows and columns [`Workbook::sheets`] reads (what agents and the plain view see).
pub const READ_LIMIT: (usize, usize) = (2000, 200);

pub type Result<T> = std::result::Result<T, String>;

/// A zero-based row as IronCalc's one-based index; an error past the sheet's last row (never a wrapped `as` cast).
fn row_at(r: usize) -> Result<i32> {
    if r >= MAX_ROWS {
        return Err(format!("row {} is past a sheet's last row ({MAX_ROWS})", r.saturating_add(1)));
    }
    i32::try_from(r + 1).map_err(|e| e.to_string())
}

/// A zero-based column as IronCalc's one-based index; an error past the sheet's last column.
fn col_at(c: usize) -> Result<i32> {
    if c >= MAX_COLUMNS {
        return Err(format!("column {} is past a sheet's last column (XFD)", column_name(c)));
    }
    i32::try_from(c + 1).map_err(|e| e.to_string())
}

/// A workbook: IronCalc's model of one `.xlsx`.
pub struct Workbook {
    model: UserModel<'static>,
}

impl std::fmt::Debug for Workbook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workbook").field("sheets", &self.sheet_names()).finish()
    }
}

/// A typed value, as the engine computed it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
}

impl Workbook {
    /// An empty workbook with one sheet, `Sheet1`.
    pub fn new(name: &str) -> Result<Self> {
        let mut model = UserModel::new_empty("Workbook", LOCALE, TZ, LANGUAGE)?;
        model.set_name(name);
        Ok(Self { model })
    }

    /// A workbook from a `.xlsx` file's bytes, recalculated. A package past PDT's limits (its unpacked size, cells,
    /// formulas) is refused before IronCalc reads it.
    pub fn from_xlsx(bytes: &[u8], name: &str) -> Result<Self> {
        limits::check_xlsx(bytes)?;
        // IronCalc's importer panics on some packages (a worksheet outside `worksheets/`): an error like any other.
        let loaded = std::panic::catch_unwind(|| load_from_xlsx_bytes(bytes, name, LOCALE, TZ));
        let wb = loaded
            .map_err(|_| "not a spreadsheet PDT can read".to_string())?
            .map_err(|e| format!("not a spreadsheet PDT can read: {e}"))?;
        let model = ironcalc::base::Model::from_workbook(wb, LANGUAGE)?;
        let mut model = UserModel::from_model(model);
        model.evaluate();
        Ok(Self { model })
    }

    /// A workbook from a `.csv` file (comma, semicolon or tab separated), one sheet named `sheet`. Cells are typed as
    /// if entered by hand: numbers stay numbers, and a cell starting with `=` is a formula. At most [`MAX_CELLS`]
    /// cells, and each formula within [`check_formula`]'s limits.
    pub fn from_csv(text: &str, name: &str, sheet: &str) -> Result<Self> {
        let delimiter = sniff_delimiter(text);
        let mut wb = Self::new(name)?;
        if !sheet.trim().is_empty() && sheet != "Sheet1" {
            wb.model.rename_sheet(0, sheet.trim())?;
        }
        let text = text.trim_start_matches('\u{feff}');
        let records = || {
            csv::ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .delimiter(delimiter)
                .from_reader(text.as_bytes())
                .into_records()
                .take(MAX_ROWS)
                .enumerate()
                .map(|(r, record)| record.map(|v| (r, v)).map_err(|e| format!("not a CSV file PDT can read: {e}")))
        };
        // Checked whole before IronCalc sees a cell.
        let mut cells = 0usize;
        for record in records() {
            let (r, record) = record?;
            for (c, v) in record.iter().enumerate().filter(|(_, v)| !v.is_empty()) {
                cells += 1;
                if cells > MAX_CELLS {
                    return Err(format!("a sheet file holds at most {MAX_CELLS} cells"));
                }
                limits::check_input(v).map_err(|e| format!("{}: {e}", cell_name(r, c)))?;
            }
        }
        wb.model.pause_evaluation();
        for record in records() {
            let (r, record) = record?;
            for (c, v) in record.iter().enumerate().filter(|(_, v)| !v.is_empty()) {
                wb.model.set_user_input(0, row_at(r)?, col_at(c)?, v)?;
            }
        }
        wb.model.resume_evaluation();
        wb.model.evaluate();
        wb.model.flush_send_queue();
        Ok(wb)
    }

    /// A workbook from IronCalc's own bytes (what a view loads).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut model = UserModel::from_bytes(bytes, LANGUAGE)?;
        model.evaluate();
        Ok(Self { model })
    }

    /// Opens a `.xlsx` file.
    pub fn open(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Self::from_xlsx(&bytes, &name)
    }

    /// The workbook as a `.xlsx` file.
    pub fn to_xlsx(&self) -> Result<Vec<u8>> {
        let out = save_xlsx_to_writer(self.model.get_model(), Cursor::new(Vec::new())).map_err(|e| e.to_string())?;
        Ok(out.into_inner())
    }

    /// Saves it as a `.xlsx`: written beside the file, then renamed over it, so a reader never sees half a file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = self.to_xlsx()?;
        let tmp = path.with_extension("xlsx.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// IronCalc's bytes of the model, for a view to load (`Model.fromBytes`).
    pub fn to_bytes(&self) -> Vec<u8> {
        self.model.to_bytes()
    }

    /// The diffs of the edits since the last flush (to send to the views), emptying the queue.
    pub fn flush(&mut self) -> Vec<u8> {
        self.model.flush_send_queue()
    }

    /// A view's edits (its `flushSendQueue()`), applied and recalculated.
    pub fn apply_diffs(&mut self, diffs: &[u8]) -> Result<()> {
        self.model.apply_external_diffs(diffs)?;
        self.model.evaluate();
        // The diffs came from a view: they are not this model's to send on (the caller relays the view's own bytes).
        self.model.flush_send_queue();
        Ok(())
    }

    pub fn sheet_names(&self) -> Vec<String> {
        self.model.get_worksheets_properties().into_iter().map(|p| p.name).collect()
    }

    /// A sheet's index by its name (case-insensitive, as spreadsheets compare them).
    pub fn sheet_index(&self, name: &str) -> Result<u32> {
        self.sheet_names()
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name.trim()))
            .map(|i| i as u32)
            .ok_or_else(|| format!("there is no sheet called {name}"))
    }

    /// The last used row and column of a sheet (1-based; (0, 0) when it is empty).
    fn used(&self, sheet: u32) -> Result<(i32, i32)> {
        let ws = self.model.get_model().workbook.worksheet(sheet)?;
        if ws.sheet_data.is_empty() {
            return Ok((0, 0));
        }
        let d = ws.dimension();
        Ok((d.max_row, d.max_column))
    }

    /// Every sheet's cells from A1 to the last used one (at most [`READ_LIMIT`]): what each shows, and its formula.
    pub fn sheets(&self) -> Result<Vec<Sheet>> {
        let mut out = Vec::new();
        for (i, name) in self.sheet_names().into_iter().enumerate() {
            let i = i as u32;
            let (rows, cols) = self.used(i)?;
            let (rows, cols) = ((rows as usize).min(READ_LIMIT.0), (cols as usize).min(READ_LIMIT.1));
            let mut grid = Vec::with_capacity(rows);
            for r in 1..=rows as i32 {
                let mut line = Vec::with_capacity(cols);
                for c in 1..=cols as i32 {
                    let input = self.model.get_cell_content(i, r, c)?;
                    let value = self.model.get_formatted_cell_value(i, r, c)?;
                    line.push(Cell { value, formula: input.starts_with('=').then_some(input) });
                }
                while line.last().is_some_and(|c: &Cell| c.value.is_empty() && c.formula.is_none()) {
                    line.pop();
                }
                grid.push(line);
            }
            out.push(Sheet { name, rows: grid, charts: vec![] });
        }
        Ok(out)
    }

    /// The typed values of a range (`B2:B5`), row by row.
    pub fn values(&self, sheet: &str, range: &str) -> Result<Vec<Vec<Value>>> {
        let i = self.sheet_index(sheet)?;
        let ((r1, c1), (r2, c2)) = parse_range(range).ok_or_else(|| format!("{range} is not a range"))?;
        let m = self.model.get_model();
        let mut out = Vec::new();
        for r in r1..=r2.min(r1 + 10_000) {
            let mut line = Vec::new();
            for c in c1..=c2.min(c1 + 1_000) {
                line.push(match m.get_cell_value_by_index(i, row_at(r)?, col_at(c)?)? {
                    CellValue::None => Value::Empty,
                    CellValue::Number(n) => Value::Number(n),
                    CellValue::String(s) => Value::Text(s),
                    CellValue::Boolean(b) => Value::Bool(b),
                });
            }
            out.push(line);
        }
        Ok(out)
    }

    /// What a range's cells show (formatted), row by row.
    pub fn shown(&self, sheet: &str, range: &str) -> Result<Vec<Vec<String>>> {
        let i = self.sheet_index(sheet)?;
        let ((r1, c1), (r2, c2)) = parse_range(range).ok_or_else(|| format!("{range} is not a range"))?;
        let mut out = Vec::new();
        for r in r1..=r2.min(r1 + 10_000) {
            let mut line = Vec::new();
            for c in c1..=c2.min(c1 + 1_000) {
                line.push(self.model.get_formatted_cell_value(i, row_at(r)?, col_at(c)?)?);
            }
            out.push(line);
        }
        Ok(out)
    }

    /// Writes values into a sheet from `cell` across and down; a value starting with `=` is a formula. Nothing is
    /// written when a formula is past [`check_formula`]'s limits.
    pub fn set_cells(&mut self, sheet: &str, cell: &str, values: &[Vec<String>]) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        let ((row, col), _) = parse_range(cell).ok_or_else(|| format!("{cell} is not a cell"))?;
        for (dr, line) in values.iter().enumerate() {
            for (dc, v) in line.iter().enumerate() {
                limits::check_input(v)
                    .map_err(|e| format!("{}: {e}", cell_name(row.saturating_add(dr), col.saturating_add(dc))))?;
            }
        }
        self.model.pause_evaluation();
        let r = (|| {
            for (dr, line) in values.iter().enumerate() {
                for (dc, v) in line.iter().enumerate() {
                    let (r, c) = (row_at(row.saturating_add(dr))?, col_at(col.saturating_add(dc))?);
                    if v.is_empty() {
                        self.model.range_clear_contents(&Area { sheet: i, row: r, column: c, width: 1, height: 1 })?;
                    } else {
                        self.model.set_user_input(i, r, c, v)?;
                    }
                }
            }
            Ok(())
        })();
        self.model.resume_evaluation();
        self.model.evaluate();
        r
    }

    pub fn add_sheet(&mut self, name: &str) -> Result<()> {
        if self.sheet_index(name).is_ok() {
            return Err(format!("there is a sheet called {name} already"));
        }
        self.model.new_sheet()?;
        let last = self.sheet_names().len() as u32 - 1;
        self.model.rename_sheet(last, name.trim())
    }

    pub fn rename_sheet(&mut self, sheet: &str, name: &str) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        self.model.rename_sheet(i, name.trim())
    }

    pub fn delete_sheet(&mut self, sheet: &str) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        if self.sheet_names().len() == 1 {
            return Err("a spreadsheet keeps at least one sheet".into());
        }
        self.model.delete_sheet(i)
    }

    /// Formats a range: an IronCalc style path (`font.b`, `fill.color`, `num_fmt`, …) and its value.
    pub fn set_style(&mut self, sheet: &str, range: &str, style: &str, value: &str) -> Result<()> {
        let area = self.area(sheet, range)?;
        self.model.update_range_style(&area, style.trim(), value.trim())
    }

    fn area(&self, sheet: &str, range: &str) -> Result<Area> {
        let i = self.sheet_index(sheet)?;
        let ((r1, c1), (r2, c2)) = parse_range(range).ok_or_else(|| format!("{range} is not a range"))?;
        let (top, left) = (row_at(r1)?, col_at(c1)?);
        Ok(Area { sheet: i, row: top, column: left, width: col_at(c2)? - left + 1, height: row_at(r2)? - top + 1 })
    }

    /// A range to sort or filter: as given, or for none the sheet's used cells from A1.
    fn table(&self, sheet: u32, range: &str) -> Result<String> {
        if !range.trim().is_empty() {
            return Ok(range.to_string());
        }
        let (rows, cols) = self.used(sheet)?;
        if rows < 1 {
            return Err("the sheet is empty".into());
        }
        Ok(format!("A1:{}", cell_name(rows as usize - 1, cols as usize - 1)))
    }

    /// The zero-based column `column` names, checked to be inside the range's columns.
    fn column_in(column: &str, c1: usize, c2: usize) -> Result<usize> {
        let key = parse_column(column).ok_or_else(|| format!("{column} is not a column (A, B, …)"))?;
        if key < c1 || key > c2 {
            return Err(format!("column {} is outside the range", column.trim()));
        }
        Ok(key)
    }

    fn typed(&self, sheet: u32, row: i32, col: i32) -> Result<Value> {
        Ok(match self.model.get_model().get_cell_value_by_index(sheet, row, col)? {
            CellValue::None => Value::Empty,
            CellValue::Number(x) => Value::Number(x),
            CellValue::String(s) if s.is_empty() => Value::Empty,
            CellValue::String(s) => Value::Text(s),
            CellValue::Boolean(b) => Value::Bool(b),
        })
    }

    /// Puts a format on a range: only the fields given change.
    pub fn format(&mut self, sheet: &str, range: &str, f: &CellFormat) -> Result<()> {
        let flag = |b: bool| if b { "true" } else { "false" }.to_string();
        let mut paths: Vec<(&str, String)> = Vec::new();
        if let Some(b) = f.bold {
            paths.push(("font.b", flag(b)));
        }
        if let Some(b) = f.italic {
            paths.push(("font.i", flag(b)));
        }
        if let Some(b) = f.underline {
            paths.push(("font.u", flag(b)));
        }
        if let Some(c) = &f.color {
            paths.push(("font.color", hex(c)?));
        }
        if let Some(c) = &f.background {
            paths.push(("fill.color", hex(c)?));
        }
        if let Some(n) = f.size {
            paths.push(("font.size", n.clamp(1, 409).to_string()));
        }
        if let Some(n) = &f.number_format {
            paths.push(("num_fmt", n.clone()));
        }
        if let Some(h) = f.horizontal {
            paths.push((
                "alignment.horizontal",
                match h {
                    HAlign::Left => "left",
                    HAlign::Center => "center",
                    HAlign::Right => "right",
                }
                .into(),
            ));
        }
        if let Some(v) = f.vertical {
            paths.push((
                "alignment.vertical",
                match v {
                    VAlign::Top => "top",
                    VAlign::Middle => "center",
                    VAlign::Bottom => "bottom",
                }
                .into(),
            ));
        }
        if let Some(b) = f.wrap {
            paths.push(("alignment.wrap_text", flag(b)));
        }
        if paths.is_empty() {
            return Err("give the format something to change".into());
        }
        for (path, value) in paths {
            self.set_style(sheet, range, path, &value)?;
        }
        Ok(())
    }

    /// Borders around a range (`inner`: between its cells too); [`BorderStyle::None`] takes them away.
    pub fn borders(
        &mut self,
        sheet: &str,
        range: &str,
        style: BorderStyle,
        color: Option<&str>,
        inner: bool,
    ) -> Result<()> {
        let area = self.area(sheet, range)?;
        let color = hex(color.unwrap_or("#000000"))?;
        let (line, kind) = match style {
            BorderStyle::Solid => ("thin", if inner { "All" } else { "Outer" }),
            BorderStyle::Thick => ("thick", if inner { "All" } else { "Outer" }),
            BorderStyle::Dashed => ("mediumdashed", if inner { "All" } else { "Outer" }),
            BorderStyle::Dotted => ("dotted", if inner { "All" } else { "Outer" }),
            BorderStyle::Double => ("double", if inner { "All" } else { "Outer" }),
            BorderStyle::None => ("thin", "None"),
        };
        // IronCalc builds a border only from its JSON (its own tests do the same).
        let json = format!(r#"{{"item": {{"style": "{line}", "color": "{color}"}}, "type": "{kind}"}}"#);
        let border: ironcalc::base::BorderArea = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        self.model.set_area_with_border(&area, &border)
    }

    /// Sets the width of columns (`B`, or `B:D`) in pixels.
    pub fn column_width(&mut self, sheet: &str, columns: &str, width: u32) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        let (a, b) = columns.split_once(':').unwrap_or((columns, columns));
        let first = parse_column(a).ok_or_else(|| format!("{a} is not a column (A, B, …)"))?;
        let last = parse_column(b).ok_or_else(|| format!("{b} is not a column (A, B, …)"))?;
        let (first, last) = (col_at(first.min(last))?, col_at(first.max(last))?);
        self.model.set_columns_width(i, first, last, f64::from(width.clamp(5, 2000)))
    }

    /// Keeps the first `rows` and `columns` in view while scrolling (0 unfreezes).
    pub fn freeze(&mut self, sheet: &str, rows: u32, columns: u32) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        self.model.set_frozen_rows_count(i, rows.min(1000) as i32)?;
        self.model.set_frozen_columns_count(i, columns.min(100) as i32)
    }

    /// Shows a format on the range's cells that meet the condition (a conditional format, kept in the file).
    pub fn conditional_format(&mut self, sheet: &str, range: &str, when: &CellCondition, f: &CellFormat) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        let ((r1, c1), (r2, c2)) = parse_range(range).ok_or_else(|| format!("{range} is not a range"))?;
        let formula = condition_formula(when, &cell_name(r1, c1));
        check_formula(&formula)?;
        let color = f.color.as_deref().map(hex).transpose()?.map(Color::Rgb).unwrap_or_default();
        let font = (f.bold.is_some() || f.italic.is_some() || f.underline.is_some() || f.color.is_some())
            .then(|| DxfFont { b: f.bold, i: f.italic, u: f.underline, color, ..Default::default() });
        let fill = match &f.background {
            Some(c) => Some(ironcalc::base::types::Fill { color: Color::Rgb(hex(c)?) }),
            None => None,
        };
        if font.is_none() && fill.is_none() {
            return Err("a conditional format shows as bold, italic, underline, a colour or a fill".into());
        }
        let format = Dxf { font, fill, ..Default::default() };
        let area = format!("{}:{}", cell_name(r1, c1), cell_name(r2, c2));
        self.model.add_conditional_formatting(i, &area, CfRuleInput::Formula { formula, format, stop_if_true: false })
    }

    /// Sorts the range's rows (none: the used cells from A1) by its columns in order: numbers before text before
    /// empty cells, text without regard to case; with `header`, the first row stays on top. As spreadsheets do, the
    /// range's cells are rewritten in their new order, a formula's references shifting with it as if copied (`=B2-C2`
    /// moved to row 4 is `=B4-C4`), and formats move with their cells; formulas elsewhere keep pointing at the same
    /// cells. Rows hidden by a filter are shown first.
    pub fn sort(&mut self, sheet: &str, range: &str, by: &[SortKey], header: bool) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        let range = self.table(i, range)?;
        let ((r1, c1), (r2, c2)) = parse_range(&range).ok_or_else(|| format!("{range} is not a range"))?;
        if by.is_empty() {
            return Err("sort needs at least one column to sort by".into());
        }
        let keys: Vec<(usize, bool)> =
            by.iter().map(|k| Ok((Self::column_in(&k.column, c1, c2)?, k.descending))).collect::<Result<_>>()?;
        let first = row_at(r1)? + i32::from(header);
        let last = row_at(r2)?;
        let (left, right) = (col_at(c1)?, col_at(c2)?);
        if first > last {
            return Ok(());
        }
        self.model.set_rows_hidden(i, first, last, false)?;
        let mut rows: Vec<(usize, Vec<Value>)> = Vec::new();
        for (n, r) in (first..=last).enumerate() {
            let vals = keys.iter().map(|(k, _)| self.typed(i, r, col_at(*k)?)).collect::<Result<Vec<_>>>()?;
            rows.push((n, vals));
        }
        let rank = |v: &Value| match v {
            Value::Number(_) => 0,
            Value::Text(_) => 1,
            Value::Bool(_) => 2,
            Value::Empty => 3,
        };
        rows.sort_by(|(_, a), (_, b)| {
            keys.iter()
                .enumerate()
                .map(|(j, (_, descending))| {
                    let (a, b) = (&a[j], &b[j]);
                    let o = rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
                        (Value::Number(x), Value::Number(y)) => x.total_cmp(y),
                        (Value::Text(x), Value::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
                        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
                        _ => std::cmp::Ordering::Equal,
                    });
                    // Empty cells stay at the end either way.
                    if *descending && rank(a) != 3 && rank(b) != 3 { o.reverse() } else { o }
                })
                .find(|o| o.is_ne())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if rows.iter().enumerate().all(|(target, (orig, _))| target == *orig) {
            return Ok(());
        }
        // Every cell's input as it reads at its new row, and its format.
        let m = self.model.get_model();
        let mut inputs = Vec::with_capacity(rows.len());
        let mut styles = Vec::with_capacity(rows.len());
        for (target, (orig, _)) in rows.iter().enumerate() {
            let (from, to) = (first + *orig as i32, first + target as i32);
            let mut line = Vec::new();
            let mut fmt = Vec::new();
            for c in left..=right {
                line.push(m.extend_to(i, from, c, to, c)?);
                fmt.push(self.model.get_cell_style(i, from, c)?);
            }
            inputs.push(line);
            styles.push(fmt);
        }
        self.model.pause_evaluation();
        let r = (|| {
            self.model.range_clear_contents(&Area {
                sheet: i,
                row: first,
                column: left,
                width: right - left + 1,
                height: last - first + 1,
            })?;
            for (dr, line) in inputs.iter().enumerate() {
                for (dc, v) in line.iter().enumerate() {
                    if !v.is_empty() {
                        self.model.set_user_input(i, first + dr as i32, left + dc as i32, v)?;
                    }
                }
            }
            self.model.set_selected_sheet(i)?;
            self.model.set_selected_cell(first, left)?;
            self.model.set_selected_range(first, left, last, right)?;
            self.model.on_paste_styles(&styles)
        })();
        self.model.resume_evaluation();
        self.model.evaluate();
        r
    }

    /// A filter on a range (none: the used cells from A1), whose first row is the header: rows below it whose
    /// `column` cell doesn't meet the condition are hidden.
    pub fn filter(&mut self, sheet: &str, range: &str, column: &str, when: &CellCondition) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        let range = self.table(i, range)?;
        let ((r1, c1), (r2, c2)) = parse_range(&range).ok_or_else(|| format!("{range} is not a range"))?;
        let key = col_at(Self::column_in(column, c1, c2)?)?;
        let (first, last) = (row_at(r1)? + 1, row_at(r2)?);
        if first > last {
            return Ok(());
        }
        self.model.set_rows_hidden(i, first, last, false)?;
        for r in first..=last {
            let v = self.typed(i, r, key)?;
            let shown = self.model.get_formatted_cell_value(i, r, key)?;
            if !meets(when, &v, &shown)? {
                self.model.set_rows_hidden(i, r, r, true)?;
            }
        }
        Ok(())
    }

    /// Shows every row of a sheet again.
    pub fn clear_filter(&mut self, sheet: &str) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        let (rows, _) = self.used(i)?;
        if rows > 0 {
            self.model.set_rows_hidden(i, 1, rows, false)?;
        }
        Ok(())
    }

    /// Whether a row (1-based) is hidden.
    pub fn row_hidden(&self, sheet: &str, row: i32) -> Result<bool> {
        let i = self.sheet_index(sheet)?;
        self.model.get_model().is_row_hidden(i, row)
    }

    /// One sheet as CSV: what each cell shows, from A1 to the last used cell; text that looks like a formula is
    /// neutralised ([`csv_cell`]).
    pub fn to_csv(&self, sheet: &str) -> Result<String> {
        let i = self.sheet_index(sheet)?;
        let (rows, cols) = self.used(i)?;
        let mut w = csv::WriterBuilder::new().flexible(true).from_writer(Vec::new());
        for r in 1..=rows {
            let mut line = Vec::with_capacity(cols as usize);
            for c in 1..=cols {
                let shown = self.model.get_formatted_cell_value(i, r, c)?;
                let number = matches!(self.typed(i, r, c)?, Value::Number(_));
                line.push(csv_cell(&shown, number).into_owned());
            }
            w.write_record(&line).map_err(|e| e.to_string())?;
        }
        let bytes = w.into_inner().map_err(|e| e.to_string())?;
        String::from_utf8(bytes).map_err(|e| e.to_string())
    }

    /// The cell's input as typed: its formula, or its value.
    pub fn input(&self, sheet: &str, cell: &str) -> Result<String> {
        let i = self.sheet_index(sheet)?;
        let ((r, c), _) = parse_range(cell).ok_or_else(|| format!("{cell} is not a cell"))?;
        self.model.get_cell_content(i, row_at(r)?, col_at(c)?)
    }

    /// The cell's style, for tests and agents: IronCalc's own.
    pub fn style(&self, sheet: &str, cell: &str) -> Result<ironcalc::base::types::Style> {
        let i = self.sheet_index(sheet)?;
        let ((r, c), _) = parse_range(cell).ok_or_else(|| format!("{cell} is not a cell"))?;
        self.model.get_cell_style(i, row_at(r)?, col_at(c)?)
    }

    /// The underlying model, for what this crate doesn't wrap.
    pub fn model(&self) -> &UserModel<'static> {
        &self.model
    }

    pub fn model_mut(&mut self) -> &mut UserModel<'static> {
        &mut self.model
    }
}

/// The separator a CSV file uses: whichever of `,`, `;` and tab its first line has most of.
fn sniff_delimiter(text: &str) -> u8 {
    let first = text.lines().next().unwrap_or("");
    b",;\t".iter().copied().max_by_key(|d| first.bytes().filter(|b| b == d).count()).unwrap_or(b',')
}

/// What PDT's sheets don't keep from a `.xlsx` (IronCalc drops them on import): charts, images and drawings, pivot
/// tables, macros, for a warning on import.
pub fn dropped_parts(xlsx: &[u8]) -> Vec<String> {
    let Ok(zip) = zip::ZipArchive::new(Cursor::new(xlsx)) else { return vec![] };
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
    let has = |prefix: &str| names.iter().any(|n| n.starts_with(prefix));
    let mut out = Vec::new();
    if has("xl/charts/") {
        out.push("charts".to_string());
    }
    if has("xl/media/") || has("xl/drawings/") {
        out.push("images and drawings".to_string());
    }
    if has("xl/pivotTables/") || has("xl/pivotCache/") {
        out.push("pivot tables".to_string());
    }
    if names.iter().any(|n| n.ends_with("vbaProject.bin")) {
        out.push("macros".to_string());
    }
    if has("xl/comments") || has("xl/threadedComments/") {
        out.push("comments".to_string());
    }
    out
}

/// A colour as `#RRGGBB`.
fn hex(c: &str) -> Result<String> {
    let c = c.trim();
    let ok = c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit());
    if ok { Ok(c.to_ascii_uppercase()) } else { Err(format!("{c} is not a colour (#rrggbb)")) }
}

/// A condition's value in a formula: a number as it is, text quoted.
fn literal(v: &str) -> String {
    let v = v.trim();
    if v.parse::<f64>().is_ok() { v.to_string() } else { format!("\"{}\"", v.replace('"', "\"\"")) }
}

/// A condition as a formula for the cell `at` (the range's top-left), true where it is met.
fn condition_formula(when: &CellCondition, at: &str) -> String {
    match when {
        CellCondition::GreaterThan { value } => format!("={at}>{}", literal(value)),
        CellCondition::LessThan { value } => format!("={at}<{}", literal(value)),
        CellCondition::Between { min, max } => format!("=AND({at}>={},{at}<={})", literal(min), literal(max)),
        CellCondition::EqualTo { value } => format!("={at}={}", literal(value)),
        CellCondition::NotEqualTo { value } => format!("={at}<>{}", literal(value)),
        CellCondition::TextContains { text } => format!("=ISNUMBER(SEARCH({},{at}))", literal(text)),
        CellCondition::Empty => format!("=LEN({at})=0"),
        CellCondition::NotEmpty => format!("=LEN({at})>0"),
        CellCondition::Formula { formula } => {
            if formula.trim_start().starts_with('=') {
                formula.trim().to_string()
            } else {
                format!("={}", formula.trim())
            }
        }
    }
}

/// Whether a cell (its typed value and what it shows) meets a filter's condition. Numbers compare as numbers; text
/// without regard to case.
fn meets(when: &CellCondition, v: &Value, shown: &str) -> Result<bool> {
    let num = |s: &str| s.trim().parse::<f64>().ok();
    let cmp = |other: &str| -> Option<std::cmp::Ordering> {
        match (v, num(other)) {
            (Value::Number(x), Some(y)) => x.partial_cmp(&y),
            _ => Some(shown.trim().to_lowercase().cmp(&other.trim().to_lowercase())),
        }
    };
    use std::cmp::Ordering::*;
    Ok(match when {
        CellCondition::GreaterThan { value } => cmp(value) == Some(Greater),
        CellCondition::LessThan { value } => cmp(value) == Some(Less),
        CellCondition::Between { min, max } => cmp(min) != Some(Less) && cmp(max) != Some(Greater),
        CellCondition::EqualTo { value } => cmp(value) == Some(Equal),
        CellCondition::NotEqualTo { value } => cmp(value) != Some(Equal),
        CellCondition::TextContains { text } => shown.to_lowercase().contains(&text.trim().to_lowercase()),
        CellCondition::Empty => matches!(v, Value::Empty),
        CellCondition::NotEmpty => !matches!(v, Value::Empty),
        CellCondition::Formula { .. } => return Err("a filter takes a value condition, not a formula".into()),
    })
}

#[cfg(test)]
mod tests;
