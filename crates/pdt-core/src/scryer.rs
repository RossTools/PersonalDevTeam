//! Scryer (PDT-147), a media manager for films, series and anime with a GraphQL API: the `scryer` connection's
//! config, its facets and monitoring modes, the external ids an add sends, and the subject of the PM's `scryer_add`
//! call. Pure: pdt-api talks to Scryer.

use serde_json::{Value, json};

/// The connection kind: `url` Scryer's address (with its base path, if any), the secret an API key (`ska_…`), sent as
/// a bearer token; `config.pmActs` lets the PM add without asking.
pub const SCRYER: &str = "scryer";

/// What a title is (Scryer's `MediaFacetValue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facet {
    Movie,
    Series,
    Anime,
}

impl Facet {
    /// `movie`, `series` or `anime` (also `film`, `tv`, any case, and Scryer's own `MOVIE`).
    pub fn parse(s: &str) -> Result<Facet, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "movie" | "movies" | "film" | "films" => Ok(Facet::Movie),
            "series" | "show" | "tv" => Ok(Facet::Series),
            "anime" => Ok(Facet::Anime),
            other => Err(format!("{other:?} is not a facet: movie, series or anime")),
        }
    }

    /// Scryer's enum value.
    pub fn gql(self) -> &'static str {
        match self {
            Facet::Movie => "MOVIE",
            Facet::Series => "SERIES",
            Facet::Anime => "ANIME",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Facet::Movie => "movie",
            Facet::Series => "series",
            Facet::Anime => "anime",
        }
    }

    /// From Scryer's answer (`MOVIE`).
    pub fn from_gql(v: &Value) -> Option<Facet> {
        v.as_str().and_then(|s| Facet::parse(s).ok())
    }

    /// The id it is mainly known by in Scryer (films by TMDB, series and anime by TVDB).
    pub fn main_source(self) -> &'static str {
        match self {
            Facet::Movie => "tmdb",
            Facet::Series | Facet::Anime => "tvdb",
        }
    }
}

/// How an added series' episodes are monitored: Scryer's `MonitorTypeValue`.
pub const MONITOR: &[&str] = &["all", "future", "missingAndFuture", "none"];

/// The monitor type an add sends: `monitor` (one of [`MONITOR`]), else every episode when monitored and none when not.
pub fn monitor_type(monitor: Option<&str>, monitored: bool) -> Result<&'static str, String> {
    match monitor.map(str::trim).filter(|m| !m.is_empty()) {
        None if monitored => Ok("ALL_EPISODES"),
        None => Ok("NONE"),
        Some(m) => match m.to_ascii_lowercase().as_str() {
            "all" => Ok("ALL_EPISODES"),
            "future" => Ok("FUTURE_EPISODES"),
            "missingandfuture" => Ok("MISSING_AND_FUTURE_EPISODES"),
            "none" => Ok("NONE"),
            _ => Err(format!("monitor {m:?} is not one of {}", MONITOR.join(", "))),
        },
    }
}

/// A title's ids at the metadata sources (what a search gives and an add takes). Empty strings count as missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ids {
    pub tvdb: Option<String>,
    pub tmdb: Option<u64>,
    pub imdb: Option<String>,
}

impl Ids {
    /// From a search result or a title (`tvdbId`, `tmdbId`, `imdbId`: strings or numbers).
    pub fn from_json(v: &Value) -> Ids {
        let text = |k: &str| match &v[k] {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        Ids { tvdb: text("tvdbId"), tmdb: text("tmdbId").and_then(|t| t.parse().ok()), imdb: text("imdbId") }
    }

    pub fn is_empty(&self) -> bool {
        self.tvdb.is_none() && self.tmdb.is_none() && self.imdb.is_none()
    }

    /// `(source, value)` for each id there is.
    pub fn pairs(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![];
        if let Some(t) = &self.tvdb {
            out.push(("tvdb", t.clone()));
        }
        if let Some(t) = self.tmdb {
            out.push(("tmdb", t.to_string()));
        }
        if let Some(i) = &self.imdb {
            out.push(("imdb", i.clone()));
        }
        out
    }

    /// `addTitle`'s `externalIds`, as Scryer's own web sends them: a series' TMDB id says it is a series (TMDB uses
    /// the same numbers for films and series).
    pub fn external_ids(&self, facet: Facet) -> Vec<Value> {
        self.pairs()
            .into_iter()
            .map(|(source, value)| match (source, facet) {
                ("tmdb", Facet::Series | Facet::Anime) => json!({"source": source, "kind": "series", "value": value}),
                _ => json!({"source": source, "value": value}),
            })
            .collect()
    }

    /// Whether a title's external ids (Scryer's `[{source, value}]`) share one of these.
    pub fn matches(&self, external: &Value) -> bool {
        let mine = self.pairs();
        external.as_array().into_iter().flatten().any(|e| {
            let (Some(s), Some(v)) = (e["source"].as_str(), e["value"].as_str()) else { return false };
            mine.iter().any(|(ms, mv)| s.eq_ignore_ascii_case(ms) && v.trim().eq_ignore_ascii_case(mv))
        })
    }

    /// `tvdb 371980, imdb tt11280740`.
    pub fn describe(&self) -> String {
        self.pairs().into_iter().map(|(s, v)| format!("{s} {v}")).collect::<Vec<_>>().join(", ")
    }
}

/// The one-line subject of a `scryer_add` call, for the user's yes (Claude's approval card).
pub fn ask_title(input: &Value) -> Option<String> {
    let field = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let who = field("connection").map(|c| format!("Scryer ({c})")).unwrap_or_else(|| "Scryer".into());
    let facet = field("facet").and_then(|f| Facet::parse(f).ok())?;
    let ids = Ids::from_json(input);
    let name = field("name").map(|n| format!("{n} ({})", ids.describe())).unwrap_or_else(|| ids.describe());
    let monitored = input.get("monitored").and_then(Value::as_bool) != Some(false);
    let mut out = format!("{who}: add the {} {name}", facet.as_str());
    if let Some(l) = field("library") {
        out.push_str(&format!(" to {l}"));
    }
    out.push_str(if monitored { ", monitored" } else { ", not monitored" });
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facets_parse_loosely_and_map_to_scryer() {
        assert_eq!(Facet::parse(" Film "), Ok(Facet::Movie));
        assert_eq!(Facet::parse("SERIES"), Ok(Facet::Series));
        assert_eq!(Facet::parse("anime").map(Facet::gql), Ok("ANIME"));
        assert!(Facet::parse("music").unwrap_err().contains("movie, series or anime"));
        assert_eq!(Facet::from_gql(&json!("MOVIE")), Some(Facet::Movie));
        assert_eq!(Facet::from_gql(&json!(null)), None);
    }

    #[test]
    fn monitoring_defaults_follow_monitored() {
        assert_eq!(monitor_type(None, true), Ok("ALL_EPISODES"));
        assert_eq!(monitor_type(Some(" "), false), Ok("NONE"));
        assert_eq!(monitor_type(Some("missingAndFuture"), true), Ok("MISSING_AND_FUTURE_EPISODES"));
        assert_eq!(monitor_type(Some("Future"), true), Ok("FUTURE_EPISODES"));
        assert!(monitor_type(Some("pilot"), true).unwrap_err().contains("all, future"));
    }

    #[test]
    fn ids_come_from_strings_or_numbers_and_say_a_series_tmdb_id_is_a_series() {
        let ids = Ids::from_json(&json!({"tvdbId": "371980", "tmdbId": "95396", "imdbId": " ", "x": 1}));
        assert_eq!(ids, Ids { tvdb: Some("371980".into()), tmdb: Some(95396), imdb: None });
        assert_eq!(
            ids.external_ids(Facet::Series),
            vec![
                json!({"source": "tvdb", "value": "371980"}),
                json!({"source": "tmdb", "kind": "series", "value": "95396"})
            ]
        );
        let film = Ids::from_json(&json!({"tvdbId": "", "tmdbId": 438631}));
        assert_eq!(film.external_ids(Facet::Movie), vec![json!({"source": "tmdb", "value": "438631"})]);
        assert!(Ids::from_json(&json!({"tvdbId": ""})).is_empty());
        assert_eq!(ids.describe(), "tvdb 371980, tmdb 95396");
    }

    #[test]
    fn a_title_matches_by_any_shared_id() {
        let ids = Ids { tmdb: Some(438631), ..Ids::default() };
        assert!(ids.matches(&json!([{"source": "TMDB", "value": "438631"}, {"source": "imdb", "value": "tt1"}])));
        assert!(!ids.matches(&json!([{"source": "tvdb", "value": "438631"}])));
        assert!(!ids.matches(&json!(null)));
    }

    #[test]
    fn the_approval_says_what_is_added_where() {
        let t = ask_title(&json!({"connection": "home", "facet": "series", "tvdbId": 371980, "name": "Severance",
                                  "library": "TV"}));
        assert_eq!(t.as_deref(), Some("Scryer (home): add the series Severance (tvdb 371980) to TV, monitored"));
        let t = ask_title(&json!({"facet": "movie", "tmdbId": "438631", "monitored": false}));
        assert_eq!(t.as_deref(), Some("Scryer: add the movie tmdb 438631, not monitored"));
        assert_eq!(ask_title(&json!({"facet": "music"})), None);
    }
}
