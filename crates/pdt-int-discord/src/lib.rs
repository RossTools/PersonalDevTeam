//! Discord (PDT-98): webhooks that Post to Discord steps post to, and a bot the PM manages a server with (its tools
//! and the `discord.channel.ensure` action), and the templates that post to Discord. The bot also lets the user talk
//! to the PM in its DMs (PDT-304: a Gateway connection, `dm.rs`). pdtd's `discord` feature.

mod bot;
mod chat;
mod core;
mod dm;
mod gateway;
mod post;
mod templates;

use crate::core::DISCORD_BOT;
use crate::post::DISCORD_WEBHOOK;
use pdt_api::sdk::{AnyAction, AppState, BoxFut, Integration, NodeKind, NotificationChannel, Service};
use pdt_core::integration::{ConfigField, ConnectionKind, ConnectionLink, ConnectionStatus, FieldType, FieldWhen};
use pdt_core::workflow::template::Template;
use serde_json::Value;
use std::sync::Arc;

pub use bot::{EnsureChannelConfig, EnsureChannelOutput};
pub use post::{DiscordConfig, DiscordOutput, split_content};

/// What a bot needs: View Channels, Manage Channels, Send Messages, Manage Webhooks.
const BOT_PERMISSIONS: u64 = 1024 + 16 + 2048 + 536870912;

/// How many times a request Discord rate-limits is tried (a webhook post, a bot request).
const TRIES: u32 = 4;

/// A Discord id: digits.
const DIGITS: &str = r"^\d+$";

pub struct Discord;

impl Integration for Discord {
    fn id(&self) -> &'static str {
        "discord"
    }

    fn check_connection_config(&self, kind: &str, config: &Value) -> Result<(), String> {
        if kind == DISCORD_BOT {
            core::bot_config(config)?;
        }
        Ok(())
    }

    fn templates(&self) -> Vec<Template> {
        templates::all()
    }

    fn notification_channels(&self) -> Vec<NotificationChannel> {
        vec![NotificationChannel { label: "Discord", connection_kind: DISCORD_WEBHOOK, node_kind: "discord.post" }]
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![
            ConnectionKind {
                id: DISCORD_WEBHOOK.into(),
                label: "Discord webhook".into(),
                plural: "Discord webhooks".into(),
                setup: "A channel that Post to Discord steps post to. In Discord, open the channel’s settings → \
                        Integrations → Webhooks → New Webhook, copy its URL, and add it here."
                    .into(),
                description: "Posts to one Discord channel (Post to Discord).".into(),
                summary: Some("#{channel}".into()),
                url: false,
                secret_label: "Webhook URL".into(),
                secret_hint: "In Discord: the channel’s settings (Edit Channel) → Integrations → Webhooks → New \
                              Webhook → Copy Webhook URL. It looks like https://discord.com/api/webhooks/…"
                    .into(),
                ..Default::default()
            },
            ConnectionKind {
                id: DISCORD_BOT.into(),
                label: "Discord bot".into(),
                plural: "Discord bots".into(),
                setup: "A bot that lets the PM manage a Discord server: make and organise channels, and make the \
                        webhooks workflows post to. In the Discord Developer Portal, make an application, then under \
                        Bot reset its token and copy it. Invite the bot to your server with the link this dialog \
                        shows (Manage Channels, Manage Webhooks, Send Messages). Give it your Discord user ID to talk \
                        to the PM in the bot’s DMs; that needs Message Content Intent turned on under Bot → \
                        Privileged Gateway Intents."
                    .into(),
                description: "Lets the PM manage a Discord server’s channels and webhooks, and you talk to the PM in \
                              the bot’s DMs."
                    .into(),
                url: false,
                secret_label: "Bot token".into(),
                secret_hint: "Discord Developer Portal → your application → Bot → Reset Token → Copy.".into(),
                fields: vec![
                    ConfigField {
                        hint: Some(
                            "In Discord, turn on Developer Mode (Settings → Advanced), then right-click the server → \
                             Copy Server ID."
                                .into(),
                        ),
                        pattern: Some(DIGITS.into()),
                        numeric: true,
                        ..ConfigField::new("guildId", "Server ID")
                    },
                    ConfigField {
                        hint: Some(
                            "Developer Portal → your application → General Information. Used only to make the invite \
                             link."
                                .into(),
                        ),
                        pattern: Some(DIGITS.into()),
                        numeric: true,
                        optional: true,
                        ..ConfigField::new("applicationId", "Application ID")
                    },
                    ConfigField {
                        hint: Some(
                            "To talk to the PM in the bot's DMs: with Developer Mode on, right-click your name → Copy \
                             User ID. Only this account's DMs and button presses reach the PM. It needs Message \
                             Content Intent on (Developer Portal → Bot → Privileged Gateway Intents)."
                                .into(),
                        ),
                        pattern: Some(DIGITS.into()),
                        numeric: true,
                        optional: true,
                        ..ConfigField::new("dmUserId", "Your Discord user ID (optional)")
                    },
                    ConfigField {
                        r#type: Some(FieldType::Switch),
                        when: Some(FieldWhen::filled("dmUserId")),
                        ..ConfigField::new("dmInbox", "Also send Inbox items (stalled, needs input) as DMs")
                    },
                ],
                link: Some(ConnectionLink {
                    title: "Invite the bot".into(),
                    url: format!(
                        "https://discord.com/oauth2/authorize?client_id={{applicationId}}&scope=bot&permissions={BOT_PERMISSIONS}"
                    ),
                    hint: Some("Open it, pick the server, and keep the permissions it asks for.".into()),
                }),
                summary: Some("Server {guildId}".into()),
                status: true,
                pm_acts: Some("The PM changes this server without asking".into()),
                ..Default::default()
            },
        ]
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(post::Discord), Box::new(bot::EnsureChannel)]
    }

    fn services(&self) -> Vec<Service> {
        vec![Service { name: "the Discord DM link", run: dm_service }]
    }

    fn connection_status<'a>(
        &'a self,
        _st: &'a AppState,
        connection: &'a pdt_core::workflow::api::Connection,
    ) -> BoxFut<'a, Option<ConnectionStatus>> {
        Box::pin(async move { dm::connection_status(connection) })
    }

    fn actions(&self) -> Vec<Arc<dyn AnyAction>> {
        bot::actions()
    }

    fn approval_title(&self, tool: &str, input: &Value) -> Option<String> {
        matches!(tool, "discord_change" | "discord_webhook")
            .then(|| core::ask_title(tool, input).unwrap_or_else(|| "Discord".into()))
    }

    fn pm_prompt(&self) -> Option<&'static str> {
        Some(PM_PROMPT)
    }
}

fn dm_service(st: AppState) -> pdt_api::sdk::BoxFut<'static, ()> {
    dm::service(st)
}

/// What the PM's prompt says about the Discord tools.
pub(crate) const PM_PROMPT: &str = "- Discord: `discord_get` shows the server (channels, categories, webhooks). `discord_change` \
creates, renames, moves and archives channels and categories; `discord_webhook` makes a channel's webhook and saves it \
as a `discord_webhook` connection that `discord.post` steps use by name (its URL never reaches you). Both wait for the \
user's yes unless the bot connection lets you act; say what you will change first.";
