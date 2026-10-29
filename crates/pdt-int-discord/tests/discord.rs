//! Discord through the API (PDT-98, Phase 7e): `discord.post` (split messages, rate limits, restarts, its
//! secret kept out of errors and logs), `discord.channel.ensure`, the templates that post to Discord, and the PM's
//! Discord tools. Discord is a local fake server.

#[path = "../../pdt-api/tests/common/mod.rs"]
mod common;

use axum::http::StatusCode;
use common::call;
use common::workflows::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// A PDT with only this integration enabled.
async fn setup() -> Env {
    common::workflows::setup(vec![Arc::new(pdt_int_discord::Discord)]).await
}

#[tokio::test]
async fn discord_posts_split_long_messages_and_wait_out_a_rate_limit() {
    let (url, seen) =
        fake_server(
            |_, n| {
                if n == 1 { (429, json!({"retry_after": 0.05})) } else { (200, json!({"id": format!("m{n}")})) }
            },
        )
        .await;
    let env = setup().await;
    connection(&env, "alerts", "discord_webhook", "", json!({}), Some(&format!("{url}/api/webhooks/1/abc"))).await;
    let long = "line of news\n".repeat(200);
    let wf = create(
        &env,
        manual_then(json!({"id": "post", "kind": "discord.post", "version": 1, "config": {
            "connection": "alerts", "content": "{{trigger.text}}", "username": "PDT"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"text": long})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let all = seen.all();
    assert_eq!(all.len(), 3, "a rate-limited try, then two messages: {all:?}");
    assert!(all.iter().all(|(m, p, _, _)| m == "POST" && p.starts_with("/api/webhooks/1/abc?wait=true")));
    assert!(all.iter().all(|(_, _, b, _)| b["content"].as_str().unwrap().chars().count() <= 2000));
    assert_eq!(all[1].2["username"], "PDT");
    assert_eq!(step(&r, "post")["output"]["messageIds"], json!(["m2", "m3"]));

    // A connection of another kind is refused.
    connection(&env, "plain", "http", &url, json!({}), Some("x")).await;
    let wf = create(
        &env,
        manual_then(json!({"id": "post", "kind": "discord.post", "version": 1,
            "config": {"connection": "plain", "content": "hi"}})),
    )
    .await;
    assert!(run_now(&env, &wf, json!({})).await["error"].as_str().unwrap().contains("not discord_webhook"));
}

#[tokio::test]
async fn a_restart_mid_discord_post_posts_each_piece_once() {
    let (url, seen) = hanging_server(2).await;
    let mut env = setup().await;
    connection(&env, "alerts", "discord_webhook", "", json!({}), Some(&format!("{url}/api/webhooks/1/abc"))).await;
    let wf = create(
        &env,
        manual_then(json!({"id": "post", "kind": "discord.post", "version": 1, "retry": {"max": 1},
            "config": {"connection": "alerts", "content": "{{trigger.text}}"}})),
    )
    .await;
    let long = format!("{}\n{}", "a".repeat(1500), "b".repeat(1500));
    let id = start_slow(&mut env, &wf, json!({"text": long})).await;
    // The first piece went out and is recorded; the daemon stops while the second is on its way.
    let r = call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1;
    assert_eq!(step(&r, "post")["status"], "running", "{r}");
    assert_eq!(step(&r, "post")["output"]["messageIds"], json!(["m1"]));

    let (state, app) = restarted(&env).await;
    pdt_api::run_workflows(&state).await;
    let r = call(&app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1;
    assert_eq!(r["status"], "succeeded", "{r}");
    let post = step(&r, "post");
    assert_eq!(post["output"]["messageIds"], json!(["m1", "m3"]));
    assert_eq!(post["attempts"], 1, "the restart doesn't use up a retry");
    let contents: Vec<String> =
        seen.all().iter().map(|(_, _, b, _)| b["content"].as_str().unwrap().chars().take(1).collect()).collect();
    assert_eq!(contents, ["a", "b"], "each piece once");
}

/// What `tracing` writes while the test's thread runs, at every level.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Log {
    fn capture(&self) -> tracing::subscriber::DefaultGuard {
        let log = self.clone();
        let sub = tracing_subscriber::fmt().with_max_level(tracing::Level::TRACE).with_writer(move || log.clone());
        tracing::subscriber::set_default(sub.finish())
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

#[tokio::test]
async fn a_failed_request_never_shows_the_connection_secret() {
    let log = Log::default();
    let _guard = log.capture();
    let env = setup().await;
    let closed = closed_address().await;

    // A Discord webhook's URL is its secret.
    let hook = format!("{closed}/api/webhooks/987654321012/s3cret-hook-token");
    connection(&env, "alerts", "discord_webhook", "", json!({}), Some(&hook)).await;
    let wf = create(
        &env,
        manual_then(json!({"id": "post", "kind": "discord.post", "version": 1,
            "config": {"connection": "alerts", "content": "hi"}})),
    )
    .await;
    // A callsite other tests' threads registered as this one set its subscriber may have cached "never": look again.
    tracing::callsite::rebuild_interest_cache();
    let discord = run_now(&env, &wf, json!({})).await;

    // A key sent in the query.
    connection(&env, "svc", "http", &closed, json!({"auth": "query", "param": "apikey"}), Some("s3cret-query-key"))
        .await;
    let wf = create(
        &env,
        manual_then(json!({"id": "get", "kind": "http.request", "version": 1,
            "config": {"connection": "svc", "url": "/items"}})),
    )
    .await;
    tracing::callsite::rebuild_interest_cache();
    let http = run_now(&env, &wf, json!({})).await;

    for (r, node) in [(&discord, "post"), (&http, "get")] {
        assert_eq!(r["status"], "failed", "{r}");
        let error = step(r, node)["error"].as_str().unwrap();
        assert!(!error.is_empty() && r["error"].as_str().is_some(), "{r}");
    }
    assert!(step(&discord, "post")["error"].as_str().unwrap().starts_with("Discord: "), "{discord}");
    assert!(step(&http, "get")["error"].as_str().unwrap().contains("/items"), "{http}");
    let log = log.text();
    assert!(log.contains("a workflow step failed"), "{log}");
    for secret in ["987654321012", "s3cret-hook-token", "s3cret-query-key"] {
        for (what, text) in [("discord run", discord.to_string()), ("http run", http.to_string()), ("log", log.clone())]
        {
            assert!(!text.contains(secret), "{secret} in the {what}: {text}");
        }
    }
}

#[tokio::test]
async fn a_discord_channel_is_made_only_when_missing() {
    // Requests in order: the first run lists the channels; the second lists them, makes the category, then the
    // channel.
    let (url, seen) = fake_server(|_, n| match n {
        1 | 2 => (200, json!([{"id": "1", "name": "alerts", "type": 0}])),
        3 => (201, json!({"id": "5", "name": "Bots", "type": 4})),
        _ => (201, json!({"id": "6", "name": "ai-briefs", "type": 0, "parent_id": "5"})),
    })
    .await;
    let env = setup().await;
    connection(&env, "home", "discord_bot", &url, json!({"guildId": "42"}), Some("tok")).await;
    let ensure = |name: &str| {
        manual_then(json!({"id": "ch", "kind": "discord.channel.ensure", "version": 1,
            "config": {"connection": "home", "name": name, "category": "Bots"}}))
    };
    let wf = create(&env, ensure("#alerts")).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "ch")["output"], json!({"channelId": "1", "name": "alerts", "created": false}), "{r}");
    let wf = create(&env, ensure("AI Briefs")).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "ch")["output"], json!({"channelId": "6", "name": "ai-briefs", "created": true}), "{r}");
    let all = seen.all();
    assert_eq!(all.len(), 4, "{all:?}");
    assert!(all.iter().all(|r| r.1 == "/guilds/42/channels" && r.3.as_deref() == Some("Bot tok")), "{all:?}");
    assert_eq!((all[2].2["name"].as_str(), all[2].2["type"].as_u64()), (Some("Bots"), Some(4)));
    assert_eq!(all[3].2, json!({"name": "ai-briefs", "type": 0, "parent_id": "5"}));
}

#[tokio::test]
async fn the_discord_templates_instantiate_and_dry_run() {
    let env = setup().await;
    let inst = |id: &str, params: Value| {
        let app = env.app.clone();
        let id = id.to_string();
        async move {
            let (s, w) = call(
                &app,
                "POST",
                &format!("/api/workflow-templates/builtin:{id}/instantiate"),
                Some(json!({"params": params, "scope": "personal"})),
            )
            .await;
            assert_eq!(s, StatusCode::CREATED, "{id}: {w}");
            assert_eq!(w["enabled"], false);
            w
        }
    };
    let w = inst("github-issues-discord", json!({"repo": "o/r", "discord": "alerts"})).await;
    let wid = w["id"].as_str().unwrap();
    let (_, d) = call(
        &env.app,
        "POST",
        "/api/workflows/dry-run",
        Some(json!({"workflowId": wid, "event": {"type": "github.issue.opened",
            "payload": {"repo": "o/r", "number": 9, "title": "Crash", "author": "al", "url": "https://i/9"}}})),
    )
    .await;
    let would = d["run"]["steps"][1]["output"]["would"].as_str().unwrap();
    assert!(would.contains("New issue in o/r #9: **Crash** by al"), "{would}");

    let (s, _) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:github-issues-discord/instantiate",
        Some(json!({"params": {"discord": "alerts"}})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "the repository is required");
    // PDT-226: the templates that weren't about development are gone.
    for id in ["health-watchdog", "brief-follow-ups", "email-processing"] {
        let (s, _) = call(&env.app, "GET", &format!("/api/workflow-templates/builtin:{id}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{id}");
    }
}
