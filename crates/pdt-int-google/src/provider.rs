//! The Google document provider (PDT-316): Docs and Sheets in the user's Drive, through each signed-in Google account.
//! A document's reference is `<connection id>/<file id>`, so it is always reached as the account that has it. New
//! documents go to the first signed-in account (by connection name), in its connection's Drive folder. Called from an
//! agent's document tools, it acts as the agent (PDT-360): with each account's agent sign-in, and not at all with an
//! account that doesn't let agents in.

use crate::accounts::{self, Shared, config_str};
use crate::api::{Account, Session};
use crate::drive;
use pdt_api::sdk::documents::{
    CommentReply, DocumentComment, DocumentContent, DocumentEdit, DocumentKind, DocumentProvider, NewFile,
    ProviderCapabilities, ProviderError, ProviderFile, ProviderPicker, ProviderResult, by_agent,
};
use pdt_api::sdk::{AppState, BoxFut};
use std::sync::Arc;

pub const ID: &str = "google";

pub struct GoogleDocs {
    pub st: AppState,
    pub shared: Arc<Shared>,
}

/// No account to act as: none is signed in, or, for an agent, none lets agents in.
fn no_account(agent: bool) -> ProviderError {
    ProviderError::Failed(if agent {
        "no Google account lets agents use it: the user allows them in Settings → Plugins → Google (Allow agents)"
            .into()
    } else {
        "no Google account is signed in: sign in in Settings → Plugins → Google".into()
    })
}

/// A reference's connection and file ids. The file id is Drive's (letters, digits, `-` and `_`, as `folder_id`
/// takes), so it can't reach another path when it goes into a URL (PDT-421).
pub fn split_ref(reference: &str) -> ProviderResult<(&str, &str)> {
    let id_char = |b: u8| b.is_ascii_alphanumeric() || b == b'-' || b == b'_';
    reference
        .split_once('/')
        .filter(|(c, f)| !c.is_empty() && !f.is_empty() && f.bytes().all(id_char))
        .ok_or_else(|| ProviderError::Invalid(format!("{reference} is not a Google document's reference")))
}

impl GoogleDocs {
    /// Runs `f` as `account`, off the async workers (Google's calls block).
    async fn run<T: Send + 'static>(
        &self,
        account: Account,
        f: impl FnOnce(&Session) -> ProviderResult<T> + Send + 'static,
    ) -> ProviderResult<T> {
        let shared = self.shared.clone();
        tokio::task::spawn_blocking(move || f(&Session::new(shared.transport.as_ref(), &account, &shared.tokens)))
            .await
            .map_err(|e| ProviderError::Failed(e.to_string()))?
    }

    /// Runs `f` on the file a reference names, as its account.
    async fn on<T: Send + 'static>(
        &self,
        reference: &str,
        f: impl FnOnce(&Session, &str) -> ProviderResult<T> + Send + 'static,
    ) -> ProviderResult<T> {
        let (conn, file) = split_ref(reference)?;
        let (_, account) = accounts::account_by_id(&self.st, &self.shared, conn, by_agent()).await?;
        let file = file.to_string();
        self.run(account, move |s| f(s, &file)).await
    }
}

impl DocumentProvider for GoogleDocs {
    fn id(&self) -> &'static str {
        ID
    }
    fn label(&self) -> &'static str {
        "Google"
    }
    fn picker(&self) -> Option<ProviderPicker> {
        Some(ProviderPicker {
            label: "Browse Drive".into(),
            module: "/api/google/picker.js".into(),
            accounts: "/api/google/accounts".into(),
            help: "To pick any file from your Drive, sign in to Google in Settings → Plugins → Google.".into(),
            unavailable: Some(
                "This build of PDT doesn't include Google's file picker. You can still search the files PDT can \
                 open below."
                    .into(),
            ),
            setup: Some(
                "To pick any file from your Drive, give the account the picker API key and project number of its \
                 Google Cloud project."
                    .into(),
            ),
            // drive.file (oauth::SCOPES): Drive lists only the files PDT made or was given through the picker.
            search_note: Some("PDT only sees Drive files it made or you opened through it.".into()),
        })
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            docs: true,
            sheets: true,
            create: true,
            search: true,
            edit: true,
            rename: true,
            delete: true,
            comments: true,
            embed: true,
            charts: false,
            live: false,
        }
    }

    fn file<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<ProviderFile>> {
        Box::pin(self.on(reference, drive::file))
    }

    fn read<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<DocumentContent>> {
        Box::pin(self.on(reference, drive::read))
    }

    /// Every signed-in account's Docs and Sheets PDT may open; an account that fails is left out, unless all do.
    fn search<'a>(&'a self, q: &'a str, kind: Option<DocumentKind>) -> BoxFut<'a, ProviderResult<Vec<ProviderFile>>> {
        Box::pin(async move {
            let agent = by_agent();
            let all = accounts::signed_in(&self.st, &self.shared, agent).await?;
            if all.is_empty() {
                return Err(no_account(agent));
            }
            let (mut out, mut failed) = (vec![], None);
            for (_, a) in all {
                let q = q.to_string();
                match self.run(a, move |s| drive::search(s, &q, kind)).await {
                    Ok(files) => out.extend(files),
                    Err(e) => failed = Some(e),
                }
            }
            match failed {
                Some(e) if out.is_empty() => Err(e),
                _ => Ok(out),
            }
        })
    }

    fn create<'a>(&'a self, new: NewFile) -> BoxFut<'a, ProviderResult<ProviderFile>> {
        Box::pin(async move {
            let agent = by_agent();
            let all = accounts::signed_in(&self.st, &self.shared, agent).await?;
            let (c, a) = all.into_iter().next().ok_or_else(|| no_account(agent))?;
            let folder = config_str(&c, "folder").and_then(|f| drive::folder_id(&f));
            self.run(a, move |s| {
                let f = drive::create(s, &new.title, new.kind, folder.as_deref())?;
                match new.markdown.filter(|m| !m.trim().is_empty()) {
                    Some(m) if new.kind == DocumentKind::Doc => {
                        let (_, id) = split_ref(&f.reference)?;
                        let version = drive::apply(s, id, &[DocumentEdit::ReplaceText { markdown: m }])?;
                        Ok(ProviderFile { version, ..f })
                    }
                    _ => Ok(f),
                }
            })
            .await
        })
    }

    fn apply<'a>(&'a self, reference: &'a str, edits: Vec<DocumentEdit>) -> BoxFut<'a, ProviderResult<Option<String>>> {
        Box::pin(self.on(reference, move |s, f| drive::apply(s, f, &edits)))
    }

    fn rename<'a>(&'a self, reference: &'a str, title: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        let title = title.to_string();
        Box::pin(self.on(reference, move |s, f| drive::rename(s, f, &title)))
    }

    fn delete<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        Box::pin(self.on(reference, drive::trash))
    }

    fn comments<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<Vec<DocumentComment>>> {
        Box::pin(self.on(reference, drive::comments))
    }

    fn reply<'a>(
        &'a self,
        reference: &'a str,
        comment: &'a str,
        body: &'a str,
        // Google writes a reply as the signed-in account.
        _by: (pdt_core::ticket::Actor, String),
    ) -> BoxFut<'a, ProviderResult<CommentReply>> {
        let (comment, body) = (comment.to_string(), body.to_string());
        Box::pin(self.on(reference, move |s, f| drive::reply(s, f, &comment, &body)))
    }

    fn resolve<'a>(&'a self, reference: &'a str, comment: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        let comment = comment.to_string();
        Box::pin(self.on(reference, move |s, f| drive::resolve(s, f, &comment)))
    }

    fn version<'a>(&'a self, reference: &'a str) -> BoxFut<'a, ProviderResult<Option<String>>> {
        Box::pin(self.on(reference, drive::version))
    }

    fn embed_url(&self, reference: &str, kind: DocumentKind) -> Option<String> {
        let (conn, file) = split_ref(reference).ok()?;
        Some(drive::embed_url(file, kind, self.shared.email(conn).as_deref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{KIND, SignIn, sign_in_key, store_sign_in};
    use crate::api::DRIVE;
    use crate::builtin::BuiltIn;
    use crate::oauth::TOKEN_URL;
    use pdt_api::sdk::documents::as_agent;
    use pdt_core::workflow::api::Connection;
    use pdt_forge::transport::Response;
    use pdt_forge::transport::testing::Recorded;
    use serde_json::json;

    const TOKEN: &str = r#"{"access_token":"at","expires_in":3600}"#;

    struct Env {
        docs: GoogleDocs,
        google: Arc<Recorded>,
        _tmp: tempfile::TempDir,
    }

    async fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let st = AppState::with_platform(store, tmp.path().join("data"), pdt_api::isolated_platform(tmp.path()));
        let google = Arc::new(Recorded::new());
        let built_in = BuiltIn::from_values([Some("std"), Some("s"), Some("agent"), Some("a"), None, None]);
        let shared = Arc::new(Shared::new(google.clone(), built_in));
        Env { docs: GoogleDocs { st, shared }, google, _tmp: tmp }
    }

    /// A signed-in Google account using PDT's own clients.
    async fn account(env: &Env, id: &str, name: &str, email: Option<&str>) {
        let c = Connection {
            id: id.into(),
            name: name.into(),
            kind: KIND.into(),
            url: String::new(),
            config: json!({}),
            has_secret: false,
            created_at: 0,
            updated_at: 0,
            last_error: None,
            last_error_at: None,
        };
        env.docs.st.store.insert_connection(&c).await.unwrap();
        let s = SignIn { refresh_token: format!("r-{id}"), email: email.map(String::from), desktop: false };
        store_sign_in(&env.docs.st, sign_in_key(id), &s).await.unwrap();
    }

    fn failed(e: ProviderError) -> String {
        match e {
            ProviderError::Failed(m) => m,
            other => panic!("not a failure: {other:?}"),
        }
    }

    #[tokio::test]
    async fn with_no_account_it_says_where_to_sign_in() {
        let env = env().await;
        let e = failed(env.docs.search("plan", None).await.unwrap_err());
        assert!(e.contains("no Google account is signed in"), "{e}");
        let new =
            || NewFile { title: "Plan".into(), kind: DocumentKind::Doc, markdown: None, folder: std::env::temp_dir() };
        let e = failed(env.docs.create(new()).await.unwrap_err());
        assert!(e.contains("no Google account is signed in"), "{e}");
        // An agent is told about the agents' permission instead, even with an account the user signed in.
        account(&env, "c1", "Work", None).await;
        let e = failed(as_agent(env.docs.search("plan", None)).await.unwrap_err());
        assert!(e.contains("no Google account lets agents use it"), "{e}");
        let e = failed(as_agent(env.docs.create(new())).await.unwrap_err());
        assert!(e.contains("lets agents use it"), "{e}");
        assert!(env.google.sent().is_empty(), "{:?}", env.google.urls());
    }

    #[tokio::test]
    async fn a_reference_reaches_only_its_own_account() {
        let env = env().await;
        assert!(matches!(env.docs.read("c1/../x").await, Err(ProviderError::Invalid(_))));
        let e = failed(env.docs.file("gone/f1").await.unwrap_err());
        assert!(e.contains("is gone from PDT"), "{e}");
        account(&env, "c1", "Work", None).await;
        let e = failed(as_agent(env.docs.version("c1/f1")).await.unwrap_err());
        assert!(e.contains("agents may not use the Google account Work"), "{e}");
        assert!(env.google.sent().is_empty(), "{:?}", env.google.urls());
    }

    #[tokio::test]
    async fn search_leaves_out_an_account_that_fails_unless_all_do() {
        let env = env().await;
        account(&env, "c1", "A", None).await;
        account(&env, "c2", "B", None).await;
        let doc = r#"{"files":[{"id":"d1","name":"Plan","mimeType":"application/vnd.google-apps.document","modifiedTime":"2026-11-02T10:00:00.000Z"}]}"#;
        env.google
            .ok("POST", TOKEN_URL, TOKEN)
            .ok("GET", &format!("{DRIVE}/files"), doc)
            .ok("POST", TOKEN_URL, TOKEN)
            .on("GET", &format!("{DRIVE}/files"), Response::new(503, "{}"));
        let found = env.docs.search("plan", None).await.unwrap();
        let refs: Vec<&str> = found.iter().map(|f| f.reference.as_str()).collect();
        assert_eq!(refs, ["c1/d1"]);
        assert_eq!(found[0].modified, Some(1_793_613_600_000), "Drive's modifiedTime");

        env.google.on("GET", &format!("{DRIVE}/files"), Response::new(503, "{}")).on(
            "GET",
            &format!("{DRIVE}/files"),
            Response::new(503, "{}"),
        );
        // Both fail now: the error comes back.
        assert!(failed(env.docs.search("plan", None).await.unwrap_err()).contains("503"));
    }

    #[tokio::test]
    async fn the_embedded_editor_opens_as_the_account_that_has_it() {
        let env = env().await;
        assert_eq!(env.docs.embed_url("bad", DocumentKind::Doc), None);
        assert_eq!(
            env.docs.embed_url("c1/d1", DocumentKind::Doc).unwrap(),
            "https://docs.google.com/document/d/d1/edit?rm=minimal"
        );
        account(&env, "c1", "Work", Some("me@x.com")).await;
        // The email is learned once the account is used.
        accounts::account_by_id(&env.docs.st, &env.docs.shared, "c1", false).await.unwrap();
        let url = env.docs.embed_url("c1/s1", DocumentKind::Sheet).unwrap();
        assert!(url.starts_with("https://docs.google.com/spreadsheets/d/s1/edit?rm=minimal&authuser=me"), "{url}");
    }

    #[test]
    fn google_offers_its_picker_and_everything_but_charts_and_live_edits() {
        let env = tokio::runtime::Runtime::new().unwrap().block_on(env());
        let p = env.docs.picker().unwrap();
        assert_eq!((p.module.as_str(), p.accounts.as_str()), ("/api/google/picker.js", "/api/google/accounts"));
        let c = env.docs.capabilities();
        assert!(c.docs && c.sheets && c.create && c.search && c.edit && c.comments && c.embed);
        assert!(!c.charts && !c.live);
        assert_eq!((env.docs.id(), env.docs.label()), ("google", "Google"));
    }

    #[test]
    fn a_reference_names_a_connection_and_a_drive_id() {
        assert_eq!(split_ref("c1/1aB-c_9").unwrap(), ("c1", "1aB-c_9"));
        for bad in ["c1", "/f", "c1/", "c1/../../forms/x", "c1/a/b", "c1/a?x=1", "c1/a#b", "c1/a%2F"] {
            assert!(matches!(split_ref(bad), Err(ProviderError::Invalid(_))), "{bad}");
        }
    }
}
