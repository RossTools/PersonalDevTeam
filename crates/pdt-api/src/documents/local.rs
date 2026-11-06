//! PDT's local document provider (PDT-315): documents as files in the local documents folder (the project's, else the
//! host's, else `documents` in PDT's data folder), a doc as a `.md` and a sheet as a `.xlsx` (IronCalc, `pdt_sheet`),
//! with PDT's record, comments and charts in the store. Its reference to a file is the file's full path.
//!
//! An open sheet is live: pdtd holds one model per sheet ([`LocalProvider::live`]), which views load
//! ([`LocalProvider::sheet_bytes`]) and send their edits to as IronCalc diffs ([`LocalProvider::view_diffs`]); agents'
//! edits ([`DocumentProvider::apply`]) change the same model. Every change goes to the document's event stream
//! ([`LocalProvider::subscribe`]), numbered, so each view applies them in order and reloads when it missed one. A file
//! changed outside PDT is noticed when a view's stream next checks ([`LocalProvider::check_external`]).

use super::{CommentInput, DocumentProvider, NewFile, ProviderError, ProviderResult};
use crate::workflow::catalog::BoxFut;
use pdt_core::document::{
    Chart, ChartSeriesView, ChartView, CommentReply, DocumentComment, DocumentContent, DocumentEdit, DocumentKind,
    ProviderCapabilities, ProviderFile, cell_name, documents_folder, parse_range,
};
use pdt_core::document_ops::{EditError, apply_doc_edit};
use pdt_core::ticket::Actor;
use pdt_sheet::{Value, Workbook};
use pdt_store::{NewDocumentComment, Store};
use serde::Serialize;
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::broadcast;

pub const ID: &str = "local";

/// A live sheet nobody has used for this long, with no view open, is dropped (the file has every change already).
const IDLE: Duration = Duration::from_secs(600);

/// The largest file PDT imports or a doc's image (bytes).
pub const MAX_FILE: usize = 50 * 1024 * 1024;

/// What changed in a document, as its event stream sends it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocEvent {
    /// A sheet's change: IronCalc diffs (base64), numbered; `from` is the view that made it (`pdt` for an agent's or
    /// PDT's own edit).
    Diffs { seq: u64, from: String, data: String },
    /// A sheet's model was replaced (its file changed outside PDT, or diffs could not be applied in order): load it
    /// again.
    Reload { seq: u64 },
    /// A doc's text changed (saved by someone else, an agent, or outside PDT).
    Changed { version: String },
    /// Its comments changed.
    Comments,
    /// A sheet's charts changed.
    Charts,
}

impl DocEvent {
    pub fn name(&self) -> &'static str {
        match self {
            DocEvent::Diffs { .. } => "diffs",
            DocEvent::Reload { .. } => "reload",
            DocEvent::Changed { .. } => "changed",
            DocEvent::Comments => "comments",
            DocEvent::Charts => "charts",
        }
    }
}

struct Live {
    wb: Workbook,
    /// The number of the last change (views send the one they have).
    seq: u64,
    used: Instant,
}

/// What a view's diffs did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffsApplied {
    /// The change's number.
    #[ts(type = "number")]
    pub seq: u64,
    /// Someone else changed the sheet since the view's last change: it should load the sheet again (it has both
    /// changes) and say so.
    pub conflict: bool,
}

/// A file to export.
pub struct Export {
    pub bytes: Vec<u8>,
    pub file_name: String,
    pub mime: &'static str,
}

pub struct LocalProvider {
    store: Store,
    data_dir: PathBuf,
    home: Option<PathBuf>,
    files: Arc<Files>,
}

/// The provider's files and live sheets: everything that reads, parses or writes a file, called off the runtime's
/// threads ([`LocalProvider::blocking`]).
#[derive(Default)]
struct Files {
    /// One slot per live sheet, filled when it is first used: the map's lock is only held to find the slot, so
    /// loading one big workbook holds up that sheet alone.
    live: Mutex<HashMap<String, Arc<Mutex<Option<Live>>>>>,
    events: Mutex<HashMap<String, broadcast::Sender<DocEvent>>>,
    /// Each file's version as PDT last wrote or read it, to notice changes made outside PDT.
    known: Mutex<HashMap<String, Seen>>,
    /// Per doc: held while its file is read and remembered or written, so those don't interleave.
    doc_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

/// A file's version as PDT last saw it, and its modified time and length then.
struct Seen {
    version: String,
    stamp: Option<Stamp>,
}

type Stamp = (SystemTime, u64);

/// A file modified this recently may change again within its file system's clock tick without its stamp moving, so
/// its bytes are read all the same.
const SETTLED: Duration = Duration::from_secs(3);

fn stamp_of(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Whether a stamp can stand for the file's bytes: it is old enough that a later write would move it.
fn settled(stamp: &Stamp) -> bool {
    SystemTime::now().duration_since(stamp.0).is_ok_and(|age| age >= SETTLED)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn failed(e: impl std::fmt::Display) -> ProviderError {
    ProviderError::Failed(e.to_string())
}

/// A file read's failure: [`ProviderError::NotFound`] only when the file is missing (PDT-524).
fn io_failed(e: std::io::Error) -> ProviderError {
    if e.kind() == std::io::ErrorKind::NotFound {
        return ProviderError::NotFound;
    }
    tracing::warn!(error = %e, "could not read a local document's file");
    ProviderError::Failed("could not read the file (PDT's log says why)".into())
}

/// The file a reference names, checked to be a doc or sheet that exists.
fn path_of(reference: &str) -> ProviderResult<(PathBuf, DocumentKind)> {
    let p = PathBuf::from(reference);
    let kind = kind_of(&p).ok_or(ProviderError::NotFound)?;
    if !p.is_absolute() || !p.is_file() {
        return Err(ProviderError::NotFound);
    }
    Ok((p, kind))
}

fn sheet_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A file's version: a hash of its bytes.
fn version_of(bytes: &[u8]) -> String {
    hex::encode(&sha2::Sha256::digest(bytes)[..8])
}

/// How many folders down the import search looks.
const SEARCH_DEPTH: usize = 4;
/// How many files and folders the import search looks at, at most.
const SEARCH_ENTRIES: usize = 20_000;

fn kind_of(path: &Path) -> Option<DocumentKind> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "md" | "markdown" => Some(DocumentKind::Doc),
        "xlsx" => Some(DocumentKind::Sheet),
        _ => None,
    }
}

/// A file name from a title: letters, digits, spaces, `-` and `_` (others become `-`), at most 80 characters.
pub fn file_stem(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '-' })
        .take(80)
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_matches(|c| c == '-' || c == ' ').to_string();
    if s.is_empty() { "Untitled".into() } else { s }
}

/// A path in `folder` for a new file named after `stem` that doesn't exist yet (`Plan.md`, `Plan 2.md`, …).
fn free_path(folder: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} {n}.{ext}") };
        let p = folder.join(name);
        if !p.exists() && !assets_dir(&p).exists() {
            return p;
        }
        n += 1;
    }
}

/// Where a doc's images go: `<name>.assets/` beside it.
fn assets_dir(doc: &Path) -> PathBuf {
    let stem = doc.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    doc.with_file_name(format!("{stem}.assets"))
}

/// Writes beside the file, then renames over it, so a reader never sees half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Text(t) => t.trim().replace(',', "").parse().ok(),
        Value::Empty => None,
    }
}

/// Marks an error from a cell edit that the local provider can't do at all.
const UNSUPPORTED: &str = "unsupported: ";

fn edit_error(e: EditError) -> ProviderError {
    match e {
        EditError::Unsupported(m) => ProviderError::Unsupported(m),
        EditError::Invalid(m) => ProviderError::Invalid(m),
    }
}

/// A chart with its numbers: its data range's first column (under the header row) labels it, and each other column
/// is a series named by its header.
fn chart_view(wb: &Workbook, sheet: String, chart: Chart) -> ChartView {
    let mut labels = Vec::new();
    let mut series = Vec::new();
    if let Some(((r1, c1), (r2, c2))) = parse_range(&chart.spec.data) {
        let range = |a: (usize, usize), b: (usize, usize)| format!("{}:{}", cell_name(a.0, a.1), cell_name(b.0, b.1));
        let header =
            wb.shown(&sheet, &range((r1, c1), (r1, c2))).ok().and_then(|r| r.into_iter().next()).unwrap_or_default();
        if r2 > r1 {
            labels = wb
                .shown(&sheet, &range((r1 + 1, c1), (r2, c1)))
                .map(|rows| rows.into_iter().flatten().collect())
                .unwrap_or_default();
            for c in c1 + 1..=c2 {
                let values = wb
                    .values(&sheet, &range((r1 + 1, c), (r2, c)))
                    .map(|rows| rows.iter().flatten().map(number).collect())
                    .unwrap_or_default();
                let name = header.get(c - c1).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| cell_name(r1, c));
                series.push(ChartSeriesView { name, values });
            }
        }
    }
    ChartView { sheet, chart, labels, series }
}

impl Files {
    /// A document's event sender, made when it has none; senders nobody listens to any more are let go.
    fn sender(&self, reference: &str) -> broadcast::Sender<DocEvent> {
        let mut all = lock(&self.events);
        all.retain(|r, tx| r == reference || tx.receiver_count() > 0);
        all.entry(reference.to_string()).or_insert_with(|| broadcast::channel(256).0).clone()
    }

    fn send(&self, reference: &str, e: DocEvent) {
        if let Some(tx) = lock(&self.events).get(reference) {
            let _ = tx.send(e);
        }
    }

    /// Remembers a file's version as PDT wrote or read it, with its stamp (taken before a read, after a write).
    fn remember(&self, reference: &str, version: &str, stamp: Option<Stamp>) {
        lock(&self.known).insert(reference.to_string(), Seen { version: version.to_string(), stamp });
    }

    /// Remembers the version of a file PDT just wrote.
    fn wrote(&self, reference: &str, bytes: &[u8]) -> String {
        let v = version_of(bytes);
        self.remember(reference, &v, stamp_of(Path::new(reference)));
        v
    }

    /// Whether the file is as PDT last saw it, by its modified time and length (no read).
    fn unchanged(&self, reference: &str, stamp: Option<&Stamp>) -> bool {
        let Some(stamp) = stamp.filter(|s| settled(s)) else { return false };
        lock(&self.known).get(reference).is_some_and(|s| s.stamp.as_ref() == Some(stamp))
    }

    /// Runs `f` on the live model of a sheet, loaded from its file when it isn't held yet. Sheets idle for a while,
    /// with no view open, are let go first.
    fn with_live<R>(&self, reference: &str, f: impl FnOnce(&mut Live) -> R) -> ProviderResult<R> {
        let (path, kind) = path_of(reference)?;
        if kind != DocumentKind::Sheet {
            return Err(ProviderError::Unsupported("a text document has no sheets or cells".into()));
        }
        let slot = {
            let mut all = lock(&self.live);
            let watched: Vec<String> = {
                let ev = lock(&self.events);
                ev.iter().filter(|(_, tx)| tx.receiver_count() > 0).map(|(r, _)| r.clone()).collect()
            };
            all.retain(|r, slot| {
                r == reference
                    || watched.contains(r)
                    || match slot.try_lock() {
                        Ok(l) => l.as_ref().is_some_and(|l| l.used.elapsed() < IDLE),
                        Err(std::sync::TryLockError::Poisoned(l)) => {
                            l.into_inner().as_ref().is_some_and(|l| l.used.elapsed() < IDLE)
                        }
                        // In use (or loading) right now.
                        Err(std::sync::TryLockError::WouldBlock) => true,
                    }
            });
            all.entry(reference.to_string()).or_default().clone()
        };
        let mut slot = lock(&slot);
        let l = match &mut *slot {
            Some(l) => l,
            empty => {
                let stamp = stamp_of(&path);
                let bytes = std::fs::read(&path).map_err(failed)?;
                let mut wb = Workbook::from_xlsx(&bytes, &sheet_name(&path)).map_err(ProviderError::Invalid)?;
                wb.flush();
                self.remember(reference, &version_of(&bytes), stamp);
                empty.insert(Live { wb, seq: 0, used: Instant::now() })
            }
        };
        l.used = Instant::now();
        Ok(f(l))
    }

    /// Reads a live sheet's file again (dropping what isn't saved), and tells its views to reload.
    fn reread(&self, reference: &str, l: &mut Live) {
        let path = Path::new(reference);
        let stamp = stamp_of(path);
        if let Ok(bytes) = std::fs::read(path)
            && let Ok(mut wb) = Workbook::from_xlsx(&bytes, &sheet_name(path))
        {
            wb.flush();
            l.wb = wb;
            self.remember(reference, &version_of(&bytes), stamp);
        }
        l.seq += 1;
        self.send(reference, DocEvent::Reload { seq: l.seq });
    }

    /// Saves a live sheet to its file, and remembers the version written.
    fn save(&self, reference: &str, l: &Live) -> ProviderResult<String> {
        let bytes = l.wb.to_xlsx().map_err(failed)?;
        write_atomic(Path::new(reference), &bytes).map_err(failed)?;
        Ok(self.wrote(reference, &bytes))
    }

    fn sheet_bytes(&self, reference: &str) -> ProviderResult<(Vec<u8>, u64)> {
        self.with_live(reference, |l| (l.wb.to_bytes(), l.seq))
    }

    fn view_diffs(&self, reference: &str, view: &str, seq: u64, diffs: &[u8]) -> ProviderResult<DiffsApplied> {
        self.with_live(reference, |l| {
            let conflict = seq != l.seq;
            if let Err(e) = l.wb.apply_diffs(diffs) {
                // A half-applied list: the model is read from the file again, and every view reloads it.
                self.reread(reference, l);
                return Err(ProviderError::Invalid(format!("the change could not be applied: {e}")));
            }
            l.seq += 1;
            self.save(reference, l)?;
            let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, diffs);
            self.send(reference, DocEvent::Diffs { seq: l.seq, from: view.to_string(), data });
            Ok(DiffsApplied { seq: l.seq, conflict })
        })?
    }

    fn check_external(&self, reference: &str) {
        let Ok((path, kind)) = path_of(reference) else { return };
        match kind {
            DocumentKind::Doc => {
                // Read, compare and remember under the doc's lock, so a save (which holds it while it writes and
                // remembers) can't land in between and make the old file look like a change.
                let doc = self.doc_lock(reference);
                let _held = lock(&doc);
                let stamp = stamp_of(&path);
                if self.unchanged(reference, stamp.as_ref()) {
                    return;
                }
                let Ok(bytes) = std::fs::read(&path) else { return };
                if let Some(v) = self.changed_outside(reference, &bytes, stamp) {
                    self.send(reference, DocEvent::Changed { version: v });
                }
            }
            DocumentKind::Sheet => {
                let Some(slot) = lock(&self.live).get(reference).cloned() else {
                    // Not open: nothing to reload; what the file is now is read when it opens.
                    return;
                };
                // The same for a live sheet: its saves hold its lock while they write and remember.
                let mut slot = lock(&slot);
                let Some(l) = slot.as_mut() else { return };
                let stamp = stamp_of(&path);
                if self.unchanged(reference, stamp.as_ref()) {
                    return;
                }
                let Ok(bytes) = std::fs::read(&path) else { return };
                if self.changed_outside(reference, &bytes, stamp).is_none() {
                    return;
                }
                let Ok(mut wb) = Workbook::from_xlsx(&bytes, &sheet_name(&path)) else { return };
                wb.flush();
                l.wb = wb;
                l.seq += 1;
                self.send(reference, DocEvent::Reload { seq: l.seq });
            }
        }
    }

    /// The file's new version when it differs from the one PDT last wrote or read (and remembers it); none when it
    /// is the same, or PDT hadn't seen the file yet.
    fn changed_outside(&self, reference: &str, bytes: &[u8], stamp: Option<Stamp>) -> Option<String> {
        let v = version_of(bytes);
        let before = lock(&self.known).insert(reference.to_string(), Seen { version: v.clone(), stamp });
        before.is_some_and(|b| b.version != v).then_some(v)
    }

    /// The lock a doc's file is read, written and remembered under; the locks nobody holds are let go.
    fn doc_lock(&self, reference: &str) -> Arc<Mutex<()>> {
        let mut all = lock(&self.doc_locks);
        all.retain(|r, l| r == reference || Arc::strong_count(l) > 1);
        all.entry(reference.to_string()).or_default().clone()
    }

    fn chart_views(&self, reference: &str, charts: Vec<(String, Chart)>) -> ProviderResult<Vec<ChartView>> {
        self.with_live(reference, |l| {
            charts.into_iter().map(|(sheet, chart)| chart_view(&l.wb, sheet, chart)).collect()
        })
    }

    fn export(&self, reference: &str, title: &str, format: &str, sheet: Option<&str>) -> ProviderResult<Export> {
        let (path, kind) = path_of(reference)?;
        let stem = file_stem(title);
        match (kind, format) {
            (DocumentKind::Doc, "md") => Ok(Export {
                bytes: std::fs::read(&path).map_err(failed)?,
                file_name: format!("{stem}.md"),
                mime: "text/markdown; charset=utf-8",
            }),
            (DocumentKind::Sheet, "xlsx") => {
                let bytes = self.with_live(reference, |l| l.wb.to_xlsx().map_err(failed))??;
                Ok(Export {
                    bytes,
                    file_name: format!("{stem}.xlsx"),
                    mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                })
            }
            (DocumentKind::Sheet, "csv") => self.with_live(reference, |l| {
                let names = l.wb.sheet_names();
                let name = sheet.map(str::to_string).unwrap_or_else(|| names[0].clone());
                let csv = l.wb.to_csv(&name).map_err(ProviderError::Invalid)?;
                let file_name =
                    if names.len() > 1 { format!("{stem} - {}.csv", file_stem(&name)) } else { format!("{stem}.csv") };
                Ok(Export { bytes: csv.into_bytes(), file_name, mime: "text/csv; charset=utf-8" })
            })?,
            (DocumentKind::Doc, _) => Err(ProviderError::Invalid("a text document exports as md".into())),
            (DocumentKind::Sheet, _) => Err(ProviderError::Invalid("a spreadsheet exports as xlsx or csv".into())),
        }
    }

    fn import_file(&self, folder: &Path, name: &str, bytes: &[u8]) -> ProviderResult<(ProviderFile, Vec<String>)> {
        let given = Path::new(name);
        let ext = given.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let title = given.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into());
        let title = pdt_core::document::clean_title(&title).unwrap_or_else(|_| "Imported".into());
        std::fs::create_dir_all(folder).map_err(failed)?;
        let (path, data, kind, dropped) = match ext.as_str() {
            "md" | "markdown" | "txt" => {
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| ProviderError::Invalid(format!("{name} is not a text file")))?;
                (free_path(folder, &file_stem(&title), "md"), text.as_bytes().to_vec(), DocumentKind::Doc, vec![])
            }
            "xlsx" => {
                let dropped = pdt_sheet::dropped_parts(bytes);
                let wb = Workbook::from_xlsx(bytes, &title).map_err(ProviderError::Invalid)?;
                (
                    free_path(folder, &file_stem(&title), "xlsx"),
                    wb.to_xlsx().map_err(failed)?,
                    DocumentKind::Sheet,
                    dropped,
                )
            }
            "csv" | "tsv" => {
                let text = String::from_utf8_lossy(bytes);
                let wb = Workbook::from_csv(&text, &title, "Sheet1").map_err(ProviderError::Invalid)?;
                (
                    free_path(folder, &file_stem(&title), "xlsx"),
                    wb.to_xlsx().map_err(failed)?,
                    DocumentKind::Sheet,
                    vec![],
                )
            }
            _ => {
                return Err(ProviderError::Invalid(
                    "PDT imports .md (a doc), and .xlsx or .csv (a spreadsheet)".into(),
                ));
            }
        };
        write_atomic(&path, &data).map_err(failed)?;
        let reference = path.to_string_lossy().into_owned();
        let version = self.wrote(&reference, &data);
        let modified = modified_ms(&path);
        Ok((ProviderFile { reference, title, kind, url: None, version: Some(version), modified }, dropped))
    }

    fn save_image(&self, reference: &str, name: &str, bytes: &[u8]) -> ProviderResult<String> {
        let (path, kind) = path_of(reference)?;
        if kind != DocumentKind::Doc {
            return Err(ProviderError::Unsupported("images go in text documents".into()));
        }
        let given = Path::new(name);
        let ext = given.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !["png", "jpg", "jpeg", "gif", "webp", "svg"].contains(&ext.as_str()) {
            return Err(ProviderError::Invalid("an image is a .png, .jpg, .gif, .webp or .svg".into()));
        }
        let stem = file_stem(&given.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
            .replace(' ', "-");
        let dir = assets_dir(&path);
        std::fs::create_dir_all(&dir).map_err(failed)?;
        let mut n = 1;
        let file = loop {
            let f = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem}-{n}.{ext}") };
            if !dir.join(&f).exists() {
                break f;
            }
            n += 1;
        };
        std::fs::write(dir.join(&file), bytes).map_err(failed)?;
        let folder = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(format!("{}/{file}", folder.replace(' ', "%20")))
    }

    fn asset(&self, reference: &str, rel: &str) -> ProviderResult<(Vec<u8>, &'static str)> {
        let (path, _) = path_of(reference)?;
        let rel = rel.replace("%20", " ");
        let mut parts = rel.split('/').filter(|p| !p.is_empty());
        let (Some(folder), Some(file), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(ProviderError::NotFound);
        };
        let dir = assets_dir(&path);
        if dir.file_name().map(|n| n.to_string_lossy() != folder).unwrap_or(true)
            || file.contains("..")
            || file.contains('\\')
        {
            return Err(ProviderError::NotFound);
        }
        let bytes = std::fs::read(dir.join(file)).map_err(io_failed)?;
        let mime = match Path::new(file).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("svg") => "image/svg+xml",
            _ => "application/octet-stream",
        };
        Ok((bytes, mime))
    }

    fn read_now(&self, reference: &str) -> ProviderResult<DocumentContent> {
        let (path, kind) = path_of(reference)?;
        match kind {
            DocumentKind::Doc => {
                let doc = self.doc_lock(reference);
                let _held = lock(&doc);
                let stamp = stamp_of(&path);
                let bytes = std::fs::read(&path).map_err(failed)?;
                let v = version_of(&bytes);
                self.remember(reference, &v, stamp);
                Ok(DocumentContent::Doc { markdown: String::from_utf8_lossy(&bytes).into_owned(), version: Some(v) })
            }
            DocumentKind::Sheet => {
                let sheets = self.with_live(reference, |l| l.wb.sheets().map_err(failed))??;
                Ok(DocumentContent::Sheet { sheets, version: self.version_now(reference) })
            }
        }
    }

    /// The file's version: the one PDT last saw when the file's stamp hasn't moved since, else a hash of it now.
    fn version_now(&self, reference: &str) -> Option<String> {
        let stamp = stamp_of(Path::new(reference));
        if self.unchanged(reference, stamp.as_ref()) {
            return lock(&self.known).get(reference).map(|s| s.version.clone());
        }
        std::fs::read(reference).ok().map(|b| version_of(&b))
    }

    fn apply_text(&self, reference: &str, edits: &[DocumentEdit]) -> ProviderResult<String> {
        let (path, _) = path_of(reference)?;
        let doc = self.doc_lock(reference);
        let _held = lock(&doc);
        let mut text = String::from_utf8_lossy(&std::fs::read(&path).map_err(failed)?).into_owned();
        for e in edits {
            text = apply_doc_edit(&text, e).map_err(edit_error)?;
        }
        write_atomic(&path, text.as_bytes()).map_err(failed)?;
        let v = self.wrote(reference, text.as_bytes());
        self.send(reference, DocEvent::Changed { version: v.clone() });
        Ok(v)
    }

    /// Cell edits on the live model, in order: all or none (a failed list leaves the model as the file has it).
    fn apply_cells(&self, reference: &str, edits: &[DocumentEdit]) -> ProviderResult<String> {
        self.with_live(reference, |l| {
            let r = edits.iter().try_for_each(|e| match e {
                DocumentEdit::SetCells { sheet, cell, values } => l.wb.set_cells(sheet, cell, values),
                DocumentEdit::AddSheet { name } => l.wb.add_sheet(name),
                DocumentEdit::RenameSheet { sheet, name } => l.wb.rename_sheet(sheet, name),
                DocumentEdit::DeleteSheet { sheet } => l.wb.delete_sheet(sheet),
                DocumentEdit::FormatRange { sheet, range, format } => l.wb.format(sheet, range, format),
                DocumentEdit::ConditionalFormat { sheet, range, when, format } => {
                    l.wb.conditional_format(sheet, range, when, format)
                }
                DocumentEdit::Sort { sheet, range, by, header } => l.wb.sort(sheet, range, by, *header),
                DocumentEdit::Filter { sheet, range, column, when } => l.wb.filter(sheet, range, column, when),
                DocumentEdit::ClearFilter { sheet } => l.wb.clear_filter(sheet),
                DocumentEdit::AddChart { sheet, chart, .. } => l.wb.sheet_index(sheet).and_then(|_| {
                    parse_range(&chart.data).map(|_| ()).ok_or_else(|| format!("{} is not a range", chart.data))
                }),
                DocumentEdit::UpdateChart { chart, .. } => {
                    parse_range(&chart.data).map(|_| ()).ok_or_else(|| format!("{} is not a range", chart.data))
                }
                DocumentEdit::DeleteChart { .. } => Ok(()),
                DocumentEdit::SetBorders { sheet, range, style, color, inner } => {
                    l.wb.borders(sheet, range, *style, color.as_deref(), *inner)
                }
                DocumentEdit::ColumnWidth { sheet, columns, width } => l.wb.column_width(sheet, columns, *width),
                DocumentEdit::Freeze { sheet, rows, columns } => l.wb.freeze(sheet, *rows, *columns),
                DocumentEdit::Validate { .. } => {
                    Err(format!("{UNSUPPORTED}PDT's local sheets have no data validation yet: leave validate out"))
                }
                e => Err(format!("{UNSUPPORTED}{} is for text documents, and this is a spreadsheet", e.op())),
            });
            if let Err(e) = r {
                // Undo what part of the list did by reading the file again; open views reload.
                self.reread(reference, l);
                return Err(match e.strip_prefix(UNSUPPORTED) {
                    Some(m) => ProviderError::Unsupported(m.to_string()),
                    None => ProviderError::Invalid(e),
                });
            }
            let diffs = l.wb.flush();
            let v = self.save(reference, l)?;
            if diffs.len() > 1 {
                l.seq += 1;
                let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &diffs);
                self.send(reference, DocEvent::Diffs { seq: l.seq, from: "pdt".into(), data });
            }
            Ok(v)
        })?
    }

    fn create(&self, new: &NewFile) -> ProviderResult<ProviderFile> {
        std::fs::create_dir_all(&new.folder).map_err(failed)?;
        let stem = file_stem(&new.title);
        let (path, bytes) = match new.kind {
            DocumentKind::Doc => {
                (free_path(&new.folder, &stem, "md"), new.markdown.clone().unwrap_or_default().into_bytes())
            }
            DocumentKind::Sheet => {
                let wb = Workbook::new(&new.title).map_err(failed)?;
                (free_path(&new.folder, &stem, "xlsx"), wb.to_xlsx().map_err(failed)?)
            }
        };
        write_atomic(&path, &bytes).map_err(failed)?;
        let reference = path.to_string_lossy().into_owned();
        let version = self.wrote(&reference, &bytes);
        let modified = modified_ms(&path);
        Ok(ProviderFile {
            reference,
            title: new.title.clone(),
            kind: new.kind,
            url: None,
            version: Some(version),
            modified,
        })
    }

    fn delete(&self, reference: &str) -> ProviderResult<()> {
        let (path, _) = path_of(reference)?;
        lock(&self.live).remove(reference);
        std::fs::remove_file(&path).map_err(failed)?;
        let assets = assets_dir(&path);
        if assets.is_dir() {
            std::fs::remove_dir_all(&assets).map_err(failed)?;
        }
        lock(&self.known).remove(reference);
        Ok(())
    }

    /// The docs and sheets in `folders` and their subfolders whose names have every word of `q` (lowercase), newest
    /// first. Hidden folders (`.git`) and docs' image folders (`.assets`) are skipped, and the walk stops at
    /// [`SEARCH_DEPTH`] folders down and [`SEARCH_ENTRIES`] entries, so a folder holding a large tree stays quick.
    fn search(folders: &[PathBuf], q: &str, kind: Option<DocumentKind>) -> Vec<ProviderFile> {
        let mut found: Vec<(SystemTime, ProviderFile)> = Vec::new();
        let mut seen = HashSet::new();
        let mut entries_left = SEARCH_ENTRIES;
        let mut stack: Vec<(PathBuf, usize)> = folders.iter().rev().map(|f| (f.clone(), 0)).collect();
        while let Some((folder, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&folder) else { continue };
            for e in entries.flatten() {
                if entries_left == 0 {
                    break;
                }
                entries_left -= 1;
                let p = e.path();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    if depth < SEARCH_DEPTH && !name.starts_with('.') && !name.ends_with(".assets") {
                        stack.push((p, depth + 1));
                    }
                    continue;
                }
                let Some(k) = kind_of(&p) else { continue };
                let title = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if kind.is_some_and(|w| w != k)
                    || !q.split_whitespace().all(|w| title.to_lowercase().contains(w))
                    || !p.is_file()
                {
                    continue;
                }
                let reference = p.to_string_lossy().into_owned();
                if !seen.insert(reference.clone()) {
                    continue;
                }
                let when = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                let modified = ms_of(when);
                found.push((when, ProviderFile { reference, title, kind: k, url: None, version: None, modified }));
            }
        }
        found.sort_by_key(|f| std::cmp::Reverse(f.0));
        found.into_iter().map(|(_, f)| f).collect()
    }
}

/// A file's last change, in ms since the epoch (none when the file system doesn't say).
fn modified_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(ms_of)
}

fn ms_of(t: SystemTime) -> Option<i64> {
    t.duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_millis()).ok()).filter(|&ms| ms > 0)
}

impl LocalProvider {
    pub fn new(store: Store, data_dir: PathBuf, home: Option<PathBuf>) -> Self {
        Self { store, data_dir, home, files: Default::default() }
    }

    /// Runs file work (reads, xlsx parsing and writing) on a blocking thread, not the runtime's.
    async fn blocking<R: Send + 'static>(
        &self,
        f: impl FnOnce(&Files) -> ProviderResult<R> + Send + 'static,
    ) -> ProviderResult<R> {
        let files = self.files.clone();
        tokio::task::spawn_blocking(move || f(&files)).await.map_err(failed)?
    }

    /// A document's changes as they happen.
    pub fn subscribe(&self, reference: &str) -> broadcast::Receiver<DocEvent> {
        self.files.sender(reference).subscribe()
    }

    /// Tells a document's open views that its comments changed.
    pub fn comments_changed(&self, reference: &str) {
        self.files.send(reference, DocEvent::Comments);
    }

    /// A sheet's model as IronCalc's bytes, for a view to load, and the number of its last change.
    pub async fn sheet_bytes(&self, reference: &str) -> ProviderResult<(Vec<u8>, u64)> {
        let r = reference.to_string();
        self.blocking(move |f| f.sheet_bytes(&r)).await
    }

    /// A view's edits (IronCalc diffs) made on top of change `seq`: applied, saved, and passed on to the other views.
    /// When someone else's change came in between, they are applied all the same (the last write wins) and the view
    /// is told to load the sheet again.
    pub async fn view_diffs(
        &self,
        reference: &str,
        view: &str,
        seq: u64,
        diffs: Vec<u8>,
    ) -> ProviderResult<DiffsApplied> {
        let (r, view) = (reference.to_string(), view.to_string());
        self.blocking(move |f| f.view_diffs(&r, &view, seq, &diffs)).await
    }

    /// Notices a file changed outside PDT (an agent editing it in place, a sync tool): a doc's views are told, a live
    /// sheet is read again and its views reload. A file whose modified time and length haven't moved isn't read.
    pub async fn check_external(&self, reference: &str) {
        let r = reference.to_string();
        let _ = self
            .blocking(move |f| {
                f.check_external(&r);
                Ok(())
            })
            .await;
    }

    /// A sheet's charts with the numbers they draw, read from the live model.
    pub async fn charts(&self, document_id: &str, reference: &str) -> ProviderResult<Vec<ChartView>> {
        let charts = self.store.document_charts(document_id).await.map_err(failed)?;
        if charts.is_empty() {
            return Ok(vec![]);
        }
        let r = reference.to_string();
        self.blocking(move |f| f.chart_views(&r, charts)).await
    }

    /// A document as a file: a doc as `.md`; a sheet as `.xlsx`, or one of its sheets as `.csv`.
    pub async fn export(
        &self,
        reference: &str,
        title: &str,
        format: &str,
        sheet: Option<&str>,
    ) -> ProviderResult<Export> {
        let (r, title, format) = (reference.to_string(), title.to_string(), format.to_string());
        let sheet = sheet.map(str::to_string);
        self.blocking(move |f| f.export(&r, &title, &format, sheet.as_deref())).await
    }

    /// A file from the user's computer made into a local document in `folder`: `.md` (or `.markdown`, `.txt`) as a
    /// doc; `.xlsx` or `.csv` as a sheet. Its title, and what of it PDT can't keep (charts, images and pivot tables of
    /// a `.xlsx`).
    pub async fn import_file(
        &self,
        folder: PathBuf,
        name: String,
        bytes: Vec<u8>,
    ) -> ProviderResult<(ProviderFile, Vec<String>)> {
        self.blocking(move |f| f.import_file(&folder, &name, &bytes)).await
    }

    /// Saves an image for a doc beside it (`<doc>.assets/<name>`); the path to write in its Markdown.
    pub async fn save_image(&self, reference: &str, name: &str, bytes: Vec<u8>) -> ProviderResult<String> {
        let (r, name) = (reference.to_string(), name.to_string());
        self.blocking(move |f| f.save_image(&r, &name, &bytes)).await
    }

    /// One of a doc's images (a path under its `.assets` folder, as its Markdown names it), and its type.
    pub async fn asset(&self, reference: &str, rel: &str) -> ProviderResult<(Vec<u8>, &'static str)> {
        let (r, rel) = (reference.to_string(), rel.to_string());
        self.blocking(move |f| f.asset(&r, &rel)).await
    }

    /// The folders local documents can be in: the host's and each project's.
    async fn folders(&self) -> ProviderResult<Vec<PathBuf>> {
        let host = self.store.get_host_settings().await.map_err(failed)?.documents_dir;
        let mut out = vec![documents_folder(None, host.as_deref(), &self.data_dir, self.home.as_deref())];
        for p in self.store.list_projects().await.map_err(failed)? {
            let own = self.store.project_document_settings(&p.id).await.map_err(failed)?.folder;
            if own.is_some() {
                let f = documents_folder(own.as_deref(), None, &self.data_dir, self.home.as_deref());
                if !out.contains(&f) {
                    out.push(f);
                }
            }
        }
        Ok(out)
    }

    async fn document_id(&self, reference: &str) -> ProviderResult<String> {
        Ok(self.store.document_by_ref(ID, reference).await.map_err(failed)?.ok_or(ProviderError::NotFound)?.id)
    }

    async fn set_resolved(&self, reference: &str, comment: &str, resolved: bool) -> ProviderResult<()> {
        let id = self.document_id(reference).await?;
        if !self.store.resolve_document_comment(&id, comment, resolved).await.map_err(failed)? {
            return Err(ProviderError::NotFound);
        }
        self.comments_changed(reference);
        Ok(())
    }

    async fn version_now(&self, reference: &str) -> Option<String> {
        let r = reference.to_string();
        self.blocking(move |f| Ok(f.version_now(&r))).await.ok().flatten()
    }
}

impl DocumentProvider for LocalProvider {
    fn id(&self) -> &'static str {
        ID
    }
    fn label(&self) -> &'static str {
        "Local"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            docs: true,
            sheets: true,
            create: true,
            search: true,
            edit: true,
            rename: false,
            delete: true,
            comments: true,
            embed: false,
            charts: true,
            live: true,
        }
    }

    fn file<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<ProviderFile>> {
        Box::pin(async move {
            let (path, kind) = path_of(reference)?;
            let title = match self.store.document_by_ref(ID, reference).await.map_err(failed)? {
                Some(d) => d.title,
                None => {
                    // A file PDT doesn't know yet is only taken from a local documents folder.
                    let file = path.canonicalize().map_err(io_failed)?;
                    let inside =
                        self.folders().await?.iter().any(|f| f.canonicalize().is_ok_and(|f| file.starts_with(f)));
                    if !inside {
                        return Err(ProviderError::Invalid(
                            "PDT imports local files from the local documents folders only".into(),
                        ));
                    }
                    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                }
            };
            Ok(ProviderFile {
                reference: reference.into(),
                title,
                kind,
                url: None,
                version: self.version_now(reference).await,
                modified: modified_ms(&path),
            })
        })
    }

    fn read<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<DocumentContent>> {
        Box::pin(async move {
            let r = reference.to_string();
            let mut content = self.blocking(move |f| f.read_now(&r)).await?;
            if let DocumentContent::Sheet { sheets, .. } = &mut content
                && let Some(d) = self.store.document_by_ref(ID, reference).await.map_err(failed)?
            {
                for (sheet, chart) in self.store.document_charts(&d.id).await.map_err(failed)? {
                    if let Some(s) = sheets.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&sheet)) {
                        s.charts.push(chart);
                    }
                }
            }
            Ok(content)
        })
    }

    fn search<'a>(&'a self, q: &'a str, kind: Option<DocumentKind>) -> BoxFut<'a, ProviderResult<Vec<ProviderFile>>> {
        Box::pin(async move {
            let q = q.to_lowercase();
            let folders = self.folders().await?;
            self.blocking(move |_| Ok(Files::search(&folders, &q, kind))).await
        })
    }

    fn create<'a>(&'a self, new: NewFile) -> BoxFut<'a, ProviderResult<ProviderFile>> {
        Box::pin(async move { self.blocking(move |f| f.create(&new)).await })
    }

    fn apply<'a>(&'a self, reference: &'a str, edits: Vec<DocumentEdit>) -> BoxFut<'a, ProviderResult<Option<String>>> {
        Box::pin(async move {
            let (_, kind) = path_of(reference)?;
            let r = reference.to_string();
            if kind == DocumentKind::Doc {
                return self.blocking(move |f| f.apply_text(&r, &edits)).await.map(Some);
            }
            let charting = edits.iter().any(|e| {
                matches!(
                    e,
                    DocumentEdit::AddChart { .. }
                        | DocumentEdit::UpdateChart { .. }
                        | DocumentEdit::DeleteChart { .. }
                        | DocumentEdit::RenameSheet { .. }
                        | DocumentEdit::DeleteSheet { .. }
                )
            });
            let doc = if charting { Some(self.document_id(reference).await?) } else { None };
            // Charts are PDT's: check the list against them before anything changes.
            if let Some(doc) = &doc {
                let mut ids: Vec<String> =
                    self.store.document_charts(doc).await.map_err(failed)?.into_iter().map(|(_, c)| c.id).collect();
                for e in &edits {
                    match e {
                        DocumentEdit::AddChart { id, .. } => {
                            let id = id.clone().filter(|i| !i.trim().is_empty());
                            let id = id.ok_or_else(|| ProviderError::Invalid("a new chart needs an id".into()))?;
                            if ids.contains(&id) {
                                return Err(ProviderError::Invalid(format!("there is a chart {id} already")));
                            }
                            ids.push(id);
                        }
                        DocumentEdit::UpdateChart { id, .. } | DocumentEdit::DeleteChart { id }
                            if !ids.contains(id) =>
                        {
                            return Err(ProviderError::Invalid(format!("there is no chart {id}")));
                        }
                        DocumentEdit::DeleteChart { id } => ids.retain(|i| i != id),
                        _ => {}
                    }
                }
            }
            let (v, edits) = self
                .blocking(move |f| {
                    let v = f.apply_cells(&r, &edits);
                    Ok((v, edits))
                })
                .await?;
            let v = v?;
            if let Some(doc) = doc {
                let mut charts = self.store.document_charts(&doc).await.map_err(failed)?;
                let mut changed = false;
                for e in edits {
                    match e {
                        DocumentEdit::AddChart { sheet, id: Some(id), chart } => {
                            let c = Chart { id, spec: chart };
                            self.store.put_document_chart(&doc, &sheet, &c).await.map_err(failed)?;
                            // Later edits in the list (a delete of its sheet) see it.
                            charts.push((sheet, c));
                            changed = true;
                        }
                        DocumentEdit::UpdateChart { id, chart } => {
                            let sheet =
                                charts.iter().find(|(_, c)| c.id == id).map(|(s, _)| s.clone()).unwrap_or_default();
                            self.store
                                .put_document_chart(&doc, &sheet, &Chart { id, spec: chart })
                                .await
                                .map_err(failed)?;
                            changed = true;
                        }
                        DocumentEdit::DeleteChart { id } => {
                            self.store.delete_document_chart(&doc, &id).await.map_err(failed)?;
                            changed = true;
                        }
                        DocumentEdit::RenameSheet { sheet, name } => {
                            let from =
                                charts.iter().find(|(s, _)| s.eq_ignore_ascii_case(&sheet)).map(|(s, _)| s.clone());
                            if let Some(from) = from {
                                self.store.rename_chart_sheet(&doc, &from, name.trim()).await.map_err(failed)?;
                                charts = self.store.document_charts(&doc).await.map_err(failed)?;
                                changed = true;
                            }
                        }
                        // A deleted sheet's charts go with it.
                        DocumentEdit::DeleteSheet { sheet } => {
                            for (_, c) in charts.iter().filter(|(s, _)| s.eq_ignore_ascii_case(&sheet)) {
                                self.store.delete_document_chart(&doc, &c.id).await.map_err(failed)?;
                                changed = true;
                            }
                            charts.retain(|(s, _)| !s.eq_ignore_ascii_case(&sheet));
                        }
                        _ => {}
                    }
                }
                if changed {
                    self.files.send(reference, DocEvent::Charts);
                }
            }
            Ok(Some(v))
        })
    }

    fn delete<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        let r = reference.to_string();
        Box::pin(async move { self.blocking(move |f| f.delete(&r)).await })
    }

    fn comments<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<Vec<DocumentComment>>> {
        Box::pin(async move {
            let id = self.document_id(reference).await?;
            self.store.document_comments(&id).await.map_err(failed)
        })
    }

    fn comment<'a>(&'a self, reference: &'a str, new: CommentInput) -> BoxFut<'a, ProviderResult<DocumentComment>> {
        Box::pin(async move {
            let document_id = self.document_id(reference).await?;
            let anchor = new.anchor.map(|a| a.cleaned()).transpose().map_err(ProviderError::Invalid)?;
            let id = uuid::Uuid::now_v7().to_string();
            let c = NewDocumentComment {
                id: id.clone(),
                document_id: document_id.clone(),
                parent_id: None,
                anchor,
                author: new.author,
                actor: new.actor,
                body: new.body,
                created_at: crate::routes::now_ms(),
            };
            if !self.store.add_document_comment(&c).await.map_err(failed)? {
                return Err(ProviderError::NotFound);
            }
            self.comments_changed(reference);
            let all = self.store.document_comments(&document_id).await.map_err(failed)?;
            all.into_iter().find(|c| c.id == id).ok_or(ProviderError::NotFound)
        })
    }

    fn reply<'a>(
        &'a self,
        reference: &'a str,
        comment: &'a str,
        body: &'a str,
        by: (Actor, String),
    ) -> BoxFut<'a, ProviderResult<CommentReply>> {
        Box::pin(async move {
            let document_id = self.document_id(reference).await?;
            let now = crate::routes::now_ms();
            let c = NewDocumentComment {
                id: uuid::Uuid::now_v7().to_string(),
                document_id,
                parent_id: Some(comment.to_string()),
                anchor: None,
                author: by.1.clone(),
                actor: by.0,
                body: body.to_string(),
                created_at: now,
            };
            if !self.store.add_document_comment(&c).await.map_err(failed)? {
                return Err(ProviderError::NotFound);
            }
            self.comments_changed(reference);
            Ok(CommentReply { id: c.id, author: by.1, body: c.body, created_at: Some(now), actor: Some(by.0) })
        })
    }

    fn resolve<'a>(&'a self, reference: &'a str, comment: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        Box::pin(self.set_resolved(reference, comment, true))
    }

    fn reopen<'a>(&'a self, reference: &'a str, comment: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        Box::pin(self.set_resolved(reference, comment, false))
    }

    fn version<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<Option<String>>> {
        Box::pin(async move {
            path_of(reference)?;
            Ok(self.version_now(reference).await)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_come_from_titles() {
        assert_eq!(file_stem("PDT-7 design: Faster boards"), "PDT-7 design- Faster boards");
        assert_eq!(file_stem("../../etc/passwd"), "etc-passwd");
        assert_eq!(file_stem("///"), "Untitled");
        assert_eq!(file_stem(&"x".repeat(200)).len(), 80);
    }

    #[test]
    fn new_files_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let a = free_path(dir.path(), "Plan", "md");
        std::fs::write(&a, "").unwrap();
        let b = free_path(dir.path(), "Plan", "md");
        assert_eq!(b.file_name().unwrap(), "Plan 2.md");
        assert_eq!(assets_dir(&a).file_name().unwrap(), "Plan.assets");
    }

    fn sheet_file(dir: &Path, name: &str) -> String {
        let p = dir.join(name);
        std::fs::write(&p, Workbook::new("S").unwrap().to_xlsx().unwrap()).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn a_sheet_in_use_holds_up_no_other() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (sheet_file(dir.path(), "A.xlsx"), sheet_file(dir.path(), "B.xlsx"));
        let files = Arc::new(Files::default());
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let t = {
            let (files, a) = (files.clone(), a.clone());
            std::thread::spawn(move || {
                files
                    .with_live(&a, |_| {
                        held_tx.send(()).unwrap();
                        go_rx.recv().unwrap();
                    })
                    .unwrap()
            })
        };
        held_rx.recv().unwrap();
        // A is held (as while it loads or saves): B still opens, and A isn't let go from under it.
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        {
            let (files, b) = (files.clone(), b.clone());
            std::thread::spawn(move || done_tx.send(files.with_live(&b, |l| l.seq).unwrap()).unwrap());
        }
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(10)), Ok(0));
        assert!(lock(&files.live).contains_key(&a));
        go_tx.send(()).unwrap();
        t.join().unwrap();
    }

    #[test]
    fn senders_and_locks_nobody_uses_are_let_go() {
        let files = Files::default();
        let rx = files.sender("/a.md").subscribe();
        drop(rx);
        let _b = files.sender("/b.md").subscribe();
        assert_eq!(lock(&files.events).keys().collect::<Vec<_>>(), vec!["/b.md"]);
        drop(files.doc_lock("/a.md"));
        let _held = files.doc_lock("/b.md");
        assert_eq!(lock(&files.doc_locks).keys().collect::<Vec<_>>(), vec!["/b.md"]);
    }

    #[test]
    fn search_looks_in_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("plans/2026")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("Roadmap.assets")).unwrap();
        for f in ["Roadmap.md", "plans/Plan A.md", "plans/2026/Plan B.md", ".git/Plan C.md", "Roadmap.assets/Plan D.md"]
        {
            std::fs::write(root.join(f), "").unwrap();
        }
        sheet_file(&root.join("plans"), "Plan sheet.xlsx");
        let titles = |q: &str, kind| {
            let mut t: Vec<_> = Files::search(&[root.to_path_buf()], q, kind).into_iter().map(|f| f.title).collect();
            t.sort();
            t
        };
        assert_eq!(titles("plan", None), ["Plan A", "Plan B", "Plan sheet"], "hidden and image folders skipped");
        assert_eq!(titles("plan", Some(DocumentKind::Sheet)), ["Plan sheet"]);
        assert_eq!(titles("", None).len(), 4);
        // A folder inside another one searched lists its files once.
        let both = Files::search(&[root.to_path_buf(), root.join("plans")], "plan b", None);
        assert_eq!(both.len(), 1);
    }

    #[test]
    fn an_unmoved_file_is_not_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Plan.md");
        let reference = path.to_string_lossy().into_owned();
        let old = SystemTime::now() - Duration::from_secs(60);
        let write = |text: &str| {
            std::fs::write(&path, text).unwrap();
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(old).unwrap();
        };
        write("one");
        let files = Files::default();
        let mut rx = files.sender(&reference).subscribe();
        let DocumentContent::Doc { version, .. } = files.read_now(&reference).unwrap() else { panic!() };
        // Same length and modified time: taken as unchanged, without reading it.
        write("two");
        files.check_external(&reference);
        assert!(rx.try_recv().is_err());
        assert_eq!(files.version_now(&reference), version);
        // Its length moved: read, and the views hear of it.
        write("three");
        files.check_external(&reference);
        assert!(matches!(rx.try_recv(), Ok(DocEvent::Changed { .. })));
        assert_eq!(files.version_now(&reference), Some(version_of(b"three")));
        // Modified just now: read whatever its stamp says.
        std::fs::write(&path, "four!").unwrap();
        files.check_external(&reference);
        assert!(matches!(rx.try_recv(), Ok(DocEvent::Changed { .. })));
    }
}
