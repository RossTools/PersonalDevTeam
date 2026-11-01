//! Google Sheets (PDT-316): a spreadsheet read as PDT's sheets of cells (what each shows, and its formula) and charts,
//! and PDT's sheet edits as Sheets API requests: values and formulas (`values.batchUpdate`, as if typed), sheets,
//! formats, borders, widths, frozen rows, conditional formats, validation, charts, sorting and filters
//! (`batchUpdate`). Sheets are named in edits; Google wants their ids, from the spreadsheet's properties.
//!
//! A chart's PDT id (`chart-…`, given when it is added) becomes Google's numeric chart id by a hash, and is kept in the
//! spreadsheet's developer metadata (`pdt.chart`: `<chart id> <PDT id>`), so reading the spreadsheet gives the charts
//! back under the ids PDT gave them. A chart made in Google's editor has its Google id. Pure.

use pdt_api::sdk::documents::{
    BorderStyle, Cell, CellCondition, CellFormat, CellValidation, Chart, ChartKind, ChartSpec, DocumentEdit, HAlign,
    ProviderError, Sheet, VAlign, cell_name, parse_cell, parse_column, parse_range,
};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// The developer metadata key that keeps PDT's chart ids.
pub const CHART_KEY: &str = "pdt.chart";

const CHART_FIELDS: &str = "charts(chartId,spec(title,basicChart(chartType,domains(domain(sourceRange(sources))),\
                            series(series(sourceRange(sources)))),pieChart(domain(sourceRange(sources)),\
                            series(sourceRange(sources)))),position(overlayPosition(anchorCell)))";

/// The fields of a read: each sheet's name, cells (shown values and formulas) and charts, and PDT's chart ids.
pub fn read_fields() -> String {
    format!(
        "sheets(properties(sheetId,title),data(rowData(values(formattedValue,userEnteredValue))),{CHART_FIELDS}),\
         developerMetadata(metadataKey,metadataValue)"
    )
}

/// The fields an edit needs: the sheets' ids, their charts' ids and PDT's chart ids.
pub const META_FIELDS: &str =
    "sheets(properties(sheetId,title),charts(chartId)),developerMetadata(metadataKey,metadataValue)";

/// PDT's chart ids, by Google's.
fn chart_names(v: &Value) -> HashMap<i64, String> {
    v["developerMetadata"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["metadataKey"] == CHART_KEY)
        .filter_map(|m| {
            let (n, id) = m["metadataValue"].as_str()?.split_once(' ')?;
            Some((n.parse().ok()?, id.to_string()))
        })
        .collect()
}

/// Google's chart id for a PDT one: its number when it is one, else a hash of it (positive, 31 bits).
pub fn chart_number(id: &str) -> i64 {
    if let Ok(n) = id.trim().parse::<i64>() {
        return n;
    }
    // FNV-1a.
    let h = id.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    i64::from(h & 0x7fff_ffff).max(1)
}

/// A grid range as A1 (`B2:C10`), when it has all its ends.
fn a1(g: &Value) -> Option<(usize, usize, usize, usize)> {
    let n = |k: &str| g[k].as_u64().map(|x| x as usize);
    Some((n("startRowIndex")?, n("startColumnIndex")?, n("endRowIndex")?, n("endColumnIndex")?))
}

fn source(v: &Value) -> Option<(usize, usize, usize, usize)> {
    a1(&v["sourceRange"]["sources"][0])
}

/// A chart as Google reads it, as PDT's spec: the range from its categories to its last series.
fn chart_of(c: &Value, names: &HashMap<i64, String>) -> Option<Chart> {
    let n = c["chartId"].as_i64()?;
    let spec = &c["spec"];
    let (kind, mut ranges) = if spec["pieChart"].is_object() {
        let p = &spec["pieChart"];
        (ChartKind::Pie, vec![source(&p["domain"])?, source(&p["series"])?])
    } else {
        let b = &spec["basicChart"];
        let kind = match b["chartType"].as_str()? {
            "LINE" => ChartKind::Line,
            "BAR" => ChartKind::Bar,
            "AREA" => ChartKind::Area,
            "SCATTER" => ChartKind::Scatter,
            _ => ChartKind::Column,
        };
        let mut r = vec![source(&b["domains"][0]["domain"])?];
        r.extend(b["series"].as_array().into_iter().flatten().filter_map(|s| source(&s["series"])));
        (kind, r)
    };
    ranges.sort();
    let (r0, c0) = (ranges.iter().map(|r| r.0).min()?, ranges.iter().map(|r| r.1).min()?);
    let (r1, c1) = (ranges.iter().map(|r| r.2).max()?, ranges.iter().map(|r| r.3).max()?);
    let anchor = &c["position"]["overlayPosition"]["anchorCell"];
    let anchor = anchor["rowIndex"].as_u64().zip(anchor["columnIndex"].as_u64());
    Some(Chart {
        id: names.get(&n).cloned().unwrap_or_else(|| n.to_string()),
        spec: ChartSpec {
            kind,
            data: format!("{}:{}", cell_name(r0, c0), cell_name(r1.saturating_sub(1), c1.saturating_sub(1))),
            title: spec["title"].as_str().filter(|t| !t.is_empty()).map(str::to_string),
            anchor: anchor.map(|(r, c)| cell_name(r as usize, c as usize)),
        },
    })
}

/// A spreadsheet's sheets, as `spreadsheets.get` with [`read_fields`] gave them.
pub fn read(v: &Value) -> Vec<Sheet> {
    let names = chart_names(v);
    let mut out = vec![];
    for s in v["sheets"].as_array().into_iter().flatten() {
        let name = s["properties"]["title"].as_str().unwrap_or_default().to_string();
        let mut rows = vec![];
        for d in s["data"].as_array().into_iter().flatten() {
            for row in d["rowData"].as_array().into_iter().flatten() {
                let cells: Vec<Cell> = row["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| Cell {
                        value: c["formattedValue"].as_str().unwrap_or_default().to_string(),
                        formula: c["userEnteredValue"]["formulaValue"].as_str().map(str::to_string),
                    })
                    .collect();
                rows.push(cells);
            }
        }
        // Google leaves out trailing empty rows; empty ones in between stay, as empty rows.
        while rows.last().is_some_and(|r: &Vec<Cell>| r.iter().all(|c| c.value.is_empty() && c.formula.is_none())) {
            rows.pop();
        }
        let charts = s["charts"].as_array().into_iter().flatten().filter_map(|c| chart_of(c, &names)).collect();
        out.push(Sheet { name, rows, charts });
    }
    out
}

/// The sheets' ids by name, and the charts: Google's id of each PDT id.
#[derive(Debug, Clone, Default)]
pub struct Ids {
    sheets: Vec<(String, i64)>,
    charts: HashMap<String, i64>,
    /// The sheet each chart is on, by Google's id.
    chart_sheets: HashMap<i64, i64>,
}

impl Ids {
    pub fn of(meta: &Value) -> Self {
        let mut ids = Ids::default();
        for (n, id) in chart_names(meta) {
            ids.charts.insert(id, n);
        }
        for s in meta["sheets"].as_array().into_iter().flatten() {
            let p = &s["properties"];
            let (Some(title), Some(id)) = (p["title"].as_str(), p["sheetId"].as_i64()) else { continue };
            ids.sheets.push((title.to_string(), id));
            for c in s["charts"].as_array().into_iter().flatten() {
                if let Some(n) = c["chartId"].as_i64() {
                    ids.chart_sheets.insert(n, id);
                }
            }
        }
        ids
    }
    fn id(&self, name: &str) -> Result<i64, ProviderError> {
        let name = name.trim();
        self.sheets
            .iter()
            .find(|(n, _)| n == name)
            .or_else(|| self.sheets.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)))
            .map(|(_, id)| *id)
            .ok_or_else(|| ProviderError::Invalid(format!("the spreadsheet has no sheet called {name}")))
    }
    /// The sheet's name as the spreadsheet has it.
    fn name(&self, name: &str) -> Result<String, ProviderError> {
        let id = self.id(name)?;
        Ok(self.sheets.iter().find(|(_, i)| *i == id).map(|(n, _)| n.clone()).unwrap_or_default())
    }
    /// Google's id of a chart PDT names, and its sheet.
    fn chart(&self, id: &str) -> Result<(i64, i64), ProviderError> {
        let n = self.charts.get(id).copied().unwrap_or_else(|| chart_number(id));
        let sheet = self.chart_sheets.get(&n).copied();
        sheet.map(|s| (n, s)).ok_or_else(|| ProviderError::Invalid(format!("there is no chart {id}")))
    }
}

/// One step of an edit: requests for `batchUpdate`, or values to write.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Requests(Vec<Value>),
    /// `values.batchUpdate`'s `data`: ranges with their values.
    Values(Vec<Value>),
}

fn invalid(m: impl Into<String>) -> ProviderError {
    ProviderError::Invalid(m.into())
}

/// A cell range (`A1:C10`, or a cell) on a sheet as Google's `GridRange`.
pub fn grid(sheet_id: i64, range: &str) -> Result<Value, ProviderError> {
    let ((r0, c0), (r1, c1)) = parse_range(range).ok_or_else(|| invalid(format!("{range} is not a range (A1:C10)")))?;
    Ok(json!({"sheetId": sheet_id, "startRowIndex": r0, "endRowIndex": r1 + 1,
              "startColumnIndex": c0, "endColumnIndex": c1 + 1}))
}

/// The steps of `edits`, in order: consecutive requests go in one `batchUpdate`, consecutive values in one write.
/// `new_id` gives a new sheet its id, so later edits in the same batch can name it.
pub fn plan(
    ids: &mut Ids,
    edits: &[DocumentEdit],
    mut new_id: impl FnMut() -> i64,
) -> Result<Vec<Step>, ProviderError> {
    let mut steps: Vec<Step> = vec![];
    let push = |steps: &mut Vec<Step>, s: Step| match (steps.last_mut(), s) {
        (Some(Step::Requests(a)), Step::Requests(b)) => a.extend(b),
        (Some(Step::Values(a)), Step::Values(b)) => a.extend(b),
        (_, s) => steps.push(s),
    };
    for e in edits {
        let req = |r: Value| Step::Requests(vec![r]);
        let step = match e {
            DocumentEdit::SetCells { sheet, cell, values } => {
                let (row, col) = parse_cell(cell).ok_or_else(|| invalid(format!("{cell} is not a cell")))?;
                let name = ids.name(sheet)?;
                let width = values.iter().map(Vec::len).max().unwrap_or(1).max(1);
                let end = cell_name(row + values.len().max(1) - 1, col + width - 1);
                let range = format!("'{}'!{}:{end}", name.replace('\'', "''"), cell_name(row, col));
                Step::Values(vec![json!({"range": range, "majorDimension": "ROWS", "values": values})])
            }
            DocumentEdit::AddSheet { name } => {
                let name = name.trim();
                if name.is_empty() {
                    return Err(invalid("a sheet needs a name"));
                }
                if ids.sheets.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
                    return Err(invalid(format!("there is a sheet called {name} already")));
                }
                let id = new_id();
                ids.sheets.push((name.to_string(), id));
                req(json!({"addSheet": {"properties": {"title": name, "sheetId": id}}}))
            }
            DocumentEdit::RenameSheet { sheet, name } => {
                let id = ids.id(sheet)?;
                for (n, i) in ids.sheets.iter_mut() {
                    if *i == id {
                        *n = name.trim().to_string();
                    }
                }
                req(
                    json!({"updateSheetProperties": {"properties": {"sheetId": id, "title": name.trim()}, "fields": "title"}}),
                )
            }
            DocumentEdit::DeleteSheet { sheet } => {
                let id = ids.id(sheet)?;
                ids.sheets.retain(|(_, i)| *i != id);
                req(json!({"deleteSheet": {"sheetId": id}}))
            }
            DocumentEdit::FormatRange { sheet, range, format } => {
                let (fmt, fields) = cell_format(format)?;
                if fields.is_empty() {
                    return Err(invalid("the format sets nothing"));
                }
                let fields = fields.iter().map(|f| format!("userEnteredFormat.{f}")).collect::<Vec<_>>().join(",");
                req(json!({"repeatCell": {"range": grid(ids.id(sheet)?, range)?,
                                          "cell": {"userEnteredFormat": fmt}, "fields": fields}}))
            }
            DocumentEdit::ConditionalFormat { sheet, range, when, format } => {
                let (fmt, fields) = cell_format(format)?;
                if fields.is_empty() {
                    return Err(invalid("the format sets nothing"));
                }
                let first =
                    parse_range(range).map(|(a, _)| a).ok_or_else(|| invalid(format!("{range} is not a range")))?;
                req(json!({"addConditionalFormatRule": {"index": 0, "rule": {
                    "ranges": [grid(ids.id(sheet)?, range)?],
                    "booleanRule": {"condition": condition(when, first)?, "format": fmt}}}}))
            }
            DocumentEdit::SetBorders { sheet, range, style, color, inner } => {
                let line = json!({"style": border_style(*style),
                                  "color": color_of(color.as_deref().unwrap_or("#000000"))?});
                let mut b = Map::new();
                b.insert("range".into(), grid(ids.id(sheet)?, range)?);
                for side in ["top", "bottom", "left", "right"] {
                    b.insert(side.into(), line.clone());
                }
                if *inner {
                    b.insert("innerHorizontal".into(), line.clone());
                    b.insert("innerVertical".into(), line);
                }
                req(json!({"updateBorders": b}))
            }
            DocumentEdit::ColumnWidth { sheet, columns, width } => {
                let (a, b) = columns.split_once(':').unwrap_or((columns, columns));
                let (Some(a), Some(b)) = (parse_column(a), parse_column(b)) else {
                    return Err(invalid(format!("{columns} are not columns (B, or B:D)")));
                };
                req(json!({"updateDimensionProperties": {
                    "range": {"sheetId": ids.id(sheet)?, "dimension": "COLUMNS",
                              "startIndex": a.min(b), "endIndex": a.max(b) + 1},
                    "properties": {"pixelSize": width}, "fields": "pixelSize"}}))
            }
            DocumentEdit::Freeze { sheet, rows, columns } => req(json!({"updateSheetProperties": {
                "properties": {"sheetId": ids.id(sheet)?,
                               "gridProperties": {"frozenRowCount": rows, "frozenColumnCount": columns}},
                "fields": "gridProperties.frozenRowCount,gridProperties.frozenColumnCount"}})),
            DocumentEdit::Validate { sheet, range, rule, strict } => {
                let first =
                    parse_range(range).map(|(a, _)| a).ok_or_else(|| invalid(format!("{range} is not a range")))?;
                let (condition, list) = match rule {
                    CellValidation::OneOf { values } if values.is_empty() => {
                        return Err(invalid("one_of needs its values"));
                    }
                    CellValidation::OneOf { values } => (
                        json!({"type": "ONE_OF_LIST",
                               "values": values.iter().map(|v| json!({"userEnteredValue": v})).collect::<Vec<_>>()}),
                        true,
                    ),
                    CellValidation::Checkbox => (json!({"type": "BOOLEAN"}), false),
                    CellValidation::Date => (json!({"type": "DATE_IS_VALID"}), false),
                    CellValidation::Matches { when } => (condition(when, first)?, false),
                };
                req(json!({"setDataValidation": {"range": grid(ids.id(sheet)?, range)?,
                    "rule": {"condition": condition, "strict": strict, "showCustomUi": list}}}))
            }
            DocumentEdit::AddChart { sheet, id, chart } => {
                let sheet_id = ids.id(sheet)?;
                let id =
                    id.clone().filter(|i| !i.trim().is_empty()).ok_or_else(|| invalid("a new chart needs an id"))?;
                let n = chart_number(&id);
                if ids.chart_sheets.contains_key(&n) {
                    return Err(invalid(format!("there is a chart {id} already")));
                }
                ids.charts.insert(id.clone(), n);
                ids.chart_sheets.insert(n, sheet_id);
                let position = position(sheet_id, chart)?;
                Step::Requests(vec![
                    json!({"addChart": {"chart": {"chartId": n, "spec": chart_spec(sheet_id, chart)?, "position": position}}}),
                    json!({"createDeveloperMetadata": {"developerMetadata": {
                        "metadataKey": CHART_KEY, "metadataValue": format!("{n} {id}"),
                        "location": {"spreadsheet": true}, "visibility": "DOCUMENT"}}}),
                ])
            }
            DocumentEdit::UpdateChart { id, chart } => {
                let (n, sheet_id) = ids.chart(id)?;
                req(json!({"updateChartSpec": {"chartId": n, "spec": chart_spec(sheet_id, chart)?}}))
            }
            DocumentEdit::DeleteChart { id } => {
                let (n, _) = ids.chart(id)?;
                ids.chart_sheets.remove(&n);
                ids.charts.remove(id);
                Step::Requests(vec![
                    json!({"deleteEmbeddedObject": {"objectId": n}}),
                    json!({"deleteDeveloperMetadata": {"dataFilter": {"developerMetadataLookup": {
                        "metadataKey": CHART_KEY, "metadataValue": format!("{n} {id}")}}}}),
                ])
            }
            DocumentEdit::Sort { sheet, range, by, header } => {
                if by.is_empty() {
                    return Err(invalid("sort needs at least one column to sort by"));
                }
                let ((r0, c0), (r1, c1)) =
                    parse_range(range).ok_or_else(|| invalid(format!("{range} is not a range")))?;
                let first = if *header { r0 + 1 } else { r0 };
                if first > r1 {
                    return Err(invalid("the range has no rows to sort"));
                }
                let specs = by
                    .iter()
                    .map(|k| {
                        let c = parse_column(&k.column)
                            .filter(|c| (c0..=c1).contains(c))
                            .ok_or_else(|| invalid(format!("column {} isn't in the range", k.column.trim())))?;
                        Ok(json!({"dimensionIndex": c, "sortOrder": if k.descending { "DESCENDING" } else { "ASCENDING" }}))
                    })
                    .collect::<Result<Vec<_>, ProviderError>>()?;
                let range = format!("{}:{}", cell_name(first, c0), cell_name(r1, c1));
                req(json!({"sortRange": {"range": grid(ids.id(sheet)?, &range)?, "sortSpecs": specs}}))
            }
            DocumentEdit::Filter { sheet, range, column, when } => {
                let ((r0, c0), (_, c1)) =
                    parse_range(range).ok_or_else(|| invalid(format!("{range} is not a range")))?;
                let c = parse_column(column)
                    .filter(|c| (c0..=c1).contains(c))
                    .ok_or_else(|| invalid(format!("column {} isn't in the range", column.trim())))?;
                // Its first row is the header: a formula is written for the first row below it.
                req(json!({"setBasicFilter": {"filter": {"range": grid(ids.id(sheet)?, range)?, "filterSpecs": [
                    {"columnIndex": c, "filterCriteria": {"condition": condition(when, (r0 + 1, c))?}}]}}}))
            }
            DocumentEdit::ClearFilter { sheet } => req(json!({"clearBasicFilter": {"sheetId": ids.id(sheet)?}})),
            e => {
                return Err(ProviderError::Unsupported(format!(
                    "a Google Sheet has no text to change ({} is for text documents)",
                    e.op()
                )));
            }
        };
        push(&mut steps, step);
    }
    Ok(steps)
}

/// `#rrggbb` as Google's colour.
pub fn color_of(hex: &str) -> Result<Value, ProviderError> {
    let h = hex.trim().trim_start_matches('#');
    let bad = || invalid(format!("{hex} is not a colour (#rrggbb)"));
    if h.len() != 6 || !h.is_ascii() {
        return Err(bad());
    }
    let part = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map(|b| f64::from(b) / 255.0).map_err(|_| bad());
    Ok(json!({"red": part(0)?, "green": part(2)?, "blue": part(4)?}))
}

/// A number format's kind from its pattern: a date or time, a percentage, else a number.
fn number_kind(pattern: &str) -> &'static str {
    let p = pattern.to_ascii_lowercase();
    if p.contains('%') {
        "PERCENT"
    } else if p.contains('#') || p.contains('0') {
        "NUMBER"
    } else if p.contains('y') || p.contains('d') {
        if p.contains('h') { "DATE_TIME" } else { "DATE" }
    } else if p.contains('h') || p.contains('s') {
        "TIME"
    } else {
        "NUMBER"
    }
}

/// A format as Google's `CellFormat`, and the fields it sets (under `userEnteredFormat`).
fn cell_format(f: &CellFormat) -> Result<(Value, Vec<&'static str>), ProviderError> {
    let mut out = Map::new();
    let mut text = Map::new();
    let mut fields = vec![];
    for (key, field, v) in [
        ("bold", "textFormat.bold", f.bold),
        ("italic", "textFormat.italic", f.italic),
        ("underline", "textFormat.underline", f.underline),
    ] {
        if let Some(v) = v {
            text.insert(key.into(), json!(v));
            fields.push(field);
        }
    }
    if let Some(s) = f.size {
        text.insert("fontSize".into(), json!(s));
        fields.push("textFormat.fontSize");
    }
    if let Some(c) = &f.color {
        text.insert("foregroundColor".into(), color_of(c)?);
        fields.push("textFormat.foregroundColor");
    }
    if !text.is_empty() {
        out.insert("textFormat".into(), Value::Object(text));
    }
    if let Some(c) = &f.background {
        out.insert("backgroundColor".into(), color_of(c)?);
        fields.push("backgroundColor");
    }
    if let Some(a) = f.horizontal {
        let a = match a {
            HAlign::Left => "LEFT",
            HAlign::Center => "CENTER",
            HAlign::Right => "RIGHT",
        };
        out.insert("horizontalAlignment".into(), json!(a));
        fields.push("horizontalAlignment");
    }
    if let Some(a) = f.vertical {
        let a = match a {
            VAlign::Top => "TOP",
            VAlign::Middle => "MIDDLE",
            VAlign::Bottom => "BOTTOM",
        };
        out.insert("verticalAlignment".into(), json!(a));
        fields.push("verticalAlignment");
    }
    if let Some(w) = f.wrap {
        out.insert("wrapStrategy".into(), json!(if w { "WRAP" } else { "OVERFLOW_CELL" }));
        fields.push("wrapStrategy");
    }
    if let Some(p) = f.number_format.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        out.insert("numberFormat".into(), json!({"type": number_kind(p), "pattern": p}));
        fields.push("numberFormat");
    }
    Ok((Value::Object(out), fields))
}

fn border_style(s: BorderStyle) -> &'static str {
    match s {
        BorderStyle::Solid => "SOLID",
        BorderStyle::Thick => "SOLID_THICK",
        BorderStyle::Dashed => "DASHED",
        BorderStyle::Dotted => "DOTTED",
        BorderStyle::Double => "DOUBLE",
        BorderStyle::None => "NONE",
    }
}

/// A Google formula string literal.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// A condition as Google's `BooleanCondition`. Numbers compare as numbers, other values as text; `at` is the first
/// cell it is tested on, for the formulas some need.
fn condition(c: &CellCondition, at: (usize, usize)) -> Result<Value, ProviderError> {
    let one = |kind: &str, v: &str| json!({"type": kind, "values": [{"userEnteredValue": v}]});
    let number = |v: &str| v.trim().parse::<f64>().is_ok();
    let cell = cell_name(at.0, at.1);
    Ok(match c {
        CellCondition::GreaterThan { value } => one("NUMBER_GREATER", value),
        CellCondition::LessThan { value } => one("NUMBER_LESS", value),
        CellCondition::Between { min, max } => json!({"type": "NUMBER_BETWEEN",
            "values": [{"userEnteredValue": min}, {"userEnteredValue": max}]}),
        CellCondition::EqualTo { value } if number(value) => one("NUMBER_EQ", value),
        CellCondition::EqualTo { value } => one("TEXT_EQ", value),
        CellCondition::NotEqualTo { value } if number(value) => one("NUMBER_NOT_EQ", value),
        CellCondition::NotEqualTo { value } => one("CUSTOM_FORMULA", &format!("={cell}<>{}", quote(value))),
        CellCondition::TextContains { text } => one("TEXT_CONTAINS", text),
        CellCondition::Empty => json!({"type": "BLANK"}),
        CellCondition::NotEmpty => json!({"type": "NOT_BLANK"}),
        CellCondition::Formula { formula } => {
            let f = formula.trim();
            if f.is_empty() {
                return Err(invalid("the condition's formula is empty"));
            }
            one("CUSTOM_FORMULA", &if f.starts_with('=') { f.to_string() } else { format!("={f}") })
        }
    })
}

/// A chart's spec: the data's first column along the axis (a pie's labels), each other column a series, named by its
/// first row.
fn chart_spec(sheet_id: i64, c: &ChartSpec) -> Result<Value, ProviderError> {
    let ((r0, c0), (r1, c1)) =
        parse_range(&c.data).ok_or_else(|| invalid(format!("{} is not a range (A1:C10)", c.data)))?;
    if c1 == c0 {
        return Err(invalid("a chart's data needs a column of categories and at least one of values"));
    }
    let col = |c: usize| -> Value {
        json!({"sourceRange": {"sources": [{"sheetId": sheet_id, "startRowIndex": r0, "endRowIndex": r1 + 1,
                                            "startColumnIndex": c, "endColumnIndex": c + 1}]}})
    };
    let mut spec = Map::new();
    if let Some(t) = &c.title {
        spec.insert("title".into(), json!(t));
    }
    if c.kind == ChartKind::Pie {
        spec.insert(
            "pieChart".into(),
            json!({"legendPosition": "RIGHT_LEGEND", "domain": col(c0), "series": col(c0 + 1)}),
        );
    } else {
        let kind = match c.kind {
            ChartKind::Line => "LINE",
            ChartKind::Bar => "BAR",
            ChartKind::Area => "AREA",
            ChartKind::Scatter => "SCATTER",
            _ => "COLUMN",
        };
        // A bar chart's values run along the bottom axis; the rest's up the left.
        let axis = if c.kind == ChartKind::Bar { "BOTTOM_AXIS" } else { "LEFT_AXIS" };
        let series: Vec<Value> = (c0 + 1..=c1).map(|i| json!({"series": col(i), "targetAxis": axis})).collect();
        spec.insert(
            "basicChart".into(),
            json!({"chartType": kind, "legendPosition": "BOTTOM_LEGEND", "headerCount": 1,
                   "domains": [{"domain": col(c0)}], "series": series}),
        );
    }
    Ok(Value::Object(spec))
}

/// Where a new chart sits: on its anchor, else beside its data.
fn position(sheet_id: i64, c: &ChartSpec) -> Result<Value, ProviderError> {
    let (row, col) = match &c.anchor {
        Some(a) => parse_cell(a).ok_or_else(|| invalid(format!("{a} is not a cell")))?,
        None => {
            let ((r0, _), (_, c1)) =
                parse_range(&c.data).ok_or_else(|| invalid(format!("{} is not a range", c.data)))?;
            (r0, c1 + 2)
        }
    };
    Ok(json!({"overlayPosition": {"anchorCell": {"sheetId": sheet_id, "rowIndex": row, "columnIndex": col}}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_api::sdk::documents::SortKey;

    fn ids() -> Ids {
        Ids::of(&json!({"sheets": [{"properties": {"sheetId": 0, "title": "Data"}, "charts": [{"chartId": 99}]},
                                   {"properties": {"sheetId": 7, "title": "Q'4"}}],
                        "developerMetadata": [{"metadataKey": CHART_KEY, "metadataValue": "99 chart-old"}]}))
    }

    fn requests(steps: &[Step]) -> Vec<Value> {
        steps.iter().filter_map(|s| if let Step::Requests(r) = s { Some(r.clone()) } else { None }).flatten().collect()
    }

    fn run(edits: &[DocumentEdit]) -> Vec<Value> {
        requests(&plan(&mut ids(), edits, || 42).unwrap())
    }

    #[test]
    fn a_spreadsheet_reads_as_values_formulas_and_charts_under_pdts_ids() {
        let v = json!({"sheets": [{"properties": {"title": "Data"}, "data": [{"rowData": [
            {"values": [{"formattedValue": "Item"}, {"formattedValue": "Cost"}]},
            {"values": [{"formattedValue": "Pens"}, {"formattedValue": "3", "userEnteredValue": {"numberValue": 3}}]},
            {"values": [{}, {"formattedValue": "3", "userEnteredValue": {"formulaValue": "=SUM(B2:B2)"}}]},
            {}
        ]}], "charts": [
            {"chartId": 99, "spec": {"title": "Costs", "basicChart": {"chartType": "COLUMN",
                "domains": [{"domain": {"sourceRange": {"sources": [{"startRowIndex": 0, "endRowIndex": 3, "startColumnIndex": 0, "endColumnIndex": 1}]}}}],
                "series": [{"series": {"sourceRange": {"sources": [{"startRowIndex": 0, "endRowIndex": 3, "startColumnIndex": 1, "endColumnIndex": 2}]}}}]}},
             "position": {"overlayPosition": {"anchorCell": {"rowIndex": 1, "columnIndex": 4}}}},
            {"chartId": 5, "spec": {"pieChart": {
                "domain": {"sourceRange": {"sources": [{"startRowIndex": 0, "endRowIndex": 3, "startColumnIndex": 0, "endColumnIndex": 1}]}},
                "series": {"sourceRange": {"sources": [{"startRowIndex": 0, "endRowIndex": 3, "startColumnIndex": 1, "endColumnIndex": 2}]}}}}}
        ]}], "developerMetadata": [{"metadataKey": CHART_KEY, "metadataValue": "99 chart-a"}]});
        let s = read(&v);
        assert_eq!(s[0].name, "Data");
        assert_eq!(s[0].rows.len(), 3);
        assert_eq!(s[0].rows[2][1], Cell { value: "3".into(), formula: Some("=SUM(B2:B2)".into()) });
        let c = &s[0].charts;
        assert_eq!(c[0].id, "chart-a");
        assert_eq!(
            c[0].spec,
            ChartSpec {
                kind: ChartKind::Column,
                data: "A1:B3".into(),
                title: Some("Costs".into()),
                anchor: Some("E2".into())
            }
        );
        assert_eq!((c[1].id.as_str(), c[1].spec.kind), ("5", ChartKind::Pie));
    }

    #[test]
    fn values_and_formulas_go_in_as_typed_with_requests_in_order() {
        let edits = vec![
            DocumentEdit::AddSheet { name: "Totals".into() },
            DocumentEdit::SetCells {
                sheet: "Totals".into(),
                cell: "B2".into(),
                values: vec![vec!["Sum".into(), "=SUM(Data!B:B)".into()]],
            },
            DocumentEdit::SetCells { sheet: "q'4".into(), cell: "A1".into(), values: vec![vec!["x".into()]] },
            DocumentEdit::Freeze { sheet: "Totals".into(), rows: 1, columns: 0 },
        ];
        let steps = plan(&mut ids(), &edits, || 42).unwrap();
        assert_eq!(steps.len(), 3);
        let Step::Values(v) = &steps[1] else { panic!("{steps:?}") };
        assert_eq!(v[0]["range"], "'Totals'!B2:C2");
        assert_eq!(v[1]["range"], "'Q''4'!A1:A1");
        let r = requests(&steps);
        assert_eq!(r[0]["addSheet"]["properties"], json!({"title": "Totals", "sheetId": 42}));
        assert_eq!(r[1]["updateSheetProperties"]["properties"]["sheetId"], 42);
    }

    #[test]
    fn formats_set_only_their_fields() {
        let f = CellFormat {
            bold: Some(true),
            background: Some("#ff0000".into()),
            number_format: Some("$#,##0.00".into()),
            vertical: Some(VAlign::Middle),
            ..Default::default()
        };
        let r = run(&[DocumentEdit::FormatRange { sheet: "Data".into(), range: "B2:B10".into(), format: f }]);
        let rc = &r[0]["repeatCell"];
        assert_eq!(
            rc["fields"],
            "userEnteredFormat.textFormat.bold,userEnteredFormat.backgroundColor,userEnteredFormat.verticalAlignment,userEnteredFormat.numberFormat"
        );
        assert_eq!(
            rc["range"],
            json!({"sheetId": 0, "startRowIndex": 1, "endRowIndex": 10, "startColumnIndex": 1, "endColumnIndex": 2})
        );
        assert_eq!(rc["cell"]["userEnteredFormat"]["backgroundColor"], json!({"red": 1.0, "green": 0.0, "blue": 0.0}));
        assert_eq!(rc["cell"]["userEnteredFormat"]["numberFormat"], json!({"type": "NUMBER", "pattern": "$#,##0.00"}));
        assert_eq!((number_kind("0%"), number_kind("yyyy-mm-dd"), number_kind("hh:mm")), ("PERCENT", "DATE", "TIME"));
        let bad =
            [DocumentEdit::FormatRange { sheet: "Data".into(), range: "B2".into(), format: CellFormat::default() }];
        assert!(plan(&mut ids(), &bad, || 1).is_err());
    }

    #[test]
    fn conditional_formats_validation_and_borders() {
        let r = run(&[
            DocumentEdit::ConditionalFormat {
                sheet: "Data".into(),
                range: "C2:C20".into(),
                when: CellCondition::GreaterThan { value: "100".into() },
                format: CellFormat { color: Some("#aa0000".into()), ..Default::default() },
            },
            DocumentEdit::ConditionalFormat {
                sheet: "Data".into(),
                range: "D2:D20".into(),
                when: CellCondition::NotEqualTo { value: "Done".into() },
                format: CellFormat { italic: Some(true), ..Default::default() },
            },
            DocumentEdit::Validate {
                sheet: "Data".into(),
                range: "E2:E50".into(),
                rule: CellValidation::OneOf { values: vec!["Yes".into(), "No".into()] },
                strict: true,
            },
            DocumentEdit::SetBorders {
                sheet: "Data".into(),
                range: "A1:D5".into(),
                style: BorderStyle::Thick,
                color: None,
                inner: true,
            },
            DocumentEdit::ColumnWidth { sheet: "Data".into(), columns: "B:C".into(), width: 140 },
        ]);
        let rule = &r[0]["addConditionalFormatRule"]["rule"];
        assert_eq!(
            rule["booleanRule"]["condition"],
            json!({"type": "NUMBER_GREATER", "values": [{"userEnteredValue": "100"}]})
        );
        assert_eq!(
            r[1]["addConditionalFormatRule"]["rule"]["booleanRule"]["condition"]["values"][0]["userEnteredValue"],
            "=D2<>\"Done\""
        );
        let v = &r[2]["setDataValidation"]["rule"];
        assert_eq!(
            (v["condition"]["type"].as_str(), v["strict"].as_bool(), v["showCustomUi"].as_bool()),
            (Some("ONE_OF_LIST"), Some(true), Some(true))
        );
        assert_eq!(r[3]["updateBorders"]["innerVertical"]["style"], "SOLID_THICK");
        assert_eq!(
            r[4]["updateDimensionProperties"]["range"],
            json!({"sheetId": 0, "dimension": "COLUMNS", "startIndex": 1, "endIndex": 3})
        );
    }

    #[test]
    fn charts_keep_pdts_ids() {
        let chart =
            ChartSpec { kind: ChartKind::Column, data: "A1:C5".into(), title: Some("Costs".into()), anchor: None };
        let r = run(&[
            DocumentEdit::AddChart { sheet: "Data".into(), id: Some("chart-x".into()), chart: chart.clone() },
            DocumentEdit::UpdateChart {
                id: "chart-x".into(),
                chart: ChartSpec { kind: ChartKind::Pie, ..chart.clone() },
            },
            DocumentEdit::DeleteChart { id: "chart-old".into() },
        ]);
        let n = chart_number("chart-x");
        let add = &r[0]["addChart"]["chart"];
        assert_eq!(add["chartId"], n);
        assert_eq!(add["spec"]["basicChart"]["series"].as_array().unwrap().len(), 2);
        assert_eq!(
            add["position"]["overlayPosition"]["anchorCell"],
            json!({"sheetId": 0, "rowIndex": 0, "columnIndex": 4})
        );
        assert_eq!(r[1]["createDeveloperMetadata"]["developerMetadata"]["metadataValue"], format!("{n} chart-x"));
        assert_eq!(r[2]["updateChartSpec"]["chartId"], n);
        assert!(r[2]["updateChartSpec"]["spec"]["pieChart"].is_object());
        assert_eq!(r[3]["deleteEmbeddedObject"]["objectId"], 99);
        let missing = [DocumentEdit::DeleteChart { id: "nope".into() }];
        assert!(matches!(plan(&mut ids(), &missing, || 1), Err(ProviderError::Invalid(m)) if m.contains("nope")));
        assert_eq!(chart_number("12"), 12);
        assert!(chart_number("chart-a") > 0);
    }

    #[test]
    fn sort_and_filter() {
        let r = run(&[
            DocumentEdit::Sort {
                sheet: "Data".into(),
                range: "A1:C5".into(),
                by: vec![SortKey { column: "B".into(), descending: true }],
                header: true,
            },
            DocumentEdit::Filter {
                sheet: "Data".into(),
                range: "A1:C5".into(),
                column: "C".into(),
                when: CellCondition::TextContains { text: "x".into() },
            },
            DocumentEdit::ClearFilter { sheet: "Data".into() },
        ]);
        assert_eq!(r[0]["sortRange"]["range"]["startRowIndex"], 1);
        assert_eq!(r[0]["sortRange"]["sortSpecs"], json!([{"dimensionIndex": 1, "sortOrder": "DESCENDING"}]));
        assert_eq!(r[1]["setBasicFilter"]["filter"]["filterSpecs"][0]["columnIndex"], 2);
        assert_eq!(r[2]["clearBasicFilter"]["sheetId"], 0);
        let outside = [DocumentEdit::Sort {
            sheet: "Data".into(),
            range: "A1:B5".into(),
            by: vec![SortKey { column: "D".into(), descending: false }],
            header: false,
        }];
        assert!(plan(&mut ids(), &outside, || 1).is_err());
    }

    #[test]
    fn unknown_sheets_and_doc_edits_are_refused() {
        let e = [DocumentEdit::DeleteSheet { sheet: "Nope".into() }];
        assert!(matches!(plan(&mut ids(), &e, || 1), Err(ProviderError::Invalid(m)) if m.contains("Nope")));
        let e = [DocumentEdit::AppendText { markdown: "x".into() }];
        assert!(matches!(plan(&mut ids(), &e, || 1), Err(ProviderError::Unsupported(_))));
    }
}
