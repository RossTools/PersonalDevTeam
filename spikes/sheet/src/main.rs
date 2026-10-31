//! PDT-313 throwaway prototype: the daemon side of a local sheet, on IronCalc.
//!
//! - `seed <file.xlsx>`: writes a sample workbook (values, formulas, formatting, two sheets) and its chart spec
//!   (`<file>.charts.json`, PDT's own: IronCalc has no charts).
//! - `read <file.xlsx> [Sheet!A1:C9]`: what an agent's `sheet_read` returns, without a browser: per cell the input
//!   (value or formula) and the computed, formatted value.
//! - `set <file.xlsx> <Sheet!B2> <input>`: an agent's `sheet_edit` without a browser: load, edit, recalculate, save.
//! - `serve <file.xlsx> [port]`: what pdtd would do for an open sheet. It holds the one live model; the browser loads it
//!   (`GET /spike/sheet`, IronCalc's bytes), sends its edits as IronCalc diffs (`POST /spike/diffs`), and gets every
//!   other edit (an agent's, `POST /spike/agent-edit`) as diffs on an event stream (`GET /spike/events`). Each edit
//!   saves the .xlsx.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use base64::Engine;
use futures_util::Stream;
use ironcalc::{
    base::{UserModel, expressions::types::Area},
    export::save_xlsx_to_writer,
    import::load_from_xlsx,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

type Model = UserModel<'static>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["seed", file] => seed(Path::new(file)),
        ["read", file] => read(Path::new(file), None),
        ["read", file, range] => read(Path::new(file), Some(range)),
        ["set", file, cell, input] => set(Path::new(file), cell, input),
        ["serve", file] => serve(PathBuf::from(file), 7499),
        ["serve", file, port] => serve(PathBuf::from(file), port.parse()?),
        _ => Err("usage: seed <f.xlsx> | read <f.xlsx> [Sheet!A1:C9] | set <f.xlsx> <Sheet!B2> <input> | serve <f.xlsx> [port]".into()),
    }
}

// ---- files ----

fn load(file: &Path) -> Result<Model, String> {
    let model = load_from_xlsx(&file.to_string_lossy(), "en", "UTC", "en").map_err(|e| e.to_string())?;
    let mut model = UserModel::from_model(model);
    // Formulas are recalculated on load: the daemon never trusts the cached values a file brings.
    model.evaluate();
    Ok(model)
}

/// Writes beside the file and renames over it, so a reader never sees half a file.
fn save(model: &Model, file: &Path) -> Result<(), String> {
    let tmp = file.with_extension("xlsx.tmp");
    let out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    save_xlsx_to_writer(model.get_model(), out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, file).map_err(|e| e.to_string())
}

fn charts_path(file: &Path) -> PathBuf {
    file.with_extension("charts.json")
}

/// PDT's chart spec (IronCalc has none): what to draw, from which ranges, anchored at a cell.
#[derive(Serialize, Deserialize, Clone)]
struct Chart {
    id: String,
    kind: String, // bar | line | pie | scatter
    title: String,
    sheet: String,
    categories: String, // A2:A5
    series: Vec<Series>,
    anchor: String, // the top-left cell it floats over, e.g. E2
}

#[derive(Serialize, Deserialize, Clone)]
struct Series {
    name: String,
    values: String, // B2:B5
}

fn seed(file: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut m: Model = UserModel::new_empty("Quarterly sales", "en", "UTC", "en")?;
    m.rename_sheet(0, "Sales")?;
    let rows = [
        ["Quarter", "Revenue", "Cost", "Margin"],
        ["Q1", "1200", "800", "=B2-C2"],
        ["Q2", "1500", "900", "=B3-C3"],
        ["Q3", "1700", "1000", "=B4-C4"],
        ["Q4", "2100", "1150", "=B5-C5"],
        ["Total", "=SUM(B2:B5)", "=SUM(C2:C5)", "=SUM(D2:D5)"],
    ];
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            m.set_user_input(0, r as i32 + 1, c as i32 + 1, v)?;
        }
    }
    m.set_user_input(0, 8, 1, "Margin %")?;
    m.set_user_input(0, 8, 2, "=D6/B6")?;
    let area = |row, column, width, height| Area { sheet: 0, row, column, width, height };
    m.update_range_style(&area(1, 1, 4, 1), "font.b", "true")?;
    m.update_range_style(&area(1, 1, 4, 1), "fill.color", "#DCE6F2")?;
    m.update_range_style(&area(6, 1, 4, 1), "font.b", "true")?;
    m.update_range_style(&area(2, 2, 3, 5), "num_fmt", "#,##0")?;
    m.update_range_style(&area(8, 2, 1, 1), "num_fmt", "0.0%")?;
    m.new_sheet()?;
    m.rename_sheet(1, "Notes")?;
    m.set_user_input(1, 1, 1, "Margin from Sales")?;
    m.set_user_input(1, 1, 2, "=Sales!D6")?;
    save(&m, file)?;
    let charts = vec![Chart {
        id: "c1".into(),
        kind: "bar".into(),
        title: "Revenue and cost by quarter".into(),
        sheet: "Sales".into(),
        categories: "A2:A5".into(),
        series: vec![
            Series { name: "Revenue".into(), values: "B2:B5".into() },
            Series { name: "Cost".into(), values: "C2:C5".into() },
        ],
        anchor: "F2".into(),
    }];
    std::fs::write(charts_path(file), serde_json::to_string_pretty(&charts)?)?;
    println!("wrote {} and {}", file.display(), charts_path(file).display());
    Ok(())
}

// ---- cell references ----

/// `B12` → (row 12, column 2).
fn cell(s: &str) -> Result<(i32, i32), String> {
    let split = s.find(|c: char| c.is_ascii_digit()).ok_or(format!("bad cell {s}"))?;
    let (letters, digits) = s.split_at(split);
    let column = letters.to_ascii_uppercase().bytes().try_fold(0i32, |n, b| {
        b.is_ascii_uppercase().then(|| n * 26 + (b - b'A' + 1) as i32).ok_or(format!("bad cell {s}"))
    })?;
    Ok((digits.parse().map_err(|_| format!("bad cell {s}"))?, column))
}

/// `Sales!B2` / `Sales!A1:C9` → (sheet index, first cell, last cell).
fn reference(m: &Model, s: &str) -> Result<(u32, (i32, i32), (i32, i32)), String> {
    let (sheet, cells) = s.split_once('!').unwrap_or(("", s));
    let sheets = m.get_worksheets_properties();
    let index = if sheet.is_empty() { 0 } else {
        sheets.iter().position(|p| p.name == sheet).ok_or(format!("no sheet {sheet}"))? as u32
    };
    let (a, b) = cells.split_once(':').unwrap_or((cells, cells));
    Ok((index, cell(a)?, cell(b)?))
}

fn column_name(mut c: i32) -> String {
    let mut s = Vec::new();
    while c > 0 {
        s.push(b'A' + ((c - 1) % 26) as u8);
        c = (c - 1) / 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

// ---- headless agent tools ----

fn read(file: &Path, range: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let m = load(file)?;
    for (i, p) in m.get_worksheets_properties().iter().enumerate() {
        let r = format!("{}!{}", p.name, range.and_then(|r| r.split_once('!').map(|x| x.1).or(Some(r))).unwrap_or("A1:D8"));
        if range.is_some_and(|r| r.contains('!') && !r.starts_with(&format!("{}!", p.name))) {
            continue;
        }
        let (sheet, (r1, c1), (r2, c2)) = reference(&m, &r)?;
        println!("== {} (sheet {i})", p.name);
        for row in r1..=r2 {
            for col in c1..=c2 {
                let input = m.get_cell_content(sheet, row, col)?;
                if input.is_empty() {
                    continue;
                }
                let value = m.get_formatted_cell_value(sheet, row, col)?;
                let style = m.get_cell_style(sheet, row, col)?;
                let fmt = if style.num_fmt == "general" { String::new() } else { format!("  [{}]", style.num_fmt) };
                let bold = if style.font.b { "  [bold]" } else { "" };
                if input.starts_with('=') {
                    println!("{}{row}: {input} → {value}{fmt}{bold}", column_name(col));
                } else {
                    println!("{}{row}: {value}{fmt}{bold}", column_name(col));
                }
            }
        }
    }
    if let Ok(charts) = std::fs::read_to_string(charts_path(file)) {
        println!("== charts\n{charts}");
    }
    Ok(())
}

fn set(file: &Path, at: &str, input: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut m = load(file)?;
    let (sheet, (row, col), _) = reference(&m, at)?;
    m.set_user_input(sheet, row, col, input)?;
    save(&m, file)?;
    println!("{at} = {input} → {}", m.get_formatted_cell_value(sheet, row, col)?);
    Ok(())
}

// ---- the live sheet ----

struct Live {
    file: PathBuf,
    model: Mutex<Model>,
    /// Diffs as they land, tagged with who made them so a view doesn't get its own edits back.
    diffs: broadcast::Sender<(String, Vec<u8>)>,
}

fn serve(file: PathBuf, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let model = load(&file)?;
    let live = Arc::new(Live { file, model: Mutex::new(model), diffs: broadcast::channel(256).0 });
    let app = Router::new()
        .route("/spike/sheet", get(sheet_bytes))
        .route("/spike/charts", get(charts))
        .route("/spike/events", get(events))
        .route("/spike/diffs", post(view_diffs))
        .route("/spike/agent-edit", post(agent_edit))
        .with_state(live);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("serving on http://{addr}/spike");
    tokio::runtime::Runtime::new()?.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await
    })?;
    Ok(())
}

fn err(e: impl ToString) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.to_string())
}

async fn sheet_bytes(State(live): State<Arc<Live>>) -> impl IntoResponse {
    live.model.lock().unwrap().to_bytes()
}

async fn charts(State(live): State<Arc<Live>>) -> impl IntoResponse {
    std::fs::read_to_string(charts_path(&live.file)).unwrap_or_else(|_| "[]".into())
}

#[derive(Deserialize)]
struct Since {
    view: Option<String>,
}

async fn events(
    State(live): State<Arc<Live>>,
    axum::extract::Query(q): axum::extract::Query<Since>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let me = q.view.unwrap_or_default();
    let stream = BroadcastStream::new(live.diffs.subscribe()).filter_map(move |d| {
        let (from, bytes) = d.ok()?;
        (from != me).then(|| Ok(Event::default().event("diffs").data(base64::engine::general_purpose::STANDARD.encode(bytes))))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// A view's own edits (IronCalc's `flushSendQueue()`), applied to the daemon's model and passed on to the other views.
async fn view_diffs(
    State(live): State<Arc<Live>>,
    axum::extract::Query(q): axum::extract::Query<Since>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut m = live.model.lock().unwrap();
    m.apply_external_diffs(&body).map_err(err)?;
    save(&m, &live.file).map_err(err)?;
    let _ = live.diffs.send((q.view.unwrap_or_default(), body.to_vec()));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AgentEdit {
    /// `Sales!B2`
    at: String,
    input: String,
}

/// An agent's `sheet_edit`: edit the daemon's model, save, and send the diffs to every open view.
async fn agent_edit(
    State(live): State<Arc<Live>>,
    Json(edits): Json<Vec<AgentEdit>>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    let mut m = live.model.lock().unwrap();
    let mut out = Vec::new();
    for e in &edits {
        let (sheet, (row, col), _) = reference(&m, &e.at).map_err(err)?;
        m.set_user_input(sheet, row, col, &e.input).map_err(err)?;
        out.push(format!("{} = {} → {}", e.at, e.input, m.get_formatted_cell_value(sheet, row, col).map_err(err)?));
    }
    save(&m, &live.file).map_err(err)?;
    let _ = live.diffs.send(("agent".into(), m.flush_send_queue()));
    Ok(Json(out))
}
