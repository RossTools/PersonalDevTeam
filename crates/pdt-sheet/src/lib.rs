//! PDT's local sheets (PDT-315) on IronCalc, the engine the browser's editor runs too (`@ironcalc/workbook`, the same
//! version: the two exchange IronCalc's bytes and diffs, see [`IRONCALC_VERSION`]). A sheet's file of record is a
//! `.xlsx`; this crate loads and saves it, reads cells with their computed values, applies PDT's edits (values,
//! formats, sheets, sort and filter), imports `.csv`, exports `.csv`, and passes IronCalc's sync diffs in and out
//! ([`Workbook::flush`], [`Workbook::apply_diffs`]).
//!
//! Formulas are recalculated on load (a file's cached values are not trusted) and after every edit.

use ironcalc::base::UserModel;
use ironcalc::base::cell::CellValue;
use ironcalc::base::expressions::types::Area;
use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use pdt_core::document::{Cell, Sheet, column_letters, parse_column, parse_range};
use std::io::Cursor;
use std::path::Path;

/// The IronCalc version in pdtd; the web UI's `@ironcalc/workbook` must be the same (a test checks).
pub const IRONCALC_VERSION: &str = "0.9.1";

const LOCALE: &str = "en";
const TZ: &str = "UTC";
const LANGUAGE: &str = "en";

/// The most rows and columns [`Workbook::sheets`] reads (what agents and the plain view see).
pub const READ_LIMIT: (usize, usize) = (2000, 200);

pub type Result<T> = std::result::Result<T, String>;

/// A zero-based (row, column).
type Place = (usize, usize);

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

    /// A workbook from a `.xlsx` file's bytes, recalculated.
    pub fn from_xlsx(bytes: &[u8], name: &str) -> Result<Self> {
        let wb = load_from_xlsx_bytes(bytes, name, LOCALE, TZ)
            .map_err(|e| format!("not a spreadsheet PDT can read: {e}"))?;
        let model = ironcalc::base::Model::from_workbook(wb, LANGUAGE)?;
        let mut model = UserModel::from_model(model);
        model.evaluate();
        Ok(Self { model })
    }

    /// A workbook from a `.csv` file (comma, semicolon or tab separated), one sheet named `sheet`. Cells are typed as
    /// if entered by hand: numbers stay numbers, and a cell starting with `=` is a formula.
    pub fn from_csv(text: &str, name: &str, sheet: &str) -> Result<Self> {
        let delimiter = sniff_delimiter(text);
        let mut wb = Self::new(name)?;
        if !sheet.trim().is_empty() && sheet != "Sheet1" {
            wb.model.rename_sheet(0, sheet.trim())?;
        }
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .delimiter(delimiter)
            .from_reader(text.trim_start_matches('\u{feff}').as_bytes());
        wb.model.pause_evaluation();
        for (r, record) in reader.records().enumerate() {
            let record = record.map_err(|e| format!("not a CSV file PDT can read: {e}"))?;
            if r >= 1_048_576 {
                break;
            }
            for (c, v) in record.iter().enumerate() {
                if !v.is_empty() {
                    wb.model.set_user_input(0, r as i32 + 1, c as i32 + 1, v)?;
                }
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
            out.push(Sheet { name, rows: grid });
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
                line.push(match m.get_cell_value_by_index(i, r as i32 + 1, c as i32 + 1)? {
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
                line.push(self.model.get_formatted_cell_value(i, r as i32 + 1, c as i32 + 1)?);
            }
            out.push(line);
        }
        Ok(out)
    }

    /// Writes values into a sheet from `cell` across and down; a value starting with `=` is a formula.
    pub fn set_cells(&mut self, sheet: &str, cell: &str, values: &[Vec<String>]) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        let ((row, col), _) = parse_range(cell).ok_or_else(|| format!("{cell} is not a cell"))?;
        self.model.pause_evaluation();
        let r = (|| {
            for (dr, line) in values.iter().enumerate() {
                for (dc, v) in line.iter().enumerate() {
                    let (r, c) = ((row + dr) as i32 + 1, (col + dc) as i32 + 1);
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
        Ok(Area {
            sheet: i,
            row: r1 as i32 + 1,
            column: c1 as i32 + 1,
            width: (c2 - c1) as i32 + 1,
            height: (r2 - r1) as i32 + 1,
        })
    }

    /// A range to sort or filter: as given, or for none the sheet's used rows below its first (the header).
    fn table(&self, sheet: u32, range: &str) -> Result<String> {
        if !range.trim().is_empty() {
            return Ok(range.to_string());
        }
        let (rows, cols) = self.used(sheet)?;
        if rows < 2 {
            return Err("there are no rows below the first to sort or filter".into());
        }
        Ok(format!("A2:{}", cell_name(rows as usize - 1, cols as usize - 1)))
    }

    /// The zero-based column `column` names, checked to be inside the range.
    fn key_column(range: &str, column: &str) -> Result<(usize, Place, Place)> {
        let ((r1, c1), (r2, c2)) = parse_range(range).ok_or_else(|| format!("{range} is not a range"))?;
        let key = parse_column(column).ok_or_else(|| format!("{column} is not a column"))?;
        if key < c1 || key > c2 {
            return Err(format!("column {column} is not in {range}"));
        }
        Ok((key, (r1, c1), (r2, c2)))
    }

    /// Sorts the range's rows (none: the used rows below the first) by one of its columns: numbers before text before empty cells, text without regard to
    /// case. As spreadsheets do, the range's cells are rewritten in their new order, a formula's references shifting
    /// with it as if copied (`=B2-C2` moved to row 4 is `=B4-C4`), and formats move with their cells; formulas
    /// elsewhere keep pointing at the same cells. Rows hidden by a filter are shown first.
    pub fn sort(&mut self, sheet: &str, range: &str, column: &str, descending: bool) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        let range = self.table(i, range)?;
        let (key, (r1, c1), (r2, c2)) = Self::key_column(&range, column)?;
        let (first, last) = (r1 as i32 + 1, r2 as i32 + 1);
        let (left, right) = (c1 as i32 + 1, c2 as i32 + 1);
        self.model.set_rows_hidden(i, first, last, false)?;
        let m = self.model.get_model();
        let mut keys: Vec<(usize, Value)> = Vec::new();
        for (n, r) in (first..=last).enumerate() {
            let v = match m.get_cell_value_by_index(i, r, key as i32 + 1)? {
                CellValue::None => Value::Empty,
                CellValue::Number(x) => Value::Number(x),
                CellValue::String(s) => Value::Text(s.to_lowercase()),
                CellValue::Boolean(b) => Value::Bool(b),
            };
            keys.push((n, v));
        }
        let rank = |v: &Value| match v {
            Value::Number(_) => 0,
            Value::Text(_) => 1,
            Value::Bool(_) => 2,
            Value::Empty => 3,
        };
        keys.sort_by(|(_, a), (_, b)| {
            let o = rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
                (Value::Number(x), Value::Number(y)) => x.total_cmp(y),
                (Value::Text(x), Value::Text(y)) => x.cmp(y),
                (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
                _ => std::cmp::Ordering::Equal,
            });
            // Empty cells stay at the end either way.
            if descending && rank(a) != 3 && rank(b) != 3 { o.reverse() } else { o }
        });
        if keys.iter().enumerate().all(|(target, (orig, _))| target == *orig) {
            return Ok(());
        }
        // Every cell's input as it reads at its new row, and its format.
        let mut inputs = Vec::with_capacity(keys.len());
        let mut styles = Vec::with_capacity(keys.len());
        for (target, (orig, _)) in keys.iter().enumerate() {
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

    /// Hides the range's rows whose `column` cell shows none of `values` (case-insensitive); no values shows every row
    /// of the range again.
    pub fn filter(&mut self, sheet: &str, range: &str, column: &str, values: &[String]) -> Result<()> {
        let i = self.sheet_index(sheet)?;
        let range = self.table(i, range)?;
        let (key, (r1, _), (r2, _)) = Self::key_column(&range, column)?;
        let (first, last) = (r1 as i32 + 1, r2 as i32 + 1);
        self.model.set_rows_hidden(i, first, last, false)?;
        if values.is_empty() {
            return Ok(());
        }
        let wanted: Vec<String> = values.iter().map(|v| v.trim().to_lowercase()).collect();
        for r in first..=last {
            let shown = self.model.get_formatted_cell_value(i, r, key as i32 + 1)?.trim().to_lowercase();
            if !wanted.contains(&shown) {
                self.model.set_rows_hidden(i, r, r, true)?;
            }
        }
        Ok(())
    }

    /// Whether a row (1-based) is hidden.
    pub fn row_hidden(&self, sheet: &str, row: i32) -> Result<bool> {
        let i = self.sheet_index(sheet)?;
        self.model.get_model().is_row_hidden(i, row)
    }

    /// One sheet as CSV: what each cell shows, from A1 to the last used cell.
    pub fn to_csv(&self, sheet: &str) -> Result<String> {
        let i = self.sheet_index(sheet)?;
        let (rows, cols) = self.used(i)?;
        let mut w = csv::WriterBuilder::new().flexible(true).from_writer(Vec::new());
        for r in 1..=rows {
            let mut line = Vec::with_capacity(cols as usize);
            for c in 1..=cols {
                line.push(self.model.get_formatted_cell_value(i, r, c)?);
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
        self.model.get_cell_content(i, r as i32 + 1, c as i32 + 1)
    }

    /// The cell's style, for tests and agents: IronCalc's own.
    pub fn style(&self, sheet: &str, cell: &str) -> Result<ironcalc::base::types::Style> {
        let i = self.sheet_index(sheet)?;
        let ((r, c), _) = parse_range(cell).ok_or_else(|| format!("{cell} is not a cell"))?;
        self.model.get_cell_style(i, r as i32 + 1, c as i32 + 1)
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

/// The cell name of a zero-based (row, column): `B3`.
pub fn cell_name(row: usize, col: usize) -> String {
    format!("{}{}", column_letters(col), row + 1)
}

#[cfg(test)]
mod tests;
