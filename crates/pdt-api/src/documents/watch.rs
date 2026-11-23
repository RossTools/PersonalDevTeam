//! The documents' sensor (PDT-318): every minute (the poll pass) PDT looks at the documents enabled workflows'
//! `document.changed` and `document.comment_added` triggers watch, and those a watch follows (PDT-616), once each
//! however many watch it. It asks each one's provider for its version and its comments, and logs an event for each
//! change and each new comment or reply. Providers are polled the same way, local or not. The first look at a document
//! only records where it is (`sensor.document_version.<id>`, `sensor.document_comments.<id>`), so turning a watcher on
//! never replays its history. A document nothing watches (its workflows turned off, it unlinked, its provider gone, its
//! watches ended) has that state forgotten, so watching it again starts with a first look too.
//!
//! For changes, the sensor also keeps the document's content as last read (`sensor.document_text.<id>`): a change's
//! event carries the diff from it (`diff`), and who edited it where the provider says (`modifiedBy`). A watch diffs
//! its own snapshot against it, so a burst of edits is told as one diff. A document its provider no longer has is
//! marked (`sensor.document_missing.<id>`), which ends its watches.
//!
//! An event is logged once for each project the document is linked to, or once without a project when it has none
//! (each trigger takes one copy: [`pdt_core::document_watch::accepts_copy`]). What PDT itself wrote (a workflow's
//! comment, an agent's reply: [`mark_ours`], or an agent's or workflow's where the provider says who) is never a new
//! comment, so an agent answering comments doesn't answer itself.

use super::nodes::DocumentTriggerConfig;
use super::{DocumentProvider, provider};
use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::parse;
use pdt_core::document::{Document, DocumentContent, DocumentFilter};
use pdt_core::document_watch::{comment_ids, new_comments};
use pdt_core::workflow::api::{NewLogEvent, Scope};
use pdt_core::workflow::events::{
    DOCUMENT_CHANGED, DOCUMENT_COMMENT_ADDED, DocumentChanged, DocumentCommentAdded, DocumentRef,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

/// How many of PDT's own comment ids are kept per document ([`mark_ours`]).
const OURS_KEPT: usize = 200;

/// What is watched on a document.
#[derive(Debug, Default, Clone, Copy)]
struct Watch {
    changes: bool,
    comments: bool,
}

const VERSION_PREFIX: &str = "sensor.document_version.";
const COMMENTS_PREFIX: &str = "sensor.document_comments.";
const TEXT_PREFIX: &str = "sensor.document_text.";
const MISSING_PREFIX: &str = "sensor.document_missing.";
const OURS_PREFIX: &str = "sensor.document_ours.";
/// The largest content kept to diff against (its JSON); a larger document's changes come without a diff.
const TEXT_MAX: usize = 2 * 1024 * 1024;

fn version_key(id: &str) -> String {
    format!("{VERSION_PREFIX}{id}")
}

fn comments_key(id: &str) -> String {
    format!("{COMMENTS_PREFIX}{id}")
}

fn text_key(id: &str) -> String {
    format!("{TEXT_PREFIX}{id}")
}

fn missing_key(id: &str) -> String {
    format!("{MISSING_PREFIX}{id}")
}

/// The document's content as the sensor last read it (to diff a watch's snapshot against); None before its first
/// look, or for one too large to keep.
pub(crate) async fn last_read(st: &AppState, id: &str) -> Option<DocumentContent> {
    let v = st.store.get_host_value(&text_key(id)).await.ok().flatten()?;
    serde_json::from_value(v["content"].clone()).ok()
}

/// A new watch's snapshot, to diff its first change against: the sensor's last read, else a read now. None when the
/// sensor found it too large to keep (or a read now is), or its provider is gone or can't read it (then the sensor's
/// first read is taken on a later tick).
pub(crate) async fn first_snapshot(st: &AppState, id: &str) -> Option<DocumentContent> {
    if let Some(v) = st.store.get_host_value(&text_key(id)).await.ok().flatten() {
        return serde_json::from_value(v["content"].clone()).ok();
    }
    let d = st.store.get_document(id).await.ok().flatten()?;
    let p = provider(st, &d.provider)?;
    let c = read_content(&p, &d.provider_ref).await?;
    tokio::task::spawn_blocking(move || keepable(&c).then_some(c)).await.ok().flatten()
}

/// Whether content is small enough to keep (its JSON within [`TEXT_MAX`]).
fn keepable(c: &DocumentContent) -> bool {
    serde_json::to_vec(c).is_ok_and(|j| j.len() <= TEXT_MAX)
}

/// The diff from `before` to `now` (None when there is nothing to show or it is too large), worked out off the
/// runtime: the paragraph diff of two large documents takes a while.
pub(crate) async fn diff(before: DocumentContent, now: DocumentContent) -> Option<String> {
    tokio::task::spawn_blocking(move || pdt_core::document_diff::diff(&before, &now, pdt_core::document_diff::BUDGET))
        .await
        .ok()
        .flatten()
}

/// Whether the provider said it has no such document at the last look (deleted, or access taken away).
pub(crate) async fn missing(st: &AppState, id: &str) -> bool {
    matches!(st.store.get_host_value(&missing_key(id)).await, Ok(Some(_)))
}

/// Its content as a provider reads it, without the version (which a diff ignores).
pub(crate) async fn read_content(p: &Arc<dyn DocumentProvider>, reference: &str) -> Option<DocumentContent> {
    let mut c = p.read(reference).await.ok()?;
    match &mut c {
        DocumentContent::Doc { version, .. } | DocumentContent::Sheet { version, .. } => *version = None,
    }
    Some(c)
}

fn ours_key(id: &str) -> String {
    format!("{OURS_PREFIX}{id}")
}

/// Remembers that PDT wrote comment (or reply) `id` on document `document` (a workflow's, an agent's tool call): it
/// never counts as a new comment, even where the provider can't tell PDT's from the user's (Google writes both as the
/// signed-in account).
pub(crate) async fn mark_ours(st: &AppState, document: &str, id: &str) {
    let key = ours_key(document);
    let mut ids: Vec<String> = match st.store.get_host_value(&key).await {
        Ok(v) => v.and_then(|v| serde_json::from_value(v["ids"].clone()).ok()).unwrap_or_default(),
        Err(e) => return tracing::warn!(document, error = %e, "could not read a document's own comments"),
    };
    ids.push(id.to_string());
    let skip = ids.len().saturating_sub(OURS_KEPT);
    if let Err(e) = st.store.put_host_value(&key, &json!({"ids": &ids[skip..]})).await {
        tracing::warn!(document, error = %e, "could not record a document's own comment");
    }
}

/// The documents enabled workflows watch, by id; Err when the store can't say (then nothing is forgotten).
async fn watched(st: &AppState) -> Result<BTreeMap<String, Watch>, pdt_store::StoreError> {
    let mut out: BTreeMap<String, Watch> = BTreeMap::new();
    let mut all: Option<Vec<Document>> = None;
    let reg = st.registry();
    for wf in st.store.enabled_workflows().await? {
        let Some(doc) = st.store.workflow_doc(&wf.id, wf.version).await? else { continue };
        for node in doc.nodes.iter().filter(|n| n.kind == DOCUMENT_CHANGED || n.kind == DOCUMENT_COMMENT_ADDED) {
            let Ok(c) = reg.catalog().current_config(node).and_then(|c| parse::<DocumentTriggerConfig>(&c)) else {
                continue;
            };
            let ids: Vec<String> = match c.document.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                Some(d) => match super::tools::find(st, d).await {
                    Ok(found) => vec![found.id],
                    Err(_) => continue,
                },
                None => {
                    if all.is_none() {
                        all = Some(st.store.list_documents(&DocumentFilter::default()).await?);
                    }
                    let linked = |d: &&Document, p: &str| d.projects.iter().any(|x| x.id == p);
                    all.iter()
                        .flatten()
                        .filter(|d| match wf.scope {
                            Scope::Project => wf.project_id.as_deref().is_some_and(|p| linked(d, p)),
                            Scope::Host => {
                                !d.projects.is_empty()
                                    && (c.projects.is_empty() || c.projects.iter().any(|p| linked(d, p)))
                            }
                            Scope::Personal => d.projects.is_empty(),
                        })
                        .map(|d| d.id.clone())
                        .collect()
                }
            };
            for id in ids {
                let w = out.entry(id).or_default();
                if node.kind == DOCUMENT_CHANGED {
                    w.changes = true;
                } else {
                    w.comments = true;
                }
            }
        }
    }
    for w in st.store.active_watches().await? {
        if w.target != pdt_core::watch::WatchTarget::Document {
            continue;
        }
        let e = out.entry(w.target_id).or_default();
        e.changes |= w.events.contains(&pdt_core::watch::WatchEvent::Changed);
        e.comments |= w.events.contains(&pdt_core::watch::WatchEvent::Comment);
    }
    Ok(out)
}

/// Forgets the last look of documents not looked at now (`changes`: their versions; `comments`: their comments and
/// the ids of PDT's own), so the next look at one only records again.
async fn forget_unwatched(st: &AppState, prefix: &str, looked: &HashSet<String>) {
    let keys = match st.store.host_keys_with_prefix(prefix).await {
        Ok(k) => k,
        Err(e) => return tracing::warn!(error = %e, "could not list what documents were seen"),
    };
    for key in keys.iter().filter(|k| !looked.contains(&k[prefix.len()..])) {
        if let Err(e) = st.store.delete_host_value(key).await {
            tracing::warn!(key, error = %e, "could not forget what a document was seen at");
        }
    }
}

/// A document's reference for its events: one per project it is linked to (or one without a project).
fn copies(d: &Document) -> Vec<DocumentRef> {
    let base = DocumentRef {
        project_id: String::new(),
        document_id: d.id.clone(),
        title: d.title.clone(),
        kind: d.kind.as_str().into(),
        provider: d.provider.clone(),
        url: d.url.clone(),
        project_ids: d.projects.iter().map(|p| p.id.clone()).collect(),
        tickets: d.tickets.iter().map(|t| t.key.clone()).collect(),
    };
    if base.project_ids.is_empty() {
        return vec![base];
    }
    base.project_ids.iter().map(|p| DocumentRef { project_id: p.clone(), ..base.clone() }).collect()
}

/// Logs `e`; false when it could not be written (the next look tries again).
async fn emit(st: &AppState, e: NewLogEvent) -> bool {
    match st.store.emit_event(&e, now_ms()).await {
        Ok(_) => true,
        Err(x) => {
            tracing::warn!(kind = %e.kind, key = %e.key, error = %x, "could not log a document event");
            false
        }
    }
}

/// Marks the document missing at its provider (or not).
async fn mark_missing(st: &AppState, id: &str, gone: bool) {
    let key = missing_key(id);
    let r = match (gone, missing(st, id).await) {
        (true, false) => st.store.put_host_value(&key, &json!({"at": now_ms()})).await,
        (false, true) => st.store.delete_host_value(&key).await.map(|_| ()),
        _ => Ok(()),
    };
    if let Err(e) = r {
        tracing::warn!(document = id, error = %e, "could not record whether a document is missing");
    }
}

/// Reads the document and keeps its content to diff the next change against: what changed since the last read kept.
async fn keep_text(st: &AppState, d: &Document, p: &Arc<dyn DocumentProvider>) -> Option<String> {
    let now = read_content(p, &d.provider_ref).await?;
    let before = last_read(st, &d.id).await;
    // Diffing and measuring up to `TEXT_MAX` of content each take a while: off the runtime.
    let (diff, kept) = tokio::task::spawn_blocking(move || {
        let diff = before.and_then(|b| pdt_core::document_diff::diff(&b, &now, pdt_core::document_diff::BUDGET));
        let kept = if keepable(&now) { json!({"content": now}) } else { json!({"tooLarge": true}) };
        (diff, kept)
    })
    .await
    .ok()?;
    if let Err(e) = st.store.put_host_value(&text_key(&d.id), &kept).await {
        tracing::warn!(document = %d.id, error = %e, "could not keep a document's text");
    }
    diff
}

/// One look at whether the document changed.
async fn look_at_version(st: &AppState, d: &Document, p: &Arc<dyn DocumentProvider>) {
    let version = match p.version(&d.provider_ref).await {
        Ok(Some(v)) => v,
        Ok(None) => return,
        Err(super::ProviderError::NotFound) => return mark_missing(st, &d.id, true).await,
        Err(e) => return tracing::debug!(document = %d.id, error = %e, "could not read a document's version"),
    };
    mark_missing(st, &d.id, false).await;
    let key = version_key(&d.id);
    let before = match st.store.get_host_value(&key).await {
        Ok(v) => v.and_then(|v| v["version"].as_str().map(String::from)),
        Err(e) => return tracing::warn!(document = %d.id, error = %e, "could not read a document's last version"),
    };
    if before.as_deref() == Some(version.as_str()) {
        // Kept since the first look; one whose read failed is read again.
        if matches!(st.store.get_host_value(&text_key(&d.id)).await, Ok(None)) {
            keep_text(st, d, p).await;
        }
        return;
    }
    let diff = keep_text(st, d, p).await;
    if let Some(prev) = before {
        let modified_by = p.file(&d.provider_ref).await.ok().and_then(|f| f.modified_by);
        for doc in copies(d) {
            let key = format!("{}:{prev}:{version}:{}", d.id, doc.project_id);
            let payload = DocumentChanged {
                document: doc,
                version: version.clone(),
                previous_version: prev.clone(),
                diff: diff.clone(),
                modified_by: modified_by.clone(),
            };
            if !emit(st, NewLogEvent::new(DOCUMENT_CHANGED, key, "documents", payload)).await {
                return;
            }
        }
    }
    if let Err(e) = st.store.put_host_value(&key, &json!({"version": version})).await {
        tracing::warn!(document = %d.id, error = %e, "could not record a document's version");
    }
}

fn id_set(v: Option<Value>, field: &str) -> Option<HashSet<String>> {
    v.and_then(|v| serde_json::from_value::<Vec<String>>(v[field].clone()).ok()).map(|v| v.into_iter().collect())
}

/// One look for new comments on the document.
async fn look_at_comments(st: &AppState, d: &Document, p: &Arc<dyn DocumentProvider>) {
    if !p.capabilities().comments {
        return;
    }
    let list = match p.comments(&d.provider_ref).await {
        Ok(l) => l,
        Err(super::ProviderError::NotFound) => return mark_missing(st, &d.id, true).await,
        Err(e) => return tracing::debug!(document = %d.id, error = %e, "could not read a document's comments"),
    };
    let key = comments_key(&d.id);
    let seen = match st.store.get_host_value(&key).await {
        Ok(v) => id_set(v, "seen"),
        Err(e) => return tracing::warn!(document = %d.id, error = %e, "could not read a document's seen comments"),
    };
    if let Some(seen) = seen {
        let ours = id_set(st.store.get_host_value(&ours_key(&d.id)).await.ok().flatten(), "ids").unwrap_or_default();
        for c in new_comments(&list, &seen, &ours) {
            for doc in copies(d) {
                let key = format!("{}:{}:{}", d.id, c.comment_id, doc.project_id);
                let payload = DocumentCommentAdded {
                    document: doc,
                    comment_id: c.comment_id.clone(),
                    thread: c.thread.clone(),
                    reply: c.reply,
                    author: c.author.clone(),
                    by_user: c.by_user,
                    body: c.body.clone(),
                    quote: c.quote.clone(),
                };
                if !emit(st, NewLogEvent::new(DOCUMENT_COMMENT_ADDED, key, "documents", payload)).await {
                    return;
                }
            }
        }
    }
    if let Err(e) = st.store.put_host_value(&key, &json!({"seen": comment_ids(&list)})).await {
        tracing::warn!(document = %d.id, error = %e, "could not record a document's comments");
    }
}

/// One look at every watched document. pdtd runs it every 60 s (the documents integration's poller). Never fails;
/// problems are logged.
pub async fn refresh(st: &AppState) {
    let watched = match watched(st).await {
        Ok(w) => w,
        Err(e) => return tracing::warn!(error = %e, "could not read which documents workflows watch"),
    };
    let (mut versions, mut comments, mut looked) = (HashSet::new(), HashSet::new(), HashSet::new());
    for (id, w) in watched {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let d = match st.store.get_document(&id).await {
            Ok(Some(d)) => d,
            Ok(None) => continue,
            // Not read: keep what was seen of the rest too.
            Err(e) => return tracing::warn!(document = %id, error = %e, "could not read a watched document"),
        };
        let Some(p) = provider(st, &d.provider) else { continue };
        looked.insert(id.clone());
        if w.changes {
            look_at_version(st, &d, &p).await;
            versions.insert(id.clone());
        }
        if w.comments {
            look_at_comments(st, &d, &p).await;
            comments.insert(id);
        }
    }
    forget_unwatched(st, VERSION_PREFIX, &versions).await;
    forget_unwatched(st, TEXT_PREFIX, &versions).await;
    forget_unwatched(st, MISSING_PREFIX, &looked).await;
    forget_unwatched(st, COMMENTS_PREFIX, &comments).await;
    // A first look records every comment there is as seen, PDT's own too: nothing to keep them for meanwhile.
    forget_unwatched(st, OURS_PREFIX, &comments).await;
}
