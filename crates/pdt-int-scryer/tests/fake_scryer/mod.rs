//! A fake Scryer (PDT-147): its GraphQL endpoint under a base path (`/scryer/graphql`), answering the operations PDT
//! sends by name, with a library that starts with Severance. The API key it takes is [`KEY`]. For the PM's GraphQL
//! tools (PDT-150) it answers introspection from the bundled schema plus a field the bundled one lacks
//! (`fakeVersion`, so a test sees the live schema is used), queries for indexers, download clients and the general
//! settings by their root fields, and `PdtMutate`'s indexer and download-client changes.

use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub const KEY: &str = "ska_test-key";

/// One request: its path, operation name, variables and Authorization header.
pub type Seen = (String, String, Value, String);

#[derive(Clone, Default)]
pub struct FakeScryer {
    pub titles: Arc<Mutex<Vec<Value>>>,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    /// Indexers and download clients as created, with the inputs they were sent (secrets too).
    pub indexers: Arc<Mutex<Vec<Value>>>,
    pub clients: Arc<Mutex<Vec<Value>>>,
    /// It answers introspection (on unless a test turns it off).
    pub introspection: Arc<AtomicBool>,
}

/// The fake's schema: the bundled one and a field only a live Scryer has.
fn schema() -> &'static apollo_compiler::validation::Valid<apollo_compiler::Schema> {
    static S: OnceLock<apollo_compiler::validation::Valid<apollo_compiler::Schema>> = OnceLock::new();
    S.get_or_init(|| {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/schema.graphql");
        let text = std::fs::read_to_string(path).unwrap() + "\nextend type QueryRoot { fakeVersion: String }\n";
        apollo_compiler::Schema::parse_and_validate(text, "schema.graphql").unwrap()
    })
}

/// An introspection query's answer, as a real GraphQL server gives it.
fn introspect(query: &str) -> Value {
    use apollo_compiler::ExecutableDocument;
    let s = schema();
    let doc = ExecutableDocument::parse_and_validate(s, query, "q.graphql").unwrap();
    let op = doc.operations.get(None).unwrap();
    let vars = apollo_compiler::request::coerce_variable_values(s, op, &Default::default()).unwrap();
    let r = apollo_compiler::introspection::partial_execute(s, &s.implementers_map(), &doc, op, &vars).unwrap();
    serde_json::to_value(r.data).unwrap()
}

fn severance() -> Value {
    json!({"id": "t1", "name": "Severance", "year": 2022, "facet": "SERIES", "monitored": true,
           "libraryName": "TV", "contentStatus": "continuing", "episodesOwned": 9, "episodesTotal": 19,
           "sizeBytes": 1000, "externalIds": [{"source": "tvdb", "value": "371980"},
                                             {"source": "tmdb", "kind": "series", "value": "95396"}]})
}

fn dune() -> Value {
    json!({"tvdbId": "", "tmdbId": 438631, "imdbId": "tt1160419", "smgId": 7, "name": "Dune", "year": 2021,
           "overview": "Spice."})
}

impl FakeScryer {
    /// The operation's name: `query PdtSearch(…` → `PdtSearch`.
    fn operation(query: &str) -> String {
        query.split(['(', '{']).next().unwrap_or("").split_whitespace().nth(1).unwrap_or("").to_string()
    }

    /// The PM's own queries, by the root fields they read.
    fn query(&self, query: &str) -> Value {
        let mut d = json!({});
        if query.contains("indexers") {
            d["indexers"] =
                json!(self.indexers.lock().unwrap().iter().map(|i| i["payload"].clone()).collect::<Vec<_>>());
        }
        if query.contains("downloadClientConfigs") {
            d["downloadClientConfigs"] =
                json!(self.clients.lock().unwrap().iter().map(|c| c["payload"].clone()).collect::<Vec<_>>());
        }
        if query.contains("generalSettings") {
            d["generalSettings"] = json!({"historyRetentionDays": 30});
        }
        if query.contains("fakeVersion") {
            d["fakeVersion"] = json!("2.0");
        }
        d
    }

    /// `PdtMutate`: the one mutation in it.
    fn mutate(&self, query: &str, v: &Value) -> Result<Value, String> {
        let i = &v["input"];
        let config = |k: &str| {
            i["config"].as_array().into_iter().flatten().any(|c| c["key"] == k && c["secretValue"].is_string())
        };
        if query.contains("createIndexerConfig(") {
            let mut all = self.indexers.lock().unwrap();
            let payload = json!({"id": format!("i{}", all.len() + 1), "name": i["name"], "providerType": i["providerType"],
                                 "baseUrl": "", "isEnabled": i["isEnabled"].as_bool().unwrap_or(true),
                                 "hasApiKey": config("apiKey")});
            all.push(json!({"payload": payload, "input": i}));
            return Ok(json!({"createIndexerConfig": payload}));
        }
        if query.contains("createDownloadClientConfig(") {
            let mut all = self.clients.lock().unwrap();
            let payload = json!({"id": format!("c{}", all.len() + 1), "name": i["name"], "clientType": i["clientType"],
                                 "isEnabled": true, "storedSecretKeys": if config("password") { json!(["password"]) } else { json!([]) },
                                 "config": [{"key": "password", "defaultValue": "echoed-by-a-bad-scryer"}]});
            all.push(json!({"payload": payload, "input": i}));
            return Ok(json!({"createDownloadClientConfig": payload}));
        }
        if query.contains("deleteIndexerConfig(") {
            let mut all = self.indexers.lock().unwrap();
            all.retain(|x| x["payload"]["id"] != v["id"]);
            return Ok(json!({"deleteIndexerConfig": {"id": v["id"]}}));
        }
        Err("the fake knows no such mutation".into())
    }

    fn data(&self, op: &str, query: &str, v: &Value) -> Result<Value, String> {
        if query.contains("__schema") {
            if !self.introspection.load(Ordering::SeqCst) {
                return Err("introspection is disabled".into());
            }
            return Ok(introspect(query));
        }
        if op == "PdtMutate" {
            return self.mutate(query, v);
        }
        if !op.starts_with("Pdt") {
            return Ok(self.query(query));
        }
        let mut titles = self.titles.lock().unwrap();
        Ok(match op {
            "PdtSearch" => match (v["type"].as_str(), v["query"].as_str()) {
                (Some("SERIES"), Some("severance")) => json!({"searchMetadata": [
                    {"tvdbId": "371980", "tmdbId": 95396, "imdbId": "tt11280740", "name": "Severance", "year": 2022,
                     "overview": "Work."},
                    {"tvdbId": "999", "name": "Severance: Making Of", "year": 2023}]}),
                _ => json!({"searchMetadata": []}),
            },
            "PdtSearchAll" => json!({"searchMetadataMulti": {"movies": [dune()], "series": [], "anime": []}}),
            "PdtTitlesByIds" => {
                let source = v["source"].as_str().unwrap();
                let values: Vec<&str> = v["values"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
                let found: Vec<Value> =
                    titles
                        .iter()
                        .filter(|t| {
                            t["externalIds"].as_array().unwrap().iter().any(|e| {
                                e["source"] == source && values.contains(&e["value"].as_str().unwrap_or_default())
                            })
                        })
                        .cloned()
                        .collect();
                json!({"titlesByExternalIds": found})
            }
            "PdtMovie" if v["input"]["tmdbId"] == 438631 => json!({"metadataMovie": dune()}),
            "PdtMovie" | "PdtSeries" => return Err("no metadata for that id".into()),
            "PdtLibraries" => match v["facet"].as_str() {
                Some("MOVIE") => json!({"libraries": [{"id": "L1", "name": "Films", "slug": "films"},
                                                      {"id": "L2", "name": "Kids", "slug": "kids"}]}),
                _ => json!({"libraries": [{"id": "L3", "name": "TV", "slug": "tv"}]}),
            },
            "PdtAddTitle" => {
                let i = &v["input"];
                let t = json!({"id": format!("t{}", titles.len() + 1), "name": i["name"], "year": 2021,
                               "facet": i["facet"], "monitored": i["monitored"],
                               "libraryName": if i["libraryId"] == "L2" { "Kids" } else { "Films" },
                               "externalIds": i["externalIds"], "input": i});
                titles.push(t.clone());
                json!({"addTitle": {"title": t, "reusedExistingTitle": false}})
            }
            "PdtQueue" => json!({"downloadQueuePage": {"totalCount": 3, "items": [
                {"titleName": "Severance", "progressPercent": 40, "displayState": "DOWNLOADING",
                 "remainingSeconds": 600, "clientName": "sabnzbd", "attentionRequired": false},
                {"titleName": "Dune", "progressPercent": 100, "displayState": "IMPORT_BLOCKED",
                 "clientName": "qbit", "attentionRequired": true, "attentionReason": "no matching file"}]}}),
            "PdtLibrary" => {
                let page: Vec<Value> = titles
                    .iter()
                    .filter(|t| v["facet"].is_null() || t["facet"] == v["facet"])
                    .filter(|t| v["filter"]["monitored"].is_null() || t["monitored"] == v["filter"]["monitored"])
                    .filter(|t| {
                        v["query"].as_str().is_none_or(|q| {
                            t["name"].as_str().unwrap_or_default().to_lowercase().contains(&q.to_lowercase())
                        })
                    })
                    .cloned()
                    .collect();
                json!({"titles": {"totalCount": page.len(), "hasMore": false, "items": page}})
            }
            other => return Err(format!("the fake knows no operation {other}")),
        })
    }

    fn answer(&self, path: &str, body: &Value, auth: &str) -> (StatusCode, Value) {
        if path != "/scryer/graphql" {
            return (StatusCode::NOT_FOUND, json!({}));
        }
        if auth != format!("Bearer {KEY}") {
            return (StatusCode::UNAUTHORIZED, json!({"errors": [{"message": "invalid API key"}]}));
        }
        let query = body["query"].as_str().unwrap_or_default();
        match self.data(&Self::operation(query), query, &body["variables"]) {
            Ok(d) => (StatusCode::OK, json!({"data": d})),
            Err(e) => (StatusCode::OK, json!({"data": null, "errors": [{"message": e}]})),
        }
    }

    /// Serves it; returns the connection's address (with the base path) and the fake.
    pub async fn serve() -> (String, FakeScryer) {
        use http_body_util::BodyExt;
        let fake = FakeScryer::default();
        fake.titles.lock().unwrap().push(severance());
        fake.introspection.store(true, Ordering::SeqCst);
        let f = fake.clone();
        let app = axum::Router::new().fallback(move |req: axum::http::Request<axum::body::Body>| {
            let f = f.clone();
            async move {
                let path = req.uri().path().to_string();
                let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                let bytes = req.into_body().collect().await.unwrap().to_bytes();
                let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                let (status, v) = f.answer(&path, &body, &auth);
                let op = Self::operation(body["query"].as_str().unwrap_or_default());
                f.seen.lock().unwrap().push((path, op, body["variables"].clone(), auth));
                (status, axum::Json(v))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/scryer"), fake)
    }

    /// The operations it was sent, in order.
    pub fn ops(&self) -> Vec<String> {
        self.seen.lock().unwrap().iter().map(|s| s.1.clone()).collect()
    }

    pub fn title(&self, name: &str) -> Option<Value> {
        self.titles.lock().unwrap().iter().find(|t| t["name"] == name).cloned()
    }
}
