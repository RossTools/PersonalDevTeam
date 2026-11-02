//! PDT's local documents (PDT-315) through the API: docs as `.md` and sheets as `.xlsx` in the documents folder,
//! created, read, edited (formulas, formats, sort, filter, charts), commented on, updated live (an agent's edit reaches
//! an open view's stream; a view's IronCalc diffs reach the file), exported and imported.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{call, call_raw};
use http_body_util::BodyExt;
use pdt_api::documents::local::DocEvent;
use pdt_api::{AppState, router};
use pdt_sheet::Workbook;
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::Duration;
use tower::ServiceExt;

struct Env {
    app: axum::Router,
    state: AppState,
    tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.home_dir = Some(tmp.path().join("home"));
    let app = router(state.clone(), None);
    Env { app, state, tmp }
}

async fn create(env: &Env, body: Value) -> Value {
    let (s, d) = call(&env.app, "POST", "/api/documents", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    d
}

async fn edit(env: &Env, id: &str, edits: Value) -> (StatusCode, Value) {
    call(&env.app, "POST", &format!("/api/documents/{id}/edits"), Some(json!({"edits": edits}))).await
}

/// A raw request with a JSON-less body; the status, headers and bytes.
async fn raw(env: &Env, method: &str, uri: &str, body: Vec<u8>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let req = Request::builder().method(method).uri(uri).body(Body::from(body)).unwrap();
    let res = env.app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, res.into_body().collect().await.unwrap().to_bytes().to_vec())
}

fn next_event(rx: &mut tokio::sync::broadcast::Receiver<DocEvent>) -> DocEvent {
    rx.try_recv().expect("an event")
}

#[tokio::test]
async fn a_local_doc_is_a_markdown_file_edited_commented_on_and_exported() {
    let env = setup().await;
    let (_, providers) = call(&env.app, "GET", "/api/documents/providers", None).await;
    assert_eq!(providers["default"], "local", "{providers}");
    assert_eq!(providers["providers"][0]["capabilities"]["live"], true);

    let d =
        create(&env, json!({"title": "Launch plan", "kind": "doc", "markdown": "# Launch\n\n## Goal\n", "by": "user"}))
            .await;
    let id = d["id"].as_str().unwrap().to_string();
    let path = std::path::PathBuf::from(d["providerRef"].as_str().unwrap());
    assert_eq!(path, env.tmp.path().join("data").join("documents").join("Launch plan.md"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Launch\n\n## Goal\n");
    // A second one of the same name gets a file of its own.
    let again = create(&env, json!({"title": "Launch plan", "kind": "doc"})).await;
    assert!(again["providerRef"].as_str().unwrap().ends_with("Launch plan 2.md"), "{again}");

    let (_, c) = call(&env.app, "GET", &format!("/api/documents/{id}/content"), None).await;
    let v1 = c["content"]["version"].as_str().unwrap().to_string();
    let mut rx = env.state.local.subscribe(&path.to_string_lossy());

    // An edit made on the version it read saves; one made on an older version is refused.
    let (s, r) = call(
        &env.app,
        "POST",
        &format!("/api/documents/{id}/edits"),
        Some(json!({"edits": [{"op": "replace_text", "markdown": "# Launch\n\nShip it.\n"}], "baseVersion": v1})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(matches!(next_event(&mut rx), DocEvent::Changed { .. }));
    let (s, e) = call(
        &env.app,
        "POST",
        &format!("/api/documents/{id}/edits"),
        Some(json!({"edits": [{"op": "replace_text", "markdown": "stale"}], "baseVersion": v1})),
    )
    .await;
    assert_eq!((s, &e["code"]), (StatusCode::CONFLICT, &json!("conflict")), "{e}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Launch\n\nShip it.\n");
    // An agent appends without a version.
    let (s, _) = edit(&env, &id, json!([{"op": "append_text", "markdown": "## Notes\n"}])).await;
    assert_eq!(s, StatusCode::OK);
    assert!(matches!(next_event(&mut rx), DocEvent::Changed { .. }));
    // And replaces a section by its heading.
    let (s, r) =
        edit(&env, &id, json!([{"op": "replace_section", "heading": "Notes", "markdown": "None yet.\n"}])).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(
        std::fs::read_to_string(&path).unwrap().ends_with("## Notes\n\nNone yet.\n"),
        "{}",
        std::fs::read_to_string(&path).unwrap()
    );
    assert!(matches!(next_event(&mut rx), DocEvent::Changed { .. }));

    // The file changed outside PDT: an open view hears of it.
    std::fs::write(&path, "# Edited by hand\n").unwrap();
    env.state.local.check_external(&path.to_string_lossy());
    assert!(matches!(next_event(&mut rx), DocEvent::Changed { .. }));
    let (_, c) = call(&env.app, "GET", &format!("/api/documents/{id}/content"), None).await;
    assert_eq!(c["content"]["markdown"], "# Edited by hand\n");

    // Comments: on a heading and on some text, a reply, resolved.
    let (s, c1) = call(
        &env.app,
        "POST",
        &format!("/api/documents/{id}/comments"),
        Some(json!({"body": "Is this the goal?", "anchor": {"on": "heading", "heading": "## Goal"}, "by": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c1}");
    assert_eq!((&c1["author"], &c1["anchor"]["heading"]), (&json!("You"), &json!("Goal")));
    assert!(matches!(next_event(&mut rx), DocEvent::Comments));
    let (s, _) = call(
        &env.app,
        "POST",
        &format!("/api/documents/{id}/comments"),
        Some(json!({"body": "Typo", "anchor": {"on": "text", "quote": "by hand"}})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let cid = c1["id"].as_str().unwrap();
    let (s, r) =
        call(&env.app, "POST", &format!("/api/documents/{id}/comments/{cid}/replies"), Some(json!({"body": "Yes."})))
            .await;
    assert_eq!((s, &r["author"]), (StatusCode::CREATED, &json!("Agent")), "{r}");
    let (s, _) = call(&env.app, "POST", &format!("/api/documents/{id}/comments/{cid}/resolve"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, all) = call(&env.app, "GET", &format!("/api/documents/{id}/comments"), None).await;
    assert_eq!(all.as_array().unwrap().len(), 2, "{all}");
    assert_eq!((&all[0]["resolved"], &all[0]["replies"][0]["body"]), (&json!(true), &json!("Yes.")));
    let (s, _) = call(&env.app, "POST", &format!("/api/documents/{id}/comments/{cid}/reopen"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, all) = call(&env.app, "GET", &format!("/api/documents/{id}/comments"), None).await;
    assert_eq!(all[0]["resolved"], false);
    assert_eq!(all[1]["quote"], "“by hand”");
    let (s, _) = call(&env.app, "POST", &format!("/api/documents/{id}/comments"), Some(json!({"body": " "}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // An image goes beside the doc, and is served from where its Markdown points.
    let (s, _, img) =
        call_raw(&env.app, "POST", &format!("/api/documents/{id}/images?name=shot.png"), b"\x89PNG".to_vec()).await;
    assert_eq!(s, StatusCode::CREATED);
    let img: Value = serde_json::from_slice(&img).unwrap();
    assert_eq!(img["path"], "Launch%20plan.assets/shot.png");
    assert!(path.with_file_name("Launch plan.assets").join("shot.png").is_file());
    let (s, ct, bytes) =
        call_raw(&env.app, "GET", &format!("/api/documents/{id}/assets/Launch%20plan.assets/shot.png"), vec![]).await;
    assert_eq!((s, ct.as_deref(), bytes.as_slice()), (StatusCode::OK, Some("image/png"), &b"\x89PNG"[..]));
    let (s, _, _) = call_raw(&env.app, "GET", &format!("/api/documents/{id}/assets/..%2F..%2Fpdt.db"), vec![]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = call_raw(&env.app, "POST", &format!("/api/documents/{id}/images?name=x.exe"), b"MZ".to_vec()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Export as .md.
    let (s, h, bytes) = raw(&env, "GET", &format!("/api/documents/{id}/export?format=md"), vec![]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(bytes, b"# Edited by hand\n");
    assert!(h["content-disposition"].to_str().unwrap().contains("Launch plan.md"), "{h:?}");
    let (s, _, _) = raw(&env, "GET", &format!("/api/documents/{id}/export?format=xlsx"), vec![]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // The local provider finds its files to import, and deleting removes the file and its images.
    let (_, files) = call(&env.app, "GET", "/api/documents/providers/local/files?q=launch", None).await;
    assert_eq!(files.as_array().unwrap().len(), 2, "{files}");
    // A file dropped into the folder can be imported; one elsewhere on the machine can't.
    let dropped = path.with_file_name("Dropped in.md");
    std::fs::write(&dropped, "# Hi\n").unwrap();
    let (s, d) = call(
        &env.app,
        "POST",
        "/api/documents/import",
        Some(json!({"provider": "local", "reference": dropped.to_string_lossy()})),
    )
    .await;
    assert_eq!((s, &d["title"]), (StatusCode::CREATED, &json!("Dropped in")), "{d}");
    let outside = env.tmp.path().join("elsewhere.md");
    std::fs::write(&outside, "secret").unwrap();
    let (s, _) = call(
        &env.app,
        "POST",
        "/api/documents/import",
        Some(json!({"provider": "local", "reference": outside.to_string_lossy()})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "DELETE", &format!("/api/documents/{id}?file=true"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(!path.exists() && !path.with_file_name("Launch plan.assets").exists());
}

#[tokio::test]
async fn a_local_sheet_computes_formats_sorts_filters_charts_and_exports() {
    let env = setup().await;
    let d = create(&env, json!({"title": "Budget", "kind": "sheet"})).await;
    let id = d["id"].as_str().unwrap().to_string();
    let reference = d["providerRef"].as_str().unwrap().to_string();
    assert!(reference.ends_with("Budget.xlsx"), "{d}");

    let (s, r) = edit(
        &env,
        &id,
        json!([
            {"op": "set_cells", "sheet": "Sheet1", "cell": "A1", "values": [
                ["Item", "Cost"], ["Pens", "3"], ["Ink", "12"], ["Paper", "7"], ["Total", "=SUM(B2:B4)"]]},
            {"op": "format_range", "sheet": "Sheet1", "range": "A1:B1", "format": {"bold": true}},
            {"op": "format_range", "sheet": "Sheet1", "range": "B2:B5", "format": {"numberFormat": "#,##0.00"}},
            {"op": "conditional_format", "sheet": "Sheet1", "range": "B2:B4", "when": {"type": "greater_than", "value": "10"},
                "format": {"background": "#ffeeee"}},
            {"op": "add_sheet", "name": "Notes"},
            {"op": "add_chart", "sheet": "Sheet1", "chart": {"kind": "column", "title": "Costs", "data": "A1:B4", "anchor": "D2"}}
        ]),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let chart_id = r["charts"][0].as_str().unwrap().to_string();
    let (_, c) = call(&env.app, "GET", &format!("/api/documents/{id}/content"), None).await;
    assert_eq!(c["content"]["sheets"][0]["charts"][0]["id"], json!(chart_id), "the sheet lists its chart: {c}");
    let rows = &c["content"]["sheets"][0]["rows"];
    assert_eq!((&rows[4][1]["value"], &rows[4][1]["formula"]), (&json!("22.00"), &json!("=SUM(B2:B4)")), "{c}");
    assert_eq!(c["content"]["sheets"][1]["name"], "Notes");
    let back = Workbook::open(std::path::Path::new(&reference)).unwrap();
    assert!(back.style("Sheet1", "A1").unwrap().font.b, "formats are saved in the file");
    assert_eq!(back.model().get_conditional_formatting_list(0).unwrap().len(), 1);

    let (_, charts) = call(&env.app, "GET", &format!("/api/documents/{id}/charts"), None).await;
    assert_eq!(charts[0]["labels"], json!(["Pens", "Ink", "Paper"]), "{charts}");
    assert_eq!(charts[0]["series"], json!([{"name": "Cost", "values": [3.0, 12.0, 7.0]}]));
    assert_eq!(charts[0]["sheet"], "Sheet1");

    // Sort by cost, highest first; the total still adds them up.
    let (s, _) = edit(
        &env,
        &id,
        json!([{"op": "sort", "sheet": "Sheet1", "range": "A1:B4", "by": [{"column": "B", "descending": true}], "header": true}]),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, charts) = call(&env.app, "GET", &format!("/api/documents/{id}/charts"), None).await;
    assert_eq!(charts[0]["labels"], json!(["Ink", "Paper", "Pens"]));
    // Filter to one row, then show them all.
    let (s, _) = edit(
        &env,
        &id,
        json!([{"op": "filter", "sheet": "Sheet1", "range": "A1:B4", "column": "A", "when": {"type": "equal_to", "value": "ink"}}]),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let wb = Workbook::open(std::path::Path::new(&reference)).unwrap();
    assert_eq!((2..=4).map(|r| wb.row_hidden("Sheet1", r).unwrap()).collect::<Vec<_>>(), [false, true, true]);
    let (s, _) = edit(&env, &id, json!([{"op": "clear_filter", "sheet": "Sheet1"}])).await;
    assert_eq!(s, StatusCode::OK);
    // Charts follow a renamed sheet; an unknown chart is refused.
    let (s, _) = edit(&env, &id, json!([{"op": "rename_sheet", "sheet": "Sheet1", "name": "Costs"}])).await;
    assert_eq!(s, StatusCode::OK);
    let (_, charts) = call(&env.app, "GET", &format!("/api/documents/{id}/charts"), None).await;
    assert_eq!((&charts[0]["sheet"], &charts[0]["labels"][0]), (&json!("Costs"), &json!("Ink")), "{charts}");
    let (s, _) = edit(&env, &id, json!([{"op": "delete_chart", "id": "nope"}])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    edit(&env, &id, json!([{"op": "rename_sheet", "sheet": "Costs", "name": "Sheet1"}])).await;
    // A deleted sheet's charts go with it, and don't come back with a new sheet of its name.
    let (s, r) = edit(
        &env,
        &id,
        json!([{"op": "add_chart", "sheet": "Notes", "chart": {"kind": "line", "data": "A1:B2"}},
               {"op": "delete_sheet", "sheet": "Notes"}, {"op": "add_sheet", "name": "Notes"}]),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (_, charts) = call(&env.app, "GET", &format!("/api/documents/{id}/charts"), None).await;
    assert_eq!(charts.as_array().unwrap().len(), 1, "only Sheet1's chart: {charts}");

    // A bad edit changes nothing (the list is all or none).
    let (s, e) = edit(
        &env,
        &id,
        json!([
        {"op": "set_cells", "sheet": "Sheet1", "cell": "A9", "values": [["half"]]},
        {"op": "set_cells", "sheet": "Nope", "cell": "A1", "values": [["x"]]}]),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    let (_, c) = call(&env.app, "GET", &format!("/api/documents/{id}/content"), None).await;
    assert_eq!(c["content"]["sheets"][0]["rows"].as_array().unwrap().len(), 5, "{c}");
    // Borders, column widths and frozen panes; data validation isn't there yet.
    let (s, r) = edit(
        &env,
        &id,
        json!([{"op": "set_borders", "sheet": "Sheet1", "range": "A1:B5", "inner": true},
               {"op": "column_width", "sheet": "Sheet1", "columns": "A", "width": 160},
               {"op": "freeze", "sheet": "Sheet1", "rows": 1}]),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let wb = Workbook::open(std::path::Path::new(&reference)).unwrap();
    assert_eq!(wb.model().get_frozen_rows_count(0).unwrap(), 1);
    let (s, e) =
        edit(&env, &id, json!([{"op": "validate", "sheet": "Sheet1", "range": "B2:B4", "rule": {"type": "checkbox"}}]))
            .await;
    assert_eq!((s, &e["code"]), (StatusCode::BAD_REQUEST, &json!("unsupported")), "{e}");
    let (s, e) = edit(&env, &id, json!([{"op": "replace_text", "markdown": "x"}])).await;
    assert_eq!((s, &e["code"]), (StatusCode::BAD_REQUEST, &json!("unsupported")), "{e}");

    // Export: the workbook, and a sheet as CSV.
    let (s, h, bytes) = raw(&env, "GET", &format!("/api/documents/{id}/export?format=xlsx"), vec![]).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h["content-type"].to_str().unwrap().contains("spreadsheetml"));
    assert_eq!(Workbook::from_xlsx(&bytes, "x").unwrap().sheet_names(), vec!["Sheet1", "Notes"]);
    let (_, _, csv) = raw(&env, "GET", &format!("/api/documents/{id}/export?format=csv"), vec![]).await;
    assert_eq!(String::from_utf8(csv).unwrap(), "Item,Cost\nInk,12.00\nPaper,7.00\nPens,3.00\nTotal,22.00\n");
    let (_, h, _) = raw(&env, "GET", &format!("/api/documents/{id}/export?format=csv&sheet=Notes"), vec![]).await;
    assert!(h["content-disposition"].to_str().unwrap().contains("Budget - Notes.csv"), "{h:?}");

    let (s, _) = edit(&env, &id, json!([{"op": "delete_chart", "id": chart_id}])).await;
    assert_eq!(s, StatusCode::OK);
    let (_, charts) = call(&env.app, "GET", &format!("/api/documents/{id}/charts"), None).await;
    assert_eq!(charts, json!([]));
}

#[tokio::test]
async fn an_open_sheet_is_live_both_ways() {
    let env = setup().await;
    let d = create(&env, json!({"title": "Live", "kind": "sheet"})).await;
    let id = d["id"].as_str().unwrap().to_string();
    let reference = d["providerRef"].as_str().unwrap().to_string();

    // A view loads the model…
    let (s, h, bytes) = raw(&env, "GET", &format!("/api/documents/{id}/sheet"), vec![]).await;
    assert_eq!(s, StatusCode::OK);
    let mut seq: u64 = h["x-sheet-seq"].to_str().unwrap().parse().unwrap();
    let mut view = Workbook::from_bytes(&bytes).unwrap();
    let mut rx = env.state.local.subscribe(&reference);

    // …and sends its edits as diffs: they reach the file, and the other views (not itself, on its stream).
    view.set_cells("Sheet1", "A1", &[vec!["2".into(), "=A1*10".into()]]).unwrap();
    let diffs = view.flush();
    let (s, _, r) = raw(&env, "POST", &format!("/api/documents/{id}/sheet/diffs?view=v1&seq={seq}"), diffs).await;
    assert_eq!(s, StatusCode::OK);
    let r: Value = serde_json::from_slice(&r).unwrap();
    assert_eq!(r["conflict"], false);
    seq = r["seq"].as_u64().unwrap();
    assert!(matches!(next_event(&mut rx), DocEvent::Diffs { from, .. } if from == "v1"));
    let saved = Workbook::open(std::path::Path::new(&reference)).unwrap();
    assert_eq!(saved.shown("Sheet1", "B1").unwrap(), vec![vec!["20".to_string()]]);

    // An agent's edit reaches the view as diffs, numbered next.
    let (s, _) =
        edit(&env, &id, json!([{"op": "set_cells", "sheet": "Sheet1", "cell": "A1", "values": [["5"]]}])).await;
    assert_eq!(s, StatusCode::OK);
    match next_event(&mut rx) {
        DocEvent::Diffs { seq: n, from, data } => {
            assert_eq!((n, from.as_str()), (seq + 1, "pdt"));
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data).unwrap();
            view.apply_diffs(&bytes).unwrap();
            assert_eq!(view.shown("Sheet1", "B1").unwrap(), vec![vec!["50".to_string()]]);
        }
        e => panic!("{e:?}"),
    }

    // A view that sends edits made before that change: applied (last write wins), and told to reload.
    view.set_cells("Sheet1", "C1", &[vec!["late".into()]]).unwrap();
    let (s, _, r) =
        raw(&env, "POST", &format!("/api/documents/{id}/sheet/diffs?view=v1&seq={seq}"), view.flush()).await;
    assert_eq!(s, StatusCode::OK);
    let r: Value = serde_json::from_slice(&r).unwrap();
    assert_eq!(r["conflict"], true, "{r}");
    let saved = Workbook::open(std::path::Path::new(&reference)).unwrap();
    assert_eq!(saved.shown("Sheet1", "A1:C1").unwrap(), vec![vec!["5".to_string(), "50".into(), "late".into()]]);
    let (s, _, _) =
        raw(&env, "POST", &format!("/api/documents/{id}/sheet/diffs?view=v1&seq=0"), b"junk".to_vec()).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // PDT's own saves are not changes from outside; a write by another program reloads the open sheet.
    while rx.try_recv().is_ok() {}
    env.state.local.check_external(&reference);
    assert!(rx.try_recv().is_err(), "its own save is not news");
    let mut outside = Workbook::open(std::path::Path::new(&reference)).unwrap();
    outside.set_cells("Sheet1", "E1", &[vec!["by hand".into()]]).unwrap();
    outside.save(std::path::Path::new(&reference)).unwrap();
    env.state.local.check_external(&reference);
    assert!(matches!(next_event(&mut rx), DocEvent::Reload { .. }));
    let (_, c) = call(&env.app, "GET", &format!("/api/documents/{id}/content"), None).await;
    assert_eq!(c["content"]["sheets"][0]["rows"][0][4]["value"], "by hand", "{c}");

    // The stream as a browser reads it: an agent's change arrives as a `diffs` event.
    let req = Request::builder().uri(format!("/api/documents/{id}/events?view=v2")).body(Body::empty()).unwrap();
    let res = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body();
    let (s, _) =
        edit(&env, &id, json!([{"op": "set_cells", "sheet": "Sheet1", "cell": "D1", "values": [["7"]]}])).await;
    assert_eq!(s, StatusCode::OK);
    let mut got = String::new();
    while !got.contains("event: diffs") {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame()).await.expect("an event in time");
        if let Some(Ok(f)) = frame
            && let Some(d) = f.data_ref()
        {
            got.push_str(&String::from_utf8_lossy(d));
        }
    }
    assert!(got.contains("\"from\":\"pdt\""), "{got}");
}

#[tokio::test]
async fn files_from_the_computer_become_local_documents() {
    let env = setup().await;
    let (s, _, r) = call_raw(
        &env.app,
        "POST",
        "/api/documents/upload?name=Stock.csv&by=user",
        b"Item,Qty\nPens,3\nInk,=B2*2\n".to_vec(),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{}", String::from_utf8_lossy(&r));
    let r: Value = serde_json::from_slice(&r).unwrap();
    assert_eq!((&r["document"]["kind"], &r["document"]["title"]), (&json!("sheet"), &json!("Stock")));
    let id = r["document"]["id"].as_str().unwrap();
    let (_, c) = call(&env.app, "GET", &format!("/api/documents/{id}/content"), None).await;
    assert_eq!(c["content"]["sheets"][0]["rows"][2][1]["value"], "6");

    // An .xlsx with a chart: imported, and the chart named as dropped.
    let wb = Workbook::from_csv("a,b\n1,2\n", "x", "Data").unwrap();
    let xlsx = wb.to_xlsx().unwrap();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(xlsx)).unwrap();
    for i in 0..src.len() {
        zip.raw_copy_file(src.by_index_raw(i).unwrap()).unwrap();
    }
    zip.start_file("xl/charts/chart1.xml", zip::write::SimpleFileOptions::default()).unwrap();
    let with_chart = zip.finish().unwrap().into_inner();
    let (s, _, r) = call_raw(&env.app, "POST", "/api/documents/upload?name=Report.xlsx", with_chart).await;
    assert_eq!(s, StatusCode::CREATED, "{}", String::from_utf8_lossy(&r));
    let r: Value = serde_json::from_slice(&r).unwrap();
    assert_eq!(r["dropped"], json!(["charts"]));

    let (s, _, r) = call_raw(&env.app, "POST", "/api/documents/upload?name=Notes.md", b"# Notes\n".to_vec()).await;
    assert_eq!(s, StatusCode::CREATED);
    let r: Value = serde_json::from_slice(&r).unwrap();
    assert_eq!(r["document"]["kind"], "doc");
    let (s, _, _) = call_raw(&env.app, "POST", "/api/documents/upload?name=x.pdf", b"%PDF".to_vec()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call_raw(&env.app, "POST", "/api/documents/upload?name=Bad.xlsx", b"nope".to_vec()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
