//! Plex through the API: search, what's new, refresh, and dry runs that call nothing. Plex is a local
//! fake server.

#[path = "../../pdt-api/tests/common/mod.rs"]
mod common;

use axum::http::StatusCode;
use common::call;
use common::workflows::*;
use serde_json::{Value, json};
use std::sync::Arc;

/// A PDT with only this integration enabled.
async fn setup() -> Env {
    common::workflows::setup(vec![Arc::new(pdt_int_plex::Plex)]).await
}

fn plex_answer(path: &str, _: usize) -> (u16, Value) {
    match path {
        p if p.starts_with("/hub/search?query=dune") => (
            200,
            json!({"MediaContainer": {"Hub": [{"type": "movie", "Metadata": [
                {"title": "Dune", "type": "movie", "year": 2021, "ratingKey": "12", "librarySectionTitle": "Films"}]}]}}),
        ),
        "/library/sections" => (
            200,
            json!({"MediaContainer": {"Directory": [{"key": "1", "title": "Films"}, {"key": "2", "title": "TV"}]}}),
        ),
        p if p.starts_with("/library/sections/2/recentlyAdded?") => (
            200,
            json!({"MediaContainer": {"Metadata": [
                {"title": "Pilot", "type": "episode", "ratingKey": "50", "grandparentTitle": "Severance", "addedAt": 1},
                {"title": "Two", "type": "episode", "ratingKey": "51", "grandparentTitle": "Severance", "addedAt": 2}]}}),
        ),
        "/library/sections/all/refresh" => (200, Value::Null),
        _ => (404, json!({})),
    }
}

#[tokio::test]
async fn plex_searches_lists_whats_new_and_refreshes() {
    let (url, seen) = fake_server(plex_answer).await;
    let env = setup().await;
    connection(&env, "home", "plex", &url, json!({}), Some("plex-token")).await;
    let doc = json!({"name": "Plex", "nodes": [
        {"id": "go", "kind": "manual", "version": 1},
        {"id": "find", "kind": "plex.search", "version": 1, "config": {"connection": "home", "query": "dune"}},
        {"id": "new", "kind": "plex.recently_added", "version": 1,
            "config": {"connection": "home", "section": "tv", "limit": 1}},
        {"id": "scan", "kind": "plex.refresh", "version": 1, "config": {"connection": "home"}}],
        "edges": [{"from": "go", "to": "find"}, {"from": "find", "to": "new"}, {"from": "new", "to": "scan"}]});
    let wf = create(&env, doc).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "find")["output"]["items"][0]["ratingKey"], "12");
    let new = step(&r, "new")["output"]["items"].as_array().unwrap().clone();
    assert_eq!((new.len(), &new[0]["show"]), (1, &json!("Severance")));
    assert_eq!(step(&r, "scan")["output"]["section"], "all");
    let all = seen.all();
    assert!(all.iter().all(|(_, _, _, key)| key.as_deref() == Some("plex-token")));
    assert!(all.iter().any(|(_, p, _, _)| p.contains("X-Plex-Container-Size=1")), "{all:?}");
}

#[tokio::test]
async fn plex_dry_runs_call_nothing() {
    let (url, seen) = fake_server(plex_answer).await;
    let env = setup().await;
    connection(&env, "films", "plex", &url, json!({}), Some("k")).await;
    for node in [json!({"id": "a", "kind": "plex.refresh", "version": 1, "config": {"connection": "films"}})] {
        let doc = manual_then(node);
        let (s, d) =
            call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"doc": doc, "event": {"type": "manual"}})))
                .await;
        assert_eq!(s, StatusCode::OK, "{d}");
        assert!(d["run"]["steps"][1]["output"]["would"].is_string(), "{d}");
    }
    assert!(seen.all().is_empty());
}
