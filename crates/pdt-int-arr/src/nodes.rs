//! Radarr and Sonarr (Phase 7e spec §7.1): `radarr.search`, `radarr.add`, `radarr.queue` and the same three for
//! Sonarr. The two speak the same v3 API (films by TMDB id, series by TVDB id), so one module serves both. A
//! connection of kind `radarr` or `sonarr`: `url` the app's address (`http://nas:7878`), the secret its API key (sent
//! as `X-Api-Key`), `config.qualityProfile` and `config.rootFolder` the defaults for adds.

use pdt_api::sdk::http::{call_json, service};
use pdt_api::sdk::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::api::Connection;
use reqwest::Method;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

/// How many search results a step keeps unless told.
const DEFAULT_LIMIT: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arr {
    Radarr,
    Sonarr,
}

impl Arr {
    /// The connection kind (and the kinds' prefix).
    pub fn kind(self) -> &'static str {
        match self {
            Arr::Radarr => "radarr",
            Arr::Sonarr => "sonarr",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Arr::Radarr => "Radarr",
            Arr::Sonarr => "Sonarr",
        }
    }
    /// The id a film or series is known by.
    pub fn id_field(self) -> &'static str {
        match self {
            Arr::Radarr => "tmdbId",
            Arr::Sonarr => "tvdbId",
        }
    }
    fn resource(self) -> &'static str {
        match self {
            Arr::Radarr => "movie",
            Arr::Sonarr => "series",
        }
    }
    fn noun(self) -> &'static str {
        match self {
            Arr::Radarr => "films",
            Arr::Sonarr => "series",
        }
    }
}

/// A kind's spec with its `connection` field saying which connection kind it takes.
fn arr_spec<C: JsonSchema, O: JsonSchema>(
    arr: Arr,
    id: &str,
    title: &str,
    description: &str,
    ports: &[(&str, &str)],
) -> KindSpec {
    let mut s = spec::<C, O>(id, 1, Category::Action, title, description, ports, false);
    if let Some(c) = s.config_schema.pointer_mut("/properties/connection") {
        c["x-connection-kind"] = json!(arr.kind());
    }
    s
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ArrSearchConfig {
    /// A `radarr` or `sonarr` connection.
    pub connection: String,
    /// A title to look up (`dune`), or `tmdb:<id>` / `tvdb:<id>`.
    pub term: String,
    /// At most this many results (default 10).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// A film or series as a step outputs it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArrItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tmdb_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tvdb_id: Option<u64>,
    pub title: String,
    pub year: Option<u64>,
    /// Already in the library.
    pub in_library: bool,
    /// Its id in the library, when it is there.
    pub id: Option<u64>,
    #[serde(default)]
    pub overview: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArrSearchOutput {
    pub results: Vec<ArrItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct RadarrAddConfig {
    /// A `radarr` connection.
    pub connection: String,
    /// The film's TMDB id (from `radarr.search`).
    #[schemars(with = "u64")]
    #[ts(type = "number | string")]
    pub tmdb_id: Value,
    /// A quality profile's name or id; default the connection's `config.qualityProfile`, else the first.
    #[serde(default)]
    pub quality_profile: Option<String>,
    /// A root folder's path; default the connection's `config.rootFolder`, else the first.
    #[serde(default)]
    pub root_folder: Option<String>,
    /// Search for it once added (default true).
    #[serde(default = "yes")]
    pub search: bool,
    /// `announced`, `inCinemas` or `released` (default `released`).
    #[serde(default)]
    pub minimum_availability: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct SonarrAddConfig {
    /// A `sonarr` connection.
    pub connection: String,
    /// The series' TVDB id (from `sonarr.search`).
    #[schemars(with = "u64")]
    #[ts(type = "number | string")]
    pub tvdb_id: Value,
    /// A quality profile's name or id; default the connection's `config.qualityProfile`, else the first.
    #[serde(default)]
    pub quality_profile: Option<String>,
    /// A root folder's path; default the connection's `config.rootFolder`, else the first.
    #[serde(default)]
    pub root_folder: Option<String>,
    /// Which episodes to monitor: `all` (default), `future`, `missing`, `existing`, `firstSeason`, `latestSeason`,
    /// `pilot` or `none`.
    #[serde(default)]
    pub monitor: Option<String>,
    /// Search for missing episodes once added (default true).
    #[serde(default = "yes")]
    pub search: bool,
}

fn yes() -> bool {
    true
}

const MONITOR: &[&str] = &["all", "future", "missing", "existing", "firstSeason", "latestSeason", "pilot", "none"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ArrQueueConfig {
    /// A `radarr` or `sonarr` connection.
    pub connection: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QueueItem {
    /// The film (`Dune (2021)`) or episode (`Severance S02E03`).
    pub title: String,
    /// The release being downloaded.
    pub release: String,
    /// Percent done.
    pub progress: f64,
    /// When it should finish (RFC 3339), if known.
    pub eta: Option<String>,
    /// Time left (`00:12:30`), if known.
    pub time_left: Option<String>,
    /// `downloading`, `importPending`, `queued`, `paused`, …
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArrQueueOutput {
    pub items: Vec<QueueItem>,
}

/// One app behind its connection.
struct Client {
    arr: Arr,
    conn: Connection,
    key: String,
}

impl Client {
    async fn open(cx: &StepCx<'_>, arr: Arr, name: &str) -> Result<Self, String> {
        let (conn, key) = service(cx.st, name, arr.kind()).await?;
        Ok(Self { arr, conn, key })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value, String> {
        let who = format!("{} ({})", self.arr.title(), self.conn.name);
        let path = format!("/api/v3/{path}");
        call_json(&who, method, &self.conn.url, &path, &[("X-Api-Key", &self.key)], query, body).await
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, String> {
        self.call(Method::GET, path, query, None).await
    }

    fn default_of(&self, key: &str) -> Option<String> {
        match self.conn.config.get(key)? {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    /// The quality profile's id: `wanted` (a name or id), else the connection's default, else the first.
    async fn profile(&self, wanted: Option<&str>) -> Result<u64, String> {
        let wanted =
            wanted.map(str::trim).filter(|w| !w.is_empty()).map(String::from).or(self.default_of("qualityProfile"));
        let list = self.get("qualityprofile", &[]).await?;
        let profiles = list.as_array().cloned().unwrap_or_default();
        let found = match &wanted {
            Some(w) => profiles.iter().find(|p| {
                p["id"].as_u64().is_some_and(|id| id.to_string() == *w)
                    || p["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(w))
            }),
            None => profiles.first(),
        };
        found.and_then(|p| p["id"].as_u64()).ok_or_else(|| match wanted {
            Some(w) => format!("{} has no quality profile {w:?}", self.conn.name),
            None => format!("{} has no quality profiles", self.conn.name),
        })
    }

    /// The root folder: `wanted`, else the connection's default, else the first.
    async fn root(&self, wanted: Option<&str>) -> Result<String, String> {
        if let Some(w) =
            wanted.map(str::trim).filter(|w| !w.is_empty()).map(String::from).or(self.default_of("rootFolder"))
        {
            return Ok(w);
        }
        let list = self.get("rootfolder", &[]).await?;
        list.as_array()
            .and_then(|l| l.first())
            .and_then(|f| f["path"].as_str())
            .map(String::from)
            .ok_or_else(|| format!("{} has no root folder", self.conn.name))
    }

    /// The library's film or series with this id, if there is one.
    async fn in_library(&self, id: u64) -> Result<Option<Value>, String> {
        let field = self.arr.id_field();
        // Newer versions filter by the id; older ones return the whole library, so check each.
        let list = self.get(self.arr.resource(), &[(field, id.to_string())]).await?;
        Ok(list.as_array().and_then(|l| l.iter().find(|m| m[field].as_u64() == Some(id)).cloned()))
    }
}

/// A film or series from Radarr's or Sonarr's JSON.
pub fn item(arr: Arr, v: &Value) -> ArrItem {
    let id = v["id"].as_u64().filter(|&i| i > 0);
    let ext = v[arr.id_field()].as_u64();
    ArrItem {
        tmdb_id: if arr == Arr::Radarr { ext } else { None },
        tvdb_id: if arr == Arr::Sonarr { ext } else { None },
        title: v["title"].as_str().unwrap_or_default().to_string(),
        year: v["year"].as_u64().filter(|&y| y > 0),
        in_library: id.is_some(),
        id,
        overview: v["overview"].as_str().unwrap_or_default().to_string(),
    }
}

/// A queue record as a [`QueueItem`].
pub fn queue_item(arr: Arr, r: &Value) -> QueueItem {
    let release = r["title"].as_str().unwrap_or_default().to_string();
    let title = match arr {
        Arr::Radarr => r["movie"]["title"].as_str().map(|t| match r["movie"]["year"].as_u64() {
            Some(y) if y > 0 => format!("{t} ({y})"),
            _ => t.to_string(),
        }),
        Arr::Sonarr => r["series"]["title"].as_str().map(|t| {
            match (r["episode"]["seasonNumber"].as_u64(), r["episode"]["episodeNumber"].as_u64()) {
                (Some(s), Some(e)) => format!("{t} S{s:02}E{e:02}"),
                _ => t.to_string(),
            }
        }),
    }
    .unwrap_or_else(|| release.clone());
    let size = r["size"].as_f64().unwrap_or(0.0);
    let left = r["sizeleft"].as_f64().unwrap_or(0.0);
    let progress = if size > 0.0 { ((size - left) / size * 1000.0).round() / 10.0 } else { 0.0 };
    let s = |k: &str| r[k].as_str().filter(|s| !s.is_empty()).map(String::from);
    QueueItem {
        title,
        release,
        progress,
        eta: s("estimatedCompletionTime"),
        time_left: s("timeleft"),
        status: s("trackedDownloadState").or(s("status")).unwrap_or_default(),
    }
}

/// An id from a config value: a number, or a string holding one.
pub fn as_id(field: &str, v: &Value) -> Result<u64, String> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|&n| n > 0)
    .ok_or_else(|| format!("{field} {v} is not an id"))
}

fn is_ref(v: &Value) -> bool {
    v.as_str().is_some_and(|s| s.contains("{{"))
}

fn would(cx: &StepCx<'_>, what: String) -> Option<Outcome> {
    cx.dry_run.then(|| Outcome::next("out", json!({"would": what})))
}

pub struct Search(pub Arr);

impl NodeKind for Search {
    fn spec(&self) -> KindSpec {
        let a = self.0;
        arr_spec::<ArrSearchConfig, ArrSearchOutput>(
            a,
            &format!("{}.search", a.kind()),
            &format!("Search {}", a.title()),
            &format!(
                "Looks up {} in {} by title: `{{{{steps.<id>.results}}}}` is a list of {{{}, title, year, inLibrary}}.",
                a.noun(),
                a.title(),
                a.id_field()
            ),
            &[("out", "Then")],
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: ArrSearchConfig = parse(config)?;
        if c.term.trim().is_empty() {
            return Err("term is empty".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let a = self.0;
            let c: ArrSearchConfig = parse(&input)?;
            if let Some(o) = would(cx, format!("search {} ({}) for {:?}", a.title(), c.connection, c.term)) {
                return Ok(o);
            }
            let client = Client::open(cx, a, &c.connection).await?;
            let list = client.get(&format!("{}/lookup", a.resource()), &[("term", c.term.trim().to_string())]).await?;
            let results = list
                .as_array()
                .map(|l| l.iter().take(c.limit.unwrap_or(DEFAULT_LIMIT)).map(|v| item(a, v)).collect())
                .unwrap_or_default();
            Ok(Outcome::next("out", ArrSearchOutput { results }))
        })
    }
}

pub struct Add(pub Arr);

impl NodeKind for Add {
    fn spec(&self) -> KindSpec {
        let a = self.0;
        let (id, title, description) = (
            format!("{}.add", a.kind()),
            format!("Add to {}", a.title()),
            format!(
                "Adds a {} to {} by its {} (quality profile and root folder from the connection unless given) and \
                 searches for it; takes `exists` when it is already in the library.",
                if a == Arr::Radarr { "film" } else { "series" },
                a.title(),
                a.id_field()
            ),
        );
        let ports = [("added", "Added"), ("exists", "Already there")];
        match a {
            Arr::Radarr => arr_spec::<RadarrAddConfig, ArrItem>(a, &id, &title, &description, &ports),
            Arr::Sonarr => arr_spec::<SonarrAddConfig, ArrItem>(a, &id, &title, &description, &ports),
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let (id, field) = match self.0 {
            Arr::Radarr => (parse::<RadarrAddConfig>(config)?.tmdb_id, "tmdbId"),
            Arr::Sonarr => {
                let c: SonarrAddConfig = parse(config)?;
                if let Some(m) = c.monitor.as_deref().filter(|m| !m.contains("{{") && !m.is_empty())
                    && !MONITOR.contains(&m)
                {
                    return Err(format!("monitor {m:?} is not one of {}", MONITOR.join(", ")));
                }
                (c.tvdb_id, "tvdbId")
            }
        };
        if !is_ref(&id) {
            as_id(field, &id)?;
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let a = self.0;
            // The fields both share; each adds its own options below.
            let (connection, id, profile, root, search) = match a {
                Arr::Radarr => {
                    let c: RadarrAddConfig = parse(&input)?;
                    (c.connection, as_id("tmdbId", &c.tmdb_id)?, c.quality_profile, c.root_folder, c.search)
                }
                Arr::Sonarr => {
                    let c: SonarrAddConfig = parse(&input)?;
                    (c.connection, as_id("tvdbId", &c.tvdb_id)?, c.quality_profile, c.root_folder, c.search)
                }
            };
            if cx.dry_run {
                let what = format!("add {} {id} to {} ({connection})", a.id_field(), a.title());
                return Ok(Outcome::next("added", json!({"would": what})));
            }
            let client = Client::open(cx, a, &connection).await?;
            if let Some(existing) = client.in_library(id).await? {
                return Ok(Outcome::next("exists", item(a, &existing)));
            }
            let mut body = match a {
                Arr::Radarr => client.get("movie/lookup/tmdb", &[("tmdbId", id.to_string())]).await?,
                Arr::Sonarr => {
                    let l = client.get("series/lookup", &[("term", format!("tvdb:{id}"))]).await?;
                    l.as_array().and_then(|l| l.first()).cloned().unwrap_or(Value::Null)
                }
            };
            if !body.is_object() || body[a.id_field()].as_u64() != Some(id) {
                return Err(format!("{} found nothing with {} {id}", a.title(), a.id_field()));
            }
            body["qualityProfileId"] = json!(client.profile(profile.as_deref()).await?);
            body["rootFolderPath"] = json!(client.root(root.as_deref()).await?);
            body["monitored"] = json!(true);
            match a {
                Arr::Radarr => {
                    let c: RadarrAddConfig = parse(&input)?;
                    let min = c.minimum_availability.filter(|m| !m.is_empty()).unwrap_or_else(|| "released".into());
                    body["minimumAvailability"] = json!(min);
                    body["addOptions"] = json!({"searchForMovie": search});
                }
                Arr::Sonarr => {
                    let c: SonarrAddConfig = parse(&input)?;
                    let monitor = c.monitor.filter(|m| !m.is_empty()).unwrap_or_else(|| "all".into());
                    body["seasonFolder"] = json!(true);
                    body["addOptions"] = json!({"monitor": monitor, "searchForMissingEpisodes": search});
                }
            }
            let added = client.call(Method::POST, a.resource(), &[], Some(&body)).await?;
            Ok(Outcome::next("added", item(a, &added)))
        })
    }
}

pub struct Queue(pub Arr);

impl NodeKind for Queue {
    fn spec(&self) -> KindSpec {
        let a = self.0;
        arr_spec::<ArrQueueConfig, ArrQueueOutput>(
            a,
            &format!("{}.queue", a.kind()),
            &format!("{} queue", a.title()),
            &format!(
                "What {} is downloading: `{{{{steps.<id>.items}}}}` is a list of {{title, release, progress, eta, \
                 status}}.",
                a.title()
            ),
            &[("out", "Then")],
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<ArrQueueConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let a = self.0;
            let c: ArrQueueConfig = parse(&input)?;
            if let Some(o) = would(cx, format!("read {}'s queue ({})", a.title(), c.connection)) {
                return Ok(o);
            }
            let client = Client::open(cx, a, &c.connection).await?;
            let mut query = vec![("page", "1".to_string()), ("pageSize", "100".to_string())];
            match a {
                Arr::Radarr => query.push(("includeMovie", "true".into())),
                Arr::Sonarr => {
                    query.push(("includeSeries", "true".into()));
                    query.push(("includeEpisode", "true".into()));
                }
            }
            let page = client.get("queue", &query).await?;
            // v3 pages the queue (`records`); some versions answer a bare list.
            let records = page["records"].as_array().or(page.as_array()).cloned().unwrap_or_default();
            Ok(Outcome::next("out", ArrQueueOutput { items: records.iter().map(|r| queue_item(a, r)).collect() }))
        })
    }
}

/// The six kinds.
pub fn kinds() -> Vec<Box<dyn NodeKind>> {
    let mut out: Vec<Box<dyn NodeKind>> = vec![];
    for a in [Arr::Radarr, Arr::Sonarr] {
        out.push(Box::new(Search(a)));
        out.push(Box::new(Add(a)));
        out.push(Box::new(Queue(a)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_say_whether_they_are_in_the_library() {
        let v = json!({"title": "Dune", "year": 2021, "tmdbId": 438631, "id": 7, "overview": "Spice."});
        let i = item(Arr::Radarr, &v);
        assert_eq!((i.tmdb_id, i.tvdb_id, i.in_library, i.id, i.year), (Some(438631), None, true, Some(7), Some(2021)));
        let i = item(Arr::Sonarr, &json!({"title": "Severance", "tvdbId": 371980, "id": 0, "year": 0}));
        assert_eq!((i.tmdb_id, i.tvdb_id, i.in_library, i.year), (None, Some(371980), false, None));
    }

    #[test]
    fn queue_records_get_a_readable_title_and_progress() {
        let r = json!({"title": "Dune.2021.2160p", "size": 1000.0, "sizeleft": 250.0, "timeleft": "00:10:00",
            "estimatedCompletionTime": "2026-10-22T10:00:00Z", "trackedDownloadState": "downloading",
            "movie": {"title": "Dune", "year": 2021}});
        let q = queue_item(Arr::Radarr, &r);
        assert_eq!((q.title.as_str(), q.release.as_str(), q.progress), ("Dune (2021)", "Dune.2021.2160p", 75.0));
        assert_eq!((q.time_left.as_deref(), q.status.as_str()), (Some("00:10:00"), "downloading"));
        let r = json!({"title": "rel", "size": 0, "status": "queued",
            "series": {"title": "Severance"}, "episode": {"seasonNumber": 2, "episodeNumber": 3}});
        let q = queue_item(Arr::Sonarr, &r);
        assert_eq!((q.title.as_str(), q.progress, q.status.as_str()), ("Severance S02E03", 0.0, "queued"));
        assert_eq!(queue_item(Arr::Sonarr, &json!({"title": "bare"})).title, "bare");
    }

    #[test]
    fn ids_are_numbers_or_numeric_strings() {
        assert_eq!(as_id("tmdbId", &json!(42)), Ok(42));
        assert_eq!(as_id("tmdbId", &json!(" 42 ")), Ok(42));
        assert!(as_id("tmdbId", &json!(0)).is_err());
        assert!(as_id("tmdbId", &json!("dune")).is_err());
        assert!(as_id("tmdbId", &json!(null)).is_err());
    }

    #[test]
    fn connection_fields_name_their_app() {
        for k in kinds() {
            let s = k.spec();
            let want = s.id.split('.').next().unwrap().to_string();
            assert_eq!(s.config_schema["properties"]["connection"]["x-connection-kind"], json!(want), "{}", s.id);
        }
    }
}
