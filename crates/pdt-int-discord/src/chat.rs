//! Talking to the PM in the bot's DMs (PDT-304), the pure part: replies cut into Discord messages (code blocks kept
//! whole or closed and reopened), the slash commands, the buttons of an approval or a question and their ids, and
//! what a message says. `dm.rs` does the talking.

use serde_json::{Value, json};

/// The most characters a Discord message holds.
pub const MAX_MESSAGE: usize = 2000;
/// A reply longer than this many messages is sent as a `.md` file instead.
pub const MAX_PIECES: usize = 4;
/// Where an approval's arguments stop fitting in its message and go in a file.
const MAX_ARGS: usize = 1400;
/// Discord's limit on a button's label.
const MAX_LABEL: usize = 80;
/// Discord's limit on a choice's name (autocomplete).
const MAX_CHOICE: usize = 100;
/// The prefix of PDT's buttons' ids.
const ID_PREFIX: &str = "pdt";

/// Discord's message flag for a reply only its user sees.
pub const EPHEMERAL: u64 = 1 << 6;

/// A reply as Discord gets it: messages, or (too long for [`MAX_PIECES`]) a short message with the whole as a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Messages(Vec<String>),
    File { note: String, name: String, body: String },
}

/// How a reply goes to Discord: `title`, when given, heads it (the conversation answering changed).
pub fn deliver(title: Option<&str>, text: &str) -> Delivery {
    let text = text.trim();
    let head = title.map(|t| format!("**{}**\n", t.trim())).unwrap_or_default();
    let pieces = split_markdown(&format!("{head}{text}"), MAX_MESSAGE);
    if pieces.len() <= MAX_PIECES {
        return Delivery::Messages(pieces);
    }
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
    let first: String = first.chars().take(300).collect();
    Delivery::File {
        note: format!("{head}The reply is long, so it is attached.\n> {first}"),
        name: "reply.md".into(),
        body: format!("{text}\n"),
    }
}

/// One block of a Markdown text: a paragraph, or a fenced code block (with its fences).
#[derive(Debug)]
struct Block {
    lines: Vec<String>,
    /// A code block's opening fence (```` ```rust ````).
    fence: Option<String>,
}

fn fence_of(line: &str) -> Option<&str> {
    let t = line.trim_start();
    (t.starts_with("```") || t.starts_with("~~~")).then_some(t)
}

/// The text's paragraphs and code blocks, in order.
fn blocks(text: &str) -> Vec<Block> {
    let mut out: Vec<Block> = vec![];
    let mut cur: Option<Block> = None;
    for line in text.lines() {
        let in_code = cur.as_ref().is_some_and(|b| b.fence.is_some());
        if in_code {
            let b = cur.as_mut().expect("in a block");
            b.lines.push(line.to_string());
            let open = b.fence.as_deref().unwrap_or("```");
            let closes = fence_of(line).is_some_and(|f| f.trim_end() == &open[..3]) && b.lines.len() > 1;
            if closes {
                out.extend(cur.take());
            }
            continue;
        }
        if let Some(f) = fence_of(line) {
            out.extend(cur.take());
            cur = Some(Block { lines: vec![line.to_string()], fence: Some(f.trim_end().to_string()) });
            continue;
        }
        if line.trim().is_empty() {
            out.extend(cur.take());
            continue;
        }
        cur.get_or_insert_with(|| Block { lines: vec![], fence: None }).lines.push(line.to_string());
    }
    out.extend(cur);
    out
}

/// `text` in messages of at most `max` characters, cut between paragraphs and code blocks where it can. A block too
/// long for one message is cut between lines; a code block's pieces each close and reopen its fence, so none is left
/// open. A line too long for a message is cut at a space, else anywhere.
pub fn split_markdown(text: &str, max: usize) -> Vec<String> {
    let len = |s: &str| s.chars().count();
    let mut out: Vec<String> = vec![];
    let mut cur = String::new();
    let push = |cur: &mut String, piece: &str, sep: &str, out: &mut Vec<String>| {
        if cur.is_empty() {
            cur.push_str(piece);
        } else if len(cur) + len(sep) + len(piece) <= max {
            cur.push_str(sep);
            cur.push_str(piece);
        } else {
            out.push(std::mem::take(cur));
            cur.push_str(piece);
        }
    };
    for b in blocks(text) {
        let whole = b.lines.join("\n");
        if len(&whole) <= max {
            push(&mut cur, &whole, "\n\n", &mut out);
            continue;
        }
        // Too long for a message: line by line, in pieces of their own.
        if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        let (open, close, body) = match &b.fence {
            Some(f) => {
                let closed = b.lines.len() > 1 && b.lines.last().and_then(|l| fence_of(l)).is_some();
                let end = if closed { b.lines.len() - 1 } else { b.lines.len() };
                (format!("{f}\n"), "\n```".to_string(), &b.lines[1..end])
            }
            None => (String::new(), String::new(), &b.lines[..]),
        };
        let room = max.saturating_sub(len(&open) + len(&close)).max(1);
        let mut piece = String::new();
        let flush = |piece: &mut String, out: &mut Vec<String>| {
            if !piece.is_empty() {
                out.push(format!("{open}{}{close}", std::mem::take(piece)));
            }
        };
        for line in body {
            for part in crate::post::split_content(line, room) {
                if piece.is_empty() {
                    piece = part;
                } else if len(&piece) + 1 + len(&part) <= room {
                    piece.push('\n');
                    piece.push_str(&part);
                } else {
                    flush(&mut piece, &mut out);
                    piece = part;
                }
            }
        }
        flush(&mut piece, &mut out);
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

/// The slash commands, registered for the bot's DMs.
pub fn commands() -> Value {
    let dm = |name: &str, description: &str, options: Value| {
        json!({"name": name, "type": 1, "description": description, "options": options,
               "contexts": [1], "integration_types": [0]})
    };
    json!([
        dm(
            "new",
            "Start a new PM conversation",
            json!([{"type": 3, "name": "topic", "required": false,
            "description": "What it is about (its title)"}])
        ),
        dm("list", "The open PM conversations", json!([])),
        dm(
            "switch",
            "Talk to another PM conversation",
            json!([{"type": 3, "name": "conversation", "required": true,
            "autocomplete": true, "description": "The conversation"}])
        ),
        dm("close", "Close this PM conversation (the PM writes its journal entry)", json!([])),
    ])
}

/// What a button of PDT's does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Press {
    Approve,
    Decline,
    /// A question's option (question, option).
    Pick(usize, usize),
    /// Send a many-answers question's picks.
    Done(usize),
}

/// A button's id: the prompt it is on (a short key of PDT's) and what it does.
pub fn button_id(key: &str, press: &Press) -> String {
    let what = match press {
        Press::Approve => "approve".to_string(),
        Press::Decline => "decline".to_string(),
        Press::Pick(q, o) => format!("q{q}o{o}"),
        Press::Done(q) => format!("q{q}done"),
    };
    format!("{ID_PREFIX}:{key}:{what}")
}

/// The prompt and press a button's id names; None for an id that is not PDT's.
pub fn parse_button(id: &str) -> Option<(String, Press)> {
    let mut parts = id.splitn(3, ':');
    if parts.next()? != ID_PREFIX {
        return None;
    }
    let key = parts.next()?.to_string();
    let what = parts.next()?;
    let press = match what {
        "approve" => Press::Approve,
        "decline" => Press::Decline,
        _ => {
            let rest = what.strip_prefix('q')?;
            if let Some(q) = rest.strip_suffix("done") {
                Press::Done(q.parse().ok()?)
            } else {
                let (q, o) = rest.split_once('o')?;
                Press::Pick(q.parse().ok()?, o.parse().ok()?)
            }
        }
    };
    Some((key, press))
}

fn label(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= MAX_LABEL {
        return t.to_string();
    }
    let mut s: String = t.chars().take(MAX_LABEL - 1).collect();
    s.push('…');
    s
}

/// The Approve and Decline buttons of a prompt (disabled once it is answered).
pub fn approval_buttons(key: &str, disabled: bool) -> Value {
    json!([{"type": 1, "components": [
        {"type": 2, "style": 3, "label": "Approve", "custom_id": button_id(key, &Press::Approve), "disabled": disabled},
        {"type": 2, "style": 4, "label": "Decline", "custom_id": button_id(key, &Press::Decline), "disabled": disabled},
    ]}])
}

/// What an approval's message says: the PM's call in a line, and its arguments; arguments too long for the message are
/// left for a file (the second value).
pub fn approval_text(title: &str, tool: &str, input: &Value) -> (String, Option<String>) {
    let short = tool.rsplit("__").next().unwrap_or(tool);
    let args = serde_json::to_string_pretty(input).unwrap_or_default();
    let head = format!("**The PM asks to:** {}\n`{short}`", title.trim());
    let empty = input.as_object().is_some_and(|o| o.is_empty()) || input.is_null();
    if empty {
        return (head, None);
    }
    if args.chars().count() <= MAX_ARGS {
        return (format!("{head}\n```json\n{args}\n```"), None);
    }
    (format!("{head}\nIts arguments are attached."), Some(args))
}

/// What an answered prompt's message says at its end.
pub fn answered_line(answer: &str, by: Option<&str>) -> String {
    match by {
        Some(by) => format!("\n**{answer}** (via {by})"),
        None => format!("\n**{answer}**"),
    }
}

/// One of an `AskUserQuestion` request's questions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub question: String,
    pub header: String,
    pub options: Vec<String>,
    pub multi: bool,
}

/// A question request's questions (Claude's `AskUserQuestion` input).
pub fn questions(input: &Value) -> Vec<Question> {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    input
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|q| Question {
            question: s(q, "question"),
            header: s(q, "header"),
            options: q.get("options").and_then(Value::as_array).into_iter().flatten().map(|o| s(o, "label")).collect(),
            multi: q.get("multiSelect").and_then(Value::as_bool) == Some(true),
        })
        .filter(|q| !q.question.is_empty())
        .collect()
}

/// A question's message: its text and, for each option, a button (`picked` shows which a many-answers question has).
pub fn question_message(key: &str, qi: usize, q: &Question, picked: &[String], disabled: bool) -> (String, Value) {
    let head = if q.header.is_empty() { String::new() } else { format!("**{}**\n", q.header.trim()) };
    let hint = if q.multi { "Pick any, then Send; or type an answer." } else { "Pick one, or type an answer." };
    let text = format!("{head}{}\n-# {hint}", q.question.trim());
    // Discord allows 5 rows of 5 buttons; a many-answers question keeps a row for Send.
    let room = if q.multi { 20 } else { 25 };
    let buttons: Vec<Value> = q
        .options
        .iter()
        .enumerate()
        .take(room)
        .map(|(oi, o)| {
            let style = if picked.contains(o) { 3 } else { 2 };
            json!({"type": 2, "style": style, "label": label(o), "custom_id": button_id(key, &Press::Pick(qi, oi)),
                   "disabled": disabled})
        })
        .collect();
    let mut rows: Vec<Value> = buttons.chunks(5).map(|c| json!({"type": 1, "components": c})).collect();
    if q.multi {
        rows.push(json!({"type": 1, "components": [{"type": 2, "style": 1, "label": "Send",
            "custom_id": button_id(key, &Press::Done(qi)), "disabled": disabled}]}));
    }
    (text, Value::Array(rows))
}

/// `/switch`'s suggestions: open conversations whose title has what was typed, as Discord's choices (title → id).
pub fn choices<'a>(typed: &str, open: impl Iterator<Item = (&'a str, &'a str)>) -> Value {
    let t = typed.trim().to_lowercase();
    let all: Vec<Value> = open
        .filter(|(_, title)| t.is_empty() || title.to_lowercase().contains(&t))
        .take(25)
        .map(|(id, title)| json!({"name": label_n(title, MAX_CHOICE), "value": id}))
        .collect();
    Value::Array(all)
}

fn label_n(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max { t.to_string() } else { t.chars().take(max - 1).chain(['…']).collect() }
}

/// The conversation `given` names among the open ones (id, title): its id, its exact title, or the only title that
/// has it.
pub fn find_conversation<'a>(given: &str, open: &[(&'a str, &'a str)]) -> Result<&'a str, String> {
    let g = given.trim();
    if let Some((id, _)) = open.iter().find(|(id, _)| *id == g) {
        return Ok(id);
    }
    let lower = g.to_lowercase();
    if let Some((id, _)) = open.iter().find(|(_, t)| t.to_lowercase() == lower) {
        return Ok(id);
    }
    let some: Vec<&(&str, &str)> = open.iter().filter(|(_, t)| t.to_lowercase().contains(&lower)).collect();
    match some.as_slice() {
        [one] => Ok(one.0),
        [] => Err(format!("No open conversation is called {g}. /list shows them.")),
        _ => Err(format!("{} conversations match {g}: pick one from the list as you type.", some.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fences_balanced(piece: &str) -> bool {
        piece.lines().filter(|l| fence_of(l).is_some()).count() % 2 == 0
    }

    #[test]
    fn a_short_reply_is_one_message_headed_by_a_new_title() {
        assert_eq!(
            deliver(None, "Two tickets are in flight."),
            Delivery::Messages(vec!["Two tickets are in flight.".into()])
        );
        assert_eq!(deliver(Some("Release plan"), "Done."), Delivery::Messages(vec!["**Release plan**\nDone.".into()]));
    }

    #[test]
    fn a_long_reply_is_cut_between_paragraphs_with_no_broken_code_block() {
        let para = "word ".repeat(150);
        let code = format!("```rust\n{}```", "let x = 1;\n".repeat(60));
        let text = format!("{para}\n\n{para}\n\n{code}\n\n{para}\n\n{para}\n\n{para}");
        assert!(text.chars().count() >= 4000, "{}", text.len());
        let pieces = split_markdown(&text, MAX_MESSAGE);
        assert!(pieces.len() >= 2 && pieces.len() <= MAX_PIECES, "{}", pieces.len());
        for p in &pieces {
            assert!(p.chars().count() <= MAX_MESSAGE, "{}", p.chars().count());
            assert!(fences_balanced(p), "an open code block: {p}");
            assert!(!p.starts_with('\n') && !p.ends_with('\n'));
        }
        // Nothing is lost.
        let words = |s: &str| s.split_whitespace().filter(|w| *w != "```" && *w != "```rust").count();
        assert_eq!(pieces.iter().map(|p| words(p)).sum::<usize>(), words(&text));
    }

    #[test]
    fn a_code_block_too_long_for_a_message_is_closed_and_reopened() {
        let code = format!("Here:\n\n```python\n{}```\n\nThat's it.", "print('hello world')\n".repeat(200));
        let pieces = split_markdown(&code, MAX_MESSAGE);
        assert!(pieces.len() >= 3, "{}", pieces.len());
        let code_pieces: Vec<&String> = pieces.iter().filter(|p| p.contains("print(")).collect();
        for p in &code_pieces {
            assert!(p.starts_with("```python\n") && p.ends_with("\n```"), "{p}");
            assert!(p.chars().count() <= MAX_MESSAGE);
        }
        assert_eq!(pieces.first().unwrap(), "Here:");
        assert_eq!(pieces.last().unwrap(), "That's it.");
        let lines: usize = code_pieces.iter().map(|p| p.matches("print(").count()).sum();
        assert_eq!(lines, 200);
    }

    #[test]
    fn one_huge_line_is_still_cut() {
        let pieces = split_markdown(&"x".repeat(4500), MAX_MESSAGE);
        assert_eq!(pieces.iter().map(|p| p.len()).collect::<Vec<_>>(), [2000, 2000, 500]);
    }

    #[test]
    fn a_very_long_reply_goes_as_a_file() {
        let text = format!("Summary line\n\n{}", "lorem ipsum ".repeat(1000));
        match deliver(Some("Big"), &text) {
            Delivery::File { note, name, body } => {
                assert!(note.starts_with("**Big**\n") && note.contains("> Summary line"), "{note}");
                assert_eq!(name, "reply.md");
                assert!(body.starts_with("Summary line"));
            }
            d => panic!("{d:?}"),
        }
    }

    #[test]
    fn buttons_name_their_prompt_and_press() {
        for p in [Press::Approve, Press::Decline, Press::Pick(1, 12), Press::Done(3)] {
            let id = button_id("k7", &p);
            assert!(id.len() <= 100);
            assert_eq!(parse_button(&id), Some(("k7".into(), p)));
        }
        assert_eq!(parse_button("other:k7:approve"), None);
        assert_eq!(parse_button("pdt:k7:launch"), None);
        let b = approval_buttons("k1", true);
        assert_eq!(b[0]["components"][0]["label"], "Approve");
        assert_eq!(b[0]["components"][1]["disabled"], true);
    }

    #[test]
    fn an_approval_says_what_the_pm_will_do() {
        let (text, file) = approval_text(
            "Create ticket: Fix the login",
            "mcp__pdt__create_ticket",
            &json!({"title": "Fix the login", "project": "PX"}),
        );
        assert!(text.starts_with("**The PM asks to:** Create ticket: Fix the login\n`create_ticket`"), "{text}");
        assert!(text.contains("\"project\": \"PX\"") && file.is_none());
        let (text, file) = approval_text("Edit", "mcp__pdt__edit_ticket", &json!({"description": "d".repeat(3000)}));
        assert!(text.ends_with("Its arguments are attached.") && file.unwrap().contains("ddd"));
        let (text, _) = approval_text("Look", "mcp__pdt__discord_get", &json!({}));
        assert!(!text.contains("```"));
    }

    #[test]
    fn questions_become_buttons() {
        let input = json!({"questions": [
            {"question": "Which project?", "header": "Project", "options": [{"label": "PX"}, {"label": "Other"}]},
            {"question": "Labels?", "header": "", "multiSelect": true, "options": [{"label": "bug"}, {"label": "ui"}]},
        ]});
        let qs = questions(&input);
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].options, ["PX", "Other"]);
        assert!(qs[1].multi);
        let (text, rows) = question_message("k2", 0, &qs[0], &[], false);
        assert!(text.starts_with("**Project**\nWhich project?"), "{text}");
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["components"][1]["custom_id"], "pdt:k2:q0o1");
        let (_, rows) = question_message("k2", 1, &qs[1], &["ui".into()], false);
        assert_eq!(rows.as_array().unwrap().len(), 2, "a Send row");
        assert_eq!(rows[0]["components"][1]["style"], 3, "picked");
        assert_eq!(rows[1]["components"][0]["custom_id"], "pdt:k2:q1done");
    }

    #[test]
    fn conversations_are_found_by_id_or_title() {
        let open = [("a1", "Release plan"), ("b2", "Fix the login"), ("c3", "Login copy")];
        assert_eq!(find_conversation("b2", &open), Ok("b2"));
        assert_eq!(find_conversation("release PLAN", &open), Ok("a1"));
        assert_eq!(find_conversation("copy", &open), Ok("c3"));
        assert!(find_conversation("login", &open).unwrap_err().contains("2 conversations"));
        assert!(find_conversation("nope", &open).unwrap_err().contains("/list"));
        let c = choices("log", open.iter().copied());
        assert_eq!(c.as_array().unwrap().len(), 2);
        assert_eq!(c[0], json!({"name": "Fix the login", "value": "b2"}));
    }

    #[test]
    fn the_commands_are_for_dms() {
        let all = commands();
        let names: Vec<&str> = all.as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["new", "list", "switch", "close"]);
        assert!(all.as_array().unwrap().iter().all(|c| c["contexts"] == json!([1])));
        assert_eq!(all[2]["options"][0]["autocomplete"], true);
    }
}
