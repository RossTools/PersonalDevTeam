//! The documents' sensor (PDT-318): every minute (the poll pass) PDT looks at the documents enabled workflows'
//! `document.changed` and `document.comment_added` triggers watch, asks each one's provider for its version and its
//! comments, and logs an event for each change and each new comment or reply. Providers are polled the same way,
//! local or not. The first look at a document only records where it is (`sensor.document_version.<id>`,
//! `sensor.document_comments.<id>`), so turning a watcher on never replays its history. A document nothing watches
//! (its workflows turned off, it unlinked, its provider gone) has that state forgotten, so watching it again starts
//! with a first look too.
//!
//! An event is logged once for each project the document is linked to, or once without a project when it has none
//! (each trigger takes one copy: [`pdt_core::document_watch::accepts_copy`]). What PDT itself wrote (a workflow's
//! comment, an agent's reply: [`mark_ours`], or an agent's or workflow's where the provider says who) is never a new
//! comment, so an agent answering comments doesn't answer itself.

use super::nodes::DocumentTriggerConfig;
use super::{DocumentProvider, provider};
use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::parse;
use pdt_core::document::{Document, DocumentFilter};
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

fn version_key(id: &str) -> String {
    format!("{VERSION_PREFIX}{id}")
}

fn comments_key(id: &str) -> String {
    format!("{COMMENTS_PREFIX}{id}")
}

fn ours_key(id: &str) -> String {
    format!("sensor.document_ours.{id}")
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
    Ok(out)
}

/// Forgets the last look of documents not looked at now (`changes`: their versions; `comments`: their comments), so
/// the next look at one only records again. The ids of PDT's own comments are kept.
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

/// One look at whether the document changed.
async fn look_at_version(st: &AppState, d: &Document, p: &Arc<dyn DocumentProvider>) {
    let version = match p.version(&d.provider_ref).await {
        Ok(Some(v)) => v,
        Ok(None) => return,
        Err(e) => return tracing::debug!(document = %d.id, error = %e, "could not read a document's version"),
    };
    let key = version_key(&d.id);
    let before = match st.store.get_host_value(&key).await {
        Ok(v) => v.and_then(|v| v["version"].as_str().map(String::from)),
        Err(e) => return tracing::warn!(document = %d.id, error = %e, "could not read a document's last version"),
    };
    if before.as_deref() == Some(version.as_str()) {
        return;
    }
    if let Some(prev) = before {
        for doc in copies(d) {
            let key = format!("{}:{prev}:{version}:{}", d.id, doc.project_id);
            let payload = DocumentChanged { document: doc, version: version.clone(), previous_version: prev.clone() };
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
    let (mut versions, mut comments) = (HashSet::new(), HashSet::new());
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
    forget_unwatched(st, COMMENTS_PREFIX, &comments).await;
}
