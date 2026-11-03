//! Radarr and Sonarr through the API: search, add once, the queue, and dry runs that call nothing.
//! Both are local fake servers.

#[path = "../../pdt-api/tests/common/mod.rs"]
mod common;

use axum::http::StatusCode;
use common::call;
use common::workflows::*;
use serde_json::{Value, json};
use std::sync::Arc;

/// A PDT with only this integration enabled.
async fn setup() -> Env {
    common::workflows::setup(vec![Arc::new(pdt_int_arr::Arr)]).await
}

fn radarr_answer(path: &str, _: usize) -> (u16, Value) {
    match path {
        p if p.starts_with("/api/v3/movie/lookup/tmdb?tmdbId=693134") => {
            (200, json!({"title": "Dune: Part Two", "year": 2024, "tmdbId": 693134, "images": []}))
        }
        p if p.starts_with("/api/v3/movie/lookup?term=dune") => (
            200,
            json!([{"title": "Dune", "year": 2021, "tmdbId": 438631, "id": 5, "overview": "Spice."},
                {"title": "Dune: Part Two", "year": 2024, "tmdbId": 693134}]),
        ),
        // In the library already.
        "/api/v3/movie?tmdbId=438631" => (200, json!([{"id": 5, "title": "Dune", "year": 2021, "tmdbId": 438631}])),
        // An older Radarr that ignores the filter and lists the whole library.
        p if p.starts_with("/api/v3/movie?tmdbId=") => (200, json!([{"id": 9, "title": "Other", "tmdbId": 1}])),
        "/api/v3/movie" => (201, json!({"id": 10, "title": "Dune: Part Two", "year": 2024, "tmdbId": 693134})),
        "/api/v3/qualityprofile" => (200, json!([{"id": 1, "name": "Any"}, {"id": 4, "name": "HD-1080p"}])),
        "/api/v3/rootfolder" => (200, json!([{"id": 1, "path": "/movies"}])),
        p if p.starts_with("/api/v3/queue?") => (
            200,
            json!({"page": 1, "records": [{"title": "Dune.Part.Two.2160p", "size": 200.0, "sizeleft": 50.0,
                "timeleft": "00:05:00", "estimatedCompletionTime": "2026-10-22T10:00:00Z",
                "trackedDownloadState": "downloading", "movie": {"title": "Dune: Part Two", "year": 2024}}]}),
        ),
        _ => (404, json!({"message": "not found"})),
    }
}

#[tokio::test]
async fn radarr_searches_adds_once_and_reads_its_queue() {
    let (url, seen) = fake_server(radarr_answer).await;
    let env = setup().await;
    connection(&env, "films", "radarr", &url, json!({}), Some("radarr-key")).await;

    let wf = create(
        &env,
        manual_then(json!({"id": "find", "kind": "radarr.search", "version": 1,
            "config": {"connection": "films", "term": "{{trigger.term}}"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"term": "dune"})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let results = &step(&r, "find")["output"]["results"];
    assert_eq!(results[0]["tmdbId"], 438631);
    assert_eq!((&results[0]["inLibrary"], &results[1]["inLibrary"]), (&json!(true), &json!(false)));
    assert!(seen.all().iter().all(|(_, _, _, key)| key.as_deref() == Some("radarr-key")), "the API key goes with each");

    // A new film: looked up, given the named profile and the first root folder, searched for.
    let add = |id: Value| {
        manual_then(json!({"id": "add", "kind": "radarr.add", "version": 1,
            "config": {"connection": "films", "tmdbId": id, "qualityProfile": "hd-1080p"}}))
    };
    let wf = create(&env, add(json!("{{trigger.id}}"))).await;
    let r = run_now(&env, &wf, json!({"id": 693134})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "add")["port"], "added");
    assert_eq!(step(&r, "add")["output"]["id"], 10);
    let (method, _, body, _) = seen.all().into_iter().find(|(m, _, _, _)| m == "POST").unwrap();
    assert_eq!(method, "POST");
    assert_eq!(
        (&body["tmdbId"], &body["qualityProfileId"], &body["rootFolderPath"]),
        (&json!(693134), &json!(4), &json!("/movies"))
    );
    assert_eq!((&body["monitored"], &body["addOptions"]["searchForMovie"]), (&json!(true), &json!(true)));
    assert_eq!(body["minimumAvailability"], "released");

    // One already there takes `exists` and adds nothing.
    let posts = seen.all().iter().filter(|(m, _, _, _)| m == "POST").count();
    let r = run_now(&env, &wf, json!({"id": 438631})).await;
    assert_eq!((step(&r, "add")["port"].as_str(), &step(&r, "add")["output"]["id"]), (Some("exists"), &json!(5)));
    assert_eq!(seen.all().iter().filter(|(m, _, _, _)| m == "POST").count(), posts);

    // An unknown profile is an error; a bad id fails validation.
    let wf = create(
        &env,
        manual_then(json!({"id": "add", "kind": "radarr.add", "version": 1,
            "config": {"connection": "films", "tmdbId": 693134, "qualityProfile": "4K"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("no quality profile \"4K\""), "{r}");
    let (_, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": add(json!("dune"))}))).await;
    assert!(v.to_string().contains("is not an id"), "{v}");

    let wf = create(
        &env,
        manual_then(json!({"id": "q", "kind": "radarr.queue", "version": 1, "config": {"connection": "films"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    let item = &step(&r, "q")["output"]["items"][0];
    assert_eq!(
        (&item["title"], &item["progress"], &item["status"]),
        (&json!("Dune: Part Two (2024)"), &json!(75.0), &json!("downloading"))
    );
}

fn sonarr_answer(path: &str, _: usize) -> (u16, Value) {
    match path {
        p if p.starts_with("/api/v3/series/lookup?term=tvdb%3A371980") => {
            (200, json!([{"title": "Severance", "year": 2022, "tvdbId": 371980, "seasons": []}]))
        }
        p if p.starts_with("/api/v3/series/lookup?term=") => {
            (200, json!([{"title": "Severance", "year": 2022, "tvdbId": 371980}]))
        }
        p if p.starts_with("/api/v3/series?tvdbId=") => (200, json!([])),
        "/api/v3/series" => (201, json!({"id": 3, "title": "Severance", "year": 2022, "tvdbId": 371980})),
        "/api/v3/qualityprofile" => (200, json!([{"id": 6, "name": "WEB-1080p"}])),
        p if p.starts_with("/api/v3/queue?") => (
            200,
            json!({"records": [{"title": "Severance.S02E03", "size": 10.0, "sizeleft": 10.0, "status": "queued",
                "series": {"title": "Severance"}, "episode": {"seasonNumber": 2, "episodeNumber": 3}}]}),
        ),
        _ => (404, json!({"message": "not found"})),
    }
}

#[tokio::test]
async fn sonarr_adds_a_series_with_the_connections_defaults() {
    let (url, seen) = fake_server(sonarr_answer).await;
    let env = setup().await;
    connection(&env, "tv", "sonarr", &url, json!({"rootFolder": "/data/tv", "qualityProfile": 6}), Some("sonarr-key"))
        .await;
    let wf = create(
        &env,
        manual_then(json!({"id": "add", "kind": "sonarr.add", "version": 1,
            "config": {"connection": "tv", "tvdbId": 371980, "monitor": "future", "search": false}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!((r["status"].as_str(), step(&r, "add")["port"].as_str()), (Some("succeeded"), Some("added")), "{r}");
    assert_eq!(step(&r, "add")["output"]["tvdbId"], 371980);
    let (_, _, body, key) = seen.all().into_iter().find(|(m, _, _, _)| m == "POST").unwrap();
    assert_eq!(key.as_deref(), Some("sonarr-key"));
    assert_eq!((&body["qualityProfileId"], &body["rootFolderPath"]), (&json!(6), &json!("/data/tv")));
    assert_eq!(body["addOptions"], json!({"monitor": "future", "searchForMissingEpisodes": false}));
    assert!(!seen.all().iter().any(|(_, p, _, _)| p == "/api/v3/rootfolder"), "the connection's folder is used");

    let wf = create(
        &env,
        manual_then(json!({"id": "q", "kind": "sonarr.queue", "version": 1, "config": {"connection": "tv"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "q")["output"]["items"][0]["title"], "Severance S02E03");

    // A monitor option Sonarr doesn't have fails validation; a Radarr kind refuses a Sonarr connection.
    let doc = manual_then(json!({"id": "add", "kind": "sonarr.add", "version": 1,
        "config": {"connection": "tv", "tvdbId": 1, "monitor": "sometimes"}}));
    let (_, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": doc}))).await;
    assert!(v.to_string().contains("monitor \\\"sometimes\\\""), "{v}");
    let wf = create(
        &env,
        manual_then(json!({"id": "q", "kind": "radarr.queue", "version": 1, "config": {"connection": "tv"}})),
    )
    .await;
    assert!(
        run_now(&env, &wf, json!({})).await["error"].as_str().unwrap().contains("is a sonarr connection, not radarr")
    );
}

#[tokio::test]
async fn arr_dry_runs_call_nothing() {
    let (url, seen) = fake_server(radarr_answer).await;
    let env = setup().await;
    connection(&env, "films", "radarr", &url, json!({}), Some("k")).await;
    for node in [
        json!({"id": "a", "kind": "radarr.add", "version": 1, "config": {"connection": "films", "tmdbId": 693134}}),
        json!({"id": "a", "kind": "radarr.search", "version": 1, "config": {"connection": "films", "term": "x"}}),
    ] {
        let doc = manual_then(node);
        let (s, d) =
            call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"doc": doc, "event": {"type": "manual"}})))
                .await;
        assert_eq!(s, StatusCode::OK, "{d}");
        assert!(d["run"]["steps"][1]["output"]["would"].is_string(), "{d}");
    }
    assert!(seen.all().is_empty());
}
