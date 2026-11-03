//! A Discord server through a bot (PDT-98): a small client for Discord's REST API (no gateway) with a `discord_bot`
//! connection's token, the PM's tools (`discord_get`, `discord_change`, `discord_webhook`: actions
//! that are tools only) and the `discord.channel.ensure` node kind. Webhook URLs (their tokens) go to the secret store and never into a result or
//! an error (`send_error` drops the URL from a failed request).

use crate::core::{self as core, ARCHIVE_CATEGORY, Change, Channel, ChannelKind, DISCORD_BOT};
use crate::post::DISCORD_WEBHOOK;
use axum::Json;
use axum::extract::{Path, State};
use pdt_api::sdk::AppState;
use pdt_api::sdk::connections::{ConnectionInput, ConnectionSecret};
use pdt_api::sdk::http::{client, read_body, send_error};
use pdt_api::sdk::{Action, ActionCx, AnyAction, ConnectionFace, ToolFace};
use pdt_api::sdk::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::integration::ToolDecl;
use pdt_core::workflow::api::Connection;
use reqwest::Method;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use ts_rs::TS;

/// A bot on one server.
pub(crate) struct Bot {
    base: String,
    token: String,
    pub guild: String,
}

/// A webhook, without its token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Webhook {
    pub id: String,
    pub name: String,
    pub channel_id: String,
}

/// The `discord_bot` connection `name` means: that one, or the only one there is.
pub(crate) async fn bot_connection(st: &AppState, name: Option<&str>) -> Result<Connection, String> {
    pdt_api::sdk::connections::connection_of_kind(st, DISCORD_BOT, name).await
}

impl Bot {
    /// The bot of a `discord_bot` connection (by name, or the only one).
    pub async fn open(st: &AppState, name: Option<&str>) -> Result<(Connection, Bot), String> {
        let conn = bot_connection(st, name).await?;
        let cfg = core::bot_config(&conn.config).map_err(|e| format!("{}: {e}", conn.name))?;
        let (_, secret) = pdt_api::sdk::connections::connection_with_secret(st, &conn.name).await?;
        let token = secret.filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("{} has no bot token", conn.name))?;
        let base = if conn.url.trim().is_empty() { core::API_BASE.to_string() } else { conn.url.clone() };
        let bot = Bot { base: base.trim_end_matches('/').into(), token: token.trim().into(), guild: cfg.guild_id };
        Ok((conn, bot))
    }

    /// One request; a 429 waits Discord's `retry_after` and tries again.
    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value, String> {
        self.call_with(method, path, body, None).await
    }

    /// One request, with a file (name, text) when given (sent as multipart, the JSON as `payload_json`).
    async fn call_with(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        file: Option<(&str, &str)>,
    ) -> Result<Value, String> {
        let url = format!("{}{path}", self.base);
        for _ in 0..crate::TRIES {
            let mut req =
                client().request(method.clone(), &url).header("Authorization", format!("Bot {}", self.token)).header(
                    "User-Agent",
                    format!(
                        "DiscordBot (https://github.com/{}, {})",
                        pdt_core::brand::RELEASE_REPO,
                        env!("CARGO_PKG_VERSION")
                    ),
                );
            match (&body, file) {
                (b, Some((name, text))) => {
                    let mut payload = b.clone().unwrap_or_else(|| json!({}));
                    payload["attachments"] = json!([{"id": 0, "filename": name}]);
                    let part = reqwest::multipart::Part::text(text.to_string())
                        .file_name(name.to_string())
                        .mime_str("text/plain; charset=utf-8")
                        .map_err(|e| e.to_string())?;
                    let form = reqwest::multipart::Form::new()
                        .text("payload_json", payload.to_string())
                        .part("files[0]", part);
                    req = req.multipart(form);
                }
                (Some(b), None) => req = req.json(b),
                (None, None) => {}
            }
            let resp = req.send().await.map_err(|e| format!("Discord: {}", send_error(e)))?;
            let status = resp.status().as_u16();
            let bytes = read_body(resp, 1024 * 1024).await?;
            let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            match status {
                200..=299 => return Ok(v),
                429 => {
                    let secs = v["retry_after"].as_f64().unwrap_or(1.0).clamp(0.0, 60.0);
                    tokio::time::sleep(std::time::Duration::from_secs_f64(secs)).await;
                }
                _ => {
                    let msg = v["message"]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
                    let hint = match status {
                        401 => " (the bot token is wrong)",
                        403 => " (the bot lacks a permission: it needs Manage Channels and Manage Webhooks)",
                        _ => "",
                    };
                    return Err(format!("Discord answered {status}: {msg}{hint}"));
                }
            }
        }
        Err("Discord kept rate-limiting the request".into())
    }

    /// The Gateway's address (PDT-304).
    pub async fn gateway_url(&self) -> Result<String, String> {
        let v = self.call(Method::GET, "/gateway/bot", None).await?;
        v["url"].as_str().map(String::from).ok_or_else(|| "Discord gave no Gateway address".into())
    }

    /// The DM channel with a user (made when there is none).
    pub async fn open_dm(&self, user_id: &str) -> Result<String, String> {
        let v = self.call(Method::POST, "/users/@me/channels", Some(json!({"recipient_id": user_id}))).await?;
        v["id"].as_str().map(String::from).ok_or_else(|| "Discord's answer had no channel".into())
    }

    /// Posts a message (`body`: Discord's, such as `content` and `components`), with a text file when given; returns
    /// its id.
    pub async fn send(&self, channel_id: &str, body: Value, file: Option<(&str, &str)>) -> Result<String, String> {
        let path = format!("/channels/{channel_id}/messages");
        let v = self.call_with(Method::POST, &path, Some(body), file).await?;
        v["id"].as_str().map(String::from).ok_or_else(|| "Discord's answer had no message".into())
    }

    /// Changes a message the bot sent.
    pub async fn edit(&self, channel_id: &str, message_id: &str, body: Value) -> Result<(), String> {
        let path = format!("/channels/{channel_id}/messages/{message_id}");
        self.call(Method::PATCH, &path, Some(body)).await.map(|_| ())
    }

    /// Shows "typing…" in a channel for 10 s or until the bot posts.
    pub async fn typing(&self, channel_id: &str) -> Result<(), String> {
        self.call(Method::POST, &format!("/channels/{channel_id}/typing"), None).await.map(|_| ())
    }

    /// Answers an interaction (a slash command, a button press, an autocomplete).
    pub async fn reply(&self, id: &str, token: &str, body: Value) -> Result<(), String> {
        self.call(Method::POST, &format!("/interactions/{id}/{token}/callback"), Some(body)).await.map(|_| ())
    }

    /// Sets the application's global slash commands.
    pub async fn set_commands(&self, application_id: &str, commands: Value) -> Result<(), String> {
        self.call(Method::PUT, &format!("/applications/{application_id}/commands"), Some(commands)).await.map(|_| ())
    }

    /// The server's name.
    pub async fn server_name(&self) -> Result<String, String> {
        let v = self.call(Method::GET, &format!("/guilds/{}", self.guild), None).await?;
        Ok(v["name"].as_str().unwrap_or_default().to_string())
    }

    pub async fn channels(&self) -> Result<Vec<Channel>, String> {
        let v = self.call(Method::GET, &format!("/guilds/{}/channels", self.guild), None).await?;
        let mut all: Vec<Channel> = v.as_array().into_iter().flatten().filter_map(Channel::from_api).collect();
        all.sort_by_key(|c| c.position);
        Ok(all)
    }

    pub async fn create_channel(
        &self,
        name: &str,
        kind: ChannelKind,
        parent: Option<&str>,
        topic: Option<&str>,
    ) -> Result<Channel, String> {
        let t = kind.to_type().ok_or("PDT makes text, voice, announcement and forum channels and categories")?;
        let mut body = json!({"name": name, "type": t});
        if let Some(p) = parent {
            body["parent_id"] = json!(p);
        }
        if let Some(topic) = topic.map(str::trim).filter(|t| !t.is_empty()) {
            body["topic"] = json!(topic);
        }
        let v = self.call(Method::POST, &format!("/guilds/{}/channels", self.guild), Some(body)).await?;
        Channel::from_api(&v).ok_or_else(|| "Discord's answer had no channel".into())
    }

    /// Changes a channel (`patch`: Discord's fields, such as `name` and `parent_id`).
    pub async fn modify_channel(&self, id: &str, patch: Value) -> Result<Channel, String> {
        let v = self.call(Method::PATCH, &format!("/channels/{id}"), Some(patch)).await?;
        Channel::from_api(&v).ok_or_else(|| "Discord's answer had no channel".into())
    }

    pub async fn webhooks(&self) -> Result<Vec<Webhook>, String> {
        let v = self.call(Method::GET, &format!("/guilds/{}/webhooks", self.guild), None).await?;
        Ok(v.as_array().into_iter().flatten().filter_map(webhook).collect())
    }

    /// A new webhook on a channel, and its URL (the secret: never shown to an agent).
    pub async fn create_webhook(&self, channel_id: &str, name: &str) -> Result<(Webhook, String), String> {
        let v =
            self.call(Method::POST, &format!("/channels/{channel_id}/webhooks"), Some(json!({"name": name}))).await?;
        let w = webhook(&v).ok_or("Discord's answer had no webhook")?;
        let url = match (v["url"].as_str(), v["token"].as_str()) {
            (Some(u), _) => u.to_string(),
            (None, Some(t)) => format!("{}/webhooks/{}/{t}", self.base, w.id),
            (None, None) => return Err("Discord's answer had no webhook token".into()),
        };
        Ok((w, url))
    }

    pub async fn delete_webhook(&self, id: &str) -> Result<(), String> {
        self.call(Method::DELETE, &format!("/webhooks/{id}"), None).await.map(|_| ())
    }
}

fn webhook(v: &Value) -> Option<Webhook> {
    Some(Webhook {
        id: v["id"].as_str()?.into(),
        name: v["name"].as_str().unwrap_or_default().into(),
        channel_id: v["channel_id"].as_str().unwrap_or_default().into(),
    })
}

/// A text channel called `name` (in `category` when given): the one there is, or a new one. Returns it and whether
/// it was made.
pub(crate) async fn ensure_channel(
    bot: &Bot,
    channels: &mut Vec<Channel>,
    name: &str,
    category: Option<&str>,
    topic: Option<&str>,
) -> Result<(Channel, bool), String> {
    let name = core::valid_name(name, ChannelKind::Text)?;
    if let Ok(c) = core::find(channels, &name, false) {
        return Ok((c.clone(), false));
    }
    let parent = match category.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => Some(category_id(bot, channels, c, true).await?),
        None => None,
    };
    let c = bot.create_channel(&name, ChannelKind::Text, parent.as_deref(), topic).await?;
    channels.push(c.clone());
    Ok((c, true))
}

/// A category's id by id or name; `make`: made when missing.
async fn category_id(bot: &Bot, channels: &mut Vec<Channel>, given: &str, make: bool) -> Result<String, String> {
    match core::find(channels, given, true) {
        Ok(c) => Ok(c.id.clone()),
        Err(_) if make => {
            let name = core::valid_name(given, ChannelKind::Category)?;
            let c = bot.create_channel(&name, ChannelKind::Category, None, None).await?;
            channels.push(c.clone());
            Ok(c.id)
        }
        Err(e) => Err(e),
    }
}

/// Makes one change; returns what was done.
async fn apply(bot: &Bot, channels: &mut Vec<Channel>, change: &Change) -> Result<Value, String> {
    let replace = |channels: &mut Vec<Channel>, c: &Channel| {
        if let Some(old) = channels.iter_mut().find(|x| x.id == c.id) {
            *old = c.clone();
        }
    };
    match change {
        Change::Create { name, kind, category, topic } => {
            let name = core::valid_name(name, *kind)?;
            if core::find(channels, &name, *kind == ChannelKind::Category).is_ok_and(|c| c.kind == *kind) {
                return Err(format!("the server already has a {} called {name}", kind.as_str()));
            }
            let parent = match category.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                Some(_) if *kind == ChannelKind::Category => return Err("a category can't be in a category".into()),
                Some(c) => Some(category_id(bot, channels, c, false).await?),
                None => None,
            };
            let c = bot.create_channel(&name, *kind, parent.as_deref(), topic.as_deref()).await?;
            channels.push(c.clone());
            Ok(json!({"created": c}))
        }
        Change::Rename { channel, name } => {
            let c = find_any(channels, channel)?;
            let name = core::valid_name(name, c.kind)?;
            let c = bot.modify_channel(&c.id, json!({"name": name})).await?;
            replace(channels, &c);
            Ok(json!({"renamed": c}))
        }
        Change::Move { channel, category } => {
            let c = core::find(channels, channel, false)?.clone();
            let parent = match category.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                Some(cat) => Value::String(category_id(bot, channels, cat, false).await?),
                None => Value::Null,
            };
            let c = bot.modify_channel(&c.id, json!({"parent_id": parent})).await?;
            replace(channels, &c);
            Ok(json!({"moved": c}))
        }
        Change::Archive { channel } => {
            let c = core::find(channels, channel, false)?.clone();
            let parent = category_id(bot, channels, ARCHIVE_CATEGORY, true).await?;
            let c = bot.modify_channel(&c.id, json!({"parent_id": parent})).await?;
            replace(channels, &c);
            Ok(json!({"archived": c}))
        }
    }
}

/// A channel or, failing that, a category.
fn find_any(channels: &[Channel], given: &str) -> Result<Channel, String> {
    core::find(channels, given, false).or_else(|e| core::find(channels, given, true).map_err(|_| e)).cloned()
}

// ---- the PM's tools ----

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DiscordGet {
    /// A Discord bot connection's name; the only one when left out.
    #[serde(default)]
    pub connection: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DiscordChange {
    /// A Discord bot connection's name; the only one when left out.
    #[serde(default)]
    pub connection: Option<String>,
    /// Made in order; the first that fails stops the rest.
    changes: Vec<Change>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DiscordWebhookArgs {
    /// A Discord bot connection's name; the only one when left out.
    #[serde(default)]
    pub connection: Option<String>,
    /// The channel (id, name or `#name`).
    channel: String,
    /// The new `discord_webhook` connection's name, which `discord.post` steps use (`briefs`).
    name: String,
    /// The webhook's name in Discord (shown as the sender); the connection's name when left out.
    #[serde(default)]
    webhook_name: Option<String>,
    /// Make the channel (a text channel) when the server has none called that.
    #[serde(default)]
    create_channel: bool,
    /// A new channel's category (made when missing).
    #[serde(default)]
    category: Option<String>,
    /// A new channel's topic.
    #[serde(default)]
    topic: Option<String>,
}

/// `discord_get`: the server's categories, channels and webhooks (which saved connection each one is).
pub(crate) async fn get(st: &AppState, a: DiscordGet) -> Result<Value, String> {
    let (conn, bot) = Bot::open(st, a.connection.as_deref()).await?;
    let server = bot.server_name().await?;
    let channels = bot.channels().await?;
    let hooks = bot.webhooks().await?;
    let saved = st.store.list_connections().await.map_err(|e| e.to_string())?;
    let name_of =
        |id: &Option<String>| id.as_ref().and_then(|id| channels.iter().find(|c| &c.id == id)).map(|c| c.name.clone());
    let categories: Vec<Value> = channels
        .iter()
        .filter(|c| c.kind == ChannelKind::Category)
        .map(|c| json!({"id": c.id, "name": c.name}))
        .collect();
    let list: Vec<Value> = channels
        .iter()
        .filter(|c| c.kind != ChannelKind::Category)
        .map(|c| {
            json!({"id": c.id, "name": c.name, "kind": c.kind, "category": name_of(&c.parent_id), "topic": c.topic})
        })
        .collect();
    let webhooks: Vec<Value> = hooks
        .iter()
        .map(|w| {
            let saved_as = saved
                .iter()
                .find(|c| c.kind == DISCORD_WEBHOOK && c.config["webhookId"].as_str() == Some(&w.id))
                .map(|c| c.name.clone());
            json!({"id": w.id, "name": w.name, "channel": name_of(&Some(w.channel_id.clone())),
                   "channelId": w.channel_id, "savedAs": saved_as})
        })
        .collect();
    let pm_acts = core::bot_config(&conn.config).is_ok_and(|c| c.pm_acts);
    Ok(json!({"connection": conn.name, "server": {"id": bot.guild, "name": server},
              "pmActsWithoutAsking": pm_acts, "categories": categories, "channels": list, "webhooks": webhooks}))
}

/// `discord_change`: makes the changes in order.
pub(crate) async fn change(st: &AppState, a: DiscordChange) -> Result<Value, String> {
    if a.changes.is_empty() {
        return Err("give at least one change".into());
    }
    let (_, bot) = Bot::open(st, a.connection.as_deref()).await?;
    let mut channels = bot.channels().await?;
    let mut done = vec![];
    for (i, c) in a.changes.iter().enumerate() {
        match apply(&bot, &mut channels, c).await {
            Ok(v) => done.push(v),
            Err(e) if done.is_empty() => return Err(format!("{}: {e}", c.describe())),
            Err(e) => {
                return Ok(json!({"done": done, "failed": {"change": c.describe(), "error": e},
                                 "notTried": a.changes[i + 1..].iter().map(Change::describe).collect::<Vec<_>>()}));
            }
        }
    }
    Ok(json!({"done": done}))
}

/// `discord_webhook`: a webhook on the channel, saved as a `discord_webhook` connection. Its URL stays in the secret
/// store.
pub(crate) async fn webhook_tool(st: &AppState, a: DiscordWebhookArgs) -> Result<Value, String> {
    let (bot_conn, bot) = Bot::open(st, a.connection.as_deref()).await?;
    let name = a.name.trim().to_string();
    if name.is_empty() {
        return Err("name the connection workflows will use (`name`)".into());
    }
    if st.store.connection_by_name(&name).await.map_err(|e| e.to_string())?.is_some() {
        return Err(format!("a connection is already called {name}: pick another name"));
    }
    let mut channels = bot.channels().await?;
    let (channel, created) = if a.create_channel {
        ensure_channel(&bot, &mut channels, &a.channel, a.category.as_deref(), a.topic.as_deref()).await?
    } else {
        (core::find(&channels, &a.channel, false)?.clone(), false)
    };
    let hook_name = a.webhook_name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&name).to_string();
    let (hook, url) = bot.create_webhook(&channel.id, &hook_name).await?;
    let input = ConnectionInput {
        name: Some(name.clone()),
        kind: Some(DISCORD_WEBHOOK.into()),
        url: None,
        config: Some(json!({"webhookId": hook.id, "channelId": channel.id, "channel": channel.name,
                            "bot": bot_conn.name})),
    };
    let saved = async {
        let (_, Json(c)) =
            pdt_api::sdk::connections::create_connection(State(st.clone()), Ok(Json(input))).await.map_err(api)?;
        let secret = ConnectionSecret { secret: url };
        if let Err(e) =
            pdt_api::sdk::connections::put_secret(State(st.clone()), Path(c.id.clone()), Ok(Json(secret))).await
        {
            if let Err(d) = st.store.delete_connection(&c.id).await {
                tracing::warn!(connection = %c.id, error = %d, "could not remove a Discord channel connection whose webhook failed to save");
            }
            return Err(api(e));
        }
        Ok(c)
    }
    .await;
    let conn = match saved {
        Ok(c) => c,
        Err(e) => {
            if let Err(d) = bot.delete_webhook(&hook.id).await {
                tracing::warn!(webhook = %hook.id, error = %d, "could not remove a webhook PDT failed to save");
            }
            return Err(format!("the webhook could not be saved, so it was removed: {e}"));
        }
    };
    tracing::info!(connection = %conn.name, channel = %channel.name, "saved a Discord webhook");
    Ok(json!({"saved": true, "connection": conn.name, "channel": {"id": channel.id, "name": channel.name},
              "channelCreated": created, "webhookId": hook.id,
              "use": format!("discord.post with connection \"{}\"", conn.name)}))
}

// ---- the actions ----

/// The bot's connection: a tool call may leave it out.
const BOT: Option<ConnectionFace> = Some(ConnectionFace { kind: DISCORD_BOT, label: "Discord bot" });

/// The actions, in `tools/list` order. They are tools only: workflows post through `discord.post` and make channels
/// with `discord.channel.ensure`.
pub(crate) fn actions() -> Vec<Arc<dyn AnyAction>> {
    vec![Arc::new(GetServer), Arc::new(ChangeServer), Arc::new(MakeWebhook)]
}

fn bot_name(c: &Option<String>) -> &str {
    c.as_deref().unwrap_or("the bot")
}

pub(crate) struct GetServer;

impl Action for GetServer {
    type Input = DiscordGet;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        ToolFace {
            decl: ToolDecl::new("discord_get", "Look at the Discord server: its channels and webhooks")
                .read_only()
                .open_world()
                .pm(),
            title: "Look at the Discord server",
            description: "The Discord server a bot connection manages: its categories, channels and webhooks (with \
                          the connection each webhook is saved as), and whether you may change it without asking.",
        }
    }

    fn connection(&self) -> Option<ConnectionFace> {
        BOT
    }

    fn approval_title(&self, a: &DiscordGet) -> String {
        format!("look at the Discord server ({})", bot_name(&a.connection))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, a: DiscordGet) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(get(cx.st, a))
    }
}

pub(crate) struct ChangeServer;

impl Action for ChangeServer {
    type Input = DiscordChange;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        ToolFace {
            decl: ToolDecl::new("discord_change", "Create, rename, move or archive Discord channels")
                .asks()
                .open_world()
                .pm(),
            title: "Change the Discord server",
            description: "Creates, renames, moves or archives (moves into Archive) channels and categories on the \
                          Discord server, in order. The user confirms each call unless the bot connection lets you \
                          act.",
        }
    }

    fn connection(&self) -> Option<ConnectionFace> {
        BOT
    }

    fn approval_title(&self, a: &DiscordChange) -> String {
        let all: Vec<String> = a.changes.iter().map(Change::describe).collect();
        format!("{} on the Discord server ({})", all.join(", "), bot_name(&a.connection))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, a: DiscordChange) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(change(cx.st, a))
    }
}

pub(crate) struct MakeWebhook;

impl Action for MakeWebhook {
    type Input = DiscordWebhookArgs;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        ToolFace {
            decl: ToolDecl::new("discord_webhook", "Make a Discord webhook for discord.post steps")
                .asks()
                .open_world()
                .pm(),
            title: "Make a Discord webhook",
            description: "Makes a webhook on a channel (and the channel, with createChannel) and saves it as a \
                          discord_webhook connection that discord.post steps use by name. Its URL is never shown. \
                          The user confirms each call unless the bot connection lets you act.",
        }
    }

    fn connection(&self) -> Option<ConnectionFace> {
        BOT
    }

    fn approval_title(&self, a: &DiscordWebhookArgs) -> String {
        format!("make a webhook on {} saved as {} ({})", a.channel, a.name, bot_name(&a.connection))
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, a: DiscordWebhookArgs) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(webhook_tool(cx.st, a))
    }
}

fn api(e: pdt_api::sdk::ApiError) -> String {
    e.message().to_string()
}

// ---- discord.channel.ensure ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct EnsureChannelConfig {
    /// A `discord_bot` connection (its secret is the bot's token).
    #[schemars(extend("x-connection-kind" = "discord_bot"))]
    pub connection: String,
    /// The text channel's name (`ai-briefs`).
    pub name: String,
    /// A new channel's category (made when missing).
    #[serde(default)]
    pub category: Option<String>,
    /// A new channel's topic.
    #[serde(default)]
    pub topic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnsureChannelOutput {
    pub channel_id: String,
    pub name: String,
    /// Whether the step made it.
    pub created: bool,
}

pub struct EnsureChannel;

impl NodeKind for EnsureChannel {
    fn spec(&self) -> KindSpec {
        spec::<EnsureChannelConfig, EnsureChannelOutput>(
            "discord.channel.ensure",
            1,
            Category::Action,
            "Ensure a Discord channel",
            "Makes a text channel on a Discord server through a bot connection (kind discord_bot) unless the server \
             has one called that.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: EnsureChannelConfig = parse(config)?;
        if c.connection.trim().is_empty() {
            return Err("connection is empty: name a discord_bot connection".into());
        }
        core::valid_name(&c.name, ChannelKind::Text).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: EnsureChannelConfig = parse(&input)?;
            if cx.dry_run {
                let what =
                    format!("make #{} through {} unless it exists", c.name.trim_start_matches('#'), c.connection);
                return Ok(Outcome::next("out", json!({"would": what})));
            }
            let (_, bot) = Bot::open(cx.st, Some(&c.connection)).await?;
            let mut channels = bot.channels().await?;
            let (ch, created) =
                ensure_channel(&bot, &mut channels, &c.name, c.category.as_deref(), c.topic.as_deref()).await?;
            Ok(Outcome::next("out", EnsureChannelOutput { channel_id: ch.id, name: ch.name, created }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use axum::http::{HeaderMap, Uri};
    use std::sync::Mutex;

    /// A request the fake Discord saw: method, path, JSON body, Authorization.
    type Seen = (String, String, Value, String);
    type Answer = dyn Fn(&str, &str, &Value) -> (u16, Value) + Send + Sync;

    struct Fake {
        seen: Mutex<Vec<Seen>>,
        answer: Box<Answer>,
    }

    /// A local Discord answering with `answer`, and the bot of server 42 on it.
    async fn fake(answer: impl Fn(&str, &str, &Value) -> (u16, Value) + Send + Sync + 'static) -> (Bot, Arc<Fake>) {
        let f = Arc::new(Fake { seen: Mutex::new(vec![]), answer: Box::new(answer) });
        let state = f.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
            let f = state.clone();
            async move {
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                let path = uri.path().to_string();
                let (status, v) = (f.answer)(method.as_str(), &path, &body);
                f.seen.lock().unwrap().push((method.to_string(), path, body, auth));
                (axum::http::StatusCode::from_u16(status).unwrap(), Json(v))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (Bot { base, token: "tok".into(), guild: "42".into() }, f)
    }

    impl Fake {
        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
    }

    fn ch(id: &str, name: &str, kind: ChannelKind, parent: Option<&str>) -> Channel {
        Channel {
            id: id.into(),
            name: name.into(),
            kind,
            parent_id: parent.map(String::from),
            position: 0,
            topic: None,
        }
    }

    /// Discord's answer to a new or changed channel: the body it was sent, as a channel with id `id`.
    fn echo(id: &str, body: &Value) -> Value {
        let mut v = body.clone();
        v["id"] = json!(id);
        if v.get("type").is_none() {
            v["type"] = json!(0);
        }
        v
    }

    #[tokio::test]
    async fn discords_errors_say_what_the_bot_lacks() {
        let (bot, seen) = fake(|_, path, _| match path {
            "/guilds/42" => (401, json!({"message": "401: Unauthorized"})),
            "/guilds/42/channels" => (403, json!({"message": "Missing Permissions"})),
            "/guilds/42/webhooks" => (404, json!({"message": "Unknown Guild"})),
            _ => (500, Value::Null),
        })
        .await;
        let e = bot.server_name().await.unwrap_err();
        assert_eq!(e, "Discord answered 401: 401: Unauthorized (the bot token is wrong)");
        let e = bot.channels().await.unwrap_err();
        assert!(e.contains("403: Missing Permissions") && e.contains("Manage Channels and Manage Webhooks"), "{e}");
        assert_eq!(bot.webhooks().await.unwrap_err(), "Discord answered 404: Unknown Guild");
        // No message: the body as it came.
        assert_eq!(bot.gateway_url().await.unwrap_err(), "Discord answered 500: null");
        assert!(seen.seen().iter().all(|s| s.3 == "Bot tok"), "{:?}", seen.seen());
    }

    #[tokio::test]
    async fn a_rate_limit_is_waited_out_a_few_times_only() {
        let n = Arc::new(Mutex::new(0));
        let count = n.clone();
        let (bot, _) = fake(move |_, path, _| {
            let mut n = count.lock().unwrap();
            *n += 1;
            match (path, *n) {
                ("/gateway/bot", 1) => (429, json!({"retry_after": 0.01})),
                ("/gateway/bot", _) => (200, json!({"url": "wss://gateway.discord.gg"})),
                _ => (429, json!({"retry_after": 0.01})),
            }
        })
        .await;
        assert_eq!(bot.gateway_url().await.unwrap(), "wss://gateway.discord.gg");
        assert_eq!(*n.lock().unwrap(), 2);
        assert_eq!(bot.server_name().await.unwrap_err(), "Discord kept rate-limiting the request");
        assert_eq!(*n.lock().unwrap(), 2 + crate::TRIES as usize);
    }

    #[tokio::test]
    async fn the_server_lists_its_channels_in_order_and_its_webhooks_without_tokens() {
        let (bot, _) = fake(|_, path, _| match path {
            "/guilds/42/channels" => (
                200,
                json!([
                    {"id": "2", "name": "b", "type": 0, "position": 2},
                    {"id": "1", "name": "a", "type": 0, "position": 1},
                    {"name": "no id"},
                ]),
            ),
            _ => (
                200,
                json!([
                    {"id": "w1", "name": "Briefs", "channel_id": "1", "token": "secret"},
                    {"id": "w2"},
                    {"name": "no id"},
                ]),
            ),
        })
        .await;
        let ids: Vec<String> = bot.channels().await.unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["1", "2"]);
        let hooks = bot.webhooks().await.unwrap();
        assert_eq!(
            hooks,
            [
                Webhook { id: "w1".into(), name: "Briefs".into(), channel_id: "1".into() },
                Webhook { id: "w2".into(), name: String::new(), channel_id: String::new() },
            ]
        );
        assert!(!serde_json::to_string(&hooks).unwrap().contains("secret"));
    }

    #[tokio::test]
    async fn a_new_webhooks_url_comes_from_discord_or_its_token() {
        let (bot, seen) = fake(|_, path, _| match path {
            "/channels/1/webhooks" => (200, json!({"id": "w1", "name": "PDT", "channel_id": "1", "token": "t1"})),
            "/channels/2/webhooks" => (200, json!({"id": "w2", "url": "https://discord.com/api/webhooks/w2/t2"})),
            _ => (200, json!({"id": "w3"})),
        })
        .await;
        let (w, url) = bot.create_webhook("1", "PDT").await.unwrap();
        assert_eq!((w.id.as_str(), url), ("w1", format!("{}/webhooks/w1/t1", bot.base)));
        assert_eq!(bot.create_webhook("2", "PDT").await.unwrap().1, "https://discord.com/api/webhooks/w2/t2");
        assert_eq!(bot.create_webhook("3", "PDT").await.unwrap_err(), "Discord's answer had no webhook token");
        assert_eq!(seen.seen()[0].2, json!({"name": "PDT"}));
    }

    #[tokio::test]
    async fn a_channel_is_made_only_when_missing_with_its_category() {
        let (bot, seen) = fake(|_, _, body| {
            let id = if body["type"] == 4 { "cat" } else { "new" };
            (201, echo(id, body))
        })
        .await;
        let mut channels = vec![ch("1", "ai-briefs", ChannelKind::Text, None)];
        let (c, made) = ensure_channel(&bot, &mut channels, "#AI Briefs", Some("Bots"), None).await.unwrap();
        assert_eq!((c.id.as_str(), made), ("1", false));
        assert!(seen.seen().is_empty(), "nothing to make");

        let (c, made) = ensure_channel(&bot, &mut channels, "News Feed", Some("Bots"), Some(" Daily ")).await.unwrap();
        assert_eq!((c.id.as_str(), c.name.as_str(), made), ("new", "news-feed", true));
        let all = seen.seen();
        assert_eq!(all.len(), 2, "{all:?}");
        assert_eq!((all[0].0.as_str(), all[0].1.as_str()), ("POST", "/guilds/42/channels"));
        assert_eq!(all[0].2, json!({"name": "Bots", "type": 4}));
        assert_eq!(all[1].2, json!({"name": "news-feed", "type": 0, "parent_id": "cat", "topic": "Daily"}));
        assert_eq!(channels.len(), 3, "both are known now");

        // The category is there now: only the channel is made, and a blank category is none.
        ensure_channel(&bot, &mut channels, "more", Some("bots"), None).await.unwrap();
        ensure_channel(&bot, &mut channels, "loose", Some("  "), Some("  ")).await.unwrap();
        let all = seen.seen();
        assert_eq!(all.len(), 4, "{all:?}");
        assert_eq!(all[2].2, json!({"name": "more", "type": 0, "parent_id": "cat"}));
        assert_eq!(all[3].2, json!({"name": "loose", "type": 0}));
        assert!(ensure_channel(&bot, &mut channels, "  ", None, None).await.is_err());
    }

    #[tokio::test]
    async fn changes_are_refused_before_discord_is_asked() {
        let (bot, seen) = fake(|_, _, body| (201, echo("x", body))).await;
        let mut channels =
            vec![ch("1", "alerts", ChannelKind::Text, None), ch("9", "Bots", ChannelKind::Category, None)];
        let create = |name: &str, kind, category: Option<&str>| Change::Create {
            name: name.into(),
            kind,
            category: category.map(String::from),
            topic: None,
        };
        let e = apply(&bot, &mut channels, &create("#Alerts", ChannelKind::Text, None)).await.unwrap_err();
        assert_eq!(e, "the server already has a text called alerts");
        let e = apply(&bot, &mut channels, &create("bots", ChannelKind::Category, None)).await.unwrap_err();
        assert_eq!(e, "the server already has a category called bots");
        let e = apply(&bot, &mut channels, &create("Sub", ChannelKind::Category, Some("Bots"))).await.unwrap_err();
        assert_eq!(e, "a category can't be in a category");
        // A change names a category that is there; only an ensure makes one.
        let e = apply(&bot, &mut channels, &create("x", ChannelKind::Text, Some("Nope"))).await.unwrap_err();
        assert_eq!(e, "the server has no category called Nope");
        let e = apply(&bot, &mut channels, &Change::Archive { channel: "#gone".into() }).await.unwrap_err();
        assert_eq!(e, "the server has no channel called #gone");
        assert!(seen.seen().is_empty(), "{:?}", seen.seen());
    }

    #[tokio::test]
    async fn changes_rename_move_and_archive_channels() {
        let (bot, seen) = fake(|method, path, body| match method {
            "POST" => (201, echo("arch", body)),
            _ => {
                let id = path.trim_start_matches("/channels/");
                let (name, kind) = if id == "9" { ("Bots", 4) } else { ("alerts", 0) };
                let mut v = json!({"id": id, "name": body.get("name").cloned().unwrap_or(json!(name)), "type": kind});
                if let Some(p) = body.get("parent_id") {
                    v["parent_id"] = p.clone();
                }
                (200, v)
            }
        })
        .await;
        let mut channels =
            vec![ch("1", "alerts", ChannelKind::Text, Some("9")), ch("9", "Bots", ChannelKind::Category, None)];
        let r = apply(&bot, &mut channels, &Change::Rename { channel: "#alerts".into(), name: "Big News".into() })
            .await
            .unwrap();
        assert_eq!(r["renamed"]["name"], "big-news");
        assert_eq!(channels[0].name, "big-news", "the list keeps up");
        // A category (found when no channel is called that) keeps its name as given.
        let r = apply(&bot, &mut channels, &Change::Rename { channel: "Bots".into(), name: "My Bots".into() })
            .await
            .unwrap();
        assert_eq!(r["renamed"]["name"], "My Bots");

        let r = apply(&bot, &mut channels, &Change::Move { channel: "1".into(), category: None }).await.unwrap();
        assert_eq!(r["moved"]["parentId"], Value::Null);
        let r = apply(&bot, &mut channels, &Change::Move { channel: "1".into(), category: Some("9".into()) })
            .await
            .unwrap();
        assert_eq!(r["moved"]["parentId"], "9");

        // Archive makes the Archive category the first time, then reuses it.
        apply(&bot, &mut channels, &Change::Archive { channel: "1".into() }).await.unwrap();
        let r = apply(&bot, &mut channels, &Change::Archive { channel: "1".into() }).await.unwrap();
        assert_eq!(r["archived"]["parentId"], "arch");
        let all: Vec<(String, String, Value)> = seen.seen().into_iter().map(|s| (s.0, s.1, s.2)).collect();
        let patch = |p: Value| ("PATCH".to_string(), "/channels/1".to_string(), p);
        assert_eq!(
            all,
            [
                patch(json!({"name": "big-news"})),
                ("PATCH".into(), "/channels/9".into(), json!({"name": "My Bots"})),
                patch(json!({"parent_id": null})),
                patch(json!({"parent_id": "9"})),
                ("POST".into(), "/guilds/42/channels".into(), json!({"name": "Archive", "type": 4})),
                patch(json!({"parent_id": "arch"})),
                patch(json!({"parent_id": "arch"})),
            ]
        );
    }

    #[test]
    fn a_channel_or_else_a_category_is_found() {
        let channels = [ch("1", "news", ChannelKind::Text, None), ch("2", "News", ChannelKind::Category, None)];
        assert_eq!(find_any(&channels, "news").unwrap().id, "1");
        assert_eq!(find_any(&channels, "2").unwrap().id, "2");
        assert_eq!(find_any(&channels, "nope").unwrap_err(), "the server has no channel called nope");
    }

    #[test]
    fn the_tools_say_what_they_will_do_for_the_users_yes() {
        assert_eq!(GetServer.approval_title(&DiscordGet { connection: None }), "look at the Discord server (the bot)");
        let change = DiscordChange {
            connection: Some("home".into()),
            changes: vec![
                Change::Create { name: "ai-briefs".into(), kind: ChannelKind::Text, category: None, topic: None },
                Change::Archive { channel: "#old".into() },
            ],
        };
        assert_eq!(
            ChangeServer.approval_title(&change),
            "create #ai-briefs, archive #old on the Discord server (home)"
        );
        let hook: DiscordWebhookArgs = serde_json::from_value(json!({"channel": "#briefs", "name": "briefs"})).unwrap();
        assert_eq!(MakeWebhook.approval_title(&hook), "make a webhook on #briefs saved as briefs (the bot)");
        assert!(
            serde_json::from_value::<DiscordWebhookArgs>(json!({"channel": "x", "name": "y", "url": "z"})).is_err()
        );

        let names: Vec<String> = actions().iter().map(|a| a.decl().name.to_string()).collect();
        assert_eq!(names, ["discord_get", "discord_change", "discord_webhook"]);
    }

    #[test]
    fn ensure_channel_needs_a_connection_and_a_channel_name() {
        assert!(EnsureChannel.check(&json!({"connection": "home", "name": "#AI Briefs"})).is_ok());
        let e = EnsureChannel.check(&json!({"connection": "  ", "name": "x"})).unwrap_err();
        assert!(e.contains("name a discord_bot connection"), "{e}");
        assert!(EnsureChannel.check(&json!({"connection": "home", "name": " # "})).is_err());
        assert!(EnsureChannel.check(&json!({"connection": "home"})).is_err());
        assert!(EnsureChannel.check(&json!({"connection": "home", "name": "x", "extra": 1})).is_err());
    }
}
