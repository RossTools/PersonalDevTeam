//! What changed in a document between two reads (PDT-616), as short text for a watch's message and the
//! `document.changed` event's `diff`: a doc's changed headings and paragraphs (`-` the old, `+` the new, under the
//! heading they are in), a sheet's changed ranges and a few of their cells. Cut to a budget, with "N more changes".
//! Pure; pdt-api's documents sensor keeps the previous read to diff against.

use crate::document::{Cell, DocumentContent, Sheet};
use crate::text::clip;
use std::fmt::Write as _;

/// The most bytes a diff takes before the rest is counted, not shown.
pub const BUDGET: usize = 4 * 1024;
/// The most characters of one paragraph shown.
const BLOCK_MAX: usize = 600;
/// The most cells shown per sheet.
const CELLS_SHOWN: usize = 8;
/// Larger than this (blocks before × blocks after, past what both share at the ends), the middle is one change.
const LCS_MAX: usize = 4_000_000;

/// What changed from `old` to `new`, within `budget` bytes (plus the "more" line); None when nothing did.
pub fn diff(old: &DocumentContent, new: &DocumentContent, budget: usize) -> Option<String> {
    match (old, new) {
        (DocumentContent::Doc { markdown: a, .. }, DocumentContent::Doc { markdown: b, .. }) => doc_diff(a, b, budget),
        (DocumentContent::Sheet { sheets: a, .. }, DocumentContent::Sheet { sheets: b, .. }) => {
            sheet_diff(a, b, budget)
        }
        _ => Some("It changed from a doc to a sheet, or the other way.".into()),
    }
}

/// A doc's paragraphs: runs of lines between blank lines, each heading line one of its own.
fn blocks(markdown: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur: Vec<&str> = vec![];
    let flush = |cur: &mut Vec<&str>, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(cur.join("\n"));
            cur.clear();
        }
    };
    for line in markdown.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            flush(&mut cur, &mut out);
        } else if is_heading(line) {
            flush(&mut cur, &mut out);
            out.push(line.trim().to_string());
        } else {
            cur.push(line);
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn is_heading(line: &str) -> bool {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && t[hashes..].starts_with(' ')
}

/// One run of changed paragraphs: those removed from the old and added in the new, and the heading they are under.
struct Hunk<'a> {
    heading: Option<&'a str>,
    removed: Vec<&'a str>,
    added: Vec<&'a str>,
}

/// The paragraphs both share, in order, as pairs of indexes (a longest common subsequence).
fn common(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > LCS_MAX {
        return vec![];
    }
    // lengths[i][j]: the longest common run of a[i..] and b[j..].
    let mut lengths = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lengths[i][j] =
                if a[i] == b[j] { lengths[i + 1][j + 1] + 1 } else { lengths[i + 1][j].max(lengths[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if lengths[i + 1][j] >= lengths[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn hunks<'a>(a: &'a [String], b: &'a [String]) -> Vec<Hunk<'a>> {
    // What both share at the start and the end is no change; the paragraphs between are matched up.
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut pairs: Vec<(usize, usize)> = common(am, bm).into_iter().map(|(i, j)| (i + pre, j + pre)).collect();
    pairs.push((a.len() - suf, b.len() - suf));
    let heading_before = |j: usize| b[..j].iter().rev().find(|p| is_heading(p)).map(String::as_str);
    let (mut i, mut j, mut out) = (pre, pre, vec![]);
    for (ni, nj) in pairs {
        if ni > i || nj > j {
            out.push(Hunk {
                heading: heading_before(j),
                removed: a[i..ni].iter().map(String::as_str).collect(),
                added: b[j..nj].iter().map(String::as_str).collect(),
            });
        }
        (i, j) = (ni + 1, nj + 1);
    }
    out
}

fn marked(out: &mut String, mark: char, block: &str) {
    for line in clip(block, BLOCK_MAX).lines() {
        let _ = writeln!(out, "{mark} {line}");
    }
}

/// Pieces of text joined until `budget` is spent; the rest counted as "N more changes".
fn within(pieces: Vec<String>, budget: usize) -> Option<String> {
    if pieces.is_empty() {
        return None;
    }
    let mut out = String::new();
    let mut shown = 0;
    for p in &pieces {
        // The first piece is shown however long (it is cut by paragraph already), so a diff is never only "more".
        if shown > 0 && out.len() + 1 + p.len() > budget {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(p);
        shown += 1;
    }
    let more = pieces.len() - shown;
    let mut out = out.trim_end().to_string();
    if more > 0 {
        let _ = write!(out, "\n\n… {more} more {}", if more == 1 { "change" } else { "changes" });
    }
    Some(out)
}

fn doc_diff(old: &str, new: &str, budget: usize) -> Option<String> {
    let (a, b) = (blocks(old), blocks(new));
    let pieces = hunks(&a, &b)
        .into_iter()
        .map(|h| {
            let mut s = String::new();
            if let Some(head) = h.heading {
                let _ = writeln!(s, "In “{}”:", clip(head.trim_start_matches('#').trim(), 120));
            }
            for r in &h.removed {
                marked(&mut s, '-', r);
            }
            for a in &h.added {
                marked(&mut s, '+', a);
            }
            s
        })
        .collect();
    within(pieces, budget)
}

/// A1-style column letters for a 0-based column.
fn column(mut c: usize) -> String {
    let mut s = vec![];
    loop {
        s.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

fn a1(r: usize, c: usize) -> String {
    format!("{}{}", column(c), r + 1)
}

fn cell_at(s: &Sheet, r: usize, c: usize) -> Option<&Cell> {
    s.rows.get(r).and_then(|row| row.get(c))
}

/// What a cell shows in a diff: its formula, else its value.
fn shown(cell: Option<&Cell>) -> String {
    match cell {
        Some(Cell { formula: Some(f), .. }) if !f.is_empty() => f.clone(),
        Some(c) => c.value.clone(),
        None => String::new(),
    }
}

/// The changed cells, as rectangles: runs of columns in a row, joined down rows where the same run changed.
fn ranges(cells: &[(usize, usize)]) -> Vec<(usize, usize, usize, usize)> {
    let mut runs: Vec<(usize, usize, usize)> = vec![];
    for &(r, c) in cells {
        match runs.last_mut() {
            Some((lr, _, end)) if *lr == r && *end + 1 == c => *end = c,
            _ => runs.push((r, c, c)),
        }
    }
    let mut rects: Vec<(usize, usize, usize, usize)> = vec![];
    for (r, c0, c1) in runs {
        match rects.iter_mut().find(|(_, r1, a, b)| *r1 + 1 == r && *a == c0 && *b == c1) {
            Some(rect) => rect.1 = r,
            None => rects.push((r, r, c0, c1)),
        }
    }
    rects
}

fn sheet_diff(old: &[Sheet], new: &[Sheet], budget: usize) -> Option<String> {
    let mut pieces = vec![];
    for s in old.iter().filter(|s| !new.iter().any(|n| n.name == s.name)) {
        pieces.push(format!("Sheet “{}” was removed.", clip(&s.name, 80)));
    }
    for n in new {
        let empty = Sheet::default();
        let (o, added) = match old.iter().find(|o| o.name == n.name) {
            Some(o) => (o, false),
            None => (&empty, true),
        };
        let rows = o.rows.len().max(n.rows.len());
        let mut changed = vec![];
        for r in 0..rows {
            let cols = o.rows.get(r).map_or(0, Vec::len).max(n.rows.get(r).map_or(0, Vec::len));
            for c in 0..cols {
                if shown(cell_at(o, r, c)) != shown(cell_at(n, r, c)) {
                    changed.push((r, c));
                }
            }
        }
        if changed.is_empty() && !added {
            continue;
        }
        let name = clip(&n.name, 80);
        let rects: Vec<String> = ranges(&changed)
            .into_iter()
            .map(
                |(r0, r1, c0, c1)| {
                    if (r0, c0) == (r1, c1) { a1(r0, c0) } else { format!("{}:{}", a1(r0, c0), a1(r1, c1)) }
                },
            )
            .collect();
        let cells = if changed.len() == 1 { "1 cell".to_string() } else { format!("{} cells", changed.len()) };
        let mut s = if added {
            format!("Sheet “{name}” was added ({cells}).\n")
        } else {
            let mut list = rects.iter().take(10).cloned().collect::<Vec<_>>().join(", ");
            if rects.len() > 10 {
                let _ = write!(list, " and {} more ranges", rects.len() - 10);
            }
            format!("Sheet “{name}”: {list} ({cells})\n")
        };
        for &(r, c) in changed.iter().take(CELLS_SHOWN) {
            let (before, after) = (shown(cell_at(o, r, c)), shown(cell_at(n, r, c)));
            let q = |v: &str| if v.is_empty() { "(empty)".to_string() } else { format!("“{}”", clip(v, 80)) };
            let _ = writeln!(s, "{}: {} → {}", a1(r, c), q(&before), q(&after));
        }
        if changed.len() > CELLS_SHOWN {
            let _ = writeln!(s, "… {} more cells", changed.len() - CELLS_SHOWN);
        }
        pieces.push(s);
    }
    within(pieces, budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(md: &str) -> DocumentContent {
        DocumentContent::Doc { markdown: md.into(), version: None }
    }

    fn sheet(name: &str, rows: &[&[&str]]) -> Sheet {
        Sheet {
            name: name.into(),
            rows: rows
                .iter()
                .map(|r| r.iter().map(|v| Cell { value: v.to_string(), formula: None }).collect())
                .collect(),
            charts: vec![],
        }
    }

    #[test]
    fn nothing_changed_is_none() {
        assert_eq!(diff(&doc("# A\n\nText"), &doc("# A\n\nText\n"), BUDGET), None);
    }

    #[test]
    fn a_changed_paragraph_is_shown_under_its_heading() {
        let old = "# Plan\n\nIntro.\n\n## Scope\n\nShip the page.\nThen the docs.\n\n## Later\n\nMaybe.";
        let new = "# Plan\n\nIntro.\n\n## Scope\n\nShip the page and the API.\nThen the docs.\n\n## Later\n\nMaybe.\n\nNew idea.";
        assert_eq!(
            diff(&doc(old), &doc(new), BUDGET).unwrap(),
            "In “Scope”:\n- Ship the page.\n- Then the docs.\n+ Ship the page and the API.\n+ Then the docs.\n\nIn “Later”:\n+ New idea."
        );
    }

    #[test]
    fn a_changed_heading_is_the_change() {
        let d = diff(&doc("# Plan\n\n## Scope\n\nA"), &doc("# Plan\n\n## Goals\n\nA"), BUDGET).unwrap();
        assert_eq!(d, "In “Plan”:\n- ## Scope\n+ ## Goals");
    }

    #[test]
    fn a_long_diff_is_cut_to_its_budget_with_a_count() {
        let old: Vec<String> = (0..50).map(|i| format!("## H{i}\n\nPara {i}.")).collect();
        let new: Vec<String> = (0..50).map(|i| format!("## H{i}\n\nPara {i}, changed with more words.")).collect();
        let d = diff(&doc(&old.join("\n\n")), &doc(&new.join("\n\n")), 400).unwrap();
        let (shown, more) = d.rsplit_once("\n\n… ").unwrap();
        assert!(shown.len() <= 400, "{}", shown.len());
        let n: usize = more.split(' ').next().unwrap().parse().unwrap();
        assert_eq!(n + shown.matches("In “").count(), 50, "every change shown or counted");
        assert!(more.ends_with("more changes"));
        // One paragraph longer than the budget is still shown (cut), never only a count.
        let long = diff(&doc("a"), &doc(&"word ".repeat(400)), 100).unwrap();
        assert!(long.starts_with("- a\n+ word") && long.ends_with('…'), "{long}");
    }

    #[test]
    fn sheets_give_changed_ranges_and_a_few_cells() {
        let old = vec![sheet("Budget", &[&["Item", "Cost"], &["Pens", "3"], &["Ink", "5"]]), sheet("Old", &[&["x"]])];
        let new = vec![
            sheet("Budget", &[&["Item", "Cost"], &["Pens", "4"], &["Ink", "6"], &["Pads", "2"]]),
            sheet("New", &[&["y"]]),
        ];
        let d = diff(
            &DocumentContent::Sheet { sheets: old, version: None },
            &DocumentContent::Sheet { sheets: new, version: None },
            BUDGET,
        )
        .unwrap();
        assert_eq!(
            d,
            "Sheet “Old” was removed.\nSheet “Budget”: B2:B3, A4:B4 (4 cells)\nB2: “3” → “4”\nB3: “5” → “6”\nA4: (empty) → \
             “Pads”\nB4: (empty) → “2”\n\nSheet “New” was added (1 cell).\nA1: (empty) → “y”"
        );
    }

    #[test]
    fn many_cells_are_counted() {
        let old = vec![sheet("S", &[])];
        let rows: Vec<Vec<String>> = (0..5).map(|r| (0..4).map(|c| format!("{r}{c}")).collect()).collect();
        let rows: Vec<Vec<&str>> = rows.iter().map(|r| r.iter().map(String::as_str).collect()).collect();
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let new = vec![sheet("S", &rows)];
        let d = sheet_diff(&old, &new, BUDGET).unwrap();
        assert!(d.starts_with("Sheet “S”: A1:D5 (20 cells)\n"), "{d}");
        assert!(d.ends_with("… 12 more cells"), "{d}");
        assert_eq!(column(0), "A");
        assert_eq!(column(27), "AB");
    }
}
