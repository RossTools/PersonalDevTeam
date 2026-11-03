//! A ticket's attachments (PDT-300): screenshots and text files the user, the PM or an agent put on a ticket, stored in
//! `<data dir>/tickets/<ticket id>/attachments/<attachment id>.<ext>` with their metadata in the store. Pure: what is
//! accepted (by the bytes, never the name), how a name is cleaned, and which local paths an agent may attach.

use crate::conversation::composer::{MAX_IMAGE_BYTES, image_extension, image_type};
use crate::ticket::{Actor, AgentBy};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ts_rs::TS;

/// The largest text file a ticket takes (1 MB). Images take the composer's limit ([`MAX_IMAGE_BYTES`]).
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// The most attachments one ticket holds.
pub const MAX_PER_TICKET: usize = 50;
/// The longest name kept (in characters).
const MAX_NAME: usize = 120;

/// One file on a ticket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketAttachment {
    pub id: String,
    pub ticket_id: String,
    /// The file's name as it was given (cleaned: no folders, no control characters).
    pub name: String,
    /// `image/png`, `image/jpeg`, `image/gif`, `image/webp`, or a text type (`text/plain`, `text/markdown`,
    /// `application/json`, `text/csv`, `application/yaml`, `application/toml`).
    pub media_type: String,
    #[ts(type = "number")]
    pub size: i64,
    /// Who added it: the user or an agent (the PM, an agent step).
    pub actor: Actor,
    /// The agent session that added it, when an agent did.
    #[serde(default)]
    pub agent: Option<AgentBy>,
    /// The comment it came with, if any.
    #[serde(default)]
    pub event_id: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
}

impl TicketAttachment {
    pub fn is_image(&self) -> bool {
        self.media_type.starts_with("image/")
    }

    /// The stored file's name in the ticket's attachments folder.
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.id, extension(&self.media_type))
    }

    /// Where the UI and agents fetch it.
    pub fn url(&self) -> String {
        format!("/api/tickets/{}/attachments/{}", self.ticket_id, self.id)
    }

    /// Markdown that shows it in a comment or description: the image inline, a text file as a link.
    pub fn markdown(&self) -> String {
        let name = self.name.replace(['[', ']'], "");
        match self.is_image() {
            true => format!("![{name}]({})", self.url()),
            false => format!("[{name}]({})", self.url()),
        }
    }
}

/// The ids of the ticket's attachments `text` links to (`/api/tickets/<ticket id>/attachments/<id>`, as
/// [`TicketAttachment::markdown`] writes them), in the order they first appear: a comment's attachments (PDT-341).
pub fn referenced_ids(text: &str, ticket_id: &str) -> Vec<String> {
    let prefix = format!("/api/tickets/{ticket_id}/attachments/");
    let mut ids: Vec<String> = Vec::new();
    for (at, _) in text.match_indices(&prefix) {
        let rest = &text[at + prefix.len()..];
        let id: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// Why a file can't be attached: a code for the API's error and a message for the person or agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub code: &'static str,
    pub message: String,
    /// Too big (the API answers 413).
    pub too_large: bool,
}

fn refused(code: &'static str, message: impl Into<String>) -> Refused {
    Refused { code, message: message.into(), too_large: false }
}

/// The media type of a text file by its name's extension (`text/plain` for any other).
fn text_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        _ => "text/plain",
    }
}

/// The stored file's extension for a media type.
pub fn extension(media_type: &str) -> &'static str {
    match media_type {
        t if t.starts_with("image/") => image_extension(t),
        "text/markdown" => "md",
        "application/json" => "json",
        "text/csv" => "csv",
        "application/yaml" => "yaml",
        "application/toml" => "toml",
        _ => "txt",
    }
}

/// Text PDT keeps: UTF-8 without NULs or other control characters than tabs, line ends and form feeds.
fn is_text(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .is_ok_and(|s| !s.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\x0c')))
}

/// What `bytes` are, by their content: an image (PNG, JPEG, GIF, WebP; at most 10 MB) or UTF-8 text (at most 1 MB,
/// its type by `name`). Anything else is refused.
pub fn sniff(name: &str, bytes: &[u8]) -> Result<&'static str, Refused> {
    if bytes.is_empty() {
        return Err(refused("empty_attachment", "the file is empty"));
    }
    if let Some(t) = image_type(bytes) {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(Refused {
                too_large: true,
                ..refused("attachment_too_large", "an image may be at most 10 MB")
            });
        }
        return Ok(t);
    }
    if bytes.len() > MAX_TEXT_BYTES {
        return Err(Refused {
            too_large: true,
            ..refused("attachment_too_large", "a text file may be at most 1 MB (images up to 10 MB)")
        });
    }
    if !is_text(bytes) {
        return Err(refused(
            "unsupported_attachment",
            "only images (PNG, JPEG, GIF, WebP) and UTF-8 text files (.txt, .log, .md, .json, .csv, .yaml, .toml) can \
             be attached",
        ));
    }
    Ok(text_type(name))
}

/// A name to keep: the last part of a path, without control characters, at most 120 characters; `attachment.<ext>`
/// when nothing is left.
pub fn clean_name(given: &str, media_type: &str) -> String {
    let last = given.rsplit(['/', '\\']).next().unwrap_or("");
    let name: String = last.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    let name = name.trim().trim_start_matches('.').trim();
    match name.is_empty() {
        true => format!("attachment.{}", extension(media_type)),
        false => name.to_string(),
    }
}

/// Folders an agent never attaches from, wherever they are: keys, credentials and keychains.
const SECRET_DIRS: &[&str] =
    &[".ssh", ".gnupg", ".aws", ".azure", ".kube", ".docker", ".password-store", "Keychains", ".config/gh"];

/// Whether paths here are compared without case: macOS's and Windows's file systems usually ignore it, so `.SSH` is
/// `.ssh` there.
const FOLD_CASE: bool = cfg!(any(target_os = "macos", windows));

/// A path's parts as compared: every component, lower-cased when `fold`.
fn parts(path: &Path, fold: bool) -> Vec<String> {
    path.components()
        .map(|c| {
            let s = c.as_os_str().to_string_lossy();
            if fold { s.to_lowercase() } else { s.into_owned() }
        })
        .collect()
}

/// `path`'s parts below `root`, or None when it isn't inside it.
fn below(path: &[String], root: &[String]) -> Option<Vec<String>> {
    path.starts_with(root).then(|| path[root.len()..].to_vec())
}

/// Why an agent may not attach the file at `path` (absolute, with links resolved), or None. Refused: dotfiles (`.env`,
/// `.netrc`), anything in a key or credential folder (`.ssh`, `.aws`, keychains), and PDT's own data folder except
/// the images pasted into conversations (`sessions/<id>/attachments/`) and other tickets' attachments. Compared
/// without case on macOS and Windows.
pub fn path_refusal(path: &Path, data_dir: &Path) -> Option<&'static str> {
    refusal(path, data_dir, FOLD_CASE)
}

fn refusal(path: &Path, data_dir: &Path, fold: bool) -> Option<&'static str> {
    if !path.is_absolute() {
        return Some("give an absolute path");
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.starts_with('.') {
        return Some("dotfiles (such as .env) are never attached");
    }
    let have = parts(path, fold);
    let in_secrets = SECRET_DIRS.iter().any(|d| {
        let want = parts(Path::new(d), fold);
        have.windows(want.len()).any(|w| w == want.as_slice())
    });
    if in_secrets {
        return Some("files in key and credential folders are never attached");
    }
    if let Some(rel) = below(&have, &parts(data_dir, fold)) {
        let allowed = rel.len() == 4 && (rel[0] == "sessions" || rel[0] == "tickets") && rel[2] == "attachments";
        if !allowed {
            return Some("PDT's own data is never attached (only images pasted into its conversations)");
        }
    }
    None
}

/// Why an agent may not attach the file at `real` (absolute, with links resolved) for where it is, or None: it must be
/// inside one of `roots` (the session's folder, the temporary folder and PDT's data folder, whose attachments
/// [`path_refusal`] lets through), so an agent can't attach files from elsewhere on the machine. Compared without
/// case on macOS and Windows.
pub fn outside_roots(real: &Path, roots: &[PathBuf]) -> Option<&'static str> {
    outside(real, roots, FOLD_CASE)
}

fn outside(real: &Path, roots: &[PathBuf], fold: bool) -> Option<&'static str> {
    let have = parts(real, fold);
    match roots.iter().any(|r| below(&have, &parts(r, fold)).is_some()) {
        true => None,
        false => Some("only files in this session's folder, the temporary folder or PDT's attachments can be attached"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    #[test]
    fn images_are_known_by_their_bytes_whatever_the_name() {
        assert_eq!(sniff("notes.txt", PNG), Ok("image/png"));
        assert_eq!(sniff("x", b"GIF89a.."), Ok("image/gif"));
    }

    #[test]
    fn text_takes_its_type_from_the_name() {
        assert_eq!(sniff("build.log", b"error: boom\n"), Ok("text/plain"));
        assert_eq!(sniff("README.MD", b"# Hi"), Ok("text/markdown"));
        assert_eq!(sniff("a.json", b"{}"), Ok("application/json"));
        assert_eq!(sniff("a.yml", b"a: 1"), Ok("application/yaml"));
        assert_eq!(sniff("Cargo.toml", b"[package]"), Ok("application/toml"));
        assert_eq!(sniff("t.csv", b"a,b\n1,2"), Ok("text/csv"));
        assert_eq!(sniff("noext", "naïve\ttext\r\n".as_bytes()), Ok("text/plain"));
    }

    #[test]
    fn binaries_and_empty_files_are_refused() {
        assert_eq!(sniff("a.txt", b"").unwrap_err().code, "empty_attachment");
        assert_eq!(sniff("a.bin", b"\x00\x01\x02").unwrap_err().code, "unsupported_attachment");
        assert_eq!(sniff("a.txt", b"\xff\xfe bad utf8").unwrap_err().code, "unsupported_attachment");
        assert_eq!(sniff("a.svg", b"<svg>\x07</svg>").unwrap_err().code, "unsupported_attachment");
    }

    #[test]
    fn sizes_are_capped() {
        let text = vec![b'a'; MAX_TEXT_BYTES + 1];
        let e = sniff("a.txt", &text).unwrap_err();
        assert!(e.too_large);
        assert!(sniff("a.txt", &text[..MAX_TEXT_BYTES]).is_ok());
        let mut img = PNG.to_vec();
        img.resize(MAX_IMAGE_BYTES + 1, 0);
        assert!(sniff("a.png", &img).unwrap_err().too_large);
    }

    #[test]
    fn names_lose_folders_and_control_characters() {
        assert_eq!(clean_name("/tmp/x/shot.png", "image/png"), "shot.png");
        assert_eq!(clean_name("C:\\Users\\me\\log.txt", "text/plain"), "log.txt");
        assert_eq!(clean_name("a\u{7}b.txt", "text/plain"), "ab.txt");
        assert_eq!(clean_name("", "image/png"), "attachment.png");
        assert_eq!(clean_name("..", "text/plain"), "attachment.txt");
        assert_eq!(clean_name(&"x".repeat(500), "text/plain").chars().count(), 120);
    }

    #[test]
    fn markdown_shows_images_inline_and_links_text() {
        let a = TicketAttachment {
            id: "a1".into(),
            ticket_id: "t1".into(),
            name: "shot [1].png".into(),
            media_type: "image/png".into(),
            size: 3,
            actor: Actor::User,
            agent: None,
            event_id: None,
            created_at: 0,
        };
        assert_eq!(a.markdown(), "![shot 1.png](/api/tickets/t1/attachments/a1)");
        assert_eq!(a.file_name(), "a1.png");
        let t = TicketAttachment { media_type: "text/plain".into(), name: "log.txt".into(), ..a };
        assert_eq!(t.markdown(), "[log.txt](/api/tickets/t1/attachments/a1)");
        assert_eq!(t.file_name(), "a1.txt");
    }

    #[test]
    fn a_comment_refers_to_the_tickets_attachments_it_links() {
        let body = "See ![shot](/api/tickets/t1/attachments/a-1) and [log](/api/tickets/t1/attachments/b2).\n\
                    Again ![shot](/api/tickets/t1/attachments/a-1), not [other](/api/tickets/t2/attachments/c3) \
                    nor /api/tickets/t1/attachments/.";
        assert_eq!(referenced_ids(body, "t1"), ["a-1", "b2"]);
        assert!(referenced_ids("no links", "t1").is_empty());
    }

    fn root() -> PathBuf {
        std::env::temp_dir().join("pdt-attach-test")
    }

    #[test]
    fn secrets_and_dotfiles_are_refused() {
        let data = root().join("data");
        let home = root().join("home");
        assert!(path_refusal(&home.join("shot.png"), &data).is_none());
        assert!(path_refusal(&home.join("project").join(".env"), &data).is_some());
        assert!(path_refusal(&home.join(".ssh").join("id_ed25519"), &data).is_some());
        assert!(path_refusal(&home.join(".aws").join("credentials"), &data).is_some());
        assert!(path_refusal(&home.join(".config").join("gh").join("hosts.yml"), &data).is_some());
        assert!(path_refusal(&home.join("Library").join("Keychains").join("login.db"), &data).is_some());
        assert!(path_refusal(Path::new("relative.png"), &data).is_some());
        // A dot folder that holds no secrets is fine (Linux keeps PDT's data in ~/.local/share).
        assert!(path_refusal(&home.join(".cache").join("shot.png"), &data).is_none());
    }

    #[test]
    fn pdt_data_is_refused_except_pasted_images() {
        let data = root().join("data");
        let pasted = data.join("sessions").join("s1").join("attachments").join("a.png");
        assert!(path_refusal(&pasted, &data).is_none());
        assert!(path_refusal(&data.join("tickets").join("t1").join("attachments").join("a.txt"), &data).is_none());
        assert!(path_refusal(&data.join("secrets.json"), &data).is_some());
        assert!(path_refusal(&data.join("pdt.db"), &data).is_some());
        assert!(path_refusal(&data.join("logs").join("pdtd.log"), &data).is_some());
        assert!(path_refusal(&data.join("sessions").join("s1").join("other").join("x"), &data).is_some());
    }

    #[test]
    fn case_never_gets_round_the_refusals_where_the_file_system_ignores_it() {
        let data = root().join("Data");
        let home = root().join("home");
        for bad in [home.join(".SSH").join("id_ed25519"), home.join(".Config").join("GH").join("hosts.yml")] {
            assert!(refusal(&bad, &data, true).is_some(), "{}", bad.display());
            assert!(refusal(&bad, &data, false).is_none(), "{}", bad.display());
        }
        let db = root().join("data").join("pdt.db");
        assert!(refusal(&db, &data, true).is_some());
        let pasted = root().join("DATA").join("Sessions").join("s1").join("Attachments").join("a.png");
        assert!(refusal(&pasted, &data, true).is_none());
    }

    #[test]
    fn only_files_in_the_sessions_folder_temp_or_pdts_data_are_attached() {
        let roots = [root().join("work").join("repo"), root().join("tmp"), root().join("data")];
        assert!(outside(&root().join("work").join("repo").join("shot.png"), &roots, false).is_none());
        assert!(outside(&root().join("tmp").join("qa").join("shot.png"), &roots, false).is_none());
        assert!(
            outside(&root().join("data").join("tickets").join("t").join("attachments").join("a"), &roots, false)
                .is_none()
        );
        for bad in [root().join("Documents").join("notes.txt"), root().join("work").join("repo2").join("x.txt")] {
            assert!(outside(&bad, &roots, false).is_some(), "{}", bad.display());
        }
        let shouty = root().join("WORK").join("Repo").join("shot.png");
        assert!(outside(&shouty, &roots, true).is_none());
        assert!(outside(&shouty, &roots, false).is_some());
    }
}
