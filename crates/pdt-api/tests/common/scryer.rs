//! A fake Scryer (PDT-147): its GraphQL endpoint under a base path (`/scryer/graphql`), answering the operations PDT
//! sends by name, with a library that starts with Severance. The API key it takes is [`KEY`].

use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub const KEY: &str = "ska_test-key";

/// One request: its path, operation name, variables and Authorization header.
pub type Seen = (String, String, Value, String);

#[derive(Clone, Default)]
pub struct FakeScryer {
    pub titles: Arc<Mutex<Vec<Value>>>,
    pub seen: Arc<Mutex<Vec<Seen>>>,
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

    fn data(&self, op: &str, v: &Value) -> Result<Value, String> {
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
        let op = Self::operation(body["query"].as_str().unwrap_or_default());
        match self.data(&op, &body["variables"]) {
            Ok(d) => (StatusCode::OK, json!({"data": d})),
            Err(e) => (StatusCode::OK, json!({"data": null, "errors": [{"message": e}]})),
        }
    }

    /// Serves it; returns the connection's address (with the base path) and the fake.
    pub async fn serve() -> (String, FakeScryer) {
        use http_body_util::BodyExt;
        let fake = FakeScryer::default();
        fake.titles.lock().unwrap().push(severance());
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
