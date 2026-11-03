//! `brief.archive` and `brief.read`: a brief's output kept as files, `<data dir>/briefs/<brief>/`
//! with one Markdown file per day and an `index.jsonl` of what each entry covered, so a brief can look back and not
//! repeat itself. Files, not tables: readable and greppable outside PDT.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use ts_rs::TS;

/// How many entries `brief.read` returns unless told.
pub const DEFAULT_LAST: usize = 7;

/// The most Markdown `brief.read` returns, over all days.
const MARKDOWN_CAP: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BriefItem {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// A line about it (archived in the Markdown only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct BriefArchiveConfig {
    /// The brief's name (`morning`, `film-news`): lowercase letters, digits, `-` and `_`.
    pub brief: String,
    #[serde(default)]
    pub title: String,
    /// The entry's text (Markdown).
    #[serde(default)]
    #[schemars(extend("x-format" = "markdown"))]
    pub body: String,
    /// What it covered: `[{title, link, note}]` (or a reference to such a list).
    #[serde(default)]
    #[schemars(with = "Vec<BriefItem>")]
    #[ts(type = "Array<{ title: string, link?: string, note?: string }> | string")]
    pub items: Value,
    /// `YYYY-MM-DD`; default today in the host's time zone.
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct BriefReadConfig {
    pub brief: String,
    /// Entries on or after this date (`YYYY-MM-DD`).
    #[serde(default)]
    pub since: Option<String>,
    /// At most this many of the newest entries (default 7 when `since` is not given).
    #[serde(default)]
    #[schemars(with = "Option<u32>")]
    #[ts(type = "number | string | null")]
    pub last: Option<Value>,
}

/// One line of a brief's `index.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BriefEntry {
    pub date: String,
    /// When it was archived (ms).
    #[ts(type = "number")]
    pub at: i64,
    pub title: String,
    pub items: Vec<BriefItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BriefDay {
    pub date: String,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BriefArchiveOutput {
    pub date: String,
    /// The day's Markdown file.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BriefReadOutput {
    /// Oldest first.
    pub entries: Vec<BriefEntry>,
    /// The Markdown of each day those entries are on, oldest first.
    pub days: Vec<BriefDay>,
}

fn has_ref(s: &str) -> bool {
    s.contains("{{")
}

/// Whether `name` is a brief name (so it can't leave the briefs folder).
pub fn valid_brief(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok { Ok(()) } else { Err(format!("{name:?} is not a brief name (lowercase letters, digits, - and _)")) }
}

/// Whether `date` is `YYYY-MM-DD`.
pub fn valid_date(date: &str) -> Result<(), String> {
    date.parse::<jiff::civil::Date>()
        .ok()
        .filter(|d| d.to_string() == date)
        .map(|_| ())
        .ok_or_else(|| format!("{date:?} is not a date like 2026-10-22"))
}

pub fn brief_dir(data_dir: &Path, brief: &str) -> PathBuf {
    data_dir.join("briefs").join(brief)
}

/// The items a resolved `items` value holds.
pub fn items_of(v: &Value) -> Result<Vec<BriefItem>, String> {
    match v {
        Value::Null => Ok(vec![]),
        Value::Array(_) => serde_json::from_value(v.clone()).map_err(|e| format!("items: {e}")),
        other => Err(format!("items is not a list of {{title, link, note}}: {other}")),
    }
}

/// One entry as Markdown: a heading (the title, else the time), the body, then the items as a list.
pub fn entry_markdown(title: &str, time: &str, body: &str, items: &[BriefItem]) -> String {
    let heading = if title.trim().is_empty() { time.to_string() } else { title.trim().to_string() };
    let mut out = format!("## {heading}\n\n");
    if !body.trim().is_empty() {
        out.push_str(body.trim());
        out.push_str("\n\n");
    }
    for i in items {
        let name = match i.link.as_deref().filter(|l| !l.is_empty()) {
            Some(l) => format!("[{}]({l})", i.title),
            None => i.title.clone(),
        };
        match i.note.as_deref().filter(|n| !n.trim().is_empty()) {
            Some(n) => out.push_str(&format!("- {name} — {}\n", n.trim())),
            None => out.push_str(&format!("- {name}\n")),
        }
    }
    if !items.is_empty() {
        out.push('\n');
    }
    out
}

/// The entries of an `index.jsonl` (lines that don't parse are skipped) that `since` and `last` pick.
pub fn pick_entries(index: &str, since: Option<&str>, last: Option<usize>) -> Vec<BriefEntry> {
    let mut all: Vec<BriefEntry> = index.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    if let Some(s) = since {
        all.retain(|e| e.date.as_str() >= s);
    }
    let last = last.or(if since.is_none() { Some(DEFAULT_LAST) } else { None });
    if let Some(n) = last {
        let skip = all.len().saturating_sub(n);
        all.drain(..skip);
    }
    all
}

/// Appends to a brief's files one writer at a time, so two runs never interleave a line.
fn write_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    &LOCK
}

async fn append(path: &Path, text: &str) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(text.as_bytes()).await.map_err(|e| format!("{}: {e}", path.display()))?;
    f.flush().await.map_err(|e| e.to_string())
}

pub struct Archive;

impl NodeKind for Archive {
    fn spec(&self) -> KindSpec {
        spec::<BriefArchiveConfig, BriefArchiveOutput>(
            "brief.archive",
            1,
            Category::Action,
            "Archive a brief",
            "Appends an entry (title, Markdown body, the items it covered) to a brief's archive: a Markdown file per day \
             under <data dir>/briefs/<brief>/, with an index brief.read reads back.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: BriefArchiveConfig = parse(config)?;
        if !has_ref(&c.brief) {
            valid_brief(&c.brief)?;
        }
        if let Some(d) = c.date.as_deref().filter(|d| !d.is_empty() && !has_ref(d)) {
            valid_date(d)?;
        }
        if !c.items.as_str().is_some_and(has_ref) {
            items_of(&c.items)?;
        }
        if c.title.trim().is_empty() && c.body.trim().is_empty() && c.items.as_array().is_none_or(|a| a.is_empty()) {
            return Err("give a title, a body or items".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: BriefArchiveConfig = parse(&input)?;
            valid_brief(&c.brief)?;
            let now = jiff::Zoned::now();
            let date = match c.date.as_deref().filter(|d| !d.is_empty()) {
                Some(d) => {
                    valid_date(d)?;
                    d.to_string()
                }
                None => now.date().to_string(),
            };
            let items = items_of(&c.items)?;
            let dir = brief_dir(&cx.st.data_dir, &c.brief);
            let path = dir.join(format!("{date}.md"));
            if cx.dry_run {
                let what = format!("archive {:?} ({} items) to {}", c.title, items.len(), path.display());
                return Ok(Outcome::next("out", json!({"would": what})));
            }
            let _guard = write_lock().lock().await;
            tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("{}: {e}", dir.display()))?;
            let mut md = String::new();
            if tokio::fs::metadata(&path).await.is_err() {
                md.push_str(&format!("# {} — {date}\n\n", c.brief));
            }
            md.push_str(&entry_markdown(&c.title, &now.strftime("%H:%M").to_string(), &c.body, &items));
            append(&path, &md).await?;
            let entry = BriefEntry {
                date: date.clone(),
                at: now.timestamp().as_millisecond(),
                title: c.title.trim().to_string(),
                items: items.into_iter().map(|i| BriefItem { note: None, ..i }).collect(),
            };
            let line = serde_json::to_string(&entry).map_err(|e| e.to_string())? + "\n";
            append(&dir.join("index.jsonl"), &line).await?;
            Ok(Outcome::next("out", BriefArchiveOutput { date, path: path.to_string_lossy().into_owned() }))
        })
    }
}

/// `last` as a count: a number, or a string holding one.
fn count(v: &Option<Value>) -> Result<Option<usize>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => {
            n.as_u64().map(|n| Some(n as usize)).ok_or_else(|| format!("last {n} is not a count"))
        }
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => s.trim().parse().map(Some).map_err(|_| format!("last {s:?} is not a count")),
        Some(other) => Err(format!("last {other} is not a count")),
    }
}

pub struct Read;

impl NodeKind for Read {
    fn spec(&self) -> KindSpec {
        spec::<BriefReadConfig, BriefReadOutput>(
            "brief.read",
            1,
            Category::Action,
            "Read a brief's archive",
            "Reads a brief's recent entries back (`last`, default 7, or `since` a date): what each covered, and the \
             Markdown of their days, so a brief can avoid repeating itself.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: BriefReadConfig = parse(config)?;
        if !has_ref(&c.brief) {
            valid_brief(&c.brief)?;
        }
        if let Some(d) = c.since.as_deref().filter(|d| !d.is_empty() && !has_ref(d)) {
            valid_date(d)?;
        }
        if !c.last.as_ref().and_then(|v| v.as_str()).is_some_and(has_ref) {
            count(&c.last)?;
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: BriefReadConfig = parse(&input)?;
            valid_brief(&c.brief)?;
            let since = c.since.filter(|s| !s.is_empty());
            if let Some(s) = &since {
                valid_date(s)?;
            }
            let dir = brief_dir(&cx.st.data_dir, &c.brief);
            let index = tokio::fs::read_to_string(dir.join("index.jsonl")).await.unwrap_or_default();
            let entries = pick_entries(&index, since.as_deref(), count(&c.last)?);
            let mut dates: Vec<&str> = entries.iter().map(|e| e.date.as_str()).collect();
            dates.sort();
            dates.dedup();
            let mut days = vec![];
            let mut room = MARKDOWN_CAP;
            for date in dates {
                let mut markdown = tokio::fs::read_to_string(dir.join(format!("{date}.md"))).await.unwrap_or_default();
                if markdown.len() > room {
                    let cut = (0..=room).rev().find(|&i| markdown.is_char_boundary(i)).unwrap_or(0);
                    markdown.truncate(cut);
                }
                room -= markdown.len();
                days.push(BriefDay { date: date.to_string(), markdown });
            }
            Ok(Outcome::next("out", BriefReadOutput { entries, days }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_dates_stay_in_the_folder() {
        assert!(valid_brief("film-news_2").is_ok());
        for bad in ["", "../x", "a/b", "Morning", "a b", "."] {
            assert!(valid_brief(bad).is_err(), "{bad}");
        }
        assert!(valid_date("2026-10-22").is_ok());
        for bad in ["2026-13-01", "22/10/2026", "2026-1-2", "../2026-10-22"] {
            assert!(valid_date(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_entry_is_a_heading_the_body_and_its_items() {
        let items = vec![
            BriefItem { title: "Dune 3".into(), link: Some("https://x/d".into()), note: Some("trailer".into()) },
            BriefItem { title: "No link".into(), link: None, note: None },
        ];
        assert_eq!(
            entry_markdown("Films", "07:00", "Two things.\n", &items),
            "## Films\n\nTwo things.\n\n- [Dune 3](https://x/d) — trailer\n- No link\n\n"
        );
        assert_eq!(entry_markdown(" ", "07:00", "", &[]), "## 07:00\n\n");
    }

    #[test]
    fn reading_picks_the_newest_or_since_a_date() {
        let line = |d: &str, t: &str| format!("{{\"date\":\"{d}\",\"at\":1,\"title\":\"{t}\",\"items\":[]}}\n");
        let index = (1..=9).map(|i| line(&format!("2026-10-0{i}"), &format!("t{i}"))).collect::<String>() + "junk\n";
        let titles = |v: Vec<BriefEntry>| v.into_iter().map(|e| e.title).collect::<Vec<_>>();
        assert_eq!(titles(pick_entries(&index, None, None)), ["t3", "t4", "t5", "t6", "t7", "t8", "t9"]);
        assert_eq!(titles(pick_entries(&index, None, Some(2))), ["t8", "t9"]);
        assert_eq!(titles(pick_entries(&index, Some("2026-10-05"), None)), ["t5", "t6", "t7", "t8", "t9"]);
        assert_eq!(titles(pick_entries(&index, Some("2026-10-05"), Some(1))), ["t9"]);
        assert!(pick_entries("", None, None).is_empty());
    }

    #[test]
    fn items_must_be_a_list() {
        assert!(items_of(&Value::Null).unwrap().is_empty());
        assert_eq!(items_of(&json!([{"title": "a"}])).unwrap()[0].title, "a");
        assert!(items_of(&json!("x")).is_err());
        assert!(items_of(&json!([{"link": "no title"}])).is_err());
    }
}
