//! Documents (PDT-314): docs and sheets kept by a [`DocumentProvider`] (PDT's local one, or a plugin's such as
//! Google's), linked to projects and tickets. PDT keeps a record of each (`pdt_store`'s `documents`) and asks the
//! provider for the rest: search, create, read, edit, comments. Like forges, a provider declares what it can do
//! ([`ProviderCapabilities`]) and PDT offers only that.
//!
//! Providers come from [`AppState::documents`] (PDT's own) and the enabled integrations
//! ([`crate::Integration::document_providers`]); [`providers`] lists them.

pub(crate) mod routes;
pub(crate) mod tools;

use crate::routes::AppState;
use crate::workflow::catalog::BoxFut;
pub use pdt_core::document::{
    CommentReply, DocumentComment, DocumentContent, DocumentEdit, DocumentKind, ProviderCapabilities, ProviderFile,
    ProviderInfo,
};
use std::sync::Arc;

/// Why a provider could not do something.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// The provider can't do this at all (see its capabilities), or not for this kind of document.
    #[error("{0}")]
    Unsupported(String),
    /// The provider has no such file (deleted, or access was taken away).
    #[error("the provider has no such document")]
    NotFound,
    /// The edit or input doesn't fit the document (a cell that isn't one, a sheet it lacks).
    #[error("{0}")]
    Invalid(String),
    /// Anything else (the service is down, a sign-in expired).
    #[error("{0}")]
    Failed(String),
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// A document to make ([`DocumentProvider::create`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFile {
    pub title: String,
    pub kind: DocumentKind,
    /// A text document's first content (Markdown); a new sheet starts empty.
    pub markdown: Option<String>,
    /// The local documents folder for it (its project's, else the host's): where a provider that keeps files on
    /// this machine puts it. Others ignore it.
    pub folder: std::path::PathBuf,
}

fn unsupported<'a, T: Send + 'a>(what: &str) -> BoxFut<'a, ProviderResult<T>> {
    let e = ProviderError::Unsupported(what.to_string());
    Box::pin(async move { Err(e) })
}

/// A document provider (PDT-314). Every one lists and reads its documents; the rest is optional, declared in
/// [`DocumentProvider::capabilities`] (the defaults answer [`ProviderError::Unsupported`]). References are the
/// provider's own (a file id, a path): PDT keeps them and gives them back.
pub trait DocumentProvider: Send + Sync {
    /// Its id, kept on each document (`local`, `google`): see [`pdt_core::document::is_provider_id`].
    fn id(&self) -> &'static str;
    /// Its name for people (`Local`, `Google`).
    fn label(&self) -> &'static str;
    fn capabilities(&self) -> ProviderCapabilities;

    /// The file a reference names (to import or link it): its title, kind, URL and version.
    fn file<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<ProviderFile>>;
    /// Its content: a doc as Markdown, a sheet as its sheets of cells (values and formulas).
    fn read<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<DocumentContent>>;

    /// Files to import or link, by words in their names (all of them for none), newest first.
    fn search<'a>(&'a self, _q: &'a str, _kind: Option<DocumentKind>) -> BoxFut<'a, ProviderResult<Vec<ProviderFile>>> {
        unsupported(&format!("{} can't search for documents", self.label()))
    }
    /// Makes a document.
    fn create<'a>(&'a self, _new: NewFile) -> BoxFut<'a, ProviderResult<ProviderFile>> {
        unsupported(&format!("{} can't make documents", self.label()))
    }
    /// Changes its content, in order; the new version.
    fn apply<'a>(
        &'a self,
        _reference: &'a str,
        _edits: Vec<DocumentEdit>,
    ) -> BoxFut<'a, ProviderResult<Option<String>>> {
        unsupported(&format!("{} can't change documents", self.label()))
    }
    fn rename<'a>(&'a self, _reference: &'a str, _title: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        unsupported(&format!("{} can't rename documents", self.label()))
    }
    /// Deletes it where it is kept (moves it to the bin, where the provider has one).
    fn delete<'a>(&'a self, _reference: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        unsupported(&format!("{} can't delete documents", self.label()))
    }
    fn comments<'a>(&'a self, _reference: &'a str) -> BoxFut<'a, ProviderResult<Vec<DocumentComment>>> {
        unsupported(&format!("{} has no comments", self.label()))
    }
    fn reply<'a>(
        &'a self,
        _reference: &'a str,
        _comment: &'a str,
        _body: &'a str,
    ) -> BoxFut<'a, ProviderResult<CommentReply>> {
        unsupported(&format!("{} has no comments", self.label()))
    }
    fn resolve<'a>(&'a self, _reference: &'a str, _comment: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        unsupported(&format!("{} has no comments", self.label()))
    }
    /// Its version or etag now, which changes whenever it does (change detection); by default the file's.
    fn version<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<Option<String>>> {
        Box::pin(async move { Ok(self.file(reference).await?.version) })
    }
    /// A page that shows it inside PDT, when the provider has one.
    fn embed_url(&self, _reference: &str) -> Option<String> {
        None
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo { id: self.id().into(), label: self.label().into(), capabilities: self.capabilities() }
    }
}

/// PDT's own document providers (PDT-314); the enabled integrations add theirs ([`providers`]).
#[derive(Clone, Default)]
pub struct DocumentProviders {
    list: Arc<[Arc<dyn DocumentProvider>]>,
}

impl DocumentProviders {
    pub fn new(list: Vec<Arc<dyn DocumentProvider>>) -> Self {
        Self { list: list.into() }
    }
    pub fn list(&self) -> &[Arc<dyn DocumentProvider>] {
        &self.list
    }
}

/// Every provider this instance has now: PDT's own, then the enabled integrations', each id once.
pub fn providers(st: &AppState) -> Vec<Arc<dyn DocumentProvider>> {
    let mut out: Vec<Arc<dyn DocumentProvider>> = st.documents.list().to_vec();
    for i in st.registry().integrations() {
        for p in i.document_providers() {
            if !out.iter().any(|o| o.id() == p.id()) {
                out.push(p);
            }
        }
    }
    out
}

/// The provider with this id, if this instance has it now.
pub fn provider(st: &AppState, id: &str) -> Option<Arc<dyn DocumentProvider>> {
    providers(st).into_iter().find(|p| p.id() == id)
}

/// An in-memory provider for tests and the demo data: docs and sheets, search, edits, comments.
pub mod testing {
    use super::*;
    use pdt_core::document::Sheet;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Debug, Clone)]
    struct Stored {
        title: String,
        content: DocumentContent,
        version: u64,
        comments: Vec<DocumentComment>,
    }

    /// Keeps documents in memory under references `<id>:<n>` (put) or `<id>:<uuid>` (created). Its `capabilities` can be narrowed.
    pub struct FakeProvider {
        id: &'static str,
        label: &'static str,
        pub capabilities: Mutex<ProviderCapabilities>,
        files: Mutex<BTreeMap<String, Stored>>,
        next: Mutex<u64>,
        /// The folders `create` was given, in order.
        pub folders: Mutex<Vec<std::path::PathBuf>>,
    }

    impl FakeProvider {
        /// A provider that can do everything.
        pub fn new(id: &'static str, label: &'static str) -> Self {
            Self {
                id,
                label,
                capabilities: Mutex::new(ProviderCapabilities {
                    docs: true,
                    sheets: true,
                    create: true,
                    search: true,
                    edit: true,
                    rename: true,
                    delete: true,
                    comments: true,
                    embed: false,
                }),
                files: Default::default(),
                next: Mutex::new(0),
                folders: Default::default(),
            }
        }

        /// Puts a file in place (as if the user had it already), with a comment on a doc; its reference.
        pub fn put(&self, title: &str, kind: DocumentKind, markdown: &str) -> String {
            let r = self.next_ref();
            let content = match kind {
                DocumentKind::Doc => DocumentContent::Doc { markdown: markdown.into(), version: None },
                DocumentKind::Sheet => DocumentContent::Sheet { sheets: vec![sheet("Sheet1")], version: None },
            };
            let comments = vec![DocumentComment {
                id: "c1".into(),
                author: "Sam".into(),
                body: "Is this still right?".into(),
                quote: None,
                resolved: false,
                created_at: None,
                replies: vec![],
            }];
            self.lock().insert(r.clone(), Stored { title: title.into(), content, version: 1, comments });
            r
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Stored>> {
            self.files.lock().unwrap_or_else(|e| e.into_inner())
        }

        /// `put`'s are numbered (the same each time the demo data puts them); `create`'s are unique.
        fn next_ref(&self) -> String {
            let mut n = self.next.lock().unwrap_or_else(|e| e.into_inner());
            *n += 1;
            format!("{}:{n}", self.id)
        }

        fn new_ref(&self) -> String {
            format!("{}:{}", self.id, uuid::Uuid::now_v7().simple())
        }

        fn caps(&self) -> ProviderCapabilities {
            *self.capabilities.lock().unwrap_or_else(|e| e.into_inner())
        }

        fn to_file(&self, reference: &str, s: &Stored) -> ProviderFile {
            ProviderFile {
                reference: reference.into(),
                title: s.title.clone(),
                kind: match s.content {
                    DocumentContent::Doc { .. } => DocumentKind::Doc,
                    DocumentContent::Sheet { .. } => DocumentKind::Sheet,
                },
                url: None,
                version: Some(s.version.to_string()),
            }
        }

        fn with<T>(&self, reference: &str, f: impl FnOnce(&mut Stored) -> ProviderResult<T>) -> ProviderResult<T> {
            f(self.lock().get_mut(reference).ok_or(ProviderError::NotFound)?)
        }
    }

    fn sheet(name: &str) -> Sheet {
        Sheet { name: name.into(), ..Default::default() }
    }

    fn ready<'a, T: Send + 'a>(r: ProviderResult<T>) -> BoxFut<'a, ProviderResult<T>> {
        Box::pin(async move { r })
    }

    /// Text edits as Markdown, sheet edits on plain cells (pdt-core's `document_ops`): formats, conditional
    /// formats and filters are unsupported, as a provider without them answers.
    fn apply_edit(content: &mut DocumentContent, edit: DocumentEdit) -> ProviderResult<()> {
        use pdt_core::document_ops::{EditError, apply_doc_edit, apply_sheet_edit};
        let r = match content {
            DocumentContent::Doc { markdown, .. } => apply_doc_edit(markdown, &edit).map(|m| *markdown = m),
            DocumentContent::Sheet { sheets, .. } => apply_sheet_edit(sheets, edit),
        };
        r.map_err(|e| match e {
            EditError::Unsupported(m) => ProviderError::Unsupported(m),
            EditError::Invalid(m) => ProviderError::Invalid(m),
        })
    }

    impl DocumentProvider for FakeProvider {
        fn id(&self) -> &'static str {
            self.id
        }
        fn label(&self) -> &'static str {
            self.label
        }
        fn capabilities(&self) -> ProviderCapabilities {
            self.caps()
        }
        fn file<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<ProviderFile>> {
            ready(self.with(reference, |s| Ok(s.clone())).map(|s| self.to_file(reference, &s)))
        }
        fn read<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<DocumentContent>> {
            ready(self.with(reference, |s| {
                let mut c = s.content.clone();
                let v = Some(s.version.to_string());
                match &mut c {
                    DocumentContent::Doc { version, .. } | DocumentContent::Sheet { version, .. } => *version = v,
                }
                Ok(c)
            }))
        }
        fn search<'a>(
            &'a self,
            q: &'a str,
            kind: Option<DocumentKind>,
        ) -> BoxFut<'a, ProviderResult<Vec<ProviderFile>>> {
            if !self.caps().search {
                return unsupported("this provider can't search");
            }
            let q = q.to_lowercase();
            let mut out: Vec<ProviderFile> = self
                .lock()
                .iter()
                .map(|(r, s)| self.to_file(r, s))
                .filter(|f| q.split_whitespace().all(|w| f.title.to_lowercase().contains(w)))
                .filter(|f| kind.is_none_or(|k| k == f.kind))
                .collect();
            out.reverse();
            ready(Ok(out))
        }
        fn create<'a>(&'a self, new: NewFile) -> BoxFut<'a, ProviderResult<ProviderFile>> {
            if !self.caps().create {
                return unsupported("this provider can't make documents");
            }
            self.folders.lock().unwrap_or_else(|e| e.into_inner()).push(new.folder.clone());
            let r = self.new_ref();
            let content = match new.kind {
                DocumentKind::Doc => DocumentContent::Doc { markdown: new.markdown.unwrap_or_default(), version: None },
                DocumentKind::Sheet => DocumentContent::Sheet { sheets: vec![sheet("Sheet1")], version: None },
            };
            let s = Stored { title: new.title, content, version: 1, comments: vec![] };
            let f = self.to_file(&r, &s);
            self.lock().insert(r, s);
            ready(Ok(f))
        }
        fn apply<'a>(
            &'a self,
            reference: &'a str,
            edits: Vec<DocumentEdit>,
        ) -> BoxFut<'a, ProviderResult<Option<String>>> {
            ready(self.with(reference, |s| {
                let mut content = s.content.clone();
                for e in edits {
                    apply_edit(&mut content, e)?;
                }
                s.content = content;
                s.version += 1;
                Ok(Some(s.version.to_string()))
            }))
        }
        fn rename<'a>(&'a self, reference: &'a str, title: &'a str) -> BoxFut<'a, ProviderResult<()>> {
            ready(self.with(reference, |s| {
                s.title = title.into();
                Ok(())
            }))
        }
        fn delete<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<()>> {
            ready(self.lock().remove(reference).map(|_| ()).ok_or(ProviderError::NotFound))
        }
        fn comments<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<Vec<DocumentComment>>> {
            ready(self.with(reference, |s| Ok(s.comments.clone())))
        }
        fn reply<'a>(
            &'a self,
            reference: &'a str,
            comment: &'a str,
            body: &'a str,
        ) -> BoxFut<'a, ProviderResult<CommentReply>> {
            ready(self.with(reference, |s| {
                let c = s.comments.iter_mut().find(|c| c.id == comment).ok_or(ProviderError::NotFound)?;
                let r = CommentReply {
                    id: format!("{comment}.{}", c.replies.len() + 1),
                    author: "PDT".into(),
                    body: body.into(),
                    created_at: None,
                };
                c.replies.push(r.clone());
                Ok(r)
            }))
        }
        fn resolve<'a>(&'a self, reference: &'a str, comment: &'a str) -> BoxFut<'a, ProviderResult<()>> {
            ready(self.with(reference, |s| {
                s.comments.iter_mut().find(|c| c.id == comment).ok_or(ProviderError::NotFound)?.resolved = true;
                Ok(())
            }))
        }
    }
}
