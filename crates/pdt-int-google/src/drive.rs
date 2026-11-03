//! One account's Drive, Docs and Sheets calls (PDT-316): files (find, make, rename, bin), a Doc read as Markdown (Drive
//! exports it), a Sheet read as cells, edits run against the Docs and Sheets APIs, comments and replies, and the
//! file's version (Drive's counter, which goes up with every change). A file is PDT's `<connection id>/<file id>`.

use crate::api::{DOCS, DRIVE, SHEETS, Session, q};
use crate::{docs, sheets};
use pdt_api::sdk::documents::{
    CommentReply, DocumentComment, DocumentContent, DocumentEdit, DocumentKind, ProviderError, ProviderFile,
    ProviderResult,
};
use serde_json::{Value, json};

pub const DOC_MIME: &str = "application/vnd.google-apps.document";
pub const SHEET_MIME: &str = "application/vnd.google-apps.spreadsheet";
const FILE_FIELDS: &str = "id,name,mimeType,webViewLink,version,modifiedTime,trashed";

pub fn kind_of(mime: &str) -> Option<DocumentKind> {
    match mime {
        DOC_MIME => Some(DocumentKind::Doc),
        SHEET_MIME => Some(DocumentKind::Sheet),
        _ => None,
    }
}

pub fn mime_of(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Doc => DOC_MIME,
        DocumentKind::Sheet => SHEET_MIME,
    }
}

/// The page that edits it in Google, embedded; signed in as `email` when PDT knows it (a browser signed in to several
/// accounts opens the right one).
pub fn embed_url(file_id: &str, kind: DocumentKind, email: Option<&str>) -> String {
    let app = match kind {
        DocumentKind::Doc => "document",
        DocumentKind::Sheet => "spreadsheets",
    };
    let mut u = format!("https://docs.google.com/{app}/d/{file_id}/edit?rm=minimal");
    if let Some(e) = email {
        u.push_str(&format!("&authuser={}", q(e)));
    }
    u
}

fn file_of(conn: &str, v: &Value) -> Option<ProviderFile> {
    Some(ProviderFile {
        reference: format!("{conn}/{}", v["id"].as_str()?),
        title: v["name"].as_str().unwrap_or_default().to_string(),
        kind: kind_of(v["mimeType"].as_str()?)?,
        url: v["webViewLink"].as_str().map(str::to_string),
        version: v["version"].as_str().map(str::to_string).or_else(|| v["version"].as_i64().map(|n| n.to_string())),
        modified: v["modifiedTime"]
            .as_str()
            .and_then(|t| t.parse::<jiff::Timestamp>().ok())
            .map(|t| t.as_millisecond()),
    })
}

/// The file, if it is a Doc or a Sheet that isn't in the bin.
pub fn file(s: &Session, id: &str) -> ProviderResult<ProviderFile> {
    let v = s.get(&format!("{DRIVE}/files/{}?fields={FILE_FIELDS}&supportsAllDrives=true", q(id)))?;
    if v["trashed"].as_bool() == Some(true) {
        return Err(ProviderError::NotFound);
    }
    file_of(&s.account.connection_id, &v).ok_or_else(|| {
        ProviderError::Unsupported(format!(
            "{} is a {}, not a Google Doc or Sheet",
            v["name"].as_str().unwrap_or("that file"),
            v["mimeType"].as_str().unwrap_or("file")
        ))
    })
}

/// A Drive query's string, quoted.
fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// The Docs and Sheets PDT may open (those it made, and those picked in the picker), by words in their names, newest
/// first.
pub fn search(s: &Session, words: &str, kind: Option<DocumentKind>) -> ProviderResult<Vec<ProviderFile>> {
    let kinds: Vec<String> = match kind {
        Some(k) => vec![format!("mimeType = {}", quoted(mime_of(k)))],
        None => [DOC_MIME, SHEET_MIME].iter().map(|m| format!("mimeType = {}", quoted(m))).collect(),
    };
    let mut query = format!("trashed = false and ({})", kinds.join(" or "));
    for w in words.split_whitespace() {
        query.push_str(&format!(" and name contains {}", quoted(w)));
    }
    let v = s.get(&format!(
        "{DRIVE}/files?q={}&fields=files({FILE_FIELDS})&orderBy=modifiedTime%20desc&pageSize=50\
         &supportsAllDrives=true&includeItemsFromAllDrives=true",
        q(&query)
    ))?;
    Ok(v["files"].as_array().into_iter().flatten().filter_map(|f| file_of(&s.account.connection_id, f)).collect())
}

/// A folder's id from what the user gave: its id, or its page's address (`…/folders/<id>`).
pub fn folder_id(given: &str) -> Option<String> {
    let g = given.trim();
    let id = match g.split_once("/folders/") {
        Some((_, rest)) => rest.split(['?', '/', '#']).next().unwrap_or_default(),
        None => g,
    };
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')).then(|| id.into())
}

/// Makes an empty Doc or Sheet, in `folder` when given (else the top of My Drive).
pub fn create(s: &Session, title: &str, kind: DocumentKind, folder: Option<&str>) -> ProviderResult<ProviderFile> {
    let mut body = json!({"name": title, "mimeType": mime_of(kind)});
    if let Some(f) = folder {
        body["parents"] = json!([f]);
    }
    let v = s.post(&format!("{DRIVE}/files?fields={FILE_FIELDS}&supportsAllDrives=true"), body)?;
    file_of(&s.account.connection_id, &v).ok_or_else(|| ProviderError::Failed("Google made no such file".into()))
}

pub fn rename(s: &Session, id: &str, title: &str) -> ProviderResult<()> {
    s.patch(&format!("{DRIVE}/files/{}?fields=id&supportsAllDrives=true", q(id)), json!({"name": title}))?;
    Ok(())
}

/// Moves it to the bin (Drive keeps it 30 days).
pub fn trash(s: &Session, id: &str) -> ProviderResult<()> {
    s.patch(&format!("{DRIVE}/files/{}?fields=id&supportsAllDrives=true", q(id)), json!({"trashed": true}))?;
    Ok(())
}

pub fn version(s: &Session, id: &str) -> ProviderResult<Option<String>> {
    Ok(file(s, id)?.version)
}

/// Its content: a Doc as Markdown, a Sheet as its sheets of cells.
pub fn read(s: &Session, id: &str) -> ProviderResult<DocumentContent> {
    let f = file(s, id)?;
    Ok(match f.kind {
        DocumentKind::Doc => {
            let markdown = s.call_raw(
                "GET",
                &format!("{DRIVE}/files/{}/export?mimeType=text%2Fmarkdown", q(id)),
                crate::api::Body::None,
            )?;
            DocumentContent::Doc { markdown, version: f.version }
        }
        DocumentKind::Sheet => {
            let v = s.get(&format!(
                "{SHEETS}/spreadsheets/{}?includeGridData=true&fields={}",
                q(id),
                q(&sheets::read_fields())
            ))?;
            DocumentContent::Sheet { sheets: sheets::read(&v), version: f.version }
        }
    })
}

/// A new sheet's id: random, so edits after it in the same batch can name it.
fn new_sheet_id() -> i64 {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("the system's random numbers");
    i64::from(u32::from_le_bytes(b) >> 2) + 1
}

/// Runs `edits` in order; the new version.
pub fn apply(s: &Session, id: &str, edits: &[DocumentEdit]) -> ProviderResult<Option<String>> {
    let f = file(s, id)?;
    match f.kind {
        DocumentKind::Doc => {
            let get = || s.get(&format!("{DOCS}/documents/{}", q(id)));
            let update = |requests: Vec<Value>| -> ProviderResult<()> {
                if !requests.is_empty() {
                    s.post(&format!("{DOCS}/documents/{}:batchUpdate", q(id)), json!({"requests": requests}))?;
                }
                Ok(())
            };
            for e in edits {
                if !e.for_docs() {
                    return Err(ProviderError::Unsupported(format!(
                        "a Google Doc has no sheets or cells ({} is a Sheets edit)",
                        e.op()
                    )));
                }
                // Each edit reads the document again: the one before moved its indexes.
                let doc = get()?;
                update(docs::requests(&doc, e)?)?;
                // A new table row is filled once it is there.
                if let DocumentEdit::TableAppendRow { heading, table, cells } = e {
                    let t = docs::find_table(&doc, heading.as_deref(), *table)?;
                    update(docs::fill_last_row(&get()?, t, cells)?)?;
                }
            }
        }
        DocumentKind::Sheet => {
            let meta = s.get(&format!("{SHEETS}/spreadsheets/{}?fields={}", q(id), q(sheets::META_FIELDS)))?;
            let steps = sheets::plan(&mut sheets::Ids::of(&meta), edits, new_sheet_id)?;
            for step in steps {
                match step {
                    sheets::Step::Requests(r) => {
                        s.post(&format!("{SHEETS}/spreadsheets/{}:batchUpdate", q(id)), json!({"requests": r}))?
                    }
                    sheets::Step::Values(data) => s.post(
                        &format!("{SHEETS}/spreadsheets/{}/values:batchUpdate", q(id)),
                        json!({"valueInputOption": "USER_ENTERED", "data": data}),
                    )?,
                };
            }
        }
    }
    version(s, id)
}

fn time(v: &Value) -> Option<i64> {
    v.as_str()?.parse::<jiff::Timestamp>().ok().map(|t| t.as_millisecond())
}

/// The user, for what the signed-in account wrote (Google's `me`); None for anyone else's.
fn mine(author: &Value) -> Option<pdt_core::ticket::Actor> {
    (author["me"].as_bool() == Some(true)).then_some(pdt_core::ticket::Actor::User)
}

fn reply_of(r: &Value) -> Option<CommentReply> {
    if r["deleted"].as_bool() == Some(true) {
        return None;
    }
    // A resolving reply has no text; it shows as the comment's state.
    let body = r["content"].as_str().unwrap_or_default();
    if body.is_empty() {
        return None;
    }
    Some(CommentReply {
        id: r["id"].as_str()?.to_string(),
        author: r["author"]["displayName"].as_str().unwrap_or("someone").to_string(),
        body: body.to_string(),
        created_at: time(&r["createdTime"]),
        actor: mine(&r["author"]),
    })
}

const COMMENT_FIELDS: &str = "comments(id,author(displayName,me),content,quotedFileContent(value),resolved,createdTime,\
                              deleted,replies(id,author(displayName,me),content,createdTime,deleted,action))";

pub fn comments(s: &Session, id: &str) -> ProviderResult<Vec<DocumentComment>> {
    let v = s.get(&format!("{DRIVE}/files/{}/comments?fields={}&pageSize=100", q(id), q(COMMENT_FIELDS)))?;
    Ok(v["comments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["deleted"].as_bool() != Some(true))
        .filter_map(|c| {
            Some(DocumentComment {
                id: c["id"].as_str()?.to_string(),
                author: c["author"]["displayName"].as_str().unwrap_or("someone").to_string(),
                body: c["content"].as_str().unwrap_or_default().to_string(),
                quote: c["quotedFileContent"]["value"].as_str().map(str::to_string),
                anchor: None,
                resolved: c["resolved"].as_bool().unwrap_or(false),
                created_at: time(&c["createdTime"]),
                replies: c["replies"].as_array().into_iter().flatten().filter_map(reply_of).collect(),
                actor: mine(&c["author"]),
            })
        })
        .collect())
}

pub fn reply(s: &Session, id: &str, comment: &str, body: &str) -> ProviderResult<CommentReply> {
    let v = s.post(
        &format!(
            "{DRIVE}/files/{}/comments/{}/replies?fields=id,author(displayName),content,createdTime",
            q(id),
            q(comment)
        ),
        json!({"content": body}),
    )?;
    reply_of(&v).ok_or_else(|| ProviderError::Failed("Google kept no reply".into()))
}

pub fn resolve(s: &Session, id: &str, comment: &str) -> ProviderResult<()> {
    s.post(
        &format!("{DRIVE}/files/{}/comments/{}/replies?fields=id", q(id), q(comment)),
        json!({"action": "resolve"}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Tokens, tests::account};
    use crate::oauth::TOKEN_URL;
    use pdt_forge::transport::testing::Recorded;

    const TOKEN: &str = r#"{"access_token":"tok","expires_in":3600}"#;
    const DOC: &str = r#"{"id":"d1","name":"Plan","mimeType":"application/vnd.google-apps.document","webViewLink":"https://docs.google.com/document/d/d1/edit","version":"12"}"#;
    const SHEET: &str =
        r#"{"id":"s1","name":"Costs","mimeType":"application/vnd.google-apps.spreadsheet","version":"4"}"#;

    fn with<T>(r: &Recorded, f: impl FnOnce(&Session) -> T) -> T {
        let (a, t) = (account(), Tokens::default());
        f(&Session::new(r, &a, &t))
    }

    #[test]
    fn a_doc_reads_as_markdown() {
        let r = Recorded::new();
        r.ok("POST", TOKEN_URL, TOKEN).ok("GET", &format!("{DRIVE}/files/d1?"), DOC).ok(
            "GET",
            &format!("{DRIVE}/files/d1/export?mimeType=text%2Fmarkdown"),
            "# Plan\n\nShip it.\n",
        );
        let c = with(&r, |s| read(s, "d1")).unwrap();
        assert_eq!(c, DocumentContent::Doc { markdown: "# Plan\n\nShip it.\n".into(), version: Some("12".into()) });
    }

    #[test]
    fn files_that_are_not_docs_or_sheets_are_refused() {
        let r = Recorded::new();
        r.ok("POST", TOKEN_URL, TOKEN).ok(
            "GET",
            &format!("{DRIVE}/files/p1?"),
            r#"{"id":"p1","name":"photo.png","mimeType":"image/png"}"#,
        );
        assert!(matches!(with(&r, |s| file(s, "p1")), Err(ProviderError::Unsupported(m)) if m.contains("photo.png")));
        let r = Recorded::new();
        r.ok("POST", TOKEN_URL, TOKEN).ok(
            "GET",
            &format!("{DRIVE}/files/d1?"),
            r#"{"id":"d1","name":"x","mimeType":"application/vnd.google-apps.document","trashed":true}"#,
        );
        assert_eq!(with(&r, |s| file(s, "d1")), Err(ProviderError::NotFound));
    }

    #[test]
    fn search_asks_drive_for_docs_and_sheets_by_name() {
        let r = Recorded::new();
        r.ok("POST", TOKEN_URL, TOKEN).ok(
            "GET",
            &format!("{DRIVE}/files?q="),
            &format!(r#"{{"files":[{DOC},{SHEET}]}}"#),
        );
        let files = with(&r, |s| search(s, "it's plan", None)).unwrap();
        assert_eq!(files.iter().map(|f| f.reference.as_str()).collect::<Vec<_>>(), ["c1/d1", "c1/s1"]);
        assert_eq!(files[1].kind, DocumentKind::Sheet);
        let url = &r.sent()[1].url;
        let query = url.split("q=").nth(1).unwrap().split('&').next().unwrap();
        assert!(query.contains(&q("name contains 'it\\'s'")), "{query}");
        assert!(query.contains(&q("trashed = false")), "{query}");
    }

    #[test]
    fn a_new_file_goes_in_the_folder() {
        let r = Recorded::new();
        r.ok("POST", TOKEN_URL, TOKEN).ok("POST", &format!("{DRIVE}/files?"), DOC);
        let f = with(&r, |s| create(s, "Plan", DocumentKind::Doc, Some("fold1"))).unwrap();
        assert_eq!(f.reference, "c1/d1");
        let body: Value = serde_json::from_str(r.sent()[1].body.as_deref().unwrap()).unwrap();
        assert_eq!(body, json!({"name": "Plan", "mimeType": DOC_MIME, "parents": ["fold1"]}));
        assert_eq!(
            folder_id("https://drive.google.com/drive/folders/1AbC_d-9?usp=sharing").as_deref(),
            Some("1AbC_d-9")
        );
        assert_eq!(folder_id(" 1AbC "), Some("1AbC".into()));
        assert_eq!(folder_id("not a folder"), None);
    }

    #[test]
    fn doc_edits_read_the_document_and_send_requests() {
        let r = Recorded::new();
        let body = r#"{"body":{"content":[{"endIndex":1,"sectionBreak":{}},{"startIndex":1,"endIndex":6,"paragraph":{"elements":[{"textRun":{"content":"Goal\n"}}],"paragraphStyle":{"namedStyleType":"HEADING_1"}}}]}}"#;
        r.ok("POST", TOKEN_URL, TOKEN)
            .ok("GET", &format!("{DRIVE}/files/d1?"), DOC)
            .ok("GET", &format!("{DOCS}/documents/d1"), body)
            .ok("POST", &format!("{DOCS}/documents/d1:batchUpdate"), "{}")
            .ok("GET", &format!("{DOCS}/documents/d1"), body)
            .ok("POST", &format!("{DOCS}/documents/d1:batchUpdate"), "{}")
            .ok("GET", &format!("{DRIVE}/files/d1?"), &DOC.replace("\"12\"", "\"13\""));
        let edits = [
            DocumentEdit::InsertAfterHeading { heading: "Goal".into(), markdown: "- a\n- b".into() },
            DocumentEdit::ReplaceString { find: "Goal".into(), replace: "Aim".into(), all: false },
        ];
        let v = with(&r, |s| apply(s, "d1", &edits)).unwrap();
        assert_eq!(v.as_deref(), Some("13"));
        let urls = r.urls();
        assert_eq!(urls.iter().filter(|u| u.contains("/documents/d1:batchUpdate")).count(), 2);
        // Each edit reads the document first.
        assert_eq!(urls.iter().filter(|u| u.starts_with("GET https://docs.googleapis.com")).count(), 2);
        let sent: Value = serde_json::from_str(r.sent()[3].body.as_deref().unwrap()).unwrap();
        assert_eq!(sent["requests"][0]["insertText"]["location"]["index"], 5);
        let sheet_edit = [DocumentEdit::AddSheet { name: "x".into() }];
        let r2 = Recorded::new();
        r2.ok("POST", TOKEN_URL, TOKEN).ok("GET", &format!("{DRIVE}/files/d1?"), DOC);
        assert!(matches!(with(&r2, |s| apply(s, "d1", &sheet_edit)), Err(ProviderError::Unsupported(_))));
    }

    #[test]
    fn sheet_edits_run_requests_and_values_in_order() {
        let r = Recorded::new();
        r.ok("POST", TOKEN_URL, TOKEN)
            .ok("GET", &format!("{DRIVE}/files/s1?"), SHEET)
            .ok(
                "GET",
                &format!("{SHEETS}/spreadsheets/s1?fields="),
                r#"{"sheets":[{"properties":{"sheetId":0,"title":"Sheet1"}}]}"#,
            )
            .ok("POST", &format!("{SHEETS}/spreadsheets/s1/values:batchUpdate"), "{}")
            .ok("POST", &format!("{SHEETS}/spreadsheets/s1:batchUpdate"), "{}")
            .ok("GET", &format!("{DRIVE}/files/s1?"), SHEET);
        let edits = [
            DocumentEdit::SetCells {
                sheet: "Sheet1".into(),
                cell: "A1".into(),
                values: vec![
                    vec!["Item".into(), "Cost".into()],
                    vec!["Pens".into(), "3".into()],
                    vec!["Total".into(), "=SUM(B2:B2)".into()],
                ],
            },
            DocumentEdit::FormatRange {
                sheet: "Sheet1".into(),
                range: "A1:B1".into(),
                format: pdt_api::sdk::documents::CellFormat { bold: Some(true), ..Default::default() },
            },
        ];
        assert_eq!(with(&r, |s| apply(s, "s1", &edits)).unwrap().as_deref(), Some("4"));
        let urls = r.urls();
        let values = urls.iter().position(|u| u.contains("values:batchUpdate")).unwrap();
        let batch = urls.iter().position(|u| u.ends_with("s1:batchUpdate")).unwrap();
        assert!(values < batch, "{urls:?}");
        let sent: Value = serde_json::from_str(r.sent()[values].body.as_deref().unwrap()).unwrap();
        assert_eq!(sent["valueInputOption"], "USER_ENTERED");
        assert_eq!(sent["data"][0]["range"], "'Sheet1'!A1:B3");
    }

    #[test]
    fn comments_list_reply_and_resolve() {
        let r = Recorded::new();
        r.ok("POST", TOKEN_URL, TOKEN)
            .ok(
                "GET",
                &format!("{DRIVE}/files/d1/comments"),
                r#"{"comments":[{"id":"k1","author":{"displayName":"Sam"},"content":"Is this right?","quotedFileContent":{"value":"Ship it"},"resolved":false,"createdTime":"2026-10-22T09:00:00.000Z","replies":[{"id":"r1","author":{"displayName":"Ann","me":true},"content":"Yes","createdTime":"2026-10-22T10:00:00Z"},{"id":"r2","action":"resolve","content":""}]},{"id":"k2","deleted":true}]}"#,
            )
            .ok("POST", TOKEN_URL, TOKEN)
            .ok("POST", TOKEN_URL, TOKEN)
            .ok("POST", &format!("{DRIVE}/files/d1/comments/k1/replies"), r#"{"id":"r3","author":{"displayName":"PDT"},"content":"Done","createdTime":"2026-10-22T11:00:00Z"}"#)
            .ok("POST", &format!("{DRIVE}/files/d1/comments/k1/replies"), r#"{"id":"r4"}"#);
        let c = with(&r, |s| comments(s, "d1")).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].author.as_str(), c[0].quote.as_deref(), c[0].replies.len()), ("Sam", Some("Ship it"), 1));
        assert_eq!(c[0].created_at, Some(1_792_659_600_000));
        // What the signed-in account wrote is the user's.
        assert_eq!((c[0].actor, c[0].replies[0].actor), (None, Some(pdt_core::ticket::Actor::User)));
        let rep = with(&r, |s| reply(s, "d1", "k1", "Done")).unwrap();
        assert_eq!(rep.id, "r3");
        with(&r, |s| resolve(s, "d1", "k1")).unwrap();
        let last: Value = serde_json::from_str(r.sent().last().unwrap().body.as_deref().unwrap()).unwrap();
        assert_eq!(last, json!({"action": "resolve"}));
    }

    #[test]
    fn the_embed_opens_the_right_account() {
        assert_eq!(
            embed_url("s1", DocumentKind::Sheet, Some("me@x.com")),
            "https://docs.google.com/spreadsheets/d/s1/edit?rm=minimal&authuser=me%40x.com"
        );
        assert_eq!(embed_url("d1", DocumentKind::Doc, None), "https://docs.google.com/document/d/d1/edit?rm=minimal");
    }
}
