//! Scryer (PDT-147), a media manager for films, series and anime, through its GraphQL API: the node kinds
//! `scryer.search`, `scryer.add`, `scryer.queue` and `scryer.library`, and the PM's tools on them (`scryer_search`,
//! `scryer_add`, `scryer_queue`, `scryer_library`). A connection of kind `scryer`: `url` Scryer's address
//! (`http://nas:8080`, with its base path if it has one), the secret an API key (Scryer: Settings → API keys), sent
//! as a bearer token to `<url>/graphql`; `config.pmActs` lets the PM add without asking. Like Radarr and Sonarr
//! (`workflow/nodes/arr.rs`), an add is by id and checks the library first.

use crate::routes::AppState;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use crate::workflow::nodes::http::{call_json, service};
use pdt_core::scryer::{Facet, Ids, SCRYER, monitor_type};
use pdt_core::workflow::api::Connection;
use reqwest::Method;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

/// How many search results are kept unless told.
const SEARCH_LIMIT: usize = 10;
/// How many library titles a page has unless told (Scryer allows 300).
const LIBRARY_LIMIT: usize = 50;
/// How many queue items are read unless told (Scryer allows 200).
const QUEUE_LIMIT: usize = 50;

const TITLE_FIELDS: &str = "id name year facet monitored libraryName contentStatus episodesOwned episodesTotal \
                            sizeBytes externalIds { source value }";
const RESULT_FIELDS: &str = "tvdbId tmdbId imdbId smgId name year overview";

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
    conn: Connection,
    key: String,
}

impl Client {
    pub async fn open(st: &AppState, name: &str) -> Result<Self, String> {
        let (conn, key) = service(st, name, SCRYER).await?;
        Ok(Self { conn, key: key.trim().to_string() })
    }

    /// One GraphQL operation; its `data`, or Scryer's errors.
    async fn gql(&self, query: &str, variables: Value) -> Result<Value, String> {
        let who = format!("Scryer ({})", self.conn.name);
        let auth = format!("Bearer {}", self.key);
        let body = json!({"query": query, "variables": variables});
        let v =
            call_json(&who, Method::POST, &self.conn.url, "/graphql", &[("Authorization", &auth)], &[], Some(&body))
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
            let q = format!(
                "query PdtTitlesByIds($source: String!, $values: [String!]!) {{ \
                 titlesByExternalIds(source: $source, values: $values) {{ {TITLE_FIELDS} }} }}"
            );
            let d = self.gql(&q, json!({"source": source, "values": values})).await?;
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
        let q = "query PdtLibraries($facet: MediaFacetValue) { libraries(facet: $facet) { id name slug } }";
        let d = self.gql(q, json!({"facet": facet.gql()})).await?;
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
            let q = format!(
                "query PdtSearch($query: String!, $type: MediaFacetValue!, $limit: Int!) {{ \
                 searchMetadata(query: $query, type: $type, limit: $limit) {{ {RESULT_FIELDS} }} }}"
            );
            let d = client.gql(&q, json!({"query": term, "type": f.gql(), "limit": limit})).await?;
            d["searchMetadata"].as_array().into_iter().flatten().map(|v| (f, v.clone())).collect()
        }
        None => {
            let q = format!(
                "query PdtSearchAll($query: String!, $limit: Int!) {{ searchMetadataMulti(query: $query, limit: \
                 $limit) {{ movies {{ {RESULT_FIELDS} }} series {{ {RESULT_FIELDS} }} anime {{ {RESULT_FIELDS} }} }} }}"
            );
            let d = client.gql(&q, json!({"query": term, "limit": limit})).await?;
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
        Facet::Movie => (
            "metadataMovie",
            "query PdtMovie($input: MetadataMovieInput!) { metadataMovie(input: $input) { tvdbId tmdbId smgId name \
             year } }",
        ),
        Facet::Series | Facet::Anime => {
            lookup["includeEpisodes"] = json!(false);
            (
                "metadataSeries",
                "query PdtSeries($input: MetadataSeriesInput!) { metadataSeries(input: $input) { tvdbId tmdbId smgId \
                 imdbId name year } }",
            )
        }
    };
    let d = client.gql(q, json!({"input": lookup})).await?;
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
    let q = format!(
        "mutation PdtAddTitle($input: AddTitleInput!) {{ addTitle(input: $input) {{ title {{ {TITLE_FIELDS} }} \
         reusedExistingTitle }} }}"
    );
    let d = client.gql(&q, json!({"input": input})).await?;
    let r = &d["addTitle"];
    Ok((r["reusedExistingTitle"].as_bool() != Some(true), title(&r["title"])))
}

pub(crate) async fn queue(client: &Client, c: &ScryerQueueConfig) -> Result<ScryerQueueOutput, String> {
    let q = "query PdtQueue($limit: Int!) { downloadQueuePage(limit: $limit) { totalCount items { titleName \
             progressPercent displayState remainingSeconds clientName attentionRequired attentionReason } } }";
    let d = client.gql(q, json!({"limit": c.limit.unwrap_or(QUEUE_LIMIT).clamp(1, 200)})).await?;
    let page = &d["downloadQueuePage"];
    let items: Vec<ScryerQueueItem> = page["items"].as_array().into_iter().flatten().map(queue_item).collect();
    Ok(ScryerQueueOutput { total: page["totalCount"].as_i64().unwrap_or(items.len() as i64), items })
}

pub(crate) async fn library(client: &Client, c: &ScryerLibraryConfig) -> Result<ScryerLibraryOutput, String> {
    let facet = facet_of(c.facet.as_deref())?;
    let query = c.query.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let filter = c.monitored.map(|m| json!({"monitored": m}));
    let q = format!(
        "query PdtLibrary($facet: MediaFacetValue, $query: String, $filter: TitleCatalogFilterInput, $limit: Int, \
         $offset: Int) {{ titles(facet: $facet, query: $query, filter: $filter, limit: $limit, offset: $offset) {{ \
         totalCount hasMore items {{ {TITLE_FIELDS} }} }} }}"
    );
    let vars = json!({"facet": facet.map(Facet::gql), "query": query, "filter": filter,
                      "limit": c.limit.unwrap_or(LIBRARY_LIMIT).clamp(1, 300), "offset": c.offset.unwrap_or(0)});
    let d = client.gql(&q, vars).await?;
    let page = &d["titles"];
    let titles: Vec<ScryerTitle> = page["items"].as_array().into_iter().flatten().map(title).collect();
    Ok(ScryerLibraryOutput {
        total: page["totalCount"].as_i64().unwrap_or(titles.len() as i64),
        has_more: page["hasMore"].as_bool().unwrap_or(false),
        titles,
    })
}

// ---- the node kinds ----

fn would(cx: &StepCx<'_>, port: &str, what: String) -> Option<Outcome> {
    cx.dry_run.then(|| Outcome::next(port, json!({"would": what})))
}

fn kind_spec<C: JsonSchema, O: JsonSchema>(
    id: &str,
    title: &str,
    description: &str,
    ports: &[(&str, &str)],
) -> KindSpec {
    spec::<C, O>(id, 1, Category::Action, title, description, ports, false)
}

pub struct Search;

impl NodeKind for Search {
    fn spec(&self) -> KindSpec {
        kind_spec::<ScryerSearchConfig, ScryerSearchOutput>(
            "scryer.search",
            "Search Scryer",
            "Looks up films, series and anime in Scryer's metadata by title: `{{steps.<id>.results}}` is a list of \
             {facet, name, year, tvdbId, tmdbId, imdbId, inLibrary}.",
            &[("out", "Then")],
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: ScryerSearchConfig = parse(config)?;
        if c.term.trim().is_empty() {
            return Err("term is empty".into());
        }
        if let Some(f) = c.facet.as_deref().filter(|f| !is_ref(f)) {
            facet_of(Some(f))?;
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: ScryerSearchConfig = parse(&input)?;
            if let Some(o) = would(cx, "out", format!("search Scryer ({}) for {:?}", c.connection, c.term)) {
                return Ok(o);
            }
            let client = Client::open(cx.st, &c.connection).await?;
            Ok(Outcome::next("out", search(&client, &c).await?))
        })
    }
}

pub struct Add;

impl NodeKind for Add {
    fn spec(&self) -> KindSpec {
        kind_spec::<ScryerAddConfig, ScryerTitle>(
            "scryer.add",
            "Add to Scryer",
            "Adds a film, series or anime to Scryer by its ids (from scryer.search), monitored unless told; takes \
             `exists` when the library already has it.",
            &[("added", "Added"), ("exists", "Already there")],
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        check_add(&parse(config)?)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: ScryerAddConfig = parse(&input)?;
            let what = format!("add the {} {} to Scryer ({})", c.facet, add_ids(&c).describe(), c.connection);
            if let Some(o) = would(cx, "added", what) {
                return Ok(o);
            }
            let client = Client::open(cx.st, &c.connection).await?;
            let (added, t) = add(&client, &c).await?;
            Ok(Outcome::next(if added { "added" } else { "exists" }, t))
        })
    }
}

pub struct Queue;

impl NodeKind for Queue {
    fn spec(&self) -> KindSpec {
        kind_spec::<ScryerQueueConfig, ScryerQueueOutput>(
            "scryer.queue",
            "Scryer queue",
            "What Scryer is downloading: `{{steps.<id>.items}}` is a list of {title, progress, status, secondsLeft, \
             client}.",
            &[("out", "Then")],
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<ScryerQueueConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: ScryerQueueConfig = parse(&input)?;
            if let Some(o) = would(cx, "out", format!("read Scryer's queue ({})", c.connection)) {
                return Ok(o);
            }
            let client = Client::open(cx.st, &c.connection).await?;
            Ok(Outcome::next("out", queue(&client, &c).await?))
        })
    }
}

pub struct Library;

impl NodeKind for Library {
    fn spec(&self) -> KindSpec {
        kind_spec::<ScryerLibraryConfig, ScryerLibraryOutput>(
            "scryer.library",
            "Scryer library",
            "Titles in Scryer's library (a facet, words in the title, monitored or not): `{{steps.<id>.titles}}` is \
             a list of {titleId, facet, name, year, monitored, library, status}.",
            &[("out", "Then")],
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: ScryerLibraryConfig = parse(config)?;
        if let Some(f) = c.facet.as_deref().filter(|f| !is_ref(f)) {
            facet_of(Some(f))?;
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: ScryerLibraryConfig = parse(&input)?;
            if let Some(o) = would(cx, "out", format!("read Scryer's library ({})", c.connection)) {
                return Ok(o);
            }
            let client = Client::open(cx.st, &c.connection).await?;
            Ok(Outcome::next("out", library(&client, &c).await?))
        })
    }
}

/// The four kinds.
pub fn kinds() -> Vec<Box<dyn NodeKind>> {
    vec![Box::new(Search), Box::new(Add), Box::new(Queue), Box::new(Library)]
}

// ---- the PM's tools ----

/// A tool's input schema: its node's config, with `connection` optional (the only `scryer` connection).
pub(crate) fn tool_schema<T: JsonSchema>() -> Value {
    let mut s = pdt_core::mcp::input_schema(crate::workflow::catalog::schema_of::<T>());
    if let Some(r) = s["required"].as_array_mut() {
        r.retain(|k| k != "connection");
    }
    if let Some(c) = s.pointer_mut("/properties/connection") {
        c["description"] = json!("A Scryer connection's name; the only one when left out.");
        if let Some(o) = c.as_object_mut() {
            o.remove("x-connection-kind");
        }
    }
    s
}

/// A tool's arguments as its node's config, `connection` resolved; and the client.
async fn tool_args<T: DeserializeOwned>(st: &AppState, mut a: Value) -> Result<(T, Client), String> {
    if a.is_null() {
        a = json!({});
    }
    let given = a.get("connection").and_then(Value::as_str).map(String::from);
    let conn = crate::workflow::routes::connection_of_kind(st, SCRYER, given.as_deref()).await?;
    a["connection"] = json!(conn.name);
    let c: T = serde_json::from_value(a).map_err(|e| format!("the arguments don't fit the tool's schema: {e}"))?;
    let client = Client::open(st, &conn.name).await?;
    Ok((c, client))
}

fn out<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

pub(crate) async fn search_tool(st: &AppState, a: Value) -> Result<Value, String> {
    let (c, client) = tool_args::<ScryerSearchConfig>(st, a).await?;
    out(search(&client, &c).await?)
}

pub(crate) async fn library_tool(st: &AppState, a: Value) -> Result<Value, String> {
    let (c, client) = tool_args::<ScryerLibraryConfig>(st, a).await?;
    out(library(&client, &c).await?)
}

pub(crate) async fn queue_tool(st: &AppState, a: Value) -> Result<Value, String> {
    let (c, client) = tool_args::<ScryerQueueConfig>(st, a).await?;
    out(queue(&client, &c).await?)
}

pub(crate) async fn add_tool(st: &AppState, a: Value) -> Result<Value, String> {
    let (c, client) = tool_args::<ScryerAddConfig>(st, a).await?;
    let (added, t) = add(&client, &c).await?;
    tracing::info!(connection = %c.connection, title = %t.name, added, "the PM added a title to Scryer");
    Ok(json!({"added": added, "alreadyInLibrary": !added, "title": t}))
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

    #[test]
    fn tool_schemas_leave_the_connection_optional() {
        let s = tool_schema::<ScryerAddConfig>();
        let required: Vec<&str> = s["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(required, ["facet"]);
        assert!(s["properties"]["connection"].get("x-connection-kind").is_none());
    }
}
