//! Talking to the PM through the Discord bot's DMs (PDT-304) end to end, with `fake-agent` standing in for Claude and
//! a fake Discord: its REST API and a Gateway websocket that the test feeds DMs, slash commands and button presses.

use crate::common;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use common::call;
use common::pm::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

const USER: &str = "77";
const DM: &str = "dm1";

async fn setup() -> Env {
    common::pm::setup_with(|st| {
        st.integrations = Arc::new(pdt_api::Integrations::new(vec![Arc::new(pdt_int_discord::Discord)]));
        st.integrations.enable(&["discord".into()]);
    })
    .await
}

/// A request the fake got: method, path, body (JSON, or the raw text of a multipart one).
type Seen = (String, String, Value);

/// A fake Discord: REST and a Gateway that sends Hello, takes Identify, sends READY, then what the test pushes.
#[derive(Clone)]
struct FakeDiscord {
    seen: Arc<Mutex<Vec<Seen>>>,
    identify: Arc<Mutex<Option<Value>>>,
    identified: Arc<Mutex<u32>>,
    push: broadcast::Sender<Value>,
    next: Arc<Mutex<u64>>,
    /// Closes the Gateway connection with this code (once), to see the bot come back.
    drop_with: Arc<Mutex<Option<u16>>>,
}

impl FakeDiscord {
    async fn serve() -> (String, FakeDiscord) {
        use http_body_util::BodyExt;
        let fake = FakeDiscord {
            seen: Default::default(),
            identify: Default::default(),
            identified: Default::default(),
            push: broadcast::channel(64).0,
            next: Default::default(),
            drop_with: Default::default(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (f, g) = (fake.clone(), fake.clone());
        let app = axum::Router::new()
            .route(
                "/gw/",
                axum::routing::get(move |ws: WebSocketUpgrade| {
                    let g = g.clone();
                    async move { ws.on_upgrade(move |s| g.gateway(s)) }
                }),
            )
            .fallback(move |req: axum::http::Request<axum::body::Body>| {
                let f = f.clone();
                async move {
                    let method = req.method().to_string();
                    let path = req.uri().path().to_string();
                    let multipart = req
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|c| c.starts_with("multipart/"));
                    let bytes = req.into_body().collect().await.unwrap().to_bytes();
                    let body = if multipart {
                        Value::String(String::from_utf8_lossy(&bytes).into_owned())
                    } else {
                        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
                    };
                    let (status, v) = f.answer(&method, &path, addr);
                    f.seen.lock().unwrap().push((method, path, body));
                    (status, axum::Json(v))
                }
            });
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), fake)
    }

    fn answer(&self, method: &str, path: &str, addr: std::net::SocketAddr) -> (StatusCode, Value) {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        match (method, parts.as_slice()) {
            ("GET", ["gateway", "bot"]) => (StatusCode::OK, json!({"url": format!("ws://{addr}/gw")})),
            ("POST", ["users", "@me", "channels"]) => (StatusCode::OK, json!({"id": DM, "type": 1})),
            ("PUT", ["applications", "app1", "commands"]) => (StatusCode::OK, json!([])),
            ("POST", ["channels", DM, "messages"]) => {
                let mut n = self.next.lock().unwrap();
                *n += 1;
                (StatusCode::OK, json!({"id": format!("m{n}")}))
            }
            ("PATCH", ["channels", DM, "messages", id]) => (StatusCode::OK, json!({"id": id})),
            ("POST", ["channels", DM, "typing"]) | ("POST", ["interactions", _, _, "callback"]) => {
                (StatusCode::NO_CONTENT, Value::Null)
            }
            _ => (StatusCode::NOT_FOUND, json!({"message": "404: Not Found"})),
        }
    }

    async fn gateway(self, mut ws: WebSocket) {
        let send = |v: Value| Message::Text(v.to_string().into());
        let mut rx = self.push.subscribe();
        ws.send(send(json!({"op": 10, "d": {"heartbeat_interval": 45000}}))).await.unwrap();
        let mut seq = 0u64;
        loop {
            tokio::select! {
                m = ws.recv() => {
                    let Some(Ok(Message::Text(t))) = m else { return };
                    let v: Value = serde_json::from_str(&t).unwrap();
                    match v["op"].as_u64() {
                        Some(2) | Some(6) => {
                            *self.identify.lock().unwrap() = Some(v.clone());
                            *self.identified.lock().unwrap() += 1;
                            seq += 1;
                            let ready = if v["op"] == 2 {
                                json!({"op": 0, "t": "READY", "s": seq, "d": {"session_id": "sess",
                                       "application": {"id": "app1"}, "user": {"id": "bot1", "bot": true}}})
                            } else {
                                json!({"op": 0, "t": "RESUMED", "s": seq, "d": {}})
                            };
                            ws.send(send(ready)).await.unwrap();
                        }
                        Some(1) => ws.send(send(json!({"op": 11}))).await.unwrap(),
                        _ => {}
                    }
                }
                p = rx.recv() => {
                    let Ok(mut p) = p else { return };
                    if p == "drop" {
                        let code = self.drop_with.lock().unwrap().take().unwrap_or(4000);
                        let frame = axum::extract::ws::CloseFrame { code, reason: "bye".into() };
                        let _ = ws.send(Message::Close(Some(frame))).await;
                        return;
                    }
                    seq += 1;
                    p["s"] = json!(seq);
                    ws.send(send(p)).await.unwrap();
                }
            }
        }
    }

    fn dispatch(&self, name: &str, d: Value) {
        self.push.send(json!({"op": 0, "t": name, "d": d})).unwrap();
    }

    fn dm(&self, from: &str, text: &str) {
        self.dispatch(
            "MESSAGE_CREATE",
            json!({"id": "x", "channel_id": DM, "content": text, "author": {"id": from, "username": "u"}}),
        );
    }

    fn command(&self, id: &str, name: &str, options: Value) {
        self.dispatch(
            "INTERACTION_CREATE",
            json!({"id": id, "token": "tok", "type": 2, "channel_id": DM, "user": {"id": USER},
                   "data": {"name": name, "options": options}}),
        );
    }

    fn press(&self, id: &str, from: &str, custom_id: &str, message: &str) {
        self.dispatch(
            "INTERACTION_CREATE",
            json!({"id": id, "token": "tok", "type": 3, "channel_id": DM, "user": {"id": from},
                   "message": {"id": message, "content": "the prompt"}, "data": {"custom_id": custom_id}}),
        );
    }

    fn posted(&self) -> Vec<(String, Value)> {
        let seen = self.seen.lock().unwrap();
        let path = format!("/channels/{DM}/messages");
        seen.iter().filter(|(m, p, _)| m == "POST" && *p == path).map(|(_, _, b)| (String::new(), b.clone())).collect()
    }

    /// Waits for a request matching `f`; returns its body.
    async fn wait_for(&self, what: &str, f: impl Fn(&Seen) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(s) = self.seen.lock().unwrap().iter().find(|s| f(s)) {
                return s.2.clone();
            }
            assert!(Instant::now() < deadline, "no {what}; saw {:#?}", self.seen.lock().unwrap());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Waits for a DM whose content has `text`.
    async fn wait_dm(&self, text: &str) -> Value {
        let path = format!("/channels/{DM}/messages");
        self.wait_for(&format!("DM saying {text:?}"), |(m, p, b)| {
            m == "POST" && *p == path && body_text(b).contains(text)
        })
        .await
    }

    /// Waits for the answer to interaction `id`.
    async fn wait_reply(&self, id: &str) -> Value {
        let path = format!("/interactions/{id}/tok/callback");
        self.wait_for(&format!("reply to {id}"), |(_, p, _)| *p == path).await
    }
}

fn body_text(b: &Value) -> String {
    b.get("content").and_then(Value::as_str).map(String::from).unwrap_or_else(|| b.as_str().unwrap_or("").to_string())
}

/// The bot connection's status line (its DMs with the PM).
async fn dm_status(env: &Env, bot: &str) -> Value {
    call(&env.app, "GET", &format!("/api/connections/{bot}/status"), None).await.1
}

async fn wait_status(env: &Env, bot: &str, state: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let s = dm_status(env, bot).await;
        if s["state"] == state {
            return s;
        }
        assert!(Instant::now() < deadline, "the link is {s}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn pm_list(env: &Env) -> Vec<Value> {
    call(&env.app, "GET", "/api/pm/sessions", None).await.1.as_array().unwrap().clone()
}

#[tokio::test]
async fn the_user_talks_to_the_pm_in_the_bots_dms() {
    let env = setup().await;
    let (base, discord) = FakeDiscord::serve().await;
    let (s, bot) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": "home", "kind": "discord_bot", "url": base,
                    "config": {"guildId": "42", "dmUserId": USER}})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{bot}");
    let bot_id = bot["id"].as_str().unwrap().to_string();
    call(&env.app, "PUT", &format!("/api/connections/{bot_id}/secret"), Some(json!({"secret": "bot-token"}))).await;
    pdt_api::start_services(&env.state);

    // It connects with the DM intents, and registers its commands.
    let st = wait_status(&env, &bot_id, "Connected").await;
    assert_eq!((st["label"].as_str(), st["tone"].as_str()), (Some("DMs with the PM"), Some("ok")));
    let id = discord.identify.lock().unwrap().clone().unwrap();
    assert_eq!((id["d"]["token"].as_str(), id["d"]["intents"].as_u64()), (Some("bot-token"), Some(4096 + 32768)));
    let cmds = discord.wait_for("commands", |(m, p, _)| m == "PUT" && p.ends_with("/commands")).await;
    assert_eq!(cmds.as_array().unwrap().len(), 4);

    // Someone else's DM gets nothing; the user's gets the PM's answer, in a conversation marked via Discord.
    discord.dm("88", "let me in");
    discord.dm(USER, "what's in flight?");
    let reply = discord.wait_dm("echo: what's in flight?").await;
    assert_eq!(body_text(&reply), "**what's in flight?**\necho: what's in flight?", "headed by its title");
    assert!(discord.posted().iter().all(|(_, b)| !body_text(b).contains("let me in")));
    discord.wait_for("typing", |(m, p, _)| m == "POST" && p.ends_with("/typing")).await;
    let list = pm_list(&env).await;
    assert_eq!(list.len(), 1);
    assert_eq!((list[0]["via"].as_str(), list[0]["open"].as_bool()), (Some("discord"), Some(true)));
    let first = list[0]["session"]["id"].as_str().unwrap().to_string();

    // A next message continues it, with no title (the same conversation answers).
    discord.dm(USER, "and now?");
    let reply = discord.wait_dm("echo: and now?").await;
    assert_eq!(body_text(&reply), "echo: and now?");

    // /new starts another; its replies are headed by its title.
    discord.command("i1", "new", json!([{"name": "topic", "value": "Release plan"}]));
    let r = discord.wait_reply("i1").await;
    assert!(r["data"]["content"].as_str().unwrap().starts_with("Started **Release plan**"), "{r}");
    discord.dm(USER, "plan it");
    discord.wait_dm("echo: plan it").await;
    assert_eq!(pm_list(&env).await.iter().filter(|c| c["open"] == true).count(), 2);

    // /list shows both; /switch goes back to the first (its answer names it, so the replies need not).
    discord.command("i2", "list", json!([]));
    let r = discord.wait_reply("i2").await;
    let text = r["data"]["content"].as_str().unwrap();
    assert!(text.contains("**Release plan** (current)") && text.contains("what's in flight?"), "{text}");
    discord.dispatch(
        "INTERACTION_CREATE",
        json!({"id": "i3", "token": "tok", "type": 4, "user": {"id": USER},
               "data": {"name": "switch", "options": [{"name": "conversation", "value": "flight", "focused": true}]}}),
    );
    let r = discord.wait_reply("i3").await;
    assert_eq!(r["type"], 8);
    assert_eq!(r["data"]["choices"][0]["value"], first);
    discord.command("i4", "switch", json!([{"name": "conversation", "value": first}]));
    let r = discord.wait_reply("i4").await;
    assert!(r["data"]["content"].as_str().unwrap().starts_with("Now talking to"), "{r}");
    discord.dm(USER, "back again");
    let reply = discord.wait_dm("echo: back again").await;
    assert_eq!(body_text(&reply), "echo: back again");

    // A call that needs a yes: Approve and Decline buttons; approving in Discord runs it.
    discord.dm(USER, r#"mcp: create_ticket {"project": "PX", "title": "Add a logout"}"#);
    let prompt = discord.wait_dm("The PM asks to:").await;
    let approve = prompt["components"][0]["components"][0]["custom_id"].as_str().unwrap().to_string();
    let decline = prompt["components"][0]["components"][1]["custom_id"].as_str().unwrap().to_string();
    assert!(body_text(&prompt).contains("create_ticket") && body_text(&prompt).contains("Add a logout"));
    // Only the user may press.
    discord.press("p0", "88", &approve, "m9");
    let r = discord.wait_reply("p0").await;
    assert_eq!(r["data"]["flags"], 64, "{r}");
    assert_eq!(ticket_titles(&env).await, ["Fix the login"]);
    discord.press("p1", USER, &approve, "m9");
    let r = discord.wait_reply("p1").await;
    assert_eq!(r["type"], 7);
    assert!(r["data"]["content"].as_str().unwrap().ends_with("**Approved**"), "{r}");
    assert_eq!(r["data"]["components"][0]["components"][0]["disabled"], true);
    discord.wait_dm("mcp create_ticket:").await;
    assert_eq!(ticket_titles(&env).await, ["Add a logout", "Fix the login"]);
    let _ = decline;

    // Approved in the web instead: the Discord message says so and its buttons go off.
    discord.dm(USER, r#"mcp: create_ticket {"project": "PX", "title": "From the web"}"#);
    let prompt = discord.wait_dm("From the web").await;
    let req = open_request(&env, &first).await;
    let url = format!("/api/sessions/{first}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let edit = discord.wait_for("edit", |(m, _, b)| m == "PATCH" && body_text(b).contains("(via web)")).await;
    assert!(body_text(&edit).ends_with("**Approved** (via web)"), "{edit}");
    assert_eq!(edit["components"][0]["components"][1]["disabled"], true);
    let _ = prompt;

    // Declined in Discord: the PM hears a refusal.
    discord.dm(USER, r#"mcp: create_ticket {"project": "PX", "title": "Not this"}"#);
    let prompt = discord.wait_dm("Not this").await;
    let decline = prompt["components"][0]["components"][1]["custom_id"].as_str().unwrap().to_string();
    discord.press("p2", USER, &decline, "m20");
    let r = discord.wait_reply("p2").await;
    assert!(r["data"]["content"].as_str().unwrap().ends_with("**Declined**"), "{r}");
    discord.wait_dm("mcp create_ticket denied").await;
    assert_eq!(ticket_titles(&env).await.len(), 3);

    // A question: a button per option, or a typed answer.
    discord.dm(USER, "ask: Which one?");
    let q = discord.wait_dm("Which one?").await;
    let b = q["components"][0]["components"][1].clone();
    assert_eq!(b["label"], "B");
    discord.press("p3", USER, b["custom_id"].as_str().unwrap(), "m30");
    let r = discord.wait_reply("p3").await;
    assert!(r["data"]["content"].as_str().unwrap().ends_with("**B**"), "{r}");
    discord.wait_dm(r#"You answered {"Which one?":"B"}"#).await;
    discord.dm(USER, "ask: Which other?");
    discord.wait_dm("Which other?").await;
    discord.dm(USER, "Neither");
    let edit =
        discord.wait_for("typed answer", |(m, _, b)| m == "PATCH" && body_text(b).ends_with("**Neither**")).await;
    assert_eq!(edit["components"][0]["components"][0]["disabled"], true);
    discord.wait_dm(r#"You answered {"Which other?":"Neither"}"#).await;

    // Inbox items as DMs: turned on, the link starts over; a call another conversation waits on is sent as a DM (one
    // held in the DMs has its own buttons instead).
    let before = *discord.identified.lock().unwrap();
    let (s, v) = call(
        &env.app,
        "PATCH",
        &format!("/api/connections/{bot_id}"),
        Some(json!({"config": {"guildId": "42", "dmUserId": USER, "dmInbox": true}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let deadline = Instant::now() + Duration::from_secs(20);
    while *discord.identified.lock().unwrap() == before {
        assert!(Instant::now() < deadline, "the link never started over");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    wait_status(&env, &bot_id, "Connected").await;
    let web = start_pm(&env, "hi from the web").await;
    wait_said(&env, &web, 1).await;
    assert!(discord.posted().iter().all(|(_, b)| !body_text(b).contains("hi from the web")), "not held in the DMs");
    let n = discord.posted().len();
    send(&env, &web, r#"mcp: create_ticket {"project": "PX", "title": "Inbox one"}"#).await;
    open_request(&env, &web).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    let note = loop {
        pdt_api::sync_notifications(&env.state).await;
        if let Some((_, b)) = discord.posted().into_iter().skip(n).find(|(_, b)| body_text(b).starts_with("**")) {
            break b;
        }
        assert!(Instant::now() < deadline, "no Inbox DM: {:?}", discord.posted());
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(note.get("components").is_none(), "an Inbox item is news, not buttons: {note}");
    call(&env.app, "POST", &format!("/api/sessions/{web}/kill"), None).await;

    // The connection drops: the bot resumes on its own.
    discord.push.send(json!("drop")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while discord.identify.lock().unwrap().as_ref().is_none_or(|v| v["op"] != 6) {
        assert!(Instant::now() < deadline, "it never resumed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    wait_status(&env, &bot_id, "Connected").await;
    discord.dm(USER, "still there?");
    discord.wait_dm("echo: still there?").await;

    // /close ends the current one (the PM writes its journal); the next DM starts a new conversation.
    discord.command("i5", "close", json!([]));
    let r = discord.wait_reply("i5").await;
    assert!(r["data"]["content"].as_str().unwrap().starts_with("Closing **"), "{r}");
    discord.dm(USER, "fresh start");
    discord.wait_dm("echo: fresh start").await;
    let via: Vec<Value> = pm_list(&env).await.into_iter().filter(|c| c["via"] == "discord").collect();
    assert_eq!(via.len(), 3, "{via:?}");
    assert!(dm_status(&env, &bot_id).await["state"] == "Connected");

    // A wrong token stops it, and the connection says why.
    *discord.drop_with.lock().unwrap() = Some(4004);
    discord.push.send(json!("drop")).unwrap();
    let st = wait_status(&env, &bot_id, "Not connected").await;
    assert_eq!(st["tone"], "failed");
    assert!(st["error"].as_str().unwrap().contains("token"), "{st}");
    let (_, conns) = call(&env.app, "GET", "/api/connections", None).await;
    assert!(conns[0]["lastError"].as_str().unwrap().contains("token"), "{conns}");

    for c in pm_list(&env).await {
        call(&env.app, "POST", &format!("/api/sessions/{}/kill", c["session"]["id"].as_str().unwrap()), None).await;
    }
}
