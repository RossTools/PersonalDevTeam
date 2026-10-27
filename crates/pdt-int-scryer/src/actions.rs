//! Scryer (PDT-147), a media manager for films, series and anime, through its GraphQL API: the actions (PDT-172)
//! library, search, queue and add, each a node kind (`scryer.search`, …) and a tool (`scryer_search`, …). A connection of kind `scryer`: `url` Scryer's address
//! (`http://nas:8080`, with its base path if it has one), the secret an API key (Scryer: Settings → API keys), sent
//! as a bearer token to `<url>/graphql`; `config.pmActs` lets the PM add without asking. Like Radarr and Sonarr
//! (pdt-int-arr), an add is by id and checks the library first. The whole API (any query or mutation,
//! checked against Scryer's schema) is in [`graphql`] (PDT-150).

use crate::core::{Facet, Ids, SCRYER, monitor_type};
use crate::graphql;
use pdt_api::sdk::AppState;
use pdt_api::sdk::http::{call_json_capped, service};
use pdt_api::sdk::{Action, ActionCx, AnyAction, ConnectionFace, NodeFace, ToolFace, Via};
use pdt_api::sdk::{BoxFut, schema_of};
use pdt_core::integration::ToolDecl;
use pdt_core::workflow::api::Connection;
use reqwest::Method;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use ts_rs::TS;

/// How many search results are kept unless told.
const SEARCH_LIMIT: usize = 10;
/// How many library titles a page has unless told (Scryer allows 300).
const LIBRARY_LIMIT: usize = 50;
/// How many queue items are read unless told (Scryer allows 200).
const QUEUE_LIMIT: usize = 50;

/// The GraphQL operations PDT sends, each checked against Scryer's published schema by a test (`scryer/schema.graphql`).
mod ops {
    const TITLE_FIELDS: &str = "id name year facet monitored libraryName contentStatus episodesOwned episodesTotal \
                                sizeBytes externalIds { source value }";
    const RESULT_FIELDS: &str = "tvdbId tmdbId imdbId smgId name year overview";

    pub fn titles_by_ids() -> String {
        format!(
            "query PdtTitlesByIds($source: String!, $values: [String!]!) {{ \
             titlesByExternalIds(source: $source, values: $values) {{ {TITLE_FIELDS} }} }}"
        )
    }

    pub fn libraries() -> String {
        "query PdtLibraries($facet: MediaFacetValue) { libraries(facet: $facet) { id name slug } }".into()
    }

    pub fn search() -> String {
        format!(
            "query PdtSearch($query: String!, $type: MediaFacetValue!, $limit: Int!) {{ \
             searchMetadata(query: $query, type: $type, limit: $limit) {{ {RESULT_FIELDS} }} }}"
        )
    }

    pub fn search_all() -> String {
        format!(
            "query PdtSearchAll($query: String!, $limit: Int!) {{ searchMetadataMulti(query: $query, limit: \
             $limit) {{ movies {{ {RESULT_FIELDS} }} series {{ {RESULT_FIELDS} }} anime {{ {RESULT_FIELDS} }} }} }}"
        )
    }

    pub fn movie() -> String {
        "query PdtMovie($input: MetadataMovieInput!) { metadataMovie(input: $input) { tvdbId tmdbId smgId name year } }"
            .into()
    }

    /// A series' metadata has no IMDb id: an add keeps the caller's.
    pub fn series() -> String {
        "query PdtSeries($input: MetadataSeriesInput!) { metadataSeries(input: $input) { tvdbId tmdbId smgId name \
         year } }"
            .into()
    }

    pub fn add_title() -> String {
        format!(
            "mutation PdtAddTitle($input: AddTitleInput!) {{ addTitle(input: $input) {{ title {{ {TITLE_FIELDS} }} \
             reusedExistingTitle }} }}"
        )
    }

    pub fn queue() -> String {
        "query PdtQueue($limit: Int!) { downloadQueuePage(limit: $limit) { totalCount items { titleName \
         progressPercent displayState remainingSeconds clientName attentionRequired attentionReason } } }"
            .into()
    }

    pub fn library() -> String {
        format!(
            "query PdtLibrary($facet: MediaFacetValue, $query: String, $filter: TitleCatalogFilterInput, $limit: \
             Int, $offset: Int) {{ titles(facet: $facet, query: $query, filter: $filter, limit: $limit, offset: \
             $offset) {{ totalCount hasMore items {{ {TITLE_FIELDS} }} }} }}"
        )
    }

    /// Every operation, for the schema test.
    #[cfg(test)]
    pub fn all() -> Vec<String> {
        vec![titles_by_ids(), libraries(), search(), search_all(), movie(), series(), add_title(), queue(), library()]
    }
}

// ---- configs and outputs ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ScryerSearchConfig {
    /// A `scryer` connection.
    #[schemars(extend("x-connection-kind" = "scryer"))]
    pub connection: String,
    /// A title to look up (`severance`).
    pub term: String,
    /// `movie`, `series` or `anime`; every facet when left out.
    #[serde(default)]
    pub facet: Option<String>,
    /// At most this many results per facet (default 10).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// A title a search found, to add by its ids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScryerItem {
    /// `movie`, `series` or `anime`.
    pub facet: String,
    pub name: String,
    pub year: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tvdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tmdb_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub imdb_id: Option<String>,
    /// Already in the library.
    pub in_library: bool,
    /// Its title's id in the library, when it is there.
    pub title_id: Option<String>,
    #[serde(default)]
    pub overview: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScryerSearchOutput {
    pub results: Vec<ScryerItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ScryerAddConfig {
    /// A `scryer` connection.
    #[schemars(extend("x-connection-kind" = "scryer"))]
    pub connection: String,
    /// `movie`, `series` or `anime`.
    pub facet: String,
    /// The TVDB id (from `scryer.search`); at least one id is needed.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    #[ts(type = "number | string | null")]
    pub tvdb_id: Option<Value>,
    /// The TMDB id (films are mostly known by it).
    #[serde(default)]
    #[schemars(with = "Option<u64>")]
    #[ts(type = "number | string | null")]
    pub tmdb_id: Option<Value>,
    /// The IMDb id (`tt…`).
    #[serde(default)]
    pub imdb_id: Option<String>,
    /// Its name, shown to the user before an add (Scryer's metadata names it).
    #[serde(default)]
    pub name: Option<String>,
    /// A library's name, slug or id; Scryer's default library for the facet when left out.
    #[serde(default)]
    pub library: Option<String>,
    /// Monitor it, so Scryer finds and downloads it (default true).
    #[serde(default = "yes")]
    pub monitored: bool,
    /// A series' or anime's episodes to monitor: `all` (default when monitored), `future`, `missingAndFuture` or
    /// `none`.
    #[serde(default)]
    pub monitor: Option<String>,
}

fn yes() -> bool {
    true
}

/// A title in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScryerTitle {
    pub title_id: String,
    /// `movie`, `series` or `anime`.
    pub facet: String,
    pub name: String,
    pub year: Option<i64>,
    pub monitored: bool,
    /// Its library's name.
    pub library: Option<String>,
    /// `continuing`, `ended`, `released`, … as Scryer says.
    pub status: Option<String>,
    /// A series' episodes on disk and in all (specials left out).
    pub episodes_owned: Option<i64>,
    pub episodes_total: Option<i64>,
    /// Its files' size in bytes.
    #[ts(type = "number | null")]
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ScryerQueueConfig {
    /// A `scryer` connection.
    #[schemars(extend("x-connection-kind" = "scryer"))]
    pub connection: String,
    /// At most this many items (default 50).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScryerQueueItem {
    /// The title (or the release, when Scryer matched none).
    pub title: String,
    /// Percent done.
    pub progress: i64,
    /// `downloading`, `queued`, `paused`, `importing`, `failed`, …
    pub status: String,
    /// Time left in seconds, if known.
    pub seconds_left: Option<i64>,
    /// The download client.
    pub client: String,
    /// Why it needs the user, when it does.
    pub attention: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScryerQueueOutput {
    pub items: Vec<ScryerQueueItem>,
    /// Items in the queue, also those past `limit`.
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ScryerLibraryConfig {
    /// A `scryer` connection.
    #[schemars(extend("x-connection-kind" = "scryer"))]
    pub connection: String,
    /// `movie`, `series` or `anime`; every facet when left out.
    #[serde(default)]
    pub facet: Option<String>,
    /// Words in the title.
    #[serde(default)]
    pub query: Option<String>,
    /// Only monitored (true) or unmonitored (false) titles.
    #[serde(default)]
    pub monitored: Option<bool>,
    /// At most this many titles (default 50, at most 300).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Titles to skip, for the next page.
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScryerLibraryOutput {
    pub titles: Vec<ScryerTitle>,
    /// Titles that match, on every page.
    pub total: i64,
    pub has_more: bool,
}

// ---- the client ----

/// Scryer behind its connection.
pub(crate) struct Client {
    pub(crate) conn: Connection,
    pub(crate) key: String,
}

impl Client {
    pub async fn open(st: &AppState, name: &str) -> Result<Self, String> {
        let (conn, key) = service(st, name, SCRYER).await?;
        Ok(Self { conn, key: key.trim().to_string() })
    }

    /// One GraphQL operation; its `data`, or Scryer's errors.
    async fn gql(&self, query: &str, variables: Value) -> Result<Value, String> {
        self.gql_named(query, variables, None).await
    }

    /// One operation of a document (by name, when it has several); its `data`, or Scryer's errors.
    pub(crate) async fn gql_named(
        &self,
        query: &str,
        variables: Value,
        operation: Option<&str>,
    ) -> Result<Value, String> {
        self.gql_capped(query, variables, operation, pdt_api::sdk::http::BODY_CAP).await
    }

    /// [`Client::gql_named`] reading at most `cap` bytes of the answer.
    pub(crate) async fn gql_capped(
        &self,
        query: &str,
        variables: Value,
        operation: Option<&str>,
        cap: usize,
    ) -> Result<Value, String> {
        let who = format!("Scryer ({})", self.conn.name);
        let auth = format!("Bearer {}", self.key);
        let mut body = json!({"query": query, "variables": variables});
        if let Some(o) = operation {
            body["operationName"] = json!(o);
        }
        let headers = [("Authorization", auth.as_str())];
        let v = call_json_capped(&who, Method::POST, &self.conn.url, "/graphql", &headers, &[], Some(&body), cap)
            .await
            .map_err(|e| if e.contains(" answered 401 ") { format!("{e} (the API key is wrong)") } else { e })?;
        if let Some(errors) = v["errors"].as_array().filter(|e| !e.is_empty()) {
            let all: Vec<&str> = errors.iter().filter_map(|e| e["message"].as_str()).collect();
            return Err(format!("{who}: {}", all.join("; ")));
        }
        Ok(v["data"].clone())
    }

    /// Titles in the library that share an id with any of `ids`.
    async fn titles_with(&self, ids: &[&Ids]) -> Result<Vec<Value>, String> {
        let mut out: Vec<Value> = vec![];
        for source in ["tvdb", "tmdb", "imdb"] {
            let mut values: Vec<String> =
                ids.iter().flat_map(|i| i.pairs()).filter(|(s, _)| *s == source).map(|(_, v)| v).collect();
            values.sort();
            values.dedup();
            if values.is_empty() {
                continue;
            }
            let d = self.gql(&ops::titles_by_ids(), json!({"source": source, "values": values})).await?;
            for t in d["titlesByExternalIds"].as_array().into_iter().flatten() {
                if !out.iter().any(|o| o["id"] == t["id"]) {
                    out.push(t.clone());
                }
            }
        }
        Ok(out)
    }

    /// A library's id by its name, slug or id, among the facet's.
    async fn library_id(&self, facet: Facet, given: &str) -> Result<String, String> {
        let d = self.gql(&ops::libraries(), json!({"facet": facet.gql()})).await?;
        let all = d["libraries"].as_array().cloned().unwrap_or_default();
        let g = given.trim();
        let found = all
            .iter()
            .find(|l| ["id", "name", "slug"].iter().any(|k| l[*k].as_str().is_some_and(|v| v.eq_ignore_ascii_case(g))));
        match found.and_then(|l| l["id"].as_str()) {
            Some(id) => Ok(id.to_string()),
            None => Err(format!(
                "{} has no {} library {g:?} (libraries: {})",
                self.conn.name,
                facet.as_str(),
                all.iter().filter_map(|l| l["name"].as_str()).collect::<Vec<_>>().join(", ")
            )),
        }
    }
}

/// A library title from Scryer's `TitlePayload`.
pub fn title(v: &Value) -> ScryerTitle {
    let s = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(String::from);
    ScryerTitle {
        title_id: s("id").unwrap_or_default(),
        facet: Facet::from_gql(&v["facet"]).map(|f| f.as_str().to_string()).unwrap_or_default(),
        name: s("name").unwrap_or_default(),
        year: v["year"].as_i64(),
        monitored: v["monitored"].as_bool().unwrap_or(false),
        library: s("libraryName"),
        status: s("contentStatus"),
        episodes_owned: v["episodesOwned"].as_i64(),
        episodes_total: v["episodesTotal"].as_i64(),
        size_bytes: v["sizeBytes"].as_i64().or_else(|| v["sizeBytes"].as_str().and_then(|s| s.parse().ok())),
    }
}

/// A search result from Scryer's `MetadataSearchItemPayload`, and whether one of `library` is it.
pub fn item(facet: Facet, v: &Value, library: &[Value]) -> ScryerItem {
    let ids = Ids::from_json(v);
    let mine = library.iter().find(|t| Facet::from_gql(&t["facet"]) == Some(facet) && ids.matches(&t["externalIds"]));
    ScryerItem {
        facet: facet.as_str().into(),
        name: v["name"].as_str().unwrap_or_default().into(),
        year: v["year"].as_i64(),
        tvdb_id: ids.tvdb,
        tmdb_id: ids.tmdb,
        imdb_id: ids.imdb,
        in_library: mine.is_some(),
        title_id: mine.and_then(|t| t["id"].as_str()).map(String::from),
        overview: v["overview"].as_str().unwrap_or_default().into(),
    }
}

/// A queue item from Scryer's `DownloadQueueItemPayload`.
pub fn queue_item(v: &Value) -> ScryerQueueItem {
    let s = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(String::from);
    ScryerQueueItem {
        title: s("titleName").unwrap_or_default(),
        progress: v["progressPercent"].as_i64().unwrap_or(0),
        status: s("displayState").map(|d| camel(&d)).unwrap_or_default(),
        seconds_left: v["remainingSeconds"].as_i64(),
        client: s("clientName").unwrap_or_default(),
        attention: if v["attentionRequired"].as_bool() == Some(true) {
            Some(s("attentionReason").unwrap_or_else(|| "needs attention".into()))
        } else {
            None
        },
    }
}

/// `IMPORT_PENDING` → `importPending`.
fn camel(s: &str) -> String {
    let mut out = String::new();
    for (i, w) in s.split('_').filter(|w| !w.is_empty()).enumerate() {
        let w = w.to_ascii_lowercase();
        if i == 0 {
            out.push_str(&w);
        } else {
            let mut c = w.chars();
            if let Some(f) = c.next() {
                out.push(f.to_ascii_uppercase());
                out.push_str(c.as_str());
            }
        }
    }
    out
}

fn facet_of(given: Option<&str>) -> Result<Option<Facet>, String> {
    given.map(str::trim).filter(|f| !f.is_empty()).map(Facet::parse).transpose()
}

fn is_ref(s: &str) -> bool {
    s.contains("{{")
}

/// An add's ids from its config (numbers or strings).
fn add_ids(c: &ScryerAddConfig) -> Ids {
    let mut v = json!({});
    if let Some(t) = &c.tvdb_id {
        v["tvdbId"] = t.clone();
    }
    if let Some(t) = &c.tmdb_id {
        v["tmdbId"] = t.clone();
    }
    if let Some(i) = &c.imdb_id {
        v["imdbId"] = json!(i);
    }
    Ids::from_json(&v)
}

fn check_add(c: &ScryerAddConfig) -> Result<(), String> {
    if !is_ref(&c.facet) {
        Facet::parse(&c.facet)?;
    }
    let refs = [c.tvdb_id.as_ref().and_then(Value::as_str), c.tmdb_id.as_ref().and_then(Value::as_str)]
        .into_iter()
        .chain([c.imdb_id.as_deref()])
        .flatten()
        .any(is_ref);
    if !refs {
        if let Some(t) = &c.tmdb_id
            && !t.is_null()
            && Ids::from_json(&json!({"tmdbId": t})).tmdb.is_none()
        {
            return Err(format!("tmdbId {t} is not an id"));
        }
        if add_ids(c).is_empty() {
            return Err("give the title's tvdbId, tmdbId or imdbId (from scryer.search)".into());
        }
    }
    if let Some(m) = c.monitor.as_deref().filter(|m| !is_ref(m)) {
        monitor_type(Some(m), true)?;
    }
    Ok(())
}

// ---- what the nodes and tools do ----

/// Looks `term` up in Scryer's metadata (one facet, or all three), each result marked when it is in the library.
pub(crate) async fn search(client: &Client, c: &ScryerSearchConfig) -> Result<ScryerSearchOutput, String> {
    let term = c.term.trim();
    if term.is_empty() {
        return Err("term is empty".into());
    }
    let limit = c.limit.unwrap_or(SEARCH_LIMIT).clamp(1, 100);
    let found: Vec<(Facet, Value)> = match facet_of(c.facet.as_deref())? {
        Some(f) => {
            let d = client.gql(&ops::search(), json!({"query": term, "type": f.gql(), "limit": limit})).await?;
            d["searchMetadata"].as_array().into_iter().flatten().map(|v| (f, v.clone())).collect()
        }
        None => {
            let d = client.gql(&ops::search_all(), json!({"query": term, "limit": limit})).await?;
            let m = &d["searchMetadataMulti"];
            [(Facet::Movie, "movies"), (Facet::Series, "series"), (Facet::Anime, "anime")]
                .into_iter()
                .flat_map(|(f, k)| m[k].as_array().cloned().unwrap_or_default().into_iter().map(move |v| (f, v)))
                .collect()
        }
    };
    let ids: Vec<Ids> = found.iter().map(|(_, v)| Ids::from_json(v)).collect();
    let library = client.titles_with(&ids.iter().collect::<Vec<_>>()).await?;
    Ok(ScryerSearchOutput { results: found.iter().map(|(f, v)| item(*f, v, &library)).collect() })
}

/// Adds a title by its ids, unless the library has it: `(true, it)` when added, `(false, the one there)` when not.
pub(crate) async fn add(client: &Client, c: &ScryerAddConfig) -> Result<(bool, ScryerTitle), String> {
    check_add(c)?;
    let facet = Facet::parse(&c.facet)?;
    let ids = add_ids(c);
    if let Some(t) = client
        .titles_with(&[&ids])
        .await?
        .into_iter()
        .find(|t| Facet::from_gql(&t["facet"]) == Some(facet) && ids.matches(&t["externalIds"]))
    {
        return Ok((false, title(&t)));
    }
    let library = match c.library.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) => Some(client.library_id(facet, l).await?),
        None => None,
    };
    // Scryer's metadata names it and fills in the ids the caller didn't give.
    let mut lookup = json!({});
    if let Some(t) = &ids.tvdb {
        lookup["tvdbId"] = json!(t);
    }
    if let Some(t) = ids.tmdb {
        lookup["tmdbId"] = json!(t);
    }
    if let Some(i) = &ids.imdb {
        lookup["imdbId"] = json!(i);
    }
    let (field, q) = match facet {
        Facet::Movie => ("metadataMovie", ops::movie()),
        Facet::Series | Facet::Anime => {
            lookup["includeEpisodes"] = json!(false);
            ("metadataSeries", ops::series())
        }
    };
    let d = client.gql(&q, json!({"input": lookup})).await?;
    let meta = &d[field];
    let name = meta["name"].as_str().map(str::trim).filter(|n| !n.is_empty());
    let Some(name) = name else {
        return Err(format!("Scryer found no {} with {}", facet.as_str(), ids.describe()));
    };
    let mut all = Ids::from_json(meta);
    all.tvdb = ids.tvdb.clone().or(all.tvdb);
    all.tmdb = ids.tmdb.or(all.tmdb);
    all.imdb = ids.imdb.clone().or(all.imdb);
    // As Scryer's own web adds one.
    let mut options = json!({"monitorType": monitor_type(c.monitor.as_deref(), c.monitored)?});
    match facet {
        Facet::Movie => options = json!({"monitorType": monitor_type(None, c.monitored)?}),
        Facet::Series => options["useSeasonFolders"] = json!(true),
        Facet::Anime => {
            options["useSeasonFolders"] = json!(true);
            options["monitorSpecials"] = json!(false);
            options["interSeasonMovies"] = json!(true);
        }
    }
    let input = json!({
        "name": name, "facet": facet.gql(), "libraryId": library, "monitored": c.monitored, "tags": [],
        "options": options, "externalIds": all.external_ids(facet), "smgId": meta["smgId"],
        "tvdbId": all.tvdb, "tmdbId": all.tmdb, "imdbId": all.imdb,
    });
    let d = client.gql(&ops::add_title(), json!({"input": input})).await?;
    let r = &d["addTitle"];
    Ok((r["reusedExistingTitle"].as_bool() != Some(true), title(&r["title"])))
}

pub(crate) async fn queue(client: &Client, c: &ScryerQueueConfig) -> Result<ScryerQueueOutput, String> {
    let d = client.gql(&ops::queue(), json!({"limit": c.limit.unwrap_or(QUEUE_LIMIT).clamp(1, 200)})).await?;
    let page = &d["downloadQueuePage"];
    let items: Vec<ScryerQueueItem> = page["items"].as_array().into_iter().flatten().map(queue_item).collect();
    Ok(ScryerQueueOutput { total: page["totalCount"].as_i64().unwrap_or(items.len() as i64), items })
}

pub(crate) async fn library(client: &Client, c: &ScryerLibraryConfig) -> Result<ScryerLibraryOutput, String> {
    let facet = facet_of(c.facet.as_deref())?;
    let query = c.query.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let filter = c.monitored.map(|m| json!({"monitored": m}));
    let vars = json!({"facet": facet.map(Facet::gql), "query": query, "filter": filter,
                      "limit": c.limit.unwrap_or(LIBRARY_LIMIT).clamp(1, 300), "offset": c.offset.unwrap_or(0)});
    let d = client.gql(&ops::library(), vars).await?;
    let page = &d["titles"];
    let titles: Vec<ScryerTitle> = page["items"].as_array().into_iter().flatten().map(title).collect();
    Ok(ScryerLibraryOutput {
        total: page["totalCount"].as_i64().unwrap_or(titles.len() as i64),
        has_more: page["hasMore"].as_bool().unwrap_or(false),
        titles,
    })
}

// ---- the actions ----

/// A Scryer action as a pdt-mcp tool.
const fn face(decl: ToolDecl, title: &'static str, description: &'static str) -> ToolFace {
    ToolFace { decl, title, description }
}

const fn node(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    ports: &'static [(&'static str, &'static str)],
) -> Option<NodeFace> {
    Some(NodeFace { id, title, description, ports })
}

/// Scryer's connection: a tool call may leave it out.
pub(crate) const CONNECTION: Option<ConnectionFace> = Some(ConnectionFace { kind: SCRYER, label: "Scryer" });

pub(crate) const THEN: &[(&str, &str)] = &[("out", "Then")];

/// The actions, in `tools/list` order: the four shortcuts, then the schema and any query or mutation.
pub(crate) fn actions() -> Vec<Arc<dyn AnyAction>> {
    vec![
        Arc::new(Library),
        Arc::new(Search),
        Arc::new(Queue),
        Arc::new(Add),
        Arc::new(graphql::SchemaLookup),
        Arc::new(graphql::Query),
        Arc::new(graphql::Mutate),
    ]
}

pub struct Library;

impl Action for Library {
    type Input = ScryerLibraryConfig;
    type Output = ScryerLibraryOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("scryer_library", "Browse the Scryer media library").read_only().open_world().pm(),
            "Browse the Scryer library",
            "Titles in the user's Scryer media library: films, series and anime, by facet, words in the title and \
             whether they are monitored, a page at a time (limit, offset), with each one's library, status and \
             episodes on disk.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "scryer.library",
            "Scryer library",
            "Titles in Scryer's library (a facet, words in the title, monitored or not): `{{steps.<id>.titles}}` is \
             a list of {titleId, facet, name, year, monitored, library, status}.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &ScryerLibraryConfig) -> Result<(), String> {
        if let Some(f) = c.facet.as_deref().filter(|f| !is_ref(f)) {
            facet_of(Some(f))?;
        }
        Ok(())
    }

    fn approval_title(&self, c: &ScryerLibraryConfig) -> String {
        format!("read Scryer's library ({})", c.connection)
    }

    fn run<'a>(
        &'a self,
        cx: &'a ActionCx<'a>,
        c: ScryerLibraryConfig,
    ) -> BoxFut<'a, Result<ScryerLibraryOutput, String>> {
        Box::pin(async move { library(&Client::open(cx.st, &c.connection).await?, &c).await })
    }
}

pub struct Search;

impl Action for Search {
    type Input = ScryerSearchConfig;
    type Output = ScryerSearchOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("scryer_search", "Look up films, series and anime to add to Scryer")
                .read_only()
                .open_world()
                .pm(),
            "Look up titles to add to Scryer",
            "Searches Scryer's metadata for films, series and anime by title (one facet, or all): each result's ids \
             (tvdbId, tmdbId, imdbId) for scryer_add, and whether it is in the library already.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "scryer.search",
            "Search Scryer",
            "Looks up films, series and anime in Scryer's metadata by title: `{{steps.<id>.results}}` is a list of \
             {facet, name, year, tvdbId, tmdbId, imdbId, inLibrary}.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &ScryerSearchConfig) -> Result<(), String> {
        if c.term.trim().is_empty() {
            return Err("term is empty".into());
        }
        if let Some(f) = c.facet.as_deref().filter(|f| !is_ref(f)) {
            facet_of(Some(f))?;
        }
        Ok(())
    }

    fn approval_title(&self, c: &ScryerSearchConfig) -> String {
        format!("search Scryer ({}) for {:?}", c.connection, c.term)
    }

    fn run<'a>(
        &'a self,
        cx: &'a ActionCx<'a>,
        c: ScryerSearchConfig,
    ) -> BoxFut<'a, Result<ScryerSearchOutput, String>> {
        Box::pin(async move { search(&Client::open(cx.st, &c.connection).await?, &c).await })
    }
}

pub struct Queue;

impl Action for Queue {
    type Input = ScryerQueueConfig;
    type Output = ScryerQueueOutput;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("scryer_queue", "See what Scryer is downloading").read_only().open_world().pm(),
            "See Scryer's downloads",
            "What Scryer is downloading: each item's title, progress, status, time left and client, and why one \
             needs the user.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "scryer.queue",
            "Scryer queue",
            "What Scryer is downloading: `{{steps.<id>.items}}` is a list of {title, progress, status, secondsLeft, \
             client}.",
            THEN,
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn approval_title(&self, c: &ScryerQueueConfig) -> String {
        format!("read Scryer's queue ({})", c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: ScryerQueueConfig) -> BoxFut<'a, Result<ScryerQueueOutput, String>> {
        Box::pin(async move { queue(&Client::open(cx.st, &c.connection).await?, &c).await })
    }
}

/// What an add came to: the title, and whether the add made it (false: the library had it).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScryerAdded {
    pub added: bool,
    pub title: ScryerTitle,
}

pub struct Add;

impl Action for Add {
    type Input = ScryerAddConfig;
    type Output = ScryerAdded;

    fn tool(&self) -> ToolFace {
        face(
            ToolDecl::new("scryer_add", "Add a film, series or anime to Scryer").asks().open_world().pm(),
            "Add to Scryer",
            "Adds a film, series or anime to the user's Scryer library by the ids scryer_search gave (facet and at \
             least one id; give name too so the user sees it), monitored unless told, so Scryer finds and downloads \
             it. A title already in the library is left as it is. The user confirms each call unless the Scryer \
             connection lets you act.",
        )
    }

    fn node(&self) -> Option<NodeFace> {
        node(
            "scryer.add",
            "Add to Scryer",
            "Adds a film, series or anime to Scryer by its ids (from scryer.search), monitored unless told; takes \
             `exists` when the library already has it.",
            &[("added", "Added"), ("exists", "Already there")],
        )
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &ScryerAddConfig) -> Result<(), String> {
        check_add(c)
    }

    fn approval_title(&self, c: &ScryerAddConfig) -> String {
        format!("add the {} {} to Scryer ({})", c.facet, add_ids(c).describe(), c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: ScryerAddConfig) -> BoxFut<'a, Result<ScryerAdded, String>> {
        Box::pin(async move {
            let (added, title) = add(&Client::open(cx.st, &c.connection).await?, &c).await?;
            if cx.via == Via::Tool {
                tracing::info!(connection = %c.connection, title = %title.name, added, "the PM added a title to Scryer");
            }
            Ok(ScryerAdded { added, title })
        })
    }

    fn port(&self, out: &ScryerAdded) -> &'static str {
        if out.added { "added" } else { "exists" }
    }

    /// A node's output is the title; whether it was added is the port.
    fn node_output(&self, out: ScryerAdded) -> Value {
        serde_json::to_value(out.title).unwrap_or(Value::Null)
    }

    fn node_output_schema(&self) -> Value {
        schema_of::<ScryerTitle>()
    }

    fn tool_output(&self, out: ScryerAdded) -> Result<Value, String> {
        Ok(json!({"added": out.added, "alreadyInLibrary": !out.added, "title": out.title}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_and_results_read_scryers_answers() {
        let t = json!({"id": "t1", "name": "Severance", "year": 2022, "facet": "SERIES", "monitored": true,
            "libraryName": "TV", "contentStatus": "continuing", "episodesOwned": 9, "episodesTotal": 19,
            "sizeBytes": "1024", "externalIds": [{"source": "tvdb", "value": "371980"}]});
        let title = title(&t);
        assert_eq!(
            (title.facet.as_str(), title.library.as_deref(), title.size_bytes),
            ("series", Some("TV"), Some(1024))
        );
        let r = json!({"name": "Severance", "year": 2022, "tvdbId": "371980", "tmdbId": 95396, "overview": "Work."});
        let i = item(Facet::Series, &r, std::slice::from_ref(&t));
        assert_eq!((i.in_library, i.title_id.as_deref(), i.tmdb_id), (true, Some("t1"), Some(95396)));
        // The same ids as a film is another title.
        assert!(!item(Facet::Movie, &r, &[t]).in_library);
    }

    #[test]
    fn queue_items_get_a_readable_status() {
        let q = queue_item(&json!({"titleName": "Dune", "progressPercent": 40, "displayState": "IMPORT_PENDING",
            "remainingSeconds": 90, "clientName": "sab", "attentionRequired": true}));
        assert_eq!(
            (q.status.as_str(), q.seconds_left, q.attention.as_deref()),
            ("importPending", Some(90), Some("needs attention"))
        );
        assert_eq!(queue_item(&json!({"displayState": "DOWNLOADING"})).status, "downloading");
    }

    #[test]
    fn an_add_needs_a_facet_and_an_id() {
        let c = |v: Value| serde_json::from_value::<ScryerAddConfig>(v).unwrap();
        assert!(check_add(&c(json!({"connection": "m", "facet": "series"}))).unwrap_err().contains("tvdbId"));
        assert!(check_add(&c(json!({"connection": "m", "facet": "music", "tvdbId": 1}))).is_err());
        assert!(
            check_add(&c(json!({"connection": "m", "facet": "movie", "tmdbId": "dune"})))
                .unwrap_err()
                .contains("not an id")
        );
        assert!(check_add(&c(json!({"connection": "m", "facet": "series", "tvdbId": 1, "monitor": "pilot"}))).is_err());
        assert!(
            check_add(&c(json!({"connection": "m", "facet": "{{trigger.f}}", "tmdbId": "{{trigger.id}}"}))).is_ok()
        );
        assert!(check_add(&c(json!({"connection": "m", "facet": "movie", "imdbId": "tt1"}))).is_ok());
    }

    /// Every operation PDT sends is valid against Scryer's published schema (the fake in the API tests answers by
    /// operation name and checks no fields).
    #[test]
    fn every_operation_fits_scryers_schema() {
        use apollo_compiler::ExecutableDocument;
        let schema = graphql::bundled();
        let schema = &schema.schema;
        // The errors, if any.
        let check = |op: &str| match ExecutableDocument::parse_and_validate(schema, op, "op.graphql") {
            Ok(_) => None,
            Err(e) => Some(e.errors.to_string()),
        };
        for op in ops::all() {
            if let Some(e) = check(&op) {
                panic!("{op}\n{e}");
            }
        }
        // The test catches a field Scryer doesn't have (a series' metadata has no imdbId).
        let wrong = "query X($input: MetadataSeriesInput!) { metadataSeries(input: $input) { imdbId } }";
        assert!(check(wrong).is_some_and(|e| e.contains("imdbId")));
    }

    #[test]
    fn tool_schemas_leave_the_connection_optional() {
        let s = AnyAction::info(&Add).input_schema;
        let required: Vec<&str> = s["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(required, ["facet"]);
        assert!(s["properties"]["connection"].get("x-connection-kind").is_none());
    }
}
