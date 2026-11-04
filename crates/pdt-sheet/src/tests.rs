use super::*;
use ironcalc::base::cf_types::CfRuleInput;
use ironcalc::base::types::{Dxf, DxfFont};
use pdt_core::document::{BorderStyle, CellCondition, CellFormat, HAlign, SortKey};

fn sales() -> Workbook {
    let mut wb = Workbook::new("Sales").unwrap();
    let rows: Vec<Vec<String>> = [
        ["Quarter", "Revenue", "Cost", "Margin"],
        ["Q1", "1200", "800", "=B2-C2"],
        ["Q2", "1500", "900", "=B3-C3"],
        ["Q3", "1700", "1000", "=B4-C4"],
        ["Total", "=SUM(B2:B4)", "=SUM(C2:C4)", "=SUM(D2:D4)"],
    ]
    .iter()
    .map(|r| r.iter().map(|s| s.to_string()).collect())
    .collect();
    wb.set_cells("Sheet1", "A1", &rows).unwrap();
    wb
}

fn shown(wb: &Workbook, sheet: &str, range: &str) -> Vec<Vec<String>> {
    wb.shown(sheet, range).unwrap()
}

#[test]
fn formulas_compute_and_read_back() {
    let wb = sales();
    let sheets = wb.sheets().unwrap();
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].rows.len(), 5);
    assert_eq!(sheets[0].rows[4][1].value, "4400");
    assert_eq!(sheets[0].rows[4][1].formula.as_deref(), Some("=SUM(B2:B4)"));
    assert_eq!(wb.values("Sheet1", "D2:D2").unwrap(), vec![vec![Value::Number(400.0)]]);
    assert_eq!(wb.values("sheet1", "A1:A1").unwrap(), vec![vec![Value::Text("Quarter".into())]]);
}

/// A leading `'` makes the rest text, as in any sheet: a workflow writes text from outside so it never runs.
#[test]
fn a_quoted_value_is_text_not_a_formula() {
    let mut wb = Workbook::new("Log").unwrap();
    wb.set_cells("Sheet1", "A1", &[vec!["'=1+1".into(), "=1+1".into()]]).unwrap();
    let rows = &wb.sheets().unwrap()[0].rows;
    assert_eq!((rows[0][0].value.as_str(), rows[0][0].formula.as_deref()), ("=1+1", None));
    assert_eq!((rows[0][1].value.as_str(), rows[0][1].formula.as_deref()), ("2", Some("=1+1")));
    assert_eq!(wb.values("Sheet1", "A1:A1").unwrap(), vec![vec![Value::Text("=1+1".into())]]);
}

#[test]
fn an_xlsx_round_trips_with_formats_merges_panes_widths_and_names() {
    let mut wb = sales();
    wb.set_style("Sheet1", "A1:D1", "font.b", "true").unwrap();
    wb.set_style("Sheet1", "A1:D1", "fill.color", "#DCE6F2").unwrap();
    wb.set_style("Sheet1", "B2:C5", "num_fmt", "#,##0").unwrap();
    wb.add_sheet("Notes").unwrap();
    wb.set_cells("Notes", "A1", &[vec!["=Sheet1!D5".into()]]).unwrap();
    let m = wb.model_mut();
    m.merge_cells(&Area { sheet: 1, row: 3, column: 1, width: 2, height: 1 }).unwrap();
    m.set_frozen_rows_count(0, 1).unwrap();
    m.set_columns_width(0, 1, 1, 150.0).unwrap();
    m.new_defined_name("Revenue", None, "Sheet1!$B$2:$B$4").unwrap();
    let red = Dxf { font: Some(DxfFont { b: Some(true), ..Default::default() }), ..Default::default() };
    m.add_conditional_formatting(
        0,
        "D2:D4",
        CfRuleInput::Formula { formula: "=D2>450".into(), format: red, stop_if_true: false },
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sales.xlsx");
    wb.save(&path).unwrap();
    assert!(!path.with_extension("xlsx.tmp").exists());
    let back = Workbook::open(&path).unwrap();

    assert_eq!(back.sheet_names(), vec!["Sheet1", "Notes"]);
    assert_eq!(shown(&back, "Sheet1", "B5:B5"), vec![vec!["4,400".to_string()]]);
    assert_eq!(back.input("Notes", "A1").unwrap(), "=Sheet1!D5");
    assert_eq!(shown(&back, "Notes", "A1:A1"), vec![vec!["1700".to_string()]]);
    let s = back.style("Sheet1", "B1").unwrap();
    assert!(s.font.b);
    assert_eq!(back.model().resolve_color(&s.fill.color).to_ascii_uppercase(), "#DCE6F2");
    let m = back.model();
    assert_eq!(m.get_merged_cells(1).unwrap().len(), 1);
    assert_eq!(m.get_frozen_rows_count(0).unwrap(), 1);
    assert!((m.get_column_width(0, 1).unwrap() - 150.0).abs() < 1.0, "{}", m.get_column_width(0, 1).unwrap());
    assert!(m.get_defined_name_list().iter().any(|(n, _, f)| n == "Revenue" && f.contains("$B$2")));
    assert_eq!(m.get_conditional_formatting_list(0).unwrap().len(), 1);
}

#[test]
fn csv_comes_in_typed_and_goes_out_as_shown() {
    let wb = Workbook::from_csv("\u{feff}Item;Qty\nPens;3\nInk;=B2*2\n", "Stock", "Stock").unwrap();
    assert_eq!(wb.sheet_names(), vec!["Stock"]);
    assert_eq!(wb.values("Stock", "B2:B3").unwrap(), vec![vec![Value::Number(3.0)], vec![Value::Number(6.0)]]);
    let csv = wb.to_csv("Stock").unwrap();
    assert_eq!(csv, "Item,Qty\nPens,3\nInk,6\n");

    let quoted = Workbook::from_csv("a,b\n\"x, y\",\"say \"\"hi\"\"\"\n", "Q", "Sheet1").unwrap();
    assert_eq!(quoted.to_csv("Sheet1").unwrap(), "a,b\n\"x, y\",\"say \"\"hi\"\"\"\n");
}

fn key(column: &str, descending: bool) -> SortKey {
    SortKey { column: column.into(), descending }
}

#[test]
fn sorting_rewrites_the_rows_and_keeps_formulas_right() {
    let mut wb = sales();
    wb.set_style("Sheet1", "A2", "font.b", "true").unwrap();
    wb.sort("Sheet1", "A2:D4", &[key("D", true)], false).unwrap();
    assert_eq!(
        shown(&wb, "Sheet1", "A2:D4"),
        vec![vec!["Q3", "1700", "1000", "700"], vec!["Q2", "1500", "900", "600"], vec!["Q1", "1200", "800", "400"],]
    );
    // Each margin still subtracts its own row's cells, and the total still covers them.
    assert_eq!(wb.input("Sheet1", "D2").unwrap(), "=B2-C2");
    assert_eq!(shown(&wb, "Sheet1", "D5:D5"), vec![vec!["1700".to_string()]]);
    assert_eq!(wb.input("Sheet1", "B5").unwrap(), "=SUM(B2:B4)", "a range over the sorted rows stays as it was");
    assert!(wb.style("Sheet1", "A4").unwrap().font.b, "a format moves with its cell");
    assert!(!wb.style("Sheet1", "A2").unwrap().font.b);
    // With a header row, which stays on top.
    wb.sort("Sheet1", "A1:D4", &[key("a", false)], true).unwrap();
    assert_eq!(
        shown(&wb, "Sheet1", "A1:A4"),
        vec![vec!["Quarter".to_string()], vec!["Q1".into()], vec!["Q2".into()], vec!["Q3".into()]]
    );
    assert!(wb.sort("Sheet1", "A2:B4", &[key("D", false)], false).is_err());
    assert!(wb.sort("Sheet1", "A2:B4", &[], false).is_err());
    // A second key breaks ties.
    wb.set_cells("Sheet1", "B2", &[vec!["1500".into()]]).unwrap();
    wb.sort("Sheet1", "A1:D4", &[key("B", true), key("A", true)], true).unwrap();
    assert_eq!(shown(&wb, "Sheet1", "A2:A4"), vec![vec!["Q3".to_string()], vec!["Q2".into()], vec!["Q1".into()]]);
}

#[test]
fn a_filter_hides_rows_and_clears() {
    let mut wb = sales();
    let hidden = |wb: &Workbook| (2..=5).map(|r| wb.row_hidden("Sheet1", r).unwrap()).collect::<Vec<_>>();
    wb.filter("Sheet1", "A1:D5", "B", &CellCondition::GreaterThan { value: "1400".into() }).unwrap();
    assert_eq!(hidden(&wb), vec![true, false, false, false], "the total (4400) shows too");
    wb.filter("Sheet1", "A1:D5", "A", &CellCondition::TextContains { text: "q".into() }).unwrap();
    assert_eq!(hidden(&wb), vec![false, false, false, true]);
    wb.filter("Sheet1", "A1:D5", "A", &CellCondition::EqualTo { value: "q2".into() }).unwrap();
    assert_eq!(hidden(&wb), vec![true, false, true, true]);
    assert!(wb.filter("Sheet1", "A1:D5", "A", &CellCondition::Formula { formula: "=1".into() }).is_err());
    wb.clear_filter("Sheet1").unwrap();
    assert_eq!(hidden(&wb), vec![false; 4]);
}

#[test]
fn formats_and_conditional_formats_go_on_ranges() {
    let mut wb = sales();
    let f = CellFormat {
        bold: Some(true),
        background: Some("#dce6f2".into()),
        number_format: Some("#,##0.00".into()),
        horizontal: Some(HAlign::Center),
        ..Default::default()
    };
    wb.format("Sheet1", "B2:B4", &f).unwrap();
    let s = wb.style("Sheet1", "B3").unwrap();
    assert!(s.font.b);
    assert_eq!(s.num_fmt, "#,##0.00");
    assert_eq!(shown(&wb, "Sheet1", "B3:B3"), vec![vec!["1,500.00".to_string()]]);
    assert!(wb.format("Sheet1", "B2", &CellFormat::default()).is_err());
    assert!(wb.format("Sheet1", "B2", &CellFormat { color: Some("red".into()), ..Default::default() }).is_err());

    let red = CellFormat { color: Some("#cc0000".into()), ..Default::default() };
    wb.conditional_format("Sheet1", "D2:D4", &CellCondition::GreaterThan { value: "500".into() }, &red).unwrap();
    let rules = wb.model().get_conditional_formatting_list(0).unwrap();
    assert_eq!(rules.len(), 1);
    assert!(wb.conditional_format("Sheet1", "D2:D4", &CellCondition::Empty, &CellFormat::default()).is_err());
    let named = CellFormat { color: Some("red".into()), ..Default::default() };
    assert!(wb.conditional_format("Sheet1", "D2:D4", &CellCondition::Empty, &named).is_err(), "a colour is #rrggbb");
    let back = Workbook::from_xlsx(&wb.to_xlsx().unwrap(), "x").unwrap();
    assert_eq!(back.model().get_conditional_formatting_list(0).unwrap().len(), 1, "kept in the file");
}

#[test]
fn sheets_are_added_renamed_and_deleted() {
    let mut wb = Workbook::new("W").unwrap();
    wb.add_sheet("Data").unwrap();
    assert!(wb.add_sheet("data").is_err());
    wb.rename_sheet("Data", "Figures").unwrap();
    assert_eq!(wb.sheet_names(), vec!["Sheet1", "Figures"]);
    wb.delete_sheet("Sheet1").unwrap();
    assert!(wb.delete_sheet("Figures").is_err());
    assert!(wb.set_cells("Nope", "A1", &[vec!["1".into()]]).is_err());
}

#[test]
fn diffs_keep_two_models_in_step() {
    let mut daemon = sales();
    daemon.flush();
    let mut view = Workbook::from_bytes(&daemon.to_bytes()).unwrap();
    // The daemon's edit reaches the view…
    daemon.set_cells("Sheet1", "B2", &[vec!["2000".into()]]).unwrap();
    view.apply_diffs(&daemon.flush()).unwrap();
    assert_eq!(shown(&view, "Sheet1", "D2:D2"), vec![vec!["1200".to_string()]]);
    // …and the view's reaches the daemon.
    view.set_cells("Sheet1", "C3", &[vec!["100".into()]]).unwrap();
    let diffs = view.flush();
    daemon.apply_diffs(&diffs).unwrap();
    assert_eq!(shown(&daemon, "Sheet1", "D3:D3"), vec![vec!["1400".to_string()]]);
    assert!(daemon.flush().len() <= 1, "a view's diffs are not sent on again");
}

#[test]
fn an_imported_files_dropped_parts_are_named() {
    let xlsx = sales().to_xlsx().unwrap();
    assert!(dropped_parts(&xlsx).is_empty());
    assert!(Workbook::from_xlsx(&xlsx, "x").is_ok());

    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for name in ["xl/workbook.xml", "xl/charts/chart1.xml", "xl/media/image1.png", "xl/pivotTables/p.xml"] {
        out.start_file(name, opts).unwrap();
    }
    let bytes = out.finish().unwrap().into_inner();
    assert_eq!(dropped_parts(&bytes), vec!["charts", "images and drawings", "pivot tables"]);
    assert!(Workbook::from_xlsx(b"not a zip", "x").is_err());
}

#[test]
fn the_web_ui_runs_the_same_ironcalc() {
    let pkg: String = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/package.json")).unwrap();
    let line =
        pkg.lines().find(|l| l.contains("\"@ironcalc/workbook\"")).expect("web/package.json has @ironcalc/workbook");
    assert!(line.contains(&format!("\"{IRONCALC_VERSION}\"")), "web's {line} differs from pdtd's {IRONCALC_VERSION}");
    let cargo = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    assert!(
        cargo.contains(&format!("ironcalc = \"={IRONCALC_VERSION}\"")),
        "pdt-sheet pins ironcalc {IRONCALC_VERSION}"
    );
    assert_eq!(cell_name(2, 27), "AB3");
}

#[test]
fn borders_widths_and_frozen_panes() {
    let mut wb = sales();
    wb.borders("Sheet1", "A1:D5", BorderStyle::Solid, None, true).unwrap();
    let b = wb.style("Sheet1", "B2").unwrap().border;
    assert!(b.top.is_some() && b.left.is_some(), "{b:?}");
    wb.borders("Sheet1", "A1:D5", BorderStyle::None, None, false).unwrap();
    assert!(wb.style("Sheet1", "B2").unwrap().border.top.is_none());
    assert!(wb.borders("Sheet1", "A1", BorderStyle::Solid, Some("blue"), false).is_err());

    wb.column_width("Sheet1", "B:C", 140).unwrap();
    assert!((wb.model().get_column_width(0, 3).unwrap() - 140.0).abs() < 1.0);
    assert!(wb.column_width("Sheet1", "1", 140).is_err());
    wb.freeze("Sheet1", 1, 1).unwrap();
    assert_eq!(wb.model().get_frozen_rows_count(0).unwrap(), 1);
    let back = Workbook::from_xlsx(&wb.to_xlsx().unwrap(), "x").unwrap();
    assert_eq!(back.model().get_frozen_columns_count(0).unwrap(), 1, "kept in the file");
}

#[test]
fn edits_near_the_last_row_land_where_asked_and_past_it_are_refused() {
    let mut wb = Workbook::new("Book").unwrap();
    wb.set_cells("Sheet1", "XFD1048576", &[vec!["last".into()]]).unwrap();
    assert_eq!(wb.input("Sheet1", "XFD1048576").unwrap(), "last");
    assert_eq!(wb.input("Sheet1", "A1").unwrap(), "");
    // Once wrapped to row 1 (or overflowed i32): now not a cell.
    for past in ["A4294967297", "A2147483648", "A1048577", "XFE1"] {
        assert!(wb.set_cells("Sheet1", past, &[vec!["x".into()]]).is_err(), "{past}");
    }
    assert_eq!(wb.input("Sheet1", "A1").unwrap(), "");
    // Values that run past the last row are an error, not a wrap.
    let err = wb.set_cells("Sheet1", "A1048576", &[vec!["a".into()], vec!["b".into()]]).unwrap_err();
    assert!(err.contains("past a sheet's last row"), "{err}");
    let err = wb.set_cells("Sheet1", "XFD1", &[vec!["a".into(), "b".into()]]).unwrap_err();
    assert!(err.contains("XFE"), "{err}");
}
