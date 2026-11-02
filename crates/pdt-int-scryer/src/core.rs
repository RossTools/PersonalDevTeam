//! Scryer (PDT-147), a media manager for films, series and anime with a GraphQL API: the `scryer` connection's
//! config, its facets and monitoring modes, the external ids an add sends, and the subject of the PM's `scryer_add`
//! call. For the PM's GraphQL tools (PDT-150): which values are credentials (redacted wherever a call is shown or
//! logged), `{"$secret": "<connection>"}` references, the mutations that always ask or are never sent, and the subject
//! of a `scryer_mutate` call. Pure: pdt-api talks to Scryer.

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

// ---- the PM's GraphQL tools (PDT-150) ----

/// Words in a field's name that say its value is a credential.
const SECRET_WORDS: &[&str] = &[
    "password",
    "passwd",
    "apikey",
    "api_key",
    "token",
    "secret",
    "passkey",
    "cookie",
    "credential",
    "privatekey",
    "private_key",
    "authorization",
];

pub use pdt_core::secret::REDACTED;

/// Whether a field called `name` holds a credential (`password`, `apiKey`, `secretValue`, `token`, …).
pub fn is_secret_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase().replace(['-', ' '], "_");
    SECRET_WORDS.iter().any(|w| n.contains(w))
}

/// The connection a `{"$secret": "<connection>"}` reference names: PDT puts that connection's secret in its place
/// before the call goes to Scryer, so the agent never sees it.
pub fn secret_ref(v: &Value) -> Option<&str> {
    let m = v.as_object().filter(|m| m.len() == 1)?;
    m.get("$secret")?.as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// Whether `v` holds a `$secret` reference anywhere.
pub fn has_secret_ref(v: &Value) -> bool {
    match v {
        Value::Object(m) => secret_ref(v).is_some() || m.values().any(has_secret_ref),
        Value::Array(a) => a.iter().any(has_secret_ref),
        _ => false,
    }
}

/// `v` with every credential written out replaced by [`REDACTED`]: a string under a secret-looking name, and the
/// values of a `{key, …Value}` pair whose key is one (Scryer's provider settings: `{key: "apiKey", stringValue}`).
/// `$secret` references stay: they name a connection, not its secret.
pub fn redact(v: &Value) -> Value {
    walk(v, &mut vec![])
}

/// The credentials [`redact`] hides, to take out of text that holds them too (a raw transcript line).
pub fn secret_values(v: &Value) -> Vec<String> {
    let mut found = vec![];
    walk(v, &mut found);
    found.sort();
    found.dedup();
    found
}

fn walk(v: &Value, found: &mut Vec<String>) -> Value {
    match v {
        Value::Object(m) if secret_ref(v).is_none() => {
            let pair = m.get("key").and_then(Value::as_str).is_some_and(is_secret_name);
            let out = m
                .iter()
                .map(|(k, x)| match x.as_str().filter(|s| !s.is_empty()) {
                    Some(s) if is_secret_name(k) || (pair && k != "key") => {
                        found.push(s.to_string());
                        (k.clone(), json!(REDACTED))
                    }
                    _ => (k.clone(), walk(x, found)),
                })
                .collect();
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| walk(x, found)).collect()),
        _ => v.clone(),
    }
}

/// What PDT does with a `scryer_mutate` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationRule {
    /// Asks, unless the connection lets the PM act (`config.pmActs`).
    Asks,
    /// Destructive or high-impact: asks even when the connection lets the PM act.
    AlwaysAsks,
    /// Signing in and the user's own credentials: never sent (the user does them in Scryer).
    Denied,
}

/// Mutations whose name starts with one of these always ask (`deleteIndexerConfig`, `resetProxyHostKey`, …).
const ALWAYS_ASK_PREFIXES: &[&str] =
    &["delete", "remove", "reset", "revoke", "uninstall", "unlink", "empty", "abandon", "clear"];

/// High-impact mutations that always ask.
const ALWAYS_ASK: &[&str] = &[
    "startApplicationUpgrade",
    "applyRestoreBundle",
    "executeExternalImport",
    "finalizeExternalImport",
    "updateSecuritySettings",
    "updateLibraryPaths",
    "createUser",
    "setUserAppPermissions",
    "setUserLibraryPermissions",
    "setUserLoginEnabled",
    "createOauthClientRegistration",
    "updateOauthClientRegistration",
    "beginInstallPlugin",
    "beginUpgradePlugin",
    "installManualPlugin",
    "installUploadedPlugin",
    "createPostProcessingScript",
    "updatePostProcessingScript",
    "togglePostProcessingScript",
    "updateScriptInterpreterSettings",
    "setMaintenanceRuleState",
    "runMaintenanceActionHandlerNow",
    "renameTitles",
    "applyMediaRenameBulk",
    "rehydrateAllMetadata",
    "startLocationOperation",
];

/// Mutations whose name starts with one of these are never sent: signing in, passkeys, two-factor.
const DENIED_PREFIXES: &[&str] = &["login", "webauthn", "totp", "mfa", "accountSecurity"];

/// Mutations never sent: the user's own password and API keys, and Scryer's first-run setup.
const DENIED: &[&str] = &[
    "completeLoginMfaEnrollment",
    "completeRequiredPasswordChange",
    "setUserPassword",
    "createMyApiKey",
    "completeSetup",
];

/// `name` starts with the word `prefix` (`deleteTitle` with `delete`, not `deletedCount`).
fn starts_with_word(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_uppercase()))
}

pub fn mutation_rule(mutation: &str) -> MutationRule {
    let m = mutation.trim();
    if DENIED.contains(&m) || DENIED_PREFIXES.iter().any(|p| starts_with_word(m, p)) {
        MutationRule::Denied
    } else if ALWAYS_ASK.contains(&m) || ALWAYS_ASK_PREFIXES.iter().any(|p| starts_with_word(m, p)) {
        MutationRule::AlwaysAsks
    } else {
        MutationRule::Asks
    }
}

/// Whether a `scryer_mutate` call asks even when the connection lets the PM act: a destructive or high-impact
/// mutation, or one that sends a connection's secret (the user sees where a credential goes).
pub fn mutate_always_asks(input: &Value) -> bool {
    let m = input.get("mutation").and_then(Value::as_str).unwrap_or("");
    mutation_rule(m) != MutationRule::Asks || input.get("args").is_some_and(has_secret_ref)
}

/// The one-line subject of a `scryer_mutate` call, for the user's yes: `Scryer: createIndexerConfig name=NZBgeek,
/// isEnabled`. Its arguments (an `input` object's fields when that is the only one), credentials redacted.
pub fn mutate_ask_title(input: &Value) -> Option<String> {
    let field = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let who = field("connection").map(|c| format!("Scryer ({c})")).unwrap_or_else(|| "Scryer".into());
    let mutation = field("mutation")?;
    let args = redact(input.get("args").unwrap_or(&Value::Null));
    let shown = match args.as_object() {
        Some(m) if m.len() == 1 && m.get("input").is_some_and(Value::is_object) => m["input"].clone(),
        _ => args,
    };
    // Its name and id first, then the rest by name.
    let mut fields: Vec<(&String, &Value)> = shown.as_object().into_iter().flatten().collect();
    fields.sort_by_key(|(k, _)| (k.as_str() != "name", k.as_str() != "id", k.as_str()));
    let parts: Vec<String> = fields
        .into_iter()
        .filter_map(|(k, v)| match v {
            Value::Null => None,
            Value::Bool(true) => Some(k.clone()),
            Value::Bool(false) => Some(format!("{k}=false")),
            Value::String(s) => {
                let short: String = s.chars().take(40).collect();
                Some(if short.len() < s.len() { format!("{k}={short}…") } else { format!("{k}={s}") })
            }
            Value::Number(n) => Some(format!("{k}={n}")),
            Value::Object(_) => Some(match secret_ref(v) {
                Some(c) => format!("{k}=the secret of {c}"),
                None => format!("{k}={{…}}"),
            }),
            Value::Array(a) => Some(format!("{k}=[{} items]", a.len())),
        })
        .collect();
    let mut out = format!("{who}: {mutation}");
    if !parts.is_empty() {
        out.push(' ');
        out.push_str(&parts.join(", "));
    }
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
    #[test]
    fn credentials_are_redacted_by_name_and_by_provider_key() {
        let v = json!({"input": {"name": "NZBgeek", "password": "hunter2", "apiKey": "", "hasApiKey": true,
            "config": [{"key": "apiKey", "stringValue": "abc123"}, {"key": "baseUrl", "stringValue": "https://x"},
                       {"key": "user", "secretValue": "s3"}],
            "token": {"$secret": "nzbgeek"}}});
        let r = redact(&v);
        assert_eq!(r["input"]["password"], REDACTED);
        assert_eq!(r["input"]["apiKey"], "", "nothing to hide");
        assert_eq!(r["input"]["hasApiKey"], true);
        assert_eq!(r["input"]["config"][0], json!({"key": "apiKey", "stringValue": REDACTED}));
        assert_eq!(r["input"]["config"][1]["stringValue"], "https://x");
        assert_eq!(r["input"]["config"][2]["secretValue"], REDACTED);
        assert_eq!(r["input"]["token"], json!({"$secret": "nzbgeek"}), "a reference names no secret");
        assert_eq!(secret_values(&v), ["abc123", "hunter2", "s3"]);
        assert!(is_secret_name("clientSecret") && is_secret_name("X-Api-Key") && !is_secret_name("name"));
    }

    #[test]
    fn secret_references_are_found_anywhere() {
        assert_eq!(secret_ref(&json!({"$secret": " sab "})), Some("sab"));
        assert_eq!(secret_ref(&json!({"$secret": "sab", "x": 1})), None);
        assert!(has_secret_ref(&json!({"input": {"config": [{"key": "k", "secretValue": {"$secret": "sab"}}]}})));
        assert!(!has_secret_ref(&json!({"input": {"secret": "plain"}})));
    }

    #[test]
    fn destructive_mutations_always_ask_and_sign_ins_are_never_sent() {
        assert_eq!(mutation_rule("createIndexerConfig"), MutationRule::Asks);
        assert_eq!(mutation_rule("deleteIndexerConfig"), MutationRule::AlwaysAsks);
        assert_eq!(mutation_rule("resetProxyHostKey"), MutationRule::AlwaysAsks);
        assert_eq!(mutation_rule("startApplicationUpgrade"), MutationRule::AlwaysAsks);
        assert_eq!(mutation_rule("deletedThings"), MutationRule::Asks, "a word, not a prefix of one");
        assert_eq!(mutation_rule("loginWithPlex"), MutationRule::Denied);
        assert_eq!(mutation_rule("createMyApiKey"), MutationRule::Denied);
        assert!(mutate_always_asks(&json!({"mutation": "deleteTitle"})));
        assert!(mutate_always_asks(&json!({"mutation": "createDownloadClientConfig",
            "args": {"input": {"config": [{"key": "password", "secretValue": {"$secret": "sab"}}]}}})));
        assert!(!mutate_always_asks(&json!({"mutation": "updateGeneralSettings", "args": {}})));
    }

    #[test]
    fn the_mutation_approval_names_the_change_without_its_credentials() {
        let t = mutate_ask_title(&json!({"mutation": "createIndexerConfig", "args": {"input": {
            "name": "NZBgeek", "providerType": "newznab", "isEnabled": true, "enableAutoSearch": false,
            "rateLimitSeconds": 2, "proxyConfigId": null, "apiKey": "abc123",
            "config": [{"key": "apiKey", "secretValue": {"$secret": "nzbgeek"}}]}}}));
        assert_eq!(
            t.as_deref(),
            Some(
                "Scryer: createIndexerConfig name=NZBgeek, apiKey=[redacted], config=[1 items], \
                 enableAutoSearch=false, isEnabled, providerType=newznab, rateLimitSeconds=2"
            )
        );
        let t = mutate_ask_title(&json!({"connection": "home", "mutation": "deleteIndexerConfig", "args": {"id": "7"},
                                         "password": {"$secret": "x"}}));
        assert_eq!(t.as_deref(), Some("Scryer (home): deleteIndexerConfig id=7"));
        assert_eq!(mutate_ask_title(&json!({"args": {}})), None);
    }
}
