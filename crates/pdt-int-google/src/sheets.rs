//! Google Sheets (PDT-316): a spreadsheet read as PDT's sheets of cells (what each shows, and its formula), and PDT's
//! sheet edits as Sheets API requests: values and formulas (`values.batchUpdate`, as if typed), sheets, formats,
//! borders, widths, frozen rows, conditional formats, validation, charts, sorting and filters (`batchUpdate`). Sheets
//! are named in edits; Google wants their ids, from the spreadsheet's properties. Pure.

use pdt_api::sdk::documents::{
    Align, BorderStyle, CellCondition, CellConditionKind, CellFormat, CellRange, ChartKind, ChartSpec, DocumentEdit,
    NumberKind, ProviderError, Sheet, column_name, parse_cell, parse_column, parse_range,
};
use pdt_core::document::Cell;
use serde_json::{Map, Value, json};

/// The fields of a read: each sheet's name and its cells' shown values and formulas.
pub const READ_FIELDS: &str =
    "sheets(properties(sheetId,title),data(rowData(values(formattedValue,userEnteredValue))))";
pub const META_FIELDS: &str = "sheets(properties(sheetId,title))";

/// A spreadsheet's sheets, as `spreadsheets.get` with [`READ_FIELDS`] gave them.
pub fn read(v: &Value) -> Vec<Sheet> {
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
        out.push(Sheet { name, rows });
    }
    out
}

/// The sheets' ids by name.
#[derive(Debug, Clone, Default)]
pub struct Ids(Vec<(String, i64)>);

impl Ids {
    pub fn of(meta: &Value) -> Self {
        Self(
            meta["sheets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| {
                    let p = &s["properties"];
                    Some((p["title"].as_str()?.to_string(), p["sheetId"].as_i64()?))
                })
                .collect(),
        )
    }
    fn id(&self, name: &str) -> Result<i64, ProviderError> {
        self.0
            .iter()
            .find(|(n, _)| n == name)
            .or_else(|| self.0.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)))
            .map(|(_, id)| *id)
            .ok_or_else(|| ProviderError::Invalid(format!("the spreadsheet has no sheet called {name}")))
    }
    /// The sheet's name as the spreadsheet has it.
    fn name(&self, name: &str) -> Result<String, ProviderError> {
        let id = self.id(name)?;
        Ok(self.0.iter().find(|(_, i)| *i == id).map(|(n, _)| n.clone()).unwrap_or_default())
    }
}

/// One step of an edit: requests for `batchUpdate`, or values to write.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Requests(Vec<Value>),
    /// `values.batchUpdate`'s `data`: ranges with their values.
    Values(Vec<Value>),
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
        let step = match e {
            DocumentEdit::SetCells { sheet, cell, values } => {
                let (row, col) =
                    parse_cell(cell).ok_or_else(|| ProviderError::Invalid(format!("{cell} is not a cell")))?;
                let name = ids.name(sheet)?;
                let width = values.iter().map(Vec::len).max().unwrap_or(1).max(1);
                let end = format!("{}{}", column_name(col + width - 1), row + values.len().max(1));
                let range = format!("'{}'!{}{}:{end}", name.replace('\'', "''"), column_name(col), row + 1);
                Step::Values(vec![json!({"range": range, "majorDimension": "ROWS", "values": values})])
            }
            DocumentEdit::AddSheet { name } => {
                if ids.0.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
                    return Err(ProviderError::Invalid(format!("there is a sheet called {name} already")));
                }
                let id = new_id();
                ids.0.push((name.clone(), id));
                Step::Requests(vec![json!({"addSheet": {"properties": {"title": name, "sheetId": id}}})])
            }
            DocumentEdit::RenameSheet { sheet, name } => {
                let id = ids.id(sheet)?;
                for (n, i) in ids.0.iter_mut() {
                    if *i == id {
                        *n = name.clone();
                    }
                }
                Step::Requests(vec![json!({"updateSheetProperties": {
                    "properties": {"sheetId": id, "title": name}, "fields": "title"}})])
            }
            DocumentEdit::DeleteSheet { sheet } => {
                let id = ids.id(sheet)?;
                ids.0.retain(|(_, i)| *i != id);
                Step::Requests(vec![json!({"deleteSheet": {"sheetId": id}})])
            }
            DocumentEdit::FormatCells { sheet, range, format } => {
                let (fmt, fields) = cell_format(format)?;
                if fields.is_empty() {
                    return Err(ProviderError::Invalid("the format sets nothing".into()));
                }
                Step::Requests(vec![json!({"repeatCell": {
                    "range": grid(ids.id(sheet)?, range)?,
                    "cell": {"userEnteredFormat": fmt},
                    "fields": fields.iter().map(|f| format!("userEnteredFormat.{f}")).collect::<Vec<_>>().join(",")}})])
            }
            DocumentEdit::SetBorders { sheet, range, style, color, inner } => {
                let border =
                    json!({"style": border_style(*style), "color": color_of(color.as_deref().unwrap_or("#000000"))?});
                let mut b = Map::new();
                b.insert("range".into(), grid(ids.id(sheet)?, range)?);
                for side in ["top", "bottom", "left", "right"] {
                    b.insert(side.into(), border.clone());
                }
                if *inner {
                    b.insert("innerHorizontal".into(), border.clone());
                    b.insert("innerVertical".into(), border);
                }
                Step::Requests(vec![json!({"updateBorders": b})])
            }
            DocumentEdit::ColumnWidth { sheet, columns, width } => {
                let r = parse_range(columns)
                    .filter(|r| r.start_col.is_some())
                    .ok_or_else(|| ProviderError::Invalid(format!("{columns} are not columns (A, or B:D)")))?;
                Step::Requests(vec![json!({"updateDimensionProperties": {
                    "range": {"sheetId": ids.id(sheet)?, "dimension": "COLUMNS",
                              "startIndex": r.start_col, "endIndex": r.end_col},
                    "properties": {"pixelSize": width}, "fields": "pixelSize"}})])
            }
            DocumentEdit::Freeze { sheet, rows, columns } => Step::Requests(vec![json!({"updateSheetProperties": {
                "properties": {"sheetId": ids.id(sheet)?,
                               "gridProperties": {"frozenRowCount": rows, "frozenColumnCount": columns}},
                "fields": "gridProperties.frozenRowCount,gridProperties.frozenColumnCount"}})]),
            DocumentEdit::ConditionalFormat { sheet, range, when, format } => {
                let (fmt, fields) = cell_format(format)?;
                if fields.is_empty() {
                    return Err(ProviderError::Invalid("the format sets nothing".into()));
                }
                if matches!(
                    when.kind,
                    CellConditionKind::OneOf | CellConditionKind::Checkbox | CellConditionKind::IsDate
                ) {
                    return Err(ProviderError::Invalid(format!(
                        "{:?} is for validation, not conditional formatting",
                        when.kind
                    )));
                }
                Step::Requests(vec![json!({"addConditionalFormatRule": {"index": 0, "rule": {
                    "ranges": [grid(ids.id(sheet)?, range)?],
                    "booleanRule": {"condition": condition(when)?, "format": fmt}}}})])
            }
            DocumentEdit::Validate { sheet, range, rule, strict } => {
                Step::Requests(vec![json!({"setDataValidation": {
                    "range": grid(ids.id(sheet)?, range)?,
                    "rule": {"condition": condition(rule)?, "strict": strict,
                             "showCustomUi": rule.kind == CellConditionKind::OneOf}}})])
            }
            DocumentEdit::AddChart { sheet, chart } => Step::Requests(vec![add_chart(ids, sheet, chart)?]),
            DocumentEdit::Sort { sheet, range, by } => {
                if by.is_empty() {
                    return Err(ProviderError::Invalid("a sort needs a column to sort by".into()));
                }
                let specs = by
                    .iter()
                    .map(|k| {
                        let c = parse_column(&k.column)
                            .ok_or_else(|| ProviderError::Invalid(format!("{} is not a column", k.column)))?;
                        Ok(json!({"dimensionIndex": c, "sortOrder": if k.descending { "DESCENDING" } else { "ASCENDING" }}))
                    })
                    .collect::<Result<Vec<_>, ProviderError>>()?;
                Step::Requests(vec![json!({"sortRange": {"range": grid(ids.id(sheet)?, range)?, "sortSpecs": specs}})])
            }
            DocumentEdit::Filter { sheet, range } => {
                let id = ids.id(sheet)?;
                Step::Requests(vec![match range {
                    Some(r) => json!({"setBasicFilter": {"filter": {"range": grid(id, r)?}}}),
                    None => json!({"clearBasicFilter": {"sheetId": id}}),
                }])
            }
            e => {
                return Err(ProviderError::Unsupported(format!(
                    "a Google Sheet has no text to change ({} is a Docs edit)",
                    e.op()
                )));
            }
        };
        push(&mut steps, step);
    }
    Ok(steps)
}

/// A range on a sheet as Google's `GridRange` (ends left out run to the edge).
pub fn grid(sheet_id: i64, range: &str) -> Result<Value, ProviderError> {
    let r: CellRange =
        parse_range(range).ok_or_else(|| ProviderError::Invalid(format!("{range} is not a range (A1:C10)")))?;
    let mut g = Map::new();
    g.insert("sheetId".into(), json!(sheet_id));
    for (k, v) in [
        ("startRowIndex", r.start_row),
        ("endRowIndex", r.end_row),
        ("startColumnIndex", r.start_col),
        ("endColumnIndex", r.end_col),
    ] {
        if let Some(v) = v {
            g.insert(k.into(), json!(v));
        }
    }
    Ok(Value::Object(g))
}

/// `#rrggbb` as Google's colour.
pub fn color_of(hex: &str) -> Result<Value, ProviderError> {
    let h = hex.trim().trim_start_matches('#');
    let bad = || ProviderError::Invalid(format!("{hex} is not a colour (#rrggbb)"));
    if h.len() != 6 {
        return Err(bad());
    }
    let part = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map(|b| f64::from(b) / 255.0).map_err(|_| bad());
    Ok(json!({"red": part(0)?, "green": part(2)?, "blue": part(4)?}))
}

/// A format as Google's `CellFormat`, and the fields it sets (under `userEnteredFormat`).
fn cell_format(f: &CellFormat) -> Result<(Value, Vec<&'static str>), ProviderError> {
    let mut out = Map::new();
    let mut text = Map::new();
    let mut fields = vec![];
    for (key, field, v) in [
        ("bold", "textFormat.bold", f.bold),
        ("italic", "textFormat.italic", f.italic),
        ("strikethrough", "textFormat.strikethrough", f.strikethrough),
    ] {
        if let Some(v) = v {
            text.insert(key.into(), json!(v));
            fields.push(field);
        }
    }
    if let Some(s) = f.font_size {
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
    if let Some(a) = f.align {
        let a = match a {
            Align::Left => "LEFT",
            Align::Center => "CENTER",
            Align::Right => "RIGHT",
        };
        out.insert("horizontalAlignment".into(), json!(a));
        fields.push("horizontalAlignment");
    }
    if let Some(w) = f.wrap {
        out.insert("wrapStrategy".into(), json!(if w { "WRAP" } else { "OVERFLOW_CELL" }));
        fields.push("wrapStrategy");
    }
    if let Some(n) = &f.number_format {
        let kind = match n.kind {
            NumberKind::Number => "NUMBER",
            NumberKind::Currency => "CURRENCY",
            NumberKind::Percent => "PERCENT",
            NumberKind::Date => "DATE",
            NumberKind::Time => "TIME",
            NumberKind::DateTime => "DATE_TIME",
            NumberKind::Scientific => "SCIENTIFIC",
            NumberKind::Text => "TEXT",
        };
        let mut nf = json!({"type": kind});
        if let Some(p) = &n.pattern {
            nf["pattern"] = json!(p);
        }
        out.insert("numberFormat".into(), nf);
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

fn condition(c: &CellCondition) -> Result<Value, ProviderError> {
    c.check().map_err(ProviderError::Invalid)?;
    let kind = match c.kind {
        CellConditionKind::GreaterThan => "NUMBER_GREATER",
        CellConditionKind::GreaterOrEqual => "NUMBER_GREATER_THAN_EQ",
        CellConditionKind::LessThan => "NUMBER_LESS",
        CellConditionKind::LessOrEqual => "NUMBER_LESS_THAN_EQ",
        CellConditionKind::Equal => "NUMBER_EQ",
        CellConditionKind::NotEqual => "NUMBER_NOT_EQ",
        CellConditionKind::Between => "NUMBER_BETWEEN",
        CellConditionKind::NotBetween => "NUMBER_NOT_BETWEEN",
        CellConditionKind::TextContains => "TEXT_CONTAINS",
        CellConditionKind::TextNotContains => "TEXT_NOT_CONTAINS",
        CellConditionKind::TextStartsWith => "TEXT_STARTS_WITH",
        CellConditionKind::TextEndsWith => "TEXT_ENDS_WITH",
        CellConditionKind::Blank => "BLANK",
        CellConditionKind::NotBlank => "NOT_BLANK",
        CellConditionKind::Formula => "CUSTOM_FORMULA",
        CellConditionKind::OneOf => "ONE_OF_LIST",
        CellConditionKind::Checkbox => "BOOLEAN",
        CellConditionKind::IsDate => "DATE_IS_VALID",
    };
    let mut out = json!({"type": kind});
    if !c.values.is_empty() {
        out["values"] = c.values.iter().map(|v| json!({"userEnteredValue": v})).collect();
    }
    Ok(out)
}

fn add_chart(ids: &Ids, sheet: &str, c: &ChartSpec) -> Result<Value, ProviderError> {
    let id = ids.id(sheet)?;
    if c.series.is_empty() {
        return Err(ProviderError::Invalid("a chart needs at least one series".into()));
    }
    let source = |r: &str| -> Result<Value, ProviderError> { Ok(json!({"sourceRange": {"sources": [grid(id, r)?]}})) };
    let headers = if c.headers.unwrap_or(true) { 1 } else { 0 };
    let mut spec = Map::new();
    if let Some(t) = &c.title {
        spec.insert("title".into(), json!(t));
    }
    if c.kind == ChartKind::Pie {
        if c.series.len() != 1 {
            return Err(ProviderError::Invalid("a pie chart takes one series".into()));
        }
        spec.insert(
            "pieChart".into(),
            json!({"legendPosition": "RIGHT_LEGEND", "domain": source(&c.domain)?, "series": source(&c.series[0])?}),
        );
    } else {
        let kind = match c.kind {
            ChartKind::Line => "LINE",
            ChartKind::Bar => "BAR",
            ChartKind::Column => "COLUMN",
            ChartKind::Area => "AREA",
            ChartKind::Scatter => "SCATTER",
            ChartKind::Pie => unreachable!(),
        };
        // A bar chart's values run along the bottom axis; the rest's up the left.
        let axis = if c.kind == ChartKind::Bar { "BOTTOM_AXIS" } else { "LEFT_AXIS" };
        let series = c
            .series
            .iter()
            .map(|s| Ok(json!({"series": source(s)?, "targetAxis": axis})))
            .collect::<Result<Vec<_>, ProviderError>>()?;
        spec.insert(
            "basicChart".into(),
            json!({"chartType": kind, "legendPosition": "BOTTOM_LEGEND", "headerCount": headers,
                   "domains": [{"domain": source(&c.domain)?}], "series": series}),
        );
    }
    let position = match (&c.anchor, &c.place_sheet) {
        (None, None) => json!({"newSheet": true}),
        (anchor, place) => {
            let on = match place {
                Some(p) => ids.id(p)?,
                None => id,
            };
            let a = anchor.as_deref().unwrap_or("A1");
            let (row, col) = parse_cell(a).ok_or_else(|| ProviderError::Invalid(format!("{a} is not a cell")))?;
            json!({"overlayPosition": {"anchorCell": {"sheetId": on, "rowIndex": row, "columnIndex": col}}})
        }
    };
    Ok(json!({"addChart": {"chart": {"spec": spec, "position": position}}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_api::sdk::documents::{NumberFormat, SortKey};

    fn ids() -> Ids {
        Ids::of(&json!({"sheets": [{"properties": {"sheetId": 0, "title": "Data"}},
                                   {"properties": {"sheetId": 7, "title": "Q'4"}}]}))
    }

    fn requests(steps: &[Step]) -> Vec<Value> {
        steps.iter().filter_map(|s| if let Step::Requests(r) = s { Some(r.clone()) } else { None }).flatten().collect()
    }

    #[test]
    fn a_spreadsheet_reads_as_values_and_formulas() {
        let v = json!({"sheets": [{"properties": {"title": "Data"}, "data": [{"rowData": [
            {"values": [{"formattedValue": "Item"}, {"formattedValue": "Cost"}]},
            {"values": [{"formattedValue": "Pens"}, {"formattedValue": "3", "userEnteredValue": {"numberValue": 3}}]},
            {"values": [{}, {"formattedValue": "3", "userEnteredValue": {"formulaValue": "=SUM(B2:B2)"}}]},
            {}
        ]}]}]});
        let s = read(&v);
        assert_eq!(s[0].name, "Data");
        assert_eq!(s[0].rows.len(), 3);
        assert_eq!(s[0].rows[2][1], Cell { value: "3".into(), formula: Some("=SUM(B2:B2)".into()) });
        assert_eq!(s[0].rows[2][0], Cell::default());
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
            number_format: Some(NumberFormat { kind: NumberKind::Currency, pattern: Some("$#,##0.00".into()) }),
            ..Default::default()
        };
        let edits = [DocumentEdit::FormatCells { sheet: "Data".into(), range: "B2:B10".into(), format: f }];
        let r = requests(&plan(&mut ids(), &edits, || 1).unwrap());
        let rc = &r[0]["repeatCell"];
        assert_eq!(
            rc["fields"],
            "userEnteredFormat.textFormat.bold,userEnteredFormat.backgroundColor,userEnteredFormat.numberFormat"
        );
        assert_eq!(
            rc["range"],
            json!({"sheetId": 0, "startRowIndex": 1, "endRowIndex": 10, "startColumnIndex": 1, "endColumnIndex": 2})
        );
        assert_eq!(rc["cell"]["userEnteredFormat"]["backgroundColor"], json!({"red": 1.0, "green": 0.0, "blue": 0.0}));
        assert_eq!(rc["cell"]["userEnteredFormat"]["numberFormat"]["type"], "CURRENCY");
        let bad =
            [DocumentEdit::FormatCells { sheet: "Data".into(), range: "B2".into(), format: CellFormat::default() }];
        assert!(plan(&mut ids(), &bad, || 1).is_err());
    }

    #[test]
    fn conditional_formats_validation_and_borders() {
        let edits = [
            DocumentEdit::ConditionalFormat {
                sheet: "Data".into(),
                range: "C:C".into(),
                when: CellCondition { kind: CellConditionKind::GreaterThan, values: vec!["100".into()] },
                format: CellFormat { color: Some("#aa0000".into()), ..Default::default() },
            },
            DocumentEdit::Validate {
                sheet: "Data".into(),
                range: "D2:D50".into(),
                rule: CellCondition { kind: CellConditionKind::OneOf, values: vec!["Yes".into(), "No".into()] },
                strict: true,
            },
            DocumentEdit::SetBorders {
                sheet: "Data".into(),
                range: "A1:D5".into(),
                style: BorderStyle::Thick,
                color: None,
                inner: true,
            },
        ];
        let r = requests(&plan(&mut ids(), &edits, || 1).unwrap());
        let rule = &r[0]["addConditionalFormatRule"]["rule"];
        assert_eq!(
            rule["booleanRule"]["condition"],
            json!({"type": "NUMBER_GREATER", "values": [{"userEnteredValue": "100"}]})
        );
        assert_eq!(rule["ranges"][0], json!({"sheetId": 0, "startColumnIndex": 2, "endColumnIndex": 3}));
        let v = &r[1]["setDataValidation"]["rule"];
        assert_eq!(
            (v["condition"]["type"].as_str(), v["strict"].as_bool(), v["showCustomUi"].as_bool()),
            (Some("ONE_OF_LIST"), Some(true), Some(true))
        );
        assert_eq!(r[2]["updateBorders"]["innerVertical"]["style"], "SOLID_THICK");
        let wrong = [DocumentEdit::ConditionalFormat {
            sheet: "Data".into(),
            range: "A1".into(),
            when: CellCondition { kind: CellConditionKind::Between, values: vec!["1".into()] },
            format: CellFormat { bold: Some(true), ..Default::default() },
        }];
        assert!(matches!(plan(&mut ids(), &wrong, || 1), Err(ProviderError::Invalid(_))));
    }

    #[test]
    fn charts_sort_filter_and_widths() {
        let chart = ChartSpec {
            kind: ChartKind::Column,
            title: Some("Costs".into()),
            domain: "A1:A5".into(),
            series: vec!["B1:B5".into(), "C1:C5".into()],
            headers: None,
            anchor: Some("E2".into()),
            place_sheet: None,
        };
        let pie = ChartSpec { kind: ChartKind::Pie, series: vec!["B1:B5".into()], anchor: None, ..chart.clone() };
        let edits = [
            DocumentEdit::AddChart { sheet: "Data".into(), chart },
            DocumentEdit::AddChart { sheet: "Data".into(), chart: pie },
            DocumentEdit::Sort {
                sheet: "Data".into(),
                range: "A2:C5".into(),
                by: vec![SortKey { column: "B".into(), descending: true }],
            },
            DocumentEdit::Filter { sheet: "Data".into(), range: Some("A1:C5".into()) },
            DocumentEdit::Filter { sheet: "Data".into(), range: None },
            DocumentEdit::ColumnWidth { sheet: "Data".into(), columns: "B:C".into(), width: 140 },
        ];
        let r = requests(&plan(&mut ids(), &edits, || 1).unwrap());
        let c = &r[0]["addChart"]["chart"];
        assert_eq!(c["spec"]["basicChart"]["chartType"], "COLUMN");
        assert_eq!(c["spec"]["basicChart"]["headerCount"], 1);
        assert_eq!(c["spec"]["basicChart"]["series"].as_array().unwrap().len(), 2);
        assert_eq!(
            c["position"]["overlayPosition"]["anchorCell"],
            json!({"sheetId": 0, "rowIndex": 1, "columnIndex": 4})
        );
        assert_eq!(r[1]["addChart"]["chart"]["position"], json!({"newSheet": true}));
        assert!(r[1]["addChart"]["chart"]["spec"]["pieChart"]["series"].is_object());
        assert_eq!(r[2]["sortRange"]["sortSpecs"], json!([{"dimensionIndex": 1, "sortOrder": "DESCENDING"}]));
        assert!(r[3]["setBasicFilter"].is_object() && r[4]["clearBasicFilter"]["sheetId"] == 0);
        assert_eq!(
            r[5]["updateDimensionProperties"]["range"],
            json!({"sheetId": 0, "dimension": "COLUMNS", "startIndex": 1, "endIndex": 3})
        );
    }

    #[test]
    fn unknown_sheets_and_doc_edits_are_refused() {
        let e = [DocumentEdit::DeleteSheet { sheet: "Nope".into() }];
        assert!(matches!(plan(&mut ids(), &e, || 1), Err(ProviderError::Invalid(m)) if m.contains("Nope")));
        let e = [DocumentEdit::AppendText { markdown: "x".into() }];
        assert!(matches!(plan(&mut ids(), &e, || 1), Err(ProviderError::Unsupported(_))));
    }
}
