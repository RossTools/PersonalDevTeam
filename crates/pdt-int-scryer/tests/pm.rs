//! The PM's Scryer tools end to end (PDT-147, PDT-150), with `fake-agent` standing in for Claude and a fake Scryer:
//! browsing and adding after a yes, and setting Scryer up through its GraphQL API with credentials kept out of what
//! PDT stores.

use crate::common;
use crate::fake_scryer;

use axum::http::StatusCode;
use common::call;
use common::pm::*;
use serde_json::{Value, json};
use std::sync::Arc;

/// The PM's project (`common::pm::setup`) with only this integration enabled.
async fn setup() -> Env {
    common::pm::setup_with(|st| {
        st.integrations = Arc::new(pdt_api::Integrations::new(vec![Arc::new(pdt_int_scryer::Scryer)]));
        st.integrations.enable(&["scryer".into()]);
    })
    .await
}

#[tokio::test]
async fn the_pm_browses_scryer_and_adds_a_title_after_a_yes() {
    use fake_scryer::{FakeScryer, KEY};
    let env = setup().await;
    let (base, scryer) = FakeScryer::serve().await;
    let (s, c) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": "media", "kind": "scryer", "url": base, "config": {}})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let conn_id = c["id"].as_str().unwrap().to_string();
    call(&env.app, "PUT", &format!("/api/connections/{conn_id}/secret"), Some(json!({"secret": KEY}))).await;

    let id = start_pm(&env, "hi").await;
    wait_said(&env, &id, 1).await;
    assert!(instructions(&env, &id).contains("`scryer_add`"));

    // Browsing, searching and the queue need no yes.
    let said = ask(&env, &id, r#"mcp: scryer_library {"facet": "series"}"#).await;
    assert!(said.contains("\"name\": \"Severance\"") && said.contains("\"episodesOwned\": 9"), "{said}");
    let said = ask(&env, &id, r#"mcp: scryer_search {"term": "dune"}"#).await;
    assert!(said.contains("\"tmdbId\": 438631") && said.contains("\"inLibrary\": false"), "{said}");
    let said = ask(&env, &id, "mcp: scryer_queue {}").await;
    assert!(said.contains("\"status\": \"downloading\""), "{said}");

    // Adding asks first, and the user sees what is added.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: scryer_add {"facet": "movie", "tmdbId": 438631, "name": "Dune"}"#).await;
    let req = open_request(&env, &id).await;
    let title = events(&env, &id)
        .await
        .iter()
        .find(|e| e["event"]["kind"] == "request")
        .map(|e| e["event"]["title"].as_str().unwrap_or_default().to_string())
        .unwrap();
    assert_eq!(title, "Scryer: add the movie Dune (tmdb 438631), monitored");
    assert!(scryer.title("Dune").is_none(), "nothing before the yes");
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("\"added\": true"), "{said}");
    assert!(!said.contains(KEY), "the API key never reaches the agent: {said}");
    assert_eq!(scryer.title("Dune").unwrap()["monitored"], true);

    // A connection that lets the PM act: PDT says yes for the user; a title already there is left alone.
    let (s, v) =
        call(&env.app, "PATCH", &format!("/api/connections/{conn_id}"), Some(json!({"config": {"pmActs": true}})))
            .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let said =
        ask(&env, &id, r#"mcp: scryer_add {"connection": "media", "facet": "series", "tvdbId": "371980"}"#).await;
    assert!(said.contains("\"alreadyInLibrary\": true"), "{said}");
    assert_eq!(scryer.ops().iter().filter(|o| *o == "PdtAddTitle").count(), 1);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// The PM sets up an indexer and a download client through Scryer's own API (PDT-150): it finds the operations,
/// reads, and changes by asking, with the credentials kept out of the transcript.
#[tokio::test]
async fn the_pm_sets_up_an_indexer_and_a_download_client_through_scryers_api() {
    use fake_scryer::{FakeScryer, KEY};
    let env = setup().await;
    let (base, scryer) = FakeScryer::serve().await;
    let conn = |name: &'static str, kind: &'static str, url: String, secret: &'static str| {
        let app = env.app.clone();
        async move {
            let (s, c) = call(
                &app,
                "POST",
                "/api/connections",
                Some(json!({"name": name, "kind": kind, "url": url,
                                                                            "config": {"pmActs": true}})),
            )
            .await;
            assert_eq!(s, StatusCode::CREATED, "{c}");
            let id = c["id"].as_str().unwrap().to_string();
            call(&app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": secret}))).await;
        }
    };
    // The connection lets the PM act; the indexer's API key is kept as a connection's secret.
    conn("media", "scryer", base, KEY).await;
    conn("nzbgeek", "http", "https://api.nzbgeek.info".into(), "nzb-api-key-123").await;

    let id = start_pm(&env, "hi").await;
    wait_said(&env, &id, 1).await;
    assert!(instructions(&env, &id).contains("`scryer_mutate`"));

    // It finds the operations, in Scryer's own schema.
    let said = ask(&env, &id, r#"mcp: scryer_schema {"search": "indexer config"}"#).await;
    assert!(said.contains("\"createIndexerConfig\""), "{said}");
    let said = ask(&env, &id, r#"mcp: scryer_schema {"operation": "createIndexerConfig"}"#).await;
    assert!(said.contains("CreateIndexerConfigInput!") && said.contains("ProviderConfigValueInput"), "{said}");
    let said = ask(
        &env,
        &id,
        r#"mcp: scryer_query {"query": "query { fakeVersion generalSettings { historyRetentionDays } }"}"#,
    )
    .await;
    assert!(said.contains("\"fakeVersion\": \"2.0\"") && said.contains("\"historyRetentionDays\": 30"), "{said}");
    // A document that doesn't fit the schema, or changes something, is never sent.
    let n = scryer.seen.lock().unwrap().len();
    let said = ask(&env, &id, r#"mcp: scryer_query {"query": "query { indexers { nope } }"}"#).await;
    assert!(said.contains("nope"), "{said}");
    let said = ask(&env, &id, r#"mcp: scryer_query {"query": "mutation { triggerRssSync { __typename } }"}"#).await;
    assert!(said.contains("scryer_mutate"), "{said}");
    assert_eq!(scryer.seen.lock().unwrap().len(), n, "nothing sent");

    // An indexer with its API key by reference: a call that sends a secret asks even though the PM may act.
    let call_text = r#"mcp: scryer_mutate {"mutation": "createIndexerConfig", "args": {"input": {"name": "NZBgeek", "providerType": "newznab", "isEnabled": true, "config": [{"key": "baseUrl", "stringValue": "https://api.nzbgeek.info"}, {"key": "apiKey", "secretValue": {"$secret": "nzbgeek"}}]}}, "select": "id name hasApiKey"}"#;
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, call_text).await;
    let req = open_request(&env, &id).await;
    let title = events(&env, &id)
        .await
        .iter()
        .rev()
        .find(|e| e["event"]["kind"] == "request")
        .map(|e| e["event"]["title"].as_str().unwrap_or_default().to_string())
        .unwrap();
    assert_eq!(title, "Scryer: createIndexerConfig name=NZBgeek, config=[2 items], isEnabled, providerType=newznab");
    assert!(scryer.indexers.lock().unwrap().is_empty(), "nothing before the yes");
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("\"hasApiKey\": true") && said.contains("\"mutation\": \"createIndexerConfig\""), "{said}");
    let sent = scryer.indexers.lock().unwrap()[0]["input"].clone();
    assert_eq!(sent["config"][1]["secretValue"], "nzb-api-key-123", "PDT filled the secret in");

    // A download client with a password written out: no secret reference, so the connection lets the PM act; the
    // password is redacted from the transcript and from what Scryer echoes.
    let said = ask(
        &env,
        &id,
        r#"mcp: scryer_mutate {"mutation": "createDownloadClientConfig", "args": {"input": {"name": "SABnzbd", "clientType": "sabnzbd", "config": [{"key": "host", "stringValue": "nas"}, {"key": "password", "secretValue": "literal-pass-789"}]}}}"#,
    )
    .await;
    assert!(said.contains("\"name\": \"SABnzbd\"") && said.contains("[redacted]"), "{said}");
    assert!(!said.contains("echoed-by-a-bad-scryer"), "{said}");
    assert_eq!(scryer.clients.lock().unwrap()[0]["input"]["config"][1]["secretValue"], "literal-pass-789");
    // (The fake agent is told what to call in a user message, which has it; a real one writes the call itself.)
    let calls: Vec<Value> = events(&env, &id)
        .await
        .into_iter()
        .filter(|e| e["event"]["kind"] == "tool_call" || e["event"]["kind"] == "request")
        .collect();
    let log = serde_json::to_string(&calls).unwrap();
    assert!(log.contains("createDownloadClientConfig"), "{log}");
    let mut raw = String::new();
    for e in &calls {
        let seq = e["seq"].as_i64().unwrap();
        raw.push_str(&env.state.store.session_event_raw(&id, seq).await.unwrap().unwrap_or_default());
    }
    assert!(raw.contains("createDownloadClientConfig"), "the raw lines are kept: {raw}");
    for secret in ["literal-pass-789", "nzb-api-key-123", KEY] {
        assert!(!log.contains(secret), "{secret} is in the transcript");
        assert!(!raw.contains(secret), "{secret} is in a raw line");
    }

    // Both are there; deleting always asks.
    let said = ask(
        &env,
        &id,
        r#"mcp: scryer_query {"query": "query { indexers { id name } downloadClientConfigs { id name } }"}"#,
    )
    .await;
    assert!(said.contains("NZBgeek") && said.contains("SABnzbd"), "{said}");
    send(&env, &id, r#"mcp: scryer_mutate {"mutation": "deleteIndexerConfig", "args": {"id": "i1"}}"#).await;
    open_request(&env, &id).await;
    assert_eq!(scryer.indexers.lock().unwrap().len(), 1, "nothing deleted before the yes");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}
