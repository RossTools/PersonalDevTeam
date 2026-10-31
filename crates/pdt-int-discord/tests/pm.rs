//! The PM's Discord tools end to end (PDT-98), with `fake-agent` standing in for Claude and a fake Discord: setting
//! up a channel and its webhook after a yes, which a workflow then posts to.

use crate::common;

use axum::http::StatusCode;
use common::call;
use common::pm::*;
use serde_json::{Value, json};
use std::sync::Arc;

/// The PM's project (`common::pm::setup`) with only this integration enabled.
async fn setup(update_every_ms: Option<i64>) -> Env {
    common::pm::setup_with(update_every_ms, |st| {
        st.integrations = Arc::new(pdt_api::Integrations::new(vec![Arc::new(pdt_int_discord::Discord)]));
        st.integrations.enable(&["discord".into()]);
    })
    .await
}

/// One request the fake Discord got: method, path, JSON body, Authorization.
type Seen = (String, String, Value, String);

/// A fake Discord: one server (`42`, "Home") with `#general`; records every request (method, path, body, auth).
#[derive(Clone, Default)]
struct FakeDiscord {
    channels: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
    hooks: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
    seen: std::sync::Arc<std::sync::Mutex<Vec<Seen>>>,
}

impl FakeDiscord {
    fn answer(&self, method: &str, path: &str, body: &Value) -> (StatusCode, Value) {
        let mut channels = self.channels.lock().unwrap();
        let mut hooks = self.hooks.lock().unwrap();
        let parts: Vec<&str> = path.split('?').next().unwrap().trim_start_matches('/').split('/').collect();
        let next = || format!("{}", 100 + channels.len() + hooks.len());
        match (method, parts.as_slice()) {
            ("GET", ["guilds", "42"]) => (StatusCode::OK, json!({"id": "42", "name": "Home"})),
            ("GET", ["guilds", "42", "channels"]) => (StatusCode::OK, json!(*channels)),
            ("POST", ["guilds", "42", "channels"]) => {
                let c = json!({"id": next(), "name": body["name"], "type": body["type"],
                               "parent_id": body["parent_id"], "position": channels.len()});
                channels.push(c.clone());
                (StatusCode::CREATED, c)
            }
            ("PATCH", ["channels", id]) => match channels.iter_mut().find(|c| c["id"] == *id) {
                Some(c) => {
                    for (k, v) in body.as_object().unwrap() {
                        c[k] = v.clone();
                    }
                    (StatusCode::OK, c.clone())
                }
                None => (StatusCode::NOT_FOUND, json!({"message": "Unknown Channel"})),
            },
            ("GET", ["guilds", "42", "webhooks"]) => (StatusCode::OK, json!(*hooks)),
            ("POST", ["channels", id, "webhooks"]) => {
                let h = json!({"id": next(), "name": body["name"], "channel_id": id, "token": "SECRET-TOKEN"});
                hooks.push(h.clone());
                (StatusCode::OK, h)
            }
            ("POST", ["webhooks", _, "SECRET-TOKEN"]) => (StatusCode::OK, json!({"id": "m1"})),
            _ => (StatusCode::NOT_FOUND, json!({"message": "404: Not Found"})),
        }
    }

    async fn serve() -> (String, FakeDiscord) {
        use http_body_util::BodyExt;
        let fake = FakeDiscord::default();
        fake.channels.lock().unwrap().push(json!({"id": "1", "name": "general", "type": 0, "position": 0}));
        let f = fake.clone();
        let app = axum::Router::new().fallback(move |req: axum::http::Request<axum::body::Body>| {
            let f = f.clone();
            async move {
                let method = req.method().to_string();
                let path = req.uri().to_string();
                let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                let bytes = req.into_body().collect().await.unwrap().to_bytes();
                let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                let (status, v) = f.answer(&method, &path, &body);
                f.seen.lock().unwrap().push((method, path, body, auth));
                (status, axum::Json(v))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), fake)
    }

    fn channel(&self, name: &str) -> Option<Value> {
        self.channels.lock().unwrap().iter().find(|c| c["name"] == name).cloned()
    }
}

async fn connection_named(env: &Env, name: &str) -> Option<Value> {
    let (_, all) = call(&env.app, "GET", "/api/connections", None).await;
    all.as_array().unwrap().iter().find(|c| c["name"] == name).cloned()
}

#[tokio::test]
async fn the_pm_sets_up_a_discord_channel_and_webhook_that_a_workflow_posts_to() {
    let env = setup(None).await;
    let (base, discord) = FakeDiscord::serve().await;

    // A bot connection needs its server's id.
    let (s, e) = call(&env.app, "POST", "/api/connections", Some(json!({"name": "home", "kind": "discord_bot"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    assert!(e["message"].as_str().unwrap().contains("server's id"), "{e}");
    let (s, bot) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": "home", "kind": "discord_bot", "url": base, "config": {"guildId": "42"}})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{bot}");
    let bot_id = bot["id"].as_str().unwrap().to_string();
    let url = format!("/api/connections/{bot_id}/secret");
    call(&env.app, "PUT", &url, Some(json!({"secret": "bot-token"}))).await;

    let id = start_pm(&env, "hi").await;
    wait_said(&env, &id, 1).await;
    assert!(instructions(&env, &id).contains("`discord_webhook`"));

    // Looking needs no yes.
    let said = ask(&env, &id, "mcp: discord_get {}").await;
    assert!(said.contains("\"name\": \"Home\"") && said.contains("\"general\""), "{said}");
    assert!(discord.seen.lock().unwrap().iter().all(|r| r.3 == "Bot bot-token"));

    // "Set up a #ai-briefs channel with a webhook for the briefs workflow": the PM asks, the user says yes.
    let n = agent_says(&env, &id).await.len();
    let call_text = r##"mcp: discord_webhook {"channel": "#ai-briefs", "name": "briefs", "createChannel": true, "category": "Bots"}"##;
    send(&env, &id, call_text).await;
    let req = open_request(&env, &id).await;
    let title = events(&env, &id)
        .await
        .iter()
        .find(|e| e["event"]["kind"] == "request")
        .map(|e| e["event"]["title"].as_str().unwrap_or_default().to_string())
        .unwrap();
    assert_eq!(title, "Discord: a webhook on #ai-briefs (made if missing), saved as connection briefs");
    assert!(discord.channel("ai-briefs").is_none(), "nothing before the yes");
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("\"saved\": true") && said.contains("\"channelCreated\": true"), "{said}");
    assert!(!said.contains("SECRET-TOKEN"), "the webhook's URL never reaches the agent: {said}");
    let category = discord.channel("Bots").expect("the category");
    let channel = discord.channel("ai-briefs").expect("the channel");
    assert_eq!(channel["parent_id"], category["id"]);
    let saved = connection_named(&env, "briefs").await.expect("the connection");
    assert_eq!((saved["kind"].as_str(), saved["hasSecret"].as_bool()), (Some("discord_webhook"), Some(true)));
    assert!(!saved.to_string().contains("SECRET-TOKEN"));

    // discord_get says which connection the webhook is.
    let said = ask(&env, &id, "mcp: discord_get {}").await;
    assert!(said.contains("\"savedAs\": \"briefs\"") && !said.contains("SECRET-TOKEN"), "{said}");

    // A workflow posts through it.
    let doc = json!({"name": "Briefs", "nodes": [{"id": "go", "kind": "manual", "version": 1},
        {"id": "post", "kind": "discord.post", "version": 1, "config": {"connection": "briefs", "content": "Hello"}}],
        "edges": [{"from": "go", "to": "post"}]});
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    pdt_api::run_workflows(&env.state).await;
    let (_, run) = call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["runId"].as_str().unwrap()), None).await;
    assert_eq!(run["status"], "succeeded", "{run}");
    let hook = discord.hooks.lock().unwrap()[0]["id"].as_str().unwrap().to_string();
    let posted = discord.seen.lock().unwrap().iter().find(|r| r.1.contains("/webhooks/")).cloned().unwrap();
    assert!(posted.1.starts_with(&format!("/webhooks/{hook}/SECRET-TOKEN")), "{posted:?}");
    assert_eq!(posted.2["content"], "Hello");

    // A name already taken is refused before anything is made.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: discord_webhook {"channel": "general", "name": "briefs"}"#).await;
    let req = open_request(&env, &id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("already called briefs"), "{said}");
    assert_eq!(discord.hooks.lock().unwrap().len(), 1);

    // A bot connection that lets the PM act: PDT says yes for the user.
    let (s, v) = call(
        &env.app,
        "PATCH",
        &format!("/api/connections/{bot_id}"),
        Some(json!({"config": {"guildId": "42", "pmActs": true}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let said = ask(
        &env,
        &id,
        r##"mcp: discord_change {"changes": [{"op": "rename", "channel": "#ai-briefs", "name": "Morning Briefs"}, {"op": "archive", "channel": "general"}]}"##,
    )
    .await;
    assert!(said.contains("\"renamed\"") && said.contains("\"archived\""), "{said}");
    assert!(discord.channel("morning-briefs").is_some());
    let archive = discord.channel("Archive").expect("the Archive category");
    assert_eq!(discord.channel("general").unwrap()["parent_id"], archive["id"]);

    // A change that fails says what was done and what wasn't.
    let said = ask(
        &env,
        &id,
        r#"mcp: discord_change {"changes": [{"op": "create", "name": "notes"}, {"op": "move", "channel": "nope"}, {"op": "create", "name": "later"}]}"#,
    )
    .await;
    assert!(
        said.contains("\"failed\"") && said.contains("no channel called nope") && said.contains("create #later"),
        "{said}"
    );
    assert!(discord.channel("notes").is_some() && discord.channel("later").is_none());
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}
