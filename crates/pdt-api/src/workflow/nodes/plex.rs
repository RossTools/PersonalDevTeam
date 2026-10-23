//! Plex (Phase 7e spec §7.1): `plex.search`, `plex.recently_added` and `plex.refresh`. A connection of kind `plex`:
//! `url` the server's address (`http://nas:32400`), the secret its `X-Plex-Token`.

use super::http::{call_json, service};
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::api::Connection;
use reqwest::Method;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const PLEX: &str = "plex";

const DEFAULT_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PlexSearchConfig {
    /// A `plex` connection.
    #[schemars(extend("x-connection-kind" = "plex"))]
    pub connection: String,
    pub query: String,
    /// At most this many results (default 20).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PlexSectionConfig {
    /// A `plex` connection.
    #[schemars(extend("x-connection-kind" = "plex"))]
    pub connection: String,
    /// A library section's name (`Films`) or key; default every section.
    #[serde(default)]
    pub section: Option<String>,
    /// At most this many items (`plex.recently_added`; default 20).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlexItem {
    pub title: String,
    /// `movie`, `show`, `season`, `episode`, `artist`, `album`, `track`, …
    #[serde(rename = "type")]
    pub kind: String,
    pub year: Option<u64>,
    pub rating_key: String,
    /// The library section it is in.
    pub section: Option<String>,
    /// An episode's show, a track's artist.
    pub show: Option<String>,
    /// When it was added (Unix seconds).
    #[ts(type = "number | null")]
    pub added_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlexItems {
    pub items: Vec<PlexItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlexRefreshed {
    /// The section key refreshed, or `all`.
    pub section: String,
}

/// An item from Plex's `Metadata` JSON.
pub fn plex_item(v: &Value) -> PlexItem {
    let s = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(String::from);
    PlexItem {
        title: s("title").unwrap_or_default(),
        kind: s("type").unwrap_or_default(),
        year: v["year"].as_u64(),
        rating_key: s("ratingKey").or_else(|| v["ratingKey"].as_u64().map(|n| n.to_string())).unwrap_or_default(),
        section: s("librarySectionTitle"),
        show: s("grandparentTitle"),
        added_at: v["addedAt"].as_i64(),
    }
}

/// Every item in a `/hub/search` answer, hub by hub.
pub fn hub_items(v: &Value) -> Vec<PlexItem> {
    let hubs = v["MediaContainer"]["Hub"].as_array().cloned().unwrap_or_default();
    hubs.iter().flat_map(|h| h["Metadata"].as_array().cloned().unwrap_or_default()).map(|m| plex_item(&m)).collect()
}

/// The key of the section `wanted` names (a key, or a title in `/library/sections`).
pub fn section_key(sections: &Value, wanted: &str) -> Option<String> {
    let dirs = sections["MediaContainer"]["Directory"].as_array()?;
    let key = |d: &Value| d["key"].as_str().map(String::from).or_else(|| d["key"].as_u64().map(|n| n.to_string()));
    dirs.iter()
        .find(|d| key(d).as_deref() == Some(wanted))
        .or_else(|| dirs.iter().find(|d| d["title"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(wanted))))
        .and_then(key)
}

struct Client {
    conn: Connection,
    token: String,
}

impl Client {
    async fn open(cx: &StepCx<'_>, name: &str) -> Result<Self, String> {
        let (conn, token) = service(cx.st, name, PLEX).await?;
        Ok(Self { conn, token })
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, String> {
        let who = format!("Plex ({})", self.conn.name);
        call_json(&who, Method::GET, &self.conn.url, path, &[("X-Plex-Token", &self.token)], query, None).await
    }

    /// The key of `section` (None: every section).
    async fn section(&self, section: Option<&str>) -> Result<Option<String>, String> {
        let Some(wanted) = section.map(str::trim).filter(|s| !s.is_empty()) else { return Ok(None) };
        let sections = self.get("/library/sections", &[]).await?;
        section_key(&sections, wanted)
            .map(Some)
            .ok_or_else(|| format!("{} has no library section {wanted:?}", self.conn.name))
    }
}

fn would(cx: &StepCx<'_>, what: String) -> Option<Outcome> {
    cx.dry_run.then(|| Outcome::next("out", json!({"would": what})))
}

pub struct Search;

impl NodeKind for Search {
    fn spec(&self) -> KindSpec {
        spec::<PlexSearchConfig, PlexItems>(
            "plex.search",
            1,
            Category::Action,
            "Search Plex",
            "Searches a Plex server's libraries: `{{steps.<id>.items}}` is a list of {title, type, year, ratingKey}.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: PlexSearchConfig = parse(config)?;
        if c.query.trim().is_empty() {
            return Err("query is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PlexSearchConfig = parse(&input)?;
            if let Some(o) = would(cx, format!("search Plex ({}) for {:?}", c.connection, c.query)) {
                return Ok(o);
            }
            let client = Client::open(cx, &c.connection).await?;
            let limit = c.limit.unwrap_or(DEFAULT_LIMIT);
            let v = client
                .get("/hub/search", &[("query", c.query.trim().to_string()), ("limit", limit.to_string())])
                .await?;
            let mut items = hub_items(&v);
            items.truncate(limit);
            Ok(Outcome::next("out", PlexItems { items }))
        })
    }
}

pub struct RecentlyAdded;

impl NodeKind for RecentlyAdded {
    fn spec(&self) -> KindSpec {
        spec::<PlexSectionConfig, PlexItems>(
            "plex.recently_added",
            1,
            Category::Action,
            "Recently added to Plex",
            "The newest items in a Plex library section (or every section): `{{steps.<id>.items}}`.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<PlexSectionConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PlexSectionConfig = parse(&input)?;
            if let Some(o) = would(cx, format!("read what was added to Plex ({})", c.connection)) {
                return Ok(o);
            }
            let client = Client::open(cx, &c.connection).await?;
            let path = match client.section(c.section.as_deref()).await? {
                Some(key) => format!("/library/sections/{key}/recentlyAdded"),
                None => "/library/recentlyAdded".to_string(),
            };
            let limit = c.limit.unwrap_or(DEFAULT_LIMIT);
            let query = [("X-Plex-Container-Start", "0".to_string()), ("X-Plex-Container-Size", limit.to_string())];
            let v = client.get(&path, &query).await?;
            let mut items: Vec<PlexItem> = v["MediaContainer"]["Metadata"]
                .as_array()
                .map(|l| l.iter().map(plex_item).collect())
                .unwrap_or_default();
            items.truncate(limit);
            Ok(Outcome::next("out", PlexItems { items }))
        })
    }
}

pub struct Refresh;

impl NodeKind for Refresh {
    fn spec(&self) -> KindSpec {
        spec::<PlexSectionConfig, PlexRefreshed>(
            "plex.refresh",
            1,
            Category::Action,
            "Refresh Plex",
            "Asks Plex to scan a library section (or every section) for new files.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<PlexSectionConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PlexSectionConfig = parse(&input)?;
            let which = c.section.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "every section".into());
            if let Some(o) = would(cx, format!("refresh {which} in Plex ({})", c.connection)) {
                return Ok(o);
            }
            let client = Client::open(cx, &c.connection).await?;
            let key = client.section(c.section.as_deref()).await?.unwrap_or_else(|| "all".into());
            client.get(&format!("/library/sections/{key}/refresh"), &[]).await?;
            Ok(Outcome::next("out", PlexRefreshed { section: key }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_answers_flatten_their_hubs() {
        let v = json!({"MediaContainer": {"Hub": [
            {"type": "movie", "Metadata": [{"title": "Dune", "type": "movie", "year": 2021, "ratingKey": "12",
                "librarySectionTitle": "Films", "addedAt": 1700000000}]},
            {"type": "episode", "Metadata": [{"title": "Pilot", "type": "episode", "ratingKey": 99,
                "grandparentTitle": "Severance"}]},
            {"type": "artist"}
        ]}});
        let items = hub_items(&v);
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].title.as_str(), items[0].rating_key.as_str()), ("Dune", "12"));
        assert_eq!((items[0].section.as_deref(), items[0].added_at), (Some("Films"), Some(1700000000)));
        assert_eq!((items[1].rating_key.as_str(), items[1].show.as_deref()), ("99", Some("Severance")));
        assert!(hub_items(&json!({})).is_empty());
    }

    #[test]
    fn sections_are_found_by_key_or_title() {
        let v = json!({"MediaContainer": {"Directory": [{"key": "1", "title": "Films"}, {"key": "2", "title": "TV"}]}});
        assert_eq!(section_key(&v, "2").as_deref(), Some("2"));
        assert_eq!(section_key(&v, "films").as_deref(), Some("1"));
        assert_eq!(section_key(&v, "Music"), None);
    }
}
