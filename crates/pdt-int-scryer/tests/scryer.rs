//! Scryer through the API (PDT-147, PDT-150): its actions, any query or mutation checked against its schema, and dry
//! runs that call nothing. Scryer is a local fake server.

use crate::common;
use crate::fake_scryer;

use axum::http::StatusCode;
use common::call;
use common::workflows::*;
use serde_json::{Value, json};
use std::sync::Arc;

/// A PDT with only this integration enabled.
async fn setup() -> Env {
    common::workflows::setup(vec![Arc::new(pdt_int_scryer::Scryer)]).await
}

#[tokio::test]
async fn scryer_searches_adds_once_and_reads_its_queue_and_library() {
    use fake_scryer::{FakeScryer, KEY};
    let (url, scryer) = FakeScryer::serve().await;
    let env = setup().await;
    connection(&env, "media", "scryer", &url, json!({}), Some(KEY)).await;

    // A series search says which result the library has.
    let wf = create(
        &env,
        manual_then(json!({"id": "find", "kind": "scryer.search", "version": 1,
            "config": {"connection": "media", "term": "{{trigger.term}}", "facet": "series"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"term": "severance"})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let results = &step(&r, "find")["output"]["results"];
    assert_eq!((&results[0]["tvdbId"], &results[0]["tmdbId"]), (&json!("371980"), &json!(95396)));
    assert_eq!((&results[0]["inLibrary"], &results[0]["titleId"]), (&json!(true), &json!("t1")));
    assert_eq!(results[1]["inLibrary"], false);
    let seen = scryer.seen.lock().unwrap().clone();
    assert!(seen.iter().all(|(path, _, _, auth)| path == "/scryer/graphql" && *auth == format!("Bearer {KEY}")));

    // A new film by its TMDB id: named by Scryer's metadata, put in the named library, monitored.
    let add = |config: Value| manual_then(json!({"id": "add", "kind": "scryer.add", "version": 1, "config": config}));
    let wf = create(
        &env,
        add(json!({"connection": "media", "facet": "movie", "tmdbId": "{{trigger.id}}", "library": "kids"})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"id": 438631})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "add")["port"], "added");
    assert_eq!(
        (&step(&r, "add")["output"]["name"], &step(&r, "add")["output"]["library"]),
        (&json!("Dune"), &json!("Kids"))
    );
    let input = scryer.title("Dune").unwrap()["input"].clone();
    assert_eq!(
        (&input["facet"], &input["libraryId"], &input["monitored"]),
        (&json!("MOVIE"), &json!("L2"), &json!(true))
    );
    assert_eq!((&input["tmdbId"], &input["imdbId"], &input["smgId"]), (&json!(438631), &json!("tt1160419"), &json!(7)));
    assert_eq!(input["options"], json!({"monitorType": "ALL_EPISODES"}));

    // Again: it is there now, so `exists` and nothing added.
    let r = run_now(&env, &wf, json!({"id": 438631})).await;
    assert_eq!(step(&r, "add")["port"], "exists", "{r}");
    assert_eq!(scryer.ops().iter().filter(|o| *o == "PdtAddTitle").count(), 1);

    // An id Scryer doesn't know, or a library it doesn't have, fails; a missing id fails validation.
    let r = run_now(&env, &wf, json!({"id": 1})).await;
    assert!(r["error"].as_str().unwrap().contains("no metadata for that id"), "{r}");
    let wf =
        create(&env, add(json!({"connection": "media", "facet": "movie", "tmdbId": 693134, "library": "4K"}))).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("no movie library \"4K\" (libraries: Films, Kids)"), "{r}");
    let doc = add(json!({"connection": "media", "facet": "series"}));
    let (_, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": doc}))).await;
    assert!(v.to_string().contains("tvdbId, tmdbId or imdbId"), "{v}");

    let wf = create(
        &env,
        manual_then(json!({"id": "q", "kind": "scryer.queue", "version": 1, "config": {"connection": "media"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    let out = &step(&r, "q")["output"];
    assert_eq!(
        (&out["total"], &out["items"][0]["status"], &out["items"][0]["secondsLeft"]),
        (&json!(3), &json!("downloading"), &json!(600))
    );
    assert_eq!(
        (&out["items"][1]["status"], &out["items"][1]["attention"]),
        (&json!("importBlocked"), &json!("no matching file"))
    );

    let wf = create(
        &env,
        manual_then(json!({"id": "lib", "kind": "scryer.library", "version": 1,
            "config": {"connection": "media", "facet": "movie", "monitored": true}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    let out = &step(&r, "lib")["output"];
    assert_eq!((&out["total"], &out["titles"][0]["name"], &out["hasMore"]), (&json!(1), &json!("Dune"), &json!(false)));

    // A wrong key says so.
    connection(&env, "bad", "scryer", &url, json!({}), Some("ska_wrong")).await;
    let wf = create(
        &env,
        manual_then(json!({"id": "q", "kind": "scryer.queue", "version": 1, "config": {"connection": "bad"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("(the API key is wrong)"), "{r}");
}

/// Any query or mutation (PDT-150), checked against the bundled schema when Scryer doesn't answer introspection, with
/// a connection's secret filled in and credentials redacted from the output.
#[tokio::test]
async fn scryer_runs_any_query_or_mutation_checked_against_its_schema() {
    use fake_scryer::{FakeScryer, KEY};
    let (url, scryer) = FakeScryer::serve().await;
    scryer.introspection.store(false, std::sync::atomic::Ordering::SeqCst);
    let env = setup().await;
    connection(&env, "media", "scryer", &url, json!({}), Some(KEY)).await;
    connection(&env, "sab", "http", "http://nas:8080", json!({}), Some("sab-pass-456")).await;

    let query = |q: &str| {
        manual_then(
            json!({"id": "q", "kind": "scryer.query", "version": 1, "config": {"connection": "media", "query": q}}),
        )
    };
    let wf = create(&env, query("query Settings { generalSettings { historyRetentionDays } }")).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "q")["output"]["data"]["generalSettings"]["historyRetentionDays"], 30);
    // The bundled schema doesn't have what only a newer Scryer has: nothing is sent.
    let n = scryer.seen.lock().unwrap().len();
    let wf = create(&env, query("query { fakeVersion }")).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("fakeVersion"), "{r}");
    let ops = scryer.ops();
    assert!(ops[n..].iter().all(|o| o == "PdtIntrospection"), "only the schema was asked for: {ops:?}");

    // A mutation with a password by reference.
    let wf = create(
        &env,
        manual_then(json!({"id": "m", "kind": "scryer.mutate", "version": 1, "config": {"connection": "media",
            "mutation": "createDownloadClientConfig", "args": {"input": {"name": "{{trigger.name}}",
            "clientType": "sabnzbd", "config": [{"key": "password", "secretValue": {"$secret": "sab"}}]}},
            "select": "id name config { key defaultValue }"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"name": "SABnzbd"})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let out = &step(&r, "m")["output"];
    assert_eq!((&out["mutation"], &out["result"]["name"]), (&json!("createDownloadClientConfig"), &json!("SABnzbd")));
    assert_eq!(out["result"]["config"][0]["defaultValue"], "[redacted]", "{out}");
    assert_eq!(scryer.clients.lock().unwrap()[0]["input"]["config"][0]["secretValue"], "sab-pass-456");
    assert!(!r.to_string().contains("sab-pass-456"), "{r}");

    // A reference to a connection that isn't there fails before anything is sent.
    let wf = create(
        &env,
        manual_then(json!({"id": "m", "kind": "scryer.mutate", "version": 1, "config": {"connection": "media",
            "mutation": "createIndexerConfig", "args": {"input": {"name": "x", "providerType": "newznab",
            "config": [{"key": "apiKey", "secretValue": {"$secret": "nope"}}]}}}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("no connection is called nope"), "{r}");
    assert!(scryer.indexers.lock().unwrap().is_empty());

    // Validation: a query node runs no mutations, and signing in is never sent.
    for (node, says) in [
        (
            json!({"kind": "scryer.query", "config": {"connection": "media", "query": "mutation { triggerRssSync { __typename } }"}}),
            "queries only",
        ),
        (json!({"kind": "scryer.query", "config": {"connection": "media", "query": "query { "}}), "doesn't parse"),
        (
            json!({"kind": "scryer.mutate", "config": {"connection": "media", "mutation": "loginWithPlex"}}),
            "never sends",
        ),
    ] {
        let mut node = node;
        node["id"] = json!("x");
        node["version"] = json!(1);
        let (_, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": manual_then(node)}))).await;
        assert!(v.to_string().contains(says), "{v}");
    }
}

#[tokio::test]
async fn scryer_dry_runs_call_nothing() {
    let (url, seen) = fake_server(|_, _| (200, json!({}))).await;
    let env = setup().await;
    connection(&env, "films", "scryer", &url, json!({}), Some("k")).await;
    for node in [
        json!({"id": "a", "kind": "scryer.add", "version": 1,
               "config": {"connection": "films", "facet": "movie", "tmdbId": 693134}}),
        json!({"id": "a", "kind": "scryer.library", "version": 1, "config": {"connection": "films"}}),
        json!({"id": "a", "kind": "scryer.search", "version": 1, "config": {"connection": "films", "term": "dune"}}),
        json!({"id": "a", "kind": "scryer.queue", "version": 1, "config": {"connection": "films"}}),
        json!({"id": "a", "kind": "scryer.query", "version": 1,
               "config": {"connection": "films", "query": "query { indexers { id } }"}}),
        json!({"id": "a", "kind": "scryer.mutate", "version": 1,
               "config": {"connection": "films", "mutation": "deleteIndexerConfig", "args": {"id": "1"}}}),
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
