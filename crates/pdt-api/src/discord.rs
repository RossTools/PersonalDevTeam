//! A Discord server through a bot (PDT-98): a small client for Discord's REST API (no gateway) with a `discord_bot`
//! connection's token, the PM's tools (`discord_get`, `discord_change`, `discord_webhook`) and the
//! `discord.channel.ensure` node kind. Webhook URLs (their tokens) go to the secret store and never into a result or
//! an error (`send_error` drops the URL from a failed request).

use crate::routes::AppState;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use crate::workflow::nodes::discord::DISCORD_WEBHOOK;
use crate::workflow::nodes::http::{client, read_body, send_error};
use crate::workflow::routes::{ConnectionInput, ConnectionSecret};
use axum::Json;
use axum::extract::{Path, State};
use pdt_core::discord::{self as core, ARCHIVE_CATEGORY, Change, Channel, ChannelKind, DISCORD_BOT};
use pdt_core::workflow::api::Connection;
use reqwest::Method;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

/// How many times a rate-limited request is tried.
const TRIES: u32 = 4;

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
    crate::workflow::routes::connection_of_kind(st, DISCORD_BOT, name).await
}

impl Bot {
    /// The bot of a `discord_bot` connection (by name, or the only one).
    pub async fn open(st: &AppState, name: Option<&str>) -> Result<(Connection, Bot), String> {
        let conn = bot_connection(st, name).await?;
        let cfg = core::bot_config(&conn.config).map_err(|e| format!("{}: {e}", conn.name))?;
        let (_, secret) = crate::workflow::routes::connection_with_secret(st, &conn.name).await?;
        let token = secret.filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("{} has no bot token", conn.name))?;
        let base = if conn.url.trim().is_empty() { core::API_BASE.to_string() } else { conn.url.clone() };
        let bot = Bot { base: base.trim_end_matches('/').into(), token: token.trim().into(), guild: cfg.guild_id };
        Ok((conn, bot))
    }

    /// One request; a 429 waits Discord's `retry_after` and tries again.
    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value, String> {
        let url = format!("{}{path}", self.base);
        for _ in 0..TRIES {
            let mut req =
                client().request(method.clone(), &url).header("Authorization", format!("Bot {}", self.token)).header(
                    "User-Agent",
                    format!(
                        "DiscordBot (https://github.com/{}, {})",
                        pdt_core::brand::RELEASE_REPO,
                        env!("CARGO_PKG_VERSION")
                    ),
                );
            if let Some(b) = &body {
                req = req.json(b);
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
            crate::workflow::routes::create_connection(State(st.clone()), Ok(Json(input))).await.map_err(api)?;
        let secret = ConnectionSecret { secret: url };
        if let Err(e) =
            crate::workflow::routes::put_secret(State(st.clone()), Path(c.id.clone()), Ok(Json(secret))).await
        {
            let _ = st.store.delete_connection(&c.id).await;
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

fn api(e: crate::error::ApiError) -> String {
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
