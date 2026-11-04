//! What a sheet may hold before IronCalc sees it (PDT-533). IronCalc's formula parser and evaluator recurse with no
//! depth limit, and its `.xlsx` importer trusts the package: a few kilobytes of nested brackets overflow the stack
//! (an abort, not a panic), and a small zip can unpack to gigabytes of cells. Every way in checks first: a formula
//! ([`check_formula`]), a `.xlsx` package ([`check_xlsx`]) and the number of cells a `.csv` makes ([`MAX_CELLS`]).
//! pdtd's threads also have a stack big enough for the deepest formula these limits let through.

use crate::Result;
use quick_xml::events::Event;
use std::io::{BufReader, Cursor, Read};

/// The longest formula, in characters (Excel's limit).
pub const MAX_FORMULA_CHARS: usize = 8192;

/// The most brackets a formula nests, `(` and `{` (Excel allows 64 levels of functions).
pub const MAX_FORMULA_DEPTH: usize = 64;

/// The most operators in a formula (`+`, `-`, `&`, `%`, …): IronCalc nests a level for each, as it does for a bracket.
pub const MAX_FORMULA_OPERATORS: usize = 1024;

/// The most cells (and shared strings) a sheet file brings in.
pub const MAX_CELLS: usize = 1_000_000;

/// The most bytes a `.xlsx` package unpacks to, all its parts together.
pub const MAX_UNPACKED: u64 = 200 * 1024 * 1024;

/// The most XML elements in a `.xlsx` package, all its parts together.
const MAX_ELEMENTS: usize = 5_000_000;

/// The deepest XML elements nest in a part.
const MAX_XML_DEPTH: usize = 256;

/// An error unless `formula` (with or without its `=`) is short enough, and nests few enough brackets and has few
/// enough operators, for IronCalc. Brackets and operators inside text (`"(("`), a quoted sheet name (`'a (b)'!A1`) or a table's column (`T[a(b]`) don't count,
/// read as IronCalc's lexer reads them. (Should the two ever disagree, the length limit and pdtd's stack still hold.)
pub fn check_formula(formula: &str) -> Result<()> {
    if formula.chars().count() > MAX_FORMULA_CHARS {
        return Err(format!("a formula is longer than {MAX_FORMULA_CHARS} characters"));
    }
    #[derive(PartialEq)]
    enum In {
        Formula,
        Text,
        Name,
    }
    let (mut depth, mut deepest, mut operators, mut at) = (0usize, 0usize, 0usize, In::Formula);
    // Inside a table's columns (`[…]`): how many are open, and the depth at the first, which a `)` there can't undo.
    let (mut columns, mut floor) = (0usize, 0usize);
    let mut chars = formula.chars();
    while let Some(ch) = chars.next() {
        match (&at, ch) {
            // `""` in text and `''` in a name close and open again, which leaves them the same.
            (In::Text, '"') | (In::Name, '\'') => at = In::Formula,
            (In::Text | In::Name, _) => {}
            (In::Formula, '[') => {
                if columns == 0 {
                    floor = depth;
                }
                columns += 1;
            }
            (In::Formula, ']') => columns = columns.saturating_sub(1),
            // In a column, `'` escapes the next character, and quotes are plain characters.
            (In::Formula, '\'') if columns > 0 => {
                chars.next();
            }
            (In::Formula, '"') if columns == 0 => at = In::Text,
            (In::Formula, '\'') => at = In::Name,
            (In::Formula, '(' | '{') => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            (In::Formula, ')' | '}') => depth = depth.saturating_sub(1).max(if columns > 0 { floor } else { 0 }),
            (In::Formula, '+' | '-' | '*' | '/' | '^' | '&' | '%' | '=' | '<' | '>' | ':' | '@') if columns == 0 => {
                operators += 1;
            }
            _ => {}
        }
    }
    if deepest > MAX_FORMULA_DEPTH {
        return Err(format!("a formula nests more than {MAX_FORMULA_DEPTH} levels of brackets"));
    }
    if operators > MAX_FORMULA_OPERATORS {
        return Err(format!("a formula has more than {MAX_FORMULA_OPERATORS} operators"));
    }
    Ok(())
}

/// [`check_formula`] for what a cell is given: only a formula (`=…`) is checked.
pub fn check_input(value: &str) -> Result<()> {
    if value.trim_start().starts_with('=') { check_formula(value) } else { Ok(()) }
}

/// An error unless a `.xlsx` package is one IronCalc can load safely: it unpacks to at most [`MAX_UNPACKED`] bytes
/// (counted as it is read, since the sizes a zip declares can lie), its XML nests and counts few enough elements, it
/// has at most [`MAX_CELLS`] cells and shared strings, and every formula in it passes [`check_formula`].
pub fn check_xlsx(bytes: &[u8]) -> Result<()> {
    let unreadable = |e: &dyn std::fmt::Display| format!("not a spreadsheet PDT can read: {e}");
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| unreadable(&e))?;
    let too_big = || format!("the spreadsheet unpacks to more than {} MB", MAX_UNPACKED / (1024 * 1024));
    let declared = (0..zip.len()).try_fold(0u64, |sum, i| {
        let size = zip.by_index_raw(i).map_err(|e| unreadable(&e))?.size();
        Ok::<_, String>(sum.saturating_add(size))
    })?;
    if declared > MAX_UNPACKED {
        return Err(too_big());
    }
    let mut count = Count::default();
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| unreadable(&e))?;
        let name = entry.name().map_err(|e| unreadable(&e))?.into_owned();
        let left = MAX_UNPACKED - count.bytes;
        let mut reader = Counted { inner: entry.take(left + 1), n: 0 };
        // Every part, whatever its name: IronCalc reads a worksheet from wherever the workbook's rels point.
        scan_xml(BufReader::new(&mut reader), &mut count).map_err(|e| format!("{name}: {e}"))?;
        std::io::copy(&mut reader, &mut std::io::sink()).map_err(|e| unreadable(&e))?;
        if reader.n > left {
            return Err(too_big());
        }
        count.bytes += reader.n;
    }
    Ok(())
}

#[derive(Default)]
struct Count {
    bytes: u64,
    elements: usize,
    cells: usize,
    strings: usize,
}

/// A reader that counts the bytes read through it.
struct Counted<R> {
    inner: R,
    n: u64,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.n += n as u64;
        Ok(n)
    }
}

/// The elements whose text IronCalc parses as a formula.
fn is_formula(name: &str) -> bool {
    matches!(name, "f" | "formula" | "formula1" | "formula2" | "definedName")
}

/// Walks one part as XML, counting its elements, cells and shared strings (in any part), and checking its formulas.
/// A part stops being read where it stops being XML (a picture, say): IronCalc can't read past there either, and
/// every formula closed before it has been checked (one still open is checked as far as it goes).
fn scan_xml(reader: impl std::io::BufRead, count: &mut Count) -> Result<()> {
    let mut xml = quick_xml::Reader::from_reader(reader);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    // The formula being read: the depth of its element, and its text so far (cut off just past the longest).
    let mut formula: Option<(usize, String)> = None;
    loop {
        let event = match xml.read_event_into(&mut buf) {
            Ok(event) => event,
            Err(_) => return formula.map_or(Ok(()), |(_, text)| check_formula(&text)),
        };
        let opened = match &event {
            Event::Start(e) | Event::Empty(e) => Some(e.local_name().as_ref().to_string()),
            _ => None,
        };
        if let Some(name) = &opened {
            count.elements += 1;
            if count.elements > MAX_ELEMENTS {
                return Err(format!("it has more than {MAX_ELEMENTS} elements"));
            }
            match name.as_str() {
                "c" => count.cells += 1,
                "si" => count.strings += 1,
                _ => {}
            }
            if count.cells > MAX_CELLS || count.strings > MAX_CELLS {
                return Err(format!("a sheet file holds at most {MAX_CELLS} cells"));
            }
            if name == "cfvo"
                && let Event::Start(e) | Event::Empty(e) = &event
            {
                for a in e.attributes() {
                    let a = a.map_err(|e| e.to_string())?;
                    if a.key.local_name().as_ref() == "val" {
                        let v = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map_err(|e| e.to_string())?;
                        check_formula(&v)?;
                    }
                }
            }
        }
        match &event {
            Event::Start(_) => {
                depth += 1;
                if depth > MAX_XML_DEPTH {
                    return Err(format!("its XML nests more than {MAX_XML_DEPTH} levels"));
                }
                if formula.is_none() && opened.as_deref().is_some_and(is_formula) {
                    formula = Some((depth, String::new()));
                }
            }
            Event::End(_) => {
                if let Some((at, text)) = &formula
                    && *at == depth
                {
                    check_formula(text)?;
                    formula = None;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(t) => {
                if let Some((_, text)) = &mut formula {
                    push_capped(text, &t.xml_content(quick_xml::XmlVersion::Implicit1_0));
                }
            }
            Event::CData(t) => {
                if let Some((_, text)) = &mut formula {
                    push_capped(text, &t.xml_content(quick_xml::XmlVersion::Implicit1_0));
                }
            }
            Event::GeneralRef(r) => {
                if let Some((_, text)) = &mut formula {
                    let ch = match r.resolve_char_ref().map_err(|e| e.to_string())? {
                        Some(c) => c.to_string(),
                        None => quick_xml::escape::resolve_predefined_entity(r).unwrap_or("").to_string(),
                    };
                    push_capped(text, &ch);
                }
            }
            Event::Eof => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

/// Adds to a formula's text, stopping a little past the longest formula (enough for [`check_formula`] to refuse it).
fn push_capped(text: &mut String, more: &str) {
    if text.len() <= MAX_FORMULA_CHARS * 4 {
        text.push_str(more);
    }
}
