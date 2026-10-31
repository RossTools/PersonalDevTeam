//! Google Docs (PDT-316): PDT's text edits as Docs API `batchUpdate` requests. Markdown becomes paragraphs with their
//! named styles (headings), bulleted and numbered lists (nested), tables, links, bold, italic, strikethrough and code.
//! Sections are found in the document's structure (`documents.get`): a heading's section runs to the next heading of
//! its level or higher. Indexes are UTF-16 code units, as the Docs API counts them. Pure.

use pdt_api::sdk::documents::{DocumentEdit, ProviderError};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde_json::{Value, json};

/// A run of text and its style.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
}

/// One paragraph's runs.
type Line = Vec<Run>;

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A paragraph; `level` 1–6 makes it a heading.
    Para {
        level: u8,
        runs: Line,
    },
    /// A list's items, each with its depth (0 at the top).
    List {
        ordered: bool,
        items: Vec<(usize, Line)>,
    },
    /// Rows of cells; the first row is the header.
    Table {
        rows: Vec<Vec<Line>>,
    },
    Code {
        lines: Vec<String>,
    },
}

fn utf16(s: &str) -> i64 {
    s.encode_utf16().count() as i64
}

#[derive(Default)]
struct Style {
    bold: usize,
    italic: usize,
    strike: usize,
    link: Vec<String>,
}

impl Style {
    fn run(&self, text: &str, code: bool) -> Run {
        Run {
            text: text.into(),
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            code,
            link: self.link.last().cloned(),
        }
    }
}

/// Markdown as blocks.
pub fn parse(markdown: &str) -> Vec<Block> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let mut out: Vec<Block> = vec![];
    let mut style = Style::default();
    // What is being filled: a paragraph's runs, the lists open, a table, a code block.
    let mut line: Line = vec![];
    let mut level: u8 = 0;
    let mut lists: Vec<bool> = vec![];
    let mut items: Vec<(usize, Line)> = vec![];
    let mut table: Option<Vec<Vec<Line>>> = None;
    let mut code: Option<String> = None;
    let push_text = |line: &mut Line, style: &Style, t: &str, code: bool| {
        let next = style.run(t, code);
        if let Some(last) = line.last_mut() {
            let same = (last.bold, last.italic, last.strike, last.code, &last.link)
                == (next.bold, next.italic, next.strike, next.code, &next.link);
            if same {
                last.text.push_str(t);
                return;
            }
        }
        line.push(next);
    };
    let flush_item = |line: &mut Line, lists: &Vec<bool>, items: &mut Vec<(usize, Line)>| {
        if !line.is_empty() {
            items.push((lists.len().saturating_sub(1), std::mem::take(line)));
        }
    };
    for ev in Parser::new_ext(markdown, opts) {
        match ev {
            Event::Start(Tag::Heading { level: l, .. }) => level = l as u8,
            Event::End(TagEnd::Heading(_)) => {
                out.push(Block::Para { level, runs: std::mem::take(&mut line) });
                level = 0;
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                if lists.is_empty() && table.is_none() {
                    out.push(Block::Para { level: 0, runs: std::mem::take(&mut line) });
                }
            }
            Event::Start(Tag::List(first)) => {
                flush_item(&mut line, &lists, &mut items);
                lists.push(first.is_some());
            }
            Event::End(TagEnd::List(_)) => {
                flush_item(&mut line, &lists, &mut items);
                let ordered = lists.pop().unwrap_or(false);
                if lists.is_empty() {
                    out.push(Block::List { ordered, items: std::mem::take(&mut items) });
                }
            }
            Event::Start(Tag::Item) => flush_item(&mut line, &lists, &mut items),
            Event::End(TagEnd::Item) => flush_item(&mut line, &lists, &mut items),
            Event::Start(Tag::Table(_)) => table = Some(vec![]),
            Event::End(TagEnd::Table) => {
                if let Some(rows) = table.take() {
                    out.push(Block::Table { rows });
                }
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                if let Some(t) = table.as_mut() {
                    t.push(vec![]);
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(row) = table.as_mut().and_then(|t| t.last_mut()) {
                    row.push(std::mem::take(&mut line));
                }
            }
            Event::Start(Tag::CodeBlock(_)) => code = Some(String::new()),
            Event::End(TagEnd::CodeBlock) => {
                let c = code.take().unwrap_or_default();
                out.push(Block::Code { lines: c.trim_end_matches('\n').split('\n').map(str::to_string).collect() });
            }
            Event::Start(Tag::Strong) => style.bold += 1,
            Event::End(TagEnd::Strong) => style.bold = style.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => style.italic += 1,
            Event::End(TagEnd::Emphasis) => style.italic = style.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => style.strike += 1,
            Event::End(TagEnd::Strikethrough) => style.strike = style.strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => style.link.push(dest_url.to_string()),
            Event::End(TagEnd::Link) => {
                style.link.pop();
            }
            Event::Text(t) => match code.as_mut() {
                Some(c) => c.push_str(&t),
                None => push_text(&mut line, &style, &t, false),
            },
            Event::Code(t) => push_text(&mut line, &style, &t, true),
            Event::SoftBreak | Event::HardBreak => push_text(&mut line, &style, " ", false),
            Event::Rule => out.push(Block::Para { level: 0, runs: vec![] }),
            _ => {}
        }
    }
    out
}

fn plain(runs: &Line) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

/// The requests that style `runs`, which start at `at`.
fn run_styles(runs: &Line, at: i64, out: &mut Vec<Value>) {
    let mut i = at;
    for r in runs {
        let n = utf16(&r.text);
        let mut style = serde_json::Map::new();
        let mut fields = vec![];
        if r.bold {
            style.insert("bold".into(), json!(true));
            fields.push("bold");
        }
        if r.italic {
            style.insert("italic".into(), json!(true));
            fields.push("italic");
        }
        if r.strike {
            style.insert("strikethrough".into(), json!(true));
            fields.push("strikethrough");
        }
        if r.code {
            style.insert("weightedFontFamily".into(), json!({"fontFamily": "Roboto Mono"}));
            fields.push("weightedFontFamily");
        }
        if let Some(u) = &r.link {
            style.insert("link".into(), json!({"url": u}));
            fields.push("link");
        }
        if n > 0 && !fields.is_empty() {
            out.push(json!({"updateTextStyle": {
                "range": {"startIndex": i, "endIndex": i + n},
                "textStyle": style, "fields": fields.join(",")}}));
        }
        i += n;
    }
}

/// Plain paragraphs: their named style, no bullets, no inherited character style.
fn reset(start: i64, end: i64, named: &str, out: &mut Vec<Value>) {
    let range = json!({"startIndex": start, "endIndex": end});
    out.push(json!({"updateParagraphStyle": {"range": range, "paragraphStyle": {"namedStyleType": named},
                                             "fields": "namedStyleType"}}));
    out.push(json!({"deleteParagraphBullets": {"range": range}}));
    out.push(json!({"updateTextStyle": {"range": range, "textStyle": {},
                                         "fields": "bold,italic,strikethrough,underline,link,weightedFontFamily"}}));
}

/// The requests that insert `blocks` at `at`, the start of a paragraph. Blocks go in from the last, each at `at`, so
/// no index moves under one being written.
pub fn insert(blocks: &[Block], at: i64) -> Vec<Value> {
    let mut out = vec![];
    for b in blocks.iter().rev() {
        match b {
            Block::Para { level, runs } => {
                let text = plain(runs);
                out.push(json!({"insertText": {"location": {"index": at}, "text": format!("{text}\n")}}));
                let named = if *level == 0 { "NORMAL_TEXT".to_string() } else { format!("HEADING_{level}") };
                reset(at, at + utf16(&text) + 1, &named, &mut out);
                run_styles(runs, at, &mut out);
            }
            Block::Code { lines } => {
                let text = lines.join("\n");
                let n = utf16(&text) + 1;
                out.push(json!({"insertText": {"location": {"index": at}, "text": format!("{text}\n")}}));
                reset(at, at + n, "NORMAL_TEXT", &mut out);
                out.push(json!({"updateTextStyle": {"range": {"startIndex": at, "endIndex": at + n - 1},
                    "textStyle": {"weightedFontFamily": {"fontFamily": "Roboto Mono"}}, "fields": "weightedFontFamily"}}));
            }
            Block::List { ordered, items } => {
                // Each item's leading tabs give its depth; Google takes them out when it makes the bullets, after
                // the runs are styled.
                let mut text = String::new();
                let mut styles = vec![];
                for (depth, runs) in items {
                    let tabs = "\t".repeat(*depth);
                    let start = at + utf16(&text) + utf16(&tabs);
                    text.push_str(&tabs);
                    text.push_str(&plain(runs));
                    text.push('\n');
                    styles.push((runs, start));
                }
                let n = utf16(&text);
                out.push(json!({"insertText": {"location": {"index": at}, "text": text}}));
                reset(at, at + n, "NORMAL_TEXT", &mut out);
                for (runs, start) in styles {
                    run_styles(runs, start, &mut out);
                }
                let preset = if *ordered { "NUMBERED_DECIMAL_ALPHA_ROMAN" } else { "BULLET_DISC_CIRCLE_SQUARE" };
                out.push(json!({"createParagraphBullets": {"range": {"startIndex": at, "endIndex": at + n},
                                                           "bulletPreset": preset}}));
            }
            Block::Table { rows } => {
                let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
                if rows.is_empty() || cols == 0 {
                    continue;
                }
                out.push(json!({"insertTable": {"rows": rows.len(), "columns": cols, "location": {"index": at}}}));
                // Google puts a newline before the table: it starts at `at + 1`; cell (r, c)'s paragraph is 3 in,
                // past the table, row and cell starts, and each row is its start and two per cell.
                let start = at + 1;
                for (r, row) in rows.iter().enumerate().rev() {
                    for (c, runs) in row.iter().enumerate().rev() {
                        let text = plain(runs);
                        if text.is_empty() {
                            continue;
                        }
                        let i = start + 3 + (r as i64) * (2 * cols as i64 + 1) + 2 * c as i64;
                        out.push(json!({"insertText": {"location": {"index": i}, "text": text}}));
                        run_styles(runs, i, &mut out);
                        if r == 0 {
                            out.push(json!({"updateTextStyle": {
                                "range": {"startIndex": i, "endIndex": i + utf16(&text)},
                                "textStyle": {"bold": true}, "fields": "bold"}}));
                        }
                    }
                }
            }
        }
    }
    out
}

/// A paragraph of the document's body: where it is, its text, and its heading level (0 for none; a title is 1).
#[derive(Debug, Clone, PartialEq)]
pub struct Para {
    pub start: i64,
    pub end: i64,
    pub text: String,
    pub level: u8,
}

/// The body's top-level elements (paragraphs, and tables as untitled paragraphs), and where the body ends.
pub fn structure(doc: &Value) -> (Vec<Para>, i64) {
    let mut out = vec![];
    let mut end = 1;
    for el in doc["body"]["content"].as_array().into_iter().flatten() {
        let (s, e) = (el["startIndex"].as_i64().unwrap_or(0), el["endIndex"].as_i64().unwrap_or(0));
        end = end.max(e);
        let p = &el["paragraph"];
        if p.is_null() {
            if el.get("table").is_some() {
                out.push(Para { start: s, end: e, text: String::new(), level: 0 });
            }
            continue;
        }
        let text: String =
            p["elements"].as_array().into_iter().flatten().filter_map(|x| x["textRun"]["content"].as_str()).collect();
        let named = p["paragraphStyle"]["namedStyleType"].as_str().unwrap_or("");
        let level = match named {
            "TITLE" => 1,
            n => n.strip_prefix("HEADING_").and_then(|l| l.parse().ok()).unwrap_or(0),
        };
        out.push(Para { start: s, end: e, text: text.trim_end_matches('\n').to_string(), level });
    }
    (out, end)
}

fn norm(s: &str) -> String {
    s.trim().trim_start_matches('#').split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The requests for one text edit, given the document as `documents.get` gave it. `None` for an edit that is not a
/// text one.
pub fn requests(doc: &Value, edit: &DocumentEdit) -> Result<Vec<Value>, ProviderError> {
    let (paras, end) = structure(doc);
    // The body always ends with a paragraph's newline, which stays.
    let last = end - 1;
    let empty = paras.iter().all(|p| p.text.is_empty() && p.level == 0) && paras.len() <= 1;
    // Blocks at the end: after a newline that makes a fresh last paragraph, unless the body is empty.
    let append = |blocks: Vec<Block>| -> Vec<Value> {
        if empty {
            return insert(&blocks, 1);
        }
        let mut out = vec![json!({"insertText": {"location": {"index": last}, "text": "\n"}})];
        out.extend(insert(&blocks, last + 1));
        out
    };
    let heading = |h: &str| -> Result<usize, ProviderError> {
        let want = norm(h);
        paras
            .iter()
            .position(|p| p.level > 0 && norm(&p.text) == want)
            .ok_or_else(|| ProviderError::Invalid(format!("the document has no heading called {}", h.trim())))
    };
    Ok(match edit {
        DocumentEdit::ReplaceText { markdown } => {
            let mut out = vec![];
            if last > 1 {
                out.push(json!({"deleteContentRange": {"range": {"startIndex": 1, "endIndex": last}}}));
            }
            out.extend(insert(&parse(markdown), 1));
            out
        }
        DocumentEdit::AppendText { markdown } => append(parse(markdown)),
        DocumentEdit::ReplaceAll { find, replace, match_case } => {
            if find.is_empty() {
                return Err(ProviderError::Invalid("there is nothing to find".into()));
            }
            vec![json!({"replaceAllText": {"containsText": {"text": find, "matchCase": match_case},
                                           "replaceText": replace}})]
        }
        DocumentEdit::InsertAfterHeading { heading: h, markdown } => {
            let i = heading(h)?;
            match paras.get(i + 1) {
                Some(next) => insert(&parse(markdown), next.start),
                None => append(parse(markdown)),
            }
        }
        DocumentEdit::ReplaceSection { heading: h, markdown } => {
            let i = heading(h)?;
            let level = paras[i].level;
            let next = paras[i + 1..].iter().find(|p| p.level > 0 && p.level <= level);
            let (from, to) = (paras[i].end, next.map_or(last, |p| p.start));
            let blocks = parse(markdown);
            let mut out = vec![];
            if to > from {
                out.push(json!({"deleteContentRange": {"range": {"startIndex": from, "endIndex": to}}}));
            }
            if from >= last {
                // The heading ends the document: its section goes after a fresh last paragraph.
                out.push(json!({"insertText": {"location": {"index": last}, "text": "\n"}}));
                out.extend(insert(&blocks, last + 1));
            } else {
                out.extend(insert(&blocks, from));
            }
            out
        }
        _ => {
            return Err(ProviderError::Unsupported(
                "a Google Doc has no sheets or cells: that is a Sheets edit".into(),
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &str) -> Run {
        Run { text: t.into(), ..Default::default() }
    }

    #[test]
    fn markdown_becomes_blocks() {
        let b = parse(
            "# Plan\n\nSome **bold** and [a link](https://x.dev).\n\n- one\n  - inner\n- two\n\n1. first\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```\nfn x()\n```\n",
        );
        assert_eq!(b[0], Block::Para { level: 1, runs: vec![run("Plan")] });
        let Block::Para { level: 0, runs } = &b[1] else { panic!("{b:?}") };
        assert_eq!(runs[1], Run { text: "bold".into(), bold: true, ..Default::default() });
        assert_eq!(runs[3].link.as_deref(), Some("https://x.dev"));
        assert_eq!(
            b[2],
            Block::List {
                ordered: false,
                items: vec![(0, vec![run("one")]), (1, vec![run("inner")]), (0, vec![run("two")])]
            }
        );
        assert!(matches!(&b[3], Block::List { ordered: true, items } if items.len() == 1));
        assert_eq!(
            b[4],
            Block::Table { rows: vec![vec![vec![run("A")], vec![run("B")]], vec![vec![run("1")], vec![run("2")]]] }
        );
        assert_eq!(b[5], Block::Code { lines: vec!["fn x()".into()] });
    }

    #[test]
    fn a_heading_and_text_are_inserted_with_their_styles() {
        let r = insert(&parse("## Goal\n\nShip **it**"), 10);
        // The last block goes in first.
        assert_eq!(r[0]["insertText"]["text"], "Ship it\n");
        assert_eq!(r[0]["insertText"]["location"]["index"], 10);
        assert_eq!(r[1]["updateParagraphStyle"]["paragraphStyle"]["namedStyleType"], "NORMAL_TEXT");
        let bold = r.iter().find(|x| x["updateTextStyle"]["fields"] == "bold").unwrap();
        assert_eq!(bold["updateTextStyle"]["range"], json!({"startIndex": 15, "endIndex": 17}));
        let h = r.iter().position(|x| x["insertText"]["text"] == "Goal\n").unwrap();
        assert_eq!(r[h + 1]["updateParagraphStyle"]["paragraphStyle"]["namedStyleType"], "HEADING_2");
    }

    #[test]
    fn indexes_count_utf16() {
        let r = insert(&parse("😀 **b**"), 1);
        let bold = r.iter().find(|x| x["updateTextStyle"]["fields"] == "bold").unwrap();
        // The emoji is two code units, then a space.
        assert_eq!(bold["updateTextStyle"]["range"]["startIndex"], 4);
    }

    #[test]
    fn a_list_is_one_set_of_bullets_with_tabs_for_depth() {
        let r = insert(&parse("1. a\n   1. b\n2. c"), 1);
        assert_eq!(r[0]["insertText"]["text"], "a\n\tb\nc\n");
        let bullets = r.last().unwrap();
        assert_eq!(bullets["createParagraphBullets"]["bulletPreset"], "NUMBERED_DECIMAL_ALPHA_ROMAN");
        assert_eq!(bullets["createParagraphBullets"]["range"], json!({"startIndex": 1, "endIndex": 8}));
    }

    #[test]
    fn a_table_fills_its_cells_from_the_last() {
        let r = insert(&parse("| A | B |\n|---|---|\n| 1 | 2 |"), 1);
        assert_eq!(r[0]["insertTable"], json!({"rows": 2, "columns": 2, "location": {"index": 1}}));
        let at: Vec<(String, i64)> = r
            .iter()
            .filter_map(|x| {
                Some((x["insertText"]["text"].as_str()?.to_string(), x["insertText"]["location"]["index"].as_i64()?))
            })
            .collect();
        // Table at 2; row 0's cells at 5 and 7, row 1's at 10 and 12.
        assert_eq!(at, [("2".into(), 12), ("1".into(), 10), ("B".into(), 7), ("A".into(), 5)]);
    }

    fn doc() -> Value {
        let para = |s: i64, e: i64, text: &str, named: &str| {
            json!({"startIndex": s, "endIndex": e, "paragraph": {
                "elements": [{"textRun": {"content": text}}], "paragraphStyle": {"namedStyleType": named}}})
        };
        json!({"body": {"content": [
            {"endIndex": 1, "sectionBreak": {}},
            para(1, 6, "Plan\n", "TITLE"),
            para(6, 11, "Goal\n", "HEADING_2"),
            para(11, 20, "Ship it.\n", "NORMAL_TEXT"),
            para(20, 25, "Risk\n", "HEADING_3"),
            para(25, 30, "Some\n", "NORMAL_TEXT"),
            para(30, 36, "Notes\n", "HEADING_2"),
            para(36, 41, "Last\n", "NORMAL_TEXT"),
        ]}})
    }

    #[test]
    fn a_section_runs_to_the_next_heading_of_its_level() {
        let e = DocumentEdit::ReplaceSection { heading: "## goal".into(), markdown: "New".into() };
        let r = requests(&doc(), &e).unwrap();
        // Goal's section has Risk (a lower heading) in it, up to Notes.
        assert_eq!(r[0]["deleteContentRange"]["range"], json!({"startIndex": 11, "endIndex": 30}));
        assert_eq!(r[1]["insertText"], json!({"location": {"index": 11}, "text": "New\n"}));
        let e = DocumentEdit::ReplaceSection { heading: "Notes".into(), markdown: "x".into() };
        let r = requests(&doc(), &e).unwrap();
        assert_eq!(r[0]["deleteContentRange"]["range"], json!({"startIndex": 36, "endIndex": 40}));
        assert!(matches!(
            requests(&doc(), &DocumentEdit::ReplaceSection { heading: "Nope".into(), markdown: String::new() }),
            Err(ProviderError::Invalid(m)) if m.contains("Nope")
        ));
    }

    #[test]
    fn inserting_after_a_heading_and_appending() {
        let r = requests(&doc(), &DocumentEdit::InsertAfterHeading { heading: "Risk".into(), markdown: "R".into() })
            .unwrap();
        assert_eq!(r[0]["insertText"]["location"]["index"], 25);
        let r = requests(&doc(), &DocumentEdit::AppendText { markdown: "End".into() }).unwrap();
        assert_eq!(r[0]["insertText"], json!({"location": {"index": 40}, "text": "\n"}));
        assert_eq!(r[1]["insertText"], json!({"location": {"index": 41}, "text": "End\n"}));
        let empty = json!({"body": {"content": [{"endIndex": 1, "sectionBreak": {}},
            {"startIndex": 1, "endIndex": 2, "paragraph": {"elements": [{"textRun": {"content": "\n"}}]}}]}});
        let r = requests(&empty, &DocumentEdit::AppendText { markdown: "First".into() }).unwrap();
        assert_eq!(r[0]["insertText"], json!({"location": {"index": 1}, "text": "First\n"}));
        let r = requests(&doc(), &DocumentEdit::ReplaceText { markdown: "All".into() }).unwrap();
        assert_eq!(r[0]["deleteContentRange"]["range"], json!({"startIndex": 1, "endIndex": 40}));
    }

    #[test]
    fn sheet_edits_are_not_doc_edits() {
        let e = DocumentEdit::AddSheet { name: "x".into() };
        assert!(matches!(requests(&doc(), &e), Err(ProviderError::Unsupported(_))));
    }
}
