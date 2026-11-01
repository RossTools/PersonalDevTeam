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
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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
    live: Mutex<HashMap<String, Arc<Mutex<Live>>>>,
    events: Mutex<HashMap<String, broadcast::Sender<DocEvent>>>,
    /// Each file's version as PDT last wrote or read it, to notice changes made outside PDT.
    known: Mutex<HashMap<String, String>>,
    /// Per doc: held while its file is read and remembered or written, so those don't interleave.
    doc_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn failed(e: impl std::fmt::Display) -> ProviderError {
    ProviderError::Failed(e.to_string())
}

/// A file's version: a hash of its bytes.
fn version_of(bytes: &[u8]) -> String {
    hex::encode(&sha2::Sha256::digest(bytes)[..8])
}

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

impl LocalProvider {
    pub fn new(store: Store, data_dir: PathBuf, home: Option<PathBuf>) -> Self {
        Self {
            store,
            data_dir,
            home,
            live: Default::default(),
            events: Default::default(),
            known: Default::default(),
            doc_locks: Default::default(),
        }
    }

    /// The file a reference names, checked to be a doc or sheet that exists.
    fn path(&self, reference: &str) -> ProviderResult<(PathBuf, DocumentKind)> {
        let p = PathBuf::from(reference);
        let kind = kind_of(&p).ok_or(ProviderError::NotFound)?;
        if !p.is_absolute() || !p.is_file() {
            return Err(ProviderError::NotFound);
        }
        Ok((p, kind))
    }

    fn sender(&self, reference: &str) -> broadcast::Sender<DocEvent> {
        lock(&self.events).entry(reference.to_string()).or_insert_with(|| broadcast::channel(256).0).clone()
    }

    fn send(&self, reference: &str, e: DocEvent) {
        if let Some(tx) = lock(&self.events).get(reference) {
            let _ = tx.send(e);
        }
    }

    /// A document's changes as they happen.
    pub fn subscribe(&self, reference: &str) -> broadcast::Receiver<DocEvent> {
        self.sender(reference).subscribe()
    }

    /// Tells a document's open views that its comments changed.
    pub fn comments_changed(&self, reference: &str) {
        self.send(reference, DocEvent::Comments);
    }

    fn remember(&self, reference: &str, version: &str) {
        lock(&self.known).insert(reference.to_string(), version.to_string());
    }

    /// The live model of a sheet, loaded from its file when it isn't held yet. Sheets idle for a while, with no view
    /// open, are let go first.
    fn live(&self, reference: &str) -> ProviderResult<Arc<Mutex<Live>>> {
        let (path, kind) = self.path(reference)?;
        if kind != DocumentKind::Sheet {
            return Err(ProviderError::Unsupported("a text document has no sheets or cells".into()));
        }
        let mut all = lock(&self.live);
        let watched: Vec<String> = {
            let ev = lock(&self.events);
            ev.iter().filter(|(_, tx)| tx.receiver_count() > 0).map(|(r, _)| r.clone()).collect()
        };
        all.retain(|r, l| r == reference || watched.contains(r) || lock(l).used.elapsed() < IDLE);
        if let Some(l) = all.get(reference) {
            lock(l).used = Instant::now();
            return Ok(l.clone());
        }
        let bytes = std::fs::read(&path).map_err(failed)?;
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut wb = Workbook::from_xlsx(&bytes, &name).map_err(ProviderError::Invalid)?;
        wb.flush();
        self.remember(reference, &version_of(&bytes));
        let l = Arc::new(Mutex::new(Live { wb, seq: 0, used: Instant::now() }));
        all.insert(reference.to_string(), l.clone());
        Ok(l)
    }

    /// Reads a live sheet's file again (dropping what isn't saved), and tells its views to reload.
    fn reread(&self, reference: &str, l: &mut Live) {
        if let Ok(bytes) = std::fs::read(reference) {
            let name = Path::new(reference).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if let Ok(mut wb) = Workbook::from_xlsx(&bytes, &name) {
                wb.flush();
                l.wb = wb;
                self.remember(reference, &version_of(&bytes));
            }
        }
        l.seq += 1;
        self.send(reference, DocEvent::Reload { seq: l.seq });
    }

    /// Saves a live sheet to its file, and remembers the version written.
    fn save(&self, reference: &str, l: &Live) -> ProviderResult<String> {
        let bytes = l.wb.to_xlsx().map_err(failed)?;
        write_atomic(Path::new(reference), &bytes).map_err(failed)?;
        let v = version_of(&bytes);
        self.remember(reference, &v);
        Ok(v)
    }

    /// A sheet's model as IronCalc's bytes, for a view to load, and the number of its last change.
    pub fn sheet_bytes(&self, reference: &str) -> ProviderResult<(Vec<u8>, u64)> {
        let l = self.live(reference)?;
        let l = lock(&l);
        Ok((l.wb.to_bytes(), l.seq))
    }

    /// A view's edits (IronCalc diffs) made on top of change `seq`: applied, saved, and passed on to the other views.
    /// When someone else's change came in between, they are applied all the same (the last write wins) and the view
    /// is told to load the sheet again.
    pub fn view_diffs(&self, reference: &str, view: &str, seq: u64, diffs: &[u8]) -> ProviderResult<DiffsApplied> {
        let l = self.live(reference)?;
        let mut l = lock(&l);
        let conflict = seq != l.seq;
        if let Err(e) = l.wb.apply_diffs(diffs) {
            // A half-applied list: the model is read from the file again, and every view reloads it.
            self.reread(reference, &mut l);
            return Err(ProviderError::Invalid(format!("the change could not be applied: {e}")));
        }
        l.seq += 1;
        l.used = Instant::now();
        self.save(reference, &l)?;
        let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, diffs);
        self.send(reference, DocEvent::Diffs { seq: l.seq, from: view.to_string(), data });
        Ok(DiffsApplied { seq: l.seq, conflict })
    }

    /// Notices a file changed outside PDT (an agent editing it in place, a sync tool): a doc's views are told, a live
    /// sheet is read again and its views reload.
    pub fn check_external(&self, reference: &str) {
        let Ok((path, kind)) = self.path(reference) else { return };
        match kind {
            DocumentKind::Doc => {
                // Read, compare and remember under the doc's lock, so a save (which holds it while it writes and
                // remembers) can't land in between and make the old file look like a change.
                let doc = self.doc_lock(reference);
                let _held = lock(&doc);
                let Ok(bytes) = std::fs::read(&path) else { return };
                if let Some(v) = self.changed_outside(reference, &bytes) {
                    self.send(reference, DocEvent::Changed { version: v });
                }
            }
            DocumentKind::Sheet => {
                let Some(l) = lock(&self.live).get(reference).cloned() else {
                    // Not open: nothing to reload; what the file is now is read when it opens.
                    return;
                };
                // The same for a live sheet: its saves hold its lock while they write and remember.
                let mut l = lock(&l);
                let Ok(bytes) = std::fs::read(&path) else { return };
                if self.changed_outside(reference, &bytes).is_none() {
                    return;
                }
                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let Ok(mut wb) = Workbook::from_xlsx(&bytes, &name) else { return };
                wb.flush();
                l.wb = wb;
                l.seq += 1;
                self.send(reference, DocEvent::Reload { seq: l.seq });
            }
        }
    }

    /// The file's new version when it differs from the one PDT last wrote or read (and remembers it); none when it
    /// is the same, or PDT hadn't seen the file yet.
    fn changed_outside(&self, reference: &str, bytes: &[u8]) -> Option<String> {
        let v = version_of(bytes);
        let before = lock(&self.known).insert(reference.to_string(), v.clone());
        (before.is_some() && before.as_deref() != Some(v.as_str())).then_some(v)
    }

    /// The lock a doc's file is read, written and remembered under.
    fn doc_lock(&self, reference: &str) -> Arc<Mutex<()>> {
        lock(&self.doc_locks).entry(reference.to_string()).or_default().clone()
    }

    /// A sheet's charts with the numbers they draw, read from the live model.
    pub async fn charts(&self, document_id: &str, reference: &str) -> ProviderResult<Vec<ChartView>> {
        let charts = self.store.document_charts(document_id).await.map_err(failed)?;
        if charts.is_empty() {
            return Ok(vec![]);
        }
        let l = self.live(reference)?;
        let l = lock(&l);
        Ok(charts.into_iter().map(|(sheet, chart)| chart_view(&l.wb, sheet, chart)).collect())
    }

    /// A document as a file: a doc as `.md`; a sheet as `.xlsx`, or one of its sheets as `.csv`.
    pub fn export(&self, reference: &str, title: &str, format: &str, sheet: Option<&str>) -> ProviderResult<Export> {
        let (path, kind) = self.path(reference)?;
        let stem = file_stem(title);
        match (kind, format) {
            (DocumentKind::Doc, "md") => Ok(Export {
                bytes: std::fs::read(&path).map_err(failed)?,
                file_name: format!("{stem}.md"),
                mime: "text/markdown; charset=utf-8",
            }),
            (DocumentKind::Sheet, "xlsx") => {
                let l = self.live(reference)?;
                let bytes = lock(&l).wb.to_xlsx().map_err(failed)?;
                Ok(Export {
                    bytes,
                    file_name: format!("{stem}.xlsx"),
                    mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                })
            }
            (DocumentKind::Sheet, "csv") => {
                let l = self.live(reference)?;
                let l = lock(&l);
                let names = l.wb.sheet_names();
                let name = sheet.map(str::to_string).unwrap_or_else(|| names[0].clone());
                let csv = l.wb.to_csv(&name).map_err(ProviderError::Invalid)?;
                let file_name =
                    if names.len() > 1 { format!("{stem} - {}.csv", file_stem(&name)) } else { format!("{stem}.csv") };
                Ok(Export { bytes: csv.into_bytes(), file_name, mime: "text/csv; charset=utf-8" })
            }
            (DocumentKind::Doc, _) => Err(ProviderError::Invalid("a text document exports as md".into())),
            (DocumentKind::Sheet, _) => Err(ProviderError::Invalid("a spreadsheet exports as xlsx or csv".into())),
        }
    }

    /// A file from the user's computer made into a local document in `folder`: `.md` (or `.markdown`, `.txt`) as a
    /// doc; `.xlsx` or `.csv` as a sheet. Its title, and what of it PDT can't keep (charts, images and pivot tables of
    /// a `.xlsx`).
    pub fn import_file(&self, folder: &Path, name: &str, bytes: &[u8]) -> ProviderResult<(ProviderFile, Vec<String>)> {
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
        let version = version_of(&data);
        self.remember(&reference, &version);
        Ok((ProviderFile { reference, title, kind, url: None, version: Some(version) }, dropped))
    }

    /// Saves an image for a doc beside it (`<doc>.assets/<name>`); the path to write in its Markdown.
    pub fn save_image(&self, reference: &str, name: &str, bytes: &[u8]) -> ProviderResult<String> {
        let (path, kind) = self.path(reference)?;
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

    /// One of a doc's images (a path under its `.assets` folder, as its Markdown names it), and its type.
    pub fn asset(&self, reference: &str, rel: &str) -> ProviderResult<(Vec<u8>, &'static str)> {
        let (path, _) = self.path(reference)?;
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
        let bytes = std::fs::read(dir.join(file)).map_err(|_| ProviderError::NotFound)?;
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

    fn read_now(&self, reference: &str) -> ProviderResult<DocumentContent> {
        let (path, kind) = self.path(reference)?;
        match kind {
            DocumentKind::Doc => {
                let doc = self.doc_lock(reference);
                let _held = lock(&doc);
                let bytes = std::fs::read(&path).map_err(failed)?;
                let v = version_of(&bytes);
                self.remember(reference, &v);
                Ok(DocumentContent::Doc { markdown: String::from_utf8_lossy(&bytes).into_owned(), version: Some(v) })
            }
            DocumentKind::Sheet => {
                let l = self.live(reference)?;
                let l = lock(&l);
                Ok(DocumentContent::Sheet {
                    sheets: l.wb.sheets().map_err(failed)?,
                    version: self.version_now(reference),
                })
            }
        }
    }

    fn version_now(&self, reference: &str) -> Option<String> {
        std::fs::read(reference).ok().map(|b| version_of(&b))
    }

    fn apply_text(&self, reference: &str, edits: Vec<DocumentEdit>) -> ProviderResult<String> {
        let (path, _) = self.path(reference)?;
        let doc = self.doc_lock(reference);
        let _held = lock(&doc);
        let mut text = String::from_utf8_lossy(&std::fs::read(&path).map_err(failed)?).into_owned();
        for e in &edits {
            text = apply_doc_edit(&text, e).map_err(edit_error)?;
        }
        write_atomic(&path, text.as_bytes()).map_err(failed)?;
        let v = version_of(text.as_bytes());
        self.remember(reference, &v);
        self.send(reference, DocEvent::Changed { version: v.clone() });
        Ok(v)
    }

    /// Cell edits on the live model, in order: all or none (a failed list leaves the model as the file has it).
    fn apply_cells(&self, reference: &str, edits: &[DocumentEdit]) -> ProviderResult<String> {
        let l = self.live(reference)?;
        let mut l = lock(&l);
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
            e => Err(format!("{UNSUPPORTED}{} is for text documents, and this is a spreadsheet", e.op())),
        });
        if let Err(e) = r {
            // Undo what part of the list did by reading the file again; open views reload.
            self.reread(reference, &mut l);
            return Err(match e.strip_prefix(UNSUPPORTED) {
                Some(m) => ProviderError::Unsupported(m.to_string()),
                None => ProviderError::Invalid(e),
            });
        }
        let diffs = l.wb.flush();
        let v = self.save(reference, &l)?;
        if diffs.len() > 1 {
            l.seq += 1;
            let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &diffs);
            self.send(reference, DocEvent::Diffs { seq: l.seq, from: "pdt".into(), data });
        }
        Ok(v)
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
            let (path, kind) = self.path(reference)?;
            let title = match self.store.document_by_ref(ID, reference).await.map_err(failed)? {
                Some(d) => d.title,
                None => {
                    // A file PDT doesn't know yet is only taken from a local documents folder.
                    let file = path.canonicalize().map_err(|_| ProviderError::NotFound)?;
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
                version: self.version_now(reference),
            })
        })
    }

    fn read<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<DocumentContent>> {
        Box::pin(async move {
            let mut content = self.read_now(reference)?;
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
            let mut found: Vec<(std::time::SystemTime, ProviderFile)> = Vec::new();
            for folder in self.folders().await? {
                let Ok(entries) = std::fs::read_dir(&folder) else { continue };
                for e in entries.flatten() {
                    let p = e.path();
                    let Some(k) = kind_of(&p) else { continue };
                    let title = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    if kind.is_some_and(|w| w != k)
                        || !q.split_whitespace().all(|w| title.to_lowercase().contains(w))
                        || !p.is_file()
                    {
                        continue;
                    }
                    let when = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                    let reference = p.to_string_lossy().into_owned();
                    found.push((when, ProviderFile { reference, title, kind: k, url: None, version: None }));
                }
            }
            found.sort_by_key(|f| std::cmp::Reverse(f.0));
            Ok(found.into_iter().map(|(_, f)| f).collect())
        })
    }

    fn create<'a>(&'a self, new: NewFile) -> BoxFut<'a, ProviderResult<ProviderFile>> {
        Box::pin(async move {
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
            let version = version_of(&bytes);
            self.remember(&reference, &version);
            Ok(ProviderFile { reference, title: new.title, kind: new.kind, url: None, version: Some(version) })
        })
    }

    fn apply<'a>(&'a self, reference: &'a str, edits: Vec<DocumentEdit>) -> BoxFut<'a, ProviderResult<Option<String>>> {
        Box::pin(async move {
            let (_, kind) = self.path(reference)?;
            if kind == DocumentKind::Doc {
                return self.apply_text(reference, edits).map(Some);
            }
            let charting = edits.iter().any(|e| {
                matches!(
                    e,
                    DocumentEdit::AddChart { .. }
                        | DocumentEdit::UpdateChart { .. }
                        | DocumentEdit::DeleteChart { .. }
                        | DocumentEdit::RenameSheet { .. }
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
            let v = self.apply_cells(reference, &edits)?;
            if let Some(doc) = doc {
                let mut charts = self.store.document_charts(&doc).await.map_err(failed)?;
                let mut changed = false;
                for e in edits {
                    match e {
                        DocumentEdit::AddChart { sheet, id: Some(id), chart } => {
                            self.store
                                .put_document_chart(&doc, &sheet, &Chart { id, spec: chart })
                                .await
                                .map_err(failed)?;
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
                        _ => {}
                    }
                }
                if changed {
                    self.send(reference, DocEvent::Charts);
                }
            }
            Ok(Some(v))
        })
    }

    fn delete<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        Box::pin(async move {
            let (path, _) = self.path(reference)?;
            lock(&self.live).remove(reference);
            std::fs::remove_file(&path).map_err(failed)?;
            let assets = assets_dir(&path);
            if assets.is_dir() {
                std::fs::remove_dir_all(&assets).map_err(failed)?;
            }
            lock(&self.known).remove(reference);
            Ok(())
        })
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
            Ok(CommentReply { id: c.id, author: by.1, body: c.body, created_at: Some(now) })
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
            self.path(reference)?;
            Ok(self.version_now(reference))
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
}
