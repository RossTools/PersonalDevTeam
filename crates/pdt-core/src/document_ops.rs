//! Document edits applied to content PDT holds in memory (PDT-317): a text document's Markdown, and a spreadsheet as
//! its sheets of cells. A provider that keeps Markdown (the local one, the test fake) applies text edits with
//! [`apply_doc_edit`]; [`apply_sheet_edit`] is the plain model of a sheet (no formulas computed, no formats), which
//! the fake uses. The rest are for the agent tools: a section of a doc, a range of a sheet, and a diff. Pure.

use crate::document::{
    Cell, CellAt, Chart, DocumentEdit, Sheet, SortKey, cell_name, parse_cell, parse_column, parse_range,
};

/// Why an edit could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// The provider (or this kind of document) can't do it.
    #[error("{0}")]
    Unsupported(String),
    /// It doesn't fit the document (a heading it lacks, a cell that isn't one).
    #[error("{0}")]
    Invalid(String),
}

fn invalid<T>(m: impl Into<String>) -> Result<T, EditError> {
    Err(EditError::Invalid(m.into()))
}

// ---- Markdown

/// A Markdown heading line's level and text (`## Scope` is (2, "Scope")).
fn heading_of(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    if line.len() - t.len() > 3 {
        return None;
    }
    let level = t.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &t[level..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((level, rest.trim().trim_end_matches('#').trim_end()))
}

/// The lines that aren't in a fenced code block (a `#` there is no heading).
fn outside_code(lines: &[&str]) -> Vec<bool> {
    let mut fence: Option<&str> = None;
    lines
        .iter()
        .map(|l| {
            let t = l.trim_start();
            let marker = ["```", "~~~"].into_iter().find(|m| t.starts_with(m));
            match (fence, marker) {
                (None, Some(m)) => {
                    fence = Some(m);
                    false
                }
                (Some(f), Some(m)) if f == m => {
                    fence = None;
                    false
                }
                (Some(_), _) => false,
                (None, None) => true,
            }
        })
        .collect()
}

fn same_heading(given: &str, text: &str) -> bool {
    let g = given.trim().trim_start_matches('#').trim();
    g.eq_ignore_ascii_case(text.trim())
}

/// A section's place: the heading's line, and the lines of its body (`start..end`).
fn find_section(lines: &[&str], heading: &str) -> Result<(usize, usize, usize), EditError> {
    let free = outside_code(lines);
    let found: Vec<(usize, usize)> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| free[*i])
        .filter_map(|(i, l)| heading_of(l).filter(|(_, t)| same_heading(heading, t)).map(|(lv, _)| (i, lv)))
        .collect();
    let (at, level) = match found.as_slice() {
        [one] => *one,
        [] => return invalid(format!("the document has no heading \"{}\"", heading.trim())),
        many => {
            return invalid(format!(
                "the document has {} headings \"{}\": change it with replace_string instead",
                many.len(),
                heading.trim()
            ));
        }
    };
    let end = (at + 1..lines.len())
        .find(|i| free[*i] && heading_of(lines[*i]).is_some_and(|(lv, _)| lv <= level))
        .unwrap_or(lines.len());
    Ok((at, at + 1, end))
}

fn split_lines(markdown: &str) -> Vec<&str> {
    markdown.lines().collect()
}

fn join(lines: Vec<String>, had_newline: bool) -> String {
    let mut s = lines.join("\n");
    if had_newline && !s.is_empty() {
        s.push('\n');
    }
    s
}

/// A block of Markdown as lines, with one blank line after it when more follows.
fn block(markdown: &str) -> Vec<String> {
    markdown.trim_matches('\n').lines().map(String::from).collect()
}

/// A section of a doc (its heading and body), for reading only part of it.
pub fn section(markdown: &str, heading: &str) -> Result<String, EditError> {
    let lines = split_lines(markdown);
    let (at, _, end) = find_section(&lines, heading)?;
    Ok(lines[at..end].join("\n").trim_end().to_string())
}

/// The headings of a doc, in order, with their levels.
pub fn headings(markdown: &str) -> Vec<(usize, String)> {
    let lines = split_lines(markdown);
    let free = outside_code(&lines);
    lines
        .iter()
        .enumerate()
        .filter(|(i, _)| free[*i])
        .filter_map(|(_, l)| heading_of(l))
        .map(|(lv, t)| (lv, t.into()))
        .collect()
}

/// A GFM table's lines: `head` the header row, `sep` the separator, `rows` its body (`rows.start..rows.end`).
struct Table {
    head: usize,
    end: usize,
}

fn is_table_line(l: &str) -> bool {
    l.trim_start().starts_with('|')
}

fn is_separator(l: &str) -> bool {
    let cells = row_cells(l);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim();
            !c.is_empty() && c.trim_matches(':').chars().all(|ch| ch == '-') && c.contains('-')
        })
}

/// A table row's cells, without the outer pipes (`\|` stays in the cell).
fn row_cells(l: &str) -> Vec<String> {
    let t = l.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').filter(|r| !r.ends_with('\\')).unwrap_or(t);
    let mut out = vec![String::new()];
    let mut esc = false;
    for ch in t.chars() {
        match ch {
            '|' if !esc => out.push(String::new()),
            _ => out.last_mut().expect("there is a cell").push(ch),
        }
        esc = ch == '\\' && !esc;
    }
    out.into_iter().map(|c| c.trim().to_string()).collect()
}

fn format_row(cells: &[String]) -> String {
    let cells: Vec<String> = cells.iter().map(|c| c.replace('\n', " ").replace('|', "\\|")).collect();
    format!("| {} |", cells.join(" | "))
}

fn tables(lines: &[&str], from: usize, to: usize) -> Vec<Table> {
    let free = outside_code(lines);
    let mut out = Vec::new();
    let mut i = from;
    while i + 1 < to {
        if free[i] && is_table_line(lines[i]) && is_table_line(lines[i + 1]) && is_separator(lines[i + 1]) {
            let mut end = i + 2;
            while end < to && free[end] && is_table_line(lines[end]) {
                end += 1;
            }
            out.push(Table { head: i, end });
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

fn find_table(lines: &[&str], heading: Option<&str>, n: Option<u32>) -> Result<Table, EditError> {
    let (from, to) = match heading {
        Some(h) => {
            let (_, start, end) = find_section(lines, h)?;
            (start, end)
        }
        None => (0, lines.len()),
    };
    let n = n.unwrap_or(1).max(1) as usize;
    let mut all = tables(lines, from, to);
    let count = all.len();
    if n > count {
        let place = heading.map(|h| format!(" under \"{}\"", h.trim())).unwrap_or_default();
        return invalid(match count {
            0 => format!("there is no table{place}"),
            c => format!("there are {c} tables{place}, not {n}"),
        });
    }
    Ok(all.swap_remove(n - 1))
}

/// A text document's Markdown after one edit.
pub fn apply_doc_edit(markdown: &str, edit: &DocumentEdit) -> Result<String, EditError> {
    let nl = markdown.ends_with('\n') || markdown.is_empty();
    let lines = split_lines(markdown);
    let owned = |r: &[&str]| r.iter().map(|l| l.to_string()).collect::<Vec<_>>();
    Ok(match edit {
        DocumentEdit::ReplaceText { markdown } => markdown.clone(),
        DocumentEdit::AppendText { markdown: m } => {
            let mut s = markdown.to_string();
            if !s.is_empty() && !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str(m);
            s
        }
        DocumentEdit::ReplaceSection { heading, markdown: m } => {
            let (_, start, end) = find_section(&lines, heading)?;
            let mut out = owned(&lines[..start]);
            out.push(String::new());
            out.extend(block(m));
            if end < lines.len() {
                out.push(String::new());
            }
            out.extend(owned(&lines[end..]));
            join(out, nl)
        }
        DocumentEdit::InsertAfterHeading { heading, markdown: m } => {
            let (_, start, end) = find_section(&lines, heading)?;
            let rest: Vec<String> = owned(&lines[start..end]);
            let body_starts = rest.iter().position(|l| !l.trim().is_empty());
            let mut out = owned(&lines[..start]);
            out.push(String::new());
            out.extend(block(m));
            if body_starts.is_some() || end < lines.len() {
                out.push(String::new());
            }
            out.extend(rest.into_iter().skip(body_starts.unwrap_or(end - start)));
            out.extend(owned(&lines[end..]));
            join(out, nl)
        }
        DocumentEdit::ReplaceString { find, replace, all } => {
            if find.is_empty() {
                return invalid("replace_string needs some text to find");
            }
            match (markdown.matches(find.as_str()).count(), all) {
                (0, _) => return invalid("the text to find isn't in the document"),
                (1, _) | (_, true) => markdown.replace(find.as_str(), replace),
                (n, false) => {
                    return invalid(format!(
                        "the text to find appears {n} times: give more of it so it appears once, or all: true"
                    ));
                }
            }
        }
        DocumentEdit::TableAppendRow { heading, table, cells } => {
            let t = find_table(&lines, heading.as_deref(), *table)?;
            let width = row_cells(lines[t.head]).len();
            let mut cells = cells.clone();
            if cells.len() > width {
                return invalid(format!("the table has {width} columns, not {}", cells.len()));
            }
            cells.resize(width, String::new());
            let mut out = owned(&lines[..t.end]);
            out.push(format_row(&cells));
            out.extend(owned(&lines[t.end..]));
            join(out, nl)
        }
        DocumentEdit::TableSetCell { heading, table, row, column, value } => {
            let t = find_table(&lines, heading.as_deref(), *table)?;
            let header = row_cells(lines[t.head]);
            let col = match column.trim().parse::<usize>() {
                Ok(n) if (1..=header.len()).contains(&n) => n - 1,
                Ok(n) => return invalid(format!("the table has {} columns, not {n}", header.len())),
                Err(_) => header
                    .iter()
                    .position(|h| h.eq_ignore_ascii_case(column.trim()))
                    .ok_or_else(|| EditError::Invalid(format!("the table has no column \"{}\"", column.trim())))?,
            };
            let rows = t.end - t.head - 2;
            let r = *row as usize;
            if r == 0 || r > rows {
                return invalid(format!("the table has {rows} rows, not {r}"));
            }
            let at = t.head + 1 + r;
            let mut cells = row_cells(lines[at]);
            cells.resize(header.len().max(cells.len()), String::new());
            cells[col] = value.clone();
            let mut out = owned(&lines);
            out[at] = format_row(&cells);
            join(out, nl)
        }
        e => {
            return Err(EditError::Unsupported(format!("{} is for spreadsheets, and this is a text document", e.op())));
        }
    })
}

/// What changed between two versions of a text, as a unified diff (empty when nothing did).
pub fn diff(old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    similar::TextDiff::from_lines(old, new).unified_diff().context_radius(2).header("before", "after").to_string()
}

// ---- Sheets

fn sheet_mut<'a>(sheets: &'a mut [Sheet], name: &str) -> Result<&'a mut Sheet, EditError> {
    sheets
        .iter_mut()
        .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| EditError::Invalid(format!("there is no sheet called {}", name.trim())))
}

fn range_of(range: &str) -> Result<(CellAt, CellAt), EditError> {
    parse_range(range).ok_or_else(|| EditError::Invalid(format!("{range} is not a range (A1:C10) or a cell")))
}

/// A number, if the text is one (`1,200` and `12%` too).
fn number(v: &str) -> Option<f64> {
    let t = v.trim().replace(',', "");
    match t.strip_suffix('%') {
        Some(p) => p.trim().parse::<f64>().ok().map(|n| n / 100.0),
        None => t.parse().ok(),
    }
}

fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    match (number(a), number(b), a.trim().is_empty(), b.trim().is_empty()) {
        // Empty cells go last either way round.
        (_, _, true, false) => std::cmp::Ordering::Greater,
        (_, _, false, true) => std::cmp::Ordering::Less,
        (Some(x), Some(y), _, _) => x.total_cmp(&y),
        (Some(_), None, _, _) => std::cmp::Ordering::Less,
        (None, Some(_), _, _) => std::cmp::Ordering::Greater,
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

/// A sheet's cells as a grid at least `rows` × `cols`.
fn grow(s: &mut Sheet, rows: usize, cols: usize) {
    if s.rows.len() < rows {
        s.rows.resize(rows, Vec::new());
    }
    for r in s.rows.iter_mut().take(rows) {
        if r.len() < cols {
            r.resize(cols, Cell::default());
        }
    }
}

/// A typed value as a cell, as a sheet reads it: `=` starts a formula, a leading `'` makes the rest text.
fn cell_from(v: String) -> Cell {
    if let Some(text) = v.strip_prefix('\'') {
        return Cell { value: text.to_string(), formula: None };
    }
    match v.starts_with('=') {
        // Nothing computes it here: a formula shows itself.
        true => Cell { value: v.clone(), formula: Some(v) },
        false => Cell { value: v, formula: None },
    }
}

fn check_chart(sheets: &[Sheet], sheet: &str, data: &str) -> Result<(), EditError> {
    if !sheets.iter().any(|s| s.name.eq_ignore_ascii_case(sheet.trim())) {
        return invalid(format!("there is no sheet called {}", sheet.trim()));
    }
    range_of(data).map(|_| ())
}

/// A spreadsheet's sheets after one edit, kept as plain cells: values and formulas (none computed) and charts.
/// Formats, conditional formats and filters need a real spreadsheet: they are unsupported here.
pub fn apply_sheet_edit(sheets: &mut Vec<Sheet>, edit: DocumentEdit) -> Result<(), EditError> {
    match edit {
        DocumentEdit::SetCells { sheet, cell, values } => {
            let s = sheet_mut(sheets, &sheet)?;
            let (row, col) = parse_cell(&cell).ok_or_else(|| EditError::Invalid(format!("{cell} is not a cell")))?;
            let width = values.iter().map(Vec::len).max().unwrap_or(0);
            grow(s, row + values.len(), col + width);
            for (i, line) in values.into_iter().enumerate() {
                for (j, v) in line.into_iter().enumerate() {
                    s.rows[row + i][col + j] = cell_from(v);
                }
            }
        }
        DocumentEdit::AddSheet { name } => {
            let name = name.trim().to_string();
            if name.is_empty() {
                return invalid("a sheet needs a name");
            }
            if sheets.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
                return invalid(format!("there is a sheet called {name} already"));
            }
            sheets.push(Sheet { name, ..Default::default() });
        }
        DocumentEdit::RenameSheet { sheet, name } => {
            let name = name.trim().to_string();
            if name.is_empty() {
                return invalid("a sheet needs a name");
            }
            if sheets.iter().any(|s| s.name.eq_ignore_ascii_case(&name) && !s.name.eq_ignore_ascii_case(sheet.trim())) {
                return invalid(format!("there is a sheet called {name} already"));
            }
            sheet_mut(sheets, &sheet)?.name = name;
        }
        DocumentEdit::DeleteSheet { sheet } => {
            sheet_mut(sheets, &sheet)?;
            if sheets.len() == 1 {
                return invalid("a spreadsheet keeps at least one sheet");
            }
            sheets.retain(|s| !s.name.eq_ignore_ascii_case(sheet.trim()));
        }
        DocumentEdit::AddChart { sheet, id, chart } => {
            check_chart(sheets, &sheet, &chart.data)?;
            let id = id
                .filter(|i| !i.trim().is_empty())
                .ok_or_else(|| EditError::Invalid("a new chart needs an id".into()))?;
            if sheets.iter().flat_map(|s| &s.charts).any(|c| c.id == id) {
                return invalid(format!("there is a chart {id} already"));
            }
            sheet_mut(sheets, &sheet)?.charts.push(Chart { id, spec: chart });
        }
        DocumentEdit::UpdateChart { id, chart } => {
            let c = sheets
                .iter_mut()
                .flat_map(|s| s.charts.iter_mut())
                .find(|c| c.id == id)
                .ok_or_else(|| EditError::Invalid(format!("there is no chart {id}")))?;
            range_of(&chart.data)?;
            c.spec = chart;
        }
        DocumentEdit::DeleteChart { id } => {
            let before: usize = sheets.iter().map(|s| s.charts.len()).sum();
            for s in sheets.iter_mut() {
                s.charts.retain(|c| c.id != id);
            }
            if before == sheets.iter().map(|s| s.charts.len()).sum::<usize>() {
                return invalid(format!("there is no chart {id}"));
            }
        }
        DocumentEdit::Sort { sheet, range, by, header } => {
            let s = sheet_mut(sheets, &sheet)?;
            let ((r0, c0), (r1, c1)) = range_of(&range)?;
            let keys = sort_keys(&by, c0, c1)?;
            grow(s, r1 + 1, c1 + 1);
            let first = if header { r0 + 1 } else { r0 };
            if first <= r1 {
                let mut block: Vec<Vec<Cell>> = s.rows[first..=r1].iter().map(|r| r[c0..=c1].to_vec()).collect();
                block.sort_by(|a, b| {
                    keys.iter()
                        .map(|(k, desc)| {
                            let o = compare(&a[*k].value, &b[*k].value);
                            if *desc { o.reverse() } else { o }
                        })
                        .find(|o| o.is_ne())
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                for (i, row) in block.into_iter().enumerate() {
                    s.rows[first + i].splice(c0..=c1, row);
                }
            }
        }
        e if e.for_docs() => {
            return Err(EditError::Unsupported(format!("{} is for text documents, and this is a spreadsheet", e.op())));
        }
        e => return Err(EditError::Unsupported(format!("{} isn't supported by this provider", e.op()))),
    }
    Ok(())
}

/// Sort keys as offsets in the range, each column inside it.
fn sort_keys(by: &[SortKey], c0: usize, c1: usize) -> Result<Vec<(usize, bool)>, EditError> {
    if by.is_empty() {
        return invalid("sort needs at least one column to sort by");
    }
    by.iter()
        .map(|k| {
            let c = parse_column(&k.column)
                .ok_or_else(|| EditError::Invalid(format!("{} is not a column (A, B, …)", k.column)))?;
            if !(c0..=c1).contains(&c) {
                return invalid(format!("column {} is outside the range", k.column.trim()));
            }
            Ok((c - c0, k.descending))
        })
        .collect()
}

/// A range of a sheet's cells, each with its place (`B3`), for reading part of a sheet; empty cells left out.
pub fn cells_in(sheet: &Sheet, range: Option<&str>) -> Result<Vec<(String, Cell)>, EditError> {
    let ((r0, c0), (r1, c1)) = match range.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => range_of(r)?,
        None => ((0, 0), (usize::MAX, usize::MAX)),
    };
    Ok(sheet
        .rows
        .iter()
        .enumerate()
        .skip(r0)
        .take_while(|(r, _)| *r <= r1)
        .flat_map(|(r, row)| {
            row.iter()
                .enumerate()
                .skip(c0)
                .take_while(move |(c, _)| *c <= c1)
                .filter(|(_, cell)| !cell.value.is_empty() || cell.formula.is_some())
                .map(move |(c, cell)| (cell_name(r, c), cell.clone()))
        })
        .collect())
}

/// A sheet's used size: (rows, columns).
pub fn used_size(sheet: &Sheet) -> (usize, usize) {
    let filled = |r: &Vec<Cell>| r.iter().rposition(|c| !c.value.is_empty() || c.formula.is_some()).map(|c| c + 1);
    let rows = sheet.rows.iter().rposition(|r| filled(r).is_some()).map_or(0, |r| r + 1);
    let cols = sheet.rows.iter().filter_map(filled).max().unwrap_or(0);
    (rows, cols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ChartKind, ChartSpec};

    const DOC: &str = "# Plan\n\nIntro.\n\n## Scope\n\nOld scope.\n\n### Detail\n\nDeep.\n\n## Notes\n\nN.\n";

    fn edit(md: &str, e: DocumentEdit) -> Result<String, EditError> {
        apply_doc_edit(md, &e)
    }

    #[test]
    fn a_section_is_replaced_up_to_the_next_heading_of_its_level() {
        let out = edit(DOC, DocumentEdit::ReplaceSection { heading: "scope".into(), markdown: "New scope.\n".into() })
            .unwrap();
        assert_eq!(out, "# Plan\n\nIntro.\n\n## Scope\n\nNew scope.\n\n## Notes\n\nN.\n");
        // `#`s are optional, and the last section runs to the end.
        let out =
            edit(DOC, DocumentEdit::ReplaceSection { heading: "## Notes".into(), markdown: "M.".into() }).unwrap();
        assert!(out.ends_with("## Notes\n\nM.\n"), "{out}");
        assert!(section(DOC, "Scope").unwrap().contains("Deep."));
        assert_eq!(section(DOC, "Detail").unwrap(), "### Detail\n\nDeep.");
    }

    #[test]
    fn missing_or_repeated_headings_are_refused() {
        let e = edit(DOC, DocumentEdit::ReplaceSection { heading: "Goal".into(), markdown: "x".into() }).unwrap_err();
        assert_eq!(e, EditError::Invalid("the document has no heading \"Goal\"".into()));
        let twice = "## A\n\nx\n\n## A\n\ny\n";
        assert!(matches!(
            edit(twice, DocumentEdit::InsertAfterHeading { heading: "A".into(), markdown: "z".into() }),
            Err(EditError::Invalid(m)) if m.contains("2 headings")
        ));
    }

    #[test]
    fn headings_in_code_blocks_are_not_headings() {
        let md = "## Real\n\n```\n## Fake\n```\n\nafter\n";
        assert_eq!(headings(md), vec![(2, "Real".to_string())]);
        assert!(section(md, "Fake").is_err());
        assert!(section(md, "Real").unwrap().contains("after"));
    }

    #[test]
    fn text_goes_under_a_heading() {
        let out =
            edit(DOC, DocumentEdit::InsertAfterHeading { heading: "Scope".into(), markdown: "First.".into() }).unwrap();
        assert!(out.contains("## Scope\n\nFirst.\n\nOld scope.\n"), "{out}");
        let out =
            edit("## Empty\n", DocumentEdit::InsertAfterHeading { heading: "Empty".into(), markdown: "x".into() })
                .unwrap();
        assert_eq!(out, "## Empty\n\nx\n");
    }

    #[test]
    fn text_is_replaced_once_unless_all() {
        let out = edit(DOC, DocumentEdit::ReplaceString { find: "Old".into(), replace: "Fresh".into(), all: false });
        assert!(out.unwrap().contains("Fresh scope."));
        let md = "a b a";
        assert!(edit(md, DocumentEdit::ReplaceString { find: "a".into(), replace: "c".into(), all: false }).is_err());
        assert_eq!(
            edit(md, DocumentEdit::ReplaceString { find: "a".into(), replace: "c".into(), all: true }).unwrap(),
            "c b c"
        );
        assert!(edit(md, DocumentEdit::ReplaceString { find: "z".into(), replace: "c".into(), all: true }).is_err());
    }

    const TABLES: &str =
        "# T\n\n| Name | Count |\n|---|--:|\n| a | 1 |\n\n## Second\n\n| X | Y |\n| - | - |\n| 1 | 2 |\n| 3 | 4 |\n";

    #[test]
    fn rows_are_added_to_tables() {
        let out =
            edit(TABLES, DocumentEdit::TableAppendRow { heading: None, table: None, cells: vec!["b".into()] }).unwrap();
        assert!(out.contains("| a | 1 |\n| b |  |\n\n## Second"), "{out}");
        let out = edit(
            TABLES,
            DocumentEdit::TableAppendRow {
                heading: Some("Second".into()),
                table: None,
                cells: vec!["5".into(), "6".into()],
            },
        )
        .unwrap();
        assert!(out.ends_with("| 3 | 4 |\n| 5 | 6 |\n"), "{out}");
        assert!(edit(TABLES, DocumentEdit::TableAppendRow { heading: None, table: Some(3), cells: vec![] }).is_err());
        let wide = DocumentEdit::TableAppendRow { heading: None, table: None, cells: vec!["1".into(); 3] };
        assert!(edit(TABLES, wide).is_err());
    }

    #[test]
    fn table_cells_are_set_by_header_or_number() {
        let set = |column: &str, row, value: &str| DocumentEdit::TableSetCell {
            heading: None,
            table: Some(2),
            row,
            column: column.into(),
            value: value.into(),
        };
        let out = edit(TABLES, set("y", 2, "a|b")).unwrap();
        assert!(out.contains("| 3 | a\\|b |"), "{out}");
        assert!(edit(TABLES, set("1", 1, "9")).unwrap().contains("| 9 | 2 |"));
        assert!(edit(TABLES, set("Z", 1, "9")).is_err());
        assert!(edit(TABLES, set("X", 3, "9")).is_err());
        // An escaped pipe stays inside its cell.
        assert_eq!(row_cells("| a\\|b | c |"), vec!["a\\|b".to_string(), "c".to_string()]);
    }

    #[test]
    fn sheet_edits_are_refused_on_a_doc() {
        let e = edit(DOC, DocumentEdit::AddSheet { name: "S".into() }).unwrap_err();
        assert!(matches!(e, EditError::Unsupported(m) if m.contains("add_sheet")));
    }

    #[test]
    fn a_diff_shows_what_changed() {
        assert_eq!(diff("a\n", "a\n"), "");
        let d = diff("a\nb\nc\n", "a\nB\nc\n");
        assert!(d.contains("-b\n+B\n"), "{d}");
    }

    fn book() -> Vec<Sheet> {
        vec![Sheet { name: "Sheet1".into(), ..Default::default() }]
    }

    fn values(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|r| r.iter().map(|v| v.to_string()).collect()).collect()
    }

    fn col(s: &Sheet, c: usize) -> Vec<String> {
        s.rows.iter().map(|r| r.get(c).map(|x| x.value.clone()).unwrap_or_default()).collect()
    }

    #[test]
    fn cells_are_set_and_read_back_by_range() {
        let mut b = book();
        let set = DocumentEdit::SetCells {
            sheet: "sheet1".into(),
            cell: "B2".into(),
            values: values(&[&["Month", "Sales"], &["Jan", "=1+1"]]),
        };
        apply_sheet_edit(&mut b, set).unwrap();
        assert_eq!(used_size(&b[0]), (3, 3));
        let cells = cells_in(&b[0], Some("C2:C3")).unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[1].0, "C3");
        assert_eq!(cells[1].1.formula.as_deref(), Some("=1+1"));
        assert_eq!(cells_in(&b[0], None).unwrap().len(), 4);
        // A leading `'` makes the rest text.
        let quoted = DocumentEdit::SetCells { sheet: "Sheet1".into(), cell: "A1".into(), values: values(&[&["'=2"]]) };
        apply_sheet_edit(&mut b, quoted).unwrap();
        let a1 = &cells_in(&b[0], Some("A1")).unwrap()[0].1;
        assert_eq!((a1.value.as_str(), a1.formula.as_deref()), ("=2", None));
        assert!(cells_in(&b[0], Some("nope")).is_err());
    }

    #[test]
    fn sheets_are_added_renamed_and_deleted() {
        let mut b = book();
        apply_sheet_edit(&mut b, DocumentEdit::AddSheet { name: "Data".into() }).unwrap();
        assert!(apply_sheet_edit(&mut b, DocumentEdit::AddSheet { name: "data".into() }).is_err());
        apply_sheet_edit(&mut b, DocumentEdit::RenameSheet { sheet: "Data".into(), name: "Raw".into() }).unwrap();
        assert!(
            apply_sheet_edit(&mut b, DocumentEdit::RenameSheet { sheet: "Raw".into(), name: "Sheet1".into() }).is_err()
        );
        apply_sheet_edit(&mut b, DocumentEdit::DeleteSheet { sheet: "Sheet1".into() }).unwrap();
        assert_eq!(b.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Raw"]);
        assert!(apply_sheet_edit(&mut b, DocumentEdit::DeleteSheet { sheet: "Raw".into() }).is_err());
    }

    #[test]
    fn charts_are_added_updated_and_deleted() {
        let mut b = book();
        let spec =
            ChartSpec { kind: ChartKind::Column, data: "A1:B4".into(), title: Some("Sales".into()), anchor: None };
        let add =
            |id: &str| DocumentEdit::AddChart { sheet: "Sheet1".into(), id: Some(id.into()), chart: spec.clone() };
        apply_sheet_edit(&mut b, add("c1")).unwrap();
        assert!(apply_sheet_edit(&mut b, add("c1")).is_err());
        let line = ChartSpec { kind: ChartKind::Line, ..spec.clone() };
        apply_sheet_edit(&mut b, DocumentEdit::UpdateChart { id: "c1".into(), chart: line }).unwrap();
        assert_eq!(b[0].charts[0].spec.kind, ChartKind::Line);
        let bad = ChartSpec { data: "x".into(), ..spec.clone() };
        assert!(
            apply_sheet_edit(
                &mut b,
                DocumentEdit::AddChart { sheet: "Sheet1".into(), id: Some("c2".into()), chart: bad }
            )
            .is_err()
        );
        apply_sheet_edit(&mut b, DocumentEdit::DeleteChart { id: "c1".into() }).unwrap();
        assert!(b[0].charts.is_empty());
        assert!(apply_sheet_edit(&mut b, DocumentEdit::DeleteChart { id: "c1".into() }).is_err());
    }

    #[test]
    fn a_range_sorts_by_its_columns_keeping_its_header() {
        let mut b = book();
        let v = values(&[&["Name", "N"], &["b", "10"], &["a", "9"], &["c", ""], &["d", "9"]]);
        apply_sheet_edit(&mut b, DocumentEdit::SetCells { sheet: "Sheet1".into(), cell: "A1".into(), values: v })
            .unwrap();
        let by =
            vec![SortKey { column: "B".into(), descending: false }, SortKey { column: "A".into(), descending: true }];
        apply_sheet_edit(
            &mut b,
            DocumentEdit::Sort { sheet: "Sheet1".into(), range: "A1:B5".into(), by, header: true },
        )
        .unwrap();
        assert_eq!(col(&b[0], 0), ["Name", "d", "a", "b", "c"]);
        let out = DocumentEdit::Sort {
            sheet: "Sheet1".into(),
            range: "A1:B5".into(),
            by: vec![SortKey { column: "C".into(), descending: false }],
            header: false,
        };
        assert!(apply_sheet_edit(&mut b, out).is_err());
    }

    #[test]
    fn formats_and_filters_are_unsupported_in_plain_sheets() {
        let mut b = book();
        let e = apply_sheet_edit(&mut b, DocumentEdit::ClearFilter { sheet: "Sheet1".into() }).unwrap_err();
        assert!(matches!(e, EditError::Unsupported(m) if m.contains("clear_filter")));
        let e = apply_sheet_edit(&mut b, DocumentEdit::AppendText { markdown: "x".into() }).unwrap_err();
        assert!(matches!(e, EditError::Unsupported(m) if m.contains("text documents")));
    }
}
